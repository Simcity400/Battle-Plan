//! Governance rows and arenas are allocated off the game thread.
//! Shared-walk costs are charged once to each READ_GROUPS leader, in QPC ticks.

use crate::{
    raw::RawLog,
    sampling::{CONSISTENCY_TARGET, qpc},
    table_capture::{Arena, CaptureRows, comparison, grow_capacity, grow_vec},
};
use engine::{
    Memory, ccombat_tactic, ccontinuous_national_focus, ccountry, cdecision_status,
    cdynamic_modifier_container, cgame_state, cnational_focus, cnational_focus_progress,
    cnational_focus_tree, cpolitical_party, cpolitical_status, cresearch_slot, ctechnology,
    ctechnology_sharing_group, ctechnology_status, ctechnology_template, cpdx_array::Array,
    msvc_string, power_balance_gfx_override, power_balance_manager, power_balance_range,
    power_balance_record, read, read_slice,
    walk::Outcome,
};
use replay::{
    frame::{TABLE_COUNT, TableView},
    rows::{self, TableId, TableSchema, tables},
};
use std::{collections::HashMap, mem::size_of};

pub const N: usize = 19;
pub const TABLES: [TableId; N] = [
    TableId::Focus,
    TableId::CompletedFocuses,
    TableId::ResearchTechnologies,
    TableId::ResearchSlots,
    TableId::TechnologyDefinitions,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::ActiveIdeas,
    TableId::DecisionCounts,
    TableId::Decisions,
    TableId::DynamicModifiers,
    TableId::PreferredTactics,
    TableId::PreferredTacticOperands,
    TableId::TechSharingGroups,
    TableId::TechSharingMembers,
    TableId::PowerBalances,
    TableId::PowerBalanceMembers,
    TableId::PowerBalanceRanges,
    TableId::PowerBalanceOverrides,
];
/// Name, local cost anchor and local members. Reports sum member ticks;
/// physically shared walks charge their anchor once and leave followers zero.
/// Country authentication is included in Focus and shared by tables 0..13.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("Focus and shared country authentication", 0, &[0, 1]),
    ("Research technologies and definitions", 2, &[2, 4]),
    ("Research slots", 3, &[3]),
    ("Politics, parties and ideas", 5, &[5, 6, 7]),
    ("Decision counts and state", 8, &[8, 9]),
    ("Dynamic modifiers", 10, &[10]),
    ("Preferred tactic and operands", 11, &[11, 12]),
    ("Technology sharing", 13, &[13, 14]),
    ("Power balance and children", 15, &[15, 16, 17, 18]),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacities {
    rows: [usize; N],
    focus_text: usize,
    definition_text: usize,
    party_text: usize,
    idea_text: usize,
    decision_text: usize,
    modifier_text: usize,
    modifier_values: usize,
    tactic_text: usize,
    balance_text: usize,
    party_countries: usize,
    idea_countries: usize,
    decision_countries: usize,
    dictionary: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: [
                512, 8192, 32768, 2048, 4096, 512, 2048, 16384, 512, 8192, 4096, 512, 512, 128,
                1024, 128, 512, 1024, 512,
            ],
            focus_text: 131072,
            definition_text: 262144,
            party_text: 32768,
            idea_text: 262144,
            decision_text: 262144,
            modifier_text: 131072,
            modifier_values: 16384,
            tactic_text: 16384,
            balance_text: 65536,
            party_countries: 2048,
            idea_countries: 16384,
            decision_countries: 8192,
            dictionary: 8192,
        }
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct CountryRow<R> {
    country: u32,
    row: R,
}
#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct TacticRow {
    row: ccombat_tactic::Row,
    token_start: usize,
}

pub struct Captured {
    focus: CaptureRows<cnational_focus_progress::Row>,
    completed: CaptureRows<cnational_focus_progress::CompletedRow>,
    technologies: CaptureRows<ctechnology::Row>,
    slots: CaptureRows<cresearch_slot::Row>,
    definitions: CaptureRows<ctechnology_template::Row>,
    politics: CaptureRows<CountryRow<cpolitical_status::Row>>,
    parties: CaptureRows<cpolitical_party::Row>,
    ideas: CaptureRows<cpolitical_status::IdeaRow>,
    counts: CaptureRows<CountryRow<cdecision_status::Counts>>,
    decisions: CaptureRows<cdecision_status::Row>,
    modifiers: CaptureRows<cdynamic_modifier_container::Row>,
    tactics: CaptureRows<TacticRow>,
    operands: CaptureRows<ccombat_tactic::WeightRow>,
    sharing: CaptureRows<ctechnology_sharing_group::Row>,
    sharing_members: CaptureRows<ctechnology_sharing_group::MemberRow>,
    balances: CaptureRows<power_balance_record::Row>,
    balance_members: CaptureRows<power_balance_record::MemberRow>,
    ranges: CaptureRows<power_balance_range::Row>,
    overrides: CaptureRows<power_balance_gfx_override::Row>,
    focus_text: Arena<u8>,
    definition_text: Arena<u8>,
    party_text: Arena<u8>,
    idea_text: Arena<u8>,
    decision_text: Arena<u8>,
    modifier_text: Arena<u8>,
    modifier_values: Arena<i64>,
    tactic_text: Arena<u8>,
    balance_text: Arena<u8>,
    party_countries: Arena<u32>,
    idea_countries: Arena<u32>,
    decision_countries: Arena<u32>,
    dictionary: Vec<ctechnology_template::Entry>,
    dictionary_second: Vec<ctechnology_template::Entry>,
    dictionary_required: usize,
    /// Raw-copy split gate for the research pilot family. Set by the game
    /// thread on refresh hours with a valid raw payload; `capture` then skips
    /// the typed pilot walks and the decode worker fills these rows before the
    /// ready handoff. Preserved across `reset` (set per refresh, cleared by
    /// the worker after decode).
    pub(crate) raw_mode: bool,
    /// Raw-copy split gate for the hourly focus pair (Focus,
    /// CompletedFocuses). Set by the game thread every hour with a valid raw
    /// payload; `capture` then skips the typed focus walks and the decode
    /// worker fills these rows before the ready handoff. Preserved across
    /// `reset` (set per hour, cleared by the worker after decode).
    pub(crate) focus_raw_mode: bool,
}

impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        let successful = group_success(successful);
        Self {
            focus: CaptureRows::new(capacity.rows[0], successful[0]),
            completed: CaptureRows::new(capacity.rows[1], successful[1]),
            technologies: CaptureRows::new(capacity.rows[2], successful[2]),
            slots: CaptureRows::new(capacity.rows[3], successful[3]),
            definitions: CaptureRows::new(capacity.rows[4], successful[4]),
            politics: CaptureRows::new(capacity.rows[5], successful[5]),
            parties: CaptureRows::new(capacity.rows[6], successful[6]),
            ideas: CaptureRows::new(capacity.rows[7], successful[7]),
            counts: CaptureRows::new(capacity.rows[8], successful[8]),
            decisions: CaptureRows::new(capacity.rows[9], successful[9]),
            modifiers: CaptureRows::new(capacity.rows[10], successful[10]),
            tactics: CaptureRows::new(capacity.rows[11], successful[11]),
            operands: CaptureRows::new(capacity.rows[12], successful[12]),
            sharing: CaptureRows::new(capacity.rows[13], successful[13]),
            sharing_members: CaptureRows::new(capacity.rows[14], successful[14]),
            balances: CaptureRows::new(capacity.rows[15], successful[15]),
            balance_members: CaptureRows::new(capacity.rows[16], successful[16]),
            ranges: CaptureRows::new(capacity.rows[17], successful[17]),
            overrides: CaptureRows::new(capacity.rows[18], successful[18]),
            focus_text: Arena::new(capacity.focus_text, successful[0]),
            definition_text: Arena::new(capacity.definition_text, successful[2]),
            party_text: Arena::new(capacity.party_text, successful[5]),
            idea_text: Arena::new(capacity.idea_text, successful[5]),
            decision_text: Arena::new(capacity.decision_text, successful[8]),
            modifier_text: Arena::new(capacity.modifier_text, successful[10]),
            modifier_values: Arena::new(capacity.modifier_values, successful[10]),
            tactic_text: Arena::new(capacity.tactic_text, successful[11]),
            balance_text: Arena::new(capacity.balance_text, successful[15]),
            party_countries: Arena::new(capacity.party_countries, successful[5]),
            idea_countries: Arena::new(capacity.idea_countries, successful[5]),
            decision_countries: Arena::new(capacity.decision_countries, successful[8]),
            dictionary: vec![ctechnology_template::Entry::default(); capacity.dictionary],
            dictionary_second: if successful[2] < CONSISTENCY_TARGET {
                vec![ctechnology_template::Entry::default(); capacity.dictionary]
            } else {
                Vec::new()
            },
            dictionary_required: 0,
            raw_mode: false,
            focus_raw_mode: false,
        }
    }
    pub fn reset(&mut self) {
        self.focus.reset();
        self.completed.reset();
        self.technologies.reset();
        self.slots.reset();
        self.definitions.reset();
        self.politics.reset();
        self.parties.reset();
        self.ideas.reset();
        self.counts.reset();
        self.decisions.reset();
        self.modifiers.reset();
        self.tactics.reset();
        self.operands.reset();
        self.sharing.reset();
        self.sharing_members.reset();
        self.balances.reset();
        self.balance_members.reset();
        self.ranges.reset();
        self.overrides.reset();
        self.focus_text.reset();
        self.definition_text.reset();
        self.party_text.reset();
        self.idea_text.reset();
        self.decision_text.reset();
        self.modifier_text.reset();
        self.modifier_values.reset();
        self.tactic_text.reset();
        self.balance_text.reset();
        self.party_countries.reset();
        self.idea_countries.reset();
        self.decision_countries.reset();
        self.dictionary_required = 0;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.focus.allocated_bytes()
            + self.completed.allocated_bytes()
            + self.technologies.allocated_bytes()
            + self.slots.allocated_bytes()
            + self.definitions.allocated_bytes()
            + self.politics.allocated_bytes()
            + self.parties.allocated_bytes()
            + self.ideas.allocated_bytes()
            + self.counts.allocated_bytes()
            + self.decisions.allocated_bytes()
            + self.modifiers.allocated_bytes()
            + self.tactics.allocated_bytes()
            + self.operands.allocated_bytes()
            + self.sharing.allocated_bytes()
            + self.sharing_members.allocated_bytes()
            + self.balances.allocated_bytes()
            + self.balance_members.allocated_bytes()
            + self.ranges.allocated_bytes()
            + self.overrides.allocated_bytes()
            + self.focus_text.allocated_bytes()
            + self.definition_text.allocated_bytes()
            + self.party_text.allocated_bytes()
            + self.idea_text.allocated_bytes()
            + self.decision_text.allocated_bytes()
            + self.modifier_text.allocated_bytes()
            + self.modifier_values.allocated_bytes()
            + self.tactic_text.allocated_bytes()
            + self.balance_text.allocated_bytes()
            + self.party_countries.allocated_bytes()
            + self.idea_countries.allocated_bytes()
            + self.decision_countries.allocated_bytes()
            + (self.dictionary.capacity() + self.dictionary_second.capacity())
                * size_of::<ctechnology_template::Entry>()
    }
    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        let successful = group_success(successful);
        self.focus.grow(&mut capacity.rows[0], successful[0]);
        self.completed.grow(&mut capacity.rows[1], successful[1]);
        grow_large(&mut self.technologies, &mut capacity.rows[2], successful[2]);
        self.slots.grow(&mut capacity.rows[3], successful[3]);
        self.definitions.grow(&mut capacity.rows[4], successful[4]);
        self.politics.grow(&mut capacity.rows[5], successful[5]);
        self.parties.grow(&mut capacity.rows[6], successful[6]);
        self.ideas.grow(&mut capacity.rows[7], successful[7]);
        self.counts.grow(&mut capacity.rows[8], successful[8]);
        self.decisions.grow(&mut capacity.rows[9], successful[9]);
        self.modifiers.grow(&mut capacity.rows[10], successful[10]);
        self.tactics.grow(&mut capacity.rows[11], successful[11]);
        self.operands.grow(&mut capacity.rows[12], successful[12]);
        self.sharing.grow(&mut capacity.rows[13], successful[13]);
        self.sharing_members
            .grow(&mut capacity.rows[14], successful[14]);
        self.balances.grow(&mut capacity.rows[15], successful[15]);
        self.balance_members
            .grow(&mut capacity.rows[16], successful[16]);
        self.ranges.grow(&mut capacity.rows[17], successful[17]);
        self.overrides.grow(&mut capacity.rows[18], successful[18]);
        capacity.party_countries = capacity.rows[6];
        capacity.idea_countries = capacity.rows[7];
        capacity.decision_countries = capacity.rows[9];
        self.focus_text
            .grow(&mut capacity.focus_text, successful[0]);
        self.definition_text
            .grow(&mut capacity.definition_text, successful[2]);
        self.party_text
            .grow(&mut capacity.party_text, successful[5]);
        self.idea_text.grow(&mut capacity.idea_text, successful[5]);
        self.decision_text
            .grow(&mut capacity.decision_text, successful[8]);
        self.modifier_text
            .grow(&mut capacity.modifier_text, successful[10]);
        self.modifier_values
            .grow(&mut capacity.modifier_values, successful[10]);
        self.tactic_text
            .grow(&mut capacity.tactic_text, successful[11]);
        self.balance_text
            .grow(&mut capacity.balance_text, successful[15]);
        self.party_countries
            .grow(&mut capacity.party_countries, successful[5]);
        self.idea_countries
            .grow(&mut capacity.idea_countries, successful[5]);
        self.decision_countries
            .grow(&mut capacity.decision_countries, successful[8]);
        grow_capacity(&mut capacity.dictionary, self.dictionary_required);
        grow_vec(&mut self.dictionary, capacity.dictionary);
        comparison(
            &mut self.dictionary_second,
            capacity.dictionary,
            successful[2],
        );
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.focus.outcome,
            self.completed.outcome,
            self.technologies.outcome,
            self.slots.outcome,
            self.definitions.outcome,
            self.politics.outcome,
            self.parties.outcome,
            self.ideas.outcome,
            self.counts.outcome,
            self.decisions.outcome,
            self.modifiers.outcome,
            self.tactics.outcome,
            self.operands.outcome,
            self.sharing.outcome,
            self.sharing_members.outcome,
            self.balances.outcome,
            self.balance_members.outcome,
            self.ranges.outcome,
            self.overrides.outcome,
        ]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [
            self.focus.mismatches,
            self.completed.mismatches,
            self.technologies.mismatches,
            self.slots.mismatches,
            self.definitions.mismatches,
            self.politics.mismatches,
            self.parties.mismatches,
            self.ideas.mismatches,
            self.counts.mismatches,
            self.decisions.mismatches,
            self.modifiers.mismatches,
            self.tactics.mismatches,
            self.operands.mismatches,
            self.sharing.mismatches,
            self.sharing_members.mismatches,
            self.balances.mismatches,
            self.balance_members.mismatches,
            self.ranges.mismatches,
            self.overrides.mismatches,
        ]
    }
    fn buffers(&mut self, second: bool) -> Buffers<'_> {
        Buffers {
            focus: select(&mut self.focus, second),
            completed: select(&mut self.completed, second),
            technologies: select(&mut self.technologies, second),
            slots: select(&mut self.slots, second),
            definitions: select(&mut self.definitions, second),
            politics: select(&mut self.politics, second),
            parties: select(&mut self.parties, second),
            ideas: select(&mut self.ideas, second),
            counts: select(&mut self.counts, second),
            decisions: select(&mut self.decisions, second),
            modifiers: select(&mut self.modifiers, second),
            tactics: select(&mut self.tactics, second),
            operands: select(&mut self.operands, second),
            sharing: select(&mut self.sharing, second),
            sharing_members: select(&mut self.sharing_members, second),
            balances: select(&mut self.balances, second),
            balance_members: select(&mut self.balance_members, second),
            ranges: select(&mut self.ranges, second),
            overrides: select(&mut self.overrides, second),
            focus_text: select_arena(&mut self.focus_text, second),
            definition_text: select_arena(&mut self.definition_text, second),
            party_text: select_arena(&mut self.party_text, second),
            idea_text: select_arena(&mut self.idea_text, second),
            decision_text: select_arena(&mut self.decision_text, second),
            modifier_text: select_arena(&mut self.modifier_text, second),
            modifier_values: select_arena(&mut self.modifier_values, second),
            tactic_text: select_arena(&mut self.tactic_text, second),
            balance_text: select_arena(&mut self.balance_text, second),
            party_countries: select_arena(&mut self.party_countries, second),
            idea_countries: select_arena(&mut self.idea_countries, second),
            decision_countries: select_arena(&mut self.decision_countries, second),
            dictionary: if second {
                &mut self.dictionary_second
            } else {
                &mut self.dictionary
            },
        }
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        needed: &[bool],
        hour: i32,
        force_full: bool,
    ) -> ([bool; N], [u64; N]) {
        self.reset();
        let mut walk: [bool; N] = std::array::from_fn(|i| {
            replay::cadence::should_walk(TABLES[i], hour, force_full)
        });
        // Raw-copy split (research pilot): on refresh hours the game thread
        // stages raw bytes via `copy_research_refresh` and the decode worker
        // fills the typed rows before the ready handoff, so both typed passes
        // skip the pilot tables here. Matched flags and costs for the pilot
        // come from the raw compare, applied by the caller after this returns.
        if self.raw_mode {
            walk[2] = false;
            walk[3] = false;
            walk[4] = false;
        }
        // Raw-copy split (hourly focus pair): every hour the game thread
        // stages raw bytes via `copy_focus_refresh` and the decode worker
        // fills the typed rows before the ready handoff, so both typed passes
        // skip the focus tables here. Matched flags and costs for the pair
        // come from the raw compare, applied by the caller after this returns.
        if self.focus_raw_mode {
            walk[0] = false;
            walk[1] = false;
        }
        let mut first = acquire(memory, base, game_state, self.buffers(false), &walk);
        self.focus.outcome = first.outcomes[0];
        self.completed.outcome = first.outcomes[1];
        self.technologies.outcome = first.outcomes[2];
        self.slots.outcome = first.outcomes[3];
        self.definitions.outcome = first.outcomes[4];
        self.politics.outcome = first.outcomes[5];
        self.parties.outcome = first.outcomes[6];
        self.ideas.outcome = first.outcomes[7];
        self.counts.outcome = first.outcomes[8];
        self.decisions.outcome = first.outcomes[9];
        self.modifiers.outcome = first.outcomes[10];
        self.tactics.outcome = first.outcomes[11];
        self.operands.outcome = first.outcomes[12];
        self.sharing.outcome = first.outcomes[13];
        self.sharing_members.outcome = first.outcomes[14];
        self.balances.outcome = first.outcomes[15];
        self.balance_members.outcome = first.outcomes[16];
        self.ranges.outcome = first.outcomes[17];
        self.overrides.outcome = first.outcomes[18];
        self.focus_text.len = first.lens[0];
        self.focus_text.required = first.required[0];
        self.definition_text.len = first.lens[1];
        self.definition_text.required = first.required[1];
        self.party_text.len = first.lens[2];
        self.party_text.required = first.required[2];
        self.idea_text.len = first.lens[3];
        self.idea_text.required = first.required[3];
        self.decision_text.len = first.lens[4];
        self.decision_text.required = first.required[4];
        self.modifier_text.len = first.lens[5];
        self.modifier_text.required = first.required[5];
        self.modifier_values.len = first.lens[6];
        self.modifier_values.required = first.required[6];
        self.tactic_text.len = first.lens[7];
        self.tactic_text.required = first.required[7];
        self.balance_text.len = first.lens[8];
        self.balance_text.required = first.required[8];
        self.party_countries.len = first.lens[9];
        self.party_countries.required = first.required[9];
        self.idea_countries.len = first.lens[10];
        self.idea_countries.required = first.required[10];
        self.decision_countries.len = first.lens[11];
        self.decision_countries.required = first.required[11];
        self.dictionary_required = first.dictionary_required;
        // Off-hours staggered tables hold last refresh; mark skipped for spool.
        // Raw-mode refreshes leave the reset outcomes for the decode worker.
        let skipped = Outcome::Unavailable {
            reason: replay::cadence::SKIPPED_REASON,
        };
        if !walk[2] && !self.raw_mode {
            self.technologies.outcome = skipped;
        }
        if !walk[3] && !self.raw_mode {
            self.slots.outcome = skipped;
        }
        if !walk[4] && !self.raw_mode {
            self.definitions.outcome = skipped;
        }
        let mut check = [false; N];
        for (_, _, group) in READ_GROUPS {
            if group.iter().any(|&i| needed[i]) && group.iter().any(|&i| walk[i]) {
                for &i in *group {
                    check[i] = walk[i];
                }
            }
        }
        // Raw mode compares raw bytes on the game thread instead of typed
        // second walks; the caller applies those results after this returns.
        if self.raw_mode {
            check[2] = false;
            check[3] = false;
            check[4] = false;
        }
        if self.focus_raw_mode {
            check[0] = false;
            check[1] = false;
        }
        let mut matched = [false; N];
        if check.iter().any(|&value| value) {
            let second = acquire(memory, base, game_state, self.buffers(true), &check);
            for (time, other) in first.ticks.iter_mut().zip(second.ticks) {
                *time = time.saturating_add(other);
            }
            self.dictionary_required = self.dictionary_required.max(second.dictionary_required);
            if check[0] {
                let compare_start = qpc();
                let arena_0 = self.focus_text.compare(second.lens[0], second.required[0]);
                if needed[0] {
                    self.focus.checked = true;
                    matched[0] = self.focus.compare(second.outcomes[0]);
                    if !(arena_0) {
                        self.focus.mismatches += 1;
                        matched[0] = false;
                    }
                }
                if needed[1] {
                    self.completed.checked = true;
                    matched[1] = self.completed.compare(second.outcomes[1]);
                    if !(arena_0) {
                        self.completed.mismatches += 1;
                        matched[1] = false;
                    }
                }
                first.ticks[0] = first.ticks[0].saturating_add(elapsed(compare_start));
            }
            if check[2] {
                let compare_start = qpc();
                let arena_1 = self
                    .definition_text
                    .compare(second.lens[1], second.required[1]);
                if needed[2] {
                    self.technologies.checked = true;
                    matched[2] = self.technologies.compare(second.outcomes[2]);
                }
                if needed[4] {
                    self.definitions.checked = true;
                    matched[4] = self.definitions.compare(second.outcomes[4]);
                    if !(arena_1) {
                        self.definitions.mismatches += 1;
                        matched[4] = false;
                    }
                }
                first.ticks[2] = first.ticks[2].saturating_add(elapsed(compare_start));
            }
            if check[3] {
                let compare_start = qpc();
                if needed[3] {
                    self.slots.checked = true;
                    matched[3] = self.slots.compare(second.outcomes[3]);
                }
                first.ticks[3] = first.ticks[3].saturating_add(elapsed(compare_start));
            }
            if check[5] {
                let compare_start = qpc();
                let arena_2 = self.party_text.compare(second.lens[2], second.required[2]);
                let arena_9 = self
                    .party_countries
                    .compare(second.lens[9], second.required[9]);
                let arena_3 = self.idea_text.compare(second.lens[3], second.required[3]);
                let arena_10 = self
                    .idea_countries
                    .compare(second.lens[10], second.required[10]);
                if needed[5] {
                    self.politics.checked = true;
                    matched[5] = self.politics.compare(second.outcomes[5]);
                }
                if needed[6] {
                    self.parties.checked = true;
                    matched[6] = self.parties.compare(second.outcomes[6]);
                    if !(arena_2 && arena_9) {
                        self.parties.mismatches += 1;
                        matched[6] = false;
                    }
                }
                if needed[7] {
                    self.ideas.checked = true;
                    matched[7] = self.ideas.compare(second.outcomes[7]);
                    if !(arena_3 && arena_10) {
                        self.ideas.mismatches += 1;
                        matched[7] = false;
                    }
                }
                first.ticks[5] = first.ticks[5].saturating_add(elapsed(compare_start));
            }
            if check[8] {
                let compare_start = qpc();
                let arena_4 = self
                    .decision_text
                    .compare(second.lens[4], second.required[4]);
                let arena_11 = self
                    .decision_countries
                    .compare(second.lens[11], second.required[11]);
                if needed[8] {
                    self.counts.checked = true;
                    matched[8] = self.counts.compare(second.outcomes[8]);
                }
                if needed[9] {
                    self.decisions.checked = true;
                    matched[9] = self.decisions.compare(second.outcomes[9]);
                    if !(arena_4 && arena_11) {
                        self.decisions.mismatches += 1;
                        matched[9] = false;
                    }
                }
                first.ticks[8] = first.ticks[8].saturating_add(elapsed(compare_start));
            }
            if check[10] {
                let compare_start = qpc();
                let arena_5 = self
                    .modifier_text
                    .compare(second.lens[5], second.required[5]);
                let arena_6 = self
                    .modifier_values
                    .compare(second.lens[6], second.required[6]);
                if needed[10] {
                    self.modifiers.checked = true;
                    matched[10] = self.modifiers.compare(second.outcomes[10]);
                    if !(arena_5 && arena_6) {
                        self.modifiers.mismatches += 1;
                        matched[10] = false;
                    }
                }
                first.ticks[10] = first.ticks[10].saturating_add(elapsed(compare_start));
            }
            if check[11] {
                let compare_start = qpc();
                let arena_7 = self.tactic_text.compare(second.lens[7], second.required[7]);
                if needed[11] {
                    self.tactics.checked = true;
                    matched[11] = self.tactics.compare(second.outcomes[11]);
                    if !(arena_7) {
                        self.tactics.mismatches += 1;
                        matched[11] = false;
                    }
                }
                if needed[12] {
                    self.operands.checked = true;
                    matched[12] = self.operands.compare(second.outcomes[12]);
                }
                first.ticks[11] = first.ticks[11].saturating_add(elapsed(compare_start));
            }
            if check[13] {
                let compare_start = qpc();
                if needed[13] {
                    self.sharing.checked = true;
                    matched[13] = self.sharing.compare(second.outcomes[13]);
                }
                if needed[14] {
                    self.sharing_members.checked = true;
                    matched[14] = self.sharing_members.compare(second.outcomes[14]);
                }
                first.ticks[13] = first.ticks[13].saturating_add(elapsed(compare_start));
            }
            if check[15] {
                let compare_start = qpc();
                let arena_8 = self
                    .balance_text
                    .compare(second.lens[8], second.required[8]);
                if needed[15] {
                    self.balances.checked = true;
                    matched[15] = self.balances.compare(second.outcomes[15]);
                    if !(arena_8) {
                        self.balances.mismatches += 1;
                        matched[15] = false;
                    }
                }
                if needed[16] {
                    self.balance_members.checked = true;
                    matched[16] = self.balance_members.compare(second.outcomes[16]);
                }
                if needed[17] {
                    self.ranges.checked = true;
                    matched[17] = self.ranges.compare(second.outcomes[17]);
                    if !(arena_8) {
                        self.ranges.mismatches += 1;
                        matched[17] = false;
                    }
                }
                if needed[18] {
                    self.overrides.checked = true;
                    matched[18] = self.overrides.compare(second.outcomes[18]);
                    if !(arena_8) {
                        self.overrides.mismatches += 1;
                        matched[18] = false;
                    }
                }
                first.ticks[15] = first.ticks[15].saturating_add(elapsed(compare_start));
            }
        }
        (matched, first.ticks)
    }
}

