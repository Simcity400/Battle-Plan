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
    cnational_focus_tree, cpdx_array::Array, cpolitical_party, cpolitical_status, cresearch_slot,
    ctechnology, ctechnology_sharing_group, ctechnology_status, ctechnology_template, msvc_string,
    power_balance_gfx_override, power_balance_manager, power_balance_range, power_balance_record,
    walk::Outcome,
};
#[cfg(test)]
use replay::frame::TABLE_COUNT;
use replay::{
    BranchId,
    frame::{TableInput, TableView},
    rows::{self, TableId, TableRows, TableSchema, tables},
};
use std::{
    mem::size_of,
    sync::{Arc, Weak},
};

pub const N: usize = 19;

#[derive(Default)]
struct NativeShadow<R> {
    rows: Vec<R>,
    output: Weak<TableRows>,
    hits: u64,
    misses: u64,
}

impl<R: Clone + PartialEq> NativeShadow<R> {
    fn clear(&mut self) {
        self.rows.clear();
        self.output = Weak::new();
    }

    fn equal(
        &mut self,
        table: TableId,
        current: &[R],
        cached: Option<&Arc<TableRows>>,
    ) -> Option<Arc<TableRows>> {
        let cached = cached?;
        let previous = self.output.upgrade()?;
        // Typed PartialEq examines fields, never uninitialized struct padding.
        // Native order is deliberately significant: reordered equal rows miss.
        if cached.table() != table || !Arc::ptr_eq(cached, &previous) || self.rows != current {
            return None;
        }
        self.hits = self.hits.saturating_add(1);
        Some(Arc::clone(cached))
    }

    fn record_miss(&mut self, outcome: Outcome) {
        if !matches!(
            outcome,
            Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON
            }
        ) {
            self.misses = self.misses.saturating_add(1);
        }
    }

    fn install(&mut self, source: &[R], output: &Arc<TableRows>) {
        self.rows.clear();
        self.rows.extend_from_slice(source);
        self.output = Arc::downgrade(output);
    }
}

/// Spool-owned conversion memo only. No native pointer, Captured lease, read
/// outcome, or consistency counter is retained or substituted for a new read.
#[derive(Default)]
pub(crate) struct ResearchShadow {
    branch: Option<BranchId>,
    technologies: NativeShadow<ctechnology::Row>,
    definitions: NativeShadow<ctechnology_template::Row>,
    definition_text: Vec<u8>,
    definition_required: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ResearchShadowStats {
    pub hits: u64,
    pub misses: u64,
    pub row_capacity_bytes: usize,
    pub arena_capacity_bytes: usize,
}

impl ResearchShadow {
    /// Current retained Vec capacity, not used length or allocator overhead.
    /// Counters survive branch invalidation and measure conversion decisions,
    /// including inputs later rejected or failed at encoding/writing.
    pub(crate) fn statistics(&self) -> [ResearchShadowStats; 2] {
        [
            ResearchShadowStats {
                hits: self.technologies.hits,
                misses: self.technologies.misses,
                row_capacity_bytes: self.technologies.rows.capacity()
                    * size_of::<ctechnology::Row>(),
                arena_capacity_bytes: 0,
            },
            ResearchShadowStats {
                hits: self.definitions.hits,
                misses: self.definitions.misses,
                row_capacity_bytes: self.definitions.rows.capacity()
                    * size_of::<ctechnology_template::Row>(),
                arena_capacity_bytes: self.definition_text.capacity(),
            },
        ]
    }

