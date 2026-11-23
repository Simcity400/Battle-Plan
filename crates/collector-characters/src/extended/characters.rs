use engine_allocation::{copy_vec, owned_vec, reserve, reserve_exact};
pub mod advisors;
mod carry;
#[cfg(test)]
mod carry_tests;
pub mod census;
#[cfg(test)]
mod census_tests;
mod daily_experience;
mod leader_carry;
mod leaders;
#[cfg(test)]
mod leaders_tests;
mod quiet;
#[cfg(test)]
mod quiet_bench;

use collector_memory::read_accounting;
use collector_primitives::{
    clock::qpc,
    table_capture::{Arena, CaptureRows},
};
use collector_projection::table_conversion::{table, unavailable};
use engine_land_inputs::active_ability_record;
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    msvc_string,
    msvc_tree_node,
    read,
    walk::Outcome,
};
use engine_native::{
    cadvisor,
    carmy_leader,
    ccharacter::{
        self,
        manager as ccharacter_manager,
        template as ccharacter_template,
    },
    ccountry,
    cgame_state,
    cunit_leader,
};
use engine_native_parts::{
    character_advisor_node,
    cnavy_leader,
    cunit_leader_skill,
    cunit_leader_trait,
};
use engine_native_politics::{ccountry_leader, character_leader_node};
use replay_data::{
    BranchId, rows::ActiveAbilityRow, rows::AdvisorRoleRow, rows::CharacterRow,
    rows::CountryLeaderRoleRow, rows::TableId, rows::TableRows, rows::TableSchema,
    rows::UnitLeaderRow, rows::UnitLeaderTraitRow, rows::tables,
};
use replay_frame::{frame::TableInput, frame::TableView};

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub bytes: [usize; N],
}

collector_primitives::capacity_max!(Capacities { rows, bytes });

impl Capacities {
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        demand::presize(self, memory, base, state)
    }

    pub fn raise_for_characters(&mut self, demand: ccharacter::Demand) {
        use collector_primitives::country_slots::{raise, rows};
        raise(&mut self.rows[0], rows(demand.rows, 1));
        raise(&mut self.bytes[0], rows(demand.bytes, 1));
    }
}

mod demand;

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

    fn growing_batch(&mut self) -> GrowingBatch<'_, R> {
        GrowingBatch::new(&mut self.rows.rows, &mut self.bytes.first)
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
    codename: Option<engine_native::cname_group_member::Inputs>,
    codename_reason: Option<&'static str>,
    formation_reference: Option<u64>,
    formation_reason: Option<&'static str>,
    training: (usize, usize),
    training_factors: (usize, usize),
    training_reason: Option<&'static str>,
}

pub struct Captured {
    active: [bool; N],
    characters: Buffer<ccharacter::Row>,
    leaders: Buffer<ccountry_leader::Row>,
    advisors: Buffer<cadvisor::Row>,
    units: Buffer<LeaderRow>,
    traits: Buffer<cunit_leader::TraitRow>,
    training_outcome: Outcome,
    pub raw_active: [bool; N],
    fallback_jobs: [Option<Box<CharactersJob>>; PARTS],
    quiet_rows: Option<quiet::RowsKey>,
    held: Held,
}

#[derive(Clone, Copy, Debug, Default)]
struct Held {
    characters: Option<quiet::RowsKey>,
    units: Option<((u64, u64), ReadResult)>,
    traits: Option<((u64, u64), ReadResult)>,
    advisors: Option<(u64, ReadResult)>,
}

pub const PART_MASKS: [[bool; N]; 4] = [
    [true, false, false, true, false, true],
    [false, true, false, false, false, false],
    [false, false, true, false, false, false],
    [false, false, false, false, true, false],
];
pub const PARTS: usize = PART_MASKS.len();

fn roster_part(active: [bool; N]) -> bool {
    active[..5] == PART_MASKS[0][..5]
}

fn anchor(active: [bool; N]) -> usize {
    active.iter().position(|&value| value).unwrap_or(0)
}

pub const SHADOW_TABLES: [TableId; 4] = [
    TableId::Characters,
    TableId::CountryLeaderRoles,
    TableId::AdvisorRoles,
    TableId::UnitLeaderTraits,
];

mod shadow;
pub use shadow::*;

impl Captured {
    pub fn character_activity(&self) -> [bool; N] {
        self.active
    }

    pub fn prepare_hour(&mut self, hour: i32, force_full: bool) {
        if force_full
            || replay_data::cadence::Cadence::Daily(replay_data::cadence::COMMANDER_PROGRESS_SLOT)
                .is_refresh(hour)
        {
            daily_experience::STORE
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .forget();
        }
    }