fn select<R>(rows: &mut CaptureRows<R>, second: bool) -> &mut [R] {
    if second {
        &mut rows.second
    } else {
        &mut rows.rows
    }
}
fn select_arena<T>(arena: &mut Arena<T>, second: bool) -> &mut [T] {
    if second {
        &mut arena.second
    } else {
        &mut arena.first
    }
}
fn elapsed(start: i64) -> u64 {
    u64::try_from(qpc().saturating_sub(start)).unwrap_or(0)
}
fn group_success(successful: &[u16]) -> [u16; N] {
    let mut result = [0; N];
    for (_, _, group) in READ_GROUPS {
        let count = group.iter().map(|&i| successful[i]).min().unwrap();
        for &i in *group {
            result[i] = count;
        }
    }
    result
}
fn grow_large<R: Default + Clone + PartialEq>(
    rows: &mut CaptureRows<R>,
    capacity: &mut usize,
    successful: u16,
) {
    let required = match rows.outcome {
        Outcome::Rows { len } => len,
        Outcome::Truncated { len, unread_rows } => len.saturating_add(unread_rows as usize),
        Outcome::Unavailable { .. } => 0,
    };
    if required > capacity.saturating_sub(*capacity / 4) {
        *capacity = (*capacity).max(required.saturating_add(required.div_ceil(3)));
    }
    grow_vec(&mut rows.rows, *capacity);
    comparison(&mut rows.second, *capacity, successful);
}
fn append(total: &mut Outcome, next: Outcome) {
    match (*total, next) {
        (Outcome::Unavailable { .. }, _) => {}
        (_, Outcome::Unavailable { reason }) => *total = Outcome::Unavailable { reason },
        (first, second) => {
            let missing = |o| {
                if let Outcome::Truncated { unread_rows, .. } = o {
                    unread_rows
                } else {
                    0_u32
                }
            };
            *total = match missing(first).checked_add(missing(second)) {
                Some(0) => Outcome::Rows {
                    len: first.len() + second.len(),
                },
                Some(unread_rows) => Outcome::Truncated {
                    len: first.len() + second.len(),
                    unread_rows,
                },
                None => Outcome::Unavailable {
                    reason: "governance omitted row count overflow",
                },
            };
        }
    }
}
fn one<R: Copy>(output: &mut [R], value: R) -> Outcome {
    if let Some(row) = output.first_mut() {
        *row = value;
        Outcome::Rows { len: 1 }
    } else {
        Outcome::Truncated {
            len: 0,
            unread_rows: 1,
        }
    }
}
struct Buffers<'a> {
    focus: &'a mut [cnational_focus_progress::Row],
    completed: &'a mut [cnational_focus_progress::CompletedRow],
    technologies: &'a mut [ctechnology::Row],
    slots: &'a mut [cresearch_slot::Row],
    definitions: &'a mut [ctechnology_template::Row],
    politics: &'a mut [CountryRow<cpolitical_status::Row>],
    parties: &'a mut [cpolitical_party::Row],
    ideas: &'a mut [cpolitical_status::IdeaRow],
    counts: &'a mut [CountryRow<cdecision_status::Counts>],
    decisions: &'a mut [cdecision_status::Row],
    modifiers: &'a mut [cdynamic_modifier_container::Row],
    tactics: &'a mut [TacticRow],
    operands: &'a mut [ccombat_tactic::WeightRow],
    sharing: &'a mut [ctechnology_sharing_group::Row],
    sharing_members: &'a mut [ctechnology_sharing_group::MemberRow],
    balances: &'a mut [power_balance_record::Row],
    balance_members: &'a mut [power_balance_record::MemberRow],
    ranges: &'a mut [power_balance_range::Row],
    overrides: &'a mut [power_balance_gfx_override::Row],
    focus_text: &'a mut [u8],
    definition_text: &'a mut [u8],
    party_text: &'a mut [u8],
    idea_text: &'a mut [u8],
    decision_text: &'a mut [u8],
    modifier_text: &'a mut [u8],
    modifier_values: &'a mut [i64],
    tactic_text: &'a mut [u8],
    balance_text: &'a mut [u8],
    party_countries: &'a mut [u32],
    idea_countries: &'a mut [u32],
    decision_countries: &'a mut [u32],
    dictionary: &'a mut [ctechnology_template::Entry],
}
struct Observation {
    outcomes: [Outcome; N],
    ticks: [u64; N],
    lens: [usize; 12],
    required: [usize; 12],
    dictionary_required: usize,
}
impl Observation {
    fn new() -> Self {
        Self {
            outcomes: [Outcome::Rows { len: 0 }; N],
            ticks: [0; N],
            lens: [0; 12],
            required: [0; 12],
            dictionary_required: 0,
        }
    }
    fn arena(&mut self, index: usize, len: usize, required: usize) {
        self.lens[index] += len;
        self.required[index] = self.required[index].saturating_add(required);
    }
}

fn acquire(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    b: Buffers<'_>,
    enabled: &[bool; N],
) -> Observation {
    let mut o = Observation::new();
    let started = qpc();
    let mut definitions = ctechnology_template::Dictionary::new(
        if enabled[2] { b.dictionary } else { &mut [] },
        if enabled[2] {
            b.definition_text
        } else {
            &mut []
        },
    );
    if enabled[2] {
        o.ticks[2] += elapsed(started);
    }
    let started = qpc();
    let tactic_context = if enabled[11] {
        ccombat_tactic::WeightContext::read(memory, base)
    } else {
        Err("tactic weight not sampled")
    };
    if enabled[11] {
        o.ticks[11] += elapsed(started);
    }
    if enabled[..13].iter().any(|&value| value) {
        let started = qpc();
        let countries = ccountry::Reader::new(memory, base, game_state);
        o.ticks[0] += elapsed(started);
        match countries {
            Err(reason) => {
                for (outcome, &active) in o.outcomes[..13].iter_mut().zip(&enabled[..13]) {
                    if active {
                        *outcome = Outcome::Unavailable { reason };
                    }
                }
            }
            Ok(countries) => {
                for slot in 1..countries.count() {
                    let started = qpc();
                    let country = countries.at(slot);
                    o.ticks[0] += elapsed(started);
                    let country = match country {
                        Ok(Some(country)) => country,
                        Ok(None) => continue,
                        Err(reason) => {
                            for (outcome, &active) in
                                o.outcomes[..13].iter_mut().zip(&enabled[..13])
                            {
                                if active {
                                    *outcome = Outcome::Unavailable { reason };
                                }
                            }
                            break;
                        }
                    };
                    if enabled[0] {
                        let started = qpc();
                        let first = o.outcomes[0].len();
                        let completed = o.outcomes[1].len();
                        let text = o.lens[0];
                        let result = cnational_focus_progress::walk(
                            memory,
                            base,
                            country.pointer,
                            country.id,
                            &mut b.focus[first..],
                            &mut b.completed[completed..],
                            &mut b.focus_text[text..],
                        );
                        for row in &mut b.focus[first..first + result.outcome.len()] {
                            for span in [&mut row.tree, &mut row.current, &mut row.continuous]
                                .into_iter()
                                .flatten()
                            {
                                span.start += text;
                            }
                        }
                        for row in
                            &mut b.completed[completed..completed + result.completed_outcome.len()]
                        {
                            row.token.start += text;
                        }
                        append(&mut o.outcomes[0], result.outcome);
                        append(&mut o.outcomes[1], result.completed_outcome);
                        o.arena(0, result.bytes_len, result.required_bytes);
                        o.ticks[0] += elapsed(started);
                    }
                    if enabled[2] {
                        let started = qpc();
                        let first = o.outcomes[2].len();
                        let result = ctechnology_status::walk_technologies(
                            memory,
                            base,
                            country.pointer,
                            country.id,
                            &mut b.technologies[first..],
                            &mut definitions,
                        );
                        append(&mut o.outcomes[2], result);
                        o.ticks[2] += elapsed(started);
                    }
                    if enabled[3] {
                        let started = qpc();
                        let first = o.outcomes[3].len();
                        let result = ctechnology_status::walk_slots(
                            memory,
                            base,
                            country.pointer,
                            country.id,
                            &mut b.slots[first..],
                        );
                        append(&mut o.outcomes[3], result);
                        o.ticks[3] += elapsed(started);
                    }
                    if enabled[5] {
                        let started = qpc();
                        match cpolitical_status::Reader::new(memory, base, country.pointer) {
                            Err(reason) => {
                                for index in 5..8 {
                                    o.outcomes[index] = Outcome::Unavailable { reason };
                                }
                            }
                            Ok(reader) => {
                                let first = o.outcomes[5].len();
                                let result = reader
                                    .row()
                                    .map(|row| {
                                        one(
                                            &mut b.politics[first..],
                                            CountryRow {
                                                country: country.id,
                                                row,
                                            },
                                        )
                                    })
                                    .unwrap_or_else(|reason| Outcome::Unavailable { reason });
                                append(&mut o.outcomes[5], result);
                                let first = o.outcomes[6].len();
                                let text = o.lens[2];
                                let capacity = b.parties.len().min(b.party_countries.len());
                                let result = reader.parties(
                                    &mut b.parties[first..capacity],
                                    &mut b.party_text[text..],
                                );
                                for row in &mut b.parties[first..first + result.outcome.len()] {
                                    row.token_start += text;
                                }
                                b.party_countries[first..first + result.outcome.len()]
                                    .fill(country.id);
                                append(&mut o.outcomes[6], result.outcome);
                                o.arena(2, result.bytes_len, result.required_bytes);
                                let first = o.outcomes[7].len();
                                let text = o.lens[3];
                                let capacity = b.ideas.len().min(b.idea_countries.len());
                                let result = reader
                                    .ideas(&mut b.ideas[first..capacity], &mut b.idea_text[text..]);
                                for row in &mut b.ideas[first..first + result.outcome.len()] {
                                    row.token_start += text;
                                }
                                b.idea_countries[first..first + result.outcome.len()]
                                    .fill(country.id);
                                append(&mut o.outcomes[7], result.outcome);
                                o.arena(3, result.bytes_len, result.required_bytes);
                            }
                        }
                        o.ticks[5] += elapsed(started);
                    }
                    if enabled[8] {
                        let started = qpc();
                        let first = o.outcomes[9].len();
                        let text = o.lens[4];
                        let capacity = b.decisions.len().min(b.decision_countries.len());
                        let result = cdecision_status::walk(
                            memory,
                            base,
                            country.pointer,
                            &mut b.decisions[first..capacity],
                            &mut b.decision_text[text..],
                        );
                        for row in &mut b.decisions[first..first + result.outcome.len()] {
                            row.token_start += text;
                        }
                        b.decision_countries[first..first + result.outcome.len()].fill(country.id);
                        let counts = if let Some(row) = result.counts {
                            let first = o.outcomes[8].len();
                            one(
                                &mut b.counts[first..],
                                CountryRow {
                                    country: country.id,
                                    row,
                                },
                            )
                        } else {
                            Outcome::Unavailable {
                                reason: "decision source counts unavailable",
                            }
                        };
                        append(&mut o.outcomes[8], counts);
                        append(&mut o.outcomes[9], result.outcome);
                        o.arena(4, result.bytes_len, result.required_bytes);
                        o.ticks[8] += elapsed(started);
                    }
                    if enabled[10] {
                        let started = qpc();
                        let first = o.outcomes[10].len();
                        let text = o.lens[5];
                        let values = o.lens[6];
                        let result = cdynamic_modifier_container::walk(
                            memory,
                            base,
                            country.pointer,
                            country.id,
                            &mut b.modifiers[first..],
                            &mut b.modifier_text[text..],
                            &mut b.modifier_values[values..],
                        );
                        for row in &mut b.modifiers[first..first + result.outcome.len()] {
                            row.token_start += text;
                            row.value_start += values;
                        }
                        append(&mut o.outcomes[10], result.outcome);
                        o.arena(5, result.token_bytes, result.required_token_bytes);
                        o.arena(6, result.values_len, result.required_values);
                        o.ticks[10] += elapsed(started);
                    }
                    if enabled[11] {
                        let started = qpc();
                        let text = o.lens[7];
                        let selected = ccombat_tactic::preferred(
                            memory,
                            base,
                            country.pointer,
                            country.id,
                            &mut b.tactic_text[text..],
                        );
                        let result = match selected.outcome {
                            Outcome::Rows { .. } => {
                                let first = o.outcomes[11].len();
                                o.arena(7, selected.row.token_len, selected.required_token_bytes);
                                one(
                                    &mut b.tactics[first..],
                                    TacticRow {
                                        row: selected.row,
                                        token_start: text,
                                    },
                                )
                            }
                            other => {
                                o.arena(7, 0, selected.required_token_bytes);
                                other
                            }
                        };
                        append(&mut o.outcomes[11], result);
                        // The context is created once below, before entering this loop.
                        let result = match tactic_context {
                            Ok(context) => match ccombat_tactic::weight(
                                memory,
                                country.pointer,
                                &selected,
                                context,
                            ) {
                                Ok(Some(row)) => {
                                    let first = o.outcomes[12].len();
                                    one(&mut b.operands[first..], row)
                                }
                                Ok(None) => Outcome::Rows { len: 0 },
                                Err(reason) => Outcome::Unavailable { reason },
                            },
                            Err(reason) => Outcome::Unavailable { reason },
                        };
                        append(&mut o.outcomes[12], result);
                        o.ticks[11] += elapsed(started);
                    }
                }
            }
        }
    }
    if enabled[2] {
        let started = qpc();
        o.lens[1] = definitions.bytes_len;
        o.required[1] = definitions.required_bytes;
        o.dictionary_required = definitions.required_capacity;
        let mut copied = Outcome::Rows { len: 0 };
        for entry in definitions.entries.iter().filter(|entry| entry.copied) {
            let first = copied.len();
            append(&mut copied, one(&mut b.definitions[first..], entry.row));
        }
        o.outcomes[4] = match (o.outcomes[2], definitions.outcome(), copied) {
            (Outcome::Rows { .. }, Outcome::Rows { .. }, output) => output,
            (Outcome::Rows { .. }, Outcome::Truncated { unread_rows, .. }, mut output) => {
                append(
                    &mut output,
                    Outcome::Truncated {
                        len: 0,
                        unread_rows,
                    },
                );
                output
            }
            (_, Outcome::Unavailable { reason }, _) => Outcome::Unavailable { reason },
            _ => Outcome::Unavailable {
                reason: "technology definition source census incomplete",
            },
        };
        o.ticks[2] += elapsed(started);
    }
    o.lens[9] = o.outcomes[6].len();
    o.required[9] = required_rows(o.outcomes[6]);
    o.lens[10] = o.outcomes[7].len();
    o.required[10] = required_rows(o.outcomes[7]);
    o.lens[11] = o.outcomes[9].len();
    o.required[11] = required_rows(o.outcomes[9]);
    if enabled[13] {
        let started = qpc();
        let result =
            ctechnology_sharing_group::walk(memory, base, game_state, b.sharing, b.sharing_members);
        o.outcomes[13] = result.groups;
        o.outcomes[14] = result.members;
        o.ticks[13] += elapsed(started);
    }
    if enabled[15] {
        let started = qpc();
        let result = power_balance_manager::walk(
            memory,
            base,
            game_state,
            power_balance_manager::Buffers {
                records: b.balances,
                members: b.balance_members,
                ranges: b.ranges,
                overrides: b.overrides,
                text: b.balance_text,
            },
        );
        o.outcomes[15] = result.records;
        o.outcomes[16] = result.members;
        o.outcomes[17] = result.ranges;
        o.outcomes[18] = result.overrides;
        o.lens[8] = result.text_len;
        o.required[8] = result.required_text;
        o.ticks[15] += elapsed(started);
    }
    o
}
fn required_rows(outcome: Outcome) -> usize {
    match outcome {
        Outcome::Rows { len } => len,
        Outcome::Truncated { len, unread_rows } => len.saturating_add(unread_rows as usize),
        Outcome::Unavailable { .. } => 0,
    }
}

// Test-only re-export so the decode worker tests can reuse the same harness.
#[cfg(test)]
pub(crate) use tests::research_test_kit;

// Test-only re-export so the decode worker tests can reuse the focus harness.
#[cfg(test)]
pub(crate) use tests::focus_test_kit;

// ===========================================================================
// Research copy/decode split (pilot family: technologies, slots, definitions).
//
// Game thread (`Captured::copy_research_refresh`): raw copies only. It uses
// the factored engine `copy_*` helpers (`copy_status`, `copy_one`,
// `copy_header`, `copy_raw`) plus plain roster-structure reads (array
// headers, pointer pages, id bytes) to discover what to copy. No vtable,
// backlink, handle, ordering or string validation happens here; discovery
// interprets only already-copied bytes to find the next spans. Every span is
// recorded with its success flag, so decode reproduces identical faults with
// no live access. Template headers repeat per occurrence (no game-thread
// dedup); the decode side deduplicates by pointer exactly like `Dictionary`.
// Instance blobs carry their own validation bytes, so decode needs no roster
// walks at all.
//
// Decode worker (`Captured::decode_research_payload`): replays the same
// validations as `acquire` through the factored `decode_*` helpers
// (`decode_status`, `decode_one`, `decode_header`, `decode_raw`,
// `Array::from_header`) into the identical typed rows, outcomes, arena bytes
// and dictionary accounting. Unavailable outcomes discard rows exactly like
// the typed walk, so any recorded fault short-circuits to the same reason.
//
// Double reads: the game thread takes two raw copies and compares them
// partition by partition (technologies, slots, definitions). Equal decodes
// once; mismatch takes the existing mismatch path (checked/mismatches on the
// same `CaptureRows`, matched stays false). The 200-check windows still count
// refreshes only: skip hours never reach the compare and matched stays false.
// Conservative rule: any live failure inside a partition blocks that
// partition's check even when both copies agree, so same-hour tearing never
// advances the windows. A readable-but-invalid refresh (deterministic fault,
// both copies equal) still counts its check while emitting the identical
// Unavailable outcome; such faults do not occur with the pinned build and
// live calibration will confirm.
// ===========================================================================

/// Pilot local indices in calibration/report order: technologies, slots,
/// definitions.
pub(crate) const RESEARCH_LOCAL: [usize; 3] = [2, 3, 4];

