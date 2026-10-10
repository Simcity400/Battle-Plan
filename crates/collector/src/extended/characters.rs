//! Persistent character state. Native walks use pooled rows and literal-byte
//! arenas; sorting, duplicate checks and owned replay strings run off-thread.

use engine::allocation::{copy_vec, owned_vec, reserve, reserve_exact};
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
    ccharacter_template, ccountry, ccountry_leader, cgame_state, character_advisor_node,
    character_leader_node, cnavy_leader, coperative_leader, cpdx_array::Array, cunit_leader,
    cunit_leader_skill, cunit_leader_trait, msvc_string, msvc_tree_node, read,
    unit_leader_trait_experience_entry, walk::Outcome,
};
use replay::{
    BranchId,
    frame::{TableInput, TableView},
    rows::{
        ActiveAbilityRow, AdvisorRoleRow, CharacterRow, CountryLeaderRoleRow, TableId, TableRows,
        TableSchema, UnitLeaderRow, UnitLeaderTraitExperienceRow, UnitLeaderTraitRow, tables,
    },
};

use std::mem::size_of;
use std::sync::{Arc, Weak};

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub bytes: [usize; N],
}

impl Capacities {
    /// Off-thread live unit/trait demand, independent of country count.
    pub(crate) fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        demand::presize(self, memory, base, state)
    }

    /// Characters keeps one row per roster occurrence and both names and the
    /// template file of each (`ccharacter::demand`), so the live roster sizes
    /// the first full hour rather than a vanilla count.
    pub(crate) fn raise_for_characters(&mut self, demand: ccharacter::Demand) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[0], rows(demand.rows, 1));
        raise(&mut self.bytes[0], rows(demand.bytes, 1));
    }
}

mod demand;
pub(crate) use demand::status as demand_status;

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
    codename: Option<engine::cname_group_member::Inputs>,
    codename_reason: Option<&'static str>,
    formation_reference: Option<u64>,
    formation_reason: Option<&'static str>,
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
    fallback_jobs: [Option<Box<CharactersJob>>; PARTS],
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