    fn clear_definitions(&mut self) {
        self.definitions.clear();
        self.definition_text.clear();
        self.definition_required = 0;
    }
}

/// The producer supplies only the current branch's complete staggered cache.
/// Weak identities bind each native shadow to that exact replay allocation.
pub(crate) struct ResearchConversion<'a> {
    shadow: &'a mut ResearchShadow,
    cached: [Option<&'a Arc<TableRows>>; 2],
}

impl<'a> ResearchConversion<'a> {
    pub(crate) fn new(
        branch: BranchId,
        shadow: &'a mut ResearchShadow,
        cached: [Option<&'a Arc<TableRows>>; 2],
    ) -> Self {
        if shadow.branch != Some(branch) {
            shadow.technologies.clear();
            shadow.clear_definitions();
            shadow.branch = Some(branch);
        }
        Self { shadow, cached }
    }
}

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
        let mut walk: [bool; N] =
            std::array::from_fn(|i| replay::cadence::should_walk(TABLES[i], hour, force_full));
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
        if !walk[2] {
            self.technologies.outcome = skipped;
        }
        if !walk[3] {
            self.slots.outcome = skipped;
        }
        if !walk[4] {
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

// Test-only re-export so the decode worker tests can reuse the focus harness.
#[cfg(test)]
pub(crate) use tests::focus_test_kit;

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
    capacity_charge: crate::raw_memory::Charge,
}

impl FocusJob {
    pub fn new() -> Self {
        let mut job = Self {
            // Steady state is 439 countries with about 3.9k completed rows
            // plus token bytes; both pools start above that so steady state
            // never overflows.
            log: RawLog::new(1 << 16, 4 << 20),
            game_state: 0,
            census: Vec::with_capacity(512),
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        job.update_capacity_charge();
        job
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
        self.update_capacity_charge();
    }

    fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            auxiliary_vectors: vector_bytes(&self.census),
            ..Default::default()
        });
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
fn copy_focus_country(log: &mut RawLog<2>, memory: &impl engine::Memory, country: &FocusCountry) {
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
            copy_focus_pass(
                second_job,
                memory,
                ctx.base,
                ctx.game_state,
                &mut second_ticks,
            );
            ticks[0] += second_ticks[0];
            let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
            for (slot, matched) in matched.iter_mut().enumerate() {
                let equal = raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
                *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
            }
        }
        // Binary per refresh per table: tearing detected (0/1), not a row
        // count. Spool gates test `==0`/`!=0` only.
        self.focus.checked = needed[0];
        self.focus.mismatches = usize::from(needed[0] && !matched[0]);
        self.completed.checked = needed[1];
        self.completed.mismatches = usize::from(needed[1] && !matched[1]);
        let raw_ok = !first.log.overflowed && second.is_none_or(|job| !job.log.overflowed);
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
        self.convert_tables(
            |view| view,
            self.technologies_table(),
            self.definitions_table(),
        )
    }

    pub(crate) fn table_inputs(
        &self,
        reuse: Option<&mut ResearchConversion<'_>>,
    ) -> [TableInput; N] {
        let Some(reuse) = reuse else {
            return self.tables().map(TableInput::Sampled);
        };
        let technologies = self.technologies_input(reuse);
        let definitions = self.definitions_input(reuse);
        self.convert_tables(TableInput::Sampled, technologies, definitions)
    }