/// One template occurrence in instance-walk order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TemplateOcc {
    pub pointer: usize,
    pub handle: u32,
    pub header: [u8; ctechnology_template::HEADER_BYTES],
    pub header_result: Result<usize, &'static str>,
    pub id_start: usize,
    pub id_len: usize,
    pub id_ok: bool,
    pub term_ok: bool,
    pub term_overflow: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlotOcc {
    pub ptr: u64,
    pub ptr_ok: bool,
    pub raw: cresearch_slot::Raw,
    pub raw_result: Result<usize, &'static str>,
    pub linked: [u8; ctechnology::INSTANCE_BYTES],
    pub linked_completed: usize,
    pub has_linked: bool,
}

/// Fixed-size per-country copy metadata. Blobs live in the job-level pools
/// addressed by the start/len ranges below.
#[derive(Debug)]
pub(crate) struct CountryMeta {
    pub country_id: u32,
    pub country_pointer: usize,
    pub status_pointer: usize,
    pub status_vtable: [u8; 8],
    pub status_backlink: [u8; 8],
    pub status_scattered: usize,
    pub status_result: Result<(), &'static str>,
    pub tech_header: [u8; 16],
    pub tech_header_ok: bool,
    pub tech_start: usize,
    pub tech_len: usize,
    pub tech_unread: u32,
    pub tech_err: Option<&'static str>,
    pub researched_header: [u8; 16],
    pub researched_header_ok: bool,
    pub researched_start: usize,
    pub researched_len: usize,
    pub researched_err: Option<&'static str>,
    pub tmpl_start: usize,
    pub tmpl_len: usize,
    pub slot_header: [u8; 16],
    pub slot_header_ok: bool,
    pub slot_start: usize,
    pub slot_len: usize,
    pub slot_unread: u32,
    pub slot_err: Option<&'static str>,
}

impl Default for CountryMeta {
    fn default() -> Self {
        Self {
            country_id: 0,
            country_pointer: 0,
            status_pointer: 0,
            status_vtable: [0; 8],
            status_backlink: [0; 8],
            status_scattered: 0,
            status_result: Ok(()),
            tech_header: [0; 16],
            tech_header_ok: false,
            tech_start: 0,
            tech_len: 0,
            tech_unread: 0,
            tech_err: None,
            researched_header: [0; 16],
            researched_header_ok: false,
            researched_start: 0,
            researched_len: 0,
            researched_err: None,
            tmpl_start: 0,
            tmpl_len: 0,
            slot_header: [0; 16],
            slot_header_ok: false,
            slot_start: 0,
            slot_len: 0,
            slot_unread: 0,
            slot_err: None,
        }
    }
}

/// One refresh of raw research payload. Flat pools keep every per-item struct
/// fixed-size so `clear` recycles capacities and the steady state allocates
/// on neither thread; growth happens on the decode side only (see
/// `crate::decode::recycle_job`).
#[derive(Debug, Default)]
pub(crate) struct ResearchJob {
    pub metas: Vec<CountryMeta>,
    pub tech_pool: Vec<u8>,
    pub researched_pool: Vec<u8>,
    pub templates: Vec<TemplateOcc>,
    /// First admitted occurrence per native handle, reset for every pass.
    /// Handles beyond this acceleration table use the same uncached path.
    template_index: Vec<usize>,
    template_seen_country: Vec<usize>,
    pub id_pool: Vec<u8>,
    pub slots: Vec<SlotOcc>,
    pub reader_err: Option<&'static str>,
    pub country_err: Option<&'static str>,
    pub second_max_handle_plus_one: usize,
    pub second_id_bytes: usize,
    pub overflowed: bool,
}

impl ResearchJob {
    pub fn new() -> Self {
        Self {
            metas: Vec::with_capacity(512),
            tech_pool: Vec::with_capacity(16 << 20),
            researched_pool: Vec::with_capacity(8 << 20),
            templates: Vec::with_capacity(1 << 18),
            template_index: vec![usize::MAX; 1 << 16],
            template_seen_country: vec![0; 1 << 16],
            id_pool: Vec::with_capacity(8 << 20),
            slots: Vec::with_capacity(4096),
            reader_err: None,
            country_err: None,
            second_max_handle_plus_one: 0,
            second_id_bytes: 0,
            overflowed: false,
        }
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.metas.clear();
        self.tech_pool.clear();
        self.researched_pool.clear();
        self.templates.clear();
        self.template_index.fill(usize::MAX);
        self.template_seen_country.fill(0);
        self.id_pool.clear();
        self.slots.clear();
        self.reader_err = None;
        self.country_err = None;
        self.second_max_handle_plus_one = 0;
        self.second_id_bytes = 0;
        self.overflowed = false;
    }

    pub fn grow_for_next(&self) -> bool {
        self.overflowed
    }

    /// Double every pool off the game thread until copies fit. Converges like
    /// the typed capacity growth; overflowed refreshes fall back to the typed
    /// walk in the meantime, so sizing is a pure performance knob.
    pub fn grow(&mut self) {
        fn double<T>(rows: &mut Vec<T>) {
crate::raw::double_capacity(rows);
        }
        double(&mut self.metas);
        double(&mut self.tech_pool);
        double(&mut self.researched_pool);
        double(&mut self.templates);
        double(&mut self.id_pool);
        double(&mut self.slots);
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct CopyReport {
    /// False when staging overflowed: the caller falls back to the typed walk
    /// (identical rows through the existing path) and enqueues no payload.
    pub raw_ok: bool,
    /// Per-partition raw equality gated by live-fault conservatism, in
    /// [technologies, slots, definitions] order.
    pub matched: [bool; 3],
    pub copy_ticks: [u64; 3],
    pub raw_bytes: [u64; 3],
}

/// Game-thread copy parameters, bundled to keep the refresh entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CopyContext {
    pub base: usize,
    pub game_state: usize,
    pub tech_cap: usize,
    pub slot_cap: usize,
    pub needed: [bool; TABLE_COUNT],
}

#[derive(Debug, Default)]
struct PassTallies {
    ticks: [u64; 3],
    bytes: [u64; 3],
    live_fault: [bool; 3],
    max_handle_plus_one: usize,
    id_bytes_attempted: usize,
}

fn fit_len(used: usize, capacity: usize, need: usize) -> bool {
    need <= capacity.saturating_sub(used)
}

/// Technology handle u32 inside a copied instance blob. Layout pinned by
/// `ctechnology::FIELDS[0]` (12 bytes at destination 0: vtable then handle).
fn blob_handle(blob: &[u8]) -> u32 {
    u32::from_le_bytes(blob[8..12].try_into().unwrap())
}

/// Template pointer u64 inside a copied instance blob. Layout pinned by
/// `ctechnology::FIELDS[1]` (16 bytes at destination 12: status then template).
fn blob_template(blob: &[u8]) -> u64 {
    u64::from_le_bytes(blob[20..28].try_into().unwrap())
}

/// Best-effort id discovery mirroring `decode_header`'s string section, used
/// only to find the id spans. Any inconsistency means decode will fail from
/// the header bytes first, so no id spans are needed.
fn template_id_source(pointer: usize, header: &[u8; 56]) -> Option<(usize, usize)> {
    let string = &header[..32];
    let id_len = usize::try_from(u64::from_le_bytes(string[16..24].try_into().unwrap())).ok()?;
    let capacity = u64::from_le_bytes(string[24..32].try_into().unwrap());
    if id_len == 0
        || id_len as u64 > capacity
        || capacity < engine::msvc_string::INLINE_CAPACITY
    {
        return None;
    }
    let source = if capacity == engine::msvc_string::INLINE_CAPACITY {
        pointer.checked_add(ctechnology_template::ID)?
    } else {
        usize::try_from(u64::from_le_bytes(string[..8].try_into().unwrap()))
            .ok()
            .filter(|&pointer| pointer != 0)?
    };
    Some((source, id_len))
}

fn record_template_occ(
    memory: &impl Memory,
    pointer: usize,
    handle: u32,
    job: &mut ResearchJob,
    tallies: &mut PassTallies,
) -> bool {
    if job.templates.len() == job.templates.capacity() {
        return true;
    }
    if let Some(&index) = job.template_index.get(handle as usize)
        && let Some(cached) = job.templates.get(index)
    {
        // Dictionary::read_definition reads a handle once, including failed
        // definitions; a different pointer is rejected by the decoder.
        let mut occ = cached.clone();
        occ.pointer = pointer;
        job.templates.push(occ);
        return false;
    }
    let mut header = [0; ctechnology_template::HEADER_BYTES];
    let header_result = ctechnology_template::copy_header(memory, pointer, &mut header);
    let mut occ = TemplateOcc {
        pointer,
        handle,
        header,
        header_result,
        id_start: job.id_pool.len(),
        id_len: 0,
        id_ok: true,
        term_ok: true,
        term_overflow: false,
    };
    if header_result == Ok(3)
        && let Some((source, id_len)) = template_id_source(pointer, &header)
    {
        tallies.id_bytes_attempted = tallies.id_bytes_attempted.saturating_add(id_len);
        if !fit_len(job.id_pool.len(), job.id_pool.capacity(), id_len) {
            return true;
        }
        occ.id_start = job.id_pool.len();
        occ.id_len = id_len;
        let base = job.id_pool.len();
        job.id_pool.resize(base + id_len, 0);
        occ.id_ok = read_slice(memory, source, &mut job.id_pool[base..base + id_len]);
        if !occ.id_ok {
            tallies.live_fault[2] = true;
        }
        match source.checked_add(id_len) {
            Some(terminator) => match read::<u8>(memory, terminator) {
                Some(0) => {}
                Some(_) => occ.term_ok = false,
                None => {
                    occ.term_ok = false;
                    tallies.live_fault[2] = true;
                }
            },
            None => occ.term_overflow = true,
        }
    }
    tallies.bytes[2] += (ctechnology_template::HEADER_BYTES + occ.id_len) as u64;
    if let Some(index) = job.template_index.get_mut(handle as usize) {
        *index = job.templates.len();
    }
    job.templates.push(occ);
    false
}

fn copy_country_tech(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
    tech_cap: usize,
    job: &mut ResearchJob,
    meta: &mut CountryMeta,
    tallies: &mut PassTallies,
) -> bool {
    meta.country_id = country.id;
    meta.country_pointer = country.pointer;
    match ctechnology_status::copy_status(memory, country.pointer) {
        Err(reason) => {
            meta.status_result = Err(reason);
            tallies.live_fault[0] = true;
            tallies.bytes[0] += 8;
            return false;
        }
        Ok(raw) => {
            meta.status_pointer = raw.pointer;
            meta.status_vtable = raw.vtable;
            meta.status_backlink = raw.backlink;
            meta.status_scattered = raw.scattered;
            meta.status_result = Ok(());
            tallies.bytes[0] += 24;
            if raw.scattered != 2 {
                tallies.live_fault[0] = true;
            }
        }
    }
    let status = meta.status_pointer;
    let tech_addr = match status.checked_add(ctechnology_status::TECHNOLOGIES_DATA) {
        Some(address) => address,
        None => {
            meta.tech_err = Some("technology roster address overflow");
            return false;
        }
    };
    match read::<[u8; 16]>(memory, tech_addr) {
        Some(header) => {
            meta.tech_header = header;
            meta.tech_header_ok = true;
            tallies.bytes[0] += 16;
        }
        None => {
            meta.tech_header_ok = false;
            meta.tech_err = Some("technology roster header invalid");
            tallies.live_fault[0] = true;
            return false;
        }
    }
    let array = match Array::from_header(meta.tech_header, ctechnology_status::POINTER_STRIDE) {
        Some(array) => array,
        None => {
            meta.tech_err = Some("technology roster header invalid");
            return false;
        }
    };
    meta.tech_start = job.tech_pool.len();
    meta.tmpl_start = job.templates.len();
    let mut page = [0u8; ctechnology::BATCH * 8];
    let mut pointers = [0usize; ctechnology::BATCH];
    let mut blobs = [0u8; ctechnology::BATCH * ctechnology::INSTANCE_BYTES];
    let status_raw = ctechnology_status::StatusRaw {
        pointer: meta.status_pointer,
        vtable: meta.status_vtable,
        backlink: meta.status_backlink,
        scattered: meta.status_scattered,
    };
    // Only the authenticated, successfully decoded prefix can admit a
    // definition. This matches the native dictionary's once-per-pass reads;
    // an invalid earlier instance must not seed its cache.
    let mut admits = ctechnology_status::decode_status(base, country.pointer, &status_raw).is_ok();
    let mut first = 0u32;
    let mut copied = 0usize;
    while first < array.count {
        let count = (array.count - first).min(ctechnology::BATCH as u32);
        let Some(page_addr) = array.element(first, ctechnology_status::POINTER_STRIDE) else {
            meta.tech_err = Some("technology roster address overflow");
            return false;
        };
        if !read_slice(memory, page_addr, &mut page[..count as usize * 8]) {
            meta.tech_err = Some("technology roster pointers unreadable");
            tallies.live_fault[0] = true;
            return false;
        }
        let mut selected = 0;
        for chunk in page[..count as usize * 8].chunks_exact(8) {
            let Ok(pointer) = usize::try_from(u64::from_le_bytes(chunk.try_into().unwrap())) else {
                meta.tech_err = Some("technology pointer overflow");
                return false;
            };
            if pointer == 0 {
                continue;
            }
            if copied + selected == tech_cap {
                meta.tech_unread += 1;
                continue;
            }
            ctechnology::hint(pointer);
            pointers[selected] = pointer;
            selected += 1;
        }
        if !fit_len(job.tech_pool.len(), job.tech_pool.capacity(),
            selected * ctechnology::INSTANCE_BYTES)
            || !fit_len(job.templates.len(), job.templates.capacity(), selected)
        {
            return true;
        }
        // The typed walker rejects the entire current gather on a field
        // fault; do not retain definitions from a partial batch.
        if ctechnology::copy_batch(memory, &pointers[..selected], &mut blobs)
            != selected * ctechnology::FIELDS.len()
        {
            meta.tech_err = Some("technology fields unreadable");
            tallies.live_fault[0] = true;
            return false;
        }
        let started = qpc();
        for blob in blobs[..selected * ctechnology::INSTANCE_BYTES]
            .chunks_exact(ctechnology::INSTANCE_BYTES)
        {
            job.tech_pool.extend_from_slice(blob);
            let template = usize::try_from(blob_template(blob)).unwrap_or(0);
            let handle = blob_handle(blob);
            tallies.max_handle_plus_one = tallies.max_handle_plus_one.max(handle as usize + 1);
            tallies.bytes[0] += ctechnology::INSTANCE_BYTES as u64;
            admits &= ctechnology::decode_one(base, status, country.id, blob.try_into().unwrap()).is_ok();
            if admits {
                let repeated = if let Some(previous) = job.template_seen_country.get_mut(handle as usize) {
                    let repeated = *previous == country.pointer;
                    *previous = country.pointer;
                    repeated
                } else {
                    job.tech_pool[meta.tech_start..job.tech_pool.len() - ctechnology::INSTANCE_BYTES]
                        .chunks_exact(ctechnology::INSTANCE_BYTES)
                        .any(|previous| blob_handle(previous) == handle)
                };
                admits = !repeated;
            }
            if admits {
                if record_template_occ(memory, template, handle, job, tallies) {
                    tallies.ticks[2] += elapsed(started);
                    return true;
                }
            } else {
                job.templates.push(TemplateOcc {
                    pointer: template, handle, header: [0; ctechnology_template::HEADER_BYTES],
                    header_result: Err("technology template fields unreadable"),
                    id_start: job.id_pool.len(), id_len: 0, id_ok: false,
                    term_ok: false, term_overflow: false,
                });
            }
            copied += 1;
            meta.tech_len = job.tech_pool.len() - meta.tech_start;
            meta.tmpl_len = job.templates.len() - meta.tmpl_start;
        }
        tallies.ticks[2] += elapsed(started);
        first += count;
    }
    meta.tech_len = job.tech_pool.len() - meta.tech_start;
    meta.tmpl_len = job.templates.len() - meta.tmpl_start;
    // Researched census: rereads full rows for the independent inventory.
    let researched_addr = match status.checked_add(ctechnology_status::RESEARCHED_DATA) {
        Some(address) => address,
        None => {
            meta.researched_err = Some("researched roster address overflow");
            return false;
        }
    };
    match read::<[u8; 16]>(memory, researched_addr) {
        Some(header) => {
            meta.researched_header = header;
            meta.researched_header_ok = true;
            tallies.bytes[0] += 16;
        }
        None => {
            meta.researched_header_ok = false;
            meta.researched_err = Some("researched roster header invalid");
            tallies.live_fault[0] = true;
            return false;
        }
    }
    let researched = match Array::from_header(
        meta.researched_header,
        ctechnology_status::POINTER_STRIDE,
    ) {
        Some(array) => array,
        None => {
            meta.researched_err = Some("researched roster header invalid");
            return false;
        }
    };
    if researched.count > array.count {
        // Decoded identically below; no blobs are needed for the verdict, but
        // recording them is harmless and keeps the pass uniform. Skip the
        // reads to keep staging small: decode errors before consulting blobs.
        meta.researched_start = job.researched_pool.len();
        meta.researched_len = 0;
        return false;
    }
    meta.researched_start = job.researched_pool.len();
    let mut first = 0u32;
    while first < researched.count {
        let count = (researched.count - first).min(ctechnology::BATCH as u32);
        let page_addr = match researched.element(first, ctechnology_status::POINTER_STRIDE) {
            Some(address) => address,
            None => {
                meta.researched_err = Some("researched roster address overflow");
                return false;
            }
        };
        if !read_slice(
            memory,
            page_addr,
            &mut page[..count as usize * ctechnology_status::POINTER_STRIDE],
        ) {
            meta.researched_err = Some("researched technology pointers unreadable");
            tallies.live_fault[0] = true;
            return false;
        }
        for (pointer, chunk) in pointers[..count as usize].iter_mut()
            .zip(page[..count as usize * ctechnology_status::POINTER_STRIDE].chunks_exact(8))
        {
            let Some(source) = usize::try_from(u64::from_le_bytes(chunk.try_into().unwrap()))
                .ok().filter(|source| *source != 0)
            else {
                meta.researched_err = Some("researched technology pointer unreadable or null");
                tallies.live_fault[0] = true;
                return false;
            };
            *pointer = source;
            ctechnology::hint(source);
        }
        let byte_count = count as usize * ctechnology::INSTANCE_BYTES;
        if !fit_len(job.researched_pool.len(), job.researched_pool.capacity(), byte_count) {
            return true;
        }
        if ctechnology::copy_batch(memory, &pointers[..count as usize], &mut blobs)
            != count as usize * ctechnology::FIELDS.len()
        {
            meta.researched_err = Some("technology fields unreadable");
            tallies.live_fault[0] = true;
            return false;
        }
        job.researched_pool.extend_from_slice(&blobs[..byte_count]);
        tallies.bytes[0] += byte_count as u64;
        first += count;
    }
    meta.researched_len = job.researched_pool.len() - meta.researched_start;
    false
}

fn copy_country_slots(
    memory: &impl Memory,
    status: usize,
    slot_cap: usize,
    job: &mut ResearchJob,
    meta: &mut CountryMeta,
    tallies: &mut PassTallies,
) -> bool {
    let header_addr = match status.checked_add(ctechnology_status::RESEARCH_SLOTS_DATA) {
        Some(address) => address,
        None => {
            meta.slot_err = Some("research slots address overflow");
            return false;
        }
    };
    match read::<[u8; 16]>(memory, header_addr) {
        Some(header) => {
            meta.slot_header = header;
            meta.slot_header_ok = true;
            tallies.bytes[1] += 16;
        }
        None => {
            meta.slot_header_ok = false;
            meta.slot_err = Some("research slots header invalid");
            tallies.live_fault[1] = true;
            return false;
        }
    }
    let array = match Array::from_header(meta.slot_header, ctechnology_status::POINTER_STRIDE) {
        Some(array) => array,
        None => {
            meta.slot_err = Some("research slots header invalid");
            return false;
        }
    };
    let len = (slot_cap).min(array.count as usize);
    meta.slot_start = job.slots.len();
    for index in 0..len {
        let element = match array.element(index as u32, ctechnology_status::POINTER_STRIDE) {
            Some(address) => address,
            None => {
                meta.slot_err = Some("research slots address overflow");
                return false;
            }
        };
        let ptr = read::<u64>(memory, element);
        tallies.bytes[1] += 8;
        let mut occ = SlotOcc {
            ptr: ptr.unwrap_or(0),
            ptr_ok: ptr.is_some(),
            raw: cresearch_slot::Raw::default(),
            raw_result: Ok(4),
            linked: [0; ctechnology::INSTANCE_BYTES],
            linked_completed: 0,
            has_linked: false,
        };
        if !occ.ptr_ok {
            meta.slot_err = Some("research slot pointer unreadable or null");
            tallies.live_fault[1] = true;
            return false;
        }
        let pointer = match usize::try_from(occ.ptr).ok().filter(|&pointer| pointer != 0) {
            Some(pointer) => pointer,
            None => {
                meta.slot_err = Some("research slot pointer unreadable or null");
                return false;
            }
        };
        occ.raw_result = cresearch_slot::copy_raw(memory, pointer, &mut occ.raw);
        tallies.bytes[1] += 56;
        match occ.raw_result {
            Err(reason) => {
                meta.slot_err = Some(reason);
                return false;
            }
            Ok(4) => {}
            Ok(_) => {
                meta.slot_err = Some("research slot fields unreadable");
                tallies.live_fault[1] = true;
                return false;
            }
        }
        let technology_u64 = u64::from_le_bytes(occ.raw.owner[8..].try_into().unwrap());
        if technology_u64 != 0 {
            occ.has_linked = true;
            let linked = usize::try_from(technology_u64).unwrap_or(0);
            occ.linked_completed = ctechnology::copy_one(memory, linked, &mut occ.linked);
            tallies.bytes[1] += ctechnology::INSTANCE_BYTES as u64;
            if occ.linked_completed != ctechnology::FIELDS.len() {
                meta.slot_err = Some("technology fields unreadable");
                tallies.live_fault[1] = true;
                return false;
            }
        }
        if job.slots.len() == job.slots.capacity() {
            return true;
        }
        // Range validation runs per slot in walk order, mirroring `walk_slots`
        // (which checks the decoded row before reading the next slot).
        let index = u32::from_le_bytes(occ.raw.speed_index[8..].try_into().unwrap());
        if index >= array.count {
            meta.slot_err = Some("research slot index outside roster");
            return false;
        }
        job.slots.push(occ);
    }
    meta.slot_len = job.slots.len() - meta.slot_start;
    // Duplicate validation runs after every slot decoded, mirroring
    // `walk_slots`; discovery reads only already-copied bytes, with no
    // allocation (pairwise scan; slot rosters are small).
    let occs = &job.slots[meta.slot_start..meta.slot_start + meta.slot_len];
    for (later, occ) in occs.iter().enumerate() {
        let index = u32::from_le_bytes(occ.raw.speed_index[8..].try_into().unwrap());
        if occs[..later]
            .iter()
            .any(|other| u32::from_le_bytes(other.raw.speed_index[8..].try_into().unwrap()) == index)
        {
            meta.slot_err = Some("duplicate research slot index");
            return false;
        }
    }
    meta.slot_unread = array.count - len as u32;
    false
}

fn copy_research_pass(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    tech_cap: usize,
    slot_cap: usize,
    job: &mut ResearchJob,
) -> PassTallies {
    job.clear();
    let mut tallies = PassTallies::default();
    let mut tech_recorded = 0usize;
    let mut slot_recorded = 0usize;
    let countries = match ccountry::Reader::new(memory, base, game_state) {
        Err(reason) => {
            job.reader_err = Some(reason);
            return tallies;
        }
        Ok(countries) => countries,
    };
    for slot in 1..countries.count() {
        let started = qpc();
        let country = match countries.at(slot) {
            Ok(Some(country)) => country,
            Ok(None) => continue,
            Err(reason) => {
                job.country_err = Some(reason);
                tallies.ticks[0] += elapsed(started);
                break;
            }
        };
        tallies.ticks[0] += elapsed(started);
        if job.metas.len() == job.metas.capacity() {
            job.overflowed = true;
            return tallies;
        }
        // Remaining typed capacities mirror the remaining output slices the
        // typed walks receive, so truncation counts match exactly.
        let tech_remaining = tech_cap.saturating_sub(tech_recorded);
        let slot_remaining = slot_cap.saturating_sub(slot_recorded);
        let mut meta = CountryMeta::default();
        let started = qpc();
        let definitions_before = tallies.ticks[2];
        let overflowed =
            copy_country_tech(memory, base, country, tech_remaining, job, &mut meta, &mut tallies);
        tallies.ticks[0] += elapsed(started).saturating_sub(tallies.ticks[2] - definitions_before);
        if overflowed {
            job.overflowed = true;
            return tallies;
        }
        tech_recorded += meta.tech_len / ctechnology::INSTANCE_BYTES;
        // Slots run even when the technology section faulted: the typed walks
        // are independent and each reports its own outcome.
        let status = meta.status_pointer;
        let status_ok = meta.status_result.is_ok();
        let started = qpc();
        let overflowed = if status_ok {
            copy_country_slots(
                memory,
                status,
                slot_remaining,
                job,
                &mut meta,
                &mut tallies,
            )
        } else {
            false
        };
        tallies.ticks[1] += elapsed(started);
        if overflowed {
            job.overflowed = true;
            return tallies;
        }
        slot_recorded += meta.slot_len;
        job.metas.push(meta);
    }
    tallies
}

fn tech_partition_equal(first: &ResearchJob, second: &ResearchJob) -> bool {
    if first.reader_err != second.reader_err
        || first.country_err != second.country_err
        || first.metas.len() != second.metas.len()
    {
        return false;
    }
    for (x, y) in first.metas.iter().zip(second.metas.iter()) {
        if x.country_id != y.country_id
            || x.country_pointer != y.country_pointer
            || x.status_result != y.status_result
            || x.status_pointer != y.status_pointer
            || x.status_vtable != y.status_vtable
            || x.status_backlink != y.status_backlink
            || x.status_scattered != y.status_scattered
            || x.tech_header != y.tech_header
            || x.tech_header_ok != y.tech_header_ok
            || x.tech_unread != y.tech_unread
            || x.tech_err != y.tech_err
            || x.researched_header != y.researched_header
            || x.researched_header_ok != y.researched_header_ok
            || x.researched_err != y.researched_err
        {
            return false;
        }
        if first.tech_pool.get(x.tech_start..x.tech_start + x.tech_len)
            != second.tech_pool.get(y.tech_start..y.tech_start + y.tech_len)
        {
            return false;
        }
        if first
            .researched_pool
            .get(x.researched_start..x.researched_start + x.researched_len)
            != second
                .researched_pool
                .get(y.researched_start..y.researched_start + y.researched_len)
        {
            return false;
        }
    }
    true
}

fn definitions_partition_equal(first: &ResearchJob, second: &ResearchJob) -> bool {
    if first.metas.len() != second.metas.len() {
        return false;
    }
    for (x, y) in first.metas.iter().zip(second.metas.iter()) {
        if x.tmpl_len != y.tmpl_len {
            return false;
        }
        if first.templates.get(x.tmpl_start..x.tmpl_start + x.tmpl_len)
            != second.templates.get(y.tmpl_start..y.tmpl_start + y.tmpl_len)
        {
            return false;
        }
        // Id bytes travel in the shared pool; compare each occurrence range.
        for (a, b) in first.templates[x.tmpl_start..x.tmpl_start + x.tmpl_len]
            .iter()
            .zip(&second.templates[y.tmpl_start..y.tmpl_start + y.tmpl_len])
        {
            if first.id_pool.get(a.id_start..a.id_start + a.id_len)
                != second.id_pool.get(b.id_start..b.id_start + b.id_len)
            {
                return false;
            }
        }
    }
    true
}

fn slots_partition_equal(first: &ResearchJob, second: &ResearchJob) -> bool {
    if first.metas.len() != second.metas.len() {
        return false;
    }
    for (x, y) in first.metas.iter().zip(second.metas.iter()) {
        if x.slot_header != y.slot_header
            || x.slot_header_ok != y.slot_header_ok
            || x.slot_unread != y.slot_unread
            || x.slot_err != y.slot_err
        {
            return false;
        }
        if first.slots.get(x.slot_start..x.slot_start + x.slot_len)
            != second.slots.get(y.slot_start..y.slot_start + y.slot_len)
        {
            return false;
        }
    }
    true
}

/// Decode-side dictionary replicating `ctechnology_template::Dictionary`
/// over already-copied bytes. Field-for-field semantics (admit-once sticky
/// errors, per-roster generations, capacity and byte accounting) so outcomes
/// match the typed walk exactly.
#[derive(Default)]
struct DictEntryRepl {
    template_pointer: usize,
    row: ctechnology_template::Row,
    copied: bool,
    generation: u32,
    instance_row: u32,
}

struct DictRepl {
    map: HashMap<u32, DictEntryRepl>,
    cap: usize,
    names_cap: usize,
    bytes_len: usize,
    required_bytes: usize,
    required_capacity: usize,
    copied: usize,
    omitted: u32,
    error: Option<&'static str>,
    generation: u32,
}

impl DictRepl {
    fn new(cap: usize, names_cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            cap,
            names_cap,
            bytes_len: 0,
            required_bytes: 0,
            required_capacity: 0,
            copied: 0,
            omitted: 0,
            error: None,
            generation: 0,
        }
    }