static CURRENT_IDEOLOGY_FAILURES: std::sync::Mutex<Vec<(&'static str, u64)>> =
    std::sync::Mutex::new(Vec::new());
static FORMATION_REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn leader_ideology_status() -> impl serde::Serialize {
    let failures = owned_vec(copy_vec(
        &CURRENT_IDEOLOGY_FAILURES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    ));
    crate::status::status!({
        "refused": failures.iter().fold(0_u64, |sum, (_, count)| sum.saturating_add(*count)),
        "refusals": crate::status::map(failures.into_iter()),
        "scope": "country-leader current-ideology conversion failures; unavailable appended values retain the existing role row",
    })
}

fn record_leader_ideologies(rows: &[ccountry_leader::Row]) {
    let mut failures = CURRENT_IDEOLOGY_FAILURES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for reason in rows.iter().filter_map(|row| row.current_ideology_failure) {
        let index = match failures.binary_search_by_key(&reason, |entry| entry.0) {
            Ok(index) => index,
            Err(index) => {
                if engine::allocation::reserve(&mut failures, 1).is_err() {
                    return;
                }
                failures.insert(index, (reason, 0));
                index
            }
        };
        failures[index].1 = failures[index].1.saturating_add(1);
    }
}

pub(crate) fn acquisition_diagnostics() -> impl serde::Serialize {
    use std::sync::atomic::Ordering::Relaxed;
    let parts = std::array::from_fn::<_, PARTS, _>(|part| {
        let calls = COPY_CALLS[part].load(Relaxed);
        let total = COPY_NS[part].load(Relaxed);
        let backend = BACKEND_NS[part].load(Relaxed);
        crate::status::status!({
            "name": READ_GROUPS[part].0, "calls": calls, "total_ns": total,
            "backend_ns": backend, "mean_backend_ns": backend.checked_div(calls).unwrap_or(0),
            "mean_other_ns": total.saturating_sub(backend).checked_div(calls).unwrap_or(0),
        })
    });
    let sections = std::array::from_fn::<_, 2, _>(|section| {
        let calls = ADVISOR_SECTION_CALLS[section].load(Relaxed);
        let ns = ADVISOR_SECTION_NS[section].load(Relaxed);
        crate::status::status!({
            "roster": section, "calls": calls,
            "complete_prepared": ADVISOR_SECTION_PLANNED[section].load(Relaxed),
            "total_ns": ns, "mean_ns": ns.checked_div(calls).unwrap_or(0),
        })
    });
    crate::status::status!({
        "scope": "cumulative active character acquisitions; backend is fresh prepared execution only; other includes discovery/continuation, trait acquisition and metadata restoration; overlapping jobs must not be summed as callback wall",
        "parts": crate::status::sequence(parts.into_iter()),
        "advisor_sections": crate::status::sequence(sections.into_iter()),
        "advisor_modifiers": advisors::modifier_status(),
        "country_leader_ideologies": leader_ideology_status(),
        "unit_leader_formation_refusals": FORMATION_REFUSALS.load(Relaxed),
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

mod shadow;
pub(crate) use shadow::*;

impl Captured {
    pub(crate) fn observe_naming(
        &self,
        work: &mut super::character_inputs::NamingWork,
    ) -> std::io::Result<()> {
        for row in &self.units.rows.rows[..self.units.rows.outcome.len()] {
            if row.formation_reason.is_some() {
                FORMATION_REFUSALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            if let Some(reason) = row.codename_reason {
                work.refuse(reason)?;
            }
        }
        Ok(())
    }

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
            fallback_jobs: std::array::from_fn(|_| None),
        }
    }

    /// Allocation-free placeholder while the final-input worker owns storage.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: [0; N],
            bytes: [0; N],
        })
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
        for (index, row_len, byte_len) in [
            (4, self.traits.rows.outcome.len(), self.traits.bytes.len),
            (
                5,
                self.trait_experience.rows.outcome.len(),
                self.trait_experience.bytes.len,
            ),
        ] {
            capacities.rows[index] = capacities.rows[index].max(row_len);
            capacities.bytes[index] = capacities.bytes[index].max(byte_len);
        }
        self.characters.grow(capacities, 0);
        self.leaders.grow(capacities, 1);
        capacities.rows[2] = capacities.rows[2].max(self.advisors.rows.outcome.len());
        capacities.bytes[2] = capacities.bytes[2].max(self.advisors.bytes.len);
        self.advisors.grow(capacities, 2);
        self.units.grow(capacities, 3);
        self.traits.grow(capacities, 4);
        self.trait_experience.grow(capacities, 5);
    }

    fn begin_capture(&mut self, hour: i32, force_full: bool) -> [bool; N] {
        self.reset();
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
        std::array::from_fn(|index| family_active[index] && !self.raw_active[index])
    }

    /// The scheduled persistent tables share one current roster.
    #[cfg(test)]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        let typed_active = self.begin_capture(hour, force_full);
        let mut ticks = [0; N];
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

    /// Worker fallback uses the same original-observation acquisition and
    /// owned decode as the role jobs. Only tables without a raw owner run.
    pub(crate) fn capture_growing(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        let typed_active = self.begin_capture(hour, force_full);
        let mut ticks = [0; N];
        for (part, mask) in PART_MASKS.into_iter().enumerate() {
            let active = std::array::from_fn(|index| typed_active[index] && mask[index]);
            if !active.iter().any(|&active| active) {
                continue;
            }
            let mut job = self.fallback_jobs[part]
                .take()
                .unwrap_or_else(|| Box::new(CharactersJob::new_part(part)));
            let (trait_rows, trait_bytes) = self.trait_capture_capacities();
            let context = CharactersCopyContext {
                base,
                game_state,
                force_full,
                active,
                trait_rows,
                trait_bytes,
            };
            let _reads = read_accounting::table(TABLES[anchor(active)]);
            let begin = qpc();
            let report = copy_characters_growing(memory, context, &mut job, Roles::Independent);
            if report.raw_ok {
                self.decode_characters_payload(base, &job, &mut [0; N]);
                self.remember_templates(&mut job);
            } else {
                self.fail_character_tables(active, "character worker acquisition incomplete");
            }
            ticks[anchor(active)] = elapsed(begin);
            job.prepare_read_plans();
            self.fallback_jobs[part] = Some(job);
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
            advisor_ranges_valid,
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
        view::<tables::Characters, _>(&self.characters, character_ranges_valid, |row, bytes| {
            CharacterRow {
                id: row.id,
                country: row.country,
                name_token_id: row.name_token_id,
                gender: row.gender,
                roster: row.roster,
                ordinal: row.ordinal,
                name: owned(bytes, row.name_start, row.name_len),
                token: owned(bytes, row.token_start, row.token_len),
                template_present: u8::from(row.template != 0),
                template_tag: row.template_identity.tag,
                template_file: owned(bytes, row.template_file_start, row.template_file_len),
                template_line: row.template_identity.line,
                template_generated: row.template_identity.generated,
            }
        })
    }

    fn leaders_table(&self) -> TableView {
        if let Some(rows) = self
            .leaders
            .rows
            .rows
            .get(..self.leaders.rows.outcome.len())
        {
            record_leader_ideologies(rows);
        }
        view::<tables::CountryLeaderRoles, _>(
            &self.leaders,
            |_, _| true,
            |row, _| CountryLeaderRoleRow {
                character: row.character,
                country: row.country,
                ideology: row.ideology,
                current_ideology: replay::rows::Recorded::Value(row.current_ideology),
            },
        )
    }

    fn advisors_table(&self) -> TableView {
        view::<tables::AdvisorRoles, _>(&self.advisors, advisor_ranges_valid, |row, bytes| {
            AdvisorRoleRow {
                character: row.character,
                country: row.country,
                ordinal: row.ordinal,
                key: owned(bytes, row.key_start, row.key_len),
                idea: owned(bytes, row.idea_start, row.idea_len),
                slot: owned(bytes, row.slot_start, row.slot_len),
                modifier_cache: replay::rows::Recorded::Value(row.modifier_recorded.then(|| {
                    let entries = bytes[row.modifier_start..row.modifier_start + row.modifier_len]
                        .chunks_exact(engine::modifier_entry::STRIDE);
                    let mut values = owned_vec(engine::allocation::vec(entries.len()));
                    for entry in entries {
                        values.push(replay::rows::CachedModifier {
                            definition: u32::from_le_bytes(entry[..4].try_into().unwrap()),
                            value: i64::from_le_bytes(entry[8..16].try_into().unwrap()),
                        });
                    }
                    replay::rows::AdvisorModifierCache { entries: values }
                })),
                tooltip_keys: replay::rows::Recorded::Value(row.tooltip_recorded.then(|| {
                    let mut source = &bytes[row.tooltip_start..row.tooltip_start + row.tooltip_len];
                    let mut remaining = source;
                    let mut count = 0;
                    while !remaining.is_empty() {
                        let len = u64::from_le_bytes(remaining[..8].try_into().unwrap()) as usize;
                        remaining = &remaining[8 + len..];
                        count += 1;
                    }
                    let mut keys = owned_vec(engine::allocation::vec(count));
                    while !source.is_empty() {
                        let len = u64::from_le_bytes(source[..8].try_into().unwrap()) as usize;
                        source = &source[8..];
                        keys.push(owned_vec(engine::allocation::copy_vec(&source[..len])));
                        source = &source[len..];
                    }
                    keys
                })),
            }
        })
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
                        && row.codename.is_none_or(|name| {
                            valid(bytes, name.custom.0, name.custom.1)
                                && valid(bytes, name.group.0, name.group.1)
                        })
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
        let index = self
            .entries
            .binary_search_by_key(&id, |entry| entry.id)
            .ok()?;
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
            let mut present: Vec<&ccharacter::Row> = Vec::new();
            if reserve_exact(&mut present, rows.len()).is_err() {
                return;
            }
            present.extend(
                rows.iter()
                    .filter(|row| row.template != 0 && character_ranges_valid(row, bytes)),
            );
            // Preserve the original first occurrence when identities repeat.
            present.sort_unstable_by_key(|row| (row.id, *row as *const ccharacter::Row as usize));
            let Some(file_bytes) = present
                .iter()
                .try_fold(0usize, |sum, row| sum.checked_add(row.template_file_len))
            else {
                let _ = crate::allocation::oom();
                return;
            };
            if reserve_exact(&mut entries, present.len()).is_err()
                || reserve_exact(&mut files, file_bytes).is_err()
            {
                return;
            }
            for group in present.chunk_by(|a, b| a.id == b.id) {
                let row = group[0];
                if group.iter().any(|other| other.template != row.template) {
                    continue;
                }
                let file = &bytes
                    [row.template_file_start..row.template_file_start + row.template_file_len];
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
        crate::raw_memory::vector_bytes(&self.entries)
            + crate::raw_memory::vector_bytes(&self.files)
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
    rows: engine::walk::Buffer<'a, R>,
    bytes: engine::walk::Buffer<'a, u8>,
    len: usize,
    bytes_len: usize,
    required_bytes: usize,
    unread: u32,
    failure: Option<&'static str>,
}

impl<'a, R> Batch<'a, R> {
    fn new(rows: &'a mut [R], bytes: &'a mut [u8]) -> Self {
        Self::storage(
            engine::walk::Buffer::Fixed(rows),
            engine::walk::Buffer::Fixed(bytes),
        )
    }
    fn storage(rows: engine::walk::Buffer<'a, R>, bytes: engine::walk::Buffer<'a, u8>) -> Self {
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
        self.append_buffer(|mut rows, mut bytes| walk(&mut rows, &mut bytes), relocate);
    }

    fn append_buffer(
        &mut self,
        walk: impl FnOnce(engine::walk::Buffer<'_, R>, engine::walk::Buffer<'_, u8>) -> ReadResult,
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
        let result = if self.unread == 0 {
            walk(
                self.rows.suffix(self.len),
                self.bytes.suffix(self.bytes_len),
            )
        } else {
            walk(
                engine::walk::Buffer::Fixed(&mut self.rows[self.len..rows_end]),
                engine::walk::Buffer::Fixed(&mut self.bytes[self.bytes_len..bytes_end]),
            )
        };
        let rows_end = if self.unread == 0 {
            self.rows.len()
        } else {
            rows_end
        };
        let bytes_end = if self.unread == 0 {
            self.bytes.len()
        } else {
            bytes_end
        };
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

impl Batch<'_, cadvisor::Row> {
    fn append_tooltips(&mut self, memory: &impl Memory, start: usize, end: usize) {
        if !self.active() {
            return;
        }
        let captured = cadvisor::append_tooltips(
            memory,
            &mut self.rows[start..end],
            &mut self.bytes,
            ccharacter::NameWalk {
                outcome: Outcome::Rows { len: end - start },
                bytes_len: self.bytes_len,
                required_bytes: self.required_bytes,
            },
        );
        self.bytes_len = captured.bytes_len;
        self.required_bytes = captured.required_bytes;
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

    fn finish_advisor_tooltips(&mut self, memory: &impl Memory) {
        self.advisors.append_tooltips(memory, 0, self.advisors.len);
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
                            |template| {
                                templates.and_then(|cache| cache.known(character.id, template))
                            },
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
                    |rows, bytes| {
                        cadvisor::walk_prefix_entry(&view, base, character, rows, bytes).into()
                    },
                    |row, offset| {
                        relocate(&mut row.key_start, offset)
                            && relocate(&mut row.idea_start, offset)
                            && relocate(&mut row.slot_start, offset)
                            && relocate(&mut row.modifier_start, offset)
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
                relocate(&mut row.text_a_start, offset)
                    && relocate(&mut row.text_b_start, offset)
                    && row.codename.as_mut().is_none_or(|name| {
                        relocate(&mut name.custom.0, offset) && relocate(&mut name.group.0, offset)
                    })
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
                            owned.project(
                                0,
                                &mut trait_cursors[0],
                                pointer,
                                id,
                                rows,
                                bytes,
                                |row, _| row,
                            )
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
                            cunit_leader::walk_trait_experience(
                                memory, base, pointer, id, rows, bytes,
                            )
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
            family.visit(
                memory,
                base,
                view,
                owned_traits,
                templates,
                &mut trait_cursors,
            );
        }
        Ok::<_, &'static str>(())
    })();
    if let Err(reason) = result {
        family.fail(reason);
    }
    family.finish_advisor_tooltips(memory);
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
    family.finish_advisor_tooltips(memory);
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
    if let Outcome::Unavailable { .. } = text.outcome {
        return ReadResult {
            outcome: text.outcome,
            bytes_len: 0,
            required_bytes: text.required_bytes,
        };
    }
    let mut names = engine::cname_group_member::Names {
        bytes,
        len: text.bytes_len,
        required: text.required_bytes,
    };
    let (codename, codename_reason) = if cunit_leader::has_codename(native.kind) {
        match cunit_leader::read_codename(memory, base, pointer, &mut names) {
            Ok(engine::cname_group_member::Naming::Row(row)) => (Some(row), None),
            Ok(engine::cname_group_member::Naming::DoesNotFit) => {
                names.len = text.bytes_len;
                (None, Some("unit leader codename byte capacity exceeded"))
            }
            Err(reason) => {
                names.len = text.bytes_len;
                (None, Some(reason))
            }
        }
    } else {
        (None, None)
    };
    let fits = matches!(text.outcome, Outcome::Rows { len: 1 });
    let formation =
        cunit_leader::read_formation_reference(memory, pointer, native.kind, native.category);
    if fits {
        *output = LeaderRow {
            native,
            // Native +20 is saved as name; +40 is a separate unsaved string.
            text_a_start: text.token_start,
            text_a_len: text.token_len,
            text_b_start: text.name_start,
            text_b_len: text.name_len,
            codename,
            codename_reason,
            formation_reference: formation.unwrap_or(None),
            formation_reason: formation.err(),
        };
    }
    ReadResult {
        outcome: if fits {
            Outcome::Rows { len: 1 }
        } else {
            Outcome::Truncated {
                len: 0,
                unread_rows: 1,
            }
        },
        bytes_len: if fits { names.len } else { 0 },
        required_bytes: names.required,
    }
}

pub(crate) fn ability_demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<(usize, usize), &'static str> {
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let mut rows = 0_usize;
    let mut bytes = 0_usize;
    for slot in 1..countries.count() {
        let Some(country) = countries.at(slot)? else {
            continue;
        };
        let address = country
            .pointer
            .checked_add(ccountry::ACTIVE_ABILITIES)
            .ok_or("country active ability array address overflow")?;
        let (count, text) = active_ability_record::demand(memory, base, address)?;
        rows = rows
            .checked_add(count)
            .ok_or("active ability demand overflow")?;
        bytes = bytes
            .checked_add(text)
            .ok_or("active ability token demand overflow")?;
    }
    Ok((rows, bytes))
}

#[cfg(test)]
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

pub(super) fn walk_abilities_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut Vec<active_ability_record::Row>,
    bytes: &mut Vec<u8>,
) -> ReadResult {
    let mut rows = engine::walk::Buffer::Growing(rows);
    let mut bytes = engine::walk::Buffer::Growing(bytes);
    let mut len = 0_usize;
    let mut bytes_len = 0_usize;
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
            let walk = active_ability_record::walk_buffer(
                memory,
                base,
                country.id,
                address,
                rows.suffix(len),
                bytes.suffix(bytes_len),
            );
            match walk.outcome {
                Outcome::Rows { len: count } => {
                    for row in &mut rows[len..len + count] {
                        if !relocate(&mut row.token_start, bytes_len) {
                            return Err("active ability token offset overflow");
                        }
                    }
                    len += count;
                    bytes_len += walk.bytes_len;
                }
                Outcome::Unavailable { reason } => return Err(reason),
                Outcome::Truncated { .. } => {
                    return Err("growing active ability capture truncated");
                }
            }
        }
        Ok(())
    })();
    ReadResult {
        outcome: match result {
            Ok(()) => Outcome::Rows { len },
            Err(reason) => Outcome::Unavailable { reason },
        },
        bytes_len,
        required_bytes: bytes_len,
    }
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