    pub fn set_character_activity(&mut self, active: [bool; N]) {
        self.active = active;
        if !active[5] {
            self.training_outcome = Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON,
            };
        }
        self.fail_character_tables(
            active.map(|value| !value),
            replay_data::cadence::SKIPPED_REASON,
        );
    }
    fn family(&mut self) -> Family<'_> {
        Family {
            characters: self.characters.batch(),
            leaders: self.leaders.batch(),
            advisors: self.advisors.batch(),
            units: self.units.growing_batch(),
            traits: self.traits.batch(),
            training: true,
        }
    }

    pub fn with_capacities(capacities: &Capacities) -> Self {
        Self {
            characters: Buffer::new(capacities, 0),
            leaders: Buffer::new(capacities, 1),
            advisors: Buffer::new(capacities, 2),
            units: Buffer::new(capacities, 3),
            traits: Buffer::new(capacities, 4),
            training_outcome: Outcome::Unavailable {
                reason: "unit leader training not captured",
            },
            raw_active: [false; N],
            active: [false; N],
            fallback_jobs: std::array::from_fn(|_| None),
            quiet_rows: None,
            held: Held::default(),
        }
    }

    pub fn vacant() -> Self {
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
    }

    pub fn allocated_bytes(&self) -> usize {
        self.characters.allocated_bytes()
            + self.leaders.allocated_bytes()
            + self.advisors.allocated_bytes()
            + self.units.allocated_bytes()
            + self.traits.allocated_bytes()
    }

    pub fn grow(&mut self, capacities: &mut Capacities) {
        capacities.rows[4] = capacities.rows[4].max(self.traits.rows.outcome.len());
        capacities.bytes[4] = capacities.bytes[4].max(self.traits.bytes.len);
        self.characters.grow(capacities, 0);
        self.leaders.grow(capacities, 1);
        capacities.rows[2] = capacities.rows[2].max(self.advisors.rows.outcome.len());
        capacities.bytes[2] = capacities.bytes[2].max(self.advisors.bytes.len);
        self.advisors.grow(capacities, 2);
        self.units.grow(capacities, 3);
        self.traits.grow(capacities, 4);
    }

    fn begin_capture(&mut self, hour: i32, force_full: bool) -> [bool; N] {
        self.reset();
        let active = TABLES.map(|table| replay_data::cadence::should_walk(table, hour, force_full));
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
                        reason: replay_data::cadence::SKIPPED_REASON,
                    };
                }
            )+};
        }
        skip!(
            characters = 0,
            leaders = 1,
            advisors = 2,
            units = 3,
            traits = 4
        );
        std::array::from_fn(|index| family_active[index] && !self.raw_active[index])
    }

    #[cfg(any(test, feature = "test-support"))]
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

    pub fn capture_growing(
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
            let context = CharactersCopyContext {
                base,
                game_state,
                force_full,
                active,
            };
            let _reads = read_accounting::table(TABLES[anchor(active)]);
            let begin = qpc();
            let report = copy_characters(memory, context, &mut job, Roles::Independent);
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
            self.training_outcome,
        ]
    }

    pub fn tables(&self) -> [TableView; N] {
        self.convert_tables(
            |view| view,
            self.characters_table(),
            self.leaders_table(),
            self.advisors_table(),
            self.traits_table(),
            self.training_table(),
        )
    }

    pub fn table_inputs(&self, reuse: Option<&mut CharacterConversion<'_>>) -> [TableInput; N] {
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
        let training = self.training_input(&mut reuse.shadow.training);
        self.convert_tables(
            TableInput::Sampled,
            characters,
            leaders,
            advisors,
            traits,
            training,
        )
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
        view::<tables::CountryLeaderRoles, _>(
            &self.leaders,
            |_, _| true,
            |row, _| CountryLeaderRoleRow {
                character: row.character,
                country: row.country,
                ideology: row.ideology,
                current_ideology: replay_data::rows::Recorded::Value(row.current_ideology),
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
                modifier_cache: replay_data::rows::Recorded::Value(row.modifier_recorded.then(
                    || {
                        let entries = bytes
                            [row.modifier_start..row.modifier_start + row.modifier_len]
                            .chunks_exact(engine_native_parts::modifier_entry::STRIDE);
                        let mut values = owned_vec(engine_allocation::vec(entries.len()));
                        for entry in entries {
                            values.push(replay_data::rows::CachedModifier {
                                definition: u32::from_le_bytes(entry[..4].try_into().unwrap()),
                                value: i64::from_le_bytes(entry[8..16].try_into().unwrap()),
                            });
                        }
                        replay_data::rows::AdvisorModifierCache { entries: values }
                    },
                )),
                tooltip_keys: replay_data::rows::Recorded::Value(row.tooltip_recorded.then(|| {
                    let mut source = &bytes[row.tooltip_start..row.tooltip_start + row.tooltip_len];
                    let mut remaining = source;
                    let mut count = 0;
                    while !remaining.is_empty() {
                        let len = u64::from_le_bytes(remaining[..8].try_into().unwrap()) as usize;
                        remaining = &remaining[8 + len..];
                        count += 1;
                    }
                    let mut keys = owned_vec(engine_allocation::vec(count));
                    while !source.is_empty() {
                        let len = u64::from_le_bytes(source[..8].try_into().unwrap()) as usize;
                        source = &source[8..];
                        keys.push(owned_vec(engine_allocation::copy_vec(&source[..len])));
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

    fn training_input(&self, cache: &mut Option<((u64, u64), Weak<TableRows>)>) -> TableInput {
        if !self.active[5] {
            return TableInput::Sampled(unavailable(replay_data::cadence::SKIPPED_REASON));
        }
        let key = self
            .held
            .units
            .filter(|(_, result)| matches!(result.outcome, Outcome::Rows { .. }))
            .map(|(key, _)| key);
        if let Some(key) = key
            && let Some((previous, output)) = cache
            && *previous == key
            && let Some(output) = output.upgrade()
        {
            return TableInput::SharedComplete(output);
        }
        match self.training_table() {
            TableView::Complete(rows) => {
                let output = Arc::new(rows);
                *cache = key.map(|key| (key, Arc::downgrade(&output)));
                TableInput::SharedComplete(output)
            }
            view => {
                *cache = None;
                TableInput::Sampled(view)
            }
        }
    }

    fn training_outcome(&self) -> Outcome {
        let outcome = self.units.rows.outcome;
        let Some(rows) = self.units.rows.rows.get(..outcome.len()) else {
            return Outcome::Unavailable {
                reason: "unit leader training source rows invalid",
            };
        };
        if let Outcome::Unavailable { .. } = outcome {
            return outcome;
        }
        if let Some(reason) = rows.iter().find_map(|row| row.training_reason) {
            return Outcome::Unavailable { reason };
        }
        let len = rows
            .iter()
            .map(|row| (row.training.1 + row.training_factors.1) / 16)
            .sum();
        match outcome {
            Outcome::Truncated { unread_rows, .. } => Outcome::Truncated { len, unread_rows },
            _ => Outcome::Rows { len },
        }
    }

    fn training_table(&self) -> TableView {
        if !self.active[5] {
            return unavailable(replay_data::cadence::SKIPPED_REASON);
        }
        let rows = &self.units.rows.rows[..self.units.rows.outcome.len()];
        if let Outcome::Unavailable { reason } = self.units.rows.outcome {
            return unavailable(reason);
        }
        if let Some(reason) = rows.iter().find_map(|row| row.training_reason) {
            return unavailable(reason);
        }
        let bytes = &self.units.bytes.first[..self.units.bytes.len];
        if rows.iter().any(|row| {
            [row.training, row.training_factors]
                .iter()
                .any(|&(start, len)| !valid(bytes, start, len) || len % 16 != 0)
        }) {
            return unavailable("unit leader training span invalid");
        }
        let count = rows
            .iter()
            .map(|row| (row.training.1 + row.training_factors.1) / 16)
            .sum();
        let mut output = owned_vec(engine_allocation::vec(count));
        for row in rows {
            for (list, (start, len)) in [(0, row.training), (1, row.training_factors)] {
                for (ordinal, entry) in bytes[start..start + len].chunks_exact(16).enumerate() {
                    output.push(replay_data::rows::UnitLeaderTraitExperienceRow {
                        leader_reference: leader_reference(row.native.id),
                        list,
                        ordinal: ordinal as u32,
                        token_id: i32::from_le_bytes(entry[..4].try_into().unwrap()),
                        definition_ordinal: i32::from_le_bytes(entry[4..8].try_into().unwrap()),
                        experience: i64::from_le_bytes(entry[8..].try_into().unwrap()),
                    });
                }
            }
        }
        let outcome = match self.units.rows.outcome {
            Outcome::Truncated { unread_rows, .. } => Outcome::Truncated {
                len: output.len(),
                unread_rows,
            },
            _ => Outcome::Rows { len: output.len() },
        };
        table::<tables::UnitLeaderTraitExperience, _>(outcome, &output, Clone::clone)
    }

    fn convert_tables<T>(
        &self,
        wrap: impl Fn(TableView) -> T,
        characters: T,
        leaders: T,
        advisors: T,
        traits: T,
        training: T,
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
            training,
        ]
    }
}

pub fn sort_dropping_repeated<T>(entries: &mut Vec<T>, pointer: impl Fn(&T) -> usize) {
    entries.sort_unstable_by_key(&pointer);
    let mut index = 0;
    while index < entries.len() {
        let at = pointer(&entries[index]);
        let end = entries[index..]
            .iter()
            .position(|entry| pointer(entry) != at)
            .map_or(entries.len(), |offset| index + offset);
        if end - index > 1 {
            entries.drain(index..end);
        } else {
            index = end;
        }
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

#[derive(Debug, Default)]
pub struct TemplateCache {
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
            present.sort_unstable_by_key(|row| (row.id, *row as *const ccharacter::Row as usize));
            let Some(file_bytes) = present
                .iter()
                .try_fold(0usize, |sum, row| sum.checked_add(row.template_file_len))
            else {
                let _ = collector_memory::terminal::oom();
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
        collector_primitives::raw_memory::vector_bytes(&self.entries)
            + collector_primitives::raw_memory::vector_bytes(&self.files)
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

struct Batch<'a, R> {
    rows: engine_native_memory::walk::Buffer<'a, R>,
    bytes: engine_native_memory::walk::Buffer<'a, u8>,
    len: usize,
    bytes_len: usize,
    required_bytes: usize,
    unread: u32,
    failure: Option<&'static str>,
}

impl<'a, R> Batch<'a, R> {
    fn new(rows: &'a mut Vec<R>, bytes: &'a mut Vec<u8>) -> Self {
        Self::storage(
            engine_native_memory::walk::Buffer::Growing(rows),
            engine_native_memory::walk::Buffer::Growing(bytes),
        )
    }
    fn storage(
        rows: engine_native_memory::walk::Buffer<'a, R>,
        bytes: engine_native_memory::walk::Buffer<'a, u8>,
    ) -> Self {
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
        walk: impl FnOnce(
            engine_native_memory::walk::Buffer<'_, R>,
            engine_native_memory::walk::Buffer<'_, u8>,
        ) -> ReadResult,
        relocate: impl Fn(&mut R, usize) -> bool,
    ) {
        if !self.active() {
            return;
        }
        let (result, rows_end, bytes_end) = if self.unread == 0 {
            let result = walk(
                self.rows.suffix(self.len),
                self.bytes.suffix(self.bytes_len),
            );
            (result, self.rows.len(), self.bytes.len())
        } else {
            let result = walk(
                engine_native_memory::walk::Buffer::Count,
                engine_native_memory::walk::Buffer::Count,
            );
            (result, self.len, self.bytes_len)
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

pub(super) struct Stored {
    pub(super) rows: usize,
    pub(super) bytes_len: usize,
    pub(super) required_bytes: usize,
}

struct GrowingBatch<'a, R> {
    rows: &'a mut Vec<R>,
    bytes: &'a mut Vec<u8>,
    len: usize,
    bytes_len: usize,
    required_bytes: usize,
    failure: Option<&'static str>,
}

impl<'a, R> GrowingBatch<'a, R> {
    fn new(rows: &'a mut Vec<R>, bytes: &'a mut Vec<u8>) -> Self {
        Self {
            rows,
            bytes,
            len: 0,
            bytes_len: 0,
            required_bytes: 0,
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
        walk: impl FnOnce(&mut Vec<R>, usize, &mut Vec<u8>, usize) -> Result<Stored, &'static str>,
        relocate: impl Fn(&mut R, usize) -> bool,
    ) {
        if !self.active() {
            return;
        }
        let stored = match walk(&mut *self.rows, self.len, &mut *self.bytes, self.bytes_len) {
            Ok(stored) => stored,
            Err(reason) => {
                self.fail(reason);
                return;
            }
        };
        if stored.rows > self.rows.len().saturating_sub(self.len)
            || stored.bytes_len > self.bytes.len().saturating_sub(self.bytes_len)
            || stored.required_bytes < stored.bytes_len
        {
            self.fail("character walk exceeded its pooled buffer");
            return;
        }
        let bytes_start = self.bytes_len;
        if !self.rows[self.len..self.len + stored.rows]
            .iter_mut()
            .all(|row| relocate(row, bytes_start))
        {
            self.fail("character byte offset overflow");
            return;
        }
        let Some(required) = self.required_bytes.checked_add(stored.required_bytes) else {
            self.fail("character byte count overflow");
            return;
        };
        self.len += stored.rows;
        self.bytes_len += stored.bytes_len;
        self.required_bytes = required;
    }

    fn result(&self) -> ReadResult {
        match self.failure {
            Some(reason) => ReadResult::rows(Outcome::Unavailable { reason }),
            None => ReadResult {
                outcome: Outcome::Rows { len: self.len },
                bytes_len: self.bytes_len,
                required_bytes: self.required_bytes,
            },
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
            self.bytes.suffix(0),
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
    units: GrowingBatch<'a, LeaderRow>,
    traits: Batch<'a, cunit_leader::TraitRow>,
    training: bool,
}

impl Family<'_> {
    fn select(mut self, active: [bool; N]) -> Self {
        self.training = active[5];
        macro_rules! select {
            ($($field:ident = $index:literal),+ $(,)?) => {$(
                if !active[$index] {
                    self.$field.fail(replay_data::cadence::SKIPPED_REASON);
                }
            )+};
        }
        select!(
            characters = 0,
            leaders = 1,
            advisors = 2,
            units = 3,
            traits = 4
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
    }

    fn required(&self) -> [bool; 4] {
        [
            self.characters.active(),
            self.leaders.active(),
            self.units.active() || self.traits.active(),
            self.advisors.active(),
        ]
    }

    fn fail(&mut self, reason: &'static str) {
        self.characters.fail(reason);
        self.leaders.fail(reason);
        self.advisors.fail(reason);
        self.units.fail(reason);
        self.traits.fail(reason);
    }

    fn results(&self) -> [ReadResult; N] {
        [
            self.characters.result(),
            self.leaders.result(),
            self.advisors.result(),
            self.units.result(),
            self.traits.result(),
            self.units.result(),
        ]
    }

    fn finish_advisor_tooltips(&mut self, memory: &impl Memory) {
        self.advisors.append_tooltips(memory, 0, self.advisors.len);
    }

    #[allow(clippy::too_many_arguments)]
    fn visit<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        view: ccharacter::View<'_, M>,
        owned_traits: Option<&TraitCapture>,
        templates: Option<&TemplateCache>,
        carried: Carried<'_>,
        trait_cursor: &mut usize,
    ) {
        let (character_pointer, character_id) = view.reference();
        match view.entry() {
            Ok(character) => {
                self.characters.append(
                    |rows, bytes| match carried
                        .characters
                        .and_then(|carry| carry.carried_row(character.pointer, character.id))
                    {
                        Some((held, text)) => leader_carry::held_rows(
                            &[ccharacter::Row {
                                id: character.id,
                                country: character.country,
                                roster: character.roster,
                                ordinal: character.ordinal,
                                ..held
                            }],
                            text,
                            rows,
                            bytes,
                        ),
                        None => ccharacter::walk_entry(
                            &view,
                            base,
                            character,
                            |template| {
                                templates.and_then(|cache| cache.known(character.id, template))
                            },
                            rows,
                            bytes,
                        )
                        .into(),
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
        let training = self.training;
        let Family { units, traits, .. } = self;
        if !units.active() && !traits.active() {
            return;
        }
        let pointer = character_pointer
            .checked_add(ccharacter::UNIT_LEADER)
            .and_then(|address| read::<u64>(&view, address))
            .and_then(|pointer| usize::try_from(pointer).ok());
        let Some(pointer) = pointer else {
            units.fail("character unit leader pointer unreadable");
            traits.fail("character unit leader pointer unreadable");
            return;
        };
        if pointer == 0 {
            return;
        }
        let previous_len = units.len;
        units.append(
            |rows, at, bytes, from| match carried
                .units
                .and_then(|carry| carry.carried_rows(pointer))
            {
                Some((held, text)) => {
                    leader_carry::held_rows_into(held, text, rows, at, bytes, from)
                }
                None => {
                    read_unit_selected(memory, base, pointer, character_id, bytes, from, training)
                        .and_then(|read| read.store(rows, at))
                }
            },
            |row, offset| {
                relocate(&mut row.text_a_start, offset)
                    && relocate(&mut row.text_b_start, offset)
                    && relocate(&mut row.training.0, offset)
                    && relocate(&mut row.training_factors.0, offset)
                    && row.codename.as_mut().is_none_or(|name| {
                        relocate(&mut name.custom.0, offset) && relocate(&mut name.group.0, offset)
                    })
            },
        );
        if let Some((held, text)) = carried.traits.and_then(|carry| carry.carried_rows(pointer)) {
            traits.append(
                |rows, bytes| leader_carry::held_rows(held, text, rows, bytes),
                |row, offset| relocate(&mut row.token_start, offset),
            );
            return;
        }
        let identity = if traits.active() {
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
                            owned.project(trait_cursor, pointer, id, rows, bytes)
                        } else {
                            cunit_leader::walk_traits(memory, base, pointer, id, rows, bytes).into()
                        }
                    },
                    |row, offset| relocate(&mut row.token_start, offset),
                );
            }
            Some(Err(reason)) => traits.fail(reason),
            None => {}
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
fn walk_family<M: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    family: Family<'_>,
    owned_traits: Option<&TraitCapture>,
    templates: Option<&TemplateCache>,
) -> [ReadResult; N] {
    walk_family_carrying(
        memory,
        base,
        game_state,
        family,
        owned_traits,
        templates,
        None,
    )
}

fn walk_family_carrying<M: Memory>(
    memory: &M,
    base: usize,
    game_state: usize,
    mut family: Family<'_>,
    owned_traits: Option<&TraitCapture>,
    templates: Option<&TemplateCache>,
    carry: Option<&carry::Carry>,
) -> [ReadResult; N] {
    let required = family.required();
    let mut trait_cursor = 0;
    let result = (|| {
        if !required.into_iter().any(|active| active) {
            return Ok(());
        }
        let roster = ccharacter_manager::Roster::read(memory, base, game_state)?;
        let mut cursor = roster.cursor();
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
                Carried {
                    characters: carry,
                    ..Carried::default()
                },
                &mut trait_cursor,
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

fn walk_census<M: Memory>(
    memory: &M,
    base: usize,
    mut family: Family<'_>,
    owned_traits: Option<&TraitCapture>,
    visited: impl IntoIterator<Item = census::Occurrence>,
    historical: usize,
    carried: Carried<'_>,
) -> [ReadResult; N] {
    let required = family.required();
    debug_assert!(
        !required[0] || carried.characters.is_some(),
        "the role census holds no character names"
    );
    let mut trait_cursor = 0;
    for occurrence in visited {
        if !family.alive() {
            break;
        }
        let identity = occurrence.identity(historical);
        match ccharacter::View::acquired(memory, base, identity, occurrence.fields(required)) {
            Ok(view) => family.visit(
                memory,
                base,
                view,
                owned_traits,
                None,
                carried,
                &mut trait_cursor,
            ),
            Err(reason) => {
                family.fail(reason);
                break;
            }
        }
    }
    family.finish_advisor_tooltips(memory);
    family.results()
}

fn fresh_traits_unchanged(job: &CharactersJob) -> bool {
    let carry = &job.trait_carry;
    let mut vectors = job.traits.vectors.iter().enumerate();
    let mut rows = Vec::new();
    let mut bytes = Vec::new();
    for at in 0..carry.visit_count() {
        let Some((pointer, _)) = carry.fresh_visit(at) else {
            continue;
        };
        let Some((mut cursor, vector)) = vectors.next() else {
            return false;
        };
        let Outcome::Rows { len } = vector.outcome else {
            return false;
        };
        rows.resize(len, cunit_leader::TraitRow::default());
        bytes.resize(vector.bytes_len, 0);
        let projected = job.traits.project(
            &mut cursor,
            pointer,
            vector.leader,
            engine_native_memory::walk::Buffer::Exact(&mut rows),
            engine_native_memory::walk::Buffer::Exact(&mut bytes),
        );
        if projected.outcome != (Outcome::Rows { len })
            || !carry.held_equals(at, &rows, &bytes[..projected.bytes_len])
        {
            return false;
        }
    }
    vectors.next().is_none()
}

#[derive(Clone, Copy, Default)]
struct Carried<'a> {
    characters: Option<&'a carry::Carry>,
    units: Option<&'a leader_carry::UnitCarry>,
    traits: Option<&'a leader_carry::TraitCarry>,
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

struct UnitRead {
    row: LeaderRow,
    bytes_len: usize,
    required_bytes: usize,
}

impl UnitRead {
    fn store(self, rows: &mut Vec<LeaderRow>, at: usize) -> Result<Stored, &'static str> {
        engine_native_memory::walk::grow(rows, at + 1)?;
        rows[at] = self.row;
        Ok(Stored {
            rows: 1,
            bytes_len: self.bytes_len,
            required_bytes: self.required_bytes,
        })
    }
}

fn read_unit_selected(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    character: u32,
    bytes: &mut Vec<u8>,
    from: usize,
    training_active: bool,
) -> Result<UnitRead, &'static str> {
    let native = cunit_leader::read(memory, base, pointer, character)?;
    let text = cunit_leader::read_text_into(memory, pointer, bytes, from)?;
    let text_len = text.name_len + text.token_len;
    let mut len = text_len;
    let mut required = text_len;
    let (codename, codename_reason) = if cunit_leader::has_codename(native.kind) {
        match cunit_leader::read_codename_into(
            memory,
            base,
            pointer,
            bytes,
            from,
            &mut len,
            &mut required,
        ) {
            Ok(row) => (Some(row), None),
            Err(reason) => {
                len = text_len;
                (None, Some(reason))
            }
        }
    } else {
        (None, None)
    };
    let training_start = len;
    let mut training_reason = None;
    let trained = training_active
        && matches!(
            native.kind,
            cunit_leader::KIND_ARMY | cunit_leader::KIND_NAVY
        );
    let mut progress_end = len;
    if trained {
        let result = cunit_leader::visit_trait_progress(
            memory,
            base,
            pointer,
            |token, definition, experience| {
                if let Some(reason) = training_entry(
                    bytes,
                    from,
                    &mut len,
                    &mut required,
                    (token, definition, experience),
                ) {
                    training_reason.get_or_insert(reason);
                }
            },
        );
        progress_end = len;
        if let Err(reason) = result {
            training_reason = Some(reason);
        }
    }
    if trained
        && let Err(reason) =
            cunit_leader::visit_trait_factors(memory, base, pointer, |token, definition, factor| {
                if let Some(reason) = training_entry(
                    bytes,
                    from,
                    &mut len,
                    &mut required,
                    (token, definition, factor),
                ) {
                    training_reason.get_or_insert(reason);
                }
            })
    {
        training_reason.get_or_insert(reason);
    }
    let mut training = (training_start, progress_end - training_start);
    let mut training_factors = (progress_end, len - progress_end);
    if training_reason.is_some() {
        len = training_start;
        training = (training_start, 0);
        training_factors = (training_start, 0);
    }
    let formation =
        cunit_leader::read_formation_reference(memory, pointer, native.kind, native.category);
    Ok(UnitRead {
        row: LeaderRow {
            native,
            text_a_start: text.name_len,
            text_a_len: text.token_len,
            text_b_start: 0,
            text_b_len: text.name_len,
            codename,
            codename_reason,
            formation_reference: formation.unwrap_or(None),
            formation_reason: formation.err(),
            training,
            training_factors,
            training_reason,
        },
        bytes_len: len,
        required_bytes: required,
    })
}

fn training_entry(
    bytes: &mut Vec<u8>,
    from: usize,
    len: &mut usize,
    required: &mut usize,
    (token, definition, value): (i32, i32, i64),
) -> Option<&'static str> {
    *required = required.saturating_add(16);
    let Some((end, stop)) = len
        .checked_add(16)
        .and_then(|end| Some((end, from.checked_add(end)?)))
    else {
        return Some("unit leader training byte count overflow");
    };
    if let Err(reason) = engine_native_memory::walk::grow(bytes, stop) {
        return Some(reason);
    }
    let start = from + *len;
    bytes[start..start + 4].copy_from_slice(&token.to_le_bytes());
    bytes[start + 4..start + 8].copy_from_slice(&definition.to_le_bytes());
    bytes[start + 8..stop].copy_from_slice(&value.to_le_bytes());
    *len = end;
    None
}

pub fn ability_demand(
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

pub(super) fn walk_abilities(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut Vec<active_ability_record::Row>,
    bytes: &mut Vec<u8>,
) -> ReadResult {
    let mut rows = engine_native_memory::walk::Buffer::Growing(rows);
    let mut bytes = engine_native_memory::walk::Buffer::Growing(bytes);
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
            .is_multiple_of(engine_native_parts::modifier_entry::STRIDE)
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
        exile_country: replay_data::rows::Recorded::Value(native.exile_country),
        codename: replay_data::rows::Recorded::Value(
            row.codename
                .map(|name| super::character_inputs::operands(name, bytes)),
        ),
        formation_reference: replay_data::rows::Recorded::Value(row.formation_reference),
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

const MASK_CHARACTERS: u32 = 0x01;
const MASK_LEADERS: u32 = 0x02;
const MASK_ADVISORS: u32 = 0x04;
const MASK_UNITS: u32 = 0x08 | MASK_TRAINING;
const MASK_TRAITS: u32 = 0x10;
const MASK_TRAINING: u32 = 0x20;
const MASK_ALL: u32 = 0x3f;
const MASK_COUNTRY: u32 = MASK_CHARACTERS | MASK_LEADERS | MASK_ADVISORS;
const MASK_UNIT_LINK: u32 = MASK_UNITS | MASK_TRAITS;
const MASK_TRAIT_FALLBACK: u32 = MASK_TRAITS;

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

const ROSTER_PAGE: u32 = 32;

mod traits;
use traits::*;

mod owned_copy;
pub use owned_copy::*;

#[derive(Clone, Copy)]
pub enum Roles<C> {
    Independent,
    Gather,
    Borrow(C),
}

impl Roles<engine_allocation::SharedBox<census::RoleCensus>> {
    pub fn as_deref(&self) -> Roles<&census::RoleCensus> {
        match self {
            Self::Independent => Roles::Independent,
            Self::Gather => Roles::Gather,
            Self::Borrow(census) => Roles::Borrow(census),
        }
    }
}

pub fn copy_characters(
    memory: &impl Memory,
    ctx: CharactersCopyContext,
    job: &mut CharactersJob,
    roles: Roles<&census::RoleCensus>,
) -> CharactersCopyReport {
    let mut daily = ctx.active[3].then(|| {
        daily_experience::STORE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    });
    if let Some(held) = daily.as_mut() {
        held.begin((ctx.base, ctx.game_state), ctx.force_full, ctx.active[5]);
        if job.experience.generation != held.generation || job.daily_training != Some(ctx.active[5])
        {
            job.unit_carry.forget();
            job.log.invalidate_read_plan();
        }
        job.daily_training = Some(ctx.active[5]);
        job.experience.forget();
        std::mem::swap(&mut job.experience, &mut **held);
    }
    let report = copy_characters_inner(memory, ctx, job, roles);
    if let Some(held) = daily.as_mut() {
        if job.log.planned_capture_succeeded() {
            job.experience.observe_prepared(&job.log);
        }
        job.experience.finish(report.raw_ok && !job.log.fault[3]);
        std::mem::swap(&mut job.experience, &mut **held);
        job.experience.generation = held.generation;
    }
    job.update_census_charge();
    job.appended_charge
        .set(collector_primitives::raw_memory::Capacity {
            auxiliary_vectors: collector_primitives::raw_memory::vector_bytes(&job.appended_roles),
            ..Default::default()
        });
    report
}

fn copy_characters_inner(
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
    let roster_token = if job.part == 0 {
        job.roster.token()
    } else {
        None
    };
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
        copy_advisor_sections(memory, ctx, job);
    } else if let Some(census) =
        loaned.filter(|_| job.leader_cache.is_some() && ctx.active == PART_MASKS[1])
    {
        let _reads = read_accounting::table(TableId::CountryLeaderRoles);
        job.clear();
        job.active = ctx.active;
        job.game_state = ctx.game_state;
        job.census_capture = true;
        job.census_historical = census.historical;
        let cache = job.leader_cache.as_mut().unwrap();
        cache.capture(
            memory,
            ctx.base,
            ctx.game_state,
            census,
            &collector_observers::character_changes::Observer,
        );
        job.leader_charge
            .set(collector_primitives::raw_memory::Capacity {
                auxiliary_vectors: cache.allocated_bytes(),
                ..Default::default()
            });
        job.leader_cached = true;
    } else if let Some(census) = loaned.filter(|_| census_part(ctx.active)) {
        copy_census(memory, ctx, job, census);
    } else if job.part == 0 && roster_part(ctx.active) {
        quiet::capture(
            memory,
            ctx,
            job,
            matches!(roles, Roles::Gather),
            roster_token,
        );
    } else {
        copy_pass(memory, ctx, job, None);
        if matches!(roles, Roles::Gather) {
            gather_role_census(memory, ctx, job);
        }
    }
    if !job.leader_cached
        && let Some(cache) = job.leader_cache.as_mut()
    {
        cache.forget();
    }
    let plan_failed = job.requires_no_retry();
    let mut copy_ticks = [0; N];
    copy_ticks[anchor(ctx.active)] = elapsed(started);
    CharactersCopyReport {
        raw_ok: !job.overflowed() && !plan_failed,
        plan_failed,
        copy_ticks,
    }
}

impl Captured {
    #[cfg(any(test, feature = "test-support"))]
    fn install_family(&mut self, active: [bool; N], results: [ReadResult; N]) {
        self.active = std::array::from_fn(|index| self.active[index] || active[index]);
        self.install_carried_family(active, results, false);
    }

    fn install_carried_family(
        &mut self,
        active: [bool; N],
        results: [ReadResult; N],
        training_unchanged: bool,
    ) {
        if active[0] {
            self.held.characters = None;
        }
        if active[2] {
            self.held.advisors = None;
        }
        if active[3] {
            self.held.units = None;
        }
        if active[4] {
            self.held.traits = None;
        }
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
            if active[5] && !training_unchanged {
                self.training_outcome = self.training_outcome();
            } else if !active[5] {
                self.training_outcome = Outcome::Unavailable {
                    reason: replay_data::cadence::SKIPPED_REASON,
                };
            }
        }
        if active[4] {
            self.traits.primary(results[4]);
        }
    }

    pub fn fail_final_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        let active = self.begin_capture(hour, force_full);
        self.fail_character_tables(active, reason);
    }

    pub fn fail_character_tables(&mut self, active: [bool; N], reason: &'static str) {
        if active[5] {
            self.training_outcome = Outcome::Unavailable { reason };
            self.raw_active[5] = false;
        }
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
            traits = 4
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn fail_characters_capture(&mut self, active: [bool; N], reason: &'static str) {
        self.active = active;
        self.training_outcome = Outcome::Unavailable {
            reason: if active[5] {
                reason
            } else {
                replay_data::cadence::SKIPPED_REASON
            },
        };
        macro_rules! fail {
            ($($field:ident = $index:literal),+ $(,)?) => {$(
                self.$field.rows.outcome = Outcome::Unavailable {
                    reason: if active[$index] { reason } else { replay_data::cadence::SKIPPED_REASON },
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
            traits = 4
        );
        self.raw_active = [false; N];
    }

    pub fn remember_templates(&mut self, job: &mut CharactersJob) {
        job.unit_carry.rebuild(
            self.units.rows.outcome,
            &self.units.rows.rows,
            self.units
                .bytes
                .first
                .get(..self.units.bytes.len)
                .unwrap_or(&[]),
            |rest| rest.first().map(|row| (1, row.native.id)),
        );
        if job.active[4] {
            self.held.traits = job
                .trait_carry
                .table_key(job.census_generation)
                .filter(|_| matches!(self.traits.rows.outcome, Outcome::Rows { .. }))
                .map(|key| {
                    (
                        key,
                        ReadResult {
                            outcome: self.traits.rows.outcome,
                            bytes_len: self.traits.bytes.len,
                            required_bytes: self.traits.bytes.required,
                        },
                    )
                });
        }
        if job.active[3] {
            self.held.units = job
                .unit_carry
                .table_key(quiet::census_generation(job))
                .filter(|_| {
                    job.roster.quiet && matches!(self.units.rows.outcome, Outcome::Rows { .. })
                })
                .map(|key| {
                    (
                        key,
                        ReadResult {
                            outcome: self.units.rows.outcome,
                            bytes_len: self.units.bytes.len,
                            required_bytes: self.units.bytes.required,
                        },
                    )
                });
        }
        let mut vectors = job.traits.vectors.iter();
        job.trait_carry.rebuild(
            self.traits.rows.outcome,
            &self.traits.rows.rows,
            self.traits
                .bytes
                .first
                .get(..self.traits.bytes.len)
                .unwrap_or(&[]),
            |_| {
                vectors.next().and_then(|vector| match vector.outcome {
                    Outcome::Rows { len } => Some((len, vector.leader)),
                    _ => None,
                })
            },
        );
        let carried = |job: &CharactersJob| collector_primitives::raw_memory::Capacity {
            auxiliary_vectors: job.carry.allocated_bytes()
                + job.published.allocated_bytes()
                + job.unit_carry.allocated_bytes()
                + job.trait_carry.allocated_bytes(),
            ..Default::default()
        };
        if !job.active[0] || job.advisor_cache.is_some() || job.advisor_dynamic.is_some() {
            job.carry_charge.set(carried(job));
            return;
        }
        let bytes = self
            .characters
            .bytes
            .first
            .get(..self.characters.bytes.len)
            .unwrap_or(&[]);
        if !job.published.matches(self.quiet_rows) {
            job.templates.rebuild(
                self.characters.rows.outcome,
                &self.characters.rows.rows,
                bytes,
            );
            match self.quiet_rows {
                Some(key) => job.published.hold(
                    key,
                    ReadResult {
                        outcome: self.characters.rows.outcome,
                        bytes_len: self.characters.bytes.len,
                        required_bytes: self.characters.bytes.required,
                    },
                    &self.characters.rows.rows,
                    bytes,
                ),
                None => job.published.forget(),
            }
        }
        job.template_charge
            .set(collector_primitives::raw_memory::Capacity {
                auxiliary_vectors: job.templates.allocated_bytes(),
                ..Default::default()
            });
        job.carry.rebuild(
            self.characters.rows.outcome,
            &self.characters.rows.rows,
            bytes,
            self.quiet_rows.is_some(),
        );
        job.carry_charge.set(carried(job));
    }

    pub fn decode_characters_payload(
        &mut self,
        base: usize,
        job: &CharactersJob,
        decode_ticks: &mut [u64; N],
    ) {
        self.active = std::array::from_fn(|index| {
            self.active[index] || self.raw_active[index] || job.active[index]
        });
        if job.active[3] {
            self.active[5] = job.active[5];
        }
        self.fail_character_tables(
            self.active.map(|value| !value),
            replay_data::cadence::SKIPPED_REASON,
        );
        self.quiet_rows = None;
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
        if job.leader_cached
            && let Some(cache) = &job.leader_cache
        {
            cache.decode(self);
            self.raw_active[1] = false;
            decode_ticks[1] += elapsed(started);
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
            let roots = collector_raw::raw::Recorded::build(&job.advisor_roots);
            let roster = roots
                .as_ref()
                .map_err(|reason| *reason)
                .and_then(|roots| ccharacter_manager::Roster::read(roots, base, job.game_state));
            let sources = roster.and_then(|roster| {
                let first = collector_raw::raw::Recorded::build(&job.log)?;
                let second = collector_raw::raw::Recorded::build(&dynamic.log)?;
                Ok((roster, [first, second]))
            });
            match sources {
                Ok((roster, recorded)) => {
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
            self.held.advisors = None;
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
        let trait_failure = (|| {
            let (row_count, byte_count) = job.traits.demand()?;
            let (carried_rows, carried_bytes) = job.trait_carry.carried_demand();
            let (row_count, byte_count) = (row_count + carried_rows, byte_count + carried_bytes);
            engine_native_memory::walk::Buffer::Growing(&mut self.traits.rows.rows).ensure(row_count)?;
            engine_native_memory::walk::Buffer::Growing(&mut self.traits.bytes.first)
                .ensure(byte_count)?;
            Ok::<_, &'static str>(())
        })()
        .err();
        fn reserve_payload<R: Default>(
            buffer: &mut Buffer<R>,
            demand: AdvisorDemand,
        ) -> Result<(), &'static str> {
            if demand.overflowed {
                return Err("character capture demand overflow");
            }
            engine_native_memory::walk::Buffer::Growing(&mut buffer.rows.rows).ensure(demand.rows)?;
            engine_native_memory::walk::Buffer::Growing(&mut buffer.bytes.first).ensure(demand.bytes)?;
            Ok(())
        }
        let quiet_rows = quiet::rows_key(base, job);
        let republish = job.published.matches(quiet_rows);
        let decoded: [bool; N] =
            std::array::from_fn(|index| job.active[index] && !(republish && index == 0));
        let payload_failures = [
            reserve_payload(&mut self.characters, job.character_demand).err(),
            reserve_payload(&mut self.units, job.unit_demand).err(),
        ];
        let leader_failure = if job.active[1] {
            engine_native_memory::walk::Buffer::Growing(&mut self.leaders.rows.rows)
                .ensure(job.leader_rows)
                .err()
        } else {
            None
        };
        let recorded = collector_raw::raw::Recorded::build(&job.log);
        let held_units = self
            .held
            .units
            .filter(|_| {
                job.roster.quiet
                    && job.unit_carry.certified()
                    && decoded[..5] == [false, false, false, true, false]
            })
            .filter(|&(key, _)| {
                job.unit_carry.table_key(quiet::census_generation(job)) == Some(key)
            })
            .filter(|_| {
                recorded
                    .as_ref()
                    .is_ok_and(|recorded| quiet::fresh_leaders_unchanged(recorded, base, job))
            })
            .map(|(_, result)| result);
        let held_traits = self
            .held
            .traits
            .filter(|_| {
                job.census_capture
                    && job.trait_carry.certified()
                    && decoded == [false, false, false, false, true, false]
            })
            .filter(|&(key, _)| job.trait_carry.table_key(job.census_generation) == Some(key))
            .filter(|_| fresh_traits_unchanged(job))
            .map(|(_, result)| result);
        let mut family = self.family().select(decoded);
        if let Some(reason) = payload_failures[0].filter(|_| decoded[0]) {
            family.characters.fail(reason);
        }
        if let Some(reason) = payload_failures[1] {
            family.units.fail(reason);
        }
        if let Some(reason) = leader_failure {
            family.leaders.fail(reason);
        }
        if let Some(reason) = trait_failure {
            family.traits.fail(reason);
        }
        if let Some(reason) = advisor_failure {
            family.advisors.fail(reason);
        }
        let results = if let Some(units) = held_units {
            let mut results = family.results();
            results[3] = units;
            results
        } else if let Some(traits) = held_traits {
            let mut results = family.results();
            results[4] = traits;
            results
        } else {
            match &recorded {
                Err(reason) => {
                    family.fail(*reason);
                    family.results()
                }
                Ok(recorded) if job.roster.quiet => quiet::walk(recorded, base, family, job),
                Ok(recorded) if job.census_capture => walk_census(
                    recorded,
                    base,
                    family,
                    Some(&job.traits),
                    job.census_visited.iter().copied(),
                    job.census_historical,
                    Carried {
                        traits: Some(&job.trait_carry),
                        ..Carried::default()
                    },
                ),
                Ok(recorded) => walk_family_carrying(
                    recorded,
                    base,
                    job.game_state,
                    family,
                    Some(&job.traits),
                    Some(&job.templates),
                    job.carry.active.then_some(&job.carry),
                ),
            }
        };
        self.install_carried_family(decoded, results, held_units.is_some());
        if republish {
            let result = if self.held.characters == quiet_rows {
                job.published.result()
            } else {
                job.published.copy_into(
                    &mut self.characters.rows.rows,
                    &mut self.characters.bytes.first,
                )
            };
            self.characters.primary(result);
        }
        self.quiet_rows =
            quiet_rows.filter(|_| matches!(self.characters.rows.outcome, Outcome::Rows { .. }));
        if job.active[0] {
            self.held.characters = self.quiet_rows;
        }
        decode_ticks[anchor(job.active)] += elapsed(started);
        for (raw, active) in self.raw_active.iter_mut().zip(job.active) {
            if active {
                *raw = false;
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn test_globals() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    census::forget_live();
    demand::forget_live();
    advisors::forget_live();
    daily_experience::STORE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .forget();
    guard
}

#[cfg(test)]
mod character_target_hint_tests;

#[cfg(any(test, feature = "test-support"))]
pub mod characters_test_kit;

#[cfg(any(test, feature = "test-support"))]
pub mod tests;

#[cfg(any(test, feature = "test-support"))]
mod shadow_fixture;
#[cfg(any(test, feature = "test-support"))]
pub use shadow_fixture::character_shadow_fixture;

#[cfg(test)]
mod shadow_tests;