    fn technologies_table(&self) -> TableView {
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
        )
    }

    fn definitions_table(&self) -> TableView {
        let Some(definitions) = self.definition_text.first.get(..self.definition_text.len) else {
            return crate::spooling::unavailable("governance arena length exceeds capacity");
        };
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
        )
    }

    fn technologies_input(&self, reuse: &mut ResearchConversion<'_>) -> TableInput {
        let source = match self.technologies.outcome {
            Outcome::Rows { len } if self.technologies.mismatches == 0 => {
                self.technologies.rows.get(..len)
            }
            _ => None,
        };
        if let Some(source) = source
            && let Some(rows) = reuse.shadow.technologies.equal(
                TableId::ResearchTechnologies,
                source,
                reuse.cached[0],
            )
        {
            return TableInput::SharedComplete(rows);
        }
        reuse
            .shadow
            .technologies
            .record_miss(self.technologies.outcome);
        match (source, self.technologies_table()) {
            (Some(source), TableView::Complete(rows)) => {
                let rows = Arc::new(rows);
                reuse.shadow.technologies.install(source, &rows);
                TableInput::SharedComplete(rows)
            }
            (_, view) => {
                if !matches!(
                    self.technologies.outcome,
                    Outcome::Unavailable {
                        reason: replay::cadence::SKIPPED_REASON
                    }
                ) {
                    reuse.shadow.technologies.clear();
                }
                TableInput::Sampled(view)
            }
        }
    }

    fn definitions_input(&self, reuse: &mut ResearchConversion<'_>) -> TableInput {
        let source = match self.definitions.outcome {
            Outcome::Rows { len } if self.definitions.mismatches == 0 => {
                self.definitions.rows.get(..len)
            }
            _ => None,
        };
        let source = source
            .zip(self.definition_text.first.get(..self.definition_text.len))
            .filter(|(rows, text)| rows.iter().all(|row| valid(text, row.id_start, row.id_len)));
        if let Some((source, text)) = source
            && reuse.shadow.definition_required == self.definition_text.required
            && reuse.shadow.definition_text == text
            && let Some(rows) = reuse.shadow.definitions.equal(
                TableId::TechnologyDefinitions,
                source,
                reuse.cached[1],
            )
        {
            return TableInput::SharedComplete(rows);
        }
        reuse
            .shadow
            .definitions
            .record_miss(self.definitions.outcome);
        match (source, self.definitions_table()) {
            (Some((source, text)), TableView::Complete(rows)) => {
                let rows = Arc::new(rows);
                reuse.shadow.definitions.install(source, &rows);
                reuse.shadow.definition_text.clear();
                reuse.shadow.definition_text.extend_from_slice(text);
                reuse.shadow.definition_required = self.definition_text.required;
                TableInput::SharedComplete(rows)
            }
            (_, view) => {
                if !matches!(
                    self.definitions.outcome,
                    Outcome::Unavailable {
                        reason: replay::cadence::SKIPPED_REASON
                    }
                ) {
                    reuse.shadow.clear_definitions();
                }
                TableInput::Sampled(view)
            }
        }
    }

    fn convert_tables<T>(
        &self,
        wrap: impl Fn(TableView) -> T,
        technologies: T,
        definitions: T,
    ) -> [T; N] {
        let focus = &self.focus_text.first[..self.focus_text.len];
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
            wrap(view::<tables::Focus, _>(
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
            )),
            wrap(view::<tables::CompletedFocuses, _>(
                &self.completed,
                |r| valid(focus, r.token.start, r.token.len),
                |r| rows::CompletedFocusRow {
                    country: r.country,
                    ordinal: r.ordinal,
                    token: copy_span(focus, r.token.start, r.token.len),
                },
            )),
            technologies,
            wrap(view::<tables::ResearchSlots, _>(
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
            )),
            definitions,
            wrap(view::<tables::Politics, _>(
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
            )),
            wrap(country_view::<tables::PoliticalParties, _>(
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
            )),
            wrap(country_view::<tables::ActiveIdeas, _>(
                &self.ideas,
                &self.idea_countries.first[..self.idea_countries.len],
                |r| valid(ideas, r.token_start, r.token_len),
                |country, r| rows::ActiveIdeaRow {
                    country,
                    ordinal: r.ordinal,
                    token: copy_span(ideas, r.token_start, r.token_len),
                },
            )),
            wrap(view::<tables::DecisionCounts, _>(
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
            )),
            wrap(country_view::<tables::Decisions, _>(
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
            )),
            wrap(view::<tables::DynamicModifiers, _>(
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
            )),
            wrap(view::<tables::PreferredTactics, _>(
                &self.tactics,
                |r| valid(tactics, r.token_start, r.row.token_len),
                |r| rows::PreferredTacticRow {
                    country: r.row.country,
                    tactic_id: r.row.tactic_id,
                    token: copy_span(tactics, r.token_start, r.row.token_len),
                },
            )),
            wrap(view::<tables::PreferredTacticOperands, _>(
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
            )),
            wrap(view::<tables::TechSharingGroups, _>(
                &self.sharing,
                |_| true,
                |r| rows::TechSharingGroupRow {
                    id: r.id,
                    base_bonus: r.base_bonus,
                    member_count: r.member_count,
                },
            )),
            wrap(view::<tables::TechSharingMembers, _>(
                &self.sharing_members,
                |_| true,
                |r| rows::TechSharingMemberRow {
                    group: r.group,
                    ordinal: r.ordinal,
                    country: r.country,
                    bonus: r.bonus,
                },
            )),
            wrap(view::<tables::PowerBalances, _>(
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
            )),
            wrap(view::<tables::PowerBalanceMembers, _>(
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
            )),
            wrap(view::<tables::PowerBalanceRanges, _>(
                &self.ranges,
                |r| balance_valid(r.identity),
                |r| rows::PowerBalanceRangeRow {
                    record: r.record,
                    ordinal: r.ordinal,
                    identity: balance_copy(r.identity),
                },
            )),
            wrap(view::<tables::PowerBalanceOverrides, _>(
                &self.overrides,
                |r| balance_valid(r.side) && balance_valid(r.gfx),
                |r| rows::PowerBalanceOverrideRow {
                    record: r.record,
                    ordinal: r.ordinal,
                    side: balance_copy(r.side),
                    gfx: balance_copy(r.gfx),
                },
            )),
        ]
    }
}

