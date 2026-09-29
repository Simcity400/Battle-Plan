//! Persistent character state. Native walks use pooled rows and literal-byte
//! arenas; sorting, duplicate checks and owned replay strings run off-thread.

pub(crate) mod advisors;
pub(crate) mod census;
#[cfg(test)]
mod census_tests;

use crate::{
    read_accounting,
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{
    Memory, active_ability_record, cadvisor, carmy_leader, ccharacter, ccharacter_manager,
    ccharacter_template, ccountry, ccountry_leader, cgame_state, character_advisor_node, character_leader_node,
    cnavy_leader, coperative_leader, cpdx_array::Array, cunit_leader, cunit_leader_skill,
    cunit_leader_trait, msvc_string, msvc_tree_node, read, unit_leader_trait_experience_entry,
    walk::Outcome,
};
use replay::{
    BranchId,
    frame::{TableInput, TableView},
    rows::{
        ActiveAbilityRow, AdvisorRoleRow, CharacterRow, CountryLeaderRoleRow, TableId, TableRows,
        TableSchema, UnitLeaderRow, UnitLeaderTraitExperienceRow, UnitLeaderTraitRow, tables,
    },
};

use std::{
    mem::size_of,
    sync::{Arc, Weak},
};

pub const N: usize = 6;
pub const TABLES: [TableId; N] = [
    TableId::Characters,
    TableId::CountryLeaderRoles,
    TableId::AdvisorRoles,
    TableId::UnitLeaders,
    TableId::UnitLeaderTraits,
    TableId::UnitLeaderTraitExperience,
];

/// The first table in each group carries its full QPC cost.
/// Companion slots are zero: their shared traversal has one
/// measured cost, not an invented allocation of time between its outputs.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("character roster and unit leaders", 0, &[0, 3]),
    ("country leader roles", 1, &[1]),
    ("advisor roles", 2, &[2]),
    ("unit leader traits and experience", 4, &[4, 5]),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub bytes: [usize; N],
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            // First full capture contained 13,095 characters, 2,019 leader
            // roles, 1,271 unit leaders and 19,555 trait-experience entries.
            rows: [16_384, 4096, 4096, 2048, 4096, 32_768],
            bytes: [524_288, 0, 262_144, 131_072, 131_072, 524_288],
        }
    }
}

impl Capacities {
    /// Characters keeps one row per roster occurrence and both names and the
    /// template file of each (`ccharacter::demand`), so the live roster sizes
    /// the first full hour rather than a vanilla count.
    pub(crate) fn raise_for_characters(&mut self, demand: ccharacter::Demand) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[0], rows(demand.rows, 1));
        raise(&mut self.bytes[0], rows(demand.bytes, 1));
    }
}

pub(super) struct Buffer<R> {
    pub(super) rows: CaptureRows<R>,
    pub(super) bytes: Arena<u8>,
}

impl<R: Default + Clone> Buffer<R> {
    fn new(capacities: &Capacities, index: usize) -> Self {
        Self::with(capacities.rows[index], capacities.bytes[index])
    }

    pub(super) fn with(rows: usize, bytes: usize) -> Self {
        Self {
            rows: CaptureRows::new(rows),
            bytes: Arena::new(bytes),
        }
    }

    fn reset(&mut self) {
        self.rows.reset();
        self.bytes.reset();
    }

    fn grow(&mut self, capacities: &mut Capacities, index: usize) {
        self.rows.grow(&mut capacities.rows[index]);
        self.bytes.grow(&mut capacities.bytes[index]);
    }

    fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes() + self.bytes.allocated_bytes()
    }

    fn batch(&mut self) -> Batch<'_, R> {
        Batch::new(&mut self.rows.rows, &mut self.bytes.first)
    }

    fn primary(&mut self, result: ReadResult) {
        self.rows.outcome = result.outcome;
        self.bytes.len = result.bytes_len;
        self.bytes.required = result.required_bytes;
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct LeaderRow {
    native: cunit_leader::Row,
    text_a_start: usize,
    text_a_len: usize,
    text_b_start: usize,
    text_b_len: usize,
}

pub struct Captured {
    active: [bool; 6],
    characters: Buffer<ccharacter::Row>,
    leaders: Buffer<ccountry_leader::Row>,
    advisors: Buffer<cadvisor::Row>,
    units: Buffer<LeaderRow>,
    traits: Buffer<cunit_leader::TraitRow>,
    trait_experience: Buffer<cunit_leader::TraitExperienceRow>,
    /// Tables acquired by independent raw jobs, including terminal failures.
    /// The typed walk handles only the remaining tables; the decode worker
    /// merges successful parts before publication. Preserved across reset
    /// and cleared per part after decode.
    pub(crate) raw_active: [bool; 6],
}

/// Disjoint table owners. Part zero supplies the current portrait census;
/// role and trait acquisition can run concurrently on the same stopped world.
pub(crate) const PART_MASKS: [[bool; 6]; 4] = [
    [true, false, false, true, false, false],
    [false, true, false, false, false, false],
    [false, false, true, false, false, false],
    [false, false, false, false, true, true],
];
pub(crate) const PARTS: usize = PART_MASKS.len();

static COPY_CALLS: [std::sync::atomic::AtomicU64; PARTS] =
    [const { std::sync::atomic::AtomicU64::new(0) }; PARTS];
static COPY_NS: [std::sync::atomic::AtomicU64; PARTS] =
    [const { std::sync::atomic::AtomicU64::new(0) }; PARTS];
static BACKEND_NS: [std::sync::atomic::AtomicU64; PARTS] =
    [const { std::sync::atomic::AtomicU64::new(0) }; PARTS];
static ADVISOR_SECTION_CALLS: [std::sync::atomic::AtomicU64; 2] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 2];
static ADVISOR_SECTION_PLANNED: [std::sync::atomic::AtomicU64; 2] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 2];
static ADVISOR_SECTION_NS: [std::sync::atomic::AtomicU64; 2] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 2];

pub(crate) fn acquisition_diagnostics() -> serde_json::Value {
    use std::sync::atomic::Ordering::Relaxed;
    serde_json::json!({
        "scope": "cumulative active character acquisitions; backend is fresh prepared execution only; other includes discovery/continuation, trait acquisition and metadata restoration; overlapping jobs must not be summed as callback wall",
        "parts": (0..PARTS).map(|part| {
            let calls = COPY_CALLS[part].load(Relaxed);
            let total = COPY_NS[part].load(Relaxed);
            let backend = BACKEND_NS[part].load(Relaxed);
            serde_json::json!({
                "name": READ_GROUPS[part].0,
                "calls": calls,
                "total_ns": total,
                "backend_ns": backend,
                "mean_backend_ns": backend.checked_div(calls).unwrap_or(0),
                "mean_other_ns": total.saturating_sub(backend).checked_div(calls).unwrap_or(0),
            })
        }).collect::<Vec<_>>(),
        "advisor_sections": (0..2).map(|section| {
            let calls = ADVISOR_SECTION_CALLS[section].load(Relaxed);
            let ns = ADVISOR_SECTION_NS[section].load(Relaxed);
            serde_json::json!({
                "roster": section, "calls": calls,
                "complete_prepared": ADVISOR_SECTION_PLANNED[section].load(Relaxed),
                "total_ns": ns, "mean_ns": ns.checked_div(calls).unwrap_or(0),
            })
        }).collect::<Vec<_>>()
    })
}

fn anchor(active: [bool; 6]) -> usize {
    active.iter().position(|&value| value).unwrap_or(0)
}

pub(crate) const SHADOW_TABLES: [TableId; 4] = [
    TableId::Characters,
    TableId::CountryLeaderRoles,
    TableId::AdvisorRoles,
    TableId::UnitLeaderTraits,
];

#[derive(Default)]
struct NativeShadow<R> {
    rows: Vec<R>,
    bytes: Vec<u8>,
    required: usize,
    output: Weak<TableRows>,
    hits: u64,
    misses: u64,
}

impl<R> NativeShadow<R> {
    fn clear(&mut self) {
        self.rows.clear();
        self.bytes.clear();
        self.required = 0;
        self.output = Weak::new();
    }

    fn statistics(&self) -> CharacterShadowStats {
        CharacterShadowStats {
            hits: self.hits,
            misses: self.misses,
            row_capacity_bytes: self.rows.capacity() * size_of::<R>(),
            arena_capacity_bytes: self.bytes.capacity(),
        }
    }
}

/// Spool-owned exact conversion memo, never a substitute for current capture.
/// Copies only used typed fields/bytes; no live pointer or Captured lease.
#[derive(Default)]
pub(crate) struct CharacterShadow {
    branch: Option<BranchId>,
    characters: NativeShadow<ccharacter::Row>,
    leaders: NativeShadow<ccountry_leader::Row>,
    advisors: NativeShadow<cadvisor::Row>,
    traits: NativeShadow<cunit_leader::TraitRow>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CharacterShadowStats {
    pub hits: u64,
    pub misses: u64,
    pub row_capacity_bytes: usize,
    pub arena_capacity_bytes: usize,
}

impl CharacterShadow {
    pub(crate) fn statistics(&self) -> [CharacterShadowStats; 4] {
        [
            self.characters.statistics(),
            self.leaders.statistics(),
            self.advisors.statistics(),
            self.traits.statistics(),
        ]
    }
}

/// Cache references must come from this branch's current complete spool cache.
/// Weak identity prevents reuse after replacement, partial rows or invalidation.
pub(crate) struct CharacterConversion<'a> {
    shadow: &'a mut CharacterShadow,
    cached: [Option<&'a Arc<TableRows>>; 4],
}

impl<'a> CharacterConversion<'a> {
    pub(crate) fn new(
        branch: BranchId,
        shadow: &'a mut CharacterShadow,
        cached: [Option<&'a Arc<TableRows>>; 4],
    ) -> Self {
        if shadow.branch != Some(branch) {
            shadow.characters.clear();
            shadow.leaders.clear();
            shadow.advisors.clear();
            shadow.traits.clear();
            shadow.branch = Some(branch);
        }
        Self { shadow, cached }
    }
}

fn shadow_input<R: Clone + PartialEq>(
    table: TableId,
    buffer: &Buffer<R>,
    shadow: &mut NativeShadow<R>,
    cached: Option<&Arc<TableRows>>,
    valid_row: impl Fn(&R, &[u8]) -> bool,
    convert: impl FnOnce() -> TableView,
) -> TableInput {
    let source = match buffer.rows.outcome {
        Outcome::Rows { len } => buffer.rows.rows.get(..len),
        _ => None,
    };
    let source = source.zip(buffer.bytes.first.get(..buffer.bytes.len));
    if let Some((rows, bytes)) = source
        && let Some(cached) = cached
        && cached.table() == table
        && let Some(previous) = shadow.output.upgrade()
        && Arc::ptr_eq(cached, &previous)
        // Fieldwise equality excludes padding. Changed native order misses.
        && shadow.rows == rows
        && shadow.required == buffer.bytes.required
        && shadow.bytes == bytes
    {
        shadow.hits = shadow.hits.saturating_add(1);
        return TableInput::SharedComplete(Arc::clone(cached));
    }
    let skipped = matches!(
        buffer.rows.outcome,
        Outcome::Unavailable {
            reason: replay::cadence::SKIPPED_REASON
        }
    );
    if !skipped {
        shadow.misses = shadow.misses.saturating_add(1);
    }
    // An exact hit above has the same ranges and arena as the previously
    // validated output. Only changed input needs another row-validation pass.
    let source = source.filter(|(rows, bytes)| rows.iter().all(|row| valid_row(row, bytes)));
    match (source, convert()) {
        (Some((rows, bytes)), TableView::Complete(output)) => {
            let output = Arc::new(output);
            shadow.rows.clear();
            shadow.rows.extend_from_slice(rows);
            shadow.bytes.clear();
            shadow.bytes.extend_from_slice(bytes);
            shadow.required = buffer.bytes.required;
            shadow.output = Arc::downgrade(&output);
            TableInput::SharedComplete(output)
        }
        (_, view) => {
            if !skipped {
                shadow.clear();
            }
            TableInput::Sampled(view)
        }
    }
}

impl Captured {
    pub(crate) fn character_activity(&self) -> [bool; 6] {
        self.active
    }

    pub(crate) fn set_character_activity(&mut self, active: [bool; 6]) {
        self.active = active;
        self.fail_character_tables(active.map(|value| !value), replay::cadence::SKIPPED_REASON);
    }
    pub(crate) fn trait_capture_capacities(&self) -> ([usize; 2], [usize; 2]) {
        (
            [
                self.traits.rows.rows.len(),
                self.trait_experience.rows.rows.len(),
            ],
            [
                self.traits.bytes.first.len(),
                self.trait_experience.bytes.first.len(),
            ],
        )
    }

    fn family(&mut self) -> Family<'_> {
        Family {
            characters: self.characters.batch(),
            leaders: self.leaders.batch(),
            advisors: self.advisors.batch(),
            units: self.units.batch(),
            traits: self.traits.batch(),
            experience: self.trait_experience.batch(),
        }
    }

    pub fn with_capacities(capacities: &Capacities) -> Self {
        Self {
            characters: Buffer::new(capacities, 0),
            leaders: Buffer::new(capacities, 1),
            advisors: Buffer::new(capacities, 2),
            units: Buffer::new(capacities, 3),
            traits: Buffer::new(capacities, 4),
            trait_experience: Buffer::new(capacities, 5),
            raw_active: [false; 6],
            active: [false; 6],
        }
    }

    pub fn reset(&mut self) {
        self.characters.reset();
        self.leaders.reset();
        self.advisors.reset();
        self.units.reset();
        self.traits.reset();
        self.trait_experience.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.characters.allocated_bytes()
            + self.leaders.allocated_bytes()
            + self.advisors.allocated_bytes()
            + self.units.allocated_bytes()
            + self.traits.allocated_bytes()
            + self.trait_experience.allocated_bytes()
    }

    pub fn grow(&mut self, capacities: &mut Capacities) {
        self.characters.grow(capacities, 0);
        self.leaders.grow(capacities, 1);
        capacities.rows[2] = capacities.rows[2].max(self.advisors.rows.rows.capacity());
        capacities.bytes[2] = capacities.bytes[2].max(self.advisors.bytes.first.capacity());
        self.advisors.grow(capacities, 2);
        self.units.grow(capacities, 3);
        self.traits.grow(capacities, 4);
        self.trait_experience.grow(capacities, 5);
    }

    /// The scheduled persistent tables share one current roster.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.reset();
        let mut ticks = [0; N];
        let active = TABLES.map(|table| replay::cadence::should_walk(table, hour, force_full));
        let family_active = if self.raw_active.iter().any(|&active| active) {
            self.active
        } else {
            std::array::from_fn(|index| active[index])
        };
        self.active = family_active;
        macro_rules! skip {
            ($($field:ident = $index:literal),+ $(,)?) => {$(
                if !family_active[$index] {
                    self.$field.rows.outcome = Outcome::Unavailable {
                        reason: replay::cadence::SKIPPED_REASON,
                    };
                }
            )+};
        }
        skip!(
            characters = 0,
            leaders = 1,
            advisors = 2,
            units = 3,
            traits = 4,
            trait_experience = 5
        );
        let typed_active: [bool; 6] = std::array::from_fn(|index| family_active[index] && !self.raw_active[index]);
        for mask in PART_MASKS {
        let typed_active = std::array::from_fn(|index| typed_active[index] && mask[index]);
        if typed_active.iter().any(|&active| active) {
            let slot = anchor(typed_active);
            let _reads = read_accounting::table(TABLES[slot]);
            let begin = qpc();
            let result = walk_family(
                memory,
                base,
                game_state,
                self.family().select(typed_active),
                None,
                None,
            );
            self.install_family(typed_active, result);
            ticks[slot] = elapsed(begin);
        }
        }
        ticks
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.characters.rows.outcome,
            self.leaders.rows.outcome,
            self.advisors.rows.outcome,
            self.units.rows.outcome,
            self.traits.rows.outcome,
            self.trait_experience.rows.outcome,
        ]
    }

    /// Called only by the spool thread after the buffer has left the callback.
    pub fn tables(&self) -> [TableView; N] {
        self.convert_tables(
            |view| view,
            self.characters_table(),
            self.leaders_table(),
            self.advisors_table(),
            self.traits_table(),
        )
    }

    pub(crate) fn table_inputs(
        &self,
        reuse: Option<&mut CharacterConversion<'_>>,
    ) -> [TableInput; N] {
        let Some(reuse) = reuse else {
            return self.tables().map(TableInput::Sampled);
        };
        let characters = shadow_input(
            SHADOW_TABLES[0],
            &self.characters,
            &mut reuse.shadow.characters,
            reuse.cached[0],
            character_ranges_valid,
            || self.characters_table(),
        );
        let leaders = shadow_input(
            SHADOW_TABLES[1],
            &self.leaders,
            &mut reuse.shadow.leaders,
            reuse.cached[1],
            |_, _| true,
            || self.leaders_table(),
        );
        let advisors = shadow_input(
            SHADOW_TABLES[2],
            &self.advisors,
            &mut reuse.shadow.advisors,
            reuse.cached[2],
            |row, bytes| {
                valid(bytes, row.key_start, row.key_len)
                    && valid(bytes, row.idea_start, row.idea_len)
                    && valid(bytes, row.slot_start, row.slot_len)
            },
            || self.advisors_table(),
        );
        let traits = shadow_input(
            SHADOW_TABLES[3],
            &self.traits,
            &mut reuse.shadow.traits,
            reuse.cached[3],
            |row, bytes| valid(bytes, row.token_start, row.token_len),
            || self.traits_table(),
        );
        self.convert_tables(TableInput::Sampled, characters, leaders, advisors, traits)
    }

    fn characters_table(&self) -> TableView {
        view::<tables::Characters, _>(
            &self.characters,
            character_ranges_valid,
            |row, bytes| CharacterRow {
                id: row.id,
                country: row.country,
                name_token_id: row.name_token_id,
                roster: row.roster,
                ordinal: row.ordinal,
                name: owned(bytes, row.name_start, row.name_len),
                token: owned(bytes, row.token_start, row.token_len),
                template_present: u8::from(row.template != 0),
                template_tag: row.template_identity.tag,
                template_file: owned(bytes, row.template_file_start, row.template_file_len),
                template_line: row.template_identity.line,
                template_generated: row.template_identity.generated,
            },
        )
    }

    fn leaders_table(&self) -> TableView {
        view::<tables::CountryLeaderRoles, _>(
            &self.leaders,
            |_, _| true,
            |row, _| CountryLeaderRoleRow {
                character: row.character,
                country: row.country,
                ideology: row.ideology,
            },
        )
    }

    fn advisors_table(&self) -> TableView {
        view::<tables::AdvisorRoles, _>(
            &self.advisors,
            |row, bytes| {
                valid(bytes, row.key_start, row.key_len)
                    && valid(bytes, row.idea_start, row.idea_len)
                    && valid(bytes, row.slot_start, row.slot_len)
            },
            |row, bytes| AdvisorRoleRow {
                character: row.character,
                country: row.country,
                ordinal: row.ordinal,
                key: owned(bytes, row.key_start, row.key_len),
                idea: owned(bytes, row.idea_start, row.idea_len),
                slot: owned(bytes, row.slot_start, row.slot_len),
            },
        )
    }

    fn traits_table(&self) -> TableView {
        view::<tables::UnitLeaderTraits, _>(
            &self.traits,
            |row, bytes| valid(bytes, row.token_start, row.token_len),
            |row, bytes| UnitLeaderTraitRow {
                leader_reference: leader_reference(row.leader_id),
                ordinal: row.ordinal,
                token_id: row.token_id,
                definition_ordinal: row.definition_ordinal,
                token: owned(bytes, row.token_start, row.token_len),
            },
        )
    }

    fn convert_tables<T>(
        &self,
        wrap: impl Fn(TableView) -> T,
        characters: T,
        leaders: T,
        advisors: T,
        traits: T,
    ) -> [T; N] {
        [
            characters,
            leaders,
            advisors,
            wrap(view::<tables::UnitLeaders, _>(
                &self.units,
                |row, bytes| {
                    valid(bytes, row.text_a_start, row.text_a_len)
                        && valid(bytes, row.text_b_start, row.text_b_len)
                },
                leader_row,
            )),
            traits,
            wrap(view::<tables::UnitLeaderTraitExperience, _>(
                &self.trait_experience,
                |row, bytes| valid(bytes, row.trait_row.token_start, row.trait_row.token_len),
                |row, bytes| UnitLeaderTraitExperienceRow {
                    leader_reference: leader_reference(row.trait_row.leader_id),
                    ordinal: row.trait_row.ordinal,
                    token_id: row.trait_row.token_id,
                    definition_ordinal: row.trait_row.definition_ordinal,
                    token: owned(bytes, row.trait_row.token_start, row.trait_row.token_len),
                    experience: row.experience,
                },
            )),
        ]
    }
}

fn elapsed(begin: i64) -> u64 {
    (qpc() - begin).max(0) as u64
}

fn character_ranges_valid(row: &ccharacter::Row, bytes: &[u8]) -> bool {
    valid(bytes, row.name_start, row.name_len)
        && valid(bytes, row.token_start, row.token_len)
        && valid(bytes, row.template_file_start, row.template_file_len)
}

/// Template identities an earlier hour read, keyed by character id and
/// guarded by the character's current template pointer (`CCharacter+0x20`,
/// in the name token span every character view copies each hour). Only a
/// character's build and a save load write that pointer, and a template's
/// tag, file, line and generated byte are written before any character holds
/// it, so an unchanged pointer keeps its identity; a changed or new pointer
/// is read again. A full hour, or another module or game state, reads every
/// template again. The owning job's copy and its decode consult the same
/// state: only the decode worker rebuilds it, from the rows it produced, and
/// a change moves `generation`, which keys the prepared read program.
#[derive(Debug, Default)]
pub(crate) struct TemplateCache {
    scope: (usize, usize),
    entries: Vec<TemplateEntry>,
    files: Vec<u8>,
    generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TemplateEntry {
    id: u32,
    pointer: usize,
    identity: ccharacter_template::Identity,
    file_start: usize,
    file_len: usize,
}

impl TemplateCache {
    /// Before a copy: forget every identity on a full hour or a new scope.
    /// Clearing keeps the allocation, so the game thread never allocates.
    fn begin(&mut self, base: usize, game_state: usize, force_full: bool) {
        if force_full || self.scope != (base, game_state) {
            if !self.entries.is_empty() || self.scope != (base, game_state) {
                self.generation = self.generation.wrapping_add(1);
            }
            self.entries.clear();
            self.files.clear();
            self.scope = (base, game_state);
        }
    }

    /// The identity read for this character's template at this pointer.
    fn known(&self, id: u32, pointer: usize) -> Option<ccharacter_template::Known<'_>> {
        let index = self.entries.binary_search_by_key(&id, |entry| entry.id).ok()?;
        let entry = &self.entries[index];
        (entry.pointer == pointer).then(|| ccharacter_template::Known {
            identity: entry.identity,
            file: &self.files[entry.file_start..entry.file_start + entry.file_len],
        })
    }

    /// Decode worker only: the identities of a complete Characters read. Any
    /// other outcome forgets them all. An id held by more than one pointer is
    /// left out, so every occurrence reads its own template again.
    fn rebuild(&mut self, outcome: Outcome, rows: &[ccharacter::Row], bytes: &[u8]) {
        let rows = match outcome {
            Outcome::Rows { len } => rows.get(..len),
            _ => None,
        };
        let mut entries = Vec::new();
        let mut files = Vec::new();
        if let Some(rows) = rows {
            let mut present: Vec<_> = rows
                .iter()
                .filter(|row| row.template != 0 && character_ranges_valid(row, bytes))
                .collect();
            present.sort_by_key(|row| row.id);
            entries.reserve_exact(present.len());
            for group in present.chunk_by(|a, b| a.id == b.id) {
                let row = group[0];
                if group.iter().any(|other| other.template != row.template) {
                    continue;
                }
                let file = &bytes[row.template_file_start..row.template_file_start + row.template_file_len];
                entries.push(TemplateEntry {
                    id: row.id,
                    pointer: row.template,
                    identity: row.template_identity,
                    file_start: files.len(),
                    file_len: file.len(),
                });
                files.extend_from_slice(file);
            }
        }
        let unchanged = entries.len() == self.entries.len()
            && entries.iter().zip(&self.entries).all(|(new, old)| {
                (new.id, new.pointer, new.identity) == (old.id, old.pointer, old.identity)
                    && files[new.file_start..new.file_start + new.file_len]
                        == self.files[old.file_start..old.file_start + old.file_len]
            });
        if !unchanged {
            self.entries = entries;
            self.files = files;
            self.generation = self.generation.wrapping_add(1);
        }
    }

    fn allocated_bytes(&self) -> u64 {
        crate::raw_memory::vector_bytes(&self.entries) + crate::raw_memory::vector_bytes(&self.files)
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ReadResult {
    pub(super) outcome: Outcome,
    pub(super) bytes_len: usize,
    pub(super) required_bytes: usize,
}

impl ReadResult {
    fn rows(outcome: Outcome) -> Self {
        Self {
            outcome,
            bytes_len: 0,
            required_bytes: 0,
        }
    }
}

impl From<ccharacter::NameWalk> for ReadResult {
    fn from(result: ccharacter::NameWalk) -> Self {
        Self {
            outcome: result.outcome,
            bytes_len: result.bytes_len,
            required_bytes: result.required_bytes,
        }
    }
}

impl From<cunit_leader::TraitWalk> for ReadResult {
    fn from(result: cunit_leader::TraitWalk) -> Self {
        Self {
            outcome: result.outcome,
            bytes_len: result.bytes_len,
            required_bytes: result.required_bytes,
        }
    }
}

impl From<active_ability_record::TokenWalk> for ReadResult {
    fn from(result: active_ability_record::TokenWalk) -> Self {
        Self {
            outcome: result.outcome,
            bytes_len: result.bytes_len,
            required_bytes: result.required_bytes,
        }
    }
}

/// Aggregates exact native counts across owners. After the first capacity miss
/// later owners contribute counts only; no later row fills a hole in the prefix.
struct Batch<'a, R> {
    rows: &'a mut [R],
    bytes: &'a mut [u8],
    len: usize,
    bytes_len: usize,
    required_bytes: usize,
    unread: u32,
    failure: Option<&'static str>,
}

impl<'a, R> Batch<'a, R> {
    fn new(rows: &'a mut [R], bytes: &'a mut [u8]) -> Self {
        Self {
            rows,
            bytes,
            len: 0,
            bytes_len: 0,
            required_bytes: 0,
            unread: 0,
            failure: None,
        }
    }

    fn active(&self) -> bool {
        self.failure.is_none()
    }

    fn fail(&mut self, reason: &'static str) {
        if self.failure.is_none() {
            self.failure = Some(reason);
        }
    }

    fn append(
        &mut self,
        walk: impl FnOnce(&mut [R], &mut [u8]) -> ReadResult,
        relocate: impl Fn(&mut R, usize) -> bool,
    ) {
        if !self.active() {
            return;
        }
        let rows_end = if self.unread == 0 {
            self.rows.len()
        } else {
            self.len
        };
        let bytes_end = if self.unread == 0 {
            self.bytes.len()
        } else {
            self.bytes_len
        };
        let result = walk(
            &mut self.rows[self.len..rows_end],
            &mut self.bytes[self.bytes_len..bytes_end],
        );
        let (len, unread) = match result.outcome {
            Outcome::Rows { len } => (len, 0),
            Outcome::Truncated { len, unread_rows } if unread_rows != 0 => (len, unread_rows),
            Outcome::Truncated { .. } => {
                self.fail("character walk returned truncation without missing rows");
                return;
            }
            Outcome::Unavailable { reason } => {
                self.fail(reason);
                return;
            }
        };
        if len > rows_end - self.len
            || result.bytes_len > bytes_end - self.bytes_len
            || result.required_bytes < result.bytes_len
        {
            self.fail("character walk exceeded its pooled buffer");
            return;
        }
        let bytes_start = self.bytes_len;
        if !self.rows[self.len..self.len + len]
            .iter_mut()
            .all(|row| relocate(row, bytes_start))
        {
            self.fail("character byte offset overflow");
            return;
        }
        let Some(required) = self.required_bytes.checked_add(result.required_bytes) else {
            self.fail("character byte count overflow");
            return;
        };
        let Some(unread) = self.unread.checked_add(unread) else {
            self.fail("character missing row count overflow");
            return;
        };
        self.len += len;
        self.bytes_len += result.bytes_len;
        self.required_bytes = required;
        self.unread = unread;
    }

    fn result(&self) -> ReadResult {
        if let Some(reason) = self.failure {
            return ReadResult::rows(Outcome::Unavailable { reason });
        }
        ReadResult {
            outcome: if self.unread == 0 {
                Outcome::Rows { len: self.len }
            } else {
                Outcome::Truncated {
                    len: self.len,
                    unread_rows: self.unread,
                }
            },
            bytes_len: self.bytes_len,
            required_bytes: self.required_bytes,
        }
    }
}

fn relocate(start: &mut usize, base: usize) -> bool {
    let Some(value) = start.checked_add(base) else {
        return false;
    };
    *start = value;
    true
}

struct Family<'a> {
    characters: Batch<'a, ccharacter::Row>,
    leaders: Batch<'a, ccountry_leader::Row>,
    advisors: Batch<'a, cadvisor::Row>,
    units: Batch<'a, LeaderRow>,
    traits: Batch<'a, cunit_leader::TraitRow>,
    experience: Batch<'a, cunit_leader::TraitExperienceRow>,
}

impl Family<'_> {
    fn select(mut self, active: [bool; 6]) -> Self {
        macro_rules! select {
            ($($field:ident = $index:literal),+ $(,)?) => {$(
                if !active[$index] {
                    self.$field.fail(replay::cadence::SKIPPED_REASON);
                }
            )+};
        }
        select!(
            characters = 0,
            leaders = 1,
            advisors = 2,
            units = 3,
            traits = 4,
            experience = 5
        );
        self
    }
}

impl Family<'_> {
    fn alive(&self) -> bool {
        self.characters.active()
            || self.leaders.active()
            || self.advisors.active()
            || self.units.active()
            || self.traits.active()
            || self.experience.active()
    }

    /// Domains a character view must copy: names, leader map, unit, advisors.
    fn required(&self) -> [bool; 4] {
        [
            self.characters.active(),
            self.leaders.active(),
            self.units.active() || self.traits.active() || self.experience.active(),
            self.advisors.active(),
        ]
    }

    fn fail(&mut self, reason: &'static str) {
        self.characters.fail(reason);
        self.leaders.fail(reason);
        self.advisors.fail(reason);
        self.units.fail(reason);
        self.traits.fail(reason);
        self.experience.fail(reason);
    }

    fn results(&self) -> [ReadResult; 6] {
        [
            self.characters.result(),
            self.leaders.result(),
            self.advisors.result(),
            self.units.result(),
            self.traits.result(),
            self.experience.result(),
        ]
    }

    /// Every table's rows for one roster occurrence. `templates` holds the
    /// template identities the decoding job's copy relied on; the live walk
    /// has none and reads every template.
    fn visit<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        view: ccharacter::View<'_, M>,
        owned_traits: Option<&TraitCapture>,
        templates: Option<&TemplateCache>,
        trait_cursors: &mut [usize; 2],
    ) {
        let (character_pointer, character_id) = view.reference();
        match view.entry() {
            Ok(character) => {
                self.characters.append(
                    |rows, bytes| {
                        ccharacter::walk_entry(
                            &view,
                            base,
                            character,
                            |template| templates.and_then(|cache| cache.known(character.id, template)),
                            rows,
                            bytes,
                        )
                        .into()
                    },
                    |row, offset| {
                        relocate(&mut row.name_start, offset)
                            && relocate(&mut row.token_start, offset)
                            && relocate(&mut row.template_file_start, offset)
                    },
                );
                self.leaders.append(
                    |rows, _| {
                        ReadResult::rows(ccountry_leader::walk_entry(&view, base, character, rows))
                    },
                    |_, _| true,
                );
                self.advisors.append(
                    |rows, bytes| cadvisor::walk_entry(&view, base, character, rows, bytes).into(),
                    |row, offset| {
                        relocate(&mut row.key_start, offset)
                            && relocate(&mut row.idea_start, offset)
                            && relocate(&mut row.slot_start, offset)
                    },
                );
            }
            Err(reason) => {
                self.characters.fail(reason);
                self.leaders.fail(reason);
                self.advisors.fail(reason);
            }
        }
        let Family {
            units,
            traits,
            experience,
            ..
        } = self;
        if !units.active() && !traits.active() && !experience.active() {
            return;
        }
        let pointer = character_pointer
            .checked_add(ccharacter::UNIT_LEADER)
            .and_then(|address| read::<u64>(&view, address))
            .and_then(|pointer| usize::try_from(pointer).ok());
        let Some(pointer) = pointer else {
            units.fail("character unit leader pointer unreadable");
            traits.fail("character unit leader pointer unreadable");
            experience.fail("character unit leader pointer unreadable");
            return;
        };
        if pointer == 0 {
            return;
        }
        let previous_len = units.len;
        units.append(
            |rows, bytes| read_unit(memory, base, pointer, character_id, rows, bytes),
            |row, offset| {
                relocate(&mut row.text_a_start, offset) && relocate(&mut row.text_b_start, offset)
            },
        );
        // Reuse the authenticated scalar header on the normal path. The
        // trait vectors retain independent availability when scalar capacity
        // or a skill/text fault requires reading only the parent's identity.
        let identity = if traits.active() || experience.active() {
            Some(if units.len > previous_len {
                Ok(units.rows[previous_len].native.id)
            } else {
                unit_identity(memory, pointer, character_id)
            })
        } else {
            None
        };
        match identity {
            Some(Ok(id)) => {
                traits.append(
                    |rows, bytes| {
                        if let Some(owned) = owned_traits {
                            owned.project(0, &mut trait_cursors[0], pointer, id, rows, bytes, |row, _| row)
                        } else {
                            cunit_leader::walk_traits(memory, base, pointer, id, rows, bytes).into()
                        }
                    },
                    |row, offset| relocate(&mut row.token_start, offset),
                );
                experience.append(
                    |rows, bytes| {
                        if let Some(owned) = owned_traits {
                            owned.project(
                                1,
                                &mut trait_cursors[1],
                                pointer,
                                id,
                                rows,
                                bytes,
                                |trait_row, experience| cunit_leader::TraitExperienceRow {
                                    trait_row,
                                    experience,
                                },
                            )
                        } else {
                            cunit_leader::walk_trait_experience(memory, base, pointer, id, rows, bytes)
                                .into()
                        }
                    },
                    |row, offset| relocate(&mut row.trait_row.token_start, offset),
                );
            }
            Some(Err(reason)) => {
                traits.fail(reason);
                experience.fail(reason);
            }
            None => {}
        }
    }
}

fn walk_family<M: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    mut family: Family<'_>,
    owned_traits: Option<&TraitCapture>,
    templates: Option<&TemplateCache>,
) -> [ReadResult; 6] {
    let required = family.required();
    let mut trait_cursors = [0; 2];
    let result = (|| {
        if !required.into_iter().any(|active| active) {
            return Ok(());
        }
        let roster = ccharacter_manager::Roster::read(memory, base, game_state)?;
        let mut cursor = roster.cursor();
        // Capture and owned decoding retain the same fixed-page protocol.
        let mut page = ccharacter_manager::ViewPage::default();
        while family.alive() {
            let next = loop {
                if let Some(view) = page.next() {
                    break Some(view?);
                }
                if !cursor.refill_page(memory, &mut page, required)? {
                    break None;
                }
            };
            let Some(view) = next else {
                break;
            };
            family.visit(memory, base, view, owned_traits, templates, &mut trait_cursors);
        }
        Ok::<_, &'static str>(())
    })();
    if let Err(reason) = result {
        family.fail(reason);
    }
    family.results()
}

/// `walk_family` over the occurrences a census-loaned acquisition visited.
/// Each view holds the same authenticated fields `View::read` would copy;
/// omitted occurrences produce no rows and read nothing in the typed walk.
fn walk_census<M: Memory>(
    memory: &M,
    base: usize,
    mut family: Family<'_>,
    owned_traits: Option<&TraitCapture>,
    visited: &[census::Occurrence],
    historical: usize,
) -> [ReadResult; 6] {
    let required = family.required();
    debug_assert!(!required[0], "the role census holds no character names");
    let mut trait_cursors = [0; 2];
    for occurrence in visited {
        if !family.alive() {
            break;
        }
        let identity = occurrence.identity(historical);
        match ccharacter::View::acquired(memory, base, identity, occurrence.fields(required)) {
            Ok(view) => family.visit(memory, base, view, owned_traits, None, &mut trait_cursors),
            Err(reason) => {
                family.fail(reason);
                break;
            }
        }
    }
    family.results()
}

fn unit_identity(
    memory: &impl Memory,
    pointer: usize,
    character: u32,
) -> Result<u32, &'static str> {
    let mut reference = [0; 8];
    let mut parent = [0; 8];
    let reference_address = pointer
        .checked_add(cunit_leader::TYPE)
        .ok_or("unit leader identity address overflow")?;
    let parent_address = pointer
        .checked_add(cunit_leader::CHARACTER_TYPE)
        .ok_or("unit leader parent address overflow")?;
    if memory.copy_scattered(
        [reference_address, parent_address],
        &mut [&mut reference, &mut parent],
    ) != 2
    {
        return Err("unit leader identity or parent unreadable");
    }
    if u32::from_le_bytes(reference[..4].try_into().unwrap()) != cunit_leader::REFERENCE_TYPE
        || u32::from_le_bytes(parent[..4].try_into().unwrap()) != ccharacter::REFERENCE_TYPE
        || u32::from_le_bytes(parent[4..].try_into().unwrap()) != character
    {
        return Err("unit leader native reference or character backlink mismatch");
    }
    Ok(u32::from_le_bytes(reference[4..].try_into().unwrap()))
}

fn read_unit(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    character: u32,
    output: &mut [LeaderRow],
    bytes: &mut [u8],
) -> ReadResult {
    let Some(output) = output.first_mut() else {
        return ReadResult::rows(Outcome::Truncated {
            len: 0,
            unread_rows: 1,
        });
    };
    let native = match cunit_leader::read(memory, base, pointer, character) {
        Ok(row) => row,
        Err(reason) => return ReadResult::rows(Outcome::Unavailable { reason }),
    };
    let text = cunit_leader::read_text(memory, pointer, bytes);
    if matches!(text.outcome, Outcome::Rows { len: 1 }) {
        *output = LeaderRow {
            native,
            // Native +20 is saved as name; +40 is a separate unsaved string.
            text_a_start: text.token_start,
            text_a_len: text.token_len,
            text_b_start: text.name_start,
            text_b_len: text.name_len,
        };
    }
    ReadResult {
        outcome: text.outcome,
        bytes_len: text.bytes_len,
        required_bytes: text.required_bytes,
    }
}

pub(super) fn walk_abilities(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut [active_ability_record::Row],
    bytes: &mut [u8],
) -> ReadResult {
    let mut output = Batch::new(rows, bytes);
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let address = country
                .pointer
                .checked_add(ccountry::ACTIVE_ABILITIES)
                .ok_or("country active ability array address overflow")?;
            output.append(
                |rows, bytes| {
                    active_ability_record::walk(memory, base, country.id, address, rows, bytes)
                        .into()
                },
                |row, offset| relocate(&mut row.token_start, offset),
            );
            if !output.active() {
                break;
            }
        }
        Ok::<_, &'static str>(())
    })();
    if let Err(reason) = result {
        output.fail(reason);
    }
    output.result()
}

pub(super) fn view<S: TableSchema, R>(
    buffer: &Buffer<R>,
    valid_row: impl Fn(&R, &[u8]) -> bool,
    convert: impl Fn(&R, &[u8]) -> S::Row,
) -> TableView {
    let len = match buffer.rows.outcome {
        Outcome::Rows { len } | Outcome::Truncated { len, .. } => len,
        Outcome::Unavailable { reason } => return unavailable(reason),
    };
    let Some(rows) = buffer.rows.rows.get(..len) else {
        return unavailable("character row count exceeds allocated buffer");
    };
    let Some(bytes) = buffer.bytes.first.get(..buffer.bytes.len) else {
        return unavailable("character byte count exceeds allocated arena");
    };
    if rows.iter().any(|row| !valid_row(row, bytes)) {
        return unavailable("character string lies outside the captured byte arena");
    }
    table::<S, _>(buffer.rows.outcome, rows, |row| convert(row, bytes))
}

pub(super) fn valid(bytes: &[u8], start: usize, len: usize) -> bool {
    start
        .checked_add(len)
        .is_some_and(|end| bytes.get(start..end).is_some())
}

/// Used only after `view` validates every range for this table.
fn owned(bytes: &[u8], start: usize, len: usize) -> Vec<u8> {
    bytes[start..start + len].to_vec()
}

fn leader_reference(id: u32) -> u64 {
    (u64::from(id) << 32) | u64::from(cunit_leader::REFERENCE_TYPE)
}

fn leader_row(row: &LeaderRow, bytes: &[u8]) -> UnitLeaderRow {
    let native = row.native;
    UnitLeaderRow {
        reference_type: native.reference_type,
        id: native.id,
        character_type: native.character_type,
        character_id: native.character_id,
        category: native.category,
        experience: native.experience,
        kind: native.kind,
        skill_status: native.skill_status,
        skill_valid: native.skill_valid,
        skill_level: native.skill_level,
        skill_required_experience: native.skill_required_experience,
        skill_category: native.skill_category,
        components: native.components,
        inherited_text_a: owned(bytes, row.text_a_start, row.text_a_len),
        inherited_text_b: owned(bytes, row.text_b_start, row.text_b_len),
        owner: native.owner,
        captured: native.captured,
        visibility_choice: native.visibility_choice,
        template_visibility: native.template_visibility,
    }
}

pub(super) fn ability_row(row: &active_ability_record::Row, bytes: &[u8]) -> ActiveAbilityRow {
    ActiveAbilityRow {
        country: row.country,
        ordinal: row.ordinal,
        scope_country: row.scope_country,
        token: owned(bytes, row.token_start, row.token_len),
        caster: row.caster,
        strategic_region: row.strategic_region,
        division_count: row.division_count,
        activation_hour: row.activation_hour,
        phase: row.phase,
        allocate_cost: row.allocate_cost,
        allocated_command_power: row.allocated_command_power,
        duration_hours: row.duration_hours,
        cooldown_hours: row.cooldown_hours,
        target_kind: row.target_kind,
    }
}

// ===========================================================================
// Raw-copy split for hourly Characters/UnitLeaders and scheduled roles,
// traits and trait experience. ActiveAbilities is its own hourly pool pack
// (`super::active_abilities`).
//
// The game thread copies raw bytes only: the manager census, the per-entry
// view spans, heap string payloads by header length, role-map nodes with
// batched gathers, and unit/trait/experience nesting. Trait definitions are
// owned once per successful current-pass observation.
// The FIFO decode worker replays `walk_family` against the
// recorded spans and projects owned trait occurrences into the typed arenas.
// Native field values, order, truncation and failure precedence are retained.
//
// Read-set rules the mirror follows (repeated address/length observations
// retain their order, including failed reads):
// - reads served locally by `ccharacter::View` need no log entry beyond the
//   seven view spans themselves;
// - trait row and byte capacities bound the acquired prefix; other tables
//   record a superset and truncate against the pooled buffers during replay;
// - on any failed span the mirror aborts that sub-path exactly where the
//   typed walk would fail it, and failed batch flags (`Dead`) skip later
//   sub-copies exactly where `Batch::append` would skip the typed walk;
// - shared view bytes are parsed from the current copy only.
// ===========================================================================

/// Fault-attribution bits, one per family table in `TABLES[0..6]` order.
const MASK_CHARACTERS: u32 = 0x01;
const MASK_LEADERS: u32 = 0x02;
const MASK_ADVISORS: u32 = 0x04;
const MASK_UNITS: u32 = 0x08;
const MASK_TRAITS: u32 = 0x10;
const MASK_EXPERIENCE: u32 = 0x20;
const MASK_ALL: u32 = 0x3f;
const MASK_COUNTRY: u32 = MASK_CHARACTERS | MASK_LEADERS | MASK_ADVISORS;
const MASK_UNIT_LINK: u32 = MASK_UNITS | MASK_TRAITS | MASK_EXPERIENCE;
/// Identity-fallback spans feed only the trait vectors: the units outcome is
/// already decided when the typed fallback runs. The overlapping identity
/// span itself already attributes units through `MASK_UNIT_LINK`.
const MASK_TRAIT_FALLBACK: u32 = MASK_TRAITS | MASK_EXPERIENCE;

/// View span offsets, lengths and fault masks in `View::read` order:
/// identity, country, name token id and template pointer, name/token headers,
/// leader map head, unit leader pointer, advisor map head.
const VIEW_OFFSETS: [usize; 7] = [
    0,
    ccharacter::COUNTRY,
    ccharacter::NAME_TOKEN_ID,
    ccharacter::NAME,
    ccharacter::LEADER_MAP_HEAD,
    ccharacter::UNIT_LEADER,
    ccharacter::ADVISOR_MAP_HEAD,
];
const VIEW_LENS: [usize; 7] = [16, 4, ccharacter::NAME_TOKEN_SPAN, 64, 16, 8, 16];
const VIEW_MASKS: [u32; 7] = [
    MASK_ALL,
    MASK_COUNTRY,
    MASK_CHARACTERS,
    MASK_CHARACTERS,
    MASK_LEADERS,
    MASK_UNIT_LINK,
    MASK_ADVISORS,
];

/// Roster pointer pages are read in 32-entry chunks, exactly like the typed
/// cursor, so the replay requests the identical page spans.
const ROSTER_PAGE: u32 = 32;

#[derive(Debug, Clone)]
struct TraitDefinition {
    address: usize,
    identity: usize,
    ordinal: usize,
    literal: usize,
    source: usize,
    len: usize,
    inline: bool,
    token: usize,
    terminated: bool,
    copied: bool,
    error: Option<&'static str>,
}

/// Each entry owns exact native observations. Only successful definitions enter
/// the address index, which is emptied before every pass. The owned log
/// supplies every definition byte consumed by the decode worker.
#[derive(Debug, Default)]
struct TraitDefinitions {
    entries: Vec<TraitDefinition>,
    by_address: std::collections::HashMap<usize, usize>,
    log: crate::raw::RawLog<2>,
    overflowed: bool,
    charge: crate::raw_memory::Charge,
}

impl TraitDefinitions {
    fn new() -> Self {
        let mut value = Self {
            entries: Vec::with_capacity(4096),
            by_address: std::collections::HashMap::with_capacity(4096),
            log: crate::raw::RawLog::new(32768, 1 << 20),
            ..Self::default()
        };
        value.update_charge();
        value
    }

    fn update_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.charge.set(Capacity {
            payload_vectors: vector_bytes(&self.entries),
            index_map_entries_lower_bound: (self.by_address.capacity()
                * std::mem::size_of::<(usize, usize)>())
                as u64,
            ..Capacity::default()
        });
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.by_address.clear();
        self.log.clear();
        self.overflowed = false;
    }

    fn grow_for_next(&self) -> bool {
        self.overflowed || self.log.grow_for_next()
    }

    fn overflowed(&self) -> bool {
        self.overflowed || self.log.overflowed
    }

    fn grow(&mut self) {
        if self.overflowed {
            crate::raw::double_capacity(&mut self.entries);
            self.by_address.reserve(
                self.entries
                    .capacity()
                    .saturating_sub(self.by_address.len()),
            );
            self.update_charge();
        }
        self.log.grow();
    }

    fn room(&mut self) -> Result<(), &'static str> {
        if self.entries.len() == self.entries.capacity() {
            self.overflowed = true;
            Err("character trait definition staging capacity exhausted")
        } else {
            Ok(())
        }
    }

    fn header(
        &mut self,
        memory: &impl Memory,
        base: usize,
        address: usize,
        table: usize,
    ) -> Result<usize, &'static str> {
        if let Some(&index) = self.by_address.get(&address) {
            return Ok(index);
        }
        self.room()?;
        let mut definition = TraitDefinition {
            address,
            identity: 0,
            ordinal: 0,
            literal: 0,
            source: 0,
            len: 0,
            inline: false,
            token: 0,
            terminated: false,
            copied: false,
            error: None,
        };
        definition.error = (|| {
            let at = |offset| {
                address
                    .checked_add(offset)
                    .ok_or("unit leader address overflow")
            };
            let expected = base
                .checked_add(cunit_leader_trait::VTABLE_RVA)
                .ok_or("unit leader address overflow")?;
            let (starts, completed) = self
                .log
                .take_scattered_prefetched(
                    memory,
                    [
                        address,
                        at(cunit_leader_trait::ORDINAL)?,
                        at(cunit_leader_trait::TOKEN)?,
                    ],
                    [12, 4, 32],
                    [1 << table; 3],
                )
                .ok_or("character trait definition staging capacity exhausted")?;
            definition.identity = starts[0];
            definition.ordinal = starts[1];
            definition.literal = starts[2];
            if completed == 0 {
                return Err("unit leader trait identity unreadable");
            }
            if u64::from_le_bytes(self.log.bytes[starts[0]..starts[0] + 8].try_into().unwrap())
                != expected as u64
            {
                return Err("unit leader trait vtable mismatch");
            }
            if completed != 3 {
                return Err("unit leader trait identity unreadable");
            }
            let header = &self.log.bytes[starts[2]..starts[2] + 32];
            let len = u64::from_le_bytes(header[16..24].try_into().unwrap());
            let capacity = u64::from_le_bytes(header[24..].try_into().unwrap());
            if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
                return Err("unit leader string header invalid");
            }
            definition.len =
                usize::try_from(len).map_err(|_| "unit leader string length overflow")?;
            definition.inline = capacity == msvc_string::INLINE_CAPACITY;
            definition.source = if definition.inline {
                at(cunit_leader_trait::TOKEN)?
            } else {
                usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
                    .ok()
                    .filter(|&pointer| pointer != 0)
                    .ok_or("unit leader string pointer null or invalid")?
            };
            Ok(())
        })()
        .err();
        let index = self.entries.len();
        self.entries.push(definition);
        Ok(index)
    }

    fn result(&self, index: usize) -> Result<(), &'static str> {
        self.entries[index].error.map_or(Ok(()), Err)
    }

    fn token_len(&self, index: usize) -> usize {
        self.entries[index].len
    }

    /// A byte-limited consumer witnesses only the terminator. Upgrading that
    /// observation creates a separate entry, so a payload fault cannot alter an
    /// earlier successful truncated observation in the other table.
    fn consume(
        &mut self,
        memory: &impl Memory,
        mut index: usize,
        table: usize,
        fits: bool,
    ) -> Result<usize, &'static str> {
        if self.entries[index].error.is_some() {
            return Ok(index);
        }
        if self.entries[index].terminated && (!fits || self.entries[index].copied) {
            return Ok(index);
        }
        if self.entries[index].terminated {
            self.room()?;
            let upgraded = self.entries[index].clone();
            index = self.entries.len();
            self.entries.push(upgraded);
        }
        let definition = &mut self.entries[index];
        definition.error = (|| {
            if definition.inline {
                if self.log.bytes[definition.literal + definition.len] != 0 {
                    return Err("unit leader string terminator invalid or unreadable");
                }
                definition.token = definition.literal;
            } else {
                let terminator = definition
                    .source
                    .checked_add(definition.len)
                    .ok_or("unit leader address overflow")?;
                if fits {
                    let (starts, completed) = self
                        .log
                        .take_scattered(
                            memory,
                            [terminator, definition.source],
                            [1, definition.len],
                            [1 << table; 2],
                        )
                        .ok_or("character trait definition staging capacity exhausted")?;
                    if completed == 0 || self.log.bytes[starts[0]] != 0 {
                        return Err("unit leader string terminator invalid or unreadable");
                    }
                    if completed != 2 {
                        return Err("unit leader trait token bytes unreadable");
                    }
                    definition.token = starts[1];
                } else {
                    let terminated = self
                        .log
                        .take(memory, terminator, 1, 1 << table)
                        .is_some_and(|start| self.log.bytes[start] == 0);
                    if !terminated {
                        return Err("unit leader string terminator invalid or unreadable");
                    }
                }
            }
            Ok(())
        })()
        .err();
        if definition.error.is_none() {
            definition.terminated = true;
            definition.copied = fits;
            if !self.by_address.contains_key(&definition.address)
                && self.by_address.len() == self.by_address.capacity()
            {
                self.overflowed = true;
                return Err("character trait definition index capacity exhausted");
            }
            self.by_address.insert(definition.address, index);
        }
        Ok(index)
    }

    fn project(&self, index: usize) -> (i32, i32, &[u8]) {
        let definition = &self.entries[index];
        (
            i32::from_le_bytes(
                self.log.bytes[definition.identity + 8..definition.identity + 12]
                    .try_into()
                    .unwrap(),
            ),
            i32::from_le_bytes(
                self.log.bytes[definition.ordinal..definition.ordinal + 4]
                    .try_into()
                    .unwrap(),
            ),
            &self.log.bytes[definition.token..definition.token + definition.len],
        )
    }
}

#[derive(Debug)]
struct TraitOccurrence {
    entry: [u8; 16],
    ordinal: u32,
    definition: usize,
}

#[derive(Debug)]
struct TraitVector {
    unit: usize,
    leader: u32,
    start: usize,
    len: usize,
    outcome: Outcome,
    bytes_len: usize,
    required_bytes: usize,
}

/// Current-pass occurrences retain native order, while successful definitions
/// are shared only inside this owned image.
#[derive(Debug, Default)]
struct TraitCapture {
    definitions: TraitDefinitions,
    vectors: [Vec<TraitVector>; 2],
    occurrences: [Vec<TraitOccurrence>; 2],
    rows: [usize; 2],
    bytes: [usize; 2],
    used_rows: [usize; 2],
    used_bytes: [usize; 2],
    truncated: [bool; 2],
    fault: [bool; 2],
    raw_bytes: [u64; 2],
    overflowed: bool,
    charge: crate::raw_memory::Charge,
}

impl TraitCapture {
    fn new() -> Self {
        let mut value = Self {
            definitions: TraitDefinitions::new(),
            vectors: std::array::from_fn(|_| Vec::with_capacity(4096)),
            occurrences: [Vec::with_capacity(4096), Vec::with_capacity(32768)],
            ..Self::default()
        };
        value.update_charge();
        value
    }

    fn update_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.charge.set(Capacity {
            payload_vectors: self.vectors.iter().map(vector_bytes).sum::<u64>()
                + self.occurrences.iter().map(vector_bytes).sum::<u64>(),
            ..Capacity::default()
        });
    }

    fn clear(&mut self) {
        self.definitions.clear();
        for vector in &mut self.vectors {
            vector.clear();
        }
        for occurrences in &mut self.occurrences {
            occurrences.clear();
        }
        self.used_rows = [0; 2];
        self.used_bytes = [0; 2];
        self.truncated = [false; 2];
        self.fault = [false; 2];
        self.raw_bytes = [0; 2];
        self.overflowed = false;
    }

    fn grow_for_next(&self) -> bool {
        self.overflowed || self.definitions.grow_for_next()
    }

    fn overflowed(&self) -> bool {
        self.overflowed || self.definitions.overflowed()
    }

    fn grow(&mut self) {
        if self.overflowed {
            for vector in &mut self.vectors {
                crate::raw::double_capacity(vector);
            }
            for occurrences in &mut self.occurrences {
                crate::raw::double_capacity(occurrences);
            }
            self.update_charge();
        }
        self.definitions.grow();
    }

    fn begin(&mut self, context: CharactersCopyContext) {
        self.clear();
        self.rows = context.trait_rows;
        self.bytes = context.trait_bytes;
    }

    fn acquire(
        &mut self,
        memory: &impl Memory,
        base: usize,
        params: TraitVectorCopy,
    ) -> Result<(), &'static str> {
        let table = usize::from(params.has_experience);
        if self.vectors[table].len() == self.vectors[table].capacity() {
            self.overflowed = true;
            return Err("character trait vector staging capacity exhausted");
        }
        let mut vector = TraitVector {
            unit: params.unit,
            leader: params.leader_id,
            start: self.occurrences[table].len(),
            len: 0,
            outcome: Outcome::Rows { len: 0 },
            bytes_len: 0,
            required_bytes: 0,
        };
        let result = (|| -> Result<(), &'static str> {
            let identity = read::<[u8; 16]>(memory, params.unit).ok_or_else(|| {
                self.fault[table] = true;
                "unit leader identity unreadable"
            })?;
            self.raw_bytes[table] += 16;
            let relative = usize::try_from(u64::from_le_bytes(identity[..8].try_into().unwrap()))
                .ok()
                .and_then(|value| value.checked_sub(base));
            if !matches!(
                relative,
                Some(
                    cunit_leader::VTABLE_RVA
                        | carmy_leader::VTABLE_RVA
                        | cnavy_leader::VTABLE_RVA
                        | coperative_leader::VTABLE_RVA
                )
            ) {
                return Err("unit leader vtable mismatch");
            }
            if u32::from_le_bytes(identity[8..12].try_into().unwrap())
                != cunit_leader::REFERENCE_TYPE
            {
                return Err("unit leader reference type mismatch");
            }
            if u32::from_le_bytes(identity[12..].try_into().unwrap()) != params.leader_id {
                return Err("unit leader trait parent identity mismatch");
            }
            let at = params
                .unit
                .checked_add(params.array_offset)
                .ok_or("unit leader address overflow")?;
            let header = read::<[u8; 16]>(memory, at);
            if header.is_none() {
                self.fault[table] = true;
            } else {
                self.raw_bytes[table] += 16;
            }
            let array = header
                .and_then(|header| Array::from_header(header, params.stride))
                .ok_or("unit leader trait vector invalid or unreadable")?;
            base.checked_add(cunit_leader_trait::VTABLE_RVA)
                .ok_or("unit leader address overflow")?;
            let row_limit = if self.truncated[table] {
                0
            } else {
                self.rows[table].saturating_sub(self.used_rows[table])
            };
            let byte_limit = if self.truncated[table] {
                0
            } else {
                self.bytes[table].saturating_sub(self.used_bytes[table])
            };
            let mut unread = 0;
            for ordinal in 0..array.count {
                if vector.len == row_limit {
                    unread = array.count - ordinal;
                    break;
                }
                if self.occurrences[table].len() == self.occurrences[table].capacity() {
                    self.overflowed = true;
                    return Err("character trait occurrence staging capacity exhausted");
                }
                let address = array
                    .element(ordinal, params.stride)
                    .ok_or("unit leader trait entry address overflow")?;
                let mut entry = [0u8; 16];
                if !memory.copy(address, &mut entry[..params.stride]) {
                    self.fault[table] = true;
                    return Err(if params.has_experience {
                        "unit leader trait experience entry unreadable"
                    } else {
                        "unit leader trait pointer unreadable"
                    });
                }
                self.raw_bytes[table] += params.stride as u64;
                let address = usize::try_from(u64::from_le_bytes(entry[..8].try_into().unwrap()))
                    .map_err(|_| "unit leader trait pointer overflow")?;
                let definition = self.definitions.header(memory, base, address, table)?;
                self.definitions.result(definition)?;
                let len = self.definitions.token_len(definition);
                vector.required_bytes = vector
                    .required_bytes
                    .checked_add(len)
                    .ok_or("unit leader trait byte count overflow")?;
                let fits = len <= byte_limit - vector.bytes_len;
                let definition = self.definitions.consume(memory, definition, table, fits)?;
                self.definitions.result(definition)?;
                if !fits {
                    unread = array.count - ordinal;
                    break;
                }
                self.occurrences[table].push(TraitOccurrence {
                    entry,
                    ordinal,
                    definition,
                });
                vector.len += 1;
                vector.bytes_len += len;
            }
            vector.outcome = if unread == 0 {
                Outcome::Rows { len: vector.len }
            } else {
                Outcome::Truncated {
                    len: vector.len,
                    unread_rows: unread,
                }
            };
            self.used_rows[table] += vector.len;
            self.used_bytes[table] += vector.bytes_len;
            self.truncated[table] |= unread != 0;
            Ok(())
        })();
        if let Err(reason) = result {
            vector.outcome = Outcome::Unavailable { reason };
        }
        self.vectors[table].push(vector);
        result
    }

    fn finish(&mut self) {
        for table in 0..2 {
            self.fault[table] |= self.definitions.log.fault[table];
            self.raw_bytes[table] += self.definitions.log.raw_bytes[table];
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn project<R>(
        &self,
        table: usize,
        cursor: &mut usize,
        unit: usize,
        leader: u32,
        rows: &mut [R],
        bytes: &mut [u8],
        make: impl Fn(cunit_leader::TraitRow, i64) -> R,
    ) -> ReadResult {
        let Some(vector) = self.vectors[table].get(*cursor) else {
            return ReadResult::rows(Outcome::Unavailable {
                reason: "character trait owned vector missing",
            });
        };
        *cursor += 1;
        if vector.unit != unit || vector.leader != leader {
            return ReadResult::rows(Outcome::Unavailable {
                reason: "character trait owned parent mismatch",
            });
        }
        if rows.len() < vector.len || bytes.len() < vector.bytes_len {
            return ReadResult::rows(Outcome::Unavailable {
                reason: "character trait owned projection capacity mismatch",
            });
        }
        let mut start = 0;
        for (output, occurrence) in rows
            .iter_mut()
            .zip(&self.occurrences[table][vector.start..vector.start + vector.len])
        {
            let (token_id, definition_ordinal, token) =
                self.definitions.project(occurrence.definition);
            bytes[start..start + token.len()].copy_from_slice(token);
            *output = make(
                cunit_leader::TraitRow {
                    leader_id: leader,
                    ordinal: occurrence.ordinal,
                    token_id,
                    definition_ordinal,
                    token_start: start,
                    token_len: token.len(),
                },
                i64::from_le_bytes(occurrence.entry[8..].try_into().unwrap()),
            );
            start += token.len();
        }
        if let Outcome::Unavailable { .. } = vector.outcome {
            return ReadResult::rows(vector.outcome);
        }
        ReadResult {
            outcome: vector.outcome,
            bytes_len: vector.bytes_len,
            required_bytes: vector.required_bytes,
        }
    }
}

/// One refresh of raw character payload. Trait occurrences reference owned,
/// verified definitions; all other observations retain the raw replay order.
#[derive(Debug, Default)]
pub(crate) struct CharactersJob {
    pub part: usize,
    pub log: crate::raw::RawLog<6>,
    pub game_state: usize,
    pub active: [bool; 6],
    traits: TraitCapture,
    advisor_demand: AdvisorDemand,
    skills: [SkillCopy; 32],
    portrait_census: Option<Arc<ccharacter_manager::PortraitCensus>>,
    census_current: bool,
    census_matches_plan: bool,
    source_bindings: Option<CharacterSourceBindings>,
    // Advisor-only jobs authenticate both headers once, then acquire the two
    // rosters sequentially with independent fresh-read programs. The child has
    // no child of its own; neither root authentication nor values are cached.
    advisor_roots: crate::raw::RawLog<6>,
    advisor_dynamic: Option<Box<CharactersJob>>,
    advisor_cache: Option<advisors::Cache>,
    advisor_section_complete: bool,
    census_charge: crate::raw_memory::Charge,
    // The roster job gathers the role census on role hours. It persists
    // like the portrait census; the flag admits only this pass's gather.
    role_census: Option<Arc<census::RoleCensus>>,
    role_census_current: bool,
    /// Historical roster length of the retained portrait census, or of the
    /// loaned census this payload was acquired from.
    census_historical: usize,
    /// A role payload acquired from a loaned census, the occurrences its
    /// decode visits, and the demand recorded when that storage ran out.
    census_capture: bool,
    census_visited: Vec<census::Occurrence>,
    census_required: usize,
    /// Template identities this job's earlier decodes read. It survives
    /// `clear`: the copy consults it, the decode rebuilds it.
    templates: TemplateCache,
    template_charge: crate::raw_memory::Charge,
}

/// Auxiliary bindings whose selecting character/unit bytes are controls in the
/// raw read program. Only addresses and capacity demand survive a refresh;
/// trait vectors, definitions and experience are acquired on every refresh.
#[derive(Debug)]
struct CharacterSourceBindings {
    sources: Vec<TraitVectorCopy>,
    overflowed: bool,
    ready: bool,
    rows: [usize; 2],
    bytes: [usize; 2],
    advisor: AdvisorDemand,
    advisor_section_complete: bool,
    charge: crate::raw_memory::Charge,
}

impl CharacterSourceBindings {
    fn new() -> Self {
        let mut value = Self {
            sources: Vec::with_capacity(8192),
            overflowed: false,
            ready: false,
            rows: [0; 2],
            bytes: [0; 2],
            advisor: AdvisorDemand::default(),
            advisor_section_complete: false,
            charge: crate::raw_memory::Charge::default(),
        };
        value.update_charge();
        value
    }

    fn update_charge(&mut self) {
        self.charge.set(crate::raw_memory::Capacity {
            auxiliary_vectors: crate::raw_memory::vector_bytes(&self.sources),
            ..Default::default()
        });
    }

    fn matches(&self, context: CharactersCopyContext) -> bool {
        self.ready && self.rows == context.trait_rows && self.bytes == context.trait_bytes
    }

    fn begin(&mut self, context: CharactersCopyContext) {
        self.sources.clear();
        self.overflowed = false;
        self.ready = false;
        self.rows = context.trait_rows;
        self.bytes = context.trait_bytes;
    }

    fn remember(&mut self, source: TraitVectorCopy) {
        if self.sources.len() == self.sources.capacity() {
            self.overflowed = true;
        } else {
            self.sources.push(source);
        }
    }

    fn grow(&mut self) {
        if self.overflowed {
            crate::raw::double_capacity(&mut self.sources);
            self.update_charge();
        }
    }
}

/// Successful shared skill definitions, retained only for this acquisition.
/// CUnitLeader::add_experience (140c05400) selects definitions from the
/// category/level database (140adace0); it changes the leader's selection.
/// Collisions simply acquire again. Every use still appends its scoped raw
/// observations so unrelated aliases and their faults keep separate cursors.
#[derive(Debug, Default, Clone, Copy)]
struct SkillCopy {
    pointer: usize,
    bytes: [u8; 21],
}

impl Memory for SkillCopy {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let source = match (address.checked_sub(self.pointer), output.len()) {
            (Some(0), 8) => &self.bytes[..8],
            (Some(cunit_leader_skill::VALID), 1) => &self.bytes[8..9],
            (Some(cunit_leader_skill::LEVEL), 12) => &self.bytes[9..],
            _ => return false,
        };
        output.copy_from_slice(source);
        true
    }

    fn copy_scattered_prefetched<const M: usize>(
        &self,
        addresses: [usize; M],
        outputs: &mut [&mut [u8]; M],
    ) -> usize {
        // Owned bytes need no native-address prefetch.
        self.copy_scattered(addresses, outputs)
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct AdvisorDemand {
    rows: usize,
    bytes: usize,
    overflowed: bool,
}

impl CharactersJob {
    pub(crate) fn has_storage(&self) -> bool {
        self.advisor_cache.is_some() || self.log.spans.capacity() != 0
    }
    fn source_mask(&self) -> u32 {
        self.active
            .iter()
            .enumerate()
            .fold(0, |mask, (index, &active)| {
                mask | (u32::from(active) << index)
            })
    }
    #[cfg(test)]
    pub fn new() -> Self {
        Self::with_storage(0, 1 << 19, 24 << 20, true)
    }

    fn with_storage(part: usize, spans: usize, bytes: usize, has_traits: bool) -> Self {
        let mut log = crate::raw::RawLog::new(spans, bytes);
        let trace_capacity = spans.min(1 << 18);
        log.initialize_scattered_read_plan(trace_capacity, trace_capacity, bytes.min(4 << 20));
        let census = (part == 0).then(|| ccharacter_manager::PortraitCensus::new(ccharacter_manager::PortraitCensus::INITIAL_CAPACITY));
        let role_census = (part == 0).then(|| census::RoleCensus::new(census::RoleCensus::INITIAL_CAPACITY));
        // A pass visits at most every census occurrence, so the list is seeded
        // at the live census length (none before the first census) and
        // follows it off-thread.
        let census_visited = Vec::with_capacity(if part == 0 { 0 } else { census::live_len() });
        let mut census_charge = crate::raw_memory::Charge::default();
        census_charge.set(crate::raw_memory::Capacity {
            auxiliary_vectors: census.as_ref().map_or(0, |census| census.allocated_bytes() as u64)
                + role_census.as_ref().map_or(0, |census| census.allocated_bytes() as u64)
                + crate::raw_memory::vector_bytes(&census_visited),
            ..Default::default()
        });
        Self {
            part,
            log,
            game_state: 0,
            active: [true; 6],
            traits: if has_traits { TraitCapture::new() } else { TraitCapture::default() },
            advisor_demand: AdvisorDemand::default(),
            skills: [SkillCopy::default(); 32],
            portrait_census: census.map(Arc::new),
            census_current: false,
            census_matches_plan: false,
            source_bindings: Some(CharacterSourceBindings::new()),
            advisor_roots: crate::raw::RawLog::default(),
            advisor_dynamic: None,
            advisor_cache: None,
            advisor_section_complete: false,
            census_charge,
            role_census: role_census.map(Arc::new),
            role_census_current: false,
            census_historical: 0,
            census_capture: false,
            census_visited,
            census_required: 0,
            templates: TemplateCache::default(),
            template_charge: crate::raw_memory::Charge::default(),
        }
    }

    pub fn new_part(part: usize) -> Self {
        if part == 2 && cfg!(feature = "advisor_changes") {
            return Self { part, advisor_cache: Some(advisors::Cache::new()), ..Default::default() };
        }
        // Each owner stages only its own tables. These are starting capacities,
        // not capture limits: the existing fallback and off-thread growth handle
        // larger saves. Non-trait owners need no trait-definition/occurrence pools.
        let (spans, bytes) = match part {
            0 | 2 => (1 << 18, 4 << 20),
            1 | 3 => (1 << 17, 2 << 20),
            _ => unreachable!("invalid character part"),
        };
        let mask = PART_MASKS[part];
        let mut job = Self::with_storage(part, spans, bytes, mask[4] || mask[5]);
        if part == 2 {
            job.advisor_roots = crate::raw::RawLog::new(4, 48);
            job.advisor_dynamic = Some(Box::new(Self::with_storage(part, 1 << 15, 512 << 10, false)));
        }
        job
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.active = [false; 6];
        self.traits.clear();
        self.advisor_demand = AdvisorDemand::default();
        self.skills.fill(SkillCopy::default());
        self.census_current = false;
        self.role_census_current = false;
        self.census_capture = false;
        self.census_visited.clear();
        self.census_required = 0;
        self.advisor_section_complete = false;
        self.advisor_roots.clear();
        if let Some(dynamic) = &mut self.advisor_dynamic { dynamic.clear(); }
    }

    pub(crate) fn portrait_census(&self) -> Option<Arc<ccharacter_manager::PortraitCensus>> {
        self.census_current.then(|| self.portrait_census.as_ref().map(Arc::clone)).flatten()
    }

    /// This pass's role census; role jobs return the loan at their join.
    pub(crate) fn role_census(&self) -> Option<Arc<census::RoleCensus>> {
        self.role_census_current.then(|| self.role_census.as_ref().map(Arc::clone)).flatten()
    }

    fn update_census_charge(&mut self) {
        self.census_charge.set(crate::raw_memory::Capacity {
            auxiliary_vectors: self.portrait_census.as_ref().map_or(0, |census| census.allocated_bytes() as u64)
                + self.role_census.as_ref().map_or(0, |census| census.allocated_bytes() as u64)
                + crate::raw_memory::vector_bytes(&self.census_visited),
            ..Default::default()
        });
    }

    pub fn grow_for_next(&self) -> bool {
        self.advisor_cache.as_ref().is_some_and(advisors::Cache::needs_growth)
            || self.log.grow_for_next() || self.traits.grow_for_next()
            || self.portrait_census.as_ref().is_some_and(|census| census.needs_growth())
            || self.source_bindings.as_ref().is_some_and(|plan| plan.overflowed)
            || self.advisor_dynamic.as_ref().is_some_and(|job| job.grow_for_next())
            || self.role_census.as_ref().is_some_and(|census| census.needs_growth())
            || self.visits_required(census::live_len()) > self.census_visited.capacity()
    }

    /// Visits the next role pass may need: its recorded demand, and every
    /// occurrence of the live census (`live`).
    fn visits_required(&self, live: usize) -> usize {
        if self.part == 0 {
            return 0;
        }
        self.census_required.max(live)
    }

    fn overflowed(&self) -> bool {
        self.log.overflowed || self.traits.overflowed() || self.advisor_roots.overflowed
            || self.advisor_dynamic.as_ref().is_some_and(|job| job.overflowed())
            || self.census_required > self.census_visited.capacity()
    }

    pub fn grow(&mut self) {
        // Every portrait lease returns before recycle. The grown census was
        // incomplete, so retire any prepared program: the next pass must
        // rediscover the roster to publish it.
        if let Some(census) = self.portrait_census.as_mut().and_then(Arc::get_mut)
            && census.needs_growth()
        {
            census.grow();
            self.log.invalidate_read_plan();
        }
        // Role loans also return before recycle.
        if let Some(census) = self.role_census.as_mut().and_then(Arc::get_mut) {
            census.grow();
        }
        self.census_visited
            .reserve(self.visits_required(census::live_len()).saturating_sub(self.census_visited.len()));
        self.update_census_charge();
        if let Some(cache) = &mut self.advisor_cache { cache.grow(); }
        self.log.grow();
        self.traits.grow();
        if let Some(plan) = &mut self.source_bindings { plan.grow(); }
        if let Some(dynamic) = &mut self.advisor_dynamic { dynamic.grow(); }
    }

    pub(crate) fn prepare_read_plans(&mut self) {
        if let Some(cache) = &mut self.advisor_cache { cache.prepare(); }
        self.log.prepare_read_plan();
        if let Some(dynamic) = &mut self.advisor_dynamic { dynamic.prepare_read_plans(); }
    }

    pub(crate) fn note_growth(&self) {
        crate::raw::note_growth("characters", &self.log);
        if let Some(dynamic) = &self.advisor_dynamic { dynamic.note_growth(); }
    }

    fn requires_no_retry(&self) -> bool {
        let mut observed = self.log.planned_source_observed();
        let mut failed = self.log.planned_capture_requires_no_retry();
        if let Some(dynamic) = &self.advisor_dynamic {
            observed |= dynamic.log.planned_source_observed();
            failed |= dynamic.requires_no_retry();
        }
        failed || (self.overflowed() && observed)
    }

    fn fate(&self) -> TreeFate {
        if self.log.overflowed {
            TreeFate::Abort
        } else {
            TreeFate::Dead
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct CharactersCopyReport {
    /// False when staging overflowed: the caller falls back to the typed walk
    /// (identical rows through the existing path) and enqueues no payload.
    pub raw_ok: bool,
    /// A stopped prepared pass may not fall back to another native capture.
    pub plan_failed: bool,
    pub copy_ticks: [u64; 6],
    pub raw_bytes: [u64; 6],
}

/// Acquisition parameters, bundled to keep the refresh entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CharactersCopyContext {
    pub base: usize,
    pub game_state: usize,
    /// A full hour reads every character template again.
    pub force_full: bool,
    pub active: [bool; 6],
    pub trait_rows: [usize; 2],
    pub trait_bytes: [usize; 2],
}

/// Per-batch failure flags mirroring `Batch::active`: once a typed batch
/// fails, its later appends are skipped without reads, and so are the
/// mirror's later sub-copies.
#[derive(Debug, Default)]
struct Dead {
    characters: bool,
    leaders: bool,
    advisors: bool,
    units: bool,
    traits: bool,
    experience: bool,
}

/// Successful view fields remain in the current capture's append-only raw
/// arena. Each roster occurrence owns its offsets; aliases never share them.
#[derive(Clone, Copy)]
struct ViewCopy {
    starts: [u32; 7],
    unit_id: Option<u32>,
}

impl Default for ViewCopy {
    fn default() -> Self {
        Self {
            starts: [u32::MAX; 7],
            unit_id: None,
        }
    }
}

impl ViewCopy {
    /// Failed and inactive spans retain the old zero-field semantics. Borrow
    /// only while parsing; later raw appends may move the arena's allocation.
    #[inline]
    fn field<'a>(&self, job: &'a CharactersJob, index: usize) -> &'a [u8] {
        static ZERO: [u8; 64] = [0; 64];
        let len = VIEW_LENS[index];
        let start = self.starts[index];
        if start == u32::MAX {
            &ZERO[..len]
        } else {
            let start = start as usize;
            &job.log.bytes[start..start + len]
        }
    }
}

/// Mirror of `View::read`'s scatter plus single-resume sequence with no
/// decoding. Returns `None` only when the identity span fails, which aborts
/// the whole copy exactly like the typed cursor error.
fn copy_view(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Option<ViewCopy> {
    let mut view = ViewCopy::default();
    let active = job.source_mask();
    let masks = VIEW_MASKS.map(|mask| mask & active);
    let lengths = std::array::from_fn(|index| {
        if masks[index] != 0 {
            VIEW_LENS[index]
        } else {
            0
        }
    });
    let (starts, completed) = job.log.take_scattered(
        memory,
        VIEW_OFFSETS.map(|offset| pointer.checked_add(offset).unwrap_or(0)),
        lengths,
        masks,
    )?;
    for index in 0..completed {
        let (start, len) = (starts[index], lengths[index]);
        if len != 0 {
            // RawLog admits start + len <= u32::MAX; a successful nonempty
            // field's start therefore cannot equal the absent marker.
            view.starts[index] = start as u32;
        }
    }
    if completed == 0 {
        return None;
    }
    // Resume only later independent fields, each at most once; the failed
    // field is never retried.
    for index in completed + 1..7 {
        if lengths[index] == 0 {
            continue;
        }
        let addr = pointer.checked_add(VIEW_OFFSETS[index]).unwrap_or(0);
        let len = lengths[index];
        if let Some(start) = job.log.take(memory, addr, len, masks[index]) {
            view.starts[index] = start as u32;
        }
    }
    // Local vtable and reference-type authentication, checked after the
    // scatter exactly like the typed view.
    let expected = base.checked_add(ccharacter::VTABLE_RVA)? as u64;
    let identity = view.field(job, 0);
    if u64::from_le_bytes(identity[..8].try_into().unwrap()) != expected
        || u32::from_le_bytes(
            identity[ccharacter::TYPE..ccharacter::TYPE + 4]
                .try_into()
                .unwrap(),
        ) != ccharacter::REFERENCE_TYPE
    {
        return None;
    }
    Some(view)
}

/// Parsed native string header shared by both string domains: payload length,
/// payload source address and inline flag. `None` means the typed walk fails
/// the enclosing row with its own header reason from the same header bytes.
struct Lit {
    len: usize,
    source: usize,
    inline: bool,
}

/// Hint only the first line of a potential upcoming read. The guard is
/// advisory: rejecting a hint never changes the later guarded read or error.
#[inline]
fn hint_character_target(address: usize, len: usize, hint: &mut impl FnMut(usize, usize)) {
    if address != 0
        && address.checked_add(len).is_some()
        && crate::raw::is_canonical_range(address, len)
    {
        hint(address, 1);
    }
}

/// These pointers come only from the fresh, authenticated character view.
/// Targets remain unauthenticated until their original reads below. Empty
/// maps still read their sentinel; failed view fields contain only zeros.
#[inline]
fn hint_character_targets(
    job: &CharactersJob,
    view: &ViewCopy,
    dead: &Dead,
    mut hint: impl FnMut(usize, usize),
) {
    if !dead.characters {
        // The page's views are staged before its bodies. Hint heap names now,
        // so their independent cache misses overlap the remaining view reads.
        // Inline sources need no hint; their header address is immaterial here.
        let headers = view.field(job, 3);
        if let (Some(name), Some(token)) = (
            parse_character_literal(&headers[..32], 0),
            parse_character_literal(&headers[32..], 0),
        ) {
            hint_character_literal(&name, &mut hint);
            hint_character_literal(&token, &mut hint);
        }
    }
    for (field, active) in [(4, !dead.leaders), (6, !dead.advisors)] {
        if !active {
            continue;
        }
        let header = view.field(job, field);
        if u32::try_from(u64::from_le_bytes(header[8..16].try_into().unwrap())).is_ok()
            && let Ok(head) =
                usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
        {
            hint_character_target(head, 26, &mut hint);
        }
    }
    if !(dead.units && dead.traits && dead.experience)
        && let Ok(unit) =
            usize::try_from(u64::from_le_bytes(view.field(job, 5).try_into().unwrap()))
        && unit != 0 && unit.checked_add(16).is_some() && crate::raw::is_canonical_range(unit, 16)
    {
        hint_character_target(unit, 16, &mut hint);
        if !dead.units {
            // These fixed base fields are independent of the leader subclass.
            // Skill, XP and category occupy one small span. Do not follow
            // the skill pointer before its ordinary validation.
            for (offset, len) in [(cunit_leader::NAME, 32), (cunit_leader::SKILL, 32), (cunit_leader::CHARACTER_TYPE, 8)] {
                if let Some(address) = unit.checked_add(offset) { hint_character_target(address, len, &mut hint); }
            }
        }
    }
}

/// Called only after both local string parses succeeded. Inline bytes are
/// already owned. Heap strings need their terminator even when empty; hint
/// a separate payload-first line only when it differs from the terminator.
#[inline]
fn hint_character_literal(literal: &Lit, mut hint: impl FnMut(usize, usize)) {
    if literal.inline {
        return;
    }
    let Some(extent) = literal.len.checked_add(1) else {
        return;
    };
    if literal.source == 0
        || literal.source.checked_add(extent).is_none()
        || !crate::raw::is_canonical_range(literal.source, extent)
    {
        return;
    }
    let terminator = literal.source + literal.len;
    hint(terminator, 1);
    if literal.len != 0 && literal.source >> 6 != terminator >> 6 {
        hint(literal.source, 1);
    }
}

/// Mirror of `ccharacter::Literal::from_header` (character name and token).
fn parse_character_literal(header: &[u8], header_addr: usize) -> Option<Lit> {
    let len = u64::from_le_bytes(header[16..24].try_into().unwrap());
    let capacity = u64::from_le_bytes(header[24..32].try_into().unwrap());
    if capacity < msvc_string::INLINE_CAPACITY || len > capacity {
        return None;
    }
    let len = usize::try_from(len).ok()?;
    let inline = capacity == msvc_string::INLINE_CAPACITY;
    let source = if inline {
        header_addr
    } else {
        usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .ok()
            .filter(|&pointer| pointer != 0)?
    };
    if inline && header[len] != 0 {
        return None;
    }
    Some(Lit {
        len,
        source,
        inline,
    })
}

/// Mirror of `cunit_leader`'s string header validation: heap strings read
/// their terminator (and payload when nonempty) from live memory, inline
/// strings are served from the already-copied header.
fn copy_unit_literal(
    job: &mut CharactersJob,
    memory: &impl Memory,
    header: &[u8],
    mask: u32,
) -> Option<usize> {
    let len = u64::from_le_bytes(
        header[msvc_string::LENGTH..msvc_string::CAPACITY]
            .try_into()
            .unwrap(),
    );
    let capacity = u64::from_le_bytes(header[msvc_string::CAPACITY..].try_into().unwrap());
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return None;
    }
    let len = usize::try_from(len).ok()?;
    if capacity == msvc_string::INLINE_CAPACITY {
        if header[len] != 0 {
            return None;
        }
        return Some(len);
    }
    let source = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
        .ok()
        .filter(|&pointer| pointer != 0)?;
    copy_heap_literal_bytes(job, memory, source, len, mask).then_some(len)
}

/// Heap payload and terminator spans for one parsed character-domain string.
/// Inline strings need no live reads beyond the copied header.
fn copy_character_literal_bytes(
    job: &mut CharactersJob,
    memory: &impl Memory,
    literal: &Lit,
    mask: u32,
) -> bool {
    if literal.inline {
        return true;
    }
    copy_heap_literal_bytes(job, memory, literal.source, literal.len, mask)
}

/// The original terminator-then-payload observations share one guard. Tight
/// staging uses the original scalar prefix so capacity cannot hide a fault.
fn copy_heap_literal_bytes(
    job: &mut CharactersJob,
    memory: &impl Memory,
    source: usize,
    len: usize,
    mask: u32,
) -> bool {
    let Some(terminator) = source.checked_add(len) else {
        return false;
    };
    let fits = len != 0
        && len <= u32::MAX as usize
        && job.log.spans.capacity() - job.log.spans.len() >= 2
        && job
            .log
            .bytes
            .len()
            .checked_add(1)
            .and_then(|end| end.checked_add(len))
            .is_some_and(|end| end <= job.log.bytes.capacity() && end <= u32::MAX as usize);
    if fits {
        return matches!(
            job.log
                .take_scattered(memory, [terminator, source], [1, len], [mask; 2]),
            Some((_, 2))
        );
    }
    if job.log.take(memory, terminator, 1, mask).is_none() {
        return false;
    }
    if len != 0 && job.log.take(memory, source, len, mask).is_none() {
        return false;
    }
    true
}

/// Traversal state mirroring `ccharacter::RoleTree`'s read set: the mirror
/// issues the identical sentinel, link and node reads in the identical
/// successor order, parsing link values locally to choose branches.
struct TreeWalk {
    head: usize,
    next: usize,
    remaining: u32,
    count: u32,
}

/// Abort the whole copy (`false`) or kill only this map's batch (`true`).
enum TreeFate {
    Abort,
    Dead,
}

/// How a map's sentinel observation is acquired.
#[derive(Clone, Copy)]
enum Sentinel<'a> {
    /// Read it now.
    Read,
    /// A batch already gathered it; `None` is that read's failure.
    Gathered(Option<&'a [u8]>),
    /// It lies inside the character view, which fails it without a read.
    Owned,
}

/// Mirror of `RoleTree::read`: sentinel plus size/coherence parses.
fn tree_read(
    job: &mut CharactersJob,
    memory: &impl Memory,
    head: u64,
    count: u64,
    mask: u32,
    source: Sentinel<'_>,
) -> Result<TreeWalk, TreeFate> {
    let head = usize::try_from(head)
        .ok()
        .filter(|&pointer| pointer != 0)
        .ok_or_else(|| job.fate())?;
    let count = u32::try_from(count).map_err(|_| job.fate())?;
    let start = match source {
        Sentinel::Read => job.log.take(memory, head, census::SENTINEL, mask),
        Sentinel::Gathered(observed) => job.log.record(head, census::SENTINEL, observed, mask),
        Sentinel::Owned => return Err(TreeFate::Dead),
    }
    .ok_or_else(|| job.fate())?;
    let mut sentinel = [0u8; census::SENTINEL];
    sentinel.copy_from_slice(&job.log.bytes[start..start + census::SENTINEL]);
    let next = tree_sentinel(head, count, &sentinel).ok_or(TreeFate::Dead)?;
    Ok(TreeWalk {
        head,
        next,
        remaining: count,
        count,
    })
}

/// `RoleTree::read`'s checks on an acquired sentinel; returns the first node.
fn tree_sentinel(head: usize, count: u32, sentinel: &[u8]) -> Option<usize> {
    if sentinel[msvc_tree_node::NIL] != 1 {
        return None;
    }
    let link = |offset: usize| {
        usize::try_from(u64::from_le_bytes(
            sentinel[offset..offset + 8].try_into().unwrap(),
        ))
        .ok()
        .filter(|&pointer| pointer != 0)
    };
    let next = link(msvc_tree_node::LEFT)?;
    if (count == 0) != (next == head) {
        return None;
    }
    if count == 0 && (link(msvc_tree_node::PARENT)? != head || link(msvc_tree_node::RIGHT)? != head)
    {
        return None;
    }
    Some(next)
}

fn tree_link(
    job: &mut CharactersJob,
    memory: &impl Memory,
    node: usize,
    offset: usize,
    mask: u32,
) -> Result<usize, TreeFate> {
    let addr = node.checked_add(offset).ok_or(TreeFate::Dead)?;
    let start = job
        .log
        .take(memory, addr, 8, mask)
        .ok_or_else(|| job.fate())?;
    usize::try_from(u64::from_le_bytes(
        job.log.bytes[start..start + 8].try_into().unwrap(),
    ))
    .ok()
    .filter(|&pointer| pointer != 0)
    .ok_or(TreeFate::Dead)
}

/// Mirror of `RoleTree::next`: the identical bounded successor sequence,
/// returning native node addresses with no decoding.
fn tree_next(
    job: &mut CharactersJob,
    memory: &impl Memory,
    walk: &mut TreeWalk,
    mask: u32,
) -> Result<Option<usize>, TreeFate> {
    if walk.remaining == 0 {
        if walk.next != walk.head {
            return Err(TreeFate::Dead);
        }
        return Ok(None);
    }
    let current = walk.next;
    if current == walk.head {
        return Err(TreeFate::Dead);
    }
    let nil_addr = current
        .checked_add(msvc_tree_node::NIL)
        .ok_or(TreeFate::Dead)?;
    let nil = job
        .log
        .take(memory, nil_addr, 1, mask)
        .ok_or_else(|| job.fate())?;
    if job.log.bytes[nil] != 0 {
        return Err(TreeFate::Dead);
    }
    let right = tree_link(job, memory, current, msvc_tree_node::RIGHT, mask)?;
    let mut next = right;
    let mut steps = 0_u64;
    if right != walk.head {
        loop {
            steps += 1;
            if steps > u64::from(walk.count) {
                return Err(TreeFate::Dead);
            }
            let nil_addr = next
                .checked_add(msvc_tree_node::NIL)
                .ok_or(TreeFate::Dead)?;
            let nil = job
                .log
                .take(memory, nil_addr, 1, mask)
                .ok_or_else(|| job.fate())?;
            if job.log.bytes[nil] != 0 {
                return Err(TreeFate::Dead);
            }
            let left = tree_link(job, memory, next, msvc_tree_node::LEFT, mask)?;
            if left == walk.head {
                break;
            }
            if tree_link(job, memory, left, msvc_tree_node::PARENT, mask)? != next {
                return Err(TreeFate::Dead);
            }
            next = left;
        }
        if tree_link(job, memory, right, msvc_tree_node::PARENT, mask)? != current {
            return Err(TreeFate::Dead);
        }
    } else {
        let mut child = current;
        next = tree_link(job, memory, child, msvc_tree_node::PARENT, mask)?;
        while next != walk.head {
            steps += 1;
            if steps > u64::from(walk.count) {
                return Err(TreeFate::Dead);
            }
            if tree_link(job, memory, next, msvc_tree_node::LEFT, mask)? == child {
                break;
            }
            if tree_link(job, memory, next, msvc_tree_node::RIGHT, mask)? != child {
                return Err(TreeFate::Dead);
            }
            child = next;
            next = tree_link(job, memory, child, msvc_tree_node::PARENT, mask)?;
        }
    }
    walk.next = next;
    walk.remaining -= 1;
    Ok(Some(current))
}

/// One mirrored span read: `None` kills the enclosing batch. The caller maps
/// `None` to a whole-copy abort when the job overflowed, matching the staged
/// fallback rule.
fn take_or_kill(
    job: &mut CharactersJob,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
    dead: &mut bool,
) -> Option<usize> {
    match addr.and_then(|addr| job.log.take(memory, addr, len, mask)) {
        Some(start) => Some(start),
        None => {
            *dead = true;
            None
        }
    }
}

/// Fixed scalar sequences with either prefix failure or independent-field
/// continuation. Tight storage and absent checked addresses use the scalar
/// path before any reads, preserving its exact retained prefix and growth.
#[inline]
fn take_fixed_fields<const M: usize, const CONTINUE: bool>(
    job: &mut CharactersJob,
    memory: &impl Memory,
    addresses: [Option<usize>; M],
    lengths: [usize; M],
    mask: u32,
    dead: &mut bool,
) -> [Option<usize>; M] {
    let mut starts = [None; M];
    let total = lengths.iter().try_fold(job.log.bytes.len(), |end, &len| {
        end.checked_add(len)
            .filter(|&end| len <= u32::MAX as usize && end <= u32::MAX as usize)
    });
    if addresses.iter().any(Option::is_none)
        || job.log.overflowed
        || lengths.iter().filter(|&&len| len != 0).count()
            > job.log.spans.capacity() - job.log.spans.len()
        || total.is_none_or(|end| end > job.log.bytes.capacity())
    {
        for index in 0..M {
            starts[index] = take_or_kill(job, memory, addresses[index], lengths[index], mask, dead);
            if !CONTINUE && starts[index].is_none() {
                break;
            }
        }
        return starts;
    }
    let Some((copied, completed)) = job.log.take_scattered_prefetched(
        memory,
        addresses.map(Option::unwrap),
        lengths,
        [mask; M],
    ) else {
        *dead = true;
        return starts;
    };
    for index in 0..completed {
        starts[index] = Some(copied[index]);
    }
    if completed != M {
        *dead = true;
        if CONTINUE {
            // The failed field is already recorded. Resume only later
            // independent fields, without retrying the failed observation.
            for index in completed + 1..M {
                starts[index] =
                    take_or_kill(job, memory, addresses[index], lengths[index], mask, dead);
            }
        }
    }
    starts
}

fn pool_u64(job: &CharactersJob, start: usize) -> u64 {
    u64::from_le_bytes(job.log.bytes[start..start + 8].try_into().unwrap())
}

fn pool_u32(job: &CharactersJob, start: usize) -> u32 {
    u32::from_le_bytes(job.log.bytes[start..start + 4].try_into().unwrap())
}

/// Mirror of `ccharacter::walk_entry`'s string reads: header parses are local
/// (the headers ride in the view span), heap payloads and terminators are
/// copied by length. Any failure kills only the characters batch.
fn copy_character_text(
    job: &mut CharactersJob,
    memory: &impl Memory,
    view: &ViewCopy,
    pointer: usize,
    dead: &mut Dead,
) {
    if dead.characters {
        return;
    }
    let name_addr = pointer.checked_add(ccharacter::NAME).unwrap_or(0);
    let token_addr = pointer.checked_add(ccharacter::TOKEN).unwrap_or(0);
    let headers = view.field(job, 3);
    let name = parse_character_literal(&headers[..32], name_addr);
    let token = parse_character_literal(&headers[32..64], token_addr);
    let (Some(name), Some(token)) = (name, token) else {
        dead.characters = true;
        return;
    };
    // The staged view already hinted both heap strings before this page's
    // bodies began. Repeating those hints here adds no useful read lead.
    if !copy_character_literal_bytes(job, memory, &name, MASK_CHARACTERS)
        || !copy_character_literal_bytes(job, memory, &token, MASK_CHARACTERS)
    {
        dead.characters = true;
    }
}

/// Mirror of `ccharacter::walk_entry`'s template reads after both names: the
/// pointer rides in the view's name token span; a pointer this job already
/// holds an identity for reads nothing, exactly as the decode then supplies
/// it. Otherwise the header and generated byte in one scatter, then the file's
/// heap terminator and bytes. Any failure kills only the characters batch.
fn copy_character_template(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    view: &ViewCopy,
    id: u32,
    dead: &mut Dead,
) {
    if dead.characters {
        return;
    }
    let at = ccharacter::TEMPLATE - ccharacter::NAME_TOKEN_ID;
    let Ok(template) = usize::try_from(u64::from_le_bytes(
        view.field(job, 2)[at..at + 8].try_into().unwrap(),
    )) else {
        dead.characters = true;
        return;
    };
    if template == 0 || job.templates.known(id, template).is_some() {
        return;
    }
    let [header, _] = take_fixed_fields::<2, false>(
        job,
        memory,
        [Some(template), template.checked_add(ccharacter_template::GENERATED)],
        [ccharacter_template::HEADER, 1],
        MASK_CHARACTERS,
        &mut dead.characters,
    );
    if dead.characters {
        return;
    }
    let Some(header) = header else {
        dead.characters = true;
        return;
    };
    let Some(expected) = base.checked_add(ccharacter_template::VTABLE_RVA) else {
        dead.characters = true;
        return;
    };
    let file = {
        let bytes = &job.log.bytes[header..header + ccharacter_template::HEADER];
        (u64::from_le_bytes(bytes[..8].try_into().unwrap()) == expected as u64)
            .then(|| {
                parse_character_literal(
                    &bytes[ccharacter_template::SOURCE_FILE..ccharacter_template::SOURCE_LINE],
                    template + ccharacter_template::SOURCE_FILE,
                )
            })
            .flatten()
    };
    match file {
        Some(file) if copy_character_literal_bytes(job, memory, &file, MASK_CHARACTERS) => {}
        _ => dead.characters = true,
    }
}

/// Mirror of `ccountry_leader::walk_entry`'s read set: the role-map traversal
/// plus one ideology/role-header pair per node. Order, duplicate and auth
/// checks are local parses; any failure kills only the leaders batch.
fn copy_leaders(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    header: impl FnOnce(&CharactersJob) -> [u8; 16],
    pointer: usize,
    sentinel: Sentinel<'_>,
    dead: &mut Dead,
) -> bool {
    if dead.leaders {
        return true;
    }
    let expected = match base.checked_add(ccountry_leader::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => {
            dead.leaders = true;
            return true;
        }
    };
    let header = header(job);
    let head = u64::from_le_bytes(header[..8].try_into().unwrap());
    let count = u64::from_le_bytes(header[8..16].try_into().unwrap());
    let mut walk = match tree_read(job, memory, head, count, MASK_LEADERS, sentinel) {
        Ok(walk) => walk,
        Err(TreeFate::Abort) => return false,
        Err(TreeFate::Dead) => {
            dead.leaders = true;
            return true;
        }
    };
    let mut previous = None;
    loop {
        let node = match tree_next(job, memory, &mut walk, MASK_LEADERS) {
            Ok(node) => node,
            Err(TreeFate::Abort) => return false,
            Err(TreeFate::Dead) => {
                dead.leaders = true;
                return true;
            }
        };
        let Some(node) = node else { break };
        let Some(ideo) = take_or_kill(
            job,
            memory,
            node.checked_add(character_leader_node::IDEOLOGY),
            16,
            MASK_LEADERS,
            &mut dead.leaders,
        ) else {
            return !job.log.overflowed;
        };
        let ideology = i32::from_le_bytes(job.log.bytes[ideo..ideo + 4].try_into().unwrap());
        if previous.is_some_and(|key| key >= ideology) {
            dead.leaders = true;
            return true;
        }
        previous = Some(ideology);
        let role = usize::try_from(pool_u64(job, ideo + 8))
            .ok()
            .filter(|&role| role != 0);
        let Some(header) = take_or_kill(job, memory, role, 16, MASK_LEADERS, &mut dead.leaders)
        else {
            return !job.log.overflowed;
        };
        if pool_u64(job, header) != expected
            || pool_u64(job, header + ccountry_leader::CHARACTER) != pointer as u64
        {
            dead.leaders = true;
            return true;
        }
    }
    true
}

/// Exact advisor field prefix. Address overflow keeps the scalar mirror's
/// earlier observations; the normal path uses one guard for the same spans.
fn take_advisor_fields<const M: usize>(
    job: &mut CharactersJob,
    memory: &impl Memory,
    addresses: [Option<usize>; M],
    lengths: [usize; M],
    dead: &mut bool,
) -> Option<[usize; M]> {
    if addresses.iter().any(Option::is_none) {
        let mut starts = [0; M];
        for index in 0..M {
            starts[index] = take_or_kill(
                job,
                memory,
                addresses[index],
                lengths[index],
                MASK_ADVISORS,
                dead,
            )?;
        }
        return Some(starts);
    }
    match job.log.take_scattered_prefetched(
        memory,
        addresses.map(Option::unwrap),
        lengths,
        [MASK_ADVISORS; M],
    ) {
        Some((starts, completed)) if completed == M => Some(starts),
        _ => {
            *dead = true;
            None
        }
    }
}

/// Mirror of `cadvisor::walk_entry`'s read set: the role-map traversal plus
/// node, role and key/idea/slot string spans per node. Hash order and auth
/// checks are local parses; any failure kills only the advisors batch.
fn copy_advisors(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    view: &ViewCopy,
    pointer: usize,
    dead: &mut Dead,
) -> bool {
    if dead.advisors {
        return true;
    }
    let expected = match base.checked_add(cadvisor::VTABLE_RVA) {
        Some(expected) => expected as u64,
        None => {
            dead.advisors = true;
            return true;
        }
    };
    let header = view.field(job, 6);
    let head = u64::from_le_bytes(header[..8].try_into().unwrap());
    let count = u64::from_le_bytes(header[8..16].try_into().unwrap());
    let mut walk = match tree_read(job, memory, head, count, MASK_ADVISORS, Sentinel::Read) {
        Ok(walk) => walk,
        Err(TreeFate::Abort) => return false,
        Err(TreeFate::Dead) => {
            dead.advisors = true;
            return true;
        }
    };
    let mut previous_hash = None;
    loop {
        // Include the failing attempt so a short destination cannot hide its
        // recorded successor/body failure. Do not allocate from an unvisited
        // tree tail merely because its header advertises a large count.
        if walk.remaining != 0 {
            match job.advisor_demand.rows.checked_add(1) {
                Some(rows) => job.advisor_demand.rows = rows,
                None => job.advisor_demand.overflowed = true,
            }
        }
        let node = match tree_next(job, memory, &mut walk, MASK_ADVISORS) {
            Ok(node) => node,
            Err(TreeFate::Abort) => return false,
            Err(TreeFate::Dead) => {
                dead.advisors = true;
                return true;
            }
        };
        let Some(node) = node else { break };
        let Some([role_ptr, key_header, hash]) = take_advisor_fields(
            job,
            memory,
            [
                node.checked_add(character_advisor_node::ADVISOR),
                node.checked_add(character_advisor_node::KEY),
                node.checked_add(character_advisor_node::KEY_HASH),
            ],
            [8, 32, 4],
            &mut dead.advisors,
        ) else {
            return !job.log.overflowed;
        };
        let role = usize::try_from(pool_u64(job, role_ptr))
            .ok()
            .filter(|&role| role != 0);
        if role.is_none() {
            dead.advisors = true;
            return true;
        }
        let role = role.unwrap();
        let Some([vtable, parent, idea_header, slot_header]) = take_advisor_fields(
            job,
            memory,
            [
                Some(role),
                role.checked_add(cadvisor::CHARACTER),
                role.checked_add(cadvisor::IDEA_TOKEN),
                role.checked_add(cadvisor::SLOT),
            ],
            [8, 8, 32, 32],
            &mut dead.advisors,
        ) else {
            return !job.log.overflowed;
        };
        if pool_u64(job, vtable) != expected || pool_u64(job, parent) != pointer as u64 {
            dead.advisors = true;
            return true;
        }
        let hash = pool_u32(job, hash);
        if previous_hash.is_some_and(|previous| previous > hash) || previous_hash == Some(hash) {
            dead.advisors = true;
            return true;
        }
        previous_hash = Some(hash);
        let key_addr = node.checked_add(character_advisor_node::KEY).unwrap_or(0);
        let idea_addr = role.checked_add(cadvisor::IDEA_TOKEN).unwrap_or(0);
        let slot_addr = role.checked_add(cadvisor::SLOT).unwrap_or(0);
        let literal = |start: usize, addr: usize| {
            let mut header = [0u8; 32];
            header.copy_from_slice(&job.log.bytes[start..start + 32]);
            parse_character_literal(&header, addr)
        };
        let (Some(key), Some(idea), Some(slot)) = (
            literal(key_header, key_addr),
            literal(idea_header, idea_addr),
            literal(slot_header, slot_addr),
        ) else {
            dead.advisors = true;
            return true;
        };
        let required_bytes = [key.len, idea.len, slot.len]
            .into_iter()
            .try_fold(job.advisor_demand.bytes, usize::checked_add);
        let mut copied = true;
        let mut validation_failed = false;
        for literal in [&key, &idea, &slot] {
            let first_span = job.log.spans.len();
            if !copy_character_literal_bytes(job, memory, literal, MASK_ADVISORS) {
                // The first heap read is its terminator. These failures occur
                // before the typed walk checks output capacity, so no byte
                // allocation is needed to retain their original failure.
                let terminator = literal.source.checked_add(literal.len);
                validation_failed = terminator.is_none()
                    || job.log.spans.get(first_span).is_some_and(|span| {
                        !span.ok && Some(span.addr) == terminator && span.len == 1
                    });
                copied = false;
                break;
            }
        }
        if !validation_failed {
            match required_bytes {
                Some(bytes) => job.advisor_demand.bytes = bytes,
                None => job.advisor_demand.overflowed = true,
            }
        }
        if !copied {
            dead.advisors = true;
            return true;
        }
    }
    true
}

/// Authenticate a unit leader header locally, mirroring `cunit_leader`'s
/// identity read. Returns the kind and leader id for the copy below.
fn parse_unit_identity(job: &CharactersJob, base: usize, start: usize) -> Option<(u8, u32)> {
    let vtable = pool_u64(job, start);
    let relative = usize::try_from(vtable).ok()?.checked_sub(base)?;
    let kind = match relative {
        cunit_leader::VTABLE_RVA => cunit_leader::KIND_BASE,
        carmy_leader::VTABLE_RVA => cunit_leader::KIND_ARMY,
        cnavy_leader::VTABLE_RVA => cunit_leader::KIND_NAVY,
        coperative_leader::VTABLE_RVA => cunit_leader::KIND_OPERATIVE,
        _ => return None,
    };
    if pool_u32(job, start + cunit_leader::TYPE) != cunit_leader::REFERENCE_TYPE {
        return None;
    }
    Some((kind, pool_u32(job, start + cunit_leader::ID)))
}

/// Mirror of `read_unit` plus `cunit_leader::read`/`read_text`: identity,
/// state, skill, kind components and both text strings. The identity-fallback
/// spans are always recorded so the trait replay finds them when the scalar
/// read fails. Any failure kills only the units batch.
fn copy_unit(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    unit: Option<usize>,
    character_id: impl FnOnce(&CharactersJob) -> u32,
    unit_id: &mut Option<u32>,
    dead: &mut Dead,
) -> bool {
    let Some(unit) = unit else {
        // Null role pointers are handled by the caller: no rows, no reads.
        return true;
    };
    if dead.units && dead.traits && dead.experience {
        // The typed walk `continue`s without reads once all three batches
        // are dead, and so does the mirror.
        return true;
    }
    // The replay calls the identity fallback while its traits or experience
    // batch is still alive at this character, which is exactly the entry
    // state here: later kills in this function happen after the replay's
    // call point, so they must not close this gate.
    let need_fallback = !dead.traits || !dead.experience;
    // Scalar and trait identities are independent native observations. The
    // trait acquisition authenticates its own identity even after a scalar
    // failure, provided the fallback can still resolve the parent handle.
    let source_mask = job.source_mask();
    let identity = take_or_kill(
        job,
        memory,
        Some(unit),
        16,
        MASK_UNIT_LINK & source_mask,
        &mut dead.units,
    );
    let parsed = identity.and_then(|start| parse_unit_identity(job, base, start));
    *unit_id = parsed.map(|(_, id)| id);
    if identity.is_some() && parsed.is_none() {
        dead.units = true;
    }
    let kind = parsed.map(|(kind, _)| kind);
    if !dead.units {
        // State scatter in `cunit_leader::read` order: character backlink,
        // category, experience, skill pointer.
        let state = take_fixed_fields::<4, false>(
            job,
            memory,
            [
                cunit_leader::CHARACTER_TYPE,
                cunit_leader::CATEGORY,
                cunit_leader::EXPERIENCE,
                cunit_leader::SKILL,
            ]
            .map(|offset| unit.checked_add(offset)),
            [8, 4, 8, 8],
            MASK_UNITS,
            &mut dead.units,
        );
        if !job.log.overflowed
            && let [
                Some(backlink),
                Some(_category),
                Some(_experience),
                Some(skill),
            ] = state
        {
            // These are independent registries: a leader's id is not the
            // id of the character whose role points to it.
            let character_id = character_id(job);
            if pool_u32(job, backlink) != ccharacter::REFERENCE_TYPE
                || pool_u32(job, backlink + 4) != character_id
            {
                dead.units = true;
            } else {
                copy_unit_skill(job, memory, base, skill, dead);
            }
        }
        if job.log.overflowed {
            return false;
        }
        if !dead.units {
            if let Some(kind) = kind {
                copy_unit_components(job, memory, unit, kind, dead);
            }
            if job.log.overflowed {
                return false;
            }
        }
        if !dead.units {
            copy_unit_text(job, memory, unit, dead);
            if job.log.overflowed {
                return false;
            }
        }
    }
    // Identity-fallback spans for the trait replay. Recorded whenever the
    // replay could still call the fallback (entry gate above); a live
    // failure here kills only traits and experience, exactly like the
    // typed fallback error, and never the units outcome.
    if need_fallback {
        let mut fallback_dead = false;
        let [first, second] = take_fixed_fields::<2, false>(
            job,
            memory,
            [
                unit.checked_add(cunit_leader::TYPE),
                unit.checked_add(cunit_leader::CHARACTER_TYPE),
            ],
            [8, 8],
            MASK_TRAIT_FALLBACK & source_mask,
            &mut fallback_dead,
        );
        if job.log.overflowed {
            return false;
        }
        if dead.units && (first.is_none() || second.is_none()) {
            dead.traits = true;
            dead.experience = true;
        }
        if dead.units
            && let Some(first) = first
        {
            *unit_id = Some(pool_u32(job, first + 4));
        }
    }
    true
}

/// Mirror of the skill block in `cunit_leader::read`. Kills only the units
/// batch; the vtable predicate is a local parse.
fn copy_unit_skill(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    skill_start: usize,
    dead: &mut Dead,
) {
    let skill = usize::try_from(pool_u64(job, skill_start)).ok();
    match skill {
        None => {
            dead.units = true;
        }
        Some(0) => {}
        Some(skill) => {
            let skill_expected = match base.checked_add(cunit_leader_skill::VTABLE_RVA) {
                Some(expected) => expected as u64,
                None => {
                    dead.units = true;
                    return;
                }
            };
            let addresses = [
                Some(skill),
                skill.checked_add(cunit_leader_skill::VALID),
                skill.checked_add(cunit_leader_skill::LEVEL),
            ];
            let cache_slot = (skill >> 3) % job.skills.len();
            let cached = job.skills[cache_slot];
            let [vtable, valid, scalars] = if cached.pointer == skill && !job.log.planning() {
                let native_bytes = job.log.raw_bytes[3];
                let starts = take_fixed_fields::<3, true>(
                    job,
                    &cached,
                    addresses,
                    [8, 1, 12],
                    MASK_UNITS,
                    &mut dead.units,
                );
                // These raw occurrences are owned copies, not native reads.
                job.log.raw_bytes[3] = native_bytes;
                starts
            } else {
                take_fixed_fields::<3, true>(
                    job,
                    memory,
                    addresses,
                    [8, 1, 12],
                    MASK_UNITS,
                    &mut dead.units,
                )
            };
            // The typed walk authenticates the vtable from the first span
            // and the remaining fields from the later spans; a completed
            // zero prefix still fails here with the vtable reason.
            if let Some(vtable) = vtable
                && pool_u64(job, vtable) != skill_expected
            {
                dead.units = true;
            }
            if !dead.units
                && cached.pointer != skill
                && let [Some(vtable), Some(valid), Some(scalars)] = [vtable, valid, scalars]
            {
                let mut bytes = [0; 21];
                bytes[..8].copy_from_slice(&job.log.bytes[vtable..vtable + 8]);
                bytes[8] = job.log.bytes[valid];
                bytes[9..].copy_from_slice(&job.log.bytes[scalars..scalars + 12]);
                job.skills[cache_slot] = SkillCopy {
                    pointer: skill,
                    bytes,
                };
            }
        }
    }
}

/// Mirror of the kind-component scatter in `cunit_leader::read`.
fn copy_unit_components(
    job: &mut CharactersJob,
    memory: &impl Memory,
    unit: usize,
    kind: u8,
    dead: &mut Dead,
) {
    let components = match kind {
        cunit_leader::KIND_ARMY => Some([
            carmy_leader::ATTACK,
            carmy_leader::DEFENSE,
            carmy_leader::PLANNING,
            carmy_leader::LOGISTICS,
        ]),
        cunit_leader::KIND_NAVY => Some([
            cnavy_leader::ATTACK,
            cnavy_leader::DEFENSE,
            cnavy_leader::MANEUVERING,
            cnavy_leader::COORDINATION,
        ]),
        _ => None,
    };
    if let Some(offsets) = components {
        take_fixed_fields::<4, false>(
            job,
            memory,
            offsets.map(|offset| unit.checked_add(offset)),
            [4; 4],
            MASK_UNITS,
            &mut dead.units,
        );
    }
    // Then `cunit_leader::read`'s owner and visibility scatter, with the
    // army leader's captured byte.
    let at = |offset| unit.checked_add(offset);
    let owner = [
        at(cunit_leader::OWNER),
        at(cunit_leader::VISIBILITY_CHOICE),
        at(cunit_leader::TEMPLATE_VISIBILITY),
    ];
    if kind == cunit_leader::KIND_ARMY {
        let [a, b, c] = owner;
        take_fixed_fields::<4, false>(
            job,
            memory,
            [a, b, c, at(carmy_leader::CAPTURED)],
            [4, 8, 1, 1],
            MASK_UNITS,
            &mut dead.units,
        );
    } else {
        take_fixed_fields::<3, false>(job, memory, owner, [4, 8, 1], MASK_UNITS, &mut dead.units);
    }
}

/// Mirror of `cunit_leader::read_text`: both string headers plus heap
/// payloads and terminators by header length.
fn copy_unit_text(job: &mut CharactersJob, memory: &impl Memory, unit: usize, dead: &mut Dead) {
    let [name_header, token_header] = take_fixed_fields::<2, true>(
        job,
        memory,
        [
            unit.checked_add(cunit_leader::NAME),
            unit.checked_add(cunit_leader::TOKEN),
        ],
        [32, 32],
        MASK_UNITS,
        &mut dead.units,
    );
    let (Some(name_header), Some(token_header)) = (name_header, token_header) else {
        return;
    };
    let mut header = [0u8; 32];
    header.copy_from_slice(&job.log.bytes[name_header..name_header + 32]);
    let name = copy_unit_literal(job, memory, &header, MASK_UNITS);
    header.copy_from_slice(&job.log.bytes[token_header..token_header + 32]);
    let token = copy_unit_literal(job, memory, &header, MASK_UNITS);
    if name.is_none() || token.is_none() {
        dead.units = true;
    }
}

/// Native trait-vector identity and layout for one owned acquisition.
#[derive(Debug, Clone, Copy)]
struct TraitVectorCopy {
    unit: usize,
    leader_id: u32,
    array_offset: usize,
    stride: usize,
    has_experience: bool,
}

/// Retain ordered occurrences and share successful current-pass definitions.
/// Any native failure kills only this table; staging exhaustion aborts the job.
fn copy_trait_vector(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    params: TraitVectorCopy,
    dead: &mut bool,
) -> bool {
    if *dead {
        return true;
    }
    if let Some(plan) = &mut job.source_bindings {
        plan.remember(params);
    }
    *dead = job.traits.acquire(memory, base, params).is_err();
    !job.traits.overflowed()
}

/// Mirror of one staged `walk_family` entry's body read set across all six
/// batches. Returns `false` only to abort the whole copy (pool overflow or a
/// census-level fault); batch kills are recorded in `dead` and the loop
/// continues exactly like the typed walk.
fn copy_character_body(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    view: &mut ViewCopy,
    dead: &mut Dead,
) -> bool {
    if job.log.overflowed {
        return false;
    }
    let id = u32::from_le_bytes(
        view.field(job, 0)[ccharacter::ID..ccharacter::ID + 4]
            .try_into()
            .unwrap(),
    );
    copy_character_text(job, memory, view, pointer, dead);
    if job.log.overflowed {
        return false;
    }
    copy_character_template(job, memory, base, view, id, dead);
    if job.log.overflowed {
        return false;
    }
    let fields = *view;
    let header = |job: &CharactersJob| fields.field(job, 4).try_into().unwrap();
    if !copy_leaders(job, memory, base, header, pointer, Sentinel::Read, dead) {
        return false;
    }
    if !copy_advisors(job, memory, base, view, pointer, dead) {
        return false;
    }
    let unit = view_unit(job, view);
    let character_id = |job: &CharactersJob| {
        u32::from_le_bytes(fields.field(job, 0)[ccharacter::ID..ccharacter::ID + 4].try_into().unwrap())
    };
    if !copy_unit(job, memory, base, unit, character_id, &mut view.unit_id, dead) {
        return false;
    }
    // Trait vectors share the unit leader pointer resolved view-locally
    // above; a null role skips them silently like the typed walk.
    match view_unit(job, view) {
        Some(unit) => copy_trait_vectors(job, memory, base, unit, view.unit_id.unwrap_or(id), dead),
        None => true,
    }
}

fn view_unit(job: &CharactersJob, view: &ViewCopy) -> Option<usize> {
    usize::try_from(u64::from_le_bytes(view.field(job, 5).try_into().unwrap()))
        .ok()
        .filter(|&unit| unit != 0)
}

/// Both trait vectors of one unit leader, in the typed walk's order.
fn copy_trait_vectors(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    unit: usize,
    leader_id: u32,
    dead: &mut Dead,
) -> bool {
    copy_trait_vector(
        job,
        memory,
        base,
        TraitVectorCopy {
            unit,
            leader_id,
            array_offset: cunit_leader::TRAIT_DATA,
            stride: 8,
            has_experience: false,
        },
        &mut dead.traits,
    ) && copy_trait_vector(
        job,
        memory,
        base,
        TraitVectorCopy {
            unit,
            leader_id,
            array_offset: cunit_leader::TRAIT_EXPERIENCE_DATA,
            stride: unit_leader_trait_experience_entry::STRIDE,
            has_experience: true,
        },
        &mut dead.experience,
    )
}

/// Mirror of the roster census plus the cursor's 32-entry page loop. Page and
/// pointer parses are local; any census failure stops the copy and the replay
/// fails identically from the same recorded spans.
fn copy_pass(memory: &impl Memory, context: CharactersCopyContext, job: &mut CharactersJob,
    section: Option<Array>) {
    let _reads = read_accounting::table(TABLES[anchor(context.active)]);
    job.log.clear();
    job.advisor_demand = AdvisorDemand::default();
    job.skills.fill(SkillCopy::default());
    job.active = context.active;
    job.traits.begin(context);
    job.game_state = context.game_state;
    job.census_current = false;
    job.role_census_current = false;
    job.advisor_section_complete = false;
    // Each table owner retains its own raw program. Parts beyond the core
    // also preserve capacity demand and freshly reacquire bound trait sources.
    let hourly = context.active == [true, false, false, true, false, false];
    let bound = !hourly && job.source_bindings.is_some();
    if bound && !job.source_bindings.as_ref().unwrap().matches(context) {
        job.log.invalidate_read_plan();
    }
    if context.active[0] {
        job.templates.begin(context.base, context.game_state, context.force_full);
    }
    // Which templates the program reads depends on the identities this job
    // already holds, so their generation keys it.
    let key = section.map_or(
        [
            context.base,
            context.game_state,
            job.source_mask() as usize,
            job.templates.generation as usize,
        ],
        |array| [context.base, context.game_state, array.data, array.count as usize],
    );
    if !(hourly || bound) || !job.log.begin_planned_capture(memory, key) {
        if bound { job.source_bindings.as_mut().unwrap().begin(context); }
        // The root returns every portrait lease before this job can be recycled.
        // A retained lease disables sharing; never allocate or wait here.
        let mut owned = job.portrait_census.take();
        let census = owned.as_mut().and_then(Arc::get_mut);
        copy_roster(memory, context.base, context.game_state, job, census, section);
        if bound {
            let complete = !job.overflowed() && !job.advisor_demand.overflowed
                && !job.log.fault.iter().any(|fault| *fault)
                && job.traits.vectors.iter().flatten().all(|v| matches!(v.outcome, Outcome::Rows { .. }));
            let plan = job.source_bindings.as_mut().unwrap();
            plan.ready = complete && !plan.overflowed;
            plan.advisor = job.advisor_demand;
            plan.advisor_section_complete = job.advisor_section_complete;
        }
        // Partial diagnostic passes do not rebuild the retained raw program.
        // They may preserve its census only when every occurrence stayed identical.
        // Otherwise retire that program before an A -> B -> A roster change
        // could certify B's identities using A's successful prepared reads.
        job.census_matches_plan = job.census_current && (hourly || bound || (job.census_matches_plan
            && owned.as_ref().is_some_and(|census| census.unchanged())));
        if !(hourly || bound || job.census_matches_plan) { job.log.invalidate_read_plan(); }
        job.portrait_census = owned;
    } else {
        if bound {
            let plan = job.source_bindings.as_ref().unwrap();
            job.advisor_demand = plan.advisor;
            job.advisor_section_complete = plan.advisor_section_complete;
            let mut dead = [false; 2];
            for &source in &plan.sources {
                let table = usize::from(source.has_experience);
                if !dead[table] {
                    dead[table] = job.traits.acquire(memory, context.base, source).is_err();
                    if job.traits.overflowed() { break; }
                }
            }
        }
        // The prepared program freshly checked both roster pointer pages and
        // every identity header. It also freshly copied every country field.
        job.census_current = job.census_matches_plan && job.portrait_census.as_ref().is_some_and(|census|
            census.rows(context.base, context.game_state).is_some());
    }
    job.log.finish_planned_capture();
    job.traits.finish();
}

fn copy_roster(memory: &impl Memory, base: usize, game_state: usize, job: &mut CharactersJob,
    mut census: Option<&mut ccharacter_manager::PortraitCensus>, section: Option<Array>) {
    if let Some(census) = census.as_mut() { census.begin(base, game_state); }
    job.game_state = game_state;
    let source_mask = job.source_mask();
    let mut dead = Dead {
        characters: !job.active[0],
        leaders: !job.active[1],
        advisors: !job.active[2],
        units: !job.active[3],
        traits: !job.active[4],
        experience: !job.active[5],
    };
    let arrays = if let Some(array) = section {
        [array, Array { data: 0, count: 0 }]
    } else {
        let Some(arrays) = copy_roster_headers(memory, base, game_state, job, source_mask) else { return; };
        arrays
    };
    // Demand the whole native roster even if staging stops this pass early.
    if let Some(census) = census.as_mut() {
        census.expect(arrays[0].count as usize + arrays[1].count as usize);
        job.census_historical = arrays[0].count as usize;
    }
    copy_roster_entries(memory, base, game_state, job, census, arrays, &mut dead);
}

fn copy_roster_headers(memory: &impl Memory, base: usize, game_state: usize,
    job: &mut CharactersJob, source_mask: u32) -> Option<[Array; 2]> {
    let manager_addr = game_state.checked_add(cgame_state::CHARACTER_MANAGER)?;
    let manager_start = job.log.take(memory, manager_addr, 8, source_mask)?;
    let manager = usize::try_from(pool_u64(job, manager_start))
        .ok()
        .filter(|&manager| manager != 0)?;
    let vtable_start = job.log.take(memory, manager, 8, source_mask)?;
    let expected = base.checked_add(ccharacter_manager::VTABLE_RVA)? as u64;
    if pool_u64(job, vtable_start) != expected {
        return None;
    }
    let mut arrays = [Array { data: 0, count: 0 }; 2];
    for (slot, offset) in [
        ccharacter_manager::HISTORICAL_CHARACTERS,
        ccharacter_manager::DYNAMIC_CHARACTERS,
    ]
    .iter()
    .enumerate()
    {
        let addr = manager.checked_add(*offset)?;
        let header_start = job.log.take(memory, addr, 16, source_mask)?;
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&job.log.bytes[header_start..header_start + 16]);
        arrays[slot] = Array::from_header(raw, 8)?;
    }
    arrays[0].count.checked_add(arrays[1].count)?;
    Some(arrays)
}

fn copy_roster_entries(memory: &impl Memory, base: usize, game_state: usize,
    job: &mut CharactersJob, mut census: Option<&mut ccharacter_manager::PortraitCensus>,
    arrays: [Array; 2], dead: &mut Dead) {
    let source_mask = job.source_mask();
    for array in arrays.iter() {
        let mut ordinal = 0u32;
        while ordinal < array.count {
            let chunk = (array.count - ordinal).min(ROSTER_PAGE);
            let Some(page_addr) = array.element(ordinal, 8) else {
                return;
            };
            let Some(page_start) = job
                .log
                .take(memory, page_addr, chunk as usize * 8, source_mask)
            else {
                return;
            };
            // Look ahead only within the already-copied roster page. These
            // hints cover the same seven view fields; the loop below still
            // performs every actual read and failure check in native order.
            for bytes in job.log.bytes[page_start..page_start + chunk as usize * 8].chunks_exact(8)
            {
                let Some(pointer) = usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
                    .ok()
                    .filter(|&pointer| pointer != 0)
                else {
                    break;
                };
                for (index, (offset, len)) in VIEW_OFFSETS.into_iter().zip(VIEW_LENS).enumerate() {
                    if VIEW_MASKS[index] & source_mask == 0 {
                        continue;
                    }
                    if let Some(address) = pointer.checked_add(offset) {
                        job.log.prefetch(address, len);
                    }
                }
            }
            let mut views: [Option<ViewCopy>; ROSTER_PAGE as usize] = std::array::from_fn(|_| None);
            let mut staged = 0;
            let mut terminal = false;
            for index in 0..chunk {
                let at = page_start + index as usize * 8;
                let pointer = usize::try_from(u64::from_le_bytes(
                    job.log.bytes[at..at + 8].try_into().unwrap(),
                ))
                .ok()
                .filter(|&pointer| pointer != 0);
                let Some(pointer) = pointer else {
                    terminal = true;
                    break;
                };
                let Some(view) = copy_view(job, memory, base, pointer) else {
                    // A fatal later slot is observed now, but its predecessors'
                    // bodies must run before the corresponding typed error.
                    terminal = true;
                    break;
                };
                if job.log.overflowed {
                    return;
                }
                if view.starts[1] != u32::MAX && let Some(census) = census.as_mut() {
                    let identity = view.field(job, 0);
                    census.push(ccharacter_manager::PortraitIdentity {
                        pointer,
                        character: u32::from_le_bytes(identity[ccharacter::ID..ccharacter::ID + 4].try_into().unwrap()),
                    });
                }
                hint_character_targets(job, &view, dead, |address, len| job.log.prefetch(address, len));
                views[staged] = Some(view);
                staged += 1;
            }
            if job.log.overflowed {
                return;
            }
            for (index, view) in views[..staged].iter_mut().enumerate() {
                let at = page_start + index * 8;
                let pointer =
                    u64::from_le_bytes(job.log.bytes[at..at + 8].try_into().unwrap()) as usize;
                if !copy_character_body(
                    job,
                    memory,
                    base,
                    pointer,
                    view.as_mut().expect("staged character view"),
                    dead,
                ) {
                    return;
                }
            }
            if terminal {
                return;
            }
            ordinal += chunk;
        }
    }
    job.advisor_section_complete = !dead.advisors && !job.log.fault[2] && !job.overflowed();
    if !job.overflowed() && let Some(census) = census {
        census.finish(arrays[0].count as usize + arrays[1].count as usize);
        job.census_current = census.rows(base, game_state).is_some();
    }
}

/// How one character job relates to this callback's role census.
#[derive(Clone, Copy)]
pub(crate) enum Roles<C> {
    /// Walk the roster independently.
    Independent,
    /// The roster job also gathers the role census for this hour's role jobs.
    Gather,
    /// A role job borrows the roster job's census; without an admitted
    /// census it walks the roster independently.
    Borrow(C),
}

impl Roles<Arc<census::RoleCensus>> {
    pub(crate) fn as_deref(&self) -> Roles<&census::RoleCensus> {
        match self {
            Self::Independent => Roles::Independent,
            Self::Gather => Roles::Gather,
            Self::Borrow(census) => Roles::Borrow(census),
        }
    }
}

/// Independent acquisition, as tests and non-role hours use it.
#[cfg(test)]
pub(crate) fn copy_characters_refresh(
    memory: &impl Memory,
    ctx: CharactersCopyContext,
    job: &mut CharactersJob,
) -> CharactersCopyReport {
    copy_characters(memory, ctx, job, Roles::Independent)
}

/// Raw acquisition of the current scheduled family with exclusively owned jobs.
/// Copies one current payload and returns its acquisition costs.
/// On staging overflow the caller takes the existing
/// typed fallback and enqueues no payload.
pub(crate) fn copy_characters(
    memory: &impl Memory,
    ctx: CharactersCopyContext,
    job: &mut CharactersJob,
    roles: Roles<&census::RoleCensus>,
) -> CharactersCopyReport {
    if !ctx.active.iter().any(|&active| active) {
        job.clear();
        return CharactersCopyReport {
            raw_ok: true,
            ..Default::default()
        };
    }
    let started = qpc();
    let wall_started = std::time::Instant::now();
    let loaned = match roles {
        Roles::Borrow(census) => census.admitted(ctx.base, ctx.game_state),
        _ => None,
    };
    if job.advisor_cache.is_some() {
        job.clear();
        job.active = ctx.active;
        job.game_state = ctx.game_state;
        let cache = job.advisor_cache.as_mut().unwrap();
        cache.capture(memory, ctx, &crate::advisor_changes::Observer, loaned);
        crate::advisor_changes::record(cache.counts);
    } else if job.advisor_dynamic.is_some() {
        // The feature-gated section reader keeps its independent roster walk.
        copy_advisor_sections(memory, ctx, job);
    } else if let Some(census) = loaned.filter(|_| census_part(ctx.active)) {
        copy_census(memory, ctx, job, census);
    } else {
        copy_pass(memory, ctx, job, None);
        if matches!(roles, Roles::Gather) {
            gather_role_census(memory, ctx, job);
        }
    }
    let total_ns = wall_started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
    use std::sync::atomic::Ordering::Relaxed;
    COPY_NS[job.part].fetch_add(total_ns, Relaxed);
    BACKEND_NS[job.part].fetch_add(job.log.backend_ns + job.advisor_dynamic.as_ref().map_or(0, |job| job.log.backend_ns), Relaxed);
    COPY_CALLS[job.part].fetch_add(1, Relaxed);
    let mut raw_bytes = job.log.raw_bytes;
    if let Some(cache) = &job.advisor_cache { raw_bytes[2] = cache.raw_bytes; }
    if let Some(dynamic) = &job.advisor_dynamic {
        for (index, bytes) in raw_bytes.iter_mut().enumerate() {
            *bytes += job.advisor_roots.raw_bytes[index] + dynamic.log.raw_bytes[index];
        }
    }
    for table in 0..2 {
        raw_bytes[table + 4] += job.traits.raw_bytes[table];
    }
    let plan_failed = job.requires_no_retry();
    let mut copy_ticks = [0; 6];
    copy_ticks[anchor(ctx.active)] = elapsed(started);
    CharactersCopyReport {
        raw_ok: !job.overflowed() && !plan_failed,
        plan_failed,
        copy_ticks,
        raw_bytes,
    }
}

/// Role jobs that take their roster from a loaned census: country-leader
/// roles alone, or unit-leader traits and experience alone.
fn census_part(active: [bool; 6]) -> bool {
    let leaders = active[1] && !active[4] && !active[5];
    let traits = !active[1] && (active[4] || active[5]);
    !active[0] && !active[2] && !active[3] && (leaders || traits)
}

/// On role hours the roster job gathers the role fields of its complete,
/// freshly certified census in one guarded call. Its reads are the roster
/// job's; a retained loan disables sharing without waiting or allocating.
fn gather_role_census(memory: &impl Memory, ctx: CharactersCopyContext, job: &mut CharactersJob) {
    if !job.census_current {
        return;
    }
    let _reads = read_accounting::table(TABLES[anchor(ctx.active)]);
    let scope = (ctx.base, ctx.game_state);
    let rows = job.portrait_census.as_ref().and_then(|census| census.rows(ctx.base, ctx.game_state));
    if let (Some(rows), Some(roles)) = (rows, job.role_census.as_mut().and_then(Arc::get_mut)) {
        job.role_census_current = roles.acquire(memory, scope, rows, job.census_historical);
        if job.role_census_current {
            job.log.raw_bytes[0] += (rows.len() * (census::ROW - 4)) as u64;
        }
    }
}

/// A role job's acquisition from the loaned census: the independent walk's
/// reads and failure order from each occurrence's map header or unit pointer
/// on, without the roster, pointer pages or character views.
fn copy_census(
    memory: &impl Memory,
    context: CharactersCopyContext,
    job: &mut CharactersJob,
    census: census::Admitted<'_>,
) {
    let _reads = read_accounting::table(TABLES[anchor(context.active)]);
    job.log.clear();
    job.advisor_demand = AdvisorDemand::default();
    job.skills.fill(SkillCopy::default());
    job.active = context.active;
    job.traits.begin(context);
    job.game_state = context.game_state;
    job.census_current = false;
    job.role_census_current = false;
    job.advisor_section_complete = false;
    job.census_capture = true;
    job.census_visited.clear();
    job.census_required = 0;
    job.census_historical = census.historical;
    let mut dead = Dead {
        characters: true,
        leaders: !context.active[1],
        advisors: true,
        units: true,
        traits: !context.active[4],
        experience: !context.active[5],
    };
    // Trait-source bindings belong to the independent walk's prepared program.
    let bindings = job.source_bindings.take();
    if context.active[1] {
        copy_census_leaders(job, memory, context.base, census, &mut dead);
    } else {
        copy_census_units(job, memory, context.base, census, &mut dead);
    }
    job.source_bindings = bindings;
    job.traits.finish();
}

/// Retain one occurrence for decode. Exhausted storage records the demand of
/// every remaining occurrence that could need one and stops the acquisition.
fn visit_census(
    job: &mut CharactersJob,
    census: census::Admitted<'_>,
    index: usize,
    needs: impl Fn(census::Occurrence) -> bool,
) -> bool {
    if job.census_visited.len() < job.census_visited.capacity() {
        job.census_visited.push(census.occurrence(index));
        return true;
    }
    let remaining = (index..census.len()).filter(|&index| needs(census.occurrence(index))).count();
    job.census_required = job.census_visited.len() + remaining.max(1);
    false
}

/// Leader-map sentinels are gathered in batches ahead of their maps. An empty
/// map whose sentinel passes produces no row and no decode read, so neither
/// its occurrence nor its sentinel is retained; every other occurrence records
/// exactly the independent walk's observations.
fn copy_census_leaders(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    census: census::Admitted<'_>,
    dead: &mut Dead,
) {
    use census::{SENTINEL, SENTINEL_BATCH};
    let needs = |occurrence: census::Occurrence| {
        census::map_head(occurrence.leaders()).is_none_or(|(_, count)| count != 0)
    };
    let omissible = base.checked_add(ccountry_leader::VTABLE_RVA).is_some();
    let mut heads = [0; SENTINEL_BATCH];
    let mut sentinels = [0; SENTINEL_BATCH * SENTINEL];
    let mut next = 0;
    while next < census.len() && !dead.leaders {
        // A header that fails before its sentinel read, or whose sentinel
        // lies inside the character view, ends the batch.
        let mut staged = 0;
        while staged < SENTINEL_BATCH && next + staged < census.len() {
            let occurrence = census.occurrence(next + staged);
            match census::map_head(occurrence.leaders()) {
                Some((head, _)) if !census::inside_view(occurrence.pointer, head, SENTINEL) => {
                    heads[staged] = head;
                }
                _ => break,
            }
            staged += 1;
        }
        let completed = census::gather_sentinels(memory, &heads[..staged], &mut sentinels);
        job.log.raw_bytes[1] += (completed * SENTINEL) as u64;
        let end = if staged == SENTINEL_BATCH {
            next + staged
        } else {
            census.len().min(next + staged + 1)
        };
        for index in next..end {
            if dead.leaders {
                return;
            }
            let occurrence = census.occurrence(index);
            let header = occurrence.leaders();
            let slot = index - next;
            let gathered = &sentinels[slot * SENTINEL..][..SENTINEL];
            let sentinel = if slot < staged {
                Sentinel::Gathered((slot < completed).then_some(gathered))
            } else if census::map_head(header).is_some() {
                Sentinel::Owned
            } else {
                Sentinel::Read
            };
            if omissible
                && let Sentinel::Gathered(Some(bytes)) = sentinel
                && let Some((head, 0)) = census::map_head(header)
                && tree_sentinel(head, 0, bytes).is_some()
            {
                continue;
            }
            if !visit_census(job, census, index, needs)
                || !copy_leaders(job, memory, base, |_| header, occurrence.pointer, sentinel, dead)
            {
                return;
            }
        }
        next = end;
    }
}

/// Unit leaders in roster order, from each occurrence's unit pointer on.
fn copy_census_units(
    job: &mut CharactersJob,
    memory: &impl Memory,
    base: usize,
    census: census::Admitted<'_>,
    dead: &mut Dead,
) {
    /// Occurrences ahead whose leader lines are hinted.
    const LOOKAHEAD: usize = 16;
    let unit = |occurrence: census::Occurrence| {
        usize::try_from(u64::from_le_bytes(occurrence.unit()))
            .ok()
            .filter(|&unit| unit != 0)
    };
    let needs = |occurrence: census::Occurrence| unit(occurrence).is_some();
    for index in 0..census.len() {
        if dead.traits && dead.experience {
            return;
        }
        if let Some(ahead) = (index + LOOKAHEAD < census.len())
            .then(|| unit(census.occurrence(index + LOOKAHEAD)))
            .flatten()
        {
            for offset in [0, cunit_leader::TRAIT_DATA, cunit_leader::TRAIT_EXPERIENCE_DATA] {
                if let Some(address) = ahead.checked_add(offset) {
                    memory.prefetch(address, 16);
                }
            }
        }
        let occurrence = census.occurrence(index);
        let Some(pointer) = unit(occurrence) else {
            continue;
        };
        if !visit_census(job, census, index, needs) {
            return;
        }
        let id = occurrence.id();
        let mut unit_id = None;
        if !copy_unit(job, memory, base, Some(pointer), |_| id, &mut unit_id, dead)
            || !copy_trait_vectors(job, memory, base, pointer, unit_id.unwrap_or(id), dead)
        {
            return;
        }
    }
}

/// Both roots are observed before either body, preserving the shared census
/// failure boundary. Only their current, validated array geometry enters the
/// two programs; a changed dynamic header cannot retire the historical plan.
fn copy_advisor_sections(memory: &impl Memory, ctx: CharactersCopyContext, job: &mut CharactersJob) {
    use std::cell::RefCell;
    use std::sync::atomic::Ordering::Relaxed;
    struct Roots<'a, M> {
        memory: &'a M,
        log: RefCell<&'a mut crate::raw::RawLog<6>>,
    }
    impl<M: Memory> Memory for Roots<'_, M> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let mut log = self.log.borrow_mut();
            let Some(start) = log.take(self.memory, address, output.len(), MASK_ADVISORS) else { return false; };
            output.copy_from_slice(&log.bytes[start..start + output.len()]);
            true
        }
    }
    debug_assert_eq!(ctx.active, PART_MASKS[2]);
    let _reads = read_accounting::table(TableId::AdvisorRoles);
    job.clear();
    job.active = ctx.active;
    job.game_state = ctx.game_state;
    let roster = ccharacter_manager::Roster::read(&Roots {
        memory, log: RefCell::new(&mut job.advisor_roots),
    }, ctx.base, ctx.game_state);
    let Ok(roster) = roster else { return; };
    let mut dynamic = job.advisor_dynamic.take().expect("advisor section storage");
    for (index, (array, _)) in roster.sections().into_iter().enumerate() {
        if index == 1 && !job.advisor_section_complete { break; }
        let part = if index == 0 { &mut *job } else { &mut *dynamic };
        let started = std::time::Instant::now();
        copy_pass(memory, ctx, part, Some(array));
        ADVISOR_SECTION_NS[index].fetch_add(started.elapsed().as_nanos().min(u64::MAX as u128) as u64, Relaxed);
        ADVISOR_SECTION_CALLS[index].fetch_add(1, Relaxed);
        if part.log.planned_capture_succeeded() {
            ADVISOR_SECTION_PLANNED[index].fetch_add(1, Relaxed);
        }
    }
    job.advisor_dynamic = Some(dynamic);
}

fn walk_advisor_section(memory: &impl Memory, base: usize, roster: ccharacter_manager::Roster,
    advisors: &mut Batch<'_, cadvisor::Row>) {
    let result = (|| {
        let mut cursor = roster.cursor();
        let mut page = ccharacter_manager::ViewPage::default();
        while advisors.active() {
            let view = loop {
                if let Some(view) = page.next() { break Some(view?); }
                if !cursor.refill_page(memory, &mut page, [false, false, false, true])? { break None; }
            };
            let Some(view) = view else { break; };
            let entry = view.entry()?;
            advisors.append(
                |rows, bytes| cadvisor::walk_entry(&view, base, entry, rows, bytes).into(),
                |row, offset| relocate(&mut row.key_start, offset)
                    && relocate(&mut row.idea_start, offset) && relocate(&mut row.slot_start, offset),
            );
        }
        Ok::<_, &'static str>(())
    })();
    if let Err(reason) = result { advisors.fail(reason); }
}

impl Captured {
    fn install_family(&mut self, active: [bool; 6], results: [ReadResult; 6]) {
        if active[0] { self.characters.primary(results[0]); }
        if active[1] { self.leaders.primary(results[1]); }
        if active[2] { self.advisors.primary(results[2]); }
        if active[3] { self.units.primary(results[3]); }
        if active[4] { self.traits.primary(results[4]); }
        if active[5] { self.trait_experience.primary(results[5]); }
    }

    /// Failure is local to the tables owned by this acquisition job.
    pub(crate) fn fail_character_tables(&mut self, active: [bool; 6], reason: &'static str) {
        macro_rules! fail {
            ($($field:ident = $index:literal),+ $(,)?) => {$(
                if active[$index] {
                    self.$field.rows.outcome = Outcome::Unavailable { reason };
                    self.$field.bytes.len = 0;
                    self.$field.bytes.required = 0;
                    self.raw_active[$index] = false;
                }
            )+};
        }
        fail!(characters = 0, leaders = 1, advisors = 2, units = 3, traits = 4, trait_experience = 5);
    }
    fn reserve_advisors(&mut self, demand: AdvisorDemand) -> Result<(), &'static str> {
        if demand.overflowed {
            return Err("advisor capture demand overflow");
        }
        let rows = &mut self.advisors.rows.rows;
        let bytes = &mut self.advisors.bytes.first;
        rows.try_reserve(demand.rows.saturating_sub(rows.len()))
            .map_err(|_| "advisor row allocation failed")?;
        bytes
            .try_reserve(demand.bytes.saturating_sub(bytes.len()))
            .map_err(|_| "advisor byte allocation failed")?;
        if demand.rows > rows.len() {
            rows.resize(demand.rows, cadvisor::Row::default());
        }
        if demand.bytes > bytes.len() {
            bytes.resize(demand.bytes, 0);
        }
        Ok(())
    }

    /// Complete a failed persistent-character job after its typed walks were
    /// gated.
    #[cfg(test)]
    pub(crate) fn fail_characters_capture(&mut self, active: [bool; 6], reason: &'static str) {
        self.active = active;
        macro_rules! fail {
            ($($field:ident = $index:literal),+ $(,)?) => {$(
                self.$field.rows.outcome = Outcome::Unavailable {
                    reason: if active[$index] { reason } else { replay::cadence::SKIPPED_REASON },
                };
                self.$field.bytes.len = 0;
                if !active[$index] {
                    self.$field.bytes.required = 0;
                }
            )+};
        }
        fail!(
            characters = 0,
            leaders = 1,
            advisors = 2,
            units = 3,
            traits = 4,
            trait_experience = 5
        );
        self.raw_active = [false; 6];
    }

    /// Decode worker, after `decode_characters_payload`: the job keeps the
    /// template identities of the Characters rows it produced for its next
    /// copy (`TemplateCache`). A job that did not read Characters keeps its own.
    pub(crate) fn remember_templates(&self, job: &mut CharactersJob) {
        if !job.active[0] || job.advisor_cache.is_some() || job.advisor_dynamic.is_some() {
            return;
        }
        let bytes = self.characters.bytes.first.get(..self.characters.bytes.len).unwrap_or(&[]);
        job.templates
            .rebuild(self.characters.rows.outcome, &self.characters.rows.rows, bytes);
        job.template_charge.set(crate::raw_memory::Capacity {
            auxiliary_vectors: job.templates.allocated_bytes(),
            ..Default::default()
        });
    }

    /// Decode-worker interpretation of a raw refresh payload into the pooled
    /// typed rows. Outcomes, arena bytes and counts match the typed walk
    /// using recorded spans and owned trait observations. Clears its raw gate
    /// so the next capture starts clean.
    pub(crate) fn decode_characters_payload(
        &mut self,
        base: usize,
        job: &CharactersJob,
        decode_ticks: &mut [u64; 6],
    ) {
        self.active = std::array::from_fn(|index| self.active[index] || job.active[index]);
        self.fail_character_tables(self.active.map(|value| !value), replay::cadence::SKIPPED_REASON);
        if !job.active.iter().any(|&active| active) {
            return;
        }
        let started = qpc();
        if let Some(cache) = &job.advisor_cache {
            cache.decode(self);
            self.raw_active[2] = false;
            decode_ticks[2] += elapsed(started);
            return;
        }
        if let Some(dynamic) = &job.advisor_dynamic {
            let demand = AdvisorDemand {
                rows: job.advisor_demand.rows.saturating_add(dynamic.advisor_demand.rows),
                bytes: job.advisor_demand.bytes.saturating_add(dynamic.advisor_demand.bytes),
                overflowed: job.advisor_demand.overflowed || dynamic.advisor_demand.overflowed
                    || job.advisor_demand.rows.checked_add(dynamic.advisor_demand.rows).is_none()
                    || job.advisor_demand.bytes.checked_add(dynamic.advisor_demand.bytes).is_none(),
            };
            let failure = self.reserve_advisors(demand).err();
            let mut advisors = self.advisors.batch();
            if let Some(reason) = failure { advisors.fail(reason); }
            let roots = crate::raw::Recorded::build(&job.advisor_roots);
            match ccharacter_manager::Roster::read(&roots, base, job.game_state) {
                Ok(roster) => for ((_, section), payload) in roster.sections().into_iter().zip([job, &**dynamic]) {
                    if !advisors.active() { break; }
                    let recorded = crate::raw::Recorded::build(&payload.log);
                    walk_advisor_section(&recorded, base, section, &mut advisors);
                },
                Err(reason) => advisors.fail(reason),
            }
            let result = advisors.result();
            self.advisors.primary(result);
            self.raw_active[2] = false;
            decode_ticks[2] += elapsed(started);
            return;
        }
        let advisor_failure = if job.active[2] {
            self.reserve_advisors(job.advisor_demand).err()
        } else {
            None
        };
        let recorded = crate::raw::Recorded::build(&job.log);
        let mut family = self.family().select(job.active);
        if let Some(reason) = advisor_failure {
            family.advisors.fail(reason);
        }
        let results = if job.census_capture {
            walk_census(&recorded, base, family, Some(&job.traits), &job.census_visited, job.census_historical)
        } else {
            walk_family(&recorded, base, job.game_state, family, Some(&job.traits), Some(&job.templates))
        };
        self.install_family(job.active, results);
        decode_ticks[anchor(job.active)] += elapsed(started);
        for (raw, active) in self.raw_active.iter_mut().zip(job.active) {
            if active { *raw = false; }
        }
    }
}

#[cfg(test)]
mod character_target_hint_tests {
    use super::*;
    use std::cell::RefCell;

    fn store_field(job: &mut CharactersJob, view: &mut ViewCopy, field: usize, bytes: &[u8]) {
        assert_eq!(bytes.len(), VIEW_LENS[field]);
        let memory = engine::memory::SliceMemory { base: 0x1000, bytes };
        let start = job
            .log
            .take(&memory, 0x1000, bytes.len(), VIEW_MASKS[field])
            .unwrap();
        view.starts[field] = u32::try_from(start).unwrap();
    }

    fn view(
        leader: usize,
        leader_count: u64,
        advisor: usize,
        advisor_count: u64,
        unit: usize,
    ) -> (CharactersJob, ViewCopy) {
        let mut job = CharactersJob {
            log: crate::raw::RawLog::new(3, 40),
            ..CharactersJob::default()
        };
        let mut view = ViewCopy::default();
        for (field, pointer, count) in [(4, leader, leader_count), (6, advisor, advisor_count)] {
            let mut header = [0; 16];
            header[..8].copy_from_slice(&(pointer as u64).to_le_bytes());
            header[8..16].copy_from_slice(&count.to_le_bytes());
            store_field(&mut job, &mut view, field, &header);
        }
        store_field(&mut job, &mut view, 5, &(unit as u64).to_le_bytes());
        (job, view)
    }

    fn targets(source: &(CharactersJob, ViewCopy), dead: &Dead) -> Vec<(usize, usize)> {
        let mut hints = Vec::new();
        hint_character_targets(&source.0, &source.1, dead, |address, len| hints.push((address, len)));
        hints
    }

    fn heap_header(source: usize, len: usize, capacity: usize) -> [u8; 32] {
        let mut header = [0; 32];
        header[..8].copy_from_slice(&(source as u64).to_le_bytes());
        header[16..24].copy_from_slice(&(len as u64).to_le_bytes());
        header[24..32].copy_from_slice(&(capacity as u64).to_le_bytes());
        header
    }

    fn literal_hints(header: &[u8; 32]) -> Option<Vec<(usize, usize)>> {
        let literal = parse_character_literal(header, 0x1000)?;
        let mut hints = Vec::new();
        hint_character_literal(&literal, |address, len| hints.push((address, len)));
        Some(hints)
    }

    #[test]
    fn staged_name_hints_require_both_valid_headers_and_keep_inline_values_owned() {
        let mut job = CharactersJob { log: crate::raw::RawLog::new(4, 256), ..Default::default() };
        let mut view = ViewCopy::default();
        let mut headers = [0; 64];
        headers[..32].copy_from_slice(&heap_header(0x2038, 20, 20));
        headers[32..].copy_from_slice(&heap_header(0x3000, 0, 16));
        store_field(&mut job, &mut view, 3, &headers);
        let mut source = (job, view);
        assert_eq!(targets(&source, &Dead::default()), [(0x204c, 1), (0x2038, 1), (0x3000, 1)]);
        assert!(targets(&source, &Dead { characters: true, ..Default::default() }).is_empty());
        headers[56..64].copy_from_slice(&14_u64.to_le_bytes());
        store_field(&mut source.0, &mut source.1, 3, &headers);
        assert!(targets(&source, &Dead::default()).is_empty(), "invalid second header must suppress both hints");
        headers.fill(0);
        headers[24..32].copy_from_slice(&15_u64.to_le_bytes());
        headers[56..64].copy_from_slice(&15_u64.to_le_bytes());
        store_field(&mut source.0, &mut source.1, 3, &headers);
        assert!(targets(&source, &Dead::default()).is_empty());
    }

    #[test]
    fn character_target_hints_obey_independent_dead_guards_and_empty_maps() {
        for bits in 0..64 {
            let dead = Dead {
                characters: bits & 1 != 0,
                leaders: bits & 2 != 0,
                advisors: bits & 4 != 0,
                units: bits & 8 != 0,
                traits: bits & 16 != 0,
                experience: bits & 32 != 0,
            };
            for count in [0, 1, u64::from(u32::MAX)] {
                let mut expected = Vec::new();
                if !dead.leaders {
                    expected.push((0x2000, 1));
                }
                if !dead.advisors {
                    expected.push((0x3000, 1));
                }
                if !(dead.units && dead.traits && dead.experience) {
                    expected.push((0x4000, 1));
                }
                if !dead.units { expected.extend([(0x4040, 1), (0x4e60, 1), (0x4f48, 1)]); }
                assert_eq!(
                    targets(&view(0x2000, count, 0x3000, count, 0x4000), &dead),
                    expected
                );
            }
        }
        assert!(targets(&view(0, 0, 0, 0, 0), &Dead::default()).is_empty());
        assert_eq!(
            targets(&view(0x2000, u64::MAX, 0x3000, 0, 0x4000), &Dead::default()),
            [(0x3000, 1), (0x4000, 1), (0x4040, 1), (0x4e60, 1), (0x4f48, 1)]
        );
        // Aliased sentinels remain two independently requested hints.
        assert_eq!(
            targets(&view(0x2000, 0, 0x2000, 0, 0x2000), &Dead::default()),
            [(0x2000, 1), (0x2000, 1), (0x2000, 1), (0x2040, 1), (0x2e60, 1), (0x2f48, 1)]
        );
    }

    #[test]
    fn character_target_hints_reject_null_overflow_and_canonical_gap_ranges() {
        let hint = |address, len| {
            let mut result = Vec::new();
            hint_character_target(address, len, &mut |at, width| result.push((at, width)));
            result
        };
        for (address, len) in [
            (0, 16),
            (usize::MAX, 16),
            (usize::MAX - 15, 16),
            (0x0000_8000_0000_0000, 16),
            (0xffff_7fff_ffff_ffff, 16),
            (0x0000_7fff_ffff_fff0, 26),
        ] {
            assert!(hint(address, len).is_empty(), "{address:x}+{len}");
        }
        for (address, len) in [
            (0x200f, 26),
            (0x0000_7fff_ffff_ffe0, 26),
            (0xffff_8000_0000_0000, 16),
            (usize::MAX - 16, 16),
        ] {
            assert_eq!(hint(address, len), [(address, 1)]);
        }
        for field in [4, 5, 6] {
            let mut source = view(0x2000, 0, 0x3000, 0, 0x4000);
            let start = source.1.starts[field] as usize;
            source.0.log.bytes[start..start + 8].copy_from_slice(&u64::MAX.to_le_bytes());
            let hinted = targets(&source, &Dead::default());
            assert_eq!(hinted.len(), if field == 5 { 2 } else { 5 });
            assert!(!hinted.iter().any(|&(address, _)| address == usize::MAX));
        }
    }

    #[test]
    fn character_heap_hints_cover_only_first_and_terminator_lines() {
        for alignment in 0..64 {
            for len in [0, 1, 15, 16, 17, 63, 64, 65, 128] {
                let source = 0x2000 + alignment;
                let hints = literal_hints(&heap_header(source, len, len.max(16))).unwrap();
                let end = source + len;
                let mut expected = vec![(end, 1)];
                if len != 0 && source / 64 != end / 64 {
                    expected.push((source, 1));
                }
                assert_eq!(hints, expected, "alignment={alignment}, len={len}");
            }
        }
        let mut inline = [0u8; 32];
        inline[..3].copy_from_slice(b"abc");
        inline[16..24].copy_from_slice(&3u64.to_le_bytes());
        inline[24..32].copy_from_slice(&15u64.to_le_bytes());
        assert_eq!(literal_hints(&inline), Some(Vec::new()));
        inline[3] = 1;
        assert!(literal_hints(&inline).is_none());
        for header in [
            heap_header(0, 17, 17),
            heap_header(0x2000, 17, 16),
            heap_header(0x2000, 0, 14),
        ] {
            assert!(literal_hints(&header).is_none());
        }
        for header in [
            heap_header(usize::MAX - 15, 16, 16),
            heap_header(0x0000_7fff_ffff_fff0, 17, 17),
            heap_header(0x0000_8000_0000_0000, 17, 17),
        ] {
            assert_eq!(literal_hints(&header), Some(Vec::new()));
        }
    }

    struct TraceMemory<'a> {
        memory: engine::memory::SliceMemory<'a>,
        failed_call: Option<usize>,
        calls: RefCell<Vec<(usize, usize)>>,
    }

    impl Memory for TraceMemory<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let index = self.calls.borrow().len();
            self.calls.borrow_mut().push((address, output.len()));
            if self.failed_call == Some(index) {
                output.fill(0xe7);
                return false;
            }
            self.memory.copy(address, output)
        }
    }

    #[test]
    fn character_fresh_view_hints_observe_no_targets_and_ignore_failed_fields() {
        use super::tests::{ADDRESS, CHARACTERS, IMAGE, UNITS, fixture};
        let bytes = fixture();
        let pointer = ADDRESS + CHARACTERS[0];
        for failed_call in [
            None,
            Some(0),
            Some(1),
            Some(2),
            Some(3),
            Some(4),
            Some(5),
            Some(6),
        ] {
            let memory = TraceMemory {
                memory: engine::memory::SliceMemory {
                    base: ADDRESS,
                    bytes: &bytes,
                },
                failed_call,
                calls: RefCell::new(Vec::new()),
            };
            let mut job = CharactersJob {
                log: crate::raw::RawLog::new(16, 256),
                game_state: ADDRESS,
                active: [true; 6],
                ..CharactersJob::default()
            };
            let copied = copy_view(&mut job, &memory, IMAGE, pointer);
            let calls = memory.calls.borrow().clone();
            let expected_calls: Vec<_> = [0, 0x68, 0x18, 0x28, 0x98, 0xa8, 0xb0]
                .into_iter()
                .zip([16, 4, 16, 64, 16, 8, 16])
                .take(if failed_call == Some(0) { 1 } else { 7 })
                .map(|(offset, len)| (pointer + offset, len))
                .collect();
            assert_eq!(calls, expected_calls);
            let Some(copied) = copied else {
                assert_eq!(failed_call, Some(0));
                continue;
            };
            let spans = job.log.spans.clone();
            let used = job.log.bytes.as_ref().to_vec();
            let mut expected = Vec::new();
            if failed_call != Some(4) {
                expected.push((ADDRESS + 0x2400, 1));
            }
            if failed_call != Some(6) {
                expected.push((ADDRESS + 0x2430, 1));
            }
            if failed_call != Some(5) {
                expected.push((ADDRESS + UNITS[0], 1));
                expected.extend([(ADDRESS + UNITS[0] + 0x40, 1), (ADDRESS + UNITS[0] + 0xe60, 1), (ADDRESS + UNITS[0] + 0xf48, 1)]);
            }
            let mut hints = Vec::new();
            hint_character_targets(&job, &copied, &Dead::default(), |address, len| {
                hints.push((address, len));
            });
            assert_eq!(hints, expected);
            assert_eq!(*memory.calls.borrow(), calls, "hint observed a target");
            assert_eq!(job.log.spans, spans);
            assert_eq!(job.log.bytes.as_ref(), used);

            // Failed partial bytes may be reused by a later successful field.
            // Only successful offsets are visible, both before and after append
            // and off-capture growth move the raw arena.
            let fields: Vec<_> = (0..VIEW_LENS.len())
                .map(|index| copied.field(&job, index).to_vec())
                .collect();
            for (index, field) in fields.iter().enumerate() {
                if failed_call == Some(index) {
                    assert_eq!(copied.starts[index], u32::MAX);
                    assert_eq!(field, &vec![0; VIEW_LENS[index]]);
                } else {
                    let start = CHARACTERS[0] + VIEW_OFFSETS[index];
                    assert_ne!(copied.starts[index], u32::MAX);
                    assert_eq!(field, &bytes[start..start + VIEW_LENS[index]]);
                }
            }
            let extension = [0xa5; 512];
            let owned = engine::memory::SliceMemory { base: 0x8000, bytes: &extension };
            assert!(job.log.take(&owned, 0x8000, 64, MASK_CHARACTERS).is_some());
            assert!(job.log.take(&owned, 0x8000, 512, MASK_CHARACTERS).is_none());
            assert!(job.log.overflowed);
            job.log.grow();
            for (index, field) in fields.iter().enumerate() {
                assert_eq!(copied.field(&job, index), field);
            }
        }
        // Zero-length inactive scatter slots may have the same raw offset as
        // the next successful field. They must never publish that offset.
        for active in [
            [true, false, false, true, true, true],
            [false, true, false, false, false, false],
        ] {
            let memory = engine::memory::SliceMemory { base: ADDRESS, bytes: &bytes };
            let mut job = CharactersJob {
                log: crate::raw::RawLog::new(16, 256),
                active,
                ..CharactersJob::default()
            };
            let copied = copy_view(&mut job, &memory, IMAGE, pointer).unwrap();
            let source_mask = job.source_mask();
            for index in 0..VIEW_LENS.len() {
                if VIEW_MASKS[index] & source_mask == 0 {
                    assert_eq!(copied.starts[index], u32::MAX);
                    assert!(copied.field(&job, index).iter().all(|&byte| byte == 0));
                } else {
                    let start = CHARACTERS[0] + VIEW_OFFSETS[index];
                    assert_eq!(copied.field(&job, index), &bytes[start..start + VIEW_LENS[index]]);
                }
            }
        }
        // The descriptor and its optional unit ID replace the 128-byte image.
        assert_eq!(size_of::<ViewCopy>(), 36);
    }

    // Frozen pre-hint text path: independent caller sequencing, with the same
    // unchanged literal parser and exact copy primitive.
    fn original_text(
        job: &mut CharactersJob,
        memory: &impl Memory,
        view: &ViewCopy,
        pointer: usize,
        dead: &mut Dead,
    ) {
        if dead.characters {
            return;
        }
        let name_addr = pointer.checked_add(ccharacter::NAME).unwrap_or(0);
        let token_addr = pointer.checked_add(ccharacter::TOKEN).unwrap_or(0);
        let headers = view.field(job, 3);
        let name = parse_character_literal(&headers[..32], name_addr);
        let token = parse_character_literal(&headers[32..64], token_addr);
        let (Some(name), Some(token)) = (name, token) else {
            dead.characters = true;
            return;
        };
        if !copy_character_literal_bytes(job, memory, &name, MASK_CHARACTERS)
            || !copy_character_literal_bytes(job, memory, &token, MASK_CHARACTERS)
        {
            dead.characters = true;
        }
    }

    #[test]
    fn character_heap_hints_preserve_exact_observations_and_failure_prefix() {
        let bytes = [0u8; 256];
        let expected_reads = [
            (0x2000 + 17, 1),
            (0x2000, 17),
            (0x2080 + 65, 1),
            (0x2080, 65),
        ];
        for failed_call in [None, Some(0), Some(1), Some(2), Some(3)] {
            for malformed_token in [false, true] {
                for already_dead in [false, true] {
                    let mut headers = [0; 64];
                    headers[..32].copy_from_slice(&heap_header(0x2000, 17, 17));
                    headers[32..64].copy_from_slice(&heap_header(
                        0x2080,
                        65,
                        if malformed_token { 64 } else { 65 },
                    ));
                    let memory = || TraceMemory {
                        memory: engine::memory::SliceMemory {
                            base: 0x2000,
                            bytes: &bytes,
                        },
                        failed_call,
                        calls: RefCell::new(Vec::new()),
                    };
                    let mut actual = CharactersJob {
                        // One owned view header leaves the former payload budget.
                        log: crate::raw::RawLog::new(9, 192),
                        game_state: 0,
                        ..CharactersJob::default()
                    };
                    let mut expected = CharactersJob {
                        // One owned view header leaves the former payload budget.
                        log: crate::raw::RawLog::new(9, 192),
                        game_state: 0,
                        ..CharactersJob::default()
                    };
                    let mut source = ViewCopy::default();
                    let mut expected_source = ViewCopy::default();
                    store_field(&mut actual, &mut source, 3, &headers);
                    store_field(&mut expected, &mut expected_source, 3, &headers);
                    let mut dead = Dead {
                        characters: already_dead,
                        ..Dead::default()
                    };
                    let mut expected_dead = Dead {
                        characters: already_dead,
                        ..Dead::default()
                    };
                    let actual_memory = memory();
                    let expected_memory = memory();
                    copy_character_text(&mut actual, &actual_memory, &source, 0x1000, &mut dead);
                    original_text(
                        &mut expected,
                        &expected_memory,
                        &expected_source,
                        0x1000,
                        &mut expected_dead,
                    );
                    assert_eq!(
                        *actual_memory.calls.borrow(),
                        *expected_memory.calls.borrow()
                    );
                    let count = if malformed_token || already_dead {
                        0
                    } else {
                        failed_call.map_or(4, |index| index + 1)
                    };
                    assert_eq!(
                        actual_memory.calls.borrow().as_slice(),
                        &expected_reads[..count]
                    );
                    assert_eq!(actual.log.spans, expected.log.spans);
                    assert_eq!(actual.log.bytes.as_ref(), expected.log.bytes.as_ref());
                    assert_eq!(actual.log.fault, expected.log.fault);
                    assert_eq!(actual.log.raw_bytes, expected.log.raw_bytes);
                    assert_eq!(actual.log.overflowed, expected.log.overflowed);
                    assert_eq!(dead.characters, expected_dead.characters);
                }
            }
        }
    }
}

/// Shared synthetic address space for the characters copy/decode split
/// harness: role maps with leader/advisor nodes, heap and inline strings,
/// skill definitions, army and navy kinds, and a third navy character.
#[cfg(test)]
pub(crate) mod characters_test_kit {
    use super::tests::{
        ADDRESS, CHARACTERS, IMAGE, MANAGER, ROSTER, UNITS, fixture, literal, pointer, put,
    };
    use super::{
        Buffer, Capacities, Captured, CharactersCopyContext, CharactersCopyReport, CharactersJob,
        copy_characters_refresh, walk_family,
    };
    use engine::{
        cadvisor, carmy_leader, ccharacter, ccharacter_manager, ccountry_leader,
        character_advisor_node, character_leader_node, cnavy_leader, cunit_leader,
        cunit_leader_skill, msvc_string, msvc_tree_node,
    };

    pub(crate) const BASE: usize = IMAGE;
    pub(crate) const GAME_STATE: usize = ADDRESS;

    pub(crate) const LEAD_NODE: usize = 0x8100;
    pub(crate) const ADV_NODE: usize = 0x8200;
    pub(crate) const HEAP_AREA: usize = 0x8300;
    pub(crate) const SKILL0: usize = 0x7900;
    const LEAD_ROLE: usize = 0x8140;
    pub(crate) const ADV_ROLE: usize = 0x8260;
    const CHAR2: usize = 0x2300;
    const UNIT2: usize = 0x5200;
    const HEAD2L: usize = 0x2500;
    const HEAD2A: usize = 0x2530;

    pub(crate) fn memory(bytes: &[u8]) -> engine::memory::SliceMemory<'_> {
        engine::memory::SliceMemory {
            base: ADDRESS,
            bytes,
        }
    }

    fn heap_string(bytes: &mut [u8], header: usize, payload: usize, value: &[u8]) {
        assert!(value.len() > msvc_string::INLINE_CAPACITY as usize);
        put(bytes, header, ((ADDRESS + payload) as u64).to_le_bytes());
        put(bytes, header + 16, (value.len() as u64).to_le_bytes());
        put(bytes, header + 24, (value.len() as u64).to_le_bytes());
        bytes[payload..payload + value.len()].copy_from_slice(value);
        bytes[payload + value.len()] = 0;
    }

    fn empty_map(bytes: &mut [u8], character: usize, offset: usize, head: usize) {
        pointer(bytes, character + offset, head);
        for link in [0, 8, 16] {
            pointer(bytes, head + link, head);
        }
        bytes[head + msvc_tree_node::NIL] = 1;
    }

    /// Base fixture plus one leader role, one heap-keyed advisor role, a
    /// valid skill on unit 0, army kind on unit 1, and a third navy character
    /// with a heap name.
    pub(crate) fn mapped_fixture() -> Vec<u8> {
        let mut bytes = fixture();
        put(
            &mut bytes,
            MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS + 8,
            3_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS + 12,
            3_u32.to_le_bytes(),
        );
        pointer(&mut bytes, ROSTER + 16, CHAR2);
        // Single leader role on character 0 with ideology key 7. The map
        // count lives in the character's map header, not the sentinel.
        put(
            &mut bytes,
            CHARACTERS[0] + ccharacter::LEADER_MAP_HEAD + 8,
            1_u64.to_le_bytes(),
        );
        pointer(&mut bytes, 0x2400, LEAD_NODE);
        pointer(&mut bytes, LEAD_NODE, 0x2400);
        pointer(&mut bytes, LEAD_NODE + 8, 0x2400);
        pointer(&mut bytes, LEAD_NODE + 16, 0x2400);
        bytes[LEAD_NODE + msvc_tree_node::NIL] = 0;
        put(
            &mut bytes,
            LEAD_NODE + character_leader_node::IDEOLOGY,
            7_i32.to_le_bytes(),
        );
        pointer(
            &mut bytes,
            LEAD_NODE + character_leader_node::LEADER,
            LEAD_ROLE,
        );
        put(
            &mut bytes,
            LEAD_ROLE,
            ((IMAGE + ccountry_leader::VTABLE_RVA) as u64).to_le_bytes(),
        );
        pointer(
            &mut bytes,
            LEAD_ROLE + ccountry_leader::CHARACTER,
            CHARACTERS[0],
        );
        // Single advisor role on character 0 with a 20-byte heap key.
        put(
            &mut bytes,
            CHARACTERS[0] + ccharacter::ADVISOR_MAP_HEAD + 8,
            1_u64.to_le_bytes(),
        );
        pointer(&mut bytes, 0x2430, ADV_NODE);
        pointer(&mut bytes, ADV_NODE, 0x2430);
        pointer(&mut bytes, ADV_NODE + 8, 0x2430);
        pointer(&mut bytes, ADV_NODE + 16, 0x2430);
        bytes[ADV_NODE + msvc_tree_node::NIL] = 0;
        pointer(
            &mut bytes,
            ADV_NODE + character_advisor_node::ADVISOR,
            ADV_ROLE,
        );
        heap_string(
            &mut bytes,
            ADV_NODE + character_advisor_node::KEY,
            HEAP_AREA,
            b"twenty-byte-advisor!",
        );
        put(
            &mut bytes,
            ADV_NODE + character_advisor_node::KEY_HASH,
            1234_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            ADV_ROLE,
            ((IMAGE + cadvisor::VTABLE_RVA) as u64).to_le_bytes(),
        );
        pointer(&mut bytes, ADV_ROLE + cadvisor::CHARACTER, CHARACTERS[0]);
        literal(&mut bytes, ADV_ROLE + cadvisor::IDEA_TOKEN, b"idea");
        literal(&mut bytes, ADV_ROLE + cadvisor::SLOT, b"slot");
        // Valid skill definition on unit 0.
        put(
            &mut bytes,
            SKILL0,
            ((IMAGE + cunit_leader_skill::VTABLE_RVA) as u64).to_le_bytes(),
        );
        bytes[SKILL0 + cunit_leader_skill::VALID] = 1;
        put(
            &mut bytes,
            SKILL0 + cunit_leader_skill::LEVEL,
            3_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            SKILL0 + cunit_leader_skill::LEVEL + 4,
            100_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            SKILL0 + cunit_leader_skill::LEVEL + 8,
            5_i32.to_le_bytes(),
        );
        pointer(&mut bytes, UNITS[0] + cunit_leader::SKILL, SKILL0);
        // Army kind with components on unit 1.
        put(
            &mut bytes,
            UNITS[1],
            ((IMAGE + carmy_leader::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            UNITS[1] + carmy_leader::ATTACK,
            11_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            UNITS[1] + carmy_leader::DEFENSE,
            22_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            UNITS[1] + carmy_leader::PLANNING,
            33_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            UNITS[1] + carmy_leader::LOGISTICS,
            44_i32.to_le_bytes(),
        );
        // Third character with a heap name and a navy unit.
        put(
            &mut bytes,
            CHAR2,
            ((IMAGE + ccharacter::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            CHAR2 + ccharacter::TYPE,
            ccharacter::REFERENCE_TYPE.to_le_bytes(),
        );
        put(&mut bytes, CHAR2 + ccharacter::ID, 42_u32.to_le_bytes());
        put(&mut bytes, CHAR2 + ccharacter::COUNTRY, 3_u32.to_le_bytes());
        pointer(&mut bytes, CHAR2 + ccharacter::UNIT_LEADER, UNIT2);
        heap_string(
            &mut bytes,
            CHAR2 + ccharacter::NAME,
            HEAP_AREA + 64,
            b"heap-named-character!",
        );
        literal(&mut bytes, CHAR2 + ccharacter::TOKEN, b"tok");
        empty_map(&mut bytes, CHAR2, ccharacter::LEADER_MAP_HEAD, HEAD2L);
        empty_map(&mut bytes, CHAR2, ccharacter::ADVISOR_MAP_HEAD, HEAD2A);
        put(
            &mut bytes,
            UNIT2,
            ((IMAGE + cnavy_leader::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            UNIT2 + cunit_leader::TYPE,
            cunit_leader::REFERENCE_TYPE.to_le_bytes(),
        );
        put(&mut bytes, UNIT2 + cunit_leader::ID, 42_u32.to_le_bytes());
        put(
            &mut bytes,
            UNIT2 + cunit_leader::CHARACTER_TYPE,
            ccharacter::REFERENCE_TYPE.to_le_bytes(),
        );
        put(
            &mut bytes,
            UNIT2 + cunit_leader::CHARACTER_ID,
            42_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            UNIT2 + cunit_leader::EXPERIENCE,
            999_i64.to_le_bytes(),
        );
        literal(&mut bytes, UNIT2 + cunit_leader::TOKEN, b"n2");
        literal(&mut bytes, UNIT2 + cunit_leader::NAME, b"m2");
        put(
            &mut bytes,
            UNIT2 + cunit_leader::TRAIT_DATA,
            0_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            UNIT2 + cunit_leader::TRAIT_EXPERIENCE_DATA,
            0_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            UNIT2 + cnavy_leader::ATTACK,
            51_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            UNIT2 + cnavy_leader::DEFENSE,
            52_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            UNIT2 + cnavy_leader::MANEUVERING,
            53_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            UNIT2 + cnavy_leader::COORDINATION,
            54_i32.to_le_bytes(),
        );
        bytes
    }

    /// Reference typed walk with identical pooled capacities, outcomes and
    /// arenas installed exactly like `capture` installs them.
    pub(crate) fn direct(memory: &impl engine::Memory, capacities: &Capacities) -> Captured {
        let mut captured = Captured::with_capacities(capacities);
        let results = walk_family(memory, BASE, GAME_STATE, captured.family(), None, None);
        captured.characters.primary(results[0]);
        captured.leaders.primary(results[1]);
        captured.advisors.primary(results[2]);
        captured.units.primary(results[3]);
        captured.traits.primary(results[4]);
        captured.trait_experience.primary(results[5]);
        captured
    }

    /// Capture one owned payload and decode into fresh pooled buffers.
    pub(crate) fn split(
        memory: &impl engine::Memory,
        capacities: &Capacities,
    ) -> (Captured, CharactersCopyReport, CharactersJob) {
        let mut captured = Captured::with_capacities(capacities);
        let (trait_rows, trait_bytes) = captured.trait_capture_capacities();
        let ctx = CharactersCopyContext {
            force_full: false,
            base: BASE,
            game_state: GAME_STATE,
            active: [true; 6],
            trait_rows,
            trait_bytes,
        };
        let mut first = CharactersJob::new();
        let report = copy_characters_refresh(memory, ctx, &mut first);
        assert!(report.raw_ok);
        let mut ticks = [0u64; 6];
        captured.decode_characters_payload(BASE, &first, &mut ticks);
        (captured, report, first)
    }

    fn assert_buffer<R: PartialEq + std::fmt::Debug>(
        name: &str,
        split: &Buffer<R>,
        direct: &Buffer<R>,
    ) {
        assert_eq!(split.rows.outcome, direct.rows.outcome, "{name} outcome");
        let len = split.rows.outcome.len();
        assert_eq!(direct.rows.outcome.len(), len, "{name} length");
        assert_eq!(
            &split.rows.rows[..len],
            &direct.rows.rows[..len],
            "{name} rows"
        );
        assert_eq!(
            &split.bytes.first[..split.bytes.len],
            &direct.bytes.first[..direct.bytes.len],
            "{name} arena"
        );
        assert_eq!(
            split.bytes.required, direct.bytes.required,
            "{name} required bytes"
        );
    }

    pub(crate) fn assert_matches_direct(split: &Captured, direct: &Captured) {
        assert_buffer("characters", &split.characters, &direct.characters);
        assert_buffer("leaders", &split.leaders, &direct.leaders);
        assert_buffer("advisors", &split.advisors, &direct.advisors);
        assert_buffer("units", &split.units, &direct.units);
        assert_buffer("traits", &split.traits, &direct.traits);
        assert_buffer(
            "experience",
            &split.trait_experience,
            &direct.trait_experience,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::{cgame_state, cunit_leader_trait, memory::SliceMemory};
    use std::cell::Cell;

    fn section_job() -> CharactersJob {
        let mut job = CharactersJob::with_storage(2, 2048, 32768, false);
        job.advisor_roots = crate::raw::RawLog::new(4, 48);
        job.advisor_dynamic = Some(Box::new(CharactersJob::with_storage(2, 2048, 32768, false)));
        job
    }

    fn section_context() -> CharactersCopyContext {
        let capacities = split_capacities();
        CharactersCopyContext {
            force_full: false,
            base: IMAGE, game_state: ADDRESS, active: PART_MASKS[2],
            trait_rows: [capacities.rows[4], capacities.rows[5]],
            trait_bytes: [capacities.bytes[4], capacities.bytes[5]],
        }
    }

    pub(super) fn section_fixture() -> Vec<u8> {
        let mut bytes = characters_test_kit::mapped_fixture();
        array(&mut bytes, MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS, ROSTER, 1);
        array(&mut bytes, MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS, 0x8800, 1);
        pointer(&mut bytes, 0x8800, CHARACTERS[1]);
        bytes
    }

    fn direct_advisors(memory: &impl Memory) -> Captured {
        let mut captured = Captured::with_capacities(&split_capacities());
        let result = walk_family(memory, IMAGE, ADDRESS, captured.family().select(PART_MASKS[2]), None, None);
        captured.advisors.primary(result[2]);
        captured
    }

    fn assert_advisors_match(job: &CharactersJob, memory: &impl Memory) {
        let mut captured = Captured::with_capacities(&split_capacities());
        captured.decode_characters_payload(IMAGE, job, &mut [0; 6]);
        let direct = direct_advisors(memory);
        assert_eq!(captured.advisors.rows.outcome, direct.advisors.rows.outcome);
        let len = direct.advisors.rows.outcome.len();
        assert_eq!(captured.advisors.rows.rows[..len], direct.advisors.rows.rows[..len]);
        assert_eq!(captured.advisors.bytes.first[..captured.advisors.bytes.len],
            direct.advisors.bytes.first[..direct.advisors.bytes.len]);
        assert_eq!(captured.advisors.bytes.required, direct.advisors.bytes.required);
    }

    #[test]
    fn advisor_roster_growth_rebuilds_only_its_section_and_reads_fresh_inactive_roles() {
        let mut bytes = section_fixture();
        let mut job = section_job();
        for _ in 0..2 {
            assert!(copy_characters_refresh(&characters_test_kit::memory(&bytes), section_context(), &mut job).raw_ok);
            assert_advisors_match(&job, &characters_test_kit::memory(&bytes));
            job.prepare_read_plans();
            job.clear();
        }
        // The advisor belongs to an unrecruited character. Both country and
        // heap string values must remain fresh without rebuilding its geometry.
        put(&mut bytes, CHARACTERS[0] + ccharacter::COUNTRY, 0_u32.to_le_bytes());
        bytes[characters_test_kit::HEAP_AREA] = b'X';
        array(&mut bytes, MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS, 0x8800, 2);
        pointer(&mut bytes, 0x8808, 0x2300);
        assert!(copy_characters_refresh(&characters_test_kit::memory(&bytes), section_context(), &mut job).raw_ok);
        assert!(job.log.planned_capture_succeeded());
        assert!(!job.advisor_dynamic.as_ref().unwrap().log.planned_capture_succeeded());
        assert_advisors_match(&job, &characters_test_kit::memory(&bytes));
        job.prepare_read_plans();
        job.clear();
        // An in-place historical string-header change leaves the generated
        // roster's program valid; reuse never substitutes yesterday's string.
        literal(&mut bytes, characters_test_kit::ADV_ROLE + cadvisor::IDEA_TOKEN, b"changed");
        assert!(copy_characters_refresh(&characters_test_kit::memory(&bytes), section_context(), &mut job).raw_ok);
        assert!(!job.log.planned_capture_succeeded());
        assert!(job.advisor_dynamic.as_ref().unwrap().log.planned_capture_succeeded());
        assert_advisors_match(&job, &characters_test_kit::memory(&bytes));
        job.prepare_read_plans();
        job.clear();
        // Relocation, emptying and A -> B -> A do not reuse obsolete pointers.
        for (data, count) in [(0x8900, 1), (0x8900, 0), (0x8800, 2)] {
            array(&mut bytes, MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS, data, count);
            pointer(&mut bytes, data, CHARACTERS[1]);
            assert!(copy_characters_refresh(&characters_test_kit::memory(&bytes), section_context(), &mut job).raw_ok);
            assert!(job.log.planned_capture_succeeded());
            assert_advisors_match(&job, &characters_test_kit::memory(&bytes));
            job.prepare_read_plans();
            job.clear();
        }
    }

    #[test]
    fn advisor_sections_preserve_alias_occurrences_and_first_fault_without_live_retry() {
        let mut bytes = section_fixture();
        pointer(&mut bytes, 0x8800, CHARACTERS[0]);
        let trace = WaveMemory::new(&bytes, None);
        let direct = direct_advisors(&trace);
        assert_eq!(direct.advisors.rows.outcome, Outcome::Rows { len: 2 });
        let mut faults = std::collections::BTreeSet::new();
        for &(address, width) in trace.reads.borrow().iter() {
            if width != 0 { faults.extend([address, address + width - 1]); }
        }
        for fault in faults.into_iter().map(Some).chain([None]) {
            for warm in [false, true] {
                let mut job = section_job();
                if warm {
                    assert!(copy_characters_refresh(&characters_test_kit::memory(&bytes), section_context(), &mut job).raw_ok);
                    job.prepare_read_plans(); job.clear();
                }
                let source = WaveMemory::new(&bytes, fault);
                assert!(copy_characters_refresh(&source, section_context(), &mut job).raw_ok);
                assert_advisors_match(&job, &WaveMemory::new(&bytes, fault));
                if let Some(fault) = fault {
                    assert_eq!(source.reads.borrow().iter().filter(|&&(a, n)| (a..a + n).contains(&fault)).count(), 1,
                        "failed occurrence retried at {fault:x}, warm={warm}");
                }
            }
        }
    }

    #[test]
    fn advisor_sections_keep_original_roster_ids_and_stop_at_semantic_failures() {
        let original = section_fixture();
        let memory = characters_test_kit::memory(&original);
        let roster = ccharacter_manager::Roster::read(&memory, IMAGE, ADDRESS).unwrap();
        for (index, (_, section)) in roster.sections().into_iter().enumerate() {
            let entry = section.cursor().next(&memory, [false, false, false, true])
                .unwrap().unwrap().entry().unwrap();
            assert_eq!((entry.roster, entry.ordinal, entry.pointer), (index as u8, 0, ADDRESS + CHARACTERS[index]));
        }
        for warm in [false, true] {
            for invalid_historical in [false, true] {
                let mut job = section_job();
                if warm {
                    assert!(copy_characters_refresh(&memory, section_context(), &mut job).raw_ok);
                    job.prepare_read_plans(); job.clear();
                }
                let mut bytes = original.clone();
                pointer(&mut bytes, CHARACTERS[1], 0);
                if invalid_historical {
                    pointer(&mut bytes, characters_test_kit::ADV_ROLE, 0);
                }
                let source = WaveMemory::new(&bytes, None);
                assert!(copy_characters_refresh(&source, section_context(), &mut job).raw_ok);
                assert_advisors_match(&job, &source);
                if invalid_historical {
                    assert!(job.advisor_dynamic.as_ref().unwrap().log.spans.is_empty());
                }
            }
        }
    }

    #[test]
    fn later_advisor_section_overflow_cannot_retry_an_earlier_prepared_source() {
        let bytes = section_fixture();
        let mut job = section_job();
        assert!(copy_characters_refresh(&characters_test_kit::memory(&bytes), section_context(), &mut job).raw_ok);
        job.prepare_read_plans(); job.clear();
        job.advisor_dynamic.as_mut().unwrap().log = crate::raw::RawLog::new(1, 1);
        let report = copy_characters_refresh(&characters_test_kit::memory(&bytes), section_context(), &mut job);
        assert!(job.log.planned_capture_succeeded());
        assert!(!report.raw_ok && report.plan_failed);
        assert!(job.grow_for_next());
        job.grow(); job.prepare_read_plans(); job.clear();
        assert!(job.advisor_dynamic.as_ref().unwrap().log.bytes.capacity() >= 8);
    }

    #[test]
    fn independent_parts_merge_fresh_rows_and_isolate_fallbacks_and_failures() {
        use super::characters_test_kit as kit;
        let capacities = split_capacities();
        let baseline = kit::mapped_fixture();
        let mut jobs: [CharactersJob; PARTS] = std::array::from_fn(|part| {
            let mut log = crate::raw::RawLog::new(8192, 65536);
            log.initialize_scattered_read_plan(8192, 8192, 65536);
            CharactersJob { part, log, traits: TraitCapture::new(),
                source_bindings: Some(CharacterSourceBindings::new()), ..Default::default() }
        });
        let context = CharactersCopyContext {
            force_full: false,
            base: kit::BASE, game_state: kit::GAME_STATE, active: [true; 6],
            trait_rows: [capacities.rows[4], capacities.rows[5]],
            trait_bytes: [capacities.bytes[4], capacities.bytes[5]],
        };
        let mut fresh = baseline.clone();
        put(&mut fresh, UNITS[0] + cunit_leader::EXPERIENCE, 123456_i64.to_le_bytes());
        put(&mut fresh, EXPERIENCE_ARRAY + 8, 654321_i64.to_le_bytes());
        let mut changed_role = fresh.clone();
        put(&mut changed_role, kit::ADV_ROLE + cadvisor::IDEA_TOKEN, 0_u64.to_le_bytes());
        let mut changed_roster = fresh.clone();
        put(&mut changed_roster, ROSTER, 0_u64.to_le_bytes());
        for bytes in [&baseline, &fresh, &changed_role, &fresh, &changed_roster, &baseline] {
            let memory = kit::memory(bytes);
            for (part, job) in jobs.iter_mut().enumerate() {
                let active = PART_MASKS[part];
                let report = copy_characters_refresh(&memory, CharactersCopyContext { active, ..context }, job);
                assert!(report.raw_ok && !report.plan_failed);
                for (slot, &ticks) in report.copy_ticks.iter().enumerate() {
                    if slot != anchor(active) { assert_eq!(ticks, 0); }
                }
            }
            for order in [[0, 1, 2, 3], [3, 2, 1, 0], [2, 0, 3, 1]] {
                let mut merged = Captured::with_capacities(&capacities);
                merged.set_character_activity([true; 6]);
                merged.raw_active = [true; 6];
                merged.capture(&memory, kit::BASE, kit::GAME_STATE, 0, false);
                for part in order { merged.decode_characters_payload(kit::BASE, &jobs[part], &mut [0; 6]); }
                kit::assert_matches_direct(&merged, &kit::direct(&memory, &capacities));
                assert_eq!(merged.raw_active, [false; 6]);
            }
            for job in &mut jobs { job.log.prepare_read_plan(); job.clear(); }
        }
        let memory = kit::memory(&fresh);
        for (part, job) in jobs.iter_mut().enumerate() {
            let report = copy_characters_refresh(&memory, CharactersCopyContext { active: PART_MASKS[part], ..context }, job);
            assert!(report.raw_ok && job.log.planned_capture_succeeded());
        }
        for (missing, missing_mask) in PART_MASKS.into_iter().enumerate() {
            for failed in [false, true] {
                let mut merged = Captured::with_capacities(&capacities);
                merged.set_character_activity([true; 6]);
                merged.raw_active = if failed { [true; 6] } else { missing_mask.map(|active| !active) };
                // An exhausted staging pool falls back only for its part.
                // A failed native program must instead remain terminal.
                merged.capture(&memory, kit::BASE, kit::GAME_STATE, 0, false);
                let mut expected = kit::direct(&memory, &capacities);
                if failed {
                    merged.fail_character_tables(missing_mask, "test failure");
                    expected.fail_character_tables(missing_mask, "test failure");
                }
                for part in (0..PARTS).rev().filter(|&part| part != missing) {
                    merged.decode_characters_payload(kit::BASE, &jobs[part], &mut [0; 6]);
                }
                kit::assert_matches_direct(&merged, &expected);
            }
        }
    }

    #[test]
    fn portrait_census_is_fresh_complete_and_never_reuses_a_failed_lease() {
        use super::characters_test_kit as kit;
        let make = || {
            let mut log = crate::raw::RawLog::new(8192, 65536);
            log.initialize_scattered_read_plan(8192, 8192, 65536);
            CharactersJob { log, portrait_census: Some(Arc::new(ccharacter_manager::PortraitCensus::new(8))), ..Default::default() }
        };
        let ctx = CharactersCopyContext {
            force_full: false,
            base: kit::BASE, game_state: kit::GAME_STATE,
            active: [true, false, false, true, false, false], trait_rows: [0; 2], trait_bytes: [0; 2],
        };
        let baseline = kit::mapped_fixture();
        let mut job = make();
        copy_characters_refresh(&kit::memory(&baseline), ctx, &mut job);
        let rows = job.portrait_census().unwrap().rows(ctx.base, ctx.game_state).unwrap().to_vec();
        assert_eq!(rows.len(), 3);
        job.log.prepare_read_plan();
        job.clear();
        assert!(job.portrait_census().is_none());
        let mut changed = baseline.clone();
        put(&mut changed, UNITS[0] + cunit_leader::EXPERIENCE, 987_i64.to_le_bytes());
        copy_characters_refresh(&kit::memory(&changed), ctx, &mut job);
        assert!(job.log.planned_capture_succeeded());
        assert_eq!(job.portrait_census().unwrap().rows(ctx.base, ctx.game_state).unwrap(), rows);
        // A roster reorder must rebuild occurrence order, even with the same IDs.
        pointer(&mut changed, ROSTER, CHARACTERS[1]);
        pointer(&mut changed, ROSTER + 8, CHARACTERS[0]);
        copy_characters_refresh(&kit::memory(&changed), ctx, &mut job);
        let reordered = job.portrait_census().unwrap();
        assert_eq!(reordered.rows(ctx.base, ctx.game_state).unwrap()[0], rows[1]);
        // A leaked lease cannot cause a new raw plan to certify the older census.
        copy_characters_refresh(&kit::memory(&baseline), ctx, &mut job);
        assert!(job.portrait_census().is_none());
        job.log.prepare_read_plan();
        drop(reordered);
        copy_characters_refresh(&kit::memory(&baseline), ctx, &mut job);
        assert!(job.log.planned_capture_succeeded());
        assert!(job.portrait_census().is_none());
        // Non-hourly passes can update the census while the old hourly plan
        // remains prepared. Exact equality preserves it; A -> B -> A must
        // rebuild, never loan B after A's prepared controls succeed.
        let mut job = make();
        let daily = CharactersCopyContext { active: [true, true, false, true, false, false], ..ctx };
        copy_characters_refresh(&kit::memory(&baseline), ctx, &mut job);
        job.log.prepare_read_plan();
        assert!(copy_characters_refresh(&kit::memory(&baseline), daily, &mut job).raw_ok);
        job.log.prepare_read_plan();
        copy_characters_refresh(&kit::memory(&baseline), ctx, &mut job);
        assert!(job.log.planned_capture_succeeded(), "unchanged daily census should preserve the hourly plan");
        assert!(copy_characters_refresh(&kit::memory(&changed), daily, &mut job).raw_ok);
        assert_eq!(job.portrait_census().unwrap().rows(ctx.base, ctx.game_state).unwrap()[0], rows[1]);
        job.log.prepare_read_plan();
        copy_characters_refresh(&kit::memory(&baseline), ctx, &mut job);
        assert!(!job.log.planned_capture_succeeded(), "changed daily census must retire the old hourly association");
        assert_eq!(job.portrait_census().unwrap().rows(ctx.base, ctx.game_state).unwrap(), rows);
        job.log.prepare_read_plan();
        copy_characters_refresh(&kit::memory(&baseline), ctx, &mut job);
        assert!(job.log.planned_capture_succeeded());
        assert_eq!(job.portrait_census().unwrap().rows(ctx.base, ctx.game_state).unwrap(), rows);
        for blocked in [ADDRESS + CHARACTERS[0], ADDRESS + CHARACTERS[1] + ccharacter::COUNTRY, ADDRESS + ROSTER] {
            let mut job = make();
            copy_characters_refresh(&WaveMemory::new(&baseline, Some(blocked)), ctx, &mut job);
            assert!(job.portrait_census().is_none(), "fault {blocked:x}");
        }
        let mut job = make();
        job.portrait_census = Some(Arc::new(ccharacter_manager::PortraitCensus::new(2)));
        copy_characters_refresh(&kit::memory(&baseline), ctx, &mut job);
        assert!(job.portrait_census().is_none(), "bounded census must not publish a prefix");
        // The short census demands the whole native roster and grows off-thread
        // in one step; the next pass publishes it.
        assert!(job.grow_for_next());
        job.grow();
        job.clear();
        assert!(!job.portrait_census.as_ref().unwrap().needs_growth());
        copy_characters_refresh(&kit::memory(&baseline), ctx, &mut job);
        assert_eq!(job.portrait_census().unwrap().rows(ctx.base, ctx.game_state).unwrap(), rows);
    }

    #[test]
    fn hourly_plan_reads_fresh_values_and_preserves_every_fault_without_retry() {
        use std::cell::RefCell;
        use super::characters_test_kit as kit;
        struct Trace<'a> {
            bytes: &'a [u8],
            calls: RefCell<Vec<(usize, usize)>>,
            fault: Option<(usize, usize)>,
        }
        impl Memory for Trace<'_> {
            fn copy(&self, address: usize, out: &mut [u8]) -> bool {
                self.calls.borrow_mut().push((address, out.len()));
                if self.fault == Some((address, out.len())) {
                    out.fill(0xEE);
                    return false;
                }
                kit::memory(self.bytes).copy(address, out)
            }
        }
        fn job() -> CharactersJob {
            let mut log = crate::raw::RawLog::new(8192, 65536);
            log.initialize_scattered_read_plan(8192, 8192, 65536);
            CharactersJob { log, ..Default::default() }
        }
        let ctx = CharactersCopyContext {
            force_full: false,
            base: kit::BASE, game_state: kit::GAME_STATE,
            active: [true, false, false, true, false, false],
            trait_rows: [0; 2], trait_bytes: [0; 2],
        };
        let baseline = kit::mapped_fixture();
        let mut first = job();
        assert!(copy_characters_refresh(&kit::memory(&baseline), ctx, &mut first).raw_ok);
        first.log.prepare_read_plan();
        let mut fresh = baseline.clone();
        put(&mut fresh, UNITS[0] + cunit_leader::EXPERIENCE, 123456789_i64.to_le_bytes());
        let report = copy_characters_refresh(&kit::memory(&fresh), ctx, &mut first);
        assert!(report.raw_ok && !report.plan_failed);
        assert!(first.log.planned_capture_succeeded(), "quiet structure must skip discovery");
        let mut canonical = job();
        assert!(copy_characters_refresh(&kit::memory(&fresh), ctx, &mut canonical).raw_ok);
        assert!(first.log.logs_equal(&canonical.log));
        let captured = |payload: &CharactersJob| {
            let mut result = Captured::with_capacities(&split_capacities());
            result.decode_characters_payload(kit::BASE, payload, &mut [0; 6]);
            result
        };
        kit::assert_matches_direct(&captured(&first), &captured(&canonical));
        let source = Trace { bytes: &baseline, calls: RefCell::new(vec![]), fault: None };
        copy_characters_refresh(&source, ctx, &mut canonical);
        let reads = source.calls.into_inner();
        for rename in [false, true] {
        for fault in reads.iter().copied().map(Some).chain([None]) {
            let mut planned = job();
            copy_characters_refresh(&kit::memory(&baseline), ctx, &mut planned);
            planned.log.prepare_read_plan();
            let mut changed = fresh.clone();
            // An in-place name update must be observed even with the same
            // header length, capacity and allocation address.
            if rename { changed[CHARACTERS[0] + ccharacter::NAME] ^= 1; }
            let actual = Trace { bytes: &changed, calls: RefCell::new(vec![]), fault };
            let report = copy_characters_refresh(&actual, ctx, &mut planned);
            assert!(report.raw_ok && !report.plan_failed, "fault {fault:?}");
            let reference = Trace { bytes: &changed, calls: RefCell::new(vec![]), fault };
            let mut expected = job();
            copy_characters_refresh(&reference, ctx, &mut expected);
            assert_eq!(*actual.calls.borrow(), *reference.calls.borrow(), "fault {fault:?}");
            assert!(planned.log.logs_equal(&expected.log), "fault {fault:?}");
            kit::assert_matches_direct(&captured(&planned), &captured(&expected));
        }
        }
        for field in [ROSTER, CHARACTERS[0] + ccharacter::UNIT_LEADER, UNITS[0] + cunit_leader::SKILL] {
            let mut planned = job();
            copy_characters_refresh(&kit::memory(&baseline), ctx, &mut planned);
            planned.log.prepare_read_plan();
            let mut changed = baseline.clone();
            put(&mut changed, field, 0_u64.to_le_bytes());
            let actual = Trace { bytes: &changed, calls: RefCell::new(vec![]), fault: None };
            let report = copy_characters_refresh(&actual, ctx, &mut planned);
            assert!(report.raw_ok && !report.plan_failed);
            let reference = Trace { bytes: &changed, calls: RefCell::new(vec![]), fault: None };
            let mut expected = job();
            copy_characters_refresh(&reference, ctx, &mut expected);
            assert_eq!(*actual.calls.borrow(), *reference.calls.borrow(), "changed field {field:x}");
            assert!(planned.log.logs_equal(&expected.log));
            kit::assert_matches_direct(&captured(&planned), &captured(&expected));
        }
    }

    /// A character's template identity is read once per template pointer:
    /// the next hours read none of it, also through the prepared program; a
    /// reload that gives the same character id a new template pointer reads
    /// that template, and a full hour reads every template again. Every hour,
    /// and every failed template read, decodes to the live walk's rows.
    #[test]
    fn template_identities_are_read_once_per_pointer_and_again_after_a_reload() {
        use super::characters_test_kit as kit;
        use std::cell::RefCell;
        struct Trace<'a> {
            bytes: &'a [u8],
            calls: RefCell<Vec<(usize, usize)>>,
            fault: Option<(usize, usize)>,
        }
        impl Memory for Trace<'_> {
            fn copy(&self, address: usize, out: &mut [u8]) -> bool {
                self.calls.borrow_mut().push((address, out.len()));
                if self.fault == Some((address, out.len())) {
                    out.fill(0xEE);
                    return false;
                }
                kit::memory(self.bytes).copy(address, out)
            }
        }
        const TEMPLATES: [usize; 3] = [0x9000, 0x9400, 0x9c00];
        const FILES: [usize; 2] = [0x9800, 0x9900];
        fn template(bytes: &mut [u8], at: usize, tag: u32, file: Option<(usize, &[u8])>, line: i32, generated: u8) {
            put(bytes, at, ((IMAGE + ccharacter_template::VTABLE_RVA) as u64).to_le_bytes());
            put(bytes, at + ccharacter_template::TAG, tag.to_le_bytes());
            let header = at + ccharacter_template::SOURCE_FILE;
            match file {
                Some((payload, value)) => {
                    pointer(bytes, header, payload);
                    put(bytes, header + 16, (value.len() as u64).to_le_bytes());
                    put(bytes, header + 24, (value.len() as u64).to_le_bytes());
                    bytes[payload..payload + value.len()].copy_from_slice(value);
                    bytes[payload + value.len()] = 0;
                }
                None => literal(bytes, header, b""),
            }
            put(bytes, at + ccharacter_template::SOURCE_LINE, line.to_le_bytes());
            bytes[at + ccharacter_template::GENERATED] = generated;
        }
        let mut world = kit::mapped_fixture();
        world.resize(0xa000, 0);
        template(&mut world, TEMPLATES[0], 501, Some((FILES[0], b"common/characters/GER.txt")), 3, 0);
        // An instance the loader never read: empty file, line -1.
        template(&mut world, TEMPLATES[1], 502, None, -1, 1);
        pointer(&mut world, CHARACTERS[0] + ccharacter::TEMPLATE, TEMPLATES[0]);
        pointer(&mut world, CHARACTERS[1] + ccharacter::TEMPLATE, TEMPLATES[1]);
        let active = PART_MASKS[0];
        let context = |force_full| CharactersCopyContext {
            force_full,
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            active,
            trait_rows: [0; 2],
            trait_bytes: [0; 2],
        };
        let direct = |world: &[u8]| {
            let mut captured = Captured::with_capacities(&split_capacities());
            let results = walk_family(
                &kit::memory(world),
                kit::BASE,
                kit::GAME_STATE,
                captured.family().select(active),
                None,
                None,
            );
            captured.install_family(active, results);
            captured.fail_character_tables(active.map(|value| !value), replay::cadence::SKIPPED_REASON);
            captured
        };
        // One hour of the part-zero job as the pool runs it: copy, decode,
        // remember, then recycle (prepare the program and clear).
        let hour = |job: &mut CharactersJob, world: &[u8], force_full: bool, fault| {
            let source = Trace { bytes: world, calls: RefCell::new(vec![]), fault };
            let report = copy_characters_refresh(&source, context(force_full), job);
            assert!(report.raw_ok && !report.plan_failed, "fault {fault:?}");
            let mut captured = Captured::with_capacities(&split_capacities());
            captured.decode_characters_payload(kit::BASE, job, &mut [0; 6]);
            captured.remember_templates(job);
            let planned = job.log.planned_capture_succeeded();
            job.prepare_read_plans();
            job.clear();
            (captured, source.calls.into_inner(), planned)
        };
        let reads = |calls: &[(usize, usize)], at: usize| {
            calls.iter().any(|&(address, _)| (ADDRESS + at..ADDRESS + at + 0x248).contains(&address))
        };
        let mut job = CharactersJob::new_part(0);

        let (first, calls, _) = hour(&mut job, &world, false, None);
        kit::assert_matches_direct(&first, &direct(&world));
        assert!(reads(&calls, TEMPLATES[0]) && reads(&calls, TEMPLATES[1]));
        assert!(calls.contains(&(ADDRESS + FILES[0], 25)));
        let TableView::Complete(rows) = first.characters_table() else {
            panic!("characters table incomplete");
        };
        let rows = rows.rows::<tables::Characters>().unwrap();
        let by_template = |tag: u32| rows.iter().find(|row| row.template_tag == tag).unwrap();
        let ger = by_template(501);
        assert_eq!(
            (ger.template_present, ger.template_file.as_slice(), ger.template_line, ger.template_generated),
            (1, b"common/characters/GER.txt".as_slice(), 3, 0)
        );
        let never_read = by_template(502);
        assert_eq!(
            (never_read.template_present, never_read.template_file.len(), never_read.template_line, never_read.template_generated),
            (1, 0, -1, 1)
        );
        assert!(rows.iter().any(|row| row.template_present == 0 && row.template_tag == 0));
        // The live census sizes a complete first hour from minimal buffers.
        let demand = ccharacter::demand(&kit::memory(&world), kit::BASE, kit::GAME_STATE).unwrap();
        let mut sized = Capacities { rows: [1; N], bytes: [1; N] };
        sized.raise_for_characters(demand);
        let mut captured = Captured::with_capacities(&sized);
        let only = [true, false, false, false, false, false];
        let results = walk_family(
            &kit::memory(&world),
            kit::BASE,
            kit::GAME_STATE,
            captured.family().select(only),
            None,
            None,
        );
        assert_eq!(results[0].outcome, Outcome::Rows { len: demand.rows });
        assert_eq!((demand.rows, results[0].bytes_len), (rows.len(), demand.bytes));

        // The next hours read no template: first rediscovery with the
        // identities held, then the prepared program keyed by them.
        for planned_hour in [false, true] {
            let (captured, calls, planned) = hour(&mut job, &world, false, None);
            kit::assert_matches_direct(&captured, &direct(&world));
            assert!(!reads(&calls, TEMPLATES[0]) && !reads(&calls, TEMPLATES[1]));
            assert!(!calls.iter().any(|&(address, _)| address == ADDRESS + FILES[0] + 25));
            assert_eq!(planned, planned_hour);
        }

        // A reload that reuses character 0's id with a new template pointer
        // reads that template, under the prepared program too; the other
        // character's unchanged pointer still reads nothing.
        let mut reloaded = world.clone();
        template(&mut reloaded, TEMPLATES[2], 503, Some((FILES[1], b"common/characters/ENG.txt")), 9, 0);
        pointer(&mut reloaded, CHARACTERS[0] + ccharacter::TEMPLATE, TEMPLATES[2]);
        let (captured, calls, _) = hour(&mut job, &reloaded, false, None);
        kit::assert_matches_direct(&captured, &direct(&reloaded));
        assert!(reads(&calls, TEMPLATES[2]) && !reads(&calls, TEMPLATES[1]));
        let (captured, calls, _) = hour(&mut job, &reloaded, false, None);
        kit::assert_matches_direct(&captured, &direct(&reloaded));
        assert!(!reads(&calls, TEMPLATES[2]));

        // A full hour reads every template again, even at a held pointer.
        let (captured, calls, _) = hour(&mut job, &reloaded, true, None);
        kit::assert_matches_direct(&captured, &direct(&reloaded));
        assert!(reads(&calls, TEMPLATES[2]) && reads(&calls, TEMPLATES[1]));

        // A failed template read fails the table as the live walk does.
        let mut fresh = CharactersJob::new_part(0);
        let (_, calls, _) = hour(&mut fresh, &world, false, None);
        let mut faults = 0;
        for fault in calls.into_iter().filter(|&(address, _)| {
            reads(&[(address, 0)], TEMPLATES[0])
                || reads(&[(address, 0)], TEMPLATES[1])
                || (ADDRESS + FILES[0]..ADDRESS + FILES[0] + 26).contains(&address)
        }) {
            let mut job = CharactersJob::new_part(0);
            let (captured, _, _) = hour(&mut job, &world, false, Some(fault));
            let reference = Trace { bytes: &world, calls: RefCell::new(vec![]), fault: Some(fault) };
            let mut expected = Captured::with_capacities(&split_capacities());
            let results = walk_family(&reference, kit::BASE, kit::GAME_STATE, expected.family().select(active), None, None);
            expected.install_family(active, results);
            expected.fail_character_tables(active.map(|value| !value), replay::cadence::SKIPPED_REASON);
            kit::assert_matches_direct(&captured, &expected);
            assert!(
                matches!(captured.characters.rows.outcome, Outcome::Unavailable { .. }),
                "fault {fault:?}"
            );
            // Nothing from a failed read is remembered.
            assert!(job.templates.entries.is_empty());
            faults += 1;
        }
        // Both headers and generated bytes, the heap file's terminator and bytes.
        assert_eq!(faults, 6);
    }

    #[test]
    fn complete_family_plan_refreshes_roles_traits_and_experience_without_retry() {
        use super::characters_test_kit as kit;
        use std::cell::RefCell;
        struct Trace<'a> {
            bytes: &'a [u8],
            calls: RefCell<Vec<(usize, usize)>>,
            fault: Option<(usize, usize)>,
        }
        impl Memory for Trace<'_> {
            fn copy(&self, address: usize, out: &mut [u8]) -> bool {
                self.calls.borrow_mut().push((address, out.len()));
                if self.fault == Some((address, out.len())) { return false; }
                kit::memory(self.bytes).copy(address, out)
            }
        }
        fn job() -> CharactersJob {
            let mut log = crate::raw::RawLog::new(8192, 65536);
            log.initialize_scattered_read_plan(8192, 8192, 65536);
            CharactersJob { log, traits: TraitCapture::new(),
                source_bindings: Some(CharacterSourceBindings::new()), ..Default::default() }
        }
        fn direct(memory: &impl Memory, capacities: &Capacities, active: [bool; 6]) -> Captured {
            let mut captured = Captured::with_capacities(capacities);
            let results = walk_family(memory, kit::BASE, kit::GAME_STATE, captured.family().select(active), None, None);
            captured.install_family([true; 6], results);
            captured
        }
        for active in [[true; 6], PART_MASKS[1], PART_MASKS[2], PART_MASKS[3]] {
        let capacities = split_capacities();
        let context = CharactersCopyContext {
            force_full: false,
            base: kit::BASE, game_state: kit::GAME_STATE, active,
            trait_rows: [capacities.rows[4], capacities.rows[5]],
            trait_bytes: [capacities.bytes[4], capacities.bytes[5]],
        };
        let baseline = kit::mapped_fixture();
        let decode = |payload: &CharactersJob| {
            let mut captured = Captured::with_capacities(&capacities);
            captured.decode_characters_payload(kit::BASE, payload, &mut [0; 6]);
            captured
        };
        let mut prepared = job();
        assert!(copy_characters_refresh(&kit::memory(&baseline), context, &mut prepared).raw_ok);
        assert!(prepared.source_bindings.as_ref().unwrap().ready);
        prepared.log.prepare_read_plan();
        prepared.clear();
        let mut fresh = baseline.clone();
        put(&mut fresh, UNITS[0] + cunit_leader::EXPERIENCE, 123456_i64.to_le_bytes());
        put(&mut fresh, EXPERIENCE_ARRAY + 8, 654321_i64.to_le_bytes());
        assert!(copy_characters_refresh(&kit::memory(&fresh), context, &mut prepared).raw_ok);
        assert!(prepared.log.planned_capture_succeeded());
        kit::assert_matches_direct(&decode(&prepared), &direct(&kit::memory(&fresh), &capacities, active));

        let source = Trace { bytes: &baseline, calls: RefCell::new(vec![]), fault: None };
        copy_characters_refresh(&source, context, &mut job());
        let reads: std::collections::BTreeSet<_> = source.calls.into_inner().into_iter().collect();
        for fault in reads.into_iter().map(Some).chain([None]) {
            let mut prepared = job();
            copy_characters_refresh(&kit::memory(&baseline), context, &mut prepared);
            prepared.log.prepare_read_plan();
            prepared.clear();
            let source = Trace { bytes: &fresh, calls: RefCell::new(vec![]), fault };
            let report = copy_characters_refresh(&source, context, &mut prepared);
            assert!(report.raw_ok && !report.plan_failed, "fault {fault:?}");
            let reference = Trace { bytes: &fresh, calls: RefCell::new(vec![]), fault };
            let mut ordinary = job();
            copy_characters_refresh(&reference, context, &mut ordinary);
            kit::assert_matches_direct(&decode(&prepared), &decode(&ordinary));
            if let Some(fault) = fault {
                assert!(source.calls.borrow().iter().filter(|&&read| read == fault).count()
                    <= reference.calls.borrow().iter().filter(|&&read| read == fault).count(),
                    "prepared path retried a failed observation {fault:?}");
            }
        }
        for field in [ROSTER, CHARACTERS[0] + ccharacter::UNIT_LEADER,
            UNITS[0] + cunit_leader::SKILL, TRAIT_ARRAY, EXPERIENCE_ARRAY,
            kit::ADV_ROLE + cadvisor::IDEA_TOKEN]
        {
            let mut prepared = job();
            copy_characters_refresh(&kit::memory(&baseline), context, &mut prepared);
            prepared.log.prepare_read_plan();
            let mut changed = baseline.clone();
            put(&mut changed, field, 0_u64.to_le_bytes());
            for source in [&changed, &baseline] {
                assert!(copy_characters_refresh(&kit::memory(source), context, &mut prepared).raw_ok);
                kit::assert_matches_direct(&decode(&prepared), &direct(&kit::memory(source), &capacities, active));
                prepared.log.prepare_read_plan();
            }
        }
        // A recycled job must rebuild bindings after capacity changes. A
        // truncated census cannot seed an apparently complete later plan.
        for table in [4, 5] {
            for limit in [0, 1, 2] {
                let mut prepared = job();
                copy_characters_refresh(&kit::memory(&baseline), context, &mut prepared);
                prepared.log.prepare_read_plan();
                for bytes_limit in [false, true] {
                    let mut limited = split_capacities();
                    if bytes_limit { limited.bytes[table] = limit; }
                    else { limited.rows[table] = limit; }
                    let limited_context = CharactersCopyContext {
                        force_full: false,
                        trait_rows: [limited.rows[4], limited.rows[5]],
                        trait_bytes: [limited.bytes[4], limited.bytes[5]], ..context
                    };
                    for (ctx, caps) in [(limited_context, &limited), (context, &capacities)] {
                        assert!(copy_characters_refresh(&kit::memory(&baseline), ctx, &mut prepared).raw_ok);
                        let mut actual = Captured::with_capacities(caps);
                        actual.decode_characters_payload(kit::BASE, &prepared, &mut [0; 6]);
                        kit::assert_matches_direct(&actual, &direct(&kit::memory(&baseline), caps, active));
                        prepared.log.prepare_read_plan();
                    }
                }
            }
        }
        if active[4] || active[5] {
        let mut bounded = job();
        bounded.source_bindings.as_mut().unwrap().sources = Vec::with_capacity(1);
        assert!(copy_characters_refresh(&kit::memory(&baseline), context, &mut bounded).raw_ok);
        assert!(!bounded.source_bindings.as_ref().unwrap().ready);
        bounded.log.prepare_read_plan();
        assert!(copy_characters_refresh(&kit::memory(&fresh), context, &mut bounded).raw_ok);
        assert!(!bounded.log.planned_capture_succeeded());
        kit::assert_matches_direct(&decode(&bounded), &direct(&kit::memory(&fresh), &capacities, active));
        }
        }
    }

    pub(crate) const ADDRESS: usize = 0x10000;
    pub(crate) const IMAGE: usize = 0x140000000;
    pub(crate) const MANAGER: usize = 0x1000;
    pub(crate) const ROSTER: usize = 0x2000;
    pub(crate) const CHARACTERS: [usize; 2] = [0x2100, 0x2200];
    pub(crate) const UNITS: [usize; 2] = [0x3000, 0x5000];
    pub(crate) const TRAIT_ARRAY: usize = 0x7000;
    pub(crate) const EXPERIENCE_ARRAY: usize = 0x7040;
    pub(crate) const DEFINITION: usize = 0x7800;

    fn walk_units(
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        units: Batch<'_, LeaderRow>,
        traits: Batch<'_, cunit_leader::TraitRow>,
        experience: Batch<'_, cunit_leader::TraitExperienceRow>,
    ) -> [ReadResult; 3] {
        let result = walk_family(
            memory,
            base,
            game_state,
            Family {
                characters: Batch::new(&mut [], &mut []),
                leaders: Batch::new(&mut [], &mut []),
                advisors: Batch::new(&mut [], &mut []),
                units,
                traits,
                experience,
            },
            None,
            None,
        );
        [result[3], result[4], result[5]]
    }

    pub(crate) fn put<const N: usize>(bytes: &mut [u8], offset: usize, value: [u8; N]) {
        bytes[offset..offset + N].copy_from_slice(&value);
    }

    pub(crate) fn pointer(bytes: &mut [u8], offset: usize, target: usize) {
        put(bytes, offset, ((ADDRESS + target) as u64).to_le_bytes());
    }

    pub(crate) fn array(bytes: &mut [u8], offset: usize, data: usize, count: u32) {
        pointer(bytes, offset, data);
        put(bytes, offset + 8, count.to_le_bytes());
        put(bytes, offset + 12, count.to_le_bytes());
    }

    pub(crate) fn literal(bytes: &mut [u8], offset: usize, value: &[u8]) {
        assert!(value.len() < 16);
        bytes[offset..offset + 32].fill(0);
        bytes[offset..offset + value.len()].copy_from_slice(value);
        put(bytes, offset + 16, (value.len() as u64).to_le_bytes());
        put(bytes, offset + 24, 15_u64.to_le_bytes());
    }

    pub(crate) fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x9000];
        pointer(&mut bytes, cgame_state::CHARACTER_MANAGER, MANAGER);
        put(
            &mut bytes,
            MANAGER,
            ((IMAGE + ccharacter_manager::VTABLE_RVA) as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
            ROSTER,
            2,
        );
        put(
            &mut bytes,
            DEFINITION,
            ((IMAGE + cunit_leader_trait::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            DEFINITION + cunit_leader_trait::TOKEN_ID,
            (-22_i32).to_le_bytes(),
        );
        put(
            &mut bytes,
            DEFINITION + cunit_leader_trait::ORDINAL,
            i32::MIN.to_le_bytes(),
        );
        literal(
            &mut bytes,
            DEFINITION + cunit_leader_trait::TOKEN,
            b"trait\xff",
        );
        for index in 0..2 {
            let character = CHARACTERS[index];
            let unit = UNITS[index];
            let id = u32::MAX - index as u32;
            pointer(&mut bytes, ROSTER + index * 8, character);
            put(
                &mut bytes,
                character,
                ((IMAGE + ccharacter::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                character + ccharacter::TYPE,
                ccharacter::REFERENCE_TYPE.to_le_bytes(),
            );
            put(&mut bytes, character + ccharacter::ID, id.to_le_bytes());
            put(
                &mut bytes,
                character + ccharacter::COUNTRY,
                (index as u32 + 1).to_le_bytes(),
            );
            pointer(&mut bytes, character + ccharacter::UNIT_LEADER, unit);
            literal(&mut bytes, character + ccharacter::NAME, b"literal");
            literal(&mut bytes, character + ccharacter::TOKEN, b"saved");
            for (role, offset) in [ccharacter::LEADER_MAP_HEAD, ccharacter::ADVISOR_MAP_HEAD]
                .into_iter()
                .enumerate()
            {
                let head = 0x2400 + (index * 2 + role) * 0x30;
                pointer(&mut bytes, character + offset, head);
                for link in [0, 8, 16] {
                    pointer(&mut bytes, head + link, head);
                }
                bytes[head + 0x19] = 1;
            }
            put(
                &mut bytes,
                unit,
                ((IMAGE + cunit_leader::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                unit + cunit_leader::TYPE,
                cunit_leader::REFERENCE_TYPE.to_le_bytes(),
            );
            put(&mut bytes, unit + cunit_leader::ID, id.to_le_bytes());
            put(
                &mut bytes,
                unit + cunit_leader::CHARACTER_TYPE,
                ccharacter::REFERENCE_TYPE.to_le_bytes(),
            );
            put(
                &mut bytes,
                unit + cunit_leader::CHARACTER_ID,
                id.to_le_bytes(),
            );
            put(
                &mut bytes,
                unit + cunit_leader::EXPERIENCE,
                (i64::MIN + index as i64).to_le_bytes(),
            );
            literal(
                &mut bytes,
                unit + cunit_leader::TOKEN,
                if index == 0 { b"saved1" } else { b"saved2" },
            );
            literal(
                &mut bytes,
                unit + cunit_leader::NAME,
                if index == 0 { b"other1" } else { b"other2" },
            );
            array(
                &mut bytes,
                unit + cunit_leader::TRAIT_DATA,
                TRAIT_ARRAY + index * 8,
                1,
            );
            pointer(&mut bytes, TRAIT_ARRAY + index * 8, DEFINITION);
            array(
                &mut bytes,
                unit + cunit_leader::TRAIT_EXPERIENCE_DATA,
                EXPERIENCE_ARRAY + index * 16,
                1,
            );
            pointer(&mut bytes, EXPERIENCE_ARRAY + index * 16, DEFINITION);
            put(
                &mut bytes,
                EXPERIENCE_ARRAY + index * 16 + 8,
                (i64::MAX - index as i64).to_le_bytes(),
            );
        }
        bytes
    }

    struct CountedMemory<'a> {
        memory: SliceMemory<'a>,
        character_header_reads: Cell<usize>,
    }

    struct FaultMemory<'a> {
        memory: SliceMemory<'a>,
        fault: std::ops::Range<usize>,
        failures: Cell<usize>,
    }

    impl Memory for FaultMemory<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let Some(end) = address.checked_add(output.len()) else {
                return false;
            };
            if address < self.fault.end && end > self.fault.start {
                self.failures.set(self.failures.get() + 1);
                // Native guarded copies may modify a prefix before failing.
                // Tests must reject the whole failed span even if its unused
                // suffix happens to contain the native default value zero.
                let prefix = self.fault.start.saturating_sub(address).min(output.len());
                if prefix != 0 {
                    self.memory.copy(address, &mut output[..prefix]);
                }
                return false;
            }
            self.memory.copy(address, output)
        }
    }

    impl Memory for CountedMemory<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if output.len() == 16
                && CHARACTERS
                    .iter()
                    .any(|&character| address == ADDRESS + character)
            {
                self.character_header_reads
                    .set(self.character_header_reads.get() + 1);
            }
            self.memory.copy(address, output)
        }
    }

    #[test]
    fn shared_roster_keeps_traits_when_scalar_capacity_is_exhausted() {
        let bytes = fixture();
        let memory = CountedMemory {
            memory: SliceMemory {
                base: ADDRESS,
                bytes: &bytes,
            },
            character_header_reads: Cell::new(0),
        };
        let mut units = [LeaderRow::default(); 1];
        let mut names = [0; 24];
        let mut traits = [cunit_leader::TraitRow::default(); 2];
        let mut tokens = [0; 12];
        let mut experience = [cunit_leader::TraitExperienceRow::default(); 2];
        let mut experience_tokens = [0; 12];
        let results = walk_units(
            &memory,
            IMAGE,
            ADDRESS,
            Batch::new(&mut units, &mut names),
            Batch::new(&mut traits, &mut tokens),
            Batch::new(&mut experience, &mut experience_tokens),
        );
        assert_eq!(memory.character_header_reads.get(), 2);
        assert_eq!(
            results[0].outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(results[1].outcome, Outcome::Rows { len: 2 });
        assert_eq!(results[2].outcome, Outcome::Rows { len: 2 });
        assert_eq!(traits[1].token_start, 6);
        assert_eq!(&tokens, b"trait\xfftrait\xff");
        assert_eq!(experience[1].experience, i64::MAX - 1);
        assert_eq!(leader_row(&units[0], &names).experience, i64::MIN);
        assert_eq!(leader_row(&units[0], &names).inherited_text_a, b"saved1");
        assert_eq!(leader_row(&units[0], &names).inherited_text_b, b"other1");
        assert_eq!(leader_reference(traits[0].leader_id), 0xffff_ffff_0000_1269);
    }

    #[test]
    fn one_trait_vector_fault_does_not_invalidate_other_family_tables() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            UNITS[1] + cunit_leader::TRAIT_COUNT,
            (-1_i32).to_le_bytes(),
        );
        let memory = SliceMemory {
            base: ADDRESS,
            bytes: &bytes,
        };
        let mut units = [LeaderRow::default(); 2];
        let mut names = [0; 24];
        let mut traits = [cunit_leader::TraitRow::default(); 2];
        let mut tokens = [0; 12];
        let mut experience = [cunit_leader::TraitExperienceRow::default(); 2];
        let mut experience_tokens = [0; 12];
        let results = walk_units(
            &memory,
            IMAGE,
            ADDRESS,
            Batch::new(&mut units, &mut names),
            Batch::new(&mut traits, &mut tokens),
            Batch::new(&mut experience, &mut experience_tokens),
        );
        assert_eq!(results[0].outcome, Outcome::Rows { len: 2 });
        assert!(matches!(results[1].outcome, Outcome::Unavailable { .. }));
        assert_eq!(results[1].bytes_len, 0);
        assert_eq!(results[2].outcome, Outcome::Rows { len: 2 });
        assert_eq!(leader_row(&units[1], &names).inherited_text_a, b"saved2");
    }

    #[test]
    fn character_parts_read_each_identity_once_per_owner_and_refresh_current_fields() {
        let mut bytes = fixture();
        let capacity = Capacities {
            rows: [2; N],
            bytes: [32; N],
        };
        let mut captured = Captured::with_capacities(&capacity);
        for token in [0_i32, -7] {
            put(
                &mut bytes,
                CHARACTERS[0] + ccharacter::NAME_TOKEN_ID,
                token.to_le_bytes(),
            );
            let memory = CountedMemory {
                memory: SliceMemory {
                    base: ADDRESS,
                    bytes: &bytes,
                },
                character_header_reads: Cell::new(0),
            };
            captured.capture(&memory, IMAGE, ADDRESS, 0, true);
            assert_eq!(memory.character_header_reads.get(), 2 * PARTS);
            assert_eq!(
                &captured.outcomes()[..6],
                &[
                    Outcome::Rows { len: 2 },
                    Outcome::Rows { len: 0 },
                    Outcome::Rows { len: 0 },
                    Outcome::Rows { len: 2 },
                    Outcome::Rows { len: 2 },
                    Outcome::Rows { len: 2 },
                ]
            );
            assert_eq!(captured.characters.rows.rows[0].name_token_id, token);
        }
    }

    #[test]
    fn invalid_name_header_does_not_hide_roles_or_unit_state() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            CHARACTERS[0] + ccharacter::NAME + 16,
            16_u64.to_le_bytes(),
        );
        let memory = SliceMemory {
            base: ADDRESS,
            bytes: &bytes,
        };
        let capacity = Capacities {
            rows: [2; N],
            bytes: [32; N],
        };
        let mut captured = Captured::with_capacities(&capacity);
        let results = walk_family(&memory, IMAGE, ADDRESS, captured.family(), None, None);
        assert!(matches!(results[0].outcome, Outcome::Unavailable { .. }));
        assert_eq!(results[1].outcome, Outcome::Rows { len: 0 });
        assert_eq!(results[2].outcome, Outcome::Rows { len: 0 });
        for result in &results[3..] {
            assert_eq!(result.outcome, Outcome::Rows { len: 2 });
        }
    }

    #[test]
    fn partial_character_span_faults_are_local_and_failed_spans_are_never_retried() {
        let bytes = fixture();
        let capacity = Capacities {
            rows: [2; N],
            bytes: [32; N],
        };
        let expected = [2, 0, 0, 2, 2, 2];
        for (offset, affected) in [
            (
                ccharacter::NAME + 32,
                [true, false, false, false, false, false],
            ),
            (
                ccharacter::LEADER_MAP_HEAD + 8,
                [false, true, false, false, false, false],
            ),
            (
                ccharacter::ADVISOR_MAP_HEAD + 8,
                [false, false, true, false, false, false],
            ),
            (
                ccharacter::UNIT_LEADER + 4,
                [false, false, false, true, true, true],
            ),
            (
                ccharacter::COUNTRY + 2,
                [true, true, true, false, false, false],
            ),
        ] {
            let fault = ADDRESS + CHARACTERS[0] + offset;
            let memory = FaultMemory {
                memory: SliceMemory {
                    base: ADDRESS,
                    bytes: &bytes,
                },
                fault: fault..fault + 1,
                failures: Cell::new(0),
            };
            let mut captured = Captured::with_capacities(&capacity);
            let results = walk_family(&memory, IMAGE, ADDRESS, captured.family(), None, None);
            for (index, result) in results.into_iter().enumerate() {
                if affected[index] {
                    assert!(
                        matches!(result.outcome, Outcome::Unavailable { .. }),
                        "offset={offset:x} table={index}"
                    );
                    assert_eq!(result.bytes_len, 0);
                } else {
                    assert_eq!(
                        result.outcome,
                        Outcome::Rows {
                            len: expected[index]
                        },
                        "offset={offset:x} table={index}"
                    );
                }
            }
            assert_eq!(
                memory.failures.get(),
                1,
                "failed span retried at {offset:x}"
            );
        }
    }

    #[test]
    fn unrelated_portrait_bytes_are_not_read() {
        let bytes = fixture();
        let capacity = Capacities {
            rows: [2; N],
            bytes: [32; N],
        };
        let portrait = ADDRESS + CHARACTERS[0] + ccharacter::PORTRAITS;
        let memory = FaultMemory {
            memory: SliceMemory {
                base: ADDRESS,
                bytes: &bytes,
            },
            fault: portrait..portrait + 1,
            failures: Cell::new(0),
        };
        let mut captured = Captured::with_capacities(&capacity);
        let results = walk_family(&memory, IMAGE, ADDRESS, captured.family(), None, None);
        for result in results {
            assert!(matches!(result.outcome, Outcome::Rows { .. }));
        }
        assert_eq!(memory.failures.get(), 0);
    }

    #[test]
    fn active_abilities_count_every_row_and_token_past_capacity_for_one_growth_step() {
        use engine::{cability, cgame_state};
        const COUNTRIES: u32 = 5000;
        const GAME_STATE: usize = 0x1000;
        const CTAB: usize = 0x10000;
        const LTAB: usize = 0x20000;
        const COUNTRY0: usize = 0x30000;
        // Country objects overlap: only +0 (vtable), +8 (id) and the ability
        // header at +0x15b8 (0x28 within a 0x50 stride) are populated.
        const STRIDE: usize = 0x50;
        const RECORDS: usize = 0x100000;
        const DEFINITION: usize = 0x180000;
        let put = |bytes: &mut Vec<u8>, at: usize, value: &[u8]| {
            bytes[at..at + value.len()].copy_from_slice(value)
        };
        let header = |bytes: &mut Vec<u8>, at: usize, data: usize, count: u32| {
            bytes[at..at + 8].copy_from_slice(&(data as u64).to_le_bytes());
            bytes[at + 8..at + 12].copy_from_slice(&count.to_le_bytes());
            bytes[at + 12..at + 16].copy_from_slice(&count.to_le_bytes());
        };
        let mut bytes = vec![0; 0x190000];
        header(&mut bytes, GAME_STATE + cgame_state::COUNTRIES, CTAB, COUNTRIES + 1);
        header(&mut bytes, GAME_STATE + cgame_state::COUNTRY_LINKS, LTAB, COUNTRIES + 1);
        for slot in 1..=COUNTRIES {
            let country = COUNTRY0 + (slot as usize - 1) * STRIDE;
            let record = RECORDS + (slot as usize - 1) * active_ability_record::STRIDE;
            put(&mut bytes, CTAB + slot as usize * 8, &(country as u64).to_le_bytes());
            put(&mut bytes, LTAB + slot as usize * 4, &slot.to_le_bytes());
            put(&mut bytes, country, &(ccountry::VTABLE_RVA as u64).to_le_bytes());
            put(&mut bytes, country + ccountry::ID, &slot.to_le_bytes());
            header(&mut bytes, country + ccountry::ACTIVE_ABILITIES, record, 1);
            put(
                &mut bytes,
                record + active_ability_record::DEFINITION,
                &(DEFINITION as u64).to_le_bytes(),
            );
        }
        put(&mut bytes, DEFINITION, &(cability::VTABLE_RVA as u64).to_le_bytes());
        put(&mut bytes, DEFINITION + cability::TOKEN, b"ability");
        put(&mut bytes, DEFINITION + cability::TOKEN_LENGTH, &7_u64.to_le_bytes());
        put(&mut bytes, DEFINITION + cability::TOKEN_CAPACITY, &15_u64.to_le_bytes());
        assert!(COUNTRY0 + COUNTRIES as usize * STRIDE + ccountry::ACTIVE_ABILITIES < RECORDS);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let defaults = super::super::active_abilities::Capacities::default();
        let (mut row_capacity, mut byte_capacity) = (defaults.rows, defaults.bytes);
        let total = COUNTRIES as usize;
        assert!(total > row_capacity && total * 7 > byte_capacity);
        let mut rows = vec![active_ability_record::Row::default(); row_capacity];
        let mut arena = vec![0; byte_capacity];
        let first = walk_abilities(&memory, 0, GAME_STATE, &mut rows, &mut arena);
        assert_eq!(
            first.outcome,
            Outcome::Truncated {
                len: row_capacity,
                unread_rows: (total - row_capacity) as u32
            }
        );
        assert_eq!(first.required_bytes, total * 7);
        crate::table_capture::grow_capacity(&mut row_capacity, total);
        crate::table_capture::grow_capacity(&mut byte_capacity, first.required_bytes);
        let mut rows = vec![active_ability_record::Row::default(); row_capacity];
        let mut arena = vec![0; byte_capacity];
        // The walk over raw memory, without the Ordinary job's cached roster
        // view: a fixed number of guarded calls per live country and per
        // ability record, whatever the viewer's country. The hourly pack's cost
        // on the roster view is pinned in `active_abilities`.
        engine::read_counts::ReadCounts::take_current();
        let second = walk_abilities(
            &engine::read_counts::CountedMemory::new(memory),
            0,
            GAME_STATE,
            &mut rows,
            &mut arena,
        );
        let with_records = engine::read_counts::ReadCounts::take_current();
        assert_eq!(second.outcome, Outcome::Rows { len: total });
        assert_eq!((second.bytes_len, second.required_bytes), (total * 7, total * 7));
        for (index, row) in rows[..total].iter().enumerate() {
            assert_eq!((row.country, row.ordinal), (index as u32 + 1, 0));
            assert_eq!((row.token_start, row.token_len), (index * 7, 7));
            assert_eq!(&arena[row.token_start..row.token_start + 7], b"ability");
        }
        for slot in 1..=COUNTRIES {
            let country = COUNTRY0 + (slot as usize - 1) * STRIDE;
            header(&mut bytes, country + ccountry::ACTIVE_ABILITIES, 0, 0);
        }
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let empty = walk_abilities(
            &engine::read_counts::CountedMemory::new(memory),
            0,
            GAME_STATE,
            &mut rows,
            &mut arena,
        );
        let countries_only = engine::read_counts::ReadCounts::take_current();
        assert_eq!(empty.outcome, Outcome::Rows { len: 0 });
        let calls = |counts: engine::read_counts::ReadCounts| {
            counts.copy_calls + counts.scattered_calls
        };
        // Over raw memory: two registry reads, eight per country
        // (authentication and the array header), three per record without
        // caster or region; a caster and a region add their own identity reads.
        // On the roster view the census answers the authentication, leaving
        // one call per country.
        let countries = u64::from(COUNTRIES);
        assert_eq!(calls(countries_only), 2 + 8 * countries);
        assert_eq!(calls(with_records) - calls(countries_only), 3 * countries);
    }

    #[test]
    fn byte_capacity_stops_the_prefix_and_retains_exact_omitted_count() {
        let mut rows = [cunit_leader::TraitRow::default(); 3];
        let mut bytes = [0; 2];
        let mut output = Batch::new(&mut rows, &mut bytes);
        output.append(
            |rows, bytes| {
                rows[0].token_len = 2;
                bytes.copy_from_slice(b"ok");
                ReadResult {
                    outcome: Outcome::Truncated {
                        len: 1,
                        unread_rows: 4,
                    },
                    bytes_len: 2,
                    required_bytes: 9,
                }
            },
            |row, offset| relocate(&mut row.token_start, offset),
        );
        output.append(
            |rows, bytes| {
                assert!(rows.is_empty());
                assert!(bytes.is_empty());
                ReadResult::rows(Outcome::Truncated {
                    len: 0,
                    unread_rows: 7,
                })
            },
            |_, _| unreachable!(),
        );
        let result = output.result();
        assert_eq!(
            result.outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 11
            }
        );
        assert_eq!((result.bytes_len, result.required_bytes), (2, 9));
    }

    fn small_capacities() -> Capacities {
        Capacities {
            rows: [2; N],
            bytes: [16; N],
        }
    }

    #[test]
    fn ability_conversion_retains_presence_native_widths_and_literal_bytes() {
        let native = active_ability_record::Row {
            country: u32::MAX,
            ordinal: u32::MAX,
            scope_country: 0x8000_0000,
            token_start: 1,
            token_len: 2,
            caster: Some(leader_reference(u32::MAX)),
            strategic_region: Some(u32::MAX),
            division_count: i32::MIN,
            activation_hour: i32::MIN,
            phase: 0xff,
            allocate_cost: 0xfe,
            allocated_command_power: i64::MIN,
            duration_hours: i32::MAX,
            cooldown_hours: i32::MIN,
            target_kind: i32::MIN,
        };
        let replay = ability_row(&native, b"x\xff\x80");
        assert_eq!(replay.token, b"\xff\x80");
        assert_eq!(replay.caster, Some(0xffff_ffff_0000_1269));
        assert_eq!(replay.strategic_region, Some(u32::MAX));
        assert_eq!(replay.division_count, i32::MIN);
        assert_eq!(replay.allocated_command_power, i64::MIN);
        assert_eq!(replay.phase, 0xff);
        assert_eq!(replay.allocate_cost, 0xfe);
        assert_eq!(replay.key(), u64::MAX as u128);
        let absent = ability_row(
            &active_ability_record::Row {
                caster: None,
                strategic_region: None,
                division_count: 1,
                allocated_command_power: -1,
                ..native
            },
            b"x\xff\x80",
        );
        assert_eq!(absent.caster, None);
        assert_eq!(absent.strategic_region, None);
        assert_eq!(
            (absent.division_count, absent.allocated_command_power),
            (1, -1)
        );
    }

    #[test]
    fn conversion_rejects_bad_ranges_and_duplicate_keys() {
        let mut captured = Captured::with_capacities(&small_capacities());
        // These walkers retain native traversal order. Their complete results
        // still cannot publish duplicate row keys through off-thread conversion.
        captured.characters.rows.rows[0].id = u32::MAX;
        captured.characters.rows.rows[1] = captured.characters.rows.rows[0];
        captured.characters.rows.outcome = Outcome::Rows { len: 2 };
        captured.leaders.rows.rows[0].character = u32::MAX;
        captured.leaders.rows.rows[0].ideology = i32::MIN;
        captured.leaders.rows.rows[1] = captured.leaders.rows.rows[0];
        captured.leaders.rows.outcome = Outcome::Rows { len: 2 };
        captured.advisors.rows.rows[0].character = u32::MAX;
        captured.advisors.rows.rows[1] = captured.advisors.rows.rows[0];
        captured.advisors.rows.outcome = Outcome::Rows { len: 2 };
        let views = captured.tables();
        for view in &views[..3] {
            assert!(matches!(view, TableView::Unavailable { .. }));
        }
    }

    // Deterministic read-cost proxy over one synthetic roster. Its proportions
    // approach the measured capture's 0.154 country-leader roles, 0.728 advisor
    // roles and 0.098 unit leaders per character: 32 characters carry 5, 22 and
    // 3 of them, that is 0.156, 0.688 and 0.094, with two traits and fifteen
    // trait experience entries per leader and heap literals throughout. The
    // counts asserted below are guarded entries into the copy shim, the spans
    // inside them, and the bytes those spans move.
    const COST_CHARACTERS: usize = 32;
    const COST_LEADER_STEP: usize = 7;
    const COST_ADVISORS_PER: usize = 2;
    const COST_UNIT_STEP: usize = 11;
    const COST_TRAITS: u32 = 2;
    const COST_EXPERIENCE: u32 = 15;
    const COST_LITERAL: &[u8] = b"seventeen_bytes__";

    const COST_MANAGER: usize = 0x1000;
    const COST_ROSTER: usize = 0x2000;
    const COST_CHARACTER: usize = 0x4000;
    const COST_LEADER_HEAD: usize = 0x8000;
    const COST_ADVISOR_HEAD: usize = 0x9000;
    const COST_LEADER_NODE: usize = 0xa000;
    const COST_ADVISOR_NODE: usize = 0xb000;
    const COST_LEADER_ROLE: usize = 0xd000;
    const COST_ADVISOR_ROLE: usize = 0xe000;
    const COST_UNIT: usize = 0x12000;
    const COST_DEFINITION: usize = 0x16000;
    const COST_TRAIT_ARRAY: usize = 0x1a000;
    const COST_EXPERIENCE_ARRAY: usize = 0x1b000;
    const COST_HEAP: usize = 0x1c000;

    struct CostHeap {
        next: usize,
    }

    impl CostHeap {
        /// Writes a heap MSVC string header, so every literal in this fixture
        /// takes the separate terminator and body read path.
        fn write(&mut self, bytes: &mut [u8], header: usize) {
            let body = self.next;
            self.next += 32;
            put(bytes, header, ((ADDRESS + body) as u64).to_le_bytes());
            put(
                bytes,
                header + 16,
                (COST_LITERAL.len() as u64).to_le_bytes(),
            );
            put(
                bytes,
                header + 24,
                (COST_LITERAL.len() as u64).to_le_bytes(),
            );
            bytes[body..body + COST_LITERAL.len()].copy_from_slice(COST_LITERAL);
            bytes[body + COST_LITERAL.len()] = 0;
        }
    }

    /// A head sentinel plus a right-leaning chain of `count` content nodes.
    fn cost_tree(
        bytes: &mut [u8],
        field: usize,
        head: usize,
        node: usize,
        stride: usize,
        count: u32,
    ) {
        pointer(bytes, field, head);
        put(bytes, field + 8, u64::from(count).to_le_bytes());
        bytes[head + 0x19] = 1;
        if count == 0 {
            for link in [0, 8, 16] {
                pointer(bytes, head + link, head);
            }
            return;
        }
        let last = node + (count as usize - 1) * stride;
        pointer(bytes, head, node);
        pointer(bytes, head + 8, node);
        pointer(bytes, head + 16, last);
        for index in 0..count as usize {
            let current = node + index * stride;
            pointer(bytes, current, head);
            pointer(
                bytes,
                current + 8,
                if index == 0 { head } else { current - stride },
            );
            pointer(
                bytes,
                current + 16,
                if index + 1 == count as usize {
                    head
                } else {
                    current + stride
                },
            );
        }
    }

    pub(crate) fn cost_fixture() -> Vec<u8> {
        use engine::{character_advisor_node, character_leader_node};
        let mut bytes = vec![0; 0x20000];
        let mut heap = CostHeap { next: COST_HEAP };
        pointer(&mut bytes, cgame_state::CHARACTER_MANAGER, COST_MANAGER);
        put(
            &mut bytes,
            COST_MANAGER,
            ((IMAGE + ccharacter_manager::VTABLE_RVA) as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            COST_MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
            COST_ROSTER,
            COST_CHARACTERS as u32,
        );
        for definition in 0..4 {
            let at = COST_DEFINITION + definition * 0x1000;
            put(
                &mut bytes,
                at,
                ((IMAGE + cunit_leader_trait::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                at + cunit_leader_trait::TOKEN_ID,
                (definition as i32).to_le_bytes(),
            );
            put(
                &mut bytes,
                at + cunit_leader_trait::ORDINAL,
                (definition as i32).to_le_bytes(),
            );
            heap.write(&mut bytes, at + cunit_leader_trait::TOKEN);
        }
        for index in 0..COST_CHARACTERS {
            let character = COST_CHARACTER + index * 0x100;
            pointer(&mut bytes, COST_ROSTER + index * 8, character);
            put(
                &mut bytes,
                character,
                ((IMAGE + ccharacter::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                character + ccharacter::TYPE,
                ccharacter::REFERENCE_TYPE.to_le_bytes(),
            );
            put(
                &mut bytes,
                character + ccharacter::ID,
                (index as u32 + 1).to_le_bytes(),
            );
            put(
                &mut bytes,
                character + ccharacter::COUNTRY,
                (index as u32 % 7 + 1).to_le_bytes(),
            );
            heap.write(&mut bytes, character + ccharacter::NAME);
            heap.write(&mut bytes, character + ccharacter::TOKEN);
            let leaders = u32::from(index % COST_LEADER_STEP == 0);
            cost_tree(
                &mut bytes,
                character + ccharacter::LEADER_MAP_HEAD,
                COST_LEADER_HEAD + index * 0x40,
                COST_LEADER_NODE + index * 0x40,
                character_leader_node::SIZE,
                leaders,
            );
            if leaders != 0 {
                let node = COST_LEADER_NODE + index * 0x40;
                let role = COST_LEADER_ROLE + index * 0x20;
                put(
                    &mut bytes,
                    node + character_leader_node::IDEOLOGY,
                    (index as i32).to_le_bytes(),
                );
                pointer(&mut bytes, node + character_leader_node::LEADER, role);
                put(
                    &mut bytes,
                    role,
                    ((IMAGE + ccountry_leader::VTABLE_RVA) as u64).to_le_bytes(),
                );
                pointer(&mut bytes, role + ccountry_leader::CHARACTER, character);
            }
            let advisors = if index % 3 == 1 {
                COST_ADVISORS_PER as u32
            } else {
                0
            };
            cost_tree(
                &mut bytes,
                character + ccharacter::ADVISOR_MAP_HEAD,
                COST_ADVISOR_HEAD + index * 0x40,
                COST_ADVISOR_NODE + index * 0x100,
                character_advisor_node::SIZE,
                advisors,
            );
            for advisor in 0..advisors as usize {
                let node =
                    COST_ADVISOR_NODE + index * 0x100 + advisor * character_advisor_node::SIZE;
                let role = COST_ADVISOR_ROLE + index * 0x200 + advisor * 0x100;
                put(
                    &mut bytes,
                    node + character_advisor_node::KEY_HASH,
                    (advisor as u32).to_le_bytes(),
                );
                heap.write(&mut bytes, node + character_advisor_node::KEY);
                pointer(&mut bytes, node + character_advisor_node::ADVISOR, role);
                put(
                    &mut bytes,
                    role,
                    ((IMAGE + cadvisor::VTABLE_RVA) as u64).to_le_bytes(),
                );
                pointer(&mut bytes, role + cadvisor::CHARACTER, character);
                heap.write(&mut bytes, role + cadvisor::IDEA_TOKEN);
                heap.write(&mut bytes, role + cadvisor::SLOT);
            }
            if index % COST_UNIT_STEP != 2 {
                continue;
            }
            let slot = index / COST_UNIT_STEP;
            let unit = COST_UNIT + slot * 0x1000;
            pointer(&mut bytes, character + ccharacter::UNIT_LEADER, unit);
            put(
                &mut bytes,
                unit,
                ((IMAGE + cunit_leader::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                unit + cunit_leader::TYPE,
                cunit_leader::REFERENCE_TYPE.to_le_bytes(),
            );
            put(
                &mut bytes,
                unit + cunit_leader::ID,
                (index as u32 + 1).to_le_bytes(),
            );
            put(
                &mut bytes,
                unit + cunit_leader::CHARACTER_TYPE,
                ccharacter::REFERENCE_TYPE.to_le_bytes(),
            );
            put(
                &mut bytes,
                unit + cunit_leader::CHARACTER_ID,
                (index as u32 + 1).to_le_bytes(),
            );
            put(
                &mut bytes,
                unit + cunit_leader::EXPERIENCE,
                (index as i64).to_le_bytes(),
            );
            heap.write(&mut bytes, unit + cunit_leader::TOKEN);
            heap.write(&mut bytes, unit + cunit_leader::NAME);
            let traits = COST_TRAIT_ARRAY + slot * 0x100;
            array(
                &mut bytes,
                unit + cunit_leader::TRAIT_DATA,
                traits,
                COST_TRAITS,
            );
            for entry in 0..COST_TRAITS as usize {
                pointer(
                    &mut bytes,
                    traits + entry * 8,
                    COST_DEFINITION + (entry % 4) * 0x1000,
                );
            }
            let experience = COST_EXPERIENCE_ARRAY + slot * 0x400;
            array(
                &mut bytes,
                unit + cunit_leader::TRAIT_EXPERIENCE_DATA,
                experience,
                COST_EXPERIENCE,
            );
            for entry in 0..COST_EXPERIENCE as usize {
                pointer(
                    &mut bytes,
                    experience + entry * 16,
                    COST_DEFINITION + (entry % 4) * 0x1000,
                );
                put(
                    &mut bytes,
                    experience + entry * 16 + 8,
                    (entry as i64).to_le_bytes(),
                );
            }
        }
        bytes
    }

    struct CostMemory<'a> {
        memory: SliceMemory<'a>,
        guards: Cell<usize>,
        spans: Cell<usize>,
        bytes: Cell<usize>,
        prefetched: std::cell::RefCell<Vec<Vec<(usize, usize)>>>,
    }

    impl<'a> CostMemory<'a> {
        fn new(bytes: &'a [u8]) -> Self {
            Self {
                memory: SliceMemory {
                    base: ADDRESS,
                    bytes,
                },
                guards: Cell::new(0),
                spans: Cell::new(0),
                bytes: Cell::new(0),
                prefetched: Default::default(),
            }
        }

        fn span(&self, address: usize, output: &mut [u8]) -> bool {
            self.spans.set(self.spans.get() + 1);
            self.bytes.set(self.bytes.get() + output.len());
            self.memory.copy(address, output)
        }
    }

    impl Memory for CostMemory<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.guards.set(self.guards.get() + 1);
            self.span(address, output)
        }

        fn copy_scattered<const M: usize>(
            &self,
            addresses: [usize; M],
            outputs: &mut [&mut [u8]; M],
        ) -> usize {
            if outputs.iter().any(|output| !output.is_empty()) {
                self.guards.set(self.guards.get() + 1);
            }
            for (index, (address, output)) in addresses.into_iter().zip(outputs).enumerate() {
                if output.is_empty() {
                    continue;
                }
                if address == 0
                    || address.checked_add(output.len()).is_none()
                    || !self.span(address, output)
                {
                    return index;
                }
            }
            M
        }

        fn copy_scattered_prefetched<const M: usize>(
            &self,
            addresses: [usize; M],
            outputs: &mut [&mut [u8]; M],
        ) -> usize {
            self.prefetched.borrow_mut().push(
                addresses
                    .iter()
                    .zip(outputs.iter())
                    .map(|(&address, output)| (address, output.len()))
                    .collect(),
            );
            self.copy_scattered(addresses, outputs)
        }
    }

    #[test]
    fn character_nested_prefetch_selects_fixed_groups_without_rehinting_main_or_heap_scatter() {
        let mut bytes = fixture();
        let heap = 0x8200;
        pointer(&mut bytes, DEFINITION + cunit_leader_trait::TOKEN, heap);
        put(
            &mut bytes,
            DEFINITION + cunit_leader_trait::TOKEN + msvc_string::LENGTH,
            17_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            DEFINITION + cunit_leader_trait::TOKEN + msvc_string::CAPACITY,
            31_u64.to_le_bytes(),
        );
        bytes[heap..heap + 17].fill(0xff);
        let source = CostMemory::new(&bytes);
        let mut job = small_wave_job();
        assert!(copy_view(&mut job, &source, IMAGE, ADDRESS + CHARACTERS[0]).is_some());
        assert!(source.prefetched.borrow().is_empty());
        let definition = job
            .traits
            .definitions
            .header(&source, IMAGE, ADDRESS + DEFINITION, 0)
            .unwrap();
        job.traits.definitions.result(definition).unwrap();
        job.traits
            .definitions
            .consume(&source, definition, 0, true)
            .unwrap();
        job.traits.definitions.result(definition).unwrap();
        assert_eq!(source.prefetched.borrow().len(), 1);

        let node = [0x1000, 0x1040, 0x1080].map(|offset| ADDRESS + offset);
        let role = [0x1100, 0x1140, 0x1180, 0x11c0].map(|offset| ADDRESS + offset);
        let unit = [
            cunit_leader::CHARACTER_TYPE,
            cunit_leader::CATEGORY,
            cunit_leader::EXPERIENCE,
            cunit_leader::SKILL,
        ]
        .map(|offset| ADDRESS + UNITS[0] + offset);
        let mut dead = false;
        assert!(
            take_advisor_fields(&mut job, &source, node.map(Some), [8, 32, 4], &mut dead).is_some()
        );
        assert!(
            take_advisor_fields(&mut job, &source, role.map(Some), [8, 8, 32, 32], &mut dead)
                .is_some()
        );
        assert!(
            take_fixed_fields::<4, false>(
                &mut job,
                &source,
                unit.map(Some),
                [8, 4, 8, 8],
                MASK_UNITS,
                &mut dead,
            )
            .iter()
            .all(Option::is_some)
        );
        assert!(!dead);
        assert_eq!(
            *source.prefetched.borrow(),
            [
                vec![
                    (ADDRESS + DEFINITION, 12),
                    (ADDRESS + DEFINITION + cunit_leader_trait::ORDINAL, 4),
                    (ADDRESS + DEFINITION + cunit_leader_trait::TOKEN, 32),
                ],
                node.into_iter().zip([8, 32, 4]).collect(),
                role.into_iter().zip([8, 8, 32, 32]).collect(),
                unit.into_iter().zip([8, 4, 8, 8]).collect(),
            ]
        );
        // All six groups already used one guard each. Hints preserve every
        // observation and byte, including the ordinary view and heap groups.
        // The view's name token span carries the template pointer (+12).
        assert_eq!(
            (source.guards.get(), source.spans.get(), source.bytes.get()),
            (6, 23, 346 + 12)
        );
    }

    #[test]
    fn character_family_read_cost_stays_at_its_measured_shape() {
        let bytes = cost_fixture();
        let capacity = Capacities {
            rows: [4096; N],
            bytes: [131_072; N],
        };
        let mut captured = Captured::with_capacities(&capacity);
        let memory = CostMemory::new(&bytes);
        let results = walk_family(&memory, IMAGE, ADDRESS, captured.family(), None, None);
        assert_eq!(
            results.map(|result| result.outcome),
            [
                Outcome::Rows { len: 32 },
                Outcome::Rows { len: 5 },
                Outcome::Rows { len: 22 },
                Outcome::Rows { len: 3 },
                Outcome::Rows { len: 6 },
                Outcome::Rows { len: 45 },
            ]
        );
        // The shared walk's read cost, pinned. Making the view's copied-span
        // lookup cheaper removed decoding work, not reads, so every one of
        // these three totals must stay exactly where it was. Each of the three
        // unit leaders adds one guarded read of its owner and visibility
        // inputs (three spans, 13 bytes: none is an army leader). Each of
        // the 32 views' name token spans also carries the template pointer
        // (12 bytes, no read); none of these characters has a template.
        assert_eq!(
            (memory.guards.get(), memory.spans.get(), memory.bytes.get()),
            (588 + 3, 1204 + 9, 16_872 + 39 + 32 * 12)
        );
    }

    #[test]
    fn primary_growth_keeps_shared_outputs_independently_bounded() {
        let mut capacities = small_capacities();
        let mut captured = Captured::with_capacities(&capacities);
        captured.traits.rows.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 19,
        };
        captured.traits.bytes.required = 80;
        captured.grow(&mut capacities);
        assert_eq!(captured.traits.rows.rows.len(), 32);
        assert_eq!(captured.traits.bytes.first.len(), 128);
        let before = captured.allocated_bytes();
        captured.reset();
        captured.grow(&mut capacities);
        assert_eq!(captured.allocated_bytes(), before);
        let idle = Captured::with_capacities(&capacities);
        assert_eq!(idle.allocated_bytes(), captured.allocated_bytes());
    }

    fn split_capacities() -> Capacities {
        Capacities {
            rows: [4096; N],
            bytes: [131_072; N],
        }
    }

    #[test]
    fn split_copy_matches_direct_walk_on_cost_census() {
        use super::characters_test_kit as kit;
        let bytes = cost_fixture();
        let memory = kit::memory(&bytes);
        let capacities = split_capacities();
        let direct = kit::direct(&memory, &capacities);
        let source = WaveMemory::new(&bytes, None);
        let (split, report, _) = kit::split(&source, &capacities);
        assert!(report.raw_ok);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.characters.rows.outcome, Outcome::Rows { len: 32 });
        assert_eq!(split.leaders.rows.outcome, Outcome::Rows { len: 5 });
        assert_eq!(split.advisors.rows.outcome, Outcome::Rows { len: 22 });
        assert_eq!(split.units.rows.outcome, Outcome::Rows { len: 3 });
        assert_eq!(split.traits.rows.outcome, Outcome::Rows { len: 6 });
        assert_eq!(
            split.trait_experience.rows.outcome,
            Outcome::Rows { len: 45 }
        );
        let reads = source.reads.borrow().clone();
        for (rows, arena) in [(0, 0), (1, 1), (21, 64), (22, 22 * 3 * COST_LITERAL.len())] {
            let mut small = capacities;
            small.rows[2] = rows;
            small.bytes[2] = arena;
            let source = WaveMemory::new(&bytes, None);
            let (mut decoded, _, mut job) = kit::split(&source, &small);
            kit::assert_matches_direct(&decoded, &direct);
            assert_eq!(*source.reads.borrow(), reads);
            assert_eq!(job.advisor_demand.rows, 22);
            assert_eq!(job.advisor_demand.bytes, 22 * 3 * COST_LITERAL.len());
            assert!(!job.advisor_demand.overflowed);
            let retained_rows = decoded.advisors.rows.rows.capacity();
            let retained_bytes = decoded.advisors.bytes.first.capacity();
            decoded.reset();
            decoded.grow(&mut small);
            assert_eq!(small.rows[2], retained_rows);
            assert_eq!(small.bytes[2], retained_bytes);
            let context = CharactersCopyContext {
                force_full: false,
                base: IMAGE,
                game_state: ADDRESS,
                active: [true; 6],
                trait_rows: [small.rows[4], small.rows[5]],
                trait_bytes: [small.bytes[4], small.bytes[5]],
            };
            assert!(copy_characters_refresh(&memory, context, &mut job).raw_ok);
            assert_eq!(job.advisor_demand.rows, 22);
            assert_eq!(job.advisor_demand.bytes, 22 * 3 * COST_LITERAL.len());
            decoded.decode_characters_payload(IMAGE, &job, &mut [0; 6]);
            kit::assert_matches_direct(&decoded, &direct);
        }
    }

    #[test]
    fn shared_trait_definitions_are_read_once_charged_and_fresh_after_recycling() {
        let mut bytes = fixture();
        let capacities = split_capacities();
        let context = CharactersCopyContext {
            force_full: false,
            base: IMAGE,
            game_state: ADDRESS,
            active: [true; 6],
            trait_rows: [capacities.rows[4], capacities.rows[5]],
            trait_bytes: [capacities.bytes[4], capacities.bytes[5]],
        };
        let mut job = small_wave_job();
        for value in [b'x', b'y'] {
            bytes[DEFINITION + cunit_leader_trait::TOKEN] = value;
            let source = engine::memory::CountedMemory::new(WaveMemory::new(&bytes, None));
            let native = read_accounting::Capture::new();
            let report = copy_characters_refresh(&source, context, &mut job);
            assert!(report.raw_ok);
            let reads = source.0.reads.borrow();
            assert_eq!(
                reads
                    .iter()
                    .filter(|&&(address, len)| address == ADDRESS + DEFINITION && len == 12)
                    .count(),
                1,
                "one current acquisition shared by four occurrences"
            );
            assert_eq!(
                report.raw_bytes.iter().sum::<u64>(),
                reads.iter().map(|&(_, len)| len as u64).sum::<u64>(),
                "all successful native reads are charged once"
            );
            let counts = read_accounting::snapshot();
            let mut expected_counts = read_accounting::ZERO;
            expected_counts[TABLES[0].index()] = counts[TABLES[0].index()];
            assert_eq!(counts, expected_counts);
            let counted = counts[TABLES[0].index()];
            assert_eq!(
                counted.spans,
                reads.iter().filter(|&&(_, len)| len != 0).count() as u64
            );
            assert_eq!(counted.bytes, report.raw_bytes.iter().sum::<u64>());
            assert!(counted.copy_calls > 0 && counted.scattered_calls > 0);
            assert_eq!((counted.record_calls, counted.plan_calls), (0, 0));
            drop(reads);
            let mut captured = Captured::with_capacities(&capacities);
            captured.decode_characters_payload(IMAGE, &job, &mut [0; 6]);
            let direct =
                characters_test_kit::direct(&characters_test_kit::memory(&bytes), &capacities);
            assert_wave_rows(&captured, &direct);
            assert_eq!(captured.traits.bytes.first[0], value);
            assert_eq!(captured.trait_experience.bytes.first[0], value);
            assert_eq!(native.finish(), counts);
        }
    }

    #[test]
    fn shared_skills_keep_current_rows_native_costs_and_recycled_freshness() {
        use super::characters_test_kit as kit;
        let mut bytes = kit::mapped_fixture();
        pointer(&mut bytes, UNITS[1] + cunit_leader::SKILL, kit::SKILL0);
        let capacities = split_capacities();
        let context = CharactersCopyContext {
            force_full: false,
            base: IMAGE,
            game_state: ADDRESS,
            active: [true; 6],
            trait_rows: [capacities.rows[4], capacities.rows[5]],
            trait_bytes: [capacities.bytes[4], capacities.bytes[5]],
        };
        let mut job = small_wave_job();
        for level in [-7_i32, 9] {
            put(
                &mut bytes,
                kit::SKILL0 + cunit_leader_skill::LEVEL,
                level.to_le_bytes(),
            );
            let source = engine::memory::CountedMemory::new(WaveMemory::new(&bytes, None));
            let native = read_accounting::Capture::new();
            let report = copy_characters_refresh(&source, context, &mut job);
            let counts = native.finish();
            assert!(report.raw_ok);
            let skill = ADDRESS + kit::SKILL0;
            for (address, len) in [
                (skill, 8),
                (skill + cunit_leader_skill::VALID, 1),
                (skill + cunit_leader_skill::LEVEL, 12),
            ] {
                assert_eq!(
                    source
                        .0
                        .reads
                        .borrow()
                        .iter()
                        .filter(|&&key| key == (address, len))
                        .count(),
                    1
                );
                assert_eq!(
                    job.log
                        .spans
                        .iter()
                        .filter(|span| span.addr == address && span.len as usize == len)
                        .count(),
                    2
                );
            }
            assert_eq!(
                counts[TABLES[0].index()].bytes,
                report.raw_bytes.iter().sum::<u64>()
            );
            let mut decoded = Captured::with_capacities(&capacities);
            decoded.decode_characters_payload(IMAGE, &job, &mut [0; 6]);
            kit::assert_matches_direct(&decoded, &kit::direct(&kit::memory(&bytes), &capacities));
            assert_eq!(decoded.units.rows.rows[0].native.skill_level, level);
            assert_eq!(decoded.units.rows.rows[1].native.skill_level, level);
        }
        for offset in [0, cunit_leader_skill::VALID, cunit_leader_skill::LEVEL] {
            let source = WaveMemory::new(&bytes, Some(ADDRESS + kit::SKILL0 + offset));
            let report = copy_characters_refresh(&source, context, &mut job);
            assert!(report.raw_ok);
            assert!(job.skills.iter().all(|skill| skill.pointer == 0));
            let mut decoded = Captured::with_capacities(&capacities);
            decoded.decode_characters_payload(IMAGE, &job, &mut [0; 6]);
            kit::assert_matches_direct(&decoded, &kit::direct(&source, &capacities));
        }
    }

    #[test]
    fn shared_skill_hits_keep_alias_failures_as_separate_raw_occurrences() {
        use super::characters_test_kit as kit;
        let bytes = kit::mapped_fixture();
        let skill = ADDRESS + kit::SKILL0;
        let source = CostMemory::new(&bytes);
        let mut job = small_wave_job();
        let pointer_start = job
            .log
            .take(
                &source,
                ADDRESS + UNITS[0] + cunit_leader::SKILL,
                8,
                MASK_UNITS,
            )
            .unwrap();
        let mut dead = Dead::default();
        copy_unit_skill(&mut job, &source, IMAGE, pointer_start, &mut dead);
        assert!(!dead.units);
        assert_eq!(source.guards.get(), 2, "pointer plus first skill gather");
        let fault = WaveMemory::new(&bytes, Some(skill));
        assert!(job.log.take(&fault, skill, 8, MASK_CHARACTERS).is_none());
        let native_bytes = job.log.raw_bytes;
        copy_unit_skill(&mut job, &source, IMAGE, pointer_start, &mut dead);
        assert!(!dead.units);
        assert_eq!(source.guards.get(), 2, "cache hit makes no native calls");
        assert_eq!(job.log.raw_bytes, native_bytes);
        let owned = crate::raw::Recorded::build(&job.log);
        let mut vtable = [0; 8];
        assert!(owned.copy(skill, &mut vtable));
        assert!(!owned.copy(skill, &mut vtable));
        assert!(owned.copy(skill, &mut vtable));
        assert_eq!(
            u64::from_le_bytes(vtable),
            (IMAGE + cunit_leader_skill::VTABLE_RVA) as u64
        );
    }

    #[test]
    fn trait_definition_staging_exhaustion_requires_growth_before_acceptance() {
        let bytes = fixture();
        let capacities = split_capacities();
        let context = CharactersCopyContext {
            force_full: false,
            base: IMAGE,
            game_state: ADDRESS,
            active: [true; 6],
            trait_rows: [capacities.rows[4], capacities.rows[5]],
            trait_bytes: [capacities.bytes[4], capacities.bytes[5]],
        };
        let source = characters_test_kit::memory(&bytes);
        let mut job = small_wave_job();
        job.traits.definitions.entries = Vec::new();
        job.traits.definitions.update_charge();
        assert!(!copy_characters_refresh(&source, context, &mut job).raw_ok);
        assert!(!job.log.overflowed);
        assert!(job.grow_for_next());
        job.grow();
        assert!(copy_characters_refresh(&source, context, &mut job).raw_ok);
        let mut captured = Captured::with_capacities(&capacities);
        captured.decode_characters_payload(IMAGE, &job, &mut [0; 6]);
        assert_wave_rows(
            &captured,
            &characters_test_kit::direct(&source, &capacities),
        );
    }

    #[test]
    fn split_copy_matches_direct_walk_with_skills_kinds_and_heap_names() {
        use super::characters_test_kit as kit;
        let bytes = kit::mapped_fixture();
        let memory = kit::memory(&bytes);
        let capacities = split_capacities();
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok);
        kit::assert_matches_direct(&split, &direct);
        // Spot-checks: leader ideology, heap advisor key, skill, army and
        // navy components, and the third character's heap name.
        assert_eq!(split.leaders.rows.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.leaders.rows.rows[0].ideology, 7);
        assert_eq!(split.advisors.rows.outcome, Outcome::Rows { len: 1 });
        let advisor = &split.advisors.rows.rows[0];
        assert_eq!(
            &split.advisors.bytes.first[advisor.key_start..advisor.key_start + advisor.key_len],
            b"twenty-byte-advisor!"
        );
        let units = &split.units.rows.rows;
        assert_eq!(units[0].native.skill_status, 2);
        assert_eq!(units[0].native.skill_level, 3);
        assert_eq!(units[0].native.skill_required_experience, 100);
        assert_eq!(units[0].native.skill_category, 5);
        assert_eq!(units[1].native.kind, cunit_leader::KIND_ARMY);
        assert_eq!(units[1].native.components, [11, 22, 33, 44]);
        assert_eq!(units[2].native.kind, cunit_leader::KIND_NAVY);
        assert_eq!(units[2].native.components, [51, 52, 53, 54]);
        let character = &split.characters.rows.rows[2];
        assert_eq!(
            &split.characters.bytes.first
                [character.name_start..character.name_start + character.name_len],
            b"heap-named-character!"
        );
    }

    #[test]
    fn split_keeps_character_and_unit_leader_registry_ids_distinct() {
        use super::characters_test_kit as kit;
        let mut bytes = kit::mapped_fixture();
        for (index, unit) in UNITS.into_iter().enumerate() {
            put(
                &mut bytes,
                unit + cunit_leader::ID,
                (17 + index as u32).to_le_bytes(),
            );
        }
        let memory = kit::memory(&bytes);
        let capacities = split_capacities();
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.units.rows.outcome, Outcome::Rows { len: 3 });
        assert!(matches!(split.traits.rows.outcome, Outcome::Rows { len } if len > 0));
        assert!(matches!(split.trait_experience.rows.outcome, Outcome::Rows { len } if len > 0));
    }

    #[test]
    fn characters_copy_sizes_cover_documented_read_set() {
        // S3: every Memory::copy size the characters copy pass issues,
        // recorded through the real pass. Roster headers (16), words (4/8),
        // msvc_string headers (32) with heap payloads and terminators (1),
        // tree nodes and skill entries. Alignment follows the engine layout.
        use super::characters_test_kit as kit;
        use super::{CharactersCopyContext, CharactersJob};
        use engine::Memory;
        use std::collections::BTreeSet;
        struct Recorder<'a> {
            inner: engine::memory::SliceMemory<'a>,
            sizes: std::cell::RefCell<BTreeSet<usize>>,
        }
        impl Memory for Recorder<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.sizes.borrow_mut().insert(output.len());
                self.inner.copy(address, output)
            }
        }
        let bytes = kit::mapped_fixture();
        let inner = kit::memory(&bytes);
        let memory = Recorder {
            inner,
            sizes: std::cell::RefCell::new(BTreeSet::new()),
        };
        let ctx = CharactersCopyContext {
            force_full: false,
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            active: [true; 6],
            trait_rows: [4096; 2],
            trait_bytes: [131_072; 2],
        };
        let mut first = CharactersJob::new();
        let report = copy_characters_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok);
        let sizes = memory.sizes.borrow().clone();
        for required in [1, 4, 8, 16, 32] {
            assert!(
                sizes.contains(&required),
                "missing size {required} in {sizes:?}"
            );
        }
    }

    #[test]
    fn characters_truncation_matrix_keeps_advisors_complete() {
        // Other tables keep their bounded outputs. Advisors grow from this
        // payload and match a sufficiently sized single typed walk.
        // Cost fixture holds 32 characters, 5 leaders, 22 advisors, 3 units,
        // 6 traits, 45 experience rows. u32 unread saturation is not
        // reachable synthetically and is covered by the walk's own
        // saturating arithmetic.
        use super::characters_test_kit as kit;
        let bytes = cost_fixture();
        let memory = kit::memory(&bytes);
        for (rows, byte) in [
            ([0; N], 0),
            ([1; N], 64),
            ([2; N], 64),
            ([32, 5, 22, 3, 6, 45], 131_072),
            ([33, 6, 23, 4, 7, 46], 131_072),
            ([4096; N], 131_072),
        ] {
            let capacities = Capacities {
                rows,
                bytes: [byte; N],
            };
            let mut complete = capacities;
            complete.rows[2] = complete.rows[2].max(22);
            complete.bytes[2] = complete.bytes[2].max(22 * 3 * COST_LITERAL.len());
            let direct = kit::direct(&memory, &complete);
            let (split, report, _) = kit::split(&memory, &capacities);
            assert!(report.raw_ok, "rows={rows:?}");
            kit::assert_matches_direct(&split, &direct);
        }
        // Exact-fit arm pins Rows on every table.
        let capacities = Capacities {
            rows: [32, 5, 22, 3, 6, 45],
            bytes: [131_072; N],
        };
        let (split, _, _) = kit::split(&memory, &capacities);
        assert_eq!(split.characters.rows.outcome, Outcome::Rows { len: 32 });
        assert_eq!(split.leaders.rows.outcome, Outcome::Rows { len: 5 });
        assert_eq!(
            split.trait_experience.rows.outcome,
            Outcome::Rows { len: 45 }
        );
    }

    #[test]
    fn advisor_decode_preserves_late_failure_with_small_buffers() {
        use super::characters_test_kit as kit;
        let bytes = cost_fixture();
        let node = COST_ADVISOR_NODE + 0x100 + character_advisor_node::SIZE;
        let role = COST_ADVISOR_ROLE + 0x300;
        let slot = role + cadvisor::SLOT;
        let payload = u64::from_le_bytes(bytes[slot..slot + 8].try_into().unwrap()) as usize;
        for fault in [
            ADDRESS + node + msvc_tree_node::NIL,
            ADDRESS + role + cadvisor::IDEA_TOKEN,
            payload,
            payload + COST_LITERAL.len(),
        ] {
            let memory = WaveMemory::new(&bytes, Some(fault));
            let direct = kit::direct(&memory, &split_capacities());
            assert!(matches!(
                direct.advisors.rows.outcome,
                Outcome::Unavailable { .. }
            ));
            for (rows, arena) in [(0, 0), (1, 3 * COST_LITERAL.len()), (22, 1)] {
                let mut capacities = split_capacities();
                capacities.rows[2] = rows;
                capacities.bytes[2] = arena;
                let source = WaveMemory::new(&bytes, Some(fault));
                let (split, report, _) = kit::split(&source, &capacities);
                assert!(report.raw_ok);
                kit::assert_matches_direct(&split, &direct);
                assert_eq!(split.advisors.rows.rows[0], direct.advisors.rows.rows[0]);
                assert_eq!(
                    source
                        .reads
                        .borrow()
                        .iter()
                        .filter(|&&(address, len)| {
                            address <= fault && fault < address.saturating_add(len)
                        })
                        .count(),
                    1,
                    "one original failed attempt, fault={fault:x}"
                );
            }
        }
    }

    struct FixedFieldMemory {
        failed_call: Option<usize>,
        calls: std::cell::RefCell<Vec<(usize, usize)>>,
        guards: Cell<usize>,
        prefetched: Cell<usize>,
    }

    impl FixedFieldMemory {
        fn new(failed_call: Option<usize>) -> Self {
            Self {
                failed_call,
                calls: Default::default(),
                guards: Cell::new(0),
                prefetched: Cell::new(0),
            }
        }

        fn observe(&self, address: usize, output: &mut [u8]) -> bool {
            let call = self.calls.borrow().len();
            self.calls.borrow_mut().push((address, output.len()));
            // Repeated exact keys deliberately observe different bytes.
            for (index, byte) in output.iter_mut().enumerate() {
                *byte = (call * 31 + index + address) as u8;
            }
            self.failed_call != Some(call)
        }
    }

    impl Memory for FixedFieldMemory {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.guards.set(self.guards.get() + 1);
            self.observe(address, output)
        }

        fn copy_scattered<const M: usize>(
            &self,
            addresses: [usize; M],
            outputs: &mut [&mut [u8]; M],
        ) -> usize {
            self.guards.set(self.guards.get() + 1);
            for (index, (address, output)) in addresses.into_iter().zip(outputs).enumerate() {
                if output.is_empty() {
                    continue;
                }
                if address == 0
                    || address.checked_add(output.len()).is_none()
                    || !self.observe(address, output)
                {
                    return index;
                }
            }
            M
        }

        fn copy_scattered_prefetched<const M: usize>(
            &self,
            addresses: [usize; M],
            outputs: &mut [&mut [u8]; M],
        ) -> usize {
            self.prefetched.set(self.prefetched.get() + 1);
            self.copy_scattered(addresses, outputs)
        }
    }

    fn scalar_fixed_fields<const M: usize, const CONTINUE: bool>(
        job: &mut CharactersJob,
        memory: &impl Memory,
        addresses: [Option<usize>; M],
        lengths: [usize; M],
        mask: u32,
        dead: &mut bool,
    ) -> [Option<usize>; M] {
        let mut starts = [None; M];
        for index in 0..M {
            // Independent reference to the old unconditional scalar calls
            // and their caller-controlled prefix breaks.
            starts[index] = addresses[index]
                .and_then(|address| job.log.take(memory, address, lengths[index], mask));
            if starts[index].is_none() {
                *dead = true;
                if !CONTINUE {
                    break;
                }
            }
        }
        starts
    }

    #[test]
    fn heap_literal_gather_keeps_scalar_observations_faults_and_growth() {
        for len in [0, 1, 17, 64] {
            for span_room in 0..=4 {
                for byte_room in [0, 1, len, len + 1, 2 * (len + 1)] {
                    for failed in [None, Some(0), Some(1), Some(2), Some(3)] {
                        let make_job = || CharactersJob {
                            log: crate::raw::RawLog::new(span_room, byte_room),
                            ..CharactersJob::default()
                        };
                        let mut actual = make_job();
                        let mut scalar = make_job();
                        let actual_memory = FixedFieldMemory::new(failed);
                        let scalar_memory = FixedFieldMemory::new(failed);
                        // Repeated exact keys deliberately produce different
                        // bytes. Each occurrence and failure must still survive.
                        for _ in 0..2 {
                            let result = copy_heap_literal_bytes(
                                &mut actual,
                                &actual_memory,
                                0x2000,
                                len,
                                MASK_CHARACTERS,
                            );
                            let expected = scalar
                                .log
                                .take(&scalar_memory, 0x2000 + len, 1, MASK_CHARACTERS)
                                .is_some()
                                && (len == 0
                                    || scalar
                                        .log
                                        .take(&scalar_memory, 0x2000, len, MASK_CHARACTERS)
                                        .is_some());
                            assert_eq!(result, expected);
                            assert_eq!(actual.log.spans, scalar.log.spans);
                            assert_eq!(actual.log.bytes, scalar.log.bytes);
                            assert_eq!(actual.log.fault, scalar.log.fault);
                            assert_eq!(actual.log.raw_bytes, scalar.log.raw_bytes);
                            assert_eq!(actual.log.overflowed, scalar.log.overflowed);
                            assert_eq!(
                                *actual_memory.calls.borrow(),
                                *scalar_memory.calls.borrow()
                            );
                        }
                        if len != 0
                            && span_room == 4
                            && byte_room == 2 * (len + 1)
                            && failed.is_none()
                        {
                            assert_eq!(actual_memory.guards.get(), 2);
                            assert_eq!(scalar_memory.guards.get(), 4);
                        }
                        actual.log.grow();
                        scalar.log.grow();
                        assert_eq!(actual.log.spans.capacity(), scalar.log.spans.capacity());
                        assert_eq!(actual.log.bytes.capacity(), scalar.log.bytes.capacity());
                    }
                }
            }
        }
    }

    fn assert_fixed_fields_match_scalar<const M: usize, const CONTINUE: bool>(
        addresses: [Option<usize>; M],
        lengths: [usize; M],
        mask: u32,
        span_room: usize,
        byte_room: usize,
        failed_call: Option<usize>,
    ) -> (usize, usize) {
        let make_job = || {
            let mut job = CharactersJob {
                log: crate::raw::RawLog::new(1 + span_room, 3 + byte_room),
                game_state: 0,
                ..CharactersJob::default()
            };
            let seed = SliceMemory {
                base: 0x900,
                bytes: &[17; 3],
            };
            assert_eq!(job.log.take(&seed, 0x900, 3, MASK_CHARACTERS), Some(0));
            job
        };
        let mut actual = make_job();
        let mut expected = make_job();
        let memory = FixedFieldMemory::new(failed_call);
        let scalar_memory = FixedFieldMemory::new(failed_call);
        let mut dead = false;
        let mut scalar_dead = false;
        let starts = take_fixed_fields::<M, CONTINUE>(
            &mut actual,
            &memory,
            addresses,
            lengths,
            mask,
            &mut dead,
        );
        let scalar_starts = scalar_fixed_fields::<M, CONTINUE>(
            &mut expected,
            &scalar_memory,
            addresses,
            lengths,
            mask,
            &mut scalar_dead,
        );
        assert_eq!(starts, scalar_starts);
        assert_eq!(dead, scalar_dead);
        assert_eq!(actual.log.spans, expected.log.spans);
        assert_eq!(actual.log.bytes, expected.log.bytes);
        assert_eq!(actual.log.fault, expected.log.fault);
        assert_eq!(actual.log.raw_bytes, expected.log.raw_bytes);
        assert_eq!(actual.log.overflowed, expected.log.overflowed);
        assert_eq!(actual.log.has_bulk, expected.log.has_bulk);
        assert_eq!(actual.log.grow_for_next(), expected.log.grow_for_next());
        assert_eq!(*memory.calls.borrow(), *scalar_memory.calls.borrow());
        assert_eq!(
            memory.prefetched.get(),
            usize::from(
                addresses.iter().all(Option::is_some)
                    && lengths.iter().filter(|&&len| len != 0).count() <= span_room
                    && lengths.iter().sum::<usize>() <= byte_room
            ),
            "only a whole admitted group selects the prefetched provider"
        );
        assert_eq!(scalar_memory.prefetched.get(), 0);
        assert_eq!(actual.log.spans.capacity(), 1 + span_room);
        assert_eq!(actual.log.bytes.capacity(), 3 + byte_room);
        // Also compare authoritative repeated-key replay, including sticky
        // terminal successes/faults, with independent reader cursors.
        {
            let reader = crate::raw::Recorded::build(&actual.log);
            let independent = crate::raw::Recorded::build(&actual.log);
            let reference = crate::raw::Recorded::build(&expected.log);
            for _ in 0..2 {
                for index in (0..M).rev() {
                    if let Some(address) = addresses[index] {
                        let mut copied = vec![0xa5; lengths[index]];
                        let mut other = copied.clone();
                        let mut scalar = copied.clone();
                        let result = reader.copy(address, &mut copied);
                        assert_eq!(result, independent.copy(address, &mut other));
                        assert_eq!(result, reference.copy(address, &mut scalar));
                        assert_eq!(copied, other);
                        assert_eq!(copied, scalar);
                    }
                }
            }
        }
        // Private exhaustion targets must produce identical off-thread
        // growth, not merely the same public overflow flag.
        actual.log.grow();
        expected.log.grow();
        assert_eq!(actual.log.spans.capacity(), expected.log.spans.capacity());
        assert_eq!(actual.log.bytes.capacity(), expected.log.bytes.capacity());
        (memory.guards.get(), scalar_memory.guards.get())
    }

    fn fixed_field_matrix<const M: usize, const CONTINUE: bool>(lengths: [usize; M], mask: u32) {
        let addresses = std::array::from_fn(|index| Some(0x1000 + index * 64));
        let bytes: usize = lengths.iter().sum();
        assert_eq!(
            assert_fixed_fields_match_scalar::<M, CONTINUE>(
                addresses, lengths, mask, M, bytes, None
            ),
            (1, M),
        );
        for failed in std::iter::once(None).chain((0..M).map(Some)) {
            for span_room in 0..=M {
                assert_fixed_fields_match_scalar::<M, CONTINUE>(
                    addresses, lengths, mask, span_room, bytes, failed,
                );
            }
            for byte_room in 0..=bytes {
                assert_fixed_fields_match_scalar::<M, CONTINUE>(
                    addresses, lengths, mask, M, byte_room, failed,
                );
            }
        }
        for index in 0..M {
            for invalid in [None, Some(0), Some(0x0000_8000_0000_0000), Some(usize::MAX)] {
                let mut changed = addresses;
                changed[index] = invalid;
                for span_room in [0, index, M] {
                    assert_fixed_fields_match_scalar::<M, CONTINUE>(
                        changed, lengths, mask, span_room, bytes, None,
                    );
                }
            }
        }
    }

    #[test]
    fn characters_fixed_gathers_match_scalar_at_every_field_fault_and_capacity() {
        fixed_field_matrix::<3, false>([12, 4, 32], MASK_TRAITS);
        fixed_field_matrix::<3, false>([12, 4, 32], MASK_EXPERIENCE);
        fixed_field_matrix::<4, false>([8, 4, 8, 8], MASK_UNITS);
        fixed_field_matrix::<4, false>([4; 4], MASK_UNITS);
        fixed_field_matrix::<2, false>([8; 2], MASK_TRAIT_FALLBACK);
        fixed_field_matrix::<3, true>([8, 1, 12], MASK_UNITS);
        fixed_field_matrix::<2, true>([32; 2], MASK_UNITS);
    }

    #[test]
    fn characters_fixed_gathers_preserve_changing_repeated_observations_and_faults() {
        for failed in [None, Some(0), Some(1), Some(2), Some(3)] {
            for span_room in 0..=4 {
                for byte_room in [0, 7, 8, 15, 16, 23, 24, 31, 32] {
                    assert_fixed_fields_match_scalar::<4, false>(
                        [Some(0x1000); 4],
                        [8; 4],
                        MASK_UNITS,
                        span_room,
                        byte_room,
                        failed,
                    );
                    assert_fixed_fields_match_scalar::<4, true>(
                        [Some(0x1000); 4],
                        [8; 4],
                        MASK_UNITS,
                        span_room,
                        byte_room,
                        failed,
                    );
                }
            }
        }
    }

    #[test]
    fn characters_fixed_gathers_keep_family_rows_and_faults_exact() {
        use super::characters_test_kit as kit;
        let bytes = kit::mapped_fixture();
        let capacities = Capacities {
            rows: [8; N],
            bytes: [256; N],
        };
        let fields = [
            (UNITS[0] + cunit_leader::CHARACTER_TYPE, 8),
            (UNITS[0] + cunit_leader::CATEGORY, 4),
            (UNITS[0] + cunit_leader::EXPERIENCE, 8),
            (UNITS[0] + cunit_leader::SKILL, 8),
            (UNITS[0] + cunit_leader::TYPE, 8),
            (kit::SKILL0, 8),
            (kit::SKILL0 + cunit_leader_skill::VALID, 1),
            (kit::SKILL0 + cunit_leader_skill::LEVEL, 12),
            (UNITS[1] + carmy_leader::ATTACK, 4),
            (UNITS[1] + carmy_leader::DEFENSE, 4),
            (UNITS[1] + carmy_leader::PLANNING, 4),
            (UNITS[1] + carmy_leader::LOGISTICS, 4),
            (0x5200 + cnavy_leader::ATTACK, 4),
            (0x5200 + cnavy_leader::DEFENSE, 4),
            (0x5200 + cnavy_leader::MANEUVERING, 4),
            (0x5200 + cnavy_leader::COORDINATION, 4),
            (UNITS[0] + cunit_leader::NAME, 32),
            (UNITS[0] + cunit_leader::TOKEN, 32),
            (DEFINITION, 12),
            (DEFINITION + cunit_leader_trait::ORDINAL, 4),
            (DEFINITION + cunit_leader_trait::TOKEN, 32),
        ];
        for (offset, len) in fields {
            for byte in [0, len - 1] {
                let fault = ADDRESS + offset + byte;
                let memory = FaultMemory {
                    memory: kit::memory(&bytes),
                    fault: fault..fault + 1,
                    failures: Cell::new(0),
                };
                let direct = kit::direct(&memory, &capacities);
                let mut captured = Captured::with_capacities(&capacities);
                let make_job = || CharactersJob {
                    log: crate::raw::RawLog::new(4096, 65_536),
                    game_state: 0,
                    active: [true; 6],
                    traits: TraitCapture::new(),
                    advisor_demand: AdvisorDemand::default(),
                    skills: [SkillCopy::default(); 32],
                    ..Default::default()
                };
                let mut first = make_job();
                let report = copy_characters_refresh(
                    &memory,
                    CharactersCopyContext {
                        force_full: false,
                        base: IMAGE,
                        game_state: ADDRESS,
                        active: [true; 6],
                        trait_rows: [capacities.rows[4], capacities.rows[5]],
                        trait_bytes: [capacities.bytes[4], capacities.bytes[5]],
                    },
                    &mut first,
                );
                assert!(report.raw_ok, "field={offset:x}/{len} byte={byte}");
                assert!(
                    memory.failures.get() > 0,
                    "unexercised field={offset:x}/{len}"
                );
                assert!(
                    first
                        .log
                        .fault
                        .iter()
                        .chain(&first.traits.fault)
                        .any(|&fault| fault)
                );
                captured.decode_characters_payload(IMAGE, &first, &mut [0; 6]);
                kit::assert_matches_direct(&captured, &direct);
            }
        }
    }

    #[test]
    fn advisor_gathers_preserve_every_partial_fault_and_exact_prefix() {
        use super::characters_test_kit as kit;
        let bytes = kit::mapped_fixture();
        let role_at = kit::ADV_NODE + character_advisor_node::ADVISOR;
        let role = u64::from_le_bytes(bytes[role_at..role_at + 8].try_into().unwrap()) as usize;
        let key = kit::ADV_NODE + character_advisor_node::KEY;
        let payload = u64::from_le_bytes(bytes[key..key + 8].try_into().unwrap()) as usize;
        let key_len = u64::from_le_bytes(bytes[key + 16..key + 24].try_into().unwrap()) as usize;
        let fields = [
            (ADDRESS + role_at, 8),
            (ADDRESS + kit::ADV_NODE + character_advisor_node::KEY, 32),
            (
                ADDRESS + kit::ADV_NODE + character_advisor_node::KEY_HASH,
                4,
            ),
            (role, 8),
            (role + cadvisor::CHARACTER, 8),
            (role + cadvisor::IDEA_TOKEN, 32),
            (role + cadvisor::SLOT, 32),
            (payload + key_len, 1),
            (payload, key_len),
        ];
        for (failed, &(address, len)) in fields.iter().enumerate() {
            for byte in [0, len - 1] {
                let memory = FaultMemory {
                    memory: kit::memory(&bytes),
                    fault: address + byte..address + byte + 1,
                    failures: Cell::new(0),
                };
                let mut capacities = split_capacities();
                let direct = kit::direct(&memory, &capacities);
                capacities.rows[2] = 0;
                capacities.bytes[2] = 0;
                let (split, report, job) = kit::split(&memory, &capacities);
                kit::assert_matches_direct(&split, &direct);
                assert!(report.raw_ok);
                assert_eq!(job.log.fault, [false, false, true, false, false, false]);
                // One failure in the direct walk and one in the current raw pass.
                // No failed field is retried and no later field is copied.
                assert_eq!(memory.failures.get(), 2, "field={failed} byte={byte}");
                let observed: Vec<_> = job
                    .log
                    .spans
                    .iter()
                    .filter(|span| fields.contains(&(span.addr, span.len as usize)))
                    .map(|span| (span.addr, span.len as usize, span.ok))
                    .collect();
                let expected: Vec<_> = fields[..=failed]
                    .iter()
                    .enumerate()
                    .map(|(index, &(address, len))| (address, len, index < failed))
                    .collect();
                assert_eq!(observed, expected, "field={failed} byte={byte}");
            }
        }
        // Unreadable or overflowing terminators fail before output capacity;
        // huge unobserved payload lengths must not replace that native error
        // with an allocation failure.
        for len in [1_u64 << 30, u64::MAX] {
            let mut invalid = bytes.clone();
            put(&mut invalid, key + 16, len.to_le_bytes());
            put(&mut invalid, key + 24, len.to_le_bytes());
            let memory = kit::memory(&invalid);
            let mut capacities = split_capacities();
            let direct = kit::direct(&memory, &capacities);
            capacities.rows[2] = 0;
            capacities.bytes[2] = 0;
            let (split, _, _) = kit::split(&memory, &capacities);
            kit::assert_matches_direct(&split, &direct);
            assert!(split.advisors.bytes.first.is_empty());
        }
    }

    #[test]
    fn advisor_gather_address_overflow_keeps_the_scalar_prefix_without_a_fault_read() {
        let bytes = [7; 24];
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut job = CharactersJob {
            log: crate::raw::RawLog::new(8, 64),
            game_state: 0,
            ..CharactersJob::default()
        };
        let mut dead = false;
        assert!(
            take_advisor_fields(
                &mut job,
                &memory,
                [Some(0x1000), None, Some(0x1010)],
                [8; 3],
                &mut dead,
            )
            .is_none()
        );
        assert!(dead);
        assert_eq!(job.log.bytes, [7; 8]);
        assert_eq!(job.log.spans.len(), 1);
        assert_eq!(job.log.spans[0].addr, 0x1000);
        assert!(job.log.spans[0].ok);
        assert_eq!(job.log.fault, [false; 6]);
        assert!(!job.log.overflowed);
    }

    #[test]
    fn split_copy_matches_direct_walk_on_fault_matrix() {
        use super::characters_test_kit as kit;
        // (fault offset from the image base, expected primary fault masks in
        // family order). Outcomes including reason strings must match the
        // typed walk exactly; masks describe source-read failures.
        for (offset, fault_mask) in [
            (
                cgame_state::CHARACTER_MANAGER,
                [true, true, true, true, true, true],
            ),
            (ROSTER, [true, true, true, true, true, true]),
            (CHARACTERS[0], [true, true, true, true, true, true]),
            (
                CHARACTERS[0] + ccharacter::COUNTRY,
                [true, true, true, false, false, false],
            ),
            (
                CHARACTERS[0] + ccharacter::NAME_TOKEN_ID,
                [true, false, false, false, false, false],
            ),
            (
                CHARACTERS[0] + ccharacter::NAME,
                [true, false, false, false, false, false],
            ),
            (
                CHARACTERS[0] + ccharacter::LEADER_MAP_HEAD,
                [false, true, false, false, false, false],
            ),
            (
                CHARACTERS[0] + ccharacter::ADVISOR_MAP_HEAD,
                [false, false, true, false, false, false],
            ),
            (
                CHARACTERS[0] + ccharacter::UNIT_LEADER,
                [false, false, false, true, true, true],
            ),
            (kit::HEAP_AREA, [false, false, true, false, false, false]),
            (
                kit::LEAD_NODE + character_leader_node::IDEOLOGY,
                [false, true, false, false, false, false],
            ),
            (UNITS[0], [false, false, false, true, true, true]),
            (
                UNITS[0] + cunit_leader::TRAIT_DATA,
                [false, false, false, false, true, false],
            ),
            (DEFINITION, [false, false, false, false, true, true]),
            (EXPERIENCE_ARRAY, [false, false, false, false, false, true]),
            (kit::SKILL0, [false, false, false, true, false, false]),
            (
                kit::HEAP_AREA + 64,
                [true, false, false, false, false, false],
            ),
        ] {
            let bytes = kit::mapped_fixture();
            let fault = ADDRESS + offset;
            let memory = FaultMemory {
                memory: SliceMemory {
                    base: ADDRESS,
                    bytes: &bytes,
                },
                fault: fault..fault + 1,
                failures: Cell::new(0),
            };
            let capacities = split_capacities();
            let direct = kit::direct(&memory, &capacities);
            let (split, report, job) = kit::split(&memory, &capacities);
            assert!(report.raw_ok, "offset={offset:x}");
            kit::assert_matches_direct(&split, &direct);
            let actual: [bool; 6] = std::array::from_fn(|index| {
                job.log.fault[index] || (index >= 4 && job.traits.fault[index - 4])
            });
            assert_eq!(actual, fault_mask, "offset={offset:x}");
        }
    }

    #[test]
    fn failed_character_refresh_preserves_capacity() {
        let mut captured = Captured::with_capacities(&small_capacities());
        captured.raw_active = [true; 6];
        captured.characters.rows.outcome = Outcome::Rows { len: 1 };
        captured.characters.bytes.len = 1;
        captured.characters.bytes.required = 7;
        let bytes = captured.allocated_bytes();
        captured.fail_characters_capture([true; 6], "character worker failed");
        assert!(!captured.raw_active.iter().any(|&active| active));
        for outcome in &captured.outcomes()[..6] {
            assert_eq!(
                *outcome,
                Outcome::Unavailable {
                    reason: "character worker failed"
                }
            );
        }
        assert_eq!(captured.characters.bytes.len, 0);
        assert_eq!(captured.characters.bytes.required, 7);
        assert_eq!(captured.allocated_bytes(), bytes);

        captured.leaders.bytes.required = 11;
        captured.traits.bytes.required = 13;
        captured.fail_characters_capture(
            [true, false, false, true, false, false],
            "next worker failed",
        );
        assert_eq!(captured.characters.bytes.required, 7);
        assert_eq!(captured.leaders.bytes.required, 0);
        assert_eq!(captured.traits.bytes.required, 0);
        assert_eq!(
            captured.leaders.rows.outcome,
            Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            }
        );
        assert_eq!(captured.allocated_bytes(), bytes);
    }

    #[test]
    fn staging_overflow_falls_back_to_typed_walk() {
        use super::characters_test_kit as kit;
        let bytes = kit::mapped_fixture();
        let memory = kit::memory(&bytes);
        let capacities = split_capacities();
        let ctx = CharactersCopyContext {
            force_full: false,
            base: IMAGE,
            game_state: ADDRESS,
            active: [true; 6],
            trait_rows: [capacities.rows[4], capacities.rows[5]],
            trait_bytes: [capacities.bytes[4], capacities.bytes[5]],
        };
        let mut first = CharactersJob {
            log: crate::raw::RawLog::new(8, 8),
            game_state: 0,
            active: [true; 6],
            traits: TraitCapture::new(),
            advisor_demand: AdvisorDemand::default(),
            skills: [SkillCopy::default(); 32],
            ..Default::default()
        };
        let report = copy_characters_refresh(&memory, ctx, &mut first);
        assert!(!report.raw_ok);
        assert!(first.log.overflowed);
        assert!(first.grow_for_next());
    }
    // Frozen serial family schedule. Engine field/body parsers are unchanged.
    fn serial_family_reference(
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        family: Family<'_>,
    ) -> [ReadResult; 6] {
        let Family {
            mut characters,
            mut leaders,
            mut advisors,
            mut units,
            mut traits,
            mut experience,
        } = family;
        let result = (|| {
            let roster = ccharacter_manager::Roster::read(memory, base, game_state)?;
            let mut cursor = roster.cursor();
            loop {
                if !characters.active()
                    && !leaders.active()
                    && !advisors.active()
                    && !units.active()
                    && !traits.active()
                    && !experience.active()
                {
                    break;
                }
                let Some(view) = cursor.next(
                    memory,
                    [
                        characters.active(),
                        leaders.active(),
                        units.active() || traits.active() || experience.active(),
                        advisors.active(),
                    ],
                )?
                else {
                    break;
                };
                let (character_pointer, character_id) = view.reference();
                match view.entry() {
                    Ok(character) => {
                        characters.append(
                            |rows, bytes| {
                                ccharacter::walk_entry(&view, base, character, |_| None, rows, bytes)
                                    .into()
                            },
                            |row, offset| {
                                relocate(&mut row.name_start, offset)
                                    && relocate(&mut row.token_start, offset)
                                    && relocate(&mut row.template_file_start, offset)
                            },
                        );
                        leaders.append(
                            |rows, _| {
                                ReadResult::rows(ccountry_leader::walk_entry(
                                    &view, base, character, rows,
                                ))
                            },
                            |_, _| true,
                        );
                        advisors.append(
                            |rows, bytes| {
                                cadvisor::walk_entry(&view, base, character, rows, bytes).into()
                            },
                            |row, offset| {
                                relocate(&mut row.key_start, offset)
                                    && relocate(&mut row.idea_start, offset)
                                    && relocate(&mut row.slot_start, offset)
                            },
                        );
                    }
                    Err(reason) => {
                        characters.fail(reason);
                        leaders.fail(reason);
                        advisors.fail(reason);
                    }
                }
                if !units.active() && !traits.active() && !experience.active() {
                    continue;
                }
                let pointer = character_pointer
                    .checked_add(ccharacter::UNIT_LEADER)
                    .and_then(|address| read::<u64>(&view, address))
                    .and_then(|pointer| usize::try_from(pointer).ok());
                let Some(pointer) = pointer else {
                    units.fail("character unit leader pointer unreadable");
                    traits.fail("character unit leader pointer unreadable");
                    experience.fail("character unit leader pointer unreadable");
                    continue;
                };
                if pointer == 0 {
                    continue;
                }
                let previous_len = units.len;
                units.append(
                    |rows, bytes| read_unit(memory, base, pointer, character_id, rows, bytes),
                    |row, offset| {
                        relocate(&mut row.text_a_start, offset)
                            && relocate(&mut row.text_b_start, offset)
                    },
                );
                // Reuse the authenticated scalar header on the normal path. The
                // trait vectors retain independent availability when scalar capacity
                // or a skill/text fault requires reading only the parent's identity.
                let identity = if traits.active() || experience.active() {
                    Some(if units.len > previous_len {
                        Ok(units.rows[previous_len].native.id)
                    } else {
                        unit_identity(memory, pointer, character_id)
                    })
                } else {
                    None
                };
                match identity {
                    Some(Ok(id)) => {
                        traits.append(
                            |rows, bytes| {
                                cunit_leader::walk_traits(memory, base, pointer, id, rows, bytes)
                                    .into()
                            },
                            |row, offset| relocate(&mut row.token_start, offset),
                        );
                        experience.append(
                            |rows, bytes| {
                                cunit_leader::walk_trait_experience(
                                    memory, base, pointer, id, rows, bytes,
                                )
                                .into()
                            },
                            |row, offset| relocate(&mut row.trait_row.token_start, offset),
                        );
                    }
                    Some(Err(reason)) => {
                        traits.fail(reason);
                        experience.fail(reason);
                    }
                    None => {}
                }
            }
            Ok::<_, &'static str>(())
        })();
        if let Err(reason) = result {
            characters.fail(reason);
            leaders.fail(reason);
            advisors.fail(reason);
            units.fail(reason);
            traits.fail(reason);
            experience.fail(reason);
        }
        [
            characters.result(),
            leaders.result(),
            advisors.result(),
            units.result(),
            traits.result(),
            experience.result(),
        ]
    }

    pub(super) struct WaveMemory<'a> {
        bytes: &'a [u8],
        blocked: Option<usize>,
        changing: Option<(usize, usize)>,
        sequence: Cell<u8>,
        fail_sequence: Option<u8>,
        pub(super) reads: std::cell::RefCell<Vec<(usize, usize)>>,
    }

    impl<'a> WaveMemory<'a> {
        pub(super) fn new(bytes: &'a [u8], blocked: Option<usize>) -> Self {
            Self {
                bytes,
                blocked,
                changing: None,
                sequence: Cell::new(0),
                fail_sequence: None,
                reads: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl Memory for WaveMemory<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, output.len()));
            if self
                .blocked
                .is_some_and(|at| address <= at && at - address < output.len())
            {
                let prefix = output.len().min(3);
                output[..prefix].fill(0xfe);
                return false;
            }
            if !(SliceMemory {
                base: ADDRESS,
                bytes: self.bytes,
            })
            .copy(address, output)
            {
                return false;
            }
            if self.changing == Some((address, output.len())) {
                self.sequence.set(self.sequence.get().wrapping_add(1));
                if self.fail_sequence == Some(self.sequence.get()) {
                    output[0] = 0xfe;
                    return false;
                }
                output[0] = b'A' + self.sequence.get() % 26;
            }
            true
        }
    }

    fn serial_family_capture(memory: &impl Memory, capacities: &Capacities) -> Captured {
        let mut captured = Captured::with_capacities(capacities);
        let results = serial_family_reference(memory, IMAGE, ADDRESS, captured.family());
        captured.characters.primary(results[0]);
        captured.leaders.primary(results[1]);
        captured.advisors.primary(results[2]);
        captured.units.primary(results[3]);
        captured.traits.primary(results[4]);
        captured.trait_experience.primary(results[5]);
        captured
    }

    fn small_wave_job() -> CharactersJob {
        CharactersJob {
            log: crate::raw::RawLog::new(8192, 262_144),
            game_state: 0,
            active: [true; 6],
            traits: TraitCapture::new(),
            advisor_demand: AdvisorDemand::default(),
            skills: [SkillCopy::default(); 32],
            ..Default::default()
        }
    }

    #[test]
    fn character_family_matches_raw_and_typed_at_every_hour() {
        let bytes = fixture();
        let capacities = Capacities {
            rows: [4; N],
            bytes: [128; N],
        };
        let private = [
            ADDRESS + CHARACTERS[0] + ccharacter::LEADER_MAP_HEAD,
            ADDRESS + CHARACTERS[0] + ccharacter::ADVISOR_MAP_HEAD,
            ADDRESS + UNITS[0] + cunit_leader::TRAIT_DATA,
            ADDRESS + UNITS[0] + cunit_leader::TRAIT_EXPERIENCE_DATA,
        ];
        for blocked in [None, Some(private[0]), Some(private[2])] {
            for hour in 0..24 {
                let active = std::array::from_fn(|index| {
                    replay::cadence::should_walk(TABLES[index], hour, false)
                });
                let typed_memory = WaveMemory::new(&bytes, blocked);
                let mut typed = Captured::with_capacities(&capacities);
                typed.capture(&typed_memory, IMAGE, ADDRESS, hour, false);
                let raw_memory = WaveMemory::new(&bytes, blocked);
                let mut job = small_wave_job();
                let report = copy_characters_refresh(
                    &raw_memory,
                    CharactersCopyContext {
                        force_full: false,
                        base: IMAGE,
                        game_state: ADDRESS,
                        active,
                        trait_rows: [capacities.rows[4], capacities.rows[5]],
                        trait_bytes: [capacities.bytes[4], capacities.bytes[5]],
                    },
                    &mut job,
                );
                assert!(report.raw_ok, "hour={hour} blocked={blocked:?}");
                let mut decoded = Captured::with_capacities(&capacities);
                decoded.decode_characters_payload(IMAGE, &job, &mut [0; 6]);
                characters_test_kit::assert_matches_direct(&decoded, &typed);
                assert_eq!(decoded.character_activity(), active);
                // Roster, roles and traits are all hourly (replay::cadence).
                assert_eq!(active, [true; N]);
            }
        }
    }

    fn wave_split_once(memory: &impl Memory, capacities: &Capacities) -> Captured {
        let mut job = small_wave_job();
        copy_pass(
            memory,
            CharactersCopyContext {
                force_full: false,
                base: IMAGE,
                game_state: ADDRESS,
                active: [true; 6],
                trait_rows: [capacities.rows[4], capacities.rows[5]],
                trait_bytes: [capacities.bytes[4], capacities.bytes[5]],
            },
            &mut job,
            None,
        );
        assert!(!job.overflowed());
        let mut captured = Captured::with_capacities(capacities);
        captured.decode_characters_payload(IMAGE, &job, &mut [0; 6]);
        captured
    }

    fn assert_wave_rows(left: &Captured, right: &Captured) {
        characters_test_kit::assert_matches_direct(left, right);
        // Preserve predecessor rows physically written before a later global
        // error, even though that unavailable outcome exposes no row prefix.
        assert_eq!(left.characters.rows.rows, right.characters.rows.rows);
        assert_eq!(left.leaders.rows.rows, right.leaders.rows.rows);
        assert_eq!(left.advisors.rows.rows, right.advisors.rows.rows);
        assert_eq!(left.units.rows.rows, right.units.rows.rows);
        assert_eq!(left.traits.rows.rows, right.traits.rows.rows);
        assert_eq!(
            left.trait_experience.rows.rows,
            right.trait_experience.rows.rows
        );
    }

    fn check_wave_static(bytes: &[u8], blocked: Option<usize>, capacities: &Capacities) {
        let serial_memory = WaveMemory::new(bytes, blocked);
        let current_memory = WaveMemory::new(bytes, blocked);
        let raw_memory = WaveMemory::new(bytes, blocked);
        let split = wave_split_once(&raw_memory, capacities);
        let mut complete = *capacities;
        complete.rows[2] = split.advisors.rows.rows.len();
        complete.bytes[2] = split.advisors.bytes.first.len();
        let serial = serial_family_capture(&serial_memory, &complete);
        let current = characters_test_kit::direct(&current_memory, &complete);
        assert_wave_rows(&current, &serial);
        assert_wave_rows(&split, &current);
    }

    #[test]
    fn character_page_wave_matches_serial_static_rows_capacity_and_every_view_fault() {
        let bytes = characters_test_kit::mapped_fixture();
        let full = Capacities {
            rows: [64; N],
            bytes: [8192; N],
        };
        check_wave_static(&bytes, None, &full);
        for count in [0, 1, 2, 3] {
            for arena in [0, 1, 16, 8192] {
                check_wave_static(
                    &bytes,
                    None,
                    &Capacities {
                        rows: [count; N],
                        bytes: [arena; N],
                    },
                );
            }
        }
        for slot in 0..6 {
            let mut capacity = full;
            capacity.rows[slot] = 0;
            check_wave_static(&bytes, None, &capacity);
        }
        for character in [CHARACTERS[0], CHARACTERS[1], 0x2300] {
            for (offset, width) in VIEW_OFFSETS.into_iter().zip(VIEW_LENS) {
                for fault in [
                    ADDRESS + character + offset,
                    ADDRESS + character + offset + width - 1,
                ] {
                    check_wave_static(&bytes, Some(fault), &full);
                }
            }
        }
        // Include a complete 32-entry page with nonempty independent domains.
        check_wave_static(
            &cost_fixture(),
            None,
            &Capacities {
                rows: [128; N],
                bytes: [131_072; N],
            },
        );
        for count in [0, 33, 65] {
            let mut bytes = fixture();
            array(
                &mut bytes,
                MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
                0x8400,
                count,
            );
            array(
                &mut bytes,
                MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS,
                0x8800,
                1,
            );
            pointer(&mut bytes, 0x8800, CHARACTERS[0]);
            for index in 0..count {
                pointer(&mut bytes, 0x8400 + index as usize * 8, CHARACTERS[0]);
            }
            check_wave_static(&bytes, None, &full);
            if count > 32 {
                let page_count = (count - 32).min(32) as usize;
                for fault in [
                    ADDRESS + 0x8400 + 32 * 8,
                    ADDRESS + 0x8400 + (32 + page_count) * 8 - 1,
                ] {
                    check_wave_static(&bytes, Some(fault), &full);
                }
                for index in [32, 32 + page_count - 1] {
                    let mut null = bytes.clone();
                    put(&mut null, 0x8400 + index * 8, 0_u64.to_le_bytes());
                    check_wave_static(&null, None, &full);
                    let mut invalid = bytes.clone();
                    pointer(&mut invalid, 0x8400 + index * 8, CHARACTERS[1]);
                    put(&mut invalid, CHARACTERS[1], 0_u64.to_le_bytes());
                    check_wave_static(&invalid, None, &full);
                }
            }
        }
    }

    #[test]
    fn character_page_wave_defers_global_errors_and_never_revives_failed_domains() {
        let full = Capacities {
            rows: [64; N],
            bytes: [8192; N],
        };
        for later in [1, 2] {
            for null in [false, true] {
                let mut bytes = characters_test_kit::mapped_fixture();
                // A's body fails before the later global failure is delivered.
                put(
                    &mut bytes,
                    CHARACTERS[0] + ccharacter::NAME + 24,
                    14_u64.to_le_bytes(),
                );
                if null {
                    put(&mut bytes, ROSTER + later * 8, 0_u64.to_le_bytes());
                } else {
                    let target = [CHARACTERS[0], CHARACTERS[1], 0x2300][later];
                    put(&mut bytes, target, 0_u64.to_le_bytes());
                }
                check_wave_static(&bytes, None, &full);
                let result = characters_test_kit::direct(&WaveMemory::new(&bytes, None), &full);
                assert_eq!(
                    result.characters.rows.outcome,
                    Outcome::Unavailable {
                        reason: "character string header invalid"
                    }
                );
            }
        }
        let bytes = characters_test_kit::mapped_fixture();
        let trace = WaveMemory::new(&bytes, None);
        let _ = characters_test_kit::direct(&trace, &full);
        // Earlier child failures are allowed to leave later staged Views
        // untouched in storage, but may not alter later active-domain results.
        for &(address, width) in trace.reads.borrow().iter() {
            if address >= ADDRESS + 0x2400 {
                for fault in [address, address + width - 1] {
                    check_wave_static(&bytes, Some(fault), &full);
                }
            }
        }
    }

    #[test]
    fn character_page_wave_preserves_alias_cursors_and_current_values() {
        let full = Capacities {
            rows: [64; N],
            bytes: [8192; N],
        };
        let mut bytes = fixture();
        let alias = ADDRESS + CHARACTERS[1] + ccharacter::NAME;
        // A's heap payload is B's complete names View key. The trailing
        // terminator is B's zero/unrecruited country low byte.
        pointer(
            &mut bytes,
            CHARACTERS[0] + ccharacter::NAME,
            CHARACTERS[1] + ccharacter::NAME,
        );
        put(
            &mut bytes,
            CHARACTERS[0] + ccharacter::NAME + 16,
            64_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            CHARACTERS[0] + ccharacter::NAME + 24,
            64_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            CHARACTERS[1] + ccharacter::COUNTRY,
            0_u32.to_le_bytes(),
        );
        let mut native = WaveMemory::new(&bytes, None);
        native.changing = Some((alias, 64));
        let mut raw = WaveMemory::new(&bytes, None);
        raw.changing = Some((alias, 64));
        let direct = characters_test_kit::direct(&native, &full);
        let split = wave_split_once(&raw, &full);
        assert_wave_rows(&split, &direct);
        assert_eq!(native.sequence.get(), 2);
        assert_eq!(raw.sequence.get(), 2);
        assert_eq!(direct.characters.rows.outcome, Outcome::Rows { len: 2 });
        let rows = &direct.characters.rows.rows;
        assert_eq!(direct.characters.bytes.first[rows[0].name_start], b'C');
        assert_eq!(direct.characters.bytes.first[rows[1].name_start], b'B');

        // This mutation deliberately distinguishes schedules: old serial
        // observes A's aliased payload before B's View, hence B then C.
        // Equality is required between the new native/raw wave, not between
        // different instants of an arbitrarily changing source.
        let mut serial_memory = WaveMemory::new(&bytes, None);
        serial_memory.changing = Some((alias, 64));
        let serial = serial_family_capture(&serial_memory, &full);
        assert_eq!(
            serial.characters.bytes.first[serial.characters.rows.rows[0].name_start],
            b'B'
        );
        assert_eq!(
            serial.characters.bytes.first[serial.characters.rows.rows[1].name_start],
            b'C'
        );
        for failure in [1, 2] {
            let mut native = WaveMemory::new(&bytes, None);
            native.changing = Some((alias, 64));
            native.fail_sequence = Some(failure);
            let mut raw = WaveMemory::new(&bytes, None);
            raw.changing = Some((alias, 64));
            raw.fail_sequence = Some(failure);
            let direct = characters_test_kit::direct(&native, &full);
            assert_wave_rows(&wave_split_once(&raw, &full), &direct);
            assert_eq!((native.sequence.get(), raw.sequence.get()), (2, 2));
        }

        // A duplicate roster occurrence is a fresh View, not a deduplicated
        // object. Keep cross-roster occurrences independent as well.
        let mut bytes = fixture();
        pointer(&mut bytes, ROSTER + 8, CHARACTERS[0]);
        array(
            &mut bytes,
            MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS,
            0x2080,
            1,
        );
        pointer(&mut bytes, 0x2080, CHARACTERS[0]);
        let key = (ADDRESS + CHARACTERS[0] + ccharacter::NAME, 64);
        let mut native = WaveMemory::new(&bytes, None);
        native.changing = Some(key);
        let mut raw = WaveMemory::new(&bytes, None);
        raw.changing = Some(key);
        let direct = characters_test_kit::direct(&native, &full);
        assert_wave_rows(&wave_split_once(&raw, &full), &direct);
        assert_eq!((native.sequence.get(), raw.sequence.get()), (3, 3));
        assert_eq!(direct.characters.rows.rows[2].roster, 1);
        assert_eq!(direct.characters.rows.rows[2].ordinal, 0);

        // Recycled raw storage and rebuilt pages expose current bytes only.
        let mut job = small_wave_job();
        let mut decoded = Captured::with_capacities(&full);
        for value in [b'x', b'y', b'z'] {
            bytes[CHARACTERS[0] + ccharacter::NAME] = value;
            copy_pass(
                &WaveMemory::new(&bytes, None),
                CharactersCopyContext {
                    force_full: false,
                    base: IMAGE,
                    game_state: ADDRESS,
                    active: [true; 6],
                    trait_rows: [full.rows[4], full.rows[5]],
                    trait_bytes: [full.bytes[4], full.bytes[5]],
                },
                &mut job,
                None,
            );
            assert!(!job.log.overflowed);
            decoded.decode_characters_payload(IMAGE, &job, &mut [0; 6]);
            let current = characters_test_kit::direct(&WaveMemory::new(&bytes, None), &full);
            assert_wave_rows(&decoded, &current);
        }
    }

    #[test]
    fn character_page_wave_keeps_staging_overflow_unaccepted() {
        let bytes = characters_test_kit::mapped_fixture();
        let capacities = Capacities {
            rows: [64; N],
            bytes: [8192; N],
        };
        for (spans, bytes_capacity) in [(0, 0), (8, 1024), (20, 1024), (8192, 128), (8192, 512)] {
            let mut job = CharactersJob {
                log: crate::raw::RawLog::new(spans, bytes_capacity),
                game_state: 0,
                active: [true; 6],
                traits: TraitCapture::new(),
                advisor_demand: AdvisorDemand::default(),
                skills: [SkillCopy::default(); 32],
                ..Default::default()
            };
            let report = copy_characters_refresh(
                &WaveMemory::new(&bytes, None),
                CharactersCopyContext {
                    force_full: false,
                    base: IMAGE,
                    game_state: ADDRESS,
                    active: [true; 6],
                    trait_rows: [capacities.rows[4], capacities.rows[5]],
                    trait_bytes: [capacities.bytes[4], capacities.bytes[5]],
                },
                &mut job,
            );
            assert!(!report.raw_ok, "spans {spans}, bytes {bytes_capacity}");
            assert!(job.log.overflowed);
            assert!(job.grow_for_next());
        }
    }
}

#[cfg(test)]
pub(crate) fn character_shadow_fixture(value: u8) -> Captured {
    let capacities = Capacities {
        rows: [2, 2, 2, 1, 2, 1],
        bytes: [8, 0, 8, 0, 8, 0],
    };
    let mut captured = Captured::with_capacities(&capacities);
    for index in 0..2 {
        captured.characters.rows.rows[index] = ccharacter::Row {
            id: (index + 3) as u32,
            country: 7,
            name_token_id: -9,
            roster: index as u8,
            ordinal: index as u32,
            name_start: index * 3,
            name_len: 2,
            token_start: index * 3 + 2,
            token_len: 1,
            ..Default::default()
        };
        captured.leaders.rows.rows[index] = ccountry_leader::Row {
            character: (index + 3) as u32,
            country: u32::from(value),
            ideology: -7,
        };
        captured.advisors.rows.rows[index] = cadvisor::Row {
            character: (index + 3) as u32,
            country: 7,
            ordinal: 0,
            key_start: index * 3,
            key_len: 1,
            idea_start: index * 3 + 1,
            idea_len: 1,
            slot_start: index * 3 + 2,
            slot_len: 1,
        };
        captured.traits.rows.rows[index] = cunit_leader::TraitRow {
            leader_id: (index + 3) as u32,
            ordinal: 0,
            token_id: -5,
            definition_ordinal: -1,
            token_start: index * 3,
            token_len: 3,
        };
    }
    for bytes in [
        &mut captured.characters.bytes,
        &mut captured.advisors.bytes,
        &mut captured.traits.bytes,
    ] {
        bytes.first[..6].copy_from_slice(&[value, 0, 0xff, b'b', b'c', b'd']);
        bytes.len = 6;
        bytes.required = 6;
    }
    captured.characters.rows.outcome = Outcome::Rows { len: 2 };
    captured.leaders.rows.outcome = Outcome::Rows { len: 2 };
    captured.advisors.rows.outcome = Outcome::Rows { len: 2 };
    captured.traits.rows.outcome = Outcome::Rows { len: 2 };
    captured.units.rows.outcome = Outcome::Rows { len: 0 };
    captured.trait_experience.rows.outcome = Outcome::Rows { len: 0 };
    captured
}

#[cfg(test)]
mod shadow_tests {
    use super::*;

    const LOCAL: [usize; 4] = [0, 1, 2, 4];

    fn inputs(
        current: &Captured,
        shadow: &mut CharacterShadow,
        cached: Option<&[Arc<TableRows>; 4]>,
        branch: BranchId,
    ) -> [TableInput; N] {
        let cached = std::array::from_fn(|index| cached.map(|rows| &rows[index]));
        current.table_inputs(Some(&mut CharacterConversion::new(branch, shadow, cached)))
    }

    fn shared(input: &[TableInput; N]) -> [Arc<TableRows>; 4] {
        LOCAL.map(|index| match &input[index] {
            TableInput::SharedComplete(rows) => Arc::clone(rows),
            _ => panic!("expected complete shared table at {index}"),
        })
    }

    fn assert_canonical(current: &Captured, input: &[TableInput; N]) {
        for (expected, actual) in current.tables().iter().zip(input) {
            match actual {
                TableInput::Sampled(actual) => assert_eq!(actual, expected),
                TableInput::SharedComplete(actual) => {
                    assert_eq!(&TableView::Complete((**actual).clone()), expected);
                }
                TableInput::ComparedEqual(_) | TableInput::ComparedChanges { .. } => {
                    panic!("character reuse cannot bypass validation")
                }
            }
        }
    }

    #[test]
    fn character_shadow_exact_hits_keep_current_arcs_and_ignore_unused_pool_tails() {
        let mut shadow = CharacterShadow::default();
        let first = character_shadow_fixture(9);
        let cached = shared(&inputs(&first, &mut shadow, None, 1));
        let pointers = (
            shadow.characters.rows.as_ptr(),
            shadow.characters.bytes.as_ptr(),
        );
        let mut current = character_shadow_fixture(9);
        current
            .characters
            .rows
            .rows
            .push(ccharacter::Row::default());
        current.characters.bytes.first[7] = 42;
        let hit = inputs(&current, &mut shadow, Some(&cached), 1);
        assert_canonical(&current, &hit);
        for (previous, current) in cached.iter().zip(shared(&hit)) {
            assert!(Arc::ptr_eq(previous, &current));
        }
        assert_eq!(
            pointers,
            (
                shadow.characters.rows.as_ptr(),
                shadow.characters.bytes.as_ptr()
            )
        );
        for stats in shadow.statistics() {
            assert_eq!((stats.hits, stats.misses), (1, 1));
        }
        assert_eq!(shadow.characters.rows.len(), 2);
        assert_eq!(shadow.characters.bytes, [9, 0, 0xff, b'b', b'c', b'd']);
        assert!(matches!(
            shadow_input(
                TableId::Characters,
                &current.characters,
                &mut shadow.characters,
                Some(&cached[0]),
                |_, _| panic!("an exact validated hit must not rescan row ranges"),
                || panic!("an exact validated hit must not convert"),
            ),
            TableInput::SharedComplete(_)
        ));
    }

    #[test]
    fn character_shadow_every_consumed_field_and_byte_change_misses_independently() {
        type Change = (usize, fn(&mut Captured));
        let changes: &[Change] = &[
            (0, |c| c.characters.rows.rows[0].id += 20),
            (0, |c| c.characters.rows.rows[0].country += 1),
            (0, |c| c.characters.rows.rows[0].name_token_id -= 1),
            (0, |c| c.characters.rows.rows[0].roster += 1),
            (0, |c| c.characters.rows.rows[0].ordinal += 1),
            (0, |c| c.characters.rows.rows[0].name_start += 1),
            (0, |c| c.characters.rows.rows[0].name_len -= 1),
            (0, |c| c.characters.rows.rows[0].token_start += 1),
            (0, |c| c.characters.rows.rows[0].token_len -= 1),
            (0, |c| c.characters.bytes.first[0] ^= 1),
            (0, |c| c.characters.bytes.first[2] ^= 1),
            (0, |c| c.characters.bytes.required += 1),
            (1, |c| c.leaders.rows.rows[0].character += 20),
            (1, |c| c.leaders.rows.rows[0].country += 1),
            (1, |c| c.leaders.rows.rows[0].ideology -= 1),
            (2, |c| c.advisors.rows.rows[0].character += 20),
            (2, |c| c.advisors.rows.rows[0].country += 1),
            (2, |c| c.advisors.rows.rows[0].ordinal += 1),
            (2, |c| c.advisors.rows.rows[0].key_start += 1),
            (2, |c| c.advisors.rows.rows[0].key_len -= 1),
            (2, |c| c.advisors.rows.rows[0].idea_start += 1),
            (2, |c| c.advisors.rows.rows[0].idea_len -= 1),
            (2, |c| c.advisors.rows.rows[0].slot_start += 1),
            (2, |c| c.advisors.rows.rows[0].slot_len -= 1),
            (2, |c| c.advisors.bytes.first[0] ^= 1),
            (2, |c| c.advisors.bytes.first[1] ^= 1),
            (2, |c| c.advisors.bytes.first[2] ^= 1),
            (2, |c| c.advisors.bytes.required += 1),
            (3, |c| c.traits.rows.rows[0].leader_id += 20),
            (3, |c| c.traits.rows.rows[0].ordinal += 1),
            (3, |c| c.traits.rows.rows[0].token_id -= 1),
            (3, |c| c.traits.rows.rows[0].definition_ordinal -= 1),
            (3, |c| c.traits.rows.rows[0].token_start += 1),
            (3, |c| c.traits.rows.rows[0].token_len -= 1),
            (3, |c| c.traits.bytes.first[0] ^= 1),
            (3, |c| c.traits.bytes.required += 1),
            (0, |c| c.characters.rows.outcome = Outcome::Rows { len: 1 }),
            (1, |c| c.leaders.rows.outcome = Outcome::Rows { len: 1 }),
            (2, |c| c.advisors.rows.outcome = Outcome::Rows { len: 1 }),
            (3, |c| c.traits.rows.outcome = Outcome::Rows { len: 1 }),
            (0, |c| c.characters.rows.rows.swap(0, 1)),
            (1, |c| c.leaders.rows.rows.swap(0, 1)),
            (2, |c| c.advisors.rows.rows.swap(0, 1)),
            (3, |c| c.traits.rows.rows.swap(0, 1)),
        ];
        for &(changed, change) in changes {
            let first = character_shadow_fixture(9);
            let mut shadow = CharacterShadow::default();
            let cached = shared(&inputs(&first, &mut shadow, None, 1));
            let mut current = character_shadow_fixture(9);
            change(&mut current);
            let result = inputs(&current, &mut shadow, Some(&cached), 1);
            assert_canonical(&current, &result);
            for (index, output) in shared(&result).iter().enumerate() {
                assert_eq!(Arc::ptr_eq(&cached[index], output), index != changed);
            }
        }
    }

    fn fault<R: Clone>(buffer: &mut Buffer<R>, fault: usize) {
        match fault {
            0 => {
                buffer.rows.outcome = Outcome::Unavailable {
                    reason: "injected native failure",
                }
            }
            1 => {
                buffer.rows.outcome = Outcome::Truncated {
                    len: 1,
                    unread_rows: 1,
                }
            }
            2 => {
                buffer.rows.outcome = Outcome::Truncated {
                    len: 1,
                    unread_rows: 0,
                }
            }
            3 => buffer.rows.outcome = Outcome::Rows { len: usize::MAX },
            5 => buffer.bytes.len = usize::MAX,
            6 => buffer.rows.rows[1] = buffer.rows.rows[0].clone(),
            _ => unreachable!(),
        }
    }

    #[test]
    fn character_shadow_faults_clear_only_current_table_and_recover_via_conversion() {
        for selected in 0..4 {
            for failure in [0, 1, 2, 3, 5, 6] {
                let first = character_shadow_fixture(9);
                let mut shadow = CharacterShadow::default();
                let cached = shared(&inputs(&first, &mut shadow, None, 1));
                let mut current = character_shadow_fixture(9);
                match selected {
                    0 => fault(&mut current.characters, failure),
                    1 => fault(&mut current.leaders, failure),
                    2 => fault(&mut current.advisors, failure),
                    3 => fault(&mut current.traits, failure),
                    _ => unreachable!(),
                }
                let result = inputs(&current, &mut shadow, Some(&cached), 1);
                assert_canonical(&current, &result);
                assert!(matches!(&result[LOCAL[selected]], TableInput::Sampled(_)));
                let restored = shared(&inputs(&first, &mut shadow, Some(&cached), 1));
                for index in 0..4 {
                    assert_eq!(
                        Arc::ptr_eq(&cached[index], &restored[index]),
                        index != selected
                    );
                }
            }
        }
        for selected in [0, 2, 3] {
            for overflow in [false, true] {
                let mut shadow = CharacterShadow::default();
                let first = character_shadow_fixture(9);
                let cached = shared(&inputs(&first, &mut shadow, None, 1));
                let mut current = character_shadow_fixture(9);
                let start = if overflow { usize::MAX } else { 7 };
                match selected {
                    0 => current.characters.rows.rows[0].name_start = start,
                    2 => current.advisors.rows.rows[0].idea_start = start,
                    3 => current.traits.rows.rows[0].token_start = start,
                    _ => unreachable!(),
                }
                let result = inputs(&current, &mut shadow, Some(&cached), 1);
                assert_canonical(&current, &result);
                assert!(matches!(
                    &result[LOCAL[selected]],
                    TableInput::Sampled(TableView::Unavailable { .. })
                ));
            }
        }
    }

    #[test]
    fn character_shadow_offhours_branch_and_allocation_identity_are_not_inferred_from_values() {
        let first = character_shadow_fixture(9);
        let mut shadow = CharacterShadow::default();
        let cached = shared(&inputs(&first, &mut shadow, None, 1));
        let stats = shadow.statistics();
        let mut skipped = character_shadow_fixture(99);
        for outcome in [
            &mut skipped.characters.rows.outcome,
            &mut skipped.leaders.rows.outcome,
            &mut skipped.advisors.rows.outcome,
            &mut skipped.traits.rows.outcome,
        ] {
            *outcome = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
        }
        assert_canonical(&skipped, &inputs(&skipped, &mut shadow, Some(&cached), 1));
        assert_eq!(shadow.statistics(), stats);
        let hit = shared(&inputs(&first, &mut shadow, Some(&cached), 1));
        assert!(cached.iter().zip(&hit).all(|(a, b)| Arc::ptr_eq(a, b)));
        let replaced = cached.each_ref().map(|rows| Arc::new((**rows).clone()));
        let fresh = shared(&inputs(&first, &mut shadow, Some(&replaced), 1));
        assert!(replaced.iter().zip(&fresh).all(|(a, b)| !Arc::ptr_eq(a, b)));
        let branch = shared(&inputs(&first, &mut shadow, Some(&fresh), 2));
        assert!(fresh.iter().zip(&branch).all(|(a, b)| !Arc::ptr_eq(a, b)));
        let missing = shared(&inputs(&first, &mut shadow, None, 2));
        assert!(branch.iter().zip(&missing).all(|(a, b)| !Arc::ptr_eq(a, b)));
        // Wrong-table cache references cannot satisfy even an equal native shadow.
        let wrong = std::array::from_fn(|_| Arc::clone(&missing[1]));
        let wrong_result = inputs(&first, &mut shadow, Some(&wrong), 2);
        assert_canonical(&first, &wrong_result);
        assert!(!Arc::ptr_eq(&shared(&wrong_result)[0], &wrong[0]));
    }

    #[test]
    fn character_shadow_empty_rows_strings_and_unselected_dynamic_rows_remain_exact() {
        let mut first = character_shadow_fixture(9);
        for row in &mut first.characters.rows.rows {
            row.name_len = 0;
            row.token_len = 0;
        }
        let mut shadow = CharacterShadow::default();
        let cached = shared(&inputs(&first, &mut shadow, None, 1));
        let mut dynamic = character_shadow_fixture(9);
        dynamic
            .characters
            .rows
            .rows
            .clone_from(&first.characters.rows.rows);
        dynamic.units.rows.outcome = Outcome::Rows { len: 1 };
        dynamic.units.rows.rows[0].native.experience = i64::MIN;
        dynamic.trait_experience.rows.outcome = Outcome::Rows { len: 1 };
        dynamic.trait_experience.rows.rows[0].experience = i64::MAX;
        let result = inputs(&dynamic, &mut shadow, Some(&cached), 1);
        assert_canonical(&dynamic, &result);
        assert!(
            cached
                .iter()
                .zip(shared(&result))
                .all(|(a, b)| Arc::ptr_eq(a, &b))
        );
        for index in [3, 5] {
            assert!(matches!(
                &result[index],
                TableInput::Sampled(TableView::Complete(_))
            ));
        }
        first.characters.rows.outcome = Outcome::Rows { len: 0 };
        first.leaders.rows.outcome = Outcome::Rows { len: 0 };
        first.advisors.rows.outcome = Outcome::Rows { len: 0 };
        first.traits.rows.outcome = Outcome::Rows { len: 0 };
        for bytes in [
            &mut first.characters.bytes,
            &mut first.advisors.bytes,
            &mut first.traits.bytes,
        ] {
            bytes.len = 0;
            bytes.required = 0;
        }
        let empty = shared(&inputs(&first, &mut shadow, Some(&cached), 1));
        let hit = inputs(&first, &mut shadow, Some(&empty), 1);
        assert_canonical(&first, &hit);
        assert!(
            empty
                .iter()
                .zip(shared(&hit))
                .all(|(a, b)| Arc::ptr_eq(a, &b))
        );
    }

    #[test]
    fn character_shadow_capacity_and_counters_survive_invalidation_truthfully() {
        let first = character_shadow_fixture(9);
        let mut shadow = CharacterShadow::default();
        let cached = shared(&inputs(&first, &mut shadow, None, 1));
        shadow.characters.rows.reserve(20);
        shadow.characters.bytes.reserve(31);
        let stats = shadow.statistics();
        assert_eq!(
            stats[0].row_capacity_bytes,
            shadow.characters.rows.capacity() * size_of::<ccharacter::Row>()
        );
        assert_eq!(
            stats[0].arena_capacity_bytes,
            shadow.characters.bytes.capacity()
        );
        assert_eq!(stats[1].arena_capacity_bytes, 0);
        let _branch = inputs(&first, &mut shadow, None, 2);
        for (before, after) in stats.into_iter().zip(shadow.statistics()) {
            assert_eq!(after.hits, 0);
            assert_eq!(after.misses, 2);
            assert_eq!(before.row_capacity_bytes, after.row_capacity_bytes);
            assert_eq!(before.arena_capacity_bytes, after.arena_capacity_bytes);
        }
        drop(cached);
        assert!(shadow.characters.output.upgrade().is_some());
    }
}