    fn outcome(&self) -> Outcome {
        if let Some(reason) = self.error {
            Outcome::Unavailable { reason }
        } else if self.omitted != 0 {
            Outcome::Truncated {
                len: self.copied,
                unread_rows: self.omitted,
            }
        } else {
            Outcome::Rows { len: self.copied }
        }
    }

    fn begin_roster(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            for entry in self.map.values_mut() {
                entry.generation = 0;
            }
            self.generation = 1;
        }
    }

    fn instance_row(&self, handle: u32, rows: &[ctechnology::Row]) -> Option<usize> {
        if let Some(entry) = self.map.get(&handle) {
            (entry.generation == self.generation).then_some(entry.instance_row as usize)
        } else {
            rows.iter().position(|row| row.technology_handle == handle)
        }
    }

    fn admit_instance(
        &mut self,
        pointer: usize,
        handle: u32,
        occ: &TemplateOcc,
        id_bytes: &[u8],
        names: &mut [u8],
        previous: &[ctechnology::Row],
    ) -> Result<(), &'static str> {
        if let Some(entry) = self.map.get_mut(&handle) {
            if entry.generation == self.generation {
                return Err("duplicate technology handle in country roster");
            }
            entry.generation = self.generation;
            entry.instance_row = previous.len() as u32;
            if entry.template_pointer == pointer {
                return Ok(());
            }
            // Entries are only created below with their pointer already set,
            // so a surviving entry that points elsewhere is the same sticky
            // inconsistency the typed walk reports.
            self.error
                .get_or_insert("technology handle has inconsistent template pointers");
            return Ok(());
        }
        if previous.iter().any(|row| row.technology_handle == handle) {
            return Err("duplicate technology handle in country roster");
        }
        self.required_capacity = self.required_capacity.max(handle as usize + 1);
        if handle as usize >= self.cap {
            self.error
                .get_or_insert("technology definition dictionary capacity exceeded");
            return Ok(());
        }
        // Fresh handles record generation and row exactly like the typed
        // walk: the researched census resolves in-cap handles only through
        // this generation mark, never through a row scan.
        let entry = self.map.entry(handle).or_default();
        entry.generation = self.generation;
        entry.instance_row = previous.len() as u32;
        entry.template_pointer = pointer;
        match occ.header_result {
            Err(reason) => {
                self.error.get_or_insert(reason);
                return Ok(());
            }
            Ok(3) => {}
            Ok(_) => {
                self.error
                    .get_or_insert("technology template fields unreadable");
                return Ok(());
            }
        }
        let decoded = match ctechnology_template::decode_header(pointer, &occ.header, handle) {
            Ok(decoded) => decoded,
            Err(reason) => {
                self.error.get_or_insert(reason);
                return Ok(());
            }
        };
        if occ.term_overflow {
            self.error.get_or_insert("technology id address overflow");
            return Ok(());
        }
        if !occ.term_ok {
            self.error
                .get_or_insert("technology id terminator invalid");
            return Ok(());
        }
        self.required_bytes = match self.required_bytes.checked_add(decoded.id_len) {
            Some(required) => required,
            None => {
                self.error.get_or_insert("technology id byte count overflow");
                return Ok(());
            }
        };
        entry.row = ctechnology_template::Row {
            technology_handle: handle,
            database_index: decoded.database_index,
            year: decoded.year,
            max_level: decoded.max_level,
            research_cost_raw: decoded.research_cost_raw,
            id_start: self.bytes_len,
            id_len: decoded.id_len,
        };
        if decoded.id_len > self.names_cap.saturating_sub(self.bytes_len) {
            self.omitted = match self.omitted.checked_add(1) {
                Some(omitted) => omitted,
                None => {
                    self.error
                        .get_or_insert("technology definition count overflow");
                    return Ok(());
                }
            };
            return Ok(());
        }
        if !occ.id_ok || id_bytes.len() != decoded.id_len {
            self.error.get_or_insert("technology id bytes unreadable");
            return Ok(());
        }
        names[self.bytes_len..self.bytes_len + decoded.id_len].copy_from_slice(id_bytes);
        self.bytes_len += decoded.id_len;
        self.copied += 1;
        entry.copied = true;
        Ok(())
    }
}

fn decode_country_tech(
    base: usize,
    meta: &CountryMeta,
    job: &ResearchJob,
    dict: &mut DictRepl,
    names: &mut [u8],
    rows: &mut [ctechnology::Row],
) -> Outcome {
    if let Err(reason) = meta.status_result {
        return Outcome::Unavailable { reason };
    }
    let status_raw = ctechnology_status::StatusRaw {
        pointer: meta.status_pointer,
        vtable: meta.status_vtable,
        backlink: meta.status_backlink,
        scattered: meta.status_scattered,
    };
    let status = match ctechnology_status::decode_status(base, meta.country_pointer, &status_raw)
    {
        Ok(status) => status,
        Err(reason) => return Outcome::Unavailable { reason },
    };
    if !meta.tech_header_ok {
        return Outcome::Unavailable {
            reason: "technology roster header invalid",
        };
    }
    let array = match Array::from_header(meta.tech_header, ctechnology_status::POINTER_STRIDE) {
        Some(array) => array,
        None => {
            return Outcome::Unavailable {
                reason: "technology roster header invalid",
            };
        }
    };
    let blobs = match job
        .tech_pool
        .get(meta.tech_start..meta.tech_start + meta.tech_len)
    {
        Some(blobs) => blobs,
        None => {
            return Outcome::Unavailable {
                reason: "technology fields unreadable",
            };
        }
    };
    let occs = match job
        .templates
        .get(meta.tmpl_start..meta.tmpl_start + meta.tmpl_len)
    {
        Some(occs) => occs,
        None => {
            return Outcome::Unavailable {
                reason: "technology fields unreadable",
            };
        }
    };
    if blobs.len() != occs.len() * ctechnology::INSTANCE_BYTES {
        return Outcome::Unavailable {
            reason: "technology fields unreadable",
        };
    }
    let mut len = 0;
    for (blob, occ) in blobs.chunks_exact(ctechnology::INSTANCE_BYTES).zip(occs) {
        let blob: &[u8; ctechnology::INSTANCE_BYTES] = match blob.try_into() {
            Ok(blob) => blob,
            Err(_) => {
                return Outcome::Unavailable {
                    reason: "technology fields unreadable",
                };
            }
        };
        let instance = match ctechnology::decode_one(base, status, meta.country_id, blob) {
            Ok(instance) => instance,
            Err(reason) => return Outcome::Unavailable { reason },
        };
        let id_bytes = job
            .id_pool
            .get(occ.id_start..occ.id_start + occ.id_len)
            .unwrap_or(&[]);
        if let Err(reason) = dict.admit_instance(
            instance.template,
            instance.row.technology_handle,
            occ,
            id_bytes,
            names,
            &rows[..len],
        ) {
            return Outcome::Unavailable { reason };
        }
        if len >= rows.len() {
            // Defensive and unreachable: the copy enforced the same remaining
            // capacity, so every recorded blob has a typed row waiting.
            return Outcome::Unavailable {
                reason: "technology fields unreadable",
            };
        }
        rows[len] = instance.row;
        len += 1;
    }
    if let Some(reason) = meta.tech_err {
        return Outcome::Unavailable { reason };
    }
    if !meta.researched_header_ok {
        return Outcome::Unavailable {
            reason: "researched roster header invalid",
        };
    }
    let researched = match Array::from_header(
        meta.researched_header,
        ctechnology_status::POINTER_STRIDE,
    ) {
        Some(array) => array,
        None => {
            return Outcome::Unavailable {
                reason: "researched roster header invalid",
            };
        }
    };
    if researched.count > array.count {
        return Outcome::Unavailable {
            reason: "researched roster exceeds all-technology roster",
        };
    }
    if let Some(reason) = meta.researched_err {
        return Outcome::Unavailable { reason };
    }
    let researched_blobs = match job
        .researched_pool
        .get(meta.researched_start..meta.researched_start + meta.researched_len)
    {
        Some(blobs) => blobs,
        None => {
            return Outcome::Unavailable {
                reason: "technology fields unreadable",
            };
        }
    };
    if researched_blobs.len() != researched.count as usize * ctechnology::INSTANCE_BYTES {
        return Outcome::Unavailable {
            reason: "technology fields unreadable",
        };
    }
    let mut previous = None;
    for blob in researched_blobs.chunks_exact(ctechnology::INSTANCE_BYTES) {
        let blob: &[u8; ctechnology::INSTANCE_BYTES] = match blob.try_into() {
            Ok(blob) => blob,
            Err(_) => {
                return Outcome::Unavailable {
                    reason: "technology fields unreadable",
                };
            }
        };
        let instance = match ctechnology::decode_one(base, status, meta.country_id, blob) {
            Ok(instance) => instance,
            Err(reason) => return Outcome::Unavailable { reason },
        };
        let handle = instance.row.technology_handle;
        if previous.is_some_and(|previous: u32| previous as i32 >= handle as i32) {
            return Outcome::Unavailable {
                reason: "researched technology roster is not strictly ordered",
            };
        }
        previous = Some(handle);
        match dict.instance_row(handle, &rows[..len]) {
            Some(index) => {
                if rows[index] != instance.row {
                    return Outcome::Unavailable {
                        reason: "researched technology disagrees with country roster",
                    };
                }
                rows[index].researched = 1;
            }
            None if meta.tech_unread != 0 => {}
            None => {
                return Outcome::Unavailable {
                    reason: "researched technology missing from country roster",
                };
            }
        }
    }
    if meta.tech_unread == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated {
            len,
            unread_rows: meta.tech_unread,
        }
    }
}

fn decode_country_slots(
    base: usize,
    meta: &CountryMeta,
    job: &ResearchJob,
    status: usize,
    rows: &mut [cresearch_slot::Row],
) -> Outcome {
    if !meta.slot_header_ok {
        return Outcome::Unavailable {
            reason: "research slots header invalid",
        };
    }
    let array = match Array::from_header(meta.slot_header, ctechnology_status::POINTER_STRIDE) {
        Some(array) => array,
        None => {
            return Outcome::Unavailable {
                reason: "research slots header invalid",
            };
        }
    };
    let occs = match job
        .slots
        .get(meta.slot_start..meta.slot_start + meta.slot_len)
    {
        Some(occs) => occs,
        None => {
            return Outcome::Unavailable {
                reason: "research slot pointer unreadable or null",
            };
        }
    };
    if let Some(reason) = meta.slot_err {
        return Outcome::Unavailable { reason };
    }
    let mut len = 0;
    for occ in occs {
        if !occ.ptr_ok || occ.ptr == 0 {
            return Outcome::Unavailable {
                reason: "research slot pointer unreadable or null",
            };
        }
        match occ.raw_result {
            Err(reason) => return Outcome::Unavailable { reason },
            Ok(4) => {}
            Ok(_) => {
                return Outcome::Unavailable {
                    reason: "research slot fields unreadable",
                };
            }
        }
        let linked = if occ.has_linked {
            if occ.linked_completed != ctechnology::FIELDS.len() {
                return Outcome::Unavailable {
                    reason: "technology fields unreadable",
                };
            }
            Some(&occ.linked)
        } else {
            None
        };
        let row = match cresearch_slot::decode_raw(base, status, meta.country_id, &occ.raw, linked)
        {
            Ok(row) => row,
            Err(reason) => return Outcome::Unavailable { reason },
        };
        if row.index >= array.count {
            return Outcome::Unavailable {
                reason: "research slot index outside roster",
            };
        }
        if len >= rows.len() {
            // Defensive and unreachable: the copy enforced the same remaining
            // capacity, so every recorded slot has a typed row waiting.
            return Outcome::Unavailable {
                reason: "research slot pointer unreadable or null",
            };
        }
        rows[len] = row;
        len += 1;
    }
    rows[..len].sort_unstable_by_key(|row| row.index);
    if rows[..len]
        .windows(2)
        .any(|pair| pair[0].index == pair[1].index)
    {
        return Outcome::Unavailable {
            reason: "duplicate research slot index",
        };
    }
    let unread_rows = array.count - len as u32;
    if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    }
}

impl Captured {
    pub(crate) fn tech_rows_cap(&self) -> usize {
        self.technologies.rows.len()
    }

    pub(crate) fn slot_rows_cap(&self) -> usize {
        self.slots.rows.len()
    }

    /// Game-thread raw copy for a pilot refresh hour. Copies `first` always
    /// and `second` while any pilot table still needs double-read checks,
    /// compares the raw partitions, and records copy-only costs plus the
    /// existing checked/mismatches bookkeeping. Returns whether the payload
    /// is complete (`raw_ok`); on staging overflow the caller falls back to
    /// the typed walk and enqueues nothing.
    pub(crate) fn copy_research_refresh(
        &mut self,
        memory: &impl Memory,
        ctx: CopyContext,
        first: &mut ResearchJob,
        mut second: Option<&mut ResearchJob>,
    ) -> CopyReport {
        let needed = [
            ctx.needed[TABLES[RESEARCH_LOCAL[0]].index()],
            ctx.needed[TABLES[RESEARCH_LOCAL[1]].index()],
            ctx.needed[TABLES[RESEARCH_LOCAL[2]].index()],
        ];
        let first_tallies = copy_research_pass(
            memory,
            ctx.base,
            ctx.game_state,
            ctx.tech_cap,
            ctx.slot_cap,
            first,
        );
        let mut matched = [true; 3];
        let mut ticks = first_tallies.ticks;
        let mut live_fault = first_tallies.live_fault;
        if let Some(second_job) = second.as_deref_mut() {
            let second_tallies = copy_research_pass(
                memory,
                ctx.base,
                ctx.game_state,
                ctx.tech_cap,
                ctx.slot_cap,
                second_job,
            );
            first.second_max_handle_plus_one = second_tallies.max_handle_plus_one;
            first.second_id_bytes = second_tallies.id_bytes_attempted;
            for (total, other) in ticks.iter_mut().zip(second_tallies.ticks) {
                *total = total.saturating_add(other);
            }
            for (total, other) in live_fault.iter_mut().zip(second_tallies.live_fault) {
                *total |= other;
            }
            let equal = [
                tech_partition_equal(first, second_job),
                slots_partition_equal(first, second_job),
                definitions_partition_equal(first, second_job),
            ];
            for index in 0..3 {
                matched[index] = equal[index] && !live_fault[index];
            }
        }
        // Binary per refresh per table: tearing detected (0/1), not a row
        // count. Spool gates test `==0`/`!=0` only.
        self.technologies.checked = needed[0];
        self.technologies.mismatches = usize::from(needed[0] && !matched[0]);
        self.slots.checked = needed[1];
        self.slots.mismatches = usize::from(needed[1] && !matched[1]);
        self.definitions.checked = needed[2];
        self.definitions.mismatches = usize::from(needed[2] && !matched[2]);
        let raw_ok = !first.overflowed
            && second.is_none_or(|job| !job.overflowed);
        CopyReport {
            raw_ok,
            matched,
            copy_ticks: ticks,
            raw_bytes: [
                first_tallies.bytes[0],
                first_tallies.bytes[1],
                first_tallies.bytes[2],
            ],
        }
    }