#[cfg(test)]
pub(crate) fn research_shadow_fixture(progress: i64) -> Captured {
    let mut captured = Captured::with_capacities(
        &Capacities {
            rows: [0; N],
            ..Default::default()
        },
        &[CONSISTENCY_TARGET; N],
    );
    captured.technologies.rows = vec![
        ctechnology::Row {
            country: 2,
            technology_handle: 7,
            classifier: 5,
            level: -1,
            researched: 0xfe,
            progress_raw: progress,
            ahead_reduction_years_raw: i64::MIN,
        },
        ctechnology::Row {
            country: 1,
            technology_handle: 8,
            ..Default::default()
        },
    ];
    captured.technologies.outcome = Outcome::Rows { len: 2 };
    captured.definitions.rows = vec![
        ctechnology_template::Row {
            technology_handle: 8,
            database_index: 4,
            year: 1936,
            max_level: 3,
            research_cost_raw: i64::MAX,
            id_start: 0,
            id_len: 3,
        },
        ctechnology_template::Row {
            technology_handle: 7,
            database_index: 5,
            year: 1938,
            max_level: 2,
            research_cost_raw: 1000,
            id_start: 3,
            id_len: 3,
        },
    ];
    captured.definitions.outcome = Outcome::Rows { len: 2 };
    captured.definition_text.first[..6].copy_from_slice(&[0xff, 0, 1, b'a', 0, b'z']);
    captured.definition_text.len = 6;
    captured.definition_text.required = 6;
    captured
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shadow_inputs(
        captured: &Captured,
        shadow: &mut ResearchShadow,
        cached: Option<&[Arc<TableRows>; 2]>,
        branch: BranchId,
    ) -> [TableInput; N] {
        let previous = cached.map_or([None, None], |rows| [Some(&rows[0]), Some(&rows[1])]);
        captured.table_inputs(Some(&mut ResearchConversion::new(branch, shadow, previous)))
    }

    fn shared_research(inputs: &[TableInput; N]) -> [Arc<TableRows>; 2] {
        [2, 4].map(|index| match &inputs[index] {
            TableInput::SharedComplete(rows) => Arc::clone(rows),
            _ => panic!("fixture expected shared complete Research rows"),
        })
    }

    fn assert_views(captured: &Captured, inputs: &[TableInput; N]) {
        for (actual, expected) in inputs.iter().zip(captured.tables()) {
            match actual {
                TableInput::SharedComplete(rows) => {
                    assert_eq!(TableView::Complete((**rows).clone()), expected)
                }
                TableInput::Sampled(view) => assert_eq!(*view, expected),
                TableInput::ComparedEqual(_) => panic!("Research does not use comparison tokens"),
            }
        }
    }

    #[test]
    fn research_shadow_statistics_count_decisions_and_retained_capacity_across_invalidation() {
        let first = research_shadow_fixture(10);
        let mut shadow = ResearchShadow::default();
        assert_eq!(shadow.statistics(), [ResearchShadowStats::default(); 2]);
        let cached = shared_research(&shadow_inputs(&first, &mut shadow, None, 1));
        shadow.technologies.rows.reserve(32);
        shadow.definitions.rows.reserve(16);
        shadow.definition_text.reserve(31);
        let capacity = shadow.statistics();
        assert_eq!(
            capacity[0].row_capacity_bytes,
            shadow.technologies.rows.capacity() * size_of::<ctechnology::Row>()
        );
        assert_eq!(
            capacity[1].row_capacity_bytes,
            shadow.definitions.rows.capacity() * size_of::<ctechnology_template::Row>()
        );
        assert_eq!(
            capacity[1].arena_capacity_bytes,
            shadow.definition_text.capacity()
        );
        assert!(
            capacity[0].row_capacity_bytes
                > first.technologies.rows.len() * size_of::<ctechnology::Row>()
        );
        assert!(
            capacity
                .iter()
                .all(|table| table.hits == 0 && table.misses == 1)
        );

        let _hit = shadow_inputs(&first, &mut shadow, Some(&cached), 1);
        let after_hit = shadow.statistics();
        assert!(
            after_hit
                .iter()
                .all(|table| table.hits == 1 && table.misses == 1)
        );
        let mut skipped = research_shadow_fixture(10);
        skipped.technologies.outcome = Outcome::Unavailable {
            reason: replay::cadence::SKIPPED_REASON,
        };
        skipped.definitions.outcome = Outcome::Unavailable {
            reason: replay::cadence::SKIPPED_REASON,
        };
        let _skipped = shadow_inputs(&skipped, &mut shadow, Some(&cached), 1);
        assert_eq!(shadow.statistics(), after_hit);

        let mut failed = research_shadow_fixture(10);
        failed.technologies.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 1,
        };
        failed.definitions.outcome = Outcome::Unavailable {
            reason: "read failed",
        };
        let _failed = shadow_inputs(&failed, &mut shadow, Some(&cached), 1);
        assert!(shadow.technologies.rows.is_empty());
        assert!(shadow.definitions.rows.is_empty());
        assert!(shadow.definition_text.is_empty());
        assert!(
            shadow
                .statistics()
                .iter()
                .all(|table| table.hits == 1 && table.misses == 2)
        );

        let _branch = shadow_inputs(&first, &mut shadow, Some(&cached), 2);
        for (current, capacity) in shadow.statistics().into_iter().zip(capacity) {
            assert_eq!((current.hits, current.misses), (1, 3));
            assert_eq!(current.row_capacity_bytes, capacity.row_capacity_bytes);
            assert_eq!(current.arena_capacity_bytes, capacity.arena_capacity_bytes);
        }
    }

    #[test]
    fn research_shadow_hits_reuse_exact_arcs_and_do_not_copy_unused_capacity() {
        let first = research_shadow_fixture(10);
        let mut shadow = ResearchShadow::default();
        let inputs = shadow_inputs(&first, &mut shadow, None, 1);
        assert_views(&first, &inputs);
        let cached = shared_research(&inputs);
        assert_eq!(shadow.technologies.rows, first.technologies.rows);
        assert_eq!(shadow.definition_text, [0xff, 0, 1, b'a', 0, b'z']);
        let pointers = (
            shadow.technologies.rows.as_ptr(),
            shadow.definition_text.as_ptr(),
        );
        let mut current = research_shadow_fixture(10);
        current.technologies.rows.push(ctechnology::Row::default());
        current.definition_text.first[7] = 0x42;
        let reused = shadow_inputs(&current, &mut shadow, Some(&cached), 1);
        assert_views(&current, &reused);
        for (previous, current) in cached.iter().zip(shared_research(&reused)) {
            assert!(Arc::ptr_eq(previous, &current));
        }
        assert_eq!(
            pointers,
            (
                shadow.technologies.rows.as_ptr(),
                shadow.definition_text.as_ptr()
            )
        );
    }

    #[test]
    fn research_shadow_every_technology_field_order_and_length_change_misses_exactly() {
        for field in 0..10 {
            let first = research_shadow_fixture(10);
            let mut shadow = ResearchShadow::default();
            let cached = shared_research(&shadow_inputs(&first, &mut shadow, None, 1));
            let mut current = research_shadow_fixture(10);
            let row = &mut current.technologies.rows[0];
            match field {
                0 => row.country = 3,
                1 => row.technology_handle = 9,
                2 => row.classifier ^= 1,
                3 => row.level += 1,
                4 => row.researched ^= 1,
                5 => row.progress_raw += 1,
                6 => row.ahead_reduction_years_raw += 1,
                7 => current.technologies.rows.swap(0, 1),
                8 => current.technologies.outcome = Outcome::Rows { len: 1 },
                9 => current.technologies.outcome = Outcome::Rows { len: 0 },
                _ => unreachable!(),
            }
            let inputs = shadow_inputs(&current, &mut shadow, Some(&cached), 1);
            assert_views(&current, &inputs);
            let next = shared_research(&inputs);
            assert!(!Arc::ptr_eq(&cached[0], &next[0]), "field {field}");
            assert!(Arc::ptr_eq(&cached[1], &next[1]));
            let hit = shared_research(&shadow_inputs(&current, &mut shadow, Some(&next), 1));
            assert!(Arc::ptr_eq(&next[0], &hit[0]));
        }
    }

    #[test]
    fn research_shadow_definition_fields_exact_text_and_arena_metadata_gate_reuse() {
        for field in 0..13 {
            let first = research_shadow_fixture(10);
            let mut shadow = ResearchShadow::default();
            let cached = shared_research(&shadow_inputs(&first, &mut shadow, None, 1));
            let mut current = research_shadow_fixture(10);
            let row = &mut current.definitions.rows[0];
            match field {
                0 => row.technology_handle = 9,
                1 => row.database_index += 1,
                2 => row.year += 1,
                3 => row.max_level += 1,
                4 => row.research_cost_raw -= 1,
                5 => row.id_start += 1,
                6 => row.id_len -= 1,
                7 => current.definition_text.first[1] = 0xfe,
                8 => current.definition_text.len += 1,
                9 => current.definition_text.required += 1,
                10 => current.definitions.rows.swap(0, 1),
                11 => current.definitions.outcome = Outcome::Rows { len: 0 },
                12 => {
                    current.definition_text.len = 0;
                    current.definition_text.required = 0;
                    current.definitions.outcome = Outcome::Rows { len: 0 };
                }
                _ => unreachable!(),
            }
            let inputs = shadow_inputs(&current, &mut shadow, Some(&cached), 1);
            assert_views(&current, &inputs);
            let next = shared_research(&inputs);
            assert!(Arc::ptr_eq(&cached[0], &next[0]));
            assert!(!Arc::ptr_eq(&cached[1], &next[1]), "field {field}");
        }
    }

    #[test]
    fn research_shadow_failures_never_reuse_complete_rows_and_recover_through_conversion() {
        for fault in 0..14 {
            let first = research_shadow_fixture(10);
            let mut shadow = ResearchShadow::default();
            let cached = shared_research(&shadow_inputs(&first, &mut shadow, None, 1));
            let mut current = research_shadow_fixture(10);
            match fault {
                0 => current.technologies.outcome = Outcome::Rows { len: 3 },
                1 => {
                    current.technologies.outcome = Outcome::Truncated {
                        len: 1,
                        unread_rows: 5,
                    }
                }
                2 => {
                    current.technologies.outcome = Outcome::Truncated {
                        len: 1,
                        unread_rows: 0,
                    }
                }
                3 => {
                    current.technologies.outcome = Outcome::Unavailable {
                        reason: "native read failed",
                    }
                }
                4 => current.technologies.rows[1] = current.technologies.rows[0],
                5 => {
                    current.technologies.checked = true;
                    current.technologies.mismatches = 1;
                }
                6 => current.definitions.rows[0].id_start = usize::MAX,
                7 => current.definition_text.len = current.definition_text.first.len() + 1,
                8 => current.definitions.outcome = Outcome::Rows { len: 3 },
                9 => {
                    current.definitions.outcome = Outcome::Truncated {
                        len: 1,
                        unread_rows: 5,
                    }
                }
                10 => {
                    current.definitions.outcome = Outcome::Truncated {
                        len: 1,
                        unread_rows: 0,
                    }
                }
                11 => {
                    current.definitions.outcome = Outcome::Unavailable {
                        reason: "native read failed",
                    }
                }
                12 => current.definitions.rows[1] = current.definitions.rows[0],
                13 => {
                    current.definitions.checked = true;
                    current.definitions.mismatches = 1;
                }
                _ => unreachable!(),
            }
            let inputs = shadow_inputs(&current, &mut shadow, Some(&cached), 1);
            assert_views(&current, &inputs);
            let failed = if fault < 6 { 0 } else { 1 };
            assert!(matches!(inputs[[2, 4][failed]], TableInput::Sampled(_)));
            let restored = shared_research(&shadow_inputs(&first, &mut shadow, Some(&cached), 1));
            assert!(!Arc::ptr_eq(&cached[failed], &restored[failed]));
        }
    }

    #[test]
    fn research_shadow_rejects_wrong_cache_identity_and_new_branches_but_keeps_off_hours() {
        let first = research_shadow_fixture(10);
        let mut shadow = ResearchShadow::default();
        let cached = shared_research(&shadow_inputs(&first, &mut shadow, None, 1));
        let mut skipped = research_shadow_fixture(99);
        skipped.technologies.outcome = Outcome::Unavailable {
            reason: replay::cadence::SKIPPED_REASON,
        };
        skipped.definitions.outcome = Outcome::Unavailable {
            reason: replay::cadence::SKIPPED_REASON,
        };
        assert_views(
            &skipped,
            &shadow_inputs(&skipped, &mut shadow, Some(&cached), 1),
        );
        let hit = shared_research(&shadow_inputs(&first, &mut shadow, Some(&cached), 1));
        assert!(Arc::ptr_eq(&cached[0], &hit[0]));
        let replacement = cached.each_ref().map(|rows| Arc::new((**rows).clone()));
        let miss = shared_research(&shadow_inputs(&first, &mut shadow, Some(&replacement), 1));
        assert!(!Arc::ptr_eq(&replacement[0], &miss[0]));
        let branch = shared_research(&shadow_inputs(&first, &mut shadow, Some(&miss), 2));
        assert!(!Arc::ptr_eq(&miss[0], &branch[0]));
        let missing = shared_research(&shadow_inputs(&first, &mut shadow, None, 2));
        assert!(!Arc::ptr_eq(&branch[0], &missing[0]));
    }

    #[test]
    fn focus_job_capacity_accounting_tracks_all_vectors_through_growth_clear_and_drop() {
        use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
        static LOCAL: Tracker = Tracker::new();
        let mut job = FocusJob::default();
        job.census.reserve_exact(3);
        job.capacity_charge = Charge::empty_on(&LOCAL);
        job.update_capacity_charge();
        let expected = |job: &FocusJob| vector_bytes(&job.census);
        let initial = LOCAL.snapshot().current.auxiliary_vectors;
        assert_eq!(initial, expected(&job));
        job.grow();
        let grown = LOCAL.snapshot().current.auxiliary_vectors;
        assert!(grown > initial);
        assert_eq!(grown, expected(&job));
        job.clear();
        assert_eq!(LOCAL.snapshot().current.auxiliary_vectors, grown);
        drop(job);
        assert_eq!(LOCAL.snapshot().current, Capacity::default());
    }

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
    /// Shared synthetic address space for the hourly focus copy/decode split
    /// harness: two authenticated countries with inline, absent and heap focus
    /// tokens plus one- and two-entry completed rosters.
    pub(crate) mod focus_test_kit {
        use super::{Capacities, Captured, FocusCopyContext, FocusJob};
        use engine::memory::SliceMemory;
        use engine::{
            ccontinuous_national_focus, ccountry, cgame_state, cnational_focus,
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
                self.w32(
                    root + cnational_focus_progress::COMPLETED_COUNT,
                    count as u32,
                );
                self.w64(
                    root + cnational_focus_progress::COMPLETED_DATA,
                    roster as u64,
                );
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
        let mut typed_small = Captured::with_capacities(&small, &[0u16; TABLE_COUNT]);
        typed_small.capture(&memory, focus_test_kit::BASE, 0x1_0000, &needed, 0, true);
        let mut replayed_small = Captured::with_capacities(&small, &[0u16; TABLE_COUNT]);
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
            assert!(
                report.raw_ok,
                "focus={focus_rows} completed={completed_rows}"
            );
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
        assert!(matches!(typed.focus.outcome, Outcome::Unavailable { .. }));
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
        assert!(matches!(typed.focus.outcome, Outcome::Unavailable { .. }));
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
        replayed.capture(
            &invalid_memory,
            focus_test_kit::BASE,
            0x1_0000,
            &needed,
            0,
            true,
        );
        let mut ticks = [0; 2];
        replayed.decode_focus_payload(focus_test_kit::BASE, &first, &mut ticks);
        let mut typed = Fixture::captured();
        typed.capture(
            &invalid_memory,
            focus_test_kit::BASE,
            0x1_0000,
            &needed,
            0,
            true,
        );
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
            capacity_charge: crate::raw_memory::Charge::default(),
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