fn advisor_ranges_valid(row: &cadvisor::Row, bytes: &[u8]) -> bool {
    valid(bytes, row.key_start, row.key_len)
        && valid(bytes, row.idea_start, row.idea_len)
        && valid(bytes, row.slot_start, row.slot_len)
        && valid(bytes, row.modifier_start, row.modifier_len)
        && row
            .tooltip_start
            .checked_add(row.tooltip_len)
            .and_then(|end| bytes.get(row.tooltip_start..end))
            .is_some_and(tooltip_ranges_valid)
        && row
            .modifier_len
            .is_multiple_of(engine::modifier_entry::STRIDE)
}

fn tooltip_ranges_valid(mut bytes: &[u8]) -> bool {
    while !bytes.is_empty() {
        let Some(header) = bytes.get(..8) else {
            return false;
        };
        let Ok(len) = usize::try_from(u64::from_le_bytes(header.try_into().unwrap())) else {
            return false;
        };
        let Some(tail) = len.checked_add(8).and_then(|end| bytes.get(end..)) else {
            return false;
        };
        bytes = tail;
    }
    true
}

/// Used only after `view` validates every range for this table.
fn owned(bytes: &[u8], start: usize, len: usize) -> Vec<u8> {
    owned_vec(copy_vec(&bytes[start..start + len]))
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
        exile_country: replay::rows::Recorded::Value(native.exile_country),
        codename: replay::rows::Recorded::Value(
            row.codename
                .map(|name| super::character_inputs::operands(name, bytes)),
        ),
        formation_reference: replay::rows::Recorded::Value(row.formation_reference),
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
/// leader map head, unit leader pointer, advisor map head, gender.
const VIEW_OFFSETS: [usize; 8] = [
    0,
    ccharacter::COUNTRY,
    ccharacter::NAME_TOKEN_ID,
    ccharacter::NAME,
    ccharacter::LEADER_MAP_HEAD,
    ccharacter::UNIT_LEADER,
    ccharacter::ADVISOR_MAP_HEAD,
    ccharacter::GENDER,
];
const VIEW_LENS: [usize; 8] = [16, 4, ccharacter::NAME_TOKEN_SPAN, 64, 16, 8, 16, 4];
const VIEW_MASKS: [u32; 8] = [
    MASK_ALL,
    MASK_COUNTRY,
    MASK_CHARACTERS,
    MASK_CHARACTERS,
    MASK_LEADERS,
    MASK_UNIT_LINK,
    MASK_ADVISORS,
    MASK_CHARACTERS,
];

/// Roster pointer pages are read in 32-entry chunks, exactly like the typed
/// cursor, so the replay requests the identical page spans.
const ROSTER_PAGE: u32 = 32;

mod traits;
use traits::*;

mod owned_copy;
pub(crate) use owned_copy::*;

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

/// Production acquisition is worker-only. Grow staging before each next
/// native observation; the fixed path remains available to fault fixtures.
pub(crate) fn copy_characters_growing(
    memory: &impl Memory,
    ctx: CharactersCopyContext,
    job: &mut CharactersJob,
    roles: Roles<&census::RoleCensus>,
) -> CharactersCopyReport {
    job.worker_growing = true;
    let report = copy_characters(memory, ctx, job, roles);
    job.update_census_charge();
    job.appended_charge.set(crate::raw_memory::Capacity {
        auxiliary_vectors: crate::raw_memory::vector_bytes(&job.appended_roles),
        ..Default::default()
    });
    job.worker_growing = false;
    report
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
    job.capture_growing = job.worker_growing;
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
        if job.worker_growing {
            cache.capture_growing(memory, ctx, &crate::advisor_changes::Observer, loaned);
        } else {
            cache.capture(memory, ctx, &crate::advisor_changes::Observer, loaned);
        }
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
    BACKEND_NS[job.part].fetch_add(
        job.log.backend_ns
            + job
                .advisor_dynamic
                .as_ref()
                .map_or(0, |job| job.log.backend_ns),
        Relaxed,
    );
    COPY_CALLS[job.part].fetch_add(1, Relaxed);
    let mut raw_bytes = job.log.raw_bytes;
    if let Some(cache) = &job.advisor_cache {
        raw_bytes[2] = cache.raw_bytes;
    }
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

impl Captured {
    fn install_family(&mut self, active: [bool; 6], results: [ReadResult; 6]) {
        if active[0] {
            self.characters.primary(results[0]);
        }
        if active[1] {
            self.leaders.primary(results[1]);
        }
        if active[2] {
            self.advisors.primary(results[2]);
            advisors::record_modifiers(&self.advisors.rows.rows[..results[2].outcome.len()]);
        }
        if active[3] {
            self.units.primary(results[3]);
        }
        if active[4] {
            self.traits.primary(results[4]);
        }
        if active[5] {
            self.trait_experience.primary(results[5]);
        }
    }

    /// Failure is local to the tables owned by this acquisition job.
    pub(crate) fn fail_final_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        let active = self.begin_capture(hour, force_full);
        self.fail_character_tables(active, reason);
    }

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
        fail!(
            characters = 0,
            leaders = 1,
            advisors = 2,
            units = 3,
            traits = 4,
            trait_experience = 5
        );
    }
    fn reserve_advisors(&mut self, demand: AdvisorDemand) -> Result<(), &'static str> {
        if demand.overflowed {
            return Err("advisor capture demand overflow");
        }
        let rows = &mut self.advisors.rows.rows;
        let bytes = &mut self.advisors.bytes.first;
        reserve(rows, demand.rows.saturating_sub(rows.len()))
            .map_err(|_| "advisor row allocation failed")?;
        reserve(bytes, demand.bytes.saturating_sub(bytes.len()))
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
        let bytes = self
            .characters
            .bytes
            .first
            .get(..self.characters.bytes.len)
            .unwrap_or(&[]);
        job.templates.rebuild(
            self.characters.rows.outcome,
            &self.characters.rows.rows,
            bytes,
        );
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
        self.active = std::array::from_fn(|index| {
            self.active[index] || self.raw_active[index] || job.active[index]
        });
        self.fail_character_tables(
            self.active.map(|value| !value),
            replay::cadence::SKIPPED_REASON,
        );
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
                rows: job
                    .advisor_demand
                    .rows
                    .saturating_add(dynamic.advisor_demand.rows),
                bytes: job
                    .advisor_demand
                    .bytes
                    .saturating_add(dynamic.advisor_demand.bytes),
                overflowed: job.advisor_demand.overflowed
                    || dynamic.advisor_demand.overflowed
                    || job
                        .advisor_demand
                        .rows
                        .checked_add(dynamic.advisor_demand.rows)
                        .is_none()
                    || job
                        .advisor_demand
                        .bytes
                        .checked_add(dynamic.advisor_demand.bytes)
                        .is_none(),
            };
            let failure = self.reserve_advisors(demand).err();
            let mut advisors = self.advisors.batch();
            if let Some(reason) = failure {
                advisors.fail(reason);
            }
            let roots = crate::raw::Recorded::build(&job.advisor_roots);
            match ccharacter_manager::Roster::read(&roots, base, job.game_state) {
                Ok(roster) => {
                    let recorded = [
                        crate::raw::Recorded::build(&job.log),
                        crate::raw::Recorded::build(&dynamic.log),
                    ];
                    let mut ends = [0; 2];
                    for (index, ((_, section), source)) in
                        roster.sections().into_iter().zip(&recorded).enumerate()
                    {
                        if !advisors.active() {
                            break;
                        }
                        walk_advisor_section(source, base, section, &mut advisors);
                        ends[index] = advisors.len;
                    }
                    let mut start = 0;
                    for (end, source) in ends.into_iter().zip(&recorded) {
                        if end >= start {
                            advisors.append_tooltips(source, start, end);
                            start = end;
                        }
                    }
                }
                Err(reason) => advisors.fail(reason),
            }
            let result = advisors.result();
            self.advisors.primary(result);
            advisors::record_modifiers(&self.advisors.rows.rows[..result.outcome.len()]);
            self.raw_active[2] = false;
            decode_ticks[2] += elapsed(started);
            return;
        }
        let advisor_failure = if job.active[2] {
            self.reserve_advisors(job.advisor_demand).err()
        } else {
            None
        };
        let trait_failures = if job.traits.growing {
            let demand = job.traits.demand();
            fn reserve<R: Default>(
                rows: &mut Vec<R>,
                bytes: &mut Vec<u8>,
                demand: Result<(usize, usize), &'static str>,
            ) -> Result<(), &'static str> {
                let (row_count, byte_count) = demand?;
                engine::walk::Buffer::Growing(rows).ensure(row_count)?;
                engine::walk::Buffer::Growing(bytes).ensure(byte_count)?;
                Ok(())
            }
            [
                reserve(
                    &mut self.traits.rows.rows,
                    &mut self.traits.bytes.first,
                    demand[0],
                )
                .err(),
                reserve(
                    &mut self.trait_experience.rows.rows,
                    &mut self.trait_experience.bytes.first,
                    demand[1],
                )
                .err(),
            ]
        } else {
            [None; 2]
        };
        // The worker counted each acquired roster/name/template occurrence;
        // decode reserves those original observations before the fixed Batch.
        fn reserve_payload<R: Default>(
            buffer: &mut Buffer<R>,
            demand: AdvisorDemand,
        ) -> Result<(), &'static str> {
            if demand.overflowed {
                return Err("character capture demand overflow");
            }
            engine::walk::Buffer::Growing(&mut buffer.rows.rows).ensure(demand.rows)?;
            engine::walk::Buffer::Growing(&mut buffer.bytes.first).ensure(demand.bytes)?;
            Ok(())
        }
        let payload_failures = if job.capture_growing {
            [
                reserve_payload(&mut self.characters, job.character_demand).err(),
                reserve_payload(&mut self.units, job.unit_demand).err(),
            ]
        } else {
            [None; 2]
        };
        let leader_failure = if job.active[1] && job.capture_growing {
            engine::walk::Buffer::Growing(&mut self.leaders.rows.rows)
                .ensure(job.leader_rows)
                .err()
        } else {
            None
        };
        let recorded = crate::raw::Recorded::build(&job.log);
        let mut family = self.family().select(job.active);
        if let Some(reason) = payload_failures[0] {
            family.characters.fail(reason);
        }
        if let Some(reason) = payload_failures[1] {
            family.units.fail(reason);
        }
        if let Some(reason) = leader_failure {
            family.leaders.fail(reason);
        }
        if let Some(reason) = trait_failures[0] {
            family.traits.fail(reason);
        }
        if let Some(reason) = trait_failures[1] {
            family.experience.fail(reason);
        }
        if let Some(reason) = advisor_failure {
            family.advisors.fail(reason);
        }
        let results = if job.census_capture {
            walk_census(
                &recorded,
                base,
                family,
                Some(&job.traits),
                &job.census_visited,
                job.census_historical,
            )
        } else {
            walk_family(
                &recorded,
                base,
                job.game_state,
                family,
                Some(&job.traits),
                Some(&job.templates),
            )
        };
        self.install_family(job.active, results);
        decode_ticks[anchor(job.active)] += elapsed(started);
        for (raw, active) in self.raw_active.iter_mut().zip(job.active) {
            if active {
                *raw = false;
            }
        }
    }
}

#[cfg(test)]
mod character_target_hint_tests;

/// Shared synthetic address space for the characters copy/decode split
/// harness: role maps with leader/advisor nodes, heap and inline strings,
/// skill definitions, army and navy kinds, and a third navy character.
#[cfg(test)]
pub(crate) mod characters_test_kit;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod shadow_fixture;
#[cfg(test)]
pub(crate) use shadow_fixture::character_shadow_fixture;

#[cfg(test)]
mod shadow_tests;