    /// Decode-worker interpretation of a raw refresh payload into the pooled
    /// typed rows. Outcomes, arena bytes and dictionary accounting match the
    /// typed walk exactly; checked/mismatches set by the raw compare survive.
    /// Clears `raw_mode` so the next capture starts clean.
    pub(crate) fn decode_research_payload(
        &mut self,
        base: usize,
        job: &ResearchJob,
        decode_ticks: &mut [u64; 3],
    ) {
        let started = qpc();
        let mut dict = DictRepl::new(self.dictionary.len(), self.definition_text.first.len());
        let mut tech_total = Outcome::Rows { len: 0 };
        let mut tech_len = 0;
        if job.reader_err.is_none() {
            for meta in &job.metas {
                dict.begin_roster();
                let outcome = decode_country_tech(
                    base,
                    meta,
                    job,
                    &mut dict,
                    &mut self.definition_text.first,
                    &mut self.technologies.rows[tech_len..],
                );
                append(&mut tech_total, outcome);
                tech_len = tech_total.len();
            }
        }
        decode_ticks[0] += elapsed(started);
        let started = qpc();
        let mut slot_total = Outcome::Rows { len: 0 };
        let mut slot_len = 0;
        if job.reader_err.is_none() {
            for meta in &job.metas {
                // Status was validated by the technology decode above; slots
                // reuse the same pointer through an identical validation.
                let status = decode_country_status_for_slots(base, meta);
                let outcome = match status {
                    Ok(status) => decode_country_slots(
                        base,
                        meta,
                        job,
                        status,
                        &mut self.slots.rows[slot_len..],
                    ),
                    Err(reason) => Outcome::Unavailable { reason },
                };
                append(&mut slot_total, outcome);
                slot_len = slot_total.len();
            }
        }
        decode_ticks[1] += elapsed(started);
        if let Some(reason) = job.reader_err {
            tech_total = Outcome::Unavailable { reason };
            slot_total = Outcome::Unavailable { reason };
        }
        if let Some(reason) = job.country_err {
            tech_total = Outcome::Unavailable { reason };
            slot_total = Outcome::Unavailable { reason };
        }
        self.technologies.outcome = tech_total;
        self.slots.outcome = slot_total;
        let started = qpc();
        self.definition_text.len = dict.bytes_len;
        self.definition_text.required = dict.required_bytes.max(job.second_id_bytes);
        self.dictionary_required = dict
            .required_capacity
            .max(job.second_max_handle_plus_one);
        let mut copied = Outcome::Rows { len: 0 };
        let mut order: Vec<u32> = dict
            .map
            .iter()
            .filter(|(_, entry)| entry.copied)
            .map(|(handle, _)| *handle)
            .collect();
        // Row order in the buffer is irrelevant: the spool sorts by key
        // off-thread. Sorted iteration keeps growth deterministic.
        order.sort_unstable();
        for handle in order {
            let row = dict.map[&handle].row;
            let first = copied.len();
            append(&mut copied, one(&mut self.definitions.rows[first..], row));
        }
        // Mirror the `capture` assembly match exactly, including the census
        // arm for partial dictionaries after a country-level fault.
        self.definitions.outcome = match (tech_total, dict.outcome(), copied) {
            (Outcome::Rows { .. }, Outcome::Rows { .. }, output) => output,
            (Outcome::Rows { .. }, Outcome::Truncated { unread_rows, .. }, mut output) => {
                append(
                    &mut output,
                    Outcome::Truncated {
                        len: 0,
                        unread_rows,
                    },
                );
                output
            }
            (_, Outcome::Unavailable { reason }, _) => Outcome::Unavailable { reason },
            _ => Outcome::Unavailable {
                reason: "technology definition source census incomplete",
            },
        };
        decode_ticks[2] += elapsed(started);
        self.raw_mode = false;
    }
}

// ===========================================================================
// Hourly Tier-H raw-copy split (Focus, CompletedFocuses).
//
// The game thread copies raw bytes only, every hour: one shared country
// census (mirroring `ccountry::Reader`), per-country focus-status roots with
// vtable checks, summary fields, three token headers with heap payloads and
// terminators, and the completed-focus roster with per-entry pointers and
// tokens. No row is decoded on the game thread. The single FIFO decode worker
// replays the existing `cnational_focus_progress::walk` per country against
// the recorded spans (`crate::raw::Recorded`), so rows, ordinals, skips,
// truncation counts and failure reasons are identical by construction (see
// `crate::raw` for the mirror rules).
//
// The mirror traverses structurally and records every discoverable span,
// never aborting except on overflow: validation, precedence and error scope
// are local parses, so the replay derives every outcome from the same
// recorded spans and later extras are harmless. Pooled row and arena
// capacities are ignored while copying (a superset is recorded and the replay
// truncates identically against the real pooled buffers).
// ===========================================================================

/// Fault-attribution bits: summary walk (Focus) and completed-roster walk
/// (CompletedFocuses). The shared census and focus-status roots block both
/// slots: a fault there fails the per-country walk, which makes both outcomes
/// unavailable with the same reason.
const FOCUS_MASK: u32 = 0x01;
const COMPLETED_MASK: u32 = 0x02;
const FOCUS_CENSUS_MASK: u32 = 0x03;

#[derive(Debug, Default, Clone, Copy)]
struct FocusCountry {
    pointer: usize,
}

/// One hour of raw focus payload: the shared census plus every country's
/// summary and completed spans. Flat pools recycle capacities and the steady
/// state allocates on neither thread; growth happens on the decode side only
/// (see `crate::decode::recycle_focus_job`).
#[derive(Debug, Default)]
pub(crate) struct FocusJob {
    pub log: RawLog<2>,
    pub game_state: usize,
    census: Vec<FocusCountry>,
}

impl FocusJob {
    pub fn new() -> Self {
        Self {
            // Steady state is 439 countries with about 3.9k completed rows
            // plus token bytes; both pools start above that so steady state
            // never overflows.
            log: RawLog::new(1 << 16, 4 << 20),
            game_state: 0,
            census: Vec::with_capacity(512),
        }
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.census.clear();
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    /// Double every pool off the game thread until copies fit. Converges like
    /// the typed capacity growth; overflowed hours fall back to the typed
    /// walk in the meantime, so sizing is a pure performance knob.
    pub fn grow(&mut self) {
        self.log.grow();
crate::raw::double_capacity(&mut self.census);
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct FocusCopyReport {
    /// False when staging overflowed: the caller falls back to the typed walk
    /// (identical rows through the existing path) and enqueues no payload.
    pub raw_ok: bool,
    /// Whole-log raw equality gated by live-fault conservatism, in
    /// [focus, completed] order.
    pub matched: [bool; 2],
    /// Single-pass copy ticks anchor-charged to the Focus slot, matching the
    /// read-group convention (the completed slot keeps zero game-thread cost).
    pub copy_ticks: [u64; 2],
    pub raw_bytes: [u64; 2],
}

/// Game-thread copy parameters, bundled to keep the tick entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FocusCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub needed: [bool; 2],
}

fn focus_pool_u64(log: &RawLog<2>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn focus_pool_u32(log: &RawLog<2>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
fn fspan(
    log: &mut RawLog<2>,
    memory: &impl engine::Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// Authenticated census shared by the focus walks. Records the exact read set
/// of `ccountry::Reader::new` plus one full `read_at` per slot. Returns
/// `false` when the typed census fails, in which case every per-country walk
/// fails identically from the same recorded spans.
fn copy_focus_census(
    log: &mut RawLog<2>,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    census: &mut Vec<FocusCountry>,
) -> Option<(Array, u32)> {
    let countries_start = fspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRIES),
        16,
        FOCUS_CENSUS_MASK,
    )?;
    let links_start = fspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRY_LINKS),
        16,
        FOCUS_CENSUS_MASK,
    )?;
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries_start..countries_start + 16]);
    let countries = Array::from_header(raw, 8)?;
    raw.copy_from_slice(&log.bytes[links_start..links_start + 16]);
    let links = Array::from_header(raw, 4)?;
    if countries.count < 2 || links.count == 0 {
        return None;
    }
    let vtable = base.checked_add(ccountry::VTABLE_RVA)? as u64;
    for slot in 1..countries.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|offset| countries.data.checked_add(offset));
        let pointer_start = fspan(log, memory, pointer_addr, 8, FOCUS_CENSUS_MASK)?;
        let pointer = match usize::try_from(focus_pool_u64(log, pointer_start))
            .ok()
            .filter(|&pointer| pointer != 0)
        {
            Some(pointer) => pointer,
            None => continue,
        };
        let vtable_start = fspan(log, memory, Some(pointer), 8, FOCUS_CENSUS_MASK)?;
        let id_start = fspan(
            log,
            memory,
            pointer.checked_add(ccountry::ID),
            4,
            FOCUS_CENSUS_MASK,
        )?;
        if focus_pool_u64(log, vtable_start) != vtable {
            return None;
        }
        let id = focus_pool_u32(log, id_start);
        let canonical = match focus_link(log, memory, &links, countries.count, id) {
            Some(Some(canonical)) => canonical,
            _ => return None,
        };
        let canonical_slot = match focus_link(log, memory, &links, countries.count, slot) {
            Some(Some(canonical_slot)) => canonical_slot,
            _ => return None,
        };
        if canonical_slot != canonical {
            return None;
        }
        let check = match focus_link(log, memory, &links, countries.count, canonical) {
            Some(Some(check)) => check,
            _ => return None,
        };
        if check != canonical {
            return None;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|offset| countries.data.checked_add(offset));
        let canonical_pointer = fspan(log, memory, canonical_addr, 8, FOCUS_CENSUS_MASK)?;
        if focus_pool_u64(log, canonical_pointer) != pointer as u64 {
            return None;
        }
        if census.len() == census.capacity() {
            log.overflowed = true;
            return None;
        }
        census.push(FocusCountry { pointer });
    }
    Some((links, countries.count))
}

/// Mirror of a link-table read with local bounds checks. Outer `None`
/// aborts the walk; inner `None` is a readable absence the typed callers
/// also turn into a walk error.
fn focus_link(
    log: &mut RawLog<2>,
    memory: &impl engine::Memory,
    links: &Array,
    countries: u32,
    raw: u32,
) -> Option<Option<u32>> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = fspan(
        log,
        memory,
        links.data.checked_add(offset),
        4,
        FOCUS_CENSUS_MASK,
    )?;
    let slot = focus_pool_u32(log, start);
    if slot == 0 {
        return Some(None);
    }
    if slot >= countries {
        return None;
    }
    Some(Some(slot))
}

/// Mirror of one focus token's string reads: header, terminator and heap
/// payload takes. Takes run unconditionally over parseable addresses; the
/// replay validates the header and terminator itself from the same recorded
/// spans, so later extras are harmless.
fn copy_focus_token(
    log: &mut RawLog<2>,
    memory: &impl engine::Memory,
    pointer: u64,
    offset: usize,
    mask: u32,
) {
    if pointer == 0 {
        return;
    }
    let Ok(pointer) = usize::try_from(pointer) else {
        return;
    };
    let Some(string) = pointer.checked_add(offset) else {
        return;
    };
    let Some(header_start) = fspan(log, memory, Some(string), 32, mask) else {
        return;
    };
    let mut header = [0u8; 32];
    header.copy_from_slice(&log.bytes[header_start..header_start + 32]);
    let len = match usize::try_from(u64::from_le_bytes(
        header[msvc_string::LENGTH..msvc_string::CAPACITY]
            .try_into()
            .unwrap(),
    )) {
        Ok(len) => len,
        Err(_) => return,
    };
    let capacity = u64::from_le_bytes(header[msvc_string::CAPACITY..].try_into().unwrap());
    if len as u64 > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return;
    }
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        string
    } else {
        match usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .ok()
            .filter(|&source| source != 0)
        {
            Some(source) => source,
            None => return,
        }
    };
    let Some(end) = source.checked_add(len) else {
        return;
    };
    if fspan(log, memory, Some(end), 1, mask).is_none() {
        return;
    }
    // Inline payloads live inside the header span, but the replay requests
    // the exact (address, length) range, so record it exactly like a heap
    // payload: service is exact-match, never sub-range.
    if len != 0 {
        let _ = fspan(log, memory, Some(source), len, mask);
    }
}

/// Mirror of one country's focus read set: status root and vtable, tree
/// pointer, scattered summary fields, three token takes, and the completed
/// roster with per-entry pointers and tokens. Takes are unconditional over
/// discoverable addresses (a superset); the replay derives every outcome.
fn copy_focus_country(
    log: &mut RawLog<2>,
    memory: &impl engine::Memory,
    country: &FocusCountry,
) {
    let root = match fspan(
        log,
        memory,
        country.pointer.checked_add(ccountry::FOCUS_PROGRESS),
        8,
        FOCUS_CENSUS_MASK,
    )
    .and_then(|start| {
        usize::try_from(focus_pool_u64(log, start))
            .ok()
            .filter(|&root| root != 0)
    }) {
        Some(root) => root,
        None => return,
    };
    let _ = fspan(log, memory, Some(root), 8, FOCUS_CENSUS_MASK);
    let tree = match fspan(
        log,
        memory,
        country.pointer.checked_add(ccountry::FOCUS_TREE),
        8,
        FOCUS_MASK,
    ) {
        Some(start) => focus_pool_u64(log, start),
        None => return,
    };
    // Scattered summary fields exactly like the typed walk: one 16-byte
    // destination at CURRENT (covering CURRENT and CONTINUOUS contiguously),
    // then progress, paused and completed count.
    let selections_start = fspan(
        log,
        memory,
        root.checked_add(cnational_focus_progress::CURRENT),
        16,
        FOCUS_MASK,
    );
    let progress_start = fspan(
        log,
        memory,
        root.checked_add(cnational_focus_progress::PROGRESS),
        8,
        FOCUS_MASK,
    );
    let paused_start = fspan(
        log,
        memory,
        root.checked_add(cnational_focus_progress::PAUSED),
        1,
        FOCUS_MASK,
    );
    let count_start = fspan(
        log,
        memory,
        root.checked_add(cnational_focus_progress::COMPLETED_COUNT),
        4,
        FOCUS_MASK,
    );
    let (Some(selections_start), Some(_), Some(_), Some(_)) =
        (selections_start, progress_start, paused_start, count_start)
    else {
        return;
    };
    let current = focus_pool_u64(log, selections_start);
    let continuous = focus_pool_u64(log, selections_start + 8);
    copy_focus_token(log, memory, tree, cnational_focus_tree::TOKEN, FOCUS_MASK);
    copy_focus_token(log, memory, current, cnational_focus::TOKEN, FOCUS_MASK);
    copy_focus_token(
        log,
        memory,
        continuous,
        ccontinuous_national_focus::TOKEN,
        FOCUS_MASK,
    );
    if log.overflowed {
        return;
    }
    // Completed roster: header plus per-entry pointers and tokens.
    let header_start = match fspan(
        log,
        memory,
        root.checked_add(cnational_focus_progress::COMPLETED_DATA),
        16,
        COMPLETED_MASK,
    ) {
        Some(start) => start,
        None => return,
    };
    let mut header = [0u8; 16];
    header.copy_from_slice(&log.bytes[header_start..header_start + 16]);
    let roster = match Array::from_header(header, 8) {
        Some(roster) => roster,
        None => return,
    };
    for ordinal in 0..roster.count {
        let entry_addr = roster
            .data
            .checked_add((ordinal as usize).saturating_mul(8));
        let pointer = match fspan(log, memory, entry_addr, 8, COMPLETED_MASK).and_then(|start| {
            usize::try_from(focus_pool_u64(log, start))
                .ok()
                .filter(|&pointer| pointer != 0)
        }) {
            Some(pointer) => pointer,
            None => continue,
        };
        copy_focus_token(
            log,
            memory,
            pointer as u64,
            cnational_focus::TOKEN,
            COMPLETED_MASK,
        );
        if log.overflowed {
            return;
        }
    }
}

/// One structural copy pass: shared census plus every country's summary and
/// completed takes. The job is cleared on entry so pooled refreshes never
/// accumulate spans across hours.
fn copy_focus_pass(
    job: &mut FocusJob,
    memory: &impl engine::Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; 2],
) {
    job.clear();
    job.game_state = game_state;
    let started = qpc();
    if copy_focus_census(&mut job.log, memory, base, game_state, &mut job.census).is_some() {
        for index in 0..job.census.len() {
            let country = job.census[index];
            copy_focus_country(&mut job.log, memory, &country);
            if job.log.overflowed {
                break;
            }
        }
    }
    ticks[0] += elapsed(started);
}

impl Captured {
    /// Game-thread raw copy for one hour: shared census plus every country's
    /// summary and completed takes. Compares per-table raw partitions while
    /// either focus table still needs double-read checks: the shared census
    /// gates both slots, focus/completed private spans only their own.
    pub(crate) fn copy_focus_refresh(
        &mut self,
        memory: &impl engine::Memory,
        ctx: FocusCopyContext,
        first: &mut FocusJob,
        mut second: Option<&mut FocusJob>,
    ) -> FocusCopyReport {
        let needed = ctx.needed;
        let mut ticks = [0u64; 2];
        copy_focus_pass(first, memory, ctx.base, ctx.game_state, &mut ticks);
        let mut matched = [true; 2];
        if let Some(second_job) = second.as_deref_mut() {
            let mut second_ticks = [0u64; 2];
            copy_focus_pass(second_job, memory, ctx.base, ctx.game_state, &mut second_ticks);
            ticks[0] += second_ticks[0];
            let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
            for (slot, matched) in matched.iter_mut().enumerate() {
                let equal =
                    raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
                *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
            }
        }
        // Binary per refresh per table: tearing detected (0/1), not a row
        // count. Spool gates test `==0`/`!=0` only.
        self.focus.checked = needed[0];
        self.focus.mismatches = usize::from(needed[0] && !matched[0]);
        self.completed.checked = needed[1];
        self.completed.mismatches = usize::from(needed[1] && !matched[1]);
        let raw_ok = !first.log.overflowed
            && second.is_none_or(|job| !job.log.overflowed);
        FocusCopyReport {
            raw_ok,
            matched,
            copy_ticks: ticks,
            raw_bytes: first.log.raw_bytes,
        }
    }

    /// Decode-worker interpretation of a raw focus payload. Replays the
    /// existing per-country walk against the recorded spans, installing
    /// identical rows, truncation counts and failure reasons into the frame's
    /// pooled buffers. The census spans serve the walk's own country
    /// authentication, so no live memory is touched. Clears `focus_raw_mode`
    /// so the next capture starts clean.
    pub(crate) fn decode_focus_payload(
        &mut self,
        base: usize,
        job: &FocusJob,
        decode_ticks: &mut [u64; 2],
    ) {
        let recorded = crate::raw::Recorded::build(&job.log);
        let started = qpc();
        let mut focus_total = Outcome::Rows { len: 0 };
        let mut completed_total = Outcome::Rows { len: 0 };
        let mut text_len = 0usize;
        let mut text_required = 0usize;
        match ccountry::Reader::new(&recorded, base, job.game_state) {
            Err(reason) => {
                focus_total = Outcome::Unavailable { reason };
                completed_total = Outcome::Unavailable { reason };
            }
            Ok(countries) => {
                for slot in 1..countries.count() {
                    let country = match countries.at(slot) {
                        Ok(Some(country)) => country,
                        Ok(None) => continue,
                        Err(reason) => {
                            focus_total = Outcome::Unavailable { reason };
                            completed_total = Outcome::Unavailable { reason };
                            break;
                        }
                    };
                    let first = focus_total.len();
                    let completed = completed_total.len();
                    let text = text_len;
                    let result = cnational_focus_progress::walk(
                        &recorded,
                        base,
                        country.pointer,
                        country.id,
                        &mut self.focus.rows[first..],
                        &mut self.completed.rows[completed..],
                        &mut self.focus_text.first[text..],
                    );
                    for row in &mut self.focus.rows[first..first + result.outcome.len()] {
                        for span in [&mut row.tree, &mut row.current, &mut row.continuous]
                            .into_iter()
                            .flatten()
                        {
                            span.start += text;
                        }
                    }
                    for row in &mut self.completed.rows
                        [completed..completed + result.completed_outcome.len()]
                    {
                        row.token.start += text;
                    }
                    append(&mut focus_total, result.outcome);
                    append(&mut completed_total, result.completed_outcome);
                    text_len += result.bytes_len;
                    text_required = text_required.saturating_add(result.required_bytes);
                }
            }
        }
        self.focus.outcome = focus_total;
        self.completed.outcome = completed_total;
        self.focus_text.len = text_len;
        self.focus_text.required = text_required;
        decode_ticks[0] += elapsed(started);
        self.focus_raw_mode = false;
    }
}

fn decode_country_status_for_slots(base: usize, meta: &CountryMeta) -> Result<usize, &'static str> {
    meta.status_result?;
    let status_raw = ctechnology_status::StatusRaw {
        pointer: meta.status_pointer,
        vtable: meta.status_vtable,
        backlink: meta.status_backlink,
        scattered: meta.status_scattered,
    };
    ctechnology_status::decode_status(base, meta.country_pointer, &status_raw)
}


fn valid<T>(arena: &[T], start: usize, len: usize) -> bool {
    start.checked_add(len).is_some_and(|end| end <= arena.len())
}
fn copy_span<T: Clone>(arena: &[T], start: usize, len: usize) -> Vec<T> {
    arena[start..start + len].to_vec()
}
fn view<S: TableSchema, R>(
    captured: &CaptureRows<R>,
    validate: impl Fn(&R) -> bool,
    convert: impl Fn(&R) -> S::Row,
) -> TableView {
    let Some(source) = captured.rows.get(..captured.outcome.len()) else {
        return crate::spooling::unavailable("governance row count exceeds capacity");
    };
    if source.iter().any(|row| !validate(row)) {
        return crate::spooling::unavailable("governance arena span exceeds captured bytes");
    }
    crate::spooling::table::<S, _>(captured.outcome, &captured.rows, convert)
}
fn country_view<S: TableSchema, R>(
    captured: &CaptureRows<R>,
    countries: &[u32],
    validate: impl Fn(&R) -> bool,
    convert: impl Fn(u32, &R) -> S::Row,
) -> TableView {
    let len = captured.outcome.len();
    let Some(source) = captured.rows.get(..len).zip(countries.get(..len)) else {
        return crate::spooling::unavailable("governance country row count exceeds capacity");
    };
    if source.0.iter().any(|row| !validate(row)) {
        return crate::spooling::unavailable("governance arena span exceeds captured bytes");
    }
    crate::spooling::table_iter::<S, _>(
        captured.outcome,
        source.1.iter().copied().zip(source.0.iter()),
        |(country, row)| convert(country, row),
    )
}

impl Captured {
    /// Spool thread only: all variable-length copies and portable conversion.
    pub fn tables(&self) -> [TableView; N] {
        let focus = &self.focus_text.first[..self.focus_text.len];
        let definitions = &self.definition_text.first[..self.definition_text.len];
        let parties = &self.party_text.first[..self.party_text.len];
        let ideas = &self.idea_text.first[..self.idea_text.len];
        let decisions = &self.decision_text.first[..self.decision_text.len];
        let modifiers = &self.modifier_text.first[..self.modifier_text.len];
        let values = &self.modifier_values.first[..self.modifier_values.len];
        let tactics = &self.tactic_text.first[..self.tactic_text.len];
        let balances = &self.balance_text.first[..self.balance_text.len];
        let focus_valid = |span: Option<cnational_focus_progress::TokenSpan>| {
            span.is_none_or(|span| valid(focus, span.start, span.len))
        };
        let focus_copy = |span: Option<cnational_focus_progress::TokenSpan>| {
            span.map(|span| copy_span(focus, span.start, span.len))
        };
        let balance_valid =
            |span: engine::power_balance_definition::Token| valid(balances, span.start, span.len);
        let balance_copy = |span: engine::power_balance_definition::Token| {
            copy_span(balances, span.start, span.len)
        };
        [
            view::<tables::Focus, _>(
                &self.focus,
                |r| focus_valid(r.tree) && focus_valid(r.current) && focus_valid(r.continuous),
                |r| rows::FocusRow {
                    country: r.country,
                    tree: focus_copy(r.tree),
                    current: focus_copy(r.current),
                    continuous: focus_copy(r.continuous),
                    progress: r.progress,
                    paused: r.paused,
                    completed_count: r.completed_count,
                },
            ),
            view::<tables::CompletedFocuses, _>(
                &self.completed,
                |r| valid(focus, r.token.start, r.token.len),
                |r| rows::CompletedFocusRow {
                    country: r.country,
                    ordinal: r.ordinal,
                    token: copy_span(focus, r.token.start, r.token.len),
                },
            ),
            view::<tables::ResearchTechnologies, _>(
                &self.technologies,
                |_| true,
                |r| rows::ResearchTechnologyRow {
                    country: r.country,
                    technology_handle: r.technology_handle,
                    classifier: r.classifier,
                    level: r.level,
                    researched: r.researched,
                    progress_raw: r.progress_raw,
                    ahead_reduction_years_raw: r.ahead_reduction_years_raw,
                },
            ),
            view::<tables::ResearchSlots, _>(
                &self.slots,
                |_| true,
                |r| rows::ResearchSlotRow {
                    country: r.country,
                    index: r.index,
                    technology_handle: r.technology_handle,
                    technology_present: r.technology_present,
                    research_speed_raw: r.research_speed_raw,
                    banked_days_raw: r.banked_days_raw,
                    applied_days_raw: r.applied_days_raw,
                },
            ),
            view::<tables::TechnologyDefinitions, _>(
                &self.definitions,
                |r| valid(definitions, r.id_start, r.id_len),
                |r| rows::TechnologyDefinitionRow {
                    technology_handle: r.technology_handle,
                    database_index: r.database_index,
                    year: r.year,
                    max_level: r.max_level,
                    research_cost_raw: r.research_cost_raw,
                    id: copy_span(definitions, r.id_start, r.id_len),
                },
            ),
            view::<tables::Politics, _>(
                &self.politics,
                |_| true,
                |r| rows::PoliticsRow {
                    country: r.country,
                    last_election: r.row.last_election,
                    election_frequency: r.row.election_frequency,
                    elections_allowed: r.row.elections_allowed,
                    capital: r.row.capital,
                    ruling_party: r.row.ruling_party,
                },
            ),
            country_view::<tables::PoliticalParties, _>(
                &self.parties,
                &self.party_countries.first[..self.party_countries.len],
                |r| valid(parties, r.token_start, r.token_len),
                |country, r| rows::PoliticalPartyRow {
                    country,
                    id: r.id,
                    popularity: r.popularity,
                    is_ruling: r.is_ruling,
                    token: copy_span(parties, r.token_start, r.token_len),
                },
            ),
            country_view::<tables::ActiveIdeas, _>(
                &self.ideas,
                &self.idea_countries.first[..self.idea_countries.len],
                |r| valid(ideas, r.token_start, r.token_len),
                |country, r| rows::ActiveIdeaRow {
                    country,
                    ordinal: r.ordinal,
                    token: copy_span(ideas, r.token_start, r.token_len),
                },
            ),
            view::<tables::DecisionCounts, _>(
                &self.counts,
                |_| true,
                |r| rows::DecisionCountsRow {
                    country: r.country,
                    taken: r.row.taken,
                    re_enable: r.row.re_enable,
                    remove: r.row.remove,
                    timed: r.row.timed,
                    targeted_timed: r.row.targeted_timed,
                    targeted: r.row.targeted,
                },
            ),
            country_view::<tables::Decisions, _>(
                &self.decisions,
                &self.decision_countries.first[..self.decision_countries.len],
                |r| valid(decisions, r.token_start, r.token_len),
                |country, r| rows::DecisionRow {
                    country,
                    container: r.container as u8,
                    ordinal: r.ordinal,
                    token: copy_span(decisions, r.token_start, r.token_len),
                    days: r.days,
                    lifecycle: r.lifecycle,
                    target_kind: r.target_kind as u8,
                    country_target: r.country_target,
                    numeric_target: r.numeric_target,
                    ignore: r.ignore,
                },
            ),
            view::<tables::DynamicModifiers, _>(
                &self.modifiers,
                |r| {
                    valid(modifiers, r.token_start, r.token_len)
                        && valid(values, r.value_start, r.value_len)
                },
                |r| rows::DynamicModifierRow {
                    country: r.country,
                    ordinal: r.ordinal,
                    scope_country: r.scope_country,
                    scope_state: r.scope_state,
                    days_remaining: r.days_remaining,
                    enabled: r.enabled,
                    token: copy_span(modifiers, r.token_start, r.token_len),
                    values: copy_span(values, r.value_start, r.value_len),
                },
            ),
            view::<tables::PreferredTactics, _>(
                &self.tactics,
                |r| valid(tactics, r.token_start, r.row.token_len),
                |r| rows::PreferredTacticRow {
                    country: r.row.country,
                    tactic_id: r.row.tactic_id,
                    token: copy_span(tactics, r.token_start, r.row.token_len),
                },
            ),
            view::<tables::PreferredTacticOperands, _>(
                &self.operands,
                |_| true,
                |r| rows::PreferredTacticOperandsRow {
                    country: r.country,
                    tactic_id: r.tactic_id,
                    modifier_key: r.modifier_key,
                    metadata_flags: r.metadata_flags,
                    metadata_gate: r.metadata_gate,
                    country_factor_raw: r.country_factor_raw,
                    accumulator_raw: r.accumulator_raw,
                    accumulator_entry_present: r.accumulator_entry_present,
                },
            ),
            view::<tables::TechSharingGroups, _>(
                &self.sharing,
                |_| true,
                |r| rows::TechSharingGroupRow {
                    id: r.id,
                    base_bonus: r.base_bonus,
                    member_count: r.member_count,
                },
            ),
            view::<tables::TechSharingMembers, _>(
                &self.sharing_members,
                |_| true,
                |r| rows::TechSharingMemberRow {
                    group: r.group,
                    ordinal: r.ordinal,
                    country: r.country,
                    bonus: r.bonus,
                },
            ),
            view::<tables::PowerBalances, _>(
                &self.balances,
                |r| {
                    [r.balance, r.left, r.right, r.trending, r.active_range]
                        .into_iter()
                        .all(balance_valid)
                },
                |r| rows::PowerBalanceRow {
                    ordinal: r.ordinal,
                    value_raw: r.value_raw,
                    balance: balance_copy(r.balance),
                    left: balance_copy(r.left),
                    right: balance_copy(r.right),
                    trending: balance_copy(r.trending),
                    active_range: balance_copy(r.active_range),
                },
            ),
            view::<tables::PowerBalanceMembers, _>(
                &self.balance_members,
                |_| true,
                |r| rows::PowerBalanceMemberRow {
                    record: r.record,
                    ordinal: r.ordinal,
                    country: r.country,
                    daily_raw: r.daily_raw,
                    weekly_raw: r.weekly_raw,
                    daily_present: r.daily_present,
                    weekly_present: r.weekly_present,
                    daily_flags: r.daily_flags,
                    weekly_flags: r.weekly_flags,
                },
            ),
            view::<tables::PowerBalanceRanges, _>(
                &self.ranges,
                |r| balance_valid(r.identity),
                |r| rows::PowerBalanceRangeRow {
                    record: r.record,
                    ordinal: r.ordinal,
                    identity: balance_copy(r.identity),
                },
            ),
            view::<tables::PowerBalanceOverrides, _>(
                &self.overrides,
                |r| balance_valid(r.side) && balance_valid(r.gfx),
                |r| rows::PowerBalanceOverrideRow {
                    record: r.record,
                    ordinal: r.ordinal,
                    side: balance_copy(r.side),
                    gfx: balance_copy(r.gfx),
                },
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn large_census_growth_leaves_quarter_capacity_free_without_power_of_two_inflation() {
        let mut rows = CaptureRows::<ctechnology::Row>::new(4, CONSISTENCY_TARGET);
        let mut capacity = 4;
        rows.outcome = Outcome::Truncated {
            len: 4,
            unread_rows: 246_996,
        };
        grow_large(&mut rows, &mut capacity, CONSISTENCY_TARGET);
        assert_eq!(capacity, 329_334);
        assert!(247_000 <= capacity - capacity / 4);
        assert!(rows.second.is_empty());
        rows.outcome = Outcome::Rows { len: 247_000 };
        grow_large(&mut rows, &mut capacity, CONSISTENCY_TARGET);
        assert_eq!(capacity, 329_334);
    }
    #[test]
    fn prior_failure_and_unread_count_survive_later_empty_country() {
        let mut outcome = Outcome::Truncated {
            len: 2,
            unread_rows: 3,
        };
        append(&mut outcome, Outcome::Rows { len: 0 });
        assert_eq!(
            outcome,
            Outcome::Truncated {
                len: 2,
                unread_rows: 3
            }
        );
        append(
            &mut outcome,
            Outcome::Unavailable {
                reason: "invalid country",
            },
        );
        append(&mut outcome, Outcome::Rows { len: 4 });
        assert_eq!(
            outcome,
            Outcome::Unavailable {
                reason: "invalid country"
            }
        );
    }
    #[test]
    fn native_decision_source_keys_and_non_utf8_literals_survive_conversion() {
        let mut captured =
            Captured::with_capacities(&Capacities::default(), &[CONSISTENCY_TARGET; N]);
        captured.decisions.rows[0] = cdecision_status::Row {
            container: cdecision_status::Container::Targeted,
            token_len: 2,
            days: i32::MIN,
            country_target: u32::MAX,
            ..Default::default()
        };
        captured.decisions.outcome = Outcome::Rows { len: 1 };
        captured.decision_text.first[..2].copy_from_slice(&[0xff, b'x']);
        captured.decision_text.len = 2;
        captured.decision_countries.first[0] = 17;
        captured.decision_countries.len = 1;
        let tables = captured.tables();
        let TableView::Complete(rows::TableRows::Decisions(rows)) = &tables[9] else {
            panic!()
        };
        assert_eq!(rows[0].country, 17);
        assert_eq!(rows[0].container, 3);
        assert_eq!(rows[0].token, [0xff, b'x']);
        assert_eq!(rows[0].days, i32::MIN);
        assert_eq!(rows[0].country_target, u32::MAX);
        captured.decisions.rows[0].token_len = 3;
        assert!(matches!(
            captured.tables()[9],
            TableView::Unavailable { .. }
        ));
    }

    /// Shared synthetic address space for the copy/decode split harness.
    /// Two countries share one template object; rosters hold native holes and
    /// unsorted acquisition order; ids cover heap and inline storage.
    #[cfg(test)]
    pub(crate) mod research_test_kit {
        use super::super::*;
        use engine::{ccountry, cgame_state};

        pub(crate) const BASE: usize = 0x140000000;
        pub(crate) const GAME_STATE: usize = 0x2000;
        pub(crate) const C1: usize = 0x1000;
        const C2: usize = 0x1100;
        pub(crate) const S1: usize = 0x3000;
        const S2: usize = 0x3100;
        pub(crate) const TECH_A: usize = 0x4000;
        pub(crate) const TECH_B: usize = 0x5000;
        const TECH_A2: usize = 0x4100;
        pub(crate) const TMPL_A: usize = 0x6000;
        pub(crate) const TMPL_B: usize = 0x7000;
        pub(crate) const SLOT_A: usize = 0x8000;
        pub(crate) const SLOT_B: usize = 0x8100;
        const SLOT_C: usize = 0x8200;
        pub(crate) const ALL1: usize = 0x9000;
        pub(crate) const DONE1: usize = 0x9100;
        pub(crate) const SLOTS1: usize = 0x9200;
        const ALL2: usize = 0x9300;
        const DONE2: usize = 0x9400;
        const SLOTS2: usize = 0x9500;
        pub(crate) const HEAP_ID: usize = 0xa000;
        const CTAB: usize = 0x2100;
        const LTAB: usize = 0x2200;

        pub(crate) fn memory(bytes: &[u8]) -> engine::memory::SliceMemory<'_> {
            engine::memory::SliceMemory { base: C1, bytes }
        }

        pub(crate) struct Layout {
            pub base: usize,
            pub game_state: usize,
        }

        fn put<const N: usize>(bytes: &mut [u8], at: usize, value: [u8; N]) {
            bytes[at - C1..at - C1 + N].copy_from_slice(&value);
        }

        fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
            put(bytes, at, (data as u64).to_le_bytes());
            put(bytes, at + 8, count.to_le_bytes());
            put(bytes, at + 12, count.to_le_bytes());
        }

        pub(crate) fn fixture() -> (Vec<u8>, Layout) {
            let mut bytes = vec![0; HEAP_ID + 16 - C1];
            array(&mut bytes, GAME_STATE + cgame_state::COUNTRIES, CTAB, 4);
            array(&mut bytes, GAME_STATE + cgame_state::COUNTRY_LINKS, LTAB, 8);
            put(&mut bytes, CTAB + 16, (C1 as u64).to_le_bytes());
            put(&mut bytes, CTAB + 24, (C2 as u64).to_le_bytes());
            put(&mut bytes, LTAB + 8, 2_u32.to_le_bytes());
            put(&mut bytes, LTAB + 12, 3_u32.to_le_bytes());
            put(&mut bytes, LTAB + 20, 3_u32.to_le_bytes());
            for (country, id, status) in [(C1, 2_u32, S1), (C2, 5, S2)] {
                put(
                    &mut bytes,
                    country,
                    ((BASE + ccountry::VTABLE_RVA) as u64).to_le_bytes(),
                );
                put(&mut bytes, country + ccountry::ID, id.to_le_bytes());
                put(
                    &mut bytes,
                    country + ccountry::TECHNOLOGY_STATUS,
                    (status as u64).to_le_bytes(),
                );
            }
            for (status, country) in [(S1, C1), (S2, C2)] {
                put(
                    &mut bytes,
                    status,
                    ((BASE + ctechnology_status::VTABLE_RVA) as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    status + ctechnology_status::COUNTRY,
                    (country as u64).to_le_bytes(),
                );
            }
            array(&mut bytes, S1 + ctechnology_status::TECHNOLOGIES_DATA, ALL1, 3);
            array(&mut bytes, S1 + ctechnology_status::RESEARCHED_DATA, DONE1, 1);
            array(
                &mut bytes,
                S1 + ctechnology_status::RESEARCH_SLOTS_DATA,
                SLOTS1,
                2,
            );
            array(&mut bytes, S2 + ctechnology_status::TECHNOLOGIES_DATA, ALL2, 1);
            array(&mut bytes, S2 + ctechnology_status::RESEARCHED_DATA, DONE2, 0);
            array(
                &mut bytes,
                S2 + ctechnology_status::RESEARCH_SLOTS_DATA,
                SLOTS2,
                1,
            );
            put(&mut bytes, ALL1, (TECH_B as u64).to_le_bytes());
            put(&mut bytes, ALL1 + 16, (TECH_A as u64).to_le_bytes());
            put(&mut bytes, DONE1, (TECH_B as u64).to_le_bytes());
            put(&mut bytes, SLOTS1, (SLOT_B as u64).to_le_bytes());
            put(&mut bytes, SLOTS1 + 8, (SLOT_A as u64).to_le_bytes());
            put(&mut bytes, ALL2, (TECH_A2 as u64).to_le_bytes());
            put(&mut bytes, SLOTS2, (SLOT_C as u64).to_le_bytes());
            for (pointer, template, status, handle, classifier, level, progress) in [
                (TECH_A, TMPL_A, S1, 3_u32, 4_u32, -2_i32, i64::MIN + 9),
                (TECH_B, TMPL_B, S1, 17, 0x100, 9, i64::MAX - 8),
                (TECH_A2, TMPL_A, S2, 3, 4, 1, 42),
            ] {
                put(
                    &mut bytes,
                    pointer,
                    ((BASE + ctechnology::VTABLE_RVA) as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    pointer + ctechnology::DEFINITION_INDEX,
                    handle.to_le_bytes(),
                );
                put(
                    &mut bytes,
                    pointer + ctechnology::STATUS,
                    (status as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    pointer + ctechnology::TEMPLATE,
                    (template as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    pointer + ctechnology::CLASSIFIER,
                    classifier.to_le_bytes(),
                );
                put(&mut bytes, pointer + ctechnology::LEVEL, level.to_le_bytes());
                put(
                    &mut bytes,
                    pointer + ctechnology::LIVE_PROGRESS,
                    progress.to_le_bytes(),
                );
                put(
                    &mut bytes,
                    pointer + ctechnology::AHEAD_REDUCTION_YEARS,
                    (-8_123_456_789_i64).to_le_bytes(),
                );
            }
            put(&mut bytes, HEAP_ID, *b"tech_alpha");
            put(&mut bytes, HEAP_ID + 10, 0_u8.to_le_bytes());
            put(
                &mut bytes,
                TMPL_A + ctechnology_template::ID,
                (HEAP_ID as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                TMPL_A + ctechnology_template::ID + 16,
                10_u64.to_le_bytes(),
            );
            put(
                &mut bytes,
                TMPL_A + ctechnology_template::ID + 24,
                32_u64.to_le_bytes(),
            );
            put(
                &mut bytes,
                TMPL_A + ctechnology_template::DATABASE_INDEX,
                23_i32.to_le_bytes(),
            );
            put(
                &mut bytes,
                TMPL_A + ctechnology_template::TECHNOLOGY_HANDLE,
                3_u32.to_le_bytes(),
            );
            put(
                &mut bytes,
                TMPL_A + ctechnology_template::YEAR,
                1940_i32.to_le_bytes(),
            );
            put(
                &mut bytes,
                TMPL_A + ctechnology_template::MAX_LEVEL,
                12_i32.to_le_bytes(),
            );
            put(
                &mut bytes,
                TMPL_A + ctechnology_template::RESEARCH_COST,
                7_654_321_234_i64.to_le_bytes(),
            );
            put(&mut bytes, TMPL_B + ctechnology_template::ID, *b"tb12");
            put(
                &mut bytes,
                TMPL_B + ctechnology_template::ID + 16,
                4_u64.to_le_bytes(),
            );
            put(
                &mut bytes,
                TMPL_B + ctechnology_template::ID + 24,
                15_u64.to_le_bytes(),
            );
            put(
                &mut bytes,
                TMPL_B + ctechnology_template::DATABASE_INDEX,
                37_i32.to_le_bytes(),
            );
            put(
                &mut bytes,
                TMPL_B + ctechnology_template::TECHNOLOGY_HANDLE,
                17_u32.to_le_bytes(),
            );
            put(
                &mut bytes,
                TMPL_B + ctechnology_template::YEAR,
                1936_i32.to_le_bytes(),
            );
            put(
                &mut bytes,
                TMPL_B + ctechnology_template::MAX_LEVEL,
                5_i32.to_le_bytes(),
            );
            put(
                &mut bytes,
                TMPL_B + ctechnology_template::RESEARCH_COST,
                100_000_i64.to_le_bytes(),
            );
            for (pointer, technology, status, handle, index) in [
                (SLOT_A, TECH_A, S1, 3_u32, 0_u32),
                (SLOT_B, 0, S1, cresearch_slot::IDLE_HANDLE, 1),
                (SLOT_C, 0, S2, cresearch_slot::IDLE_HANDLE, 0),
            ] {
                put(
                    &mut bytes,
                    pointer,
                    ((BASE + cresearch_slot::VTABLE_RVA) as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    pointer + cresearch_slot::STATUS,
                    (status as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    pointer + cresearch_slot::TECHNOLOGY_HANDLE,
                    handle.to_le_bytes(),
                );
                put(
                    &mut bytes,
                    pointer + cresearch_slot::TECHNOLOGY,
                    (technology as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    pointer + cresearch_slot::INDEX,
                    index.to_le_bytes(),
                );
                put(
                    &mut bytes,
                    pointer + cresearch_slot::RESEARCH_SPEED,
                    5_123_456_789_i64.to_le_bytes(),
                );
                put(
                    &mut bytes,
                    pointer + cresearch_slot::BANKED_DAYS,
                    (-5_234_567_890_i64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    pointer + cresearch_slot::APPLIED_DAYS,
                    6_345_678_901_i64.to_le_bytes(),
                );
            }
            (bytes, Layout { base: BASE, game_state: GAME_STATE })
        }

        pub(crate) struct Direct {
            pub tech_rows: Vec<ctechnology::Row>,
            pub tech_outcome: Outcome,
            pub slot_rows: Vec<cresearch_slot::Row>,
            pub slot_outcome: Outcome,
            pub def_rows: Vec<ctechnology_template::Row>,
            pub def_outcome: Outcome,
            pub names: Vec<u8>,
            pub dict_required: usize,
        }

        /// Typed reference path: the existing `acquire` with pilot tables only.
        pub(crate) fn direct(
            memory: &impl Memory,
            layout: &Layout,
            tech_cap: usize,
            slot_cap: usize,
            def_cap: usize,
            dict_cap: usize,
            names_cap: usize,
        ) -> Direct {
            let mut tech = vec![ctechnology::Row::default(); tech_cap];
            let mut slots = vec![cresearch_slot::Row::default(); slot_cap];
            let mut defs = vec![ctechnology_template::Row::default(); def_cap];
            let mut names = vec![0; names_cap];
            let mut entries = vec![ctechnology_template::Entry::default(); dict_cap];
            let buffers = Buffers {
                focus: &mut [],
                completed: &mut [],
                technologies: &mut tech,
                slots: &mut slots,
                definitions: &mut defs,
                politics: &mut [],
                parties: &mut [],
                ideas: &mut [],
                counts: &mut [],
                decisions: &mut [],
                modifiers: &mut [],
                tactics: &mut [],
                operands: &mut [],
                sharing: &mut [],
                sharing_members: &mut [],
                balances: &mut [],
                balance_members: &mut [],
                ranges: &mut [],
                overrides: &mut [],
                focus_text: &mut [],
                definition_text: &mut names,
                party_text: &mut [],
                idea_text: &mut [],
                decision_text: &mut [],
                modifier_text: &mut [],
                modifier_values: &mut [],
                tactic_text: &mut [],
                balance_text: &mut [],
                party_countries: &mut [],
                idea_countries: &mut [],
                decision_countries: &mut [],
                dictionary: &mut entries,
            };
            let mut enabled = [false; N];
            enabled[2] = true;
            enabled[3] = true;
            let observation =
                acquire(memory, layout.base, layout.game_state, buffers, &enabled);
            Direct {
                tech_rows: tech[..observation.outcomes[2].len()].to_vec(),
                tech_outcome: observation.outcomes[2],
                slot_rows: slots[..observation.outcomes[3].len()].to_vec(),
                slot_outcome: observation.outcomes[3],
                def_rows: defs[..observation.outcomes[4].len()].to_vec(),
                def_outcome: observation.outcomes[4],
                names: names[..observation.lens[1]].to_vec(),
                dict_required: observation.dictionary_required,
            }
        }

        /// Split path under test: raw copy on `memory`, decode into `pilot`.
        pub(crate) fn split(
            memory: &impl Memory,
            layout: &Layout,
            pilot: &mut Captured,
            with_second: bool,
        ) -> (CopyReport, ResearchJob) {
            let mut needed = [false; TABLE_COUNT];
            needed[TABLES[2].index()] = true;
            needed[TABLES[3].index()] = true;
            needed[TABLES[4].index()] = true;
            let mut first = ResearchJob::new();
            let mut second = with_second.then(|| Box::new(ResearchJob::new()));
            let ctx = CopyContext {
                base: layout.base,
                game_state: layout.game_state,
                tech_cap: pilot.tech_rows_cap(),
                slot_cap: pilot.slot_rows_cap(),
                needed,
            };
            let report = pilot.copy_research_refresh(
                memory,
                ctx,
                &mut first,
                second.as_mut().map(|job| job.as_mut()),
            );
            assert!(report.raw_ok);
            let mut ticks = [0; 3];
            pilot.decode_research_payload(layout.base, &first, &mut ticks);
            (report, first)
        }

        pub(crate) fn assert_matches_direct(pilot: &Captured, direct: &Direct) {
            assert_eq!(pilot.technologies.outcome, direct.tech_outcome);
            assert_eq!(
                &pilot.technologies.rows[..direct.tech_outcome.len()],
                direct.tech_rows.as_slice()
            );
            assert_eq!(pilot.slots.outcome, direct.slot_outcome);
            assert_eq!(
                &pilot.slots.rows[..direct.slot_outcome.len()],
                direct.slot_rows.as_slice()
            );
            assert_eq!(pilot.definitions.outcome, direct.def_outcome);
            assert_eq!(
                &pilot.definitions.rows[..direct.def_outcome.len()],
                direct.def_rows.as_slice()
            );
            assert_eq!(
                &pilot.definition_text.first[..pilot.definition_text.len],
                direct.names.as_slice()
            );
            assert_eq!(pilot.dictionary_required, direct.dict_required);
        }

        pub(crate) fn pilot_captured() -> Captured {
            Captured::with_capacities(
                &Capacities::default(),
                &[CONSISTENCY_TARGET; TABLE_COUNT],
            )
        }
    }

    use research_test_kit as kit;

    fn kit_memory(bytes: &[u8]) -> engine::memory::SliceMemory<'_> {
        kit::memory(bytes)
    }

    #[test]
    fn copy_decode_matches_typed_rows_including_holes_order_and_shared_templates() {
        let (bytes, layout) = kit::fixture();
        let memory = kit_memory(&bytes);
        let direct = kit::direct(&memory, &layout, 8, 4, 8, 32, 64);
        assert_eq!(direct.tech_outcome, Outcome::Rows { len: 3 });
        // Native roster order is retained; the researched flag rides the
        // independent census, not the classifier.
        assert_eq!(
            direct
                .tech_rows
                .iter()
                .map(|row| (row.country, row.technology_handle, row.researched))
                .collect::<Vec<_>>(),
            [(2, 17, 1), (2, 3, 0), (5, 3, 0)]
        );
        assert_eq!(direct.slot_outcome, Outcome::Rows { len: 3 });
        assert_eq!(direct.def_outcome, Outcome::Rows { len: 2 });
        let mut pilot = kit::pilot_captured();
        let (report, _) = kit::split(&memory, &layout, &mut pilot, true);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 3]);
        assert!(report.raw_bytes.iter().all(|&bytes| bytes > 0));
        kit::assert_matches_direct(&pilot, &direct);
        // Checked bookkeeping mirrors the typed double read on a match.
        assert!(pilot.technologies.checked);
        assert_eq!(pilot.technologies.mismatches, 0);
        assert!(pilot.slots.checked);
        assert!(pilot.definitions.checked);
    }

    #[test]
    fn research_batches_instances_and_reads_shared_definitions_once_per_pass() {
        use engine::Memory;
        use std::cell::Cell;
        struct Counts<'a> {
            inner: engine::memory::SliceMemory<'a>,
            headers: Cell<usize>,
            batch: Cell<usize>,
        }
        impl Memory for Counts<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == kit::TMPL_A + ctechnology_template::ID && output.len() == 32 {
                    self.headers.set(self.headers.get() + 1);
                }
                self.inner.copy(address, output)
            }
            fn copy_records(&self, pointers: &[usize], fields: &[engine::memory::RecordField],
                stride: usize, output: &mut [u8]) -> usize {
                if stride == ctechnology::INSTANCE_BYTES {
                    self.batch.set(self.batch.get().max(pointers.len()));
                }
                // Preserve field-order failures while keeping reads visible
                // to the definition-header counter above.
                let mut completed = 0;
                for (index, pointer) in pointers.iter().enumerate() {
                    for field in fields {
                        let Some(address) = pointer.checked_add(field.source_offset) else { return completed; };
                        let start = index * stride + field.destination_offset;
                        if !self.copy(address, &mut output[start..start + field.len]) { return completed; }
                        completed += 1;
                    }
                }
                completed
            }
        }
        let (bytes, layout) = kit::fixture();
        let memory = Counts { inner: kit_memory(&bytes), headers: Cell::new(0), batch: Cell::new(0) };
        let direct = kit::direct(&kit_memory(&bytes), &layout, 8, 4, 8, 32, 64);
        let mut pilot = kit::pilot_captured();
        let (report, _) = kit::split(&memory, &layout, &mut pilot, true);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 3]);
        kit::assert_matches_direct(&pilot, &direct);
        assert_eq!(memory.headers.get(), 2, "one shared header in each independent pass");
        assert_eq!(memory.batch.get(), 2, "both non-null first-country instances share a gather");
    }

    #[test]
    fn research_copy_sizes_cover_documented_read_set() {
        // S3: every Memory::copy size the research copy pass issues,
        // recorded through the real pass. Array headers (16), words (4/8),
        // msvc_string headers (32) with heap payloads and terminators (1),
        // token handles and template dictionary entries. Alignment follows
        // the engine layout (headers 8-aligned, strings byte-packed).
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
        let (bytes, layout) = kit::fixture();
        let inner = kit::memory(&bytes);
        let memory = Recorder { inner, sizes: std::cell::RefCell::new(BTreeSet::new()) };
        let mut pilot = kit::pilot_captured();
        let mut needed = [false; TABLE_COUNT];
        needed[TABLES[2].index()] = true;
        needed[TABLES[3].index()] = true;
        needed[TABLES[4].index()] = true;
        let ctx = CopyContext {
            base: layout.base,
            game_state: layout.game_state,
            tech_cap: pilot.tech_rows_cap(),
            slot_cap: pilot.slot_rows_cap(),
            needed,
        };
        let mut first = ResearchJob::new();
        let report = pilot.copy_research_refresh(&memory, ctx, &mut first, None);
        assert!(report.raw_ok);
        let sizes = memory.sizes.borrow().clone();
        for required in [1, 4, 8, 16, 32] {
            assert!(sizes.contains(&required), "missing size {required} in {sizes:?}");
        }
    }

    #[test]
    fn research_truncation_matrix_matches_direct_with_same_caps() {
        // B3: zero/one/exact-fit/overflow-by-one/massive run typed AND
        // copy→decode against the SAME memory with the SAME pooled caps,
        // asserting identical outcome (incl. len/unread) and identical rows.
        // Fixture holds 3 technologies, 3 slots, 2 deduplicated definitions
        // (TECH_A and TECH_A2 share TMPL_A); tech cap 2 is the
        // overflow-by-one arm (total 3 == cap+1), def cap 1 the overflow arm
        // (total 2 == cap+1), cap 0 the zero/massive arm. u32 unread
        // saturation is not reachable and is covered by the walk's own
        // saturating arithmetic. Dedup arms vary the template dictionary
        // capacity with rows ample, proving shared-template accounting
        // agrees on both paths.
        let (bytes, layout) = kit::fixture();
        let memory = kit_memory(&bytes);
        for (tech, slot, def) in [(0, 0, 0), (1, 1, 1), (2, 2, 1), (3, 3, 2), (4, 4, 3), (8, 4, 8)] {
            let direct = kit::direct(&memory, &layout, tech, slot, def, 32, 64);
            let mut pilot = kit::pilot_captured();
            pilot.technologies.rows.truncate(tech);
            pilot.slots.rows.truncate(slot);
            pilot.definitions.rows.truncate(def);
            let (report, _) = kit::split(&memory, &layout, &mut pilot, true);
            assert!(report.raw_ok, "tech={tech} slot={slot} def={def}");
            kit::assert_matches_direct(&pilot, &direct);
        }
        // Exact-fit arm pins Rows with all instances present.
        let direct = kit::direct(&memory, &layout, 3, 3, 2, 32, 64);
        assert_eq!(direct.tech_outcome, Outcome::Rows { len: 3 });
        assert_eq!(direct.slot_outcome, Outcome::Rows { len: 3 });
        assert_eq!(direct.def_outcome, Outcome::Rows { len: 2 });
        // Dedup arms: dictionary capacity pressure fails definitions
        // identically without losing instances.
        for dict in [0, 1, 2, 32] {
            let direct = kit::direct(&memory, &layout, 8, 4, 8, dict, 64);
            let mut pilot = kit::pilot_captured();
            pilot.dictionary.truncate(dict);
            pilot.dictionary_second.truncate(dict);
            let (report, _) = kit::split(&memory, &layout, &mut pilot, true);
            assert!(report.raw_ok, "dict={dict}");
            kit::assert_matches_direct(&pilot, &direct);
        }
    }

    #[test]
    fn copy_decode_matches_truncation_and_exact_required_counts() {
        let (bytes, layout) = kit::fixture();
        let memory = kit_memory(&bytes);
        let direct = kit::direct(&memory, &layout, 1, 1, 1, 32, 64);
        // Each country truncates against its remaining slice; the second
        // country's empty remainder still counts its omitted row.
        assert_eq!(
            direct.tech_outcome,
            Outcome::Truncated { len: 1, unread_rows: 2 }
        );
        assert_eq!(
            direct.slot_outcome,
            Outcome::Truncated { len: 1, unread_rows: 2 }
        );
        assert_eq!(
            direct.def_outcome,
            Outcome::Unavailable {
                reason: "technology definition source census incomplete"
            }
        );
        let mut pilot = kit::pilot_captured();
        pilot.technologies.rows.truncate(1);
        pilot.slots.rows.truncate(1);
        pilot.definitions.rows.truncate(1);
        let (report, _) = kit::split(&memory, &layout, &mut pilot, true);
        assert!(report.raw_ok);
        kit::assert_matches_direct(&pilot, &direct);
    }

    #[test]
    fn copy_decode_matches_faults_with_identical_reasons() {
        // Each mutation faults one partition; both paths must agree on the
        // outcome reason while the other partitions stay readable.
        let cases: &[(&str, usize, u64)] = &[
            ("status vtable", kit::S1, 0),
            ("tech header", kit::ALL1 + 8, 0),
            ("instance vtable", kit::TECH_A, 0),
            ("researched count", kit::DONE1 + 12, 99),
            ("slot header", kit::SLOTS1 + 8, 0),
            ("slot vtable", kit::SLOT_A, 0),
            ("template handle", kit::TMPL_A + 60, 99),
        ];
        for (name, address, value) in cases {
            let (mut bytes, layout) = kit::fixture();
            let offset = address - kit::C1;
            bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
            let memory = kit_memory(&bytes);
            let direct = kit::direct(&memory, &layout, 8, 4, 8, 32, 64);
            let mut pilot = kit::pilot_captured();
            let (report, _) = kit::split(&memory, &layout, &mut pilot, true);
            assert!(report.raw_ok, "{name}");
            assert_eq!(pilot.technologies.outcome, direct.tech_outcome, "{name}");
            assert_eq!(pilot.slots.outcome, direct.slot_outcome, "{name}");
            assert_eq!(pilot.definitions.outcome, direct.def_outcome, "{name}");
            if matches!(direct.tech_outcome, Outcome::Rows { .. }) {
                assert_eq!(
                    &pilot.technologies.rows[..direct.tech_outcome.len()],
                    direct.tech_rows.as_slice(),
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn researched_second_census_faults_match() {
        // Null researched pointer invalidates the whole technology table.
        let (mut bytes, layout) = kit::fixture();
        for offset in 0..8 {
            bytes[kit::DONE1 - kit::C1 + offset] = 0;
        }
        let memory = kit_memory(&bytes);
        let direct = kit::direct(&memory, &layout, 8, 4, 8, 32, 64);
        assert_eq!(
            direct.tech_outcome,
            Outcome::Unavailable {
                reason: "researched technology pointer unreadable or null"
            }
        );
        let mut pilot = kit::pilot_captured();
        let (_, _) = kit::split(&memory, &layout, &mut pilot, true);
        assert_eq!(pilot.technologies.outcome, direct.tech_outcome);
        // Progress torn between the roster and the census disagrees: point the
        // researched roster at a clone of B with different progress, as a
        // same-hour mutation would.
        let (mut bytes, layout) = kit::fixture();
        const CLONE_B: usize = 0x4400;
        let from = kit::TECH_B - kit::C1;
        let to = CLONE_B - kit::C1;
        bytes.copy_within(from..from + 0x200, to);
        bytes[to + ctechnology::LIVE_PROGRESS] ^= 0xff;
        bytes[kit::DONE1 - kit::C1..kit::DONE1 - kit::C1 + 8]
            .copy_from_slice(&(CLONE_B as u64).to_le_bytes());
        let memory = kit_memory(&bytes);
        let direct = kit::direct(&memory, &layout, 8, 4, 8, 32, 64);
        assert_eq!(
            direct.tech_outcome,
            Outcome::Unavailable {
                reason: "researched technology disagrees with country roster"
            }
        );
        let mut pilot = kit::pilot_captured();
        let (_, _) = kit::split(&memory, &layout, &mut pilot, true);
        assert_eq!(pilot.technologies.outcome, direct.tech_outcome);
    }

    #[test]
    fn slot_identity_faults_match() {
        // Idle slot with a non-idle handle, duplicate ordinals, and a bad
        // ordinal each invalidate the slots table identically on both paths.
        let idle = kit::SLOT_B - kit::C1 + cresearch_slot::TECHNOLOGY_HANDLE;
        let index_a = kit::SLOT_A - kit::C1 + cresearch_slot::INDEX;
        let cases: &[(&str, usize, u32)] = &[
            ("idle handle", idle, 7),
            ("duplicate ordinal", index_a, 1),
            ("bad ordinal", index_a, 99),
        ];
        for (name, offset, value) in cases {
            let (mut bytes, layout) = kit::fixture();
            bytes[*offset..*offset + 4].copy_from_slice(&value.to_le_bytes());
            let memory = kit_memory(&bytes);
            let direct = kit::direct(&memory, &layout, 8, 4, 8, 32, 64);
            assert!(
                matches!(direct.slot_outcome, Outcome::Unavailable { .. }),
                "{name}"
            );
            let mut pilot = kit::pilot_captured();
            let (_, _) = kit::split(&memory, &layout, &mut pilot, true);
            assert_eq!(pilot.slots.outcome, direct.slot_outcome, "{name}");
            assert_eq!(pilot.technologies.outcome, direct.tech_outcome, "{name}");
        }
    }

    #[test]
    fn dictionary_pressure_matches() {
        // Tiny dictionary capacity fails definitions without losing instances.
        let (bytes, layout) = kit::fixture();
        let memory = kit_memory(&bytes);
        let direct = kit::direct(&memory, &layout, 8, 4, 8, 4, 64);
        assert!(matches!(
            direct.def_outcome,
            Outcome::Unavailable { .. }
        ));
        assert_eq!(direct.tech_outcome, Outcome::Rows { len: 3 });
        let mut pilot = kit::pilot_captured();
        pilot.dictionary.truncate(4);
        pilot.dictionary_second.truncate(4);
        let (_, _) = kit::split(&memory, &layout, &mut pilot, true);
        assert_eq!(pilot.definitions.outcome, direct.def_outcome);
        assert_eq!(pilot.technologies.outcome, direct.tech_outcome);
        // Small name arenas truncate definitions with exact required bytes.
        let direct = kit::direct(&memory, &layout, 8, 4, 8, 32, 10);
        assert_eq!(
            direct.def_outcome,
            Outcome::Truncated { len: 1, unread_rows: 1 }
        );
        assert_eq!(direct.names, b"tb12");
        let mut pilot = kit::pilot_captured();
        pilot.definition_text.first.truncate(10);
        let (_, _) = kit::split(&memory, &layout, &mut pilot, true);
        kit::assert_matches_direct(&pilot, &direct);
        // Growth accounting stays safe: the split may retain redundant
        // per-occurrence id bytes from its second pass, so required only
        // needs to cover the typed walk's exact need, never less.
        assert!(pilot.definition_text.required >= 14);
    }

    #[test]
    fn raw_compare_matches_per_partition_and_counts_mismatch_once() {
        let (bytes, layout) = kit::fixture();
        let memory = kit_memory(&bytes);
        let mut pilot = kit::pilot_captured();
        let (report, _) = kit::split(&memory, &layout, &mut pilot, true);
        assert_eq!(report.matched, [true; 3]);
        // One flipped TECH_B byte faults only the technology partition: TECH_B
        // is never slot-linked and its template spans are untouched. The two
        // passes read different frozen memories, as live tearing would show
        // them at different times.
        let (mut mutated, layout) = kit::fixture();
        mutated[kit::TECH_B - kit::C1 + ctechnology::LIVE_PROGRESS] ^= 0x01;
        let (clean, _) = kit::fixture();
        let first_memory = kit_memory(&mutated);
        let second_memory = kit_memory(&clean);
        let pilot = kit::pilot_captured();
        let tech_cap = pilot.tech_rows_cap();
        let slot_cap = pilot.slot_rows_cap();
        let mut first = ResearchJob::new();
        let mut second = ResearchJob::new();
        copy_research_pass(
            &first_memory,
            layout.base,
            layout.game_state,
            tech_cap,
            slot_cap,
            &mut first,
        );
        copy_research_pass(
            &second_memory,
            layout.base,
            layout.game_state,
            tech_cap,
            slot_cap,
            &mut second,
        );
        assert!(!tech_partition_equal(&first, &second));
        assert!(slots_partition_equal(&first, &second));
        assert!(definitions_partition_equal(&first, &second));
        // The full method wires the same verdict into the existing mismatch
        // bookkeeping: technologies checked once, slots and definitions clean.
        // TECH_B progress is read twice per pass (roster plus researched
        // census) and never appears in slot or template spans, so flipping
        // its third read replays a same-hour live mutation confined to the
        // technology partition.
        struct Tearing<'a> {
            clean: engine::memory::SliceMemory<'a>,
            mutated: engine::memory::SliceMemory<'a>,
            progress_reads: std::cell::Cell<usize>,
        }
        impl engine::Memory for Tearing<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                let progress = kit::TECH_B + ctechnology::LIVE_PROGRESS;
                if address <= progress && progress < address + destination.len() {
                    let reads = self.progress_reads.get();
                    self.progress_reads.set(reads + 1);
                    let memory = if reads < 2 { &self.clean } else { &self.mutated };
                    return memory.copy(address, destination);
                }
                self.clean.copy(address, destination)
            }
        }
        let tearing = Tearing {
            clean: kit_memory(&clean),
            mutated: kit_memory(&mutated),
            progress_reads: std::cell::Cell::new(0),
        };
        let mut pilot = kit::pilot_captured();
        let mut first = ResearchJob::new();
        let mut second = ResearchJob::new();
        let mut needed = [false; TABLE_COUNT];
        needed[TABLES[2].index()] = true;
        needed[TABLES[3].index()] = true;
        needed[TABLES[4].index()] = true;
        let tech_cap = pilot.tech_rows_cap();
        let slot_cap = pilot.slot_rows_cap();
        let ctx = CopyContext {
            base: layout.base,
            game_state: layout.game_state,
            tech_cap,
            slot_cap,
            needed,
        };
        let report = pilot.copy_research_refresh(
            &tearing,
            ctx,
            &mut first,
            Some(&mut second),
        );
        assert!(report.raw_ok);
        assert_eq!(report.matched, [false, true, true]);
        assert_eq!(pilot.technologies.mismatches, 1);
        assert_eq!(pilot.slots.mismatches, 0);
        assert_eq!(pilot.definitions.mismatches, 0);
        // Without a second copy no check is recorded, mirroring completed
        // windows: nothing checked, nothing mismatched.
        let mut pilot = kit::pilot_captured();
        let mut first = ResearchJob::new();
        let needed = [false; TABLE_COUNT];
        let tech_cap = pilot.tech_rows_cap();
        let slot_cap = pilot.slot_rows_cap();
        let ctx = CopyContext {
            base: layout.base,
            game_state: layout.game_state,
            tech_cap,
            slot_cap,
            needed,
        };
        let report = pilot.copy_research_refresh(&second_memory, ctx, &mut first, None);
        assert!(report.raw_ok);
        assert!(!pilot.technologies.checked);
        assert_eq!(pilot.technologies.mismatches, 0);
    }

    #[test]
    fn stable_but_invalid_bytes_count_the_check_with_unavailable_rows() {
        // Readable-but-wrong vtable faults decode deterministically on both
        // copies: outcomes agree as Unavailable while the check still counts
        // the refresh (documented window nuance, pinned by this test).
        let (mut bytes, layout) = kit::fixture();
        bytes[kit::S1 - kit::C1] ^= 0xff;
        let memory = kit_memory(&bytes);
        let direct = kit::direct(&memory, &layout, 8, 4, 8, 32, 64);
        assert!(matches!(
            direct.tech_outcome,
            Outcome::Unavailable { .. }
        ));
        let mut pilot = kit::pilot_captured();
        let (report, _) = kit::split(&memory, &layout, &mut pilot, true);
        assert!(report.matched[0]);
        assert_eq!(pilot.technologies.outcome, direct.tech_outcome);
    }

    #[test]
    fn staging_overflow_reports_fallback_without_blocking() {
        let (bytes, layout) = kit::fixture();
        let memory = kit_memory(&bytes);
        let mut pilot = kit::pilot_captured();
        // Zero-capacity staging cannot hold any copy: the report declines the
        // payload so the caller falls back to the typed walk.
        let mut tiny = ResearchJob {
            metas: Vec::new(),
            tech_pool: Vec::new(),
            researched_pool: Vec::new(),
            templates: Vec::new(),
            template_index: Vec::new(),
            template_seen_country: Vec::new(),
            id_pool: Vec::new(),
            slots: Vec::new(),
            reader_err: None,
            country_err: None,
            second_max_handle_plus_one: 0,
            second_id_bytes: 0,
            overflowed: false,
        };
        let mut needed = [false; TABLE_COUNT];
        needed[TABLES[2].index()] = true;
        let tech_cap = pilot.tech_rows_cap();
        let slot_cap = pilot.slot_rows_cap();
        let ctx = CopyContext {
            base: layout.base,
            game_state: layout.game_state,
            tech_cap,
            slot_cap,
            needed,
        };
        let report = pilot.copy_research_refresh(&memory, ctx, &mut tiny, None);
        assert!(!report.raw_ok);
        // The typed path still reads the same fixture cleanly.
        let direct = kit::direct(&memory, &layout, 8, 4, 8, 32, 64);
        assert_eq!(direct.tech_outcome, Outcome::Rows { len: 3 });
    }

    /// Shared synthetic address space for the hourly focus copy/decode split
    /// harness: two authenticated countries with inline, absent and heap focus
    /// tokens plus one- and two-entry completed rosters.
    pub(crate) mod focus_test_kit {
        use super::{Capacities, Captured, FocusCopyContext, FocusJob};
        use engine::memory::SliceMemory;
        use engine::{
            ccountry, ccontinuous_national_focus, cgame_state, cnational_focus,
            cnational_focus_progress, cnational_focus_tree,
        };
        use replay::frame::TABLE_COUNT;

        pub(crate) const BASE: usize = 0x1400_0000;
        const MEM: usize = 0x1_0000;
        const GS: usize = 0x1_0000;
        const CTAB: usize = 0x1_1000;
        const LTAB: usize = 0x1_1200;
        const C1: usize = 0x1_2000;
        const C2: usize = 0x1_4000;
        const R1: usize = 0x1_6000;
        const R2: usize = 0x1_7000;
        const TT1: usize = 0x1_8000;
        const FC1: usize = 0x1_8200;
        const FC2: usize = 0x1_8300;
        const CC1: usize = 0x1_8400;
        const FA: usize = 0x1_8500;
        const FB: usize = 0x1_8600;
        const FC: usize = 0x1_8700;
        const RS1: usize = 0x1_8800;
        const RS2: usize = 0x1_8900;
        const HB: usize = 0x1_8a00;
        const HEAP_TOKEN: &[u8] = b"completed_heap_token_longer_than_fifteen";

        pub(crate) struct Fixture {
            pub bytes: Vec<u8>,
        }

        impl Fixture {
            pub(crate) fn w64(&mut self, addr: usize, value: u64) {
                let at = addr - MEM;
                self.bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
            }

            pub(crate) fn w32(&mut self, addr: usize, value: u32) {
                let at = addr - MEM;
                self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }

            fn inline(&mut self, addr: usize, value: &[u8]) {
                assert!(value.len() <= 15);
                let at = addr - MEM;
                self.bytes[at..at + 32].fill(0);
                self.bytes[at..at + value.len()].copy_from_slice(value);
                self.w64(addr + 16, value.len() as u64);
                self.w64(addr + 24, 15);
            }

            fn heap_token(&mut self, addr: usize, value: &[u8]) {
                let at = addr - MEM;
                self.bytes[at..at + 32].fill(0);
                self.w64(addr, HB as u64);
                self.w64(addr + 16, value.len() as u64);
                self.w64(addr + 24, value.len() as u64);
                let heap = HB - MEM;
                self.bytes[heap..heap + value.len()].copy_from_slice(value);
                self.bytes[heap + value.len()] = 0;
            }

            fn country(&mut self, country: usize, id: u32, tree: usize, root: usize) {
                self.w64(country, (BASE + ccountry::VTABLE_RVA) as u64);
                self.w32(country + ccountry::ID, id);
                self.w64(country + ccountry::FOCUS_TREE, tree as u64);
                self.w64(country + ccountry::FOCUS_PROGRESS, root as u64);
            }

            #[allow(clippy::too_many_arguments)]
            fn root(
                &mut self,
                root: usize,
                current: usize,
                continuous: usize,
                progress: i64,
                paused: u8,
                count: i32,
                roster: usize,
                roster_count: u32,
            ) {
                self.w64(root, (BASE + cnational_focus_progress::VTABLE_RVA) as u64);
                self.w64(root + cnational_focus_progress::CURRENT, current as u64);
                self.w64(
                    root + cnational_focus_progress::CURRENT + 8,
                    continuous as u64,
                );
                self.w64(root + cnational_focus_progress::PROGRESS, progress as u64);
                self.bytes[root - MEM + cnational_focus_progress::PAUSED] = paused;
                self.w32(root + cnational_focus_progress::COMPLETED_COUNT, count as u32);
                self.w64(root + cnational_focus_progress::COMPLETED_DATA, roster as u64);
                self.w32(
                    root + cnational_focus_progress::COMPLETED_DATA + 8,
                    roster_count,
                );
                self.w32(
                    root + cnational_focus_progress::COMPLETED_DATA + 12,
                    roster_count,
                );
            }

            pub(crate) fn world() -> Self {
                let mut fixture = Self {
                    bytes: vec![0; 0x1_0000],
                };
                fixture.w64(GS + cgame_state::COUNTRIES, CTAB as u64);
                fixture.w32(GS + cgame_state::COUNTRIES + 8, 4);
                fixture.w32(GS + cgame_state::COUNTRIES + 12, 3);
                fixture.w64(GS + cgame_state::COUNTRY_LINKS, LTAB as u64);
                fixture.w32(GS + cgame_state::COUNTRY_LINKS + 8, 4);
                fixture.w32(GS + cgame_state::COUNTRY_LINKS + 12, 3);
                fixture.w64(CTAB, 0);
                fixture.w64(CTAB + 8, C1 as u64);
                fixture.w64(CTAB + 16, C2 as u64);
                fixture.w32(LTAB, 0);
                fixture.w32(LTAB + 4, 1);
                fixture.w32(LTAB + 8, 2);
                fixture.country(C1, 1, TT1, R1);
                fixture.country(C2, 2, 0, R2);
                fixture.inline(TT1 + cnational_focus_tree::TOKEN, b"tree_one");
                fixture.root(R1, FC1, CC1, 12_345, 1, 1, RS1, 1);
                fixture.inline(FC1 + cnational_focus::TOKEN, b"active_one");
                fixture.inline(CC1 + ccontinuous_national_focus::TOKEN, b"cont_one");
                fixture.inline(FA + cnational_focus::TOKEN, b"done_a");
                fixture.w64(RS1, FA as u64);
                fixture.root(R2, FC2, 0, -7, 0, 2, RS2, 2);
                fixture.inline(FC2 + cnational_focus::TOKEN, b"active_two");
                fixture.heap_token(FB + cnational_focus::TOKEN, HEAP_TOKEN);
                fixture.inline(FC + cnational_focus::TOKEN, b"done_c");
                fixture.w64(RS2, FB as u64);
                fixture.w64(RS2 + 8, FC as u64);
                fixture
            }

            pub(crate) fn memory(&self) -> SliceMemory<'_> {
                SliceMemory {
                    base: MEM,
                    bytes: &self.bytes,
                }
            }

            pub(crate) fn captured() -> Captured {
                Captured::with_capacities(&Capacities::default(), &[0u16; TABLE_COUNT])
            }

            pub(crate) fn copy_first(
                captured: &mut Captured,
                memory: &impl engine::Memory,
                second: Option<&mut FocusJob>,
            ) -> (super::FocusCopyReport, FocusJob) {
                let mut first = FocusJob::new();
                let ctx = FocusCopyContext {
                    base: BASE,
                    game_state: GS,
                    needed: [true, true],
                };
                let report = captured.copy_focus_refresh(memory, ctx, &mut first, second);
                (report, first)
            }

            /// Byte-identity assertion for the decode worker tests: typed
            /// rows, outcomes and arena bytes match exactly.
            pub(crate) fn assert_matches_typed(decoded: &Captured, typed: &Captured) {
                assert_eq!(decoded.focus.outcome, typed.focus.outcome);
                assert_eq!(decoded.completed.outcome, typed.completed.outcome);
                let focus_len = typed.focus.outcome.len();
                assert_eq!(
                    &decoded.focus.rows[..focus_len],
                    &typed.focus.rows[..focus_len]
                );
                let completed_len = typed.completed.outcome.len();
                assert_eq!(
                    &decoded.completed.rows[..completed_len],
                    &typed.completed.rows[..completed_len]
                );
                assert_eq!(decoded.focus_text.len, typed.focus_text.len);
                assert_eq!(decoded.focus_text.required, typed.focus_text.required);
                assert_eq!(
                    &decoded.focus_text.first[..decoded.focus_text.len],
                    &typed.focus_text.first[..typed.focus_text.len]
                );
            }
        }
    }

    #[test]
    fn focus_copy_decode_is_byte_identical_with_truncation() {
        use focus_test_kit::Fixture;
        let fixture = Fixture::world();
        let memory = fixture.memory();
        let needed = [true; TABLE_COUNT];
        let mut typed = Fixture::captured();
        typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
        assert_eq!(typed.focus.outcome, Outcome::Rows { len: 2 });
        assert_eq!(typed.completed.outcome, Outcome::Rows { len: 3 });
        assert_eq!(typed.focus_text.len, 88);
        assert_eq!(typed.focus_text.required, 88);
        // Heap token bytes survive exactly, including the long payload.
        let heap = b"completed_heap_token_longer_than_fifteen";
        assert!(
            typed.focus_text.first[..88]
                .windows(heap.len())
                .any(|window| window == heap)
        );

        let mut replayed = Fixture::captured();
        replayed.focus_raw_mode = true;
        let (report, first) = Fixture::copy_first(&mut replayed, &memory, None);
        assert!(report.raw_ok);
        replayed.capture(&memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
        let mut ticks = [0; 2];
        replayed.decode_focus_payload(focus_test_kit::BASE, &first, &mut ticks);
        assert!(!replayed.focus_raw_mode);
        assert_eq!(replayed.focus.outcome, typed.focus.outcome);
        assert_eq!(replayed.completed.outcome, typed.completed.outcome);
        let focus_len = typed.focus.outcome.len();
        assert_eq!(
            &typed.focus.rows[..focus_len],
            &replayed.focus.rows[..focus_len]
        );
        let completed_len = typed.completed.outcome.len();
        assert_eq!(
            &typed.completed.rows[..completed_len],
            &replayed.completed.rows[..completed_len]
        );
        assert_eq!(replayed.focus_text.len, typed.focus_text.len);
        assert_eq!(replayed.focus_text.required, typed.focus_text.required);
        assert_eq!(
            &replayed.focus_text.first[..replayed.focus_text.len],
            &typed.focus_text.first[..typed.focus_text.len]
        );

        // Truncated pooled buffers truncate identically with exact demand.
        let mut small = Capacities::default();
        small.rows[0] = 1;
        small.rows[1] = 1;
        small.focus_text = 20;
        let mut typed_small =
            Captured::with_capacities(&small, &[0u16; TABLE_COUNT]);
        typed_small.capture(&memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
        let mut replayed_small =
            Captured::with_capacities(&small, &[0u16; TABLE_COUNT]);
        replayed_small.focus_raw_mode = true;
        let (_, small_first) = Fixture::copy_first(&mut replayed_small, &memory, None);
        replayed_small.capture(&memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
        let mut small_ticks = [0; 2];
        replayed_small.decode_focus_payload(focus_test_kit::BASE, &small_first, &mut small_ticks);
        assert_eq!(replayed_small.focus.outcome, typed_small.focus.outcome);
        assert_eq!(
            replayed_small.completed.outcome,
            typed_small.completed.outcome
        );
        assert!(matches!(
            replayed_small.focus.outcome,
            Outcome::Truncated { .. }
        ));
        assert_eq!(
            replayed_small.focus_text.required,
            typed_small.focus_text.required
        );
        assert_eq!(replayed_small.focus_text.len, typed_small.focus_text.len);
    }

    #[test]
    fn focus_truncation_matrix_matches_direct_with_same_caps() {
        // B3: zero/one/exact-fit/overflow-by-one/massive run typed AND
        // copy→decode against the SAME memory with the SAME pooled caps,
        // asserting identical outcome (incl. len/unread) and identical rows.
        // Fixture holds 2 focus rows and 3 completed rows; focus cap 1 is
        // the overflow-by-one arm (total 2 == cap+1), completed cap 2 the
        // overflow arm (total 3 == cap+1), cap 0 the zero/massive arm.
        // u32 unread saturation is not reachable and is covered by the
        // walk's own saturating arithmetic.
        use focus_test_kit::Fixture;
        let fixture = Fixture::world();
        let memory = fixture.memory();
        let needed = [true; TABLE_COUNT];
        for (focus_rows, completed_rows, text) in [
            (0, 0, 0),
            (1, 1, 20),
            (1, 2, 20),
            (2, 3, 88),
            (3, 4, 128),
            (64, 64, 8192),
        ] {
            let mut caps = Capacities::default();
            caps.rows[0] = focus_rows;
            caps.rows[1] = completed_rows;
            caps.focus_text = text;
            let mut typed = Captured::with_capacities(&caps, &[0u16; TABLE_COUNT]);
            typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
            let mut replayed = Captured::with_capacities(&caps, &[0u16; TABLE_COUNT]);
            replayed.focus_raw_mode = true;
            let (report, first) = Fixture::copy_first(&mut replayed, &memory, None);
            assert!(report.raw_ok, "focus={focus_rows} completed={completed_rows}");
            replayed.capture(&memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
            let mut ticks = [0; 2];
            replayed.decode_focus_payload(focus_test_kit::BASE, &first, &mut ticks);
            Fixture::assert_matches_typed(&replayed, &typed);
        }
        // Exact-fit arm pins Rows on both tables.
        let mut caps = Capacities::default();
        caps.rows[0] = 2;
        caps.rows[1] = 3;
        caps.focus_text = 88;
        let mut typed = Captured::with_capacities(&caps, &[0u16; TABLE_COUNT]);
        typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
        assert_eq!(typed.focus.outcome, Outcome::Rows { len: 2 });
        assert_eq!(typed.completed.outcome, Outcome::Rows { len: 3 });
    }

    #[test]
    fn focus_copy_decode_matches_faults_with_identical_reasons() {
        use focus_test_kit::Fixture;
        let needed = [true; TABLE_COUNT];
        // Null status root fails both tables with the same reason.
        let mut broken = Fixture::world();
        broken.w64(0x1_2000 + ccountry::FOCUS_PROGRESS, 0);
        let memory = broken.memory();
        let mut typed = Fixture::captured();
        typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
        assert!(matches!(
            typed.focus.outcome,
            Outcome::Unavailable { .. }
        ));
        let mut replayed = Fixture::captured();
        replayed.focus_raw_mode = true;
        let (report, first) = Fixture::copy_first(&mut replayed, &memory, None);
        assert!(report.raw_ok);
        replayed.capture(&memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
        let mut ticks = [0; 2];
        replayed.decode_focus_payload(focus_test_kit::BASE, &first, &mut ticks);
        assert_eq!(replayed.focus.outcome, typed.focus.outcome);
        assert_eq!(replayed.completed.outcome, typed.completed.outcome);

        // Vtable mismatch fails both tables together.
        let mut broken = Fixture::world();
        broken.w64(0x1_6000, 0);
        let memory = broken.memory();
        let mut typed = Fixture::captured();
        typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
        let mut replayed = Fixture::captured();
        replayed.focus_raw_mode = true;
        let (_, first) = Fixture::copy_first(&mut replayed, &memory, None);
        replayed.capture(&memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
        let mut ticks = [0; 2];
        replayed.decode_focus_payload(focus_test_kit::BASE, &first, &mut ticks);
        assert_eq!(replayed.focus.outcome, typed.focus.outcome);
        assert_eq!(replayed.completed.outcome, typed.completed.outcome);
        assert!(matches!(
            typed.focus.outcome,
            Outcome::Unavailable {
                reason: "focus status vtable mismatch"
            }
        ));

        // A bad summary token fails only the summary; completions survive.
        let mut broken = Fixture::world();
        broken.w64(0x1_8200 + cnational_focus::TOKEN + 16, 99);
        let memory = broken.memory();
        let mut typed = Fixture::captured();
        typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
        assert!(matches!(
            typed.focus.outcome,
            Outcome::Unavailable { .. }
        ));
        let mut replayed = Fixture::captured();
        replayed.focus_raw_mode = true;
        let (_, first) = Fixture::copy_first(&mut replayed, &memory, None);
        replayed.capture(&memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
        let mut ticks = [0; 2];
        replayed.decode_focus_payload(focus_test_kit::BASE, &first, &mut ticks);
        assert_eq!(replayed.focus.outcome, typed.focus.outcome);
        assert_eq!(replayed.completed.outcome, typed.completed.outcome);
        assert_eq!(typed.completed.outcome, Outcome::Rows { len: 3 });

        // A null completed entry fails only the roster; the summary survives.
        let mut broken = Fixture::world();
        broken.w64(0x1_8800, 0);
        let memory = broken.memory();
        let mut typed = Fixture::captured();
        typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
        assert_eq!(typed.focus.outcome, Outcome::Rows { len: 2 });
        assert!(matches!(
            typed.completed.outcome,
            Outcome::Unavailable { .. }
        ));
        let mut replayed = Fixture::captured();
        replayed.focus_raw_mode = true;
        let (_, first) = Fixture::copy_first(&mut replayed, &memory, None);
        replayed.capture(&memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
        let mut ticks = [0; 2];
        replayed.decode_focus_payload(focus_test_kit::BASE, &first, &mut ticks);
        assert_eq!(replayed.focus.outcome, typed.focus.outcome);
        assert_eq!(replayed.completed.outcome, typed.completed.outcome);
    }

    #[test]
    fn focus_raw_compare_counts_checks_and_blocks_tearing() {
        use focus_test_kit::Fixture;
        let fixture = Fixture::world();
        let memory = fixture.memory();
        let mut captured = Fixture::captured();
        let mut second = FocusJob::new();
        let (report, _) = Fixture::copy_first(&mut captured, &memory, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true, true]);
        assert!(captured.focus.checked && captured.completed.checked);
        assert_eq!(captured.focus.mismatches, 0);
        assert_eq!(captured.completed.mismatches, 0);

        // A torn second copy (one progress byte flips) mismatches both logs.
        let mut torn = Fixture::world();
        torn.bytes[0x1_6000 + cnational_focus_progress::PROGRESS - 0x1_0000] ^= 0xff;
        let torn_memory = torn.memory();
        let mut after = FocusJob::new();
        let mut after_ticks = [0; 2];
        copy_focus_pass(
            &mut after,
            &memory,
            focus_test_kit::BASE,
            0x1_0000,
            &mut after_ticks,
        );
        let mut after_torn = FocusJob::new();
        let mut torn_ticks = [0; 2];
        copy_focus_pass(
            &mut after_torn,
            &torn_memory,
            focus_test_kit::BASE,
            0x1_0000,
            &mut torn_ticks,
        );
        assert!(!after.log.logs_equal(&after_torn.log));
        // Per-partition isolation: tearing a focus-private span flips only
        // slot 0; the completed partition (slot 1) still agrees.
        let focus_idx = after
            .log
            .spans
            .iter()
            .position(|span| span.ok && span.mask == 0x01)
            .expect("focus-private span");
        let mut second_torn = FocusJob::new();
        let mut second_ticks = [0; 2];
        copy_focus_pass(
            &mut second_torn,
            &memory,
            focus_test_kit::BASE,
            0x1_0000,
            &mut second_ticks,
        );
        let start = second_torn.log.spans[focus_idx].start as usize;
        // Align by span order: both logs took the same sequence, so the
        // same index addresses the same logical read.
        second_torn.log.bytes[start] ^= 0xff;
        assert!(!after.log.partition_equal(&second_torn.log, 0));
        assert!(after.log.partition_equal(&second_torn.log, 1));
        // Identical images agree byte-for-byte.
        let mut repeat = FocusJob::new();
        let mut repeat_ticks = [0; 2];
        copy_focus_pass(
            &mut repeat,
            &memory,
            focus_test_kit::BASE,
            0x1_0000,
            &mut repeat_ticks,
        );
        assert!(after.log.logs_equal(&repeat.log));

        // A readable-but-invalid refresh in both copies still counts its
        // check while emitting the identical unavailable outcome.
        let mut invalid = Fixture::world();
        invalid.w64(0x1_6000, 0);
        let invalid_memory = invalid.memory();
        let mut captured = Fixture::captured();
        let mut second = FocusJob::new();
        let (report, first) =
            Fixture::copy_first(&mut captured, &invalid_memory, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true, true]);
        let needed = [true; TABLE_COUNT];
        let mut replayed = Fixture::captured();
        replayed.focus_raw_mode = true;
        replayed.capture(&invalid_memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
        let mut ticks = [0; 2];
        replayed.decode_focus_payload(focus_test_kit::BASE, &first, &mut ticks);
        let mut typed = Fixture::captured();
        typed.capture(&invalid_memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
        assert_eq!(replayed.focus.outcome, typed.focus.outcome);
    }

    #[test]
    fn focus_staging_overflow_reports_fallback_without_blocking() {
        use focus_test_kit::Fixture;
        let fixture = Fixture::world();
        let memory = fixture.memory();
        let mut captured = Fixture::captured();
        // Zero-capacity staging cannot hold any copy: the report declines the
        // payload so the caller falls back to the typed walk.
        let mut tiny = FocusJob {
            log: RawLog::new(0, 0),
            game_state: 0,
            census: Vec::new(),
        };
        let ctx = FocusCopyContext {
            base: focus_test_kit::BASE,
            game_state: 0x1_0000,
            needed: [true, true],
        };
        let report = captured.copy_focus_refresh(&memory, ctx, &mut tiny, None);
        assert!(!report.raw_ok);
        // The typed path still reads the same fixture cleanly.
        let needed = [true; TABLE_COUNT];
        let mut typed = Fixture::captured();
        typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
        assert_eq!(typed.focus.outcome, Outcome::Rows { len: 2 });
        assert_eq!(typed.completed.outcome, Outcome::Rows { len: 3 });
    }

    #[test]
    fn focus_walks_hourly_like_every_tier_h_table() {
        // Focus and CompletedFocuses are hourly Tier-H tables: every hour
        // walks, and branch starts and keyframes force a full refresh like
        // every other table.
        for hour in -8..8i32 {
            assert!(
                replay::cadence::should_walk(TableId::Focus, hour, false),
                "hour {hour}"
            );
            assert!(
                replay::cadence::should_walk(TableId::CompletedFocuses, hour, false),
                "hour {hour}"
            );
            assert!(replay::cadence::should_walk(TableId::Focus, hour, true));
            assert!(replay::cadence::should_walk(
                TableId::CompletedFocuses,
                hour,
                true
            ));
        }
    }
}
