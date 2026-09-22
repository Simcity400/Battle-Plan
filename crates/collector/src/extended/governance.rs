//! Governance rows and arenas are allocated off the game thread.
//! Shared-walk costs are charged once to each READ_GROUPS leader, in QPC ticks.

use crate::{
    read_accounting,
    sampling::qpc,
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
};
use engine::{
    Memory, ccombat_tactic, ccountry, cdecision_status, cdynamic_modifier_container,
    cnational_focus_progress, cpolitical_party, cpolitical_status, cresearch_slot, ctechnology,
    ctechnology_sharing_group, ctechnology_status, ctechnology_template,
    power_balance_gfx_override, power_balance_manager, power_balance_range, power_balance_record,
    walk::Outcome,
};
use replay::{
    BranchId,
    frame::{TableInput, TableView},
    rows::{self, TableId, TableRows, TableSchema, tables},
};
use std::{
    mem::size_of,
    sync::{Arc, Weak},
};

pub const N: usize = 22;

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
/// outcome is retained or substituted for a new read.
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
    TableId::ResearchInputs,
    TableId::ResearchAppliedBonuses,
    TableId::ResearchModifiers,
];
/// Name, local cost anchor and local members. Reports sum member ticks;
/// physically shared walks charge their anchor once and leave followers zero.
/// Country authentication is included in hourly Politics and shared by tables 0..13.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("Focus", 0, &[0, 1]),
    ("Research technologies and definitions", 2, &[2, 4]),
    ("Research slots", 3, &[3]),
    (
        "Politics, parties, ideas and shared country authentication",
        5,
        &[5, 6, 7],
    ),
    ("Decision counts and state", 8, &[8, 9]),
    ("Dynamic modifiers", 10, &[10]),
    ("Preferred tactic and operands", 11, &[11, 12]),
    ("Technology sharing", 13, &[13, 14]),
    ("Power balance and children", 15, &[15, 16, 17, 18]),
    ("Hourly research inputs, bonuses and modifiers", 19, &[19, 20, 21]),
];

/// Callback-local inputs for an exclusively owned governance acquisition.
/// Native readers are constructed on the executing thread and never retained.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CaptureContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
}

/// Per-table costs from the current acquisition.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CaptureReport {
    pub ticks: [u64; N],
}

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
                1024, 128, 512, 1024, 512, 2048, 4096, 16384,
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
    /// Acquisition-only native pointers, parallel to the current technology rows.
    technology_pointers: Vec<usize>,
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
    research_inputs: CaptureRows<engine::research_inputs::Row>,
    research_bonuses: CaptureRows<engine::research_inputs::BonusRow>,
    research_modifiers: CaptureRows<engine::research_modifiers::Row>,
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
    dictionary_required: usize,
    /// Owned acquisition gate for the scheduled focus pair (Focus,
    /// CompletedFocuses). Set for an accepted World payload; `capture` skips
    /// duplicate focus walks and the decode worker installs the owned rows
    /// before the ready handoff. Preserved across
    /// `reset` (set per hour, cleared by the worker after decode).
    pub(crate) focus_raw_mode: bool,
}

impl Captured {
    /// Allocation-free replacement while this frame's buffers belong to an
    /// acquisition worker. The caller restores the original before any view,
    /// growth, delegated Focus installation, or publication.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: [0; N],
            focus_text: 0,
            definition_text: 0,
            party_text: 0,
            idea_text: 0,
            decision_text: 0,
            modifier_text: 0,
            modifier_values: 0,
            tactic_text: 0,
            balance_text: 0,
            party_countries: 0,
            idea_countries: 0,
            decision_countries: 0,
            dictionary: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            focus: CaptureRows::new(capacity.rows[0]),
            completed: CaptureRows::new(capacity.rows[1]),
            technologies: CaptureRows::new(capacity.rows[2]),
            technology_pointers: vec![0; capacity.rows[2]],
            slots: CaptureRows::new(capacity.rows[3]),
            definitions: CaptureRows::new(capacity.rows[4]),
            politics: CaptureRows::new(capacity.rows[5]),
            parties: CaptureRows::new(capacity.rows[6]),
            ideas: CaptureRows::new(capacity.rows[7]),
            counts: CaptureRows::new(capacity.rows[8]),
            decisions: CaptureRows::new(capacity.rows[9]),
            modifiers: CaptureRows::new(capacity.rows[10]),
            tactics: CaptureRows::new(capacity.rows[11]),
            operands: CaptureRows::new(capacity.rows[12]),
            sharing: CaptureRows::new(capacity.rows[13]),
            sharing_members: CaptureRows::new(capacity.rows[14]),
            balances: CaptureRows::new(capacity.rows[15]),
            balance_members: CaptureRows::new(capacity.rows[16]),
            ranges: CaptureRows::new(capacity.rows[17]),
            research_inputs: CaptureRows::new(capacity.rows[19]),
            research_bonuses: CaptureRows::new(capacity.rows[20]),
            research_modifiers: CaptureRows::new(capacity.rows[21]),
            overrides: CaptureRows::new(capacity.rows[18]),
            focus_text: Arena::new(capacity.focus_text),
            definition_text: Arena::new(capacity.definition_text),
            party_text: Arena::new(capacity.party_text),
            idea_text: Arena::new(capacity.idea_text),
            decision_text: Arena::new(capacity.decision_text),
            modifier_text: Arena::new(capacity.modifier_text),
            modifier_values: Arena::new(capacity.modifier_values),
            tactic_text: Arena::new(capacity.tactic_text),
            balance_text: Arena::new(capacity.balance_text),
            party_countries: Arena::new(capacity.party_countries),
            idea_countries: Arena::new(capacity.idea_countries),
            decision_countries: Arena::new(capacity.decision_countries),
            dictionary: vec![ctechnology_template::Entry::default(); capacity.dictionary],
            dictionary_required: 0,
            focus_raw_mode: false,
        }
    }

    /// Run the unchanged complete family capture on its owning thread. Focus
    /// delegation must already describe this callback before dispatch.
    pub(crate) fn capture_owned(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
    ) -> CaptureReport {
        let ticks = self.capture(
            memory,
            context.base,
            context.game_state,
            context.hour,
            context.force_full,
        );
        CaptureReport { ticks }
    }

    /// Terminal completion after an accepted worker failure. Partial rows and
    /// arena lengths cannot become publishable, genuine cadence skips remain
    /// skips, and an independently delegated Focus payload retains ownership.
    /// No native reads or synchronous retry are performed here.
    pub(crate) fn fail_owned_capture(
        &mut self,
        context: CaptureContext,
        reason: &'static str,
    ) -> CaptureReport {
        self.reset();
        macro_rules! reject {
            ($($field:ident = $index:literal),+ $(,)?) => {
                $({
                    let active = replay::cadence::should_walk(
                        TABLES[$index], context.hour, context.force_full,
                    );
                    self.$field.outcome = Outcome::Unavailable {
                        reason: if active { reason } else { replay::cadence::SKIPPED_REASON },
                    };
                })+
            };
        }
        if !self.focus_raw_mode {
            reject!(focus = 0, completed = 1);
        }
        reject!(
            technologies = 2,
            slots = 3,
            definitions = 4,
            politics = 5,
            parties = 6,
            ideas = 7,
            counts = 8,
            decisions = 9,
            modifiers = 10,
            tactics = 11,
            operands = 12,
            sharing = 13,
            sharing_members = 14,
            balances = 15,
            balance_members = 16,
            ranges = 17,
            overrides = 18,
            research_bonuses = 20,
            research_modifiers = 21,
            research_inputs = 19,
        );
        CaptureReport::default()
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
        self.research_bonuses.reset();
        self.research_modifiers.reset();
        self.research_inputs.reset();
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
            + self.technology_pointers.capacity() * size_of::<usize>()
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
            + self.research_bonuses.allocated_bytes()
            + self.research_modifiers.allocated_bytes()
            + self.research_inputs.allocated_bytes()
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
            + self.dictionary.capacity() * size_of::<ctechnology_template::Entry>()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        capacity.rows[2] = capacity.rows[2].max(self.technologies.rows.len());
        capacity.rows[4] = capacity.rows[4].max(self.definitions.rows.len());
        self.focus.grow(&mut capacity.rows[0]);
        self.completed.grow(&mut capacity.rows[1]);
        grow_large(&mut self.technologies, &mut capacity.rows[2]);
        grow_vec(&mut self.technology_pointers, capacity.rows[2]);
        self.slots.grow(&mut capacity.rows[3]);
        self.definitions.grow(&mut capacity.rows[4]);
        self.politics.grow(&mut capacity.rows[5]);
        self.parties.grow(&mut capacity.rows[6]);
        self.ideas.grow(&mut capacity.rows[7]);
        self.counts.grow(&mut capacity.rows[8]);
        self.decisions.grow(&mut capacity.rows[9]);
        self.modifiers.grow(&mut capacity.rows[10]);
        self.tactics.grow(&mut capacity.rows[11]);
        self.operands.grow(&mut capacity.rows[12]);
        self.sharing.grow(&mut capacity.rows[13]);
        self.sharing_members.grow(&mut capacity.rows[14]);
        self.balances.grow(&mut capacity.rows[15]);
        self.balance_members.grow(&mut capacity.rows[16]);
        self.ranges.grow(&mut capacity.rows[17]);
        self.research_inputs.grow(&mut capacity.rows[19]);
        self.research_bonuses.grow(&mut capacity.rows[20]);
        self.research_modifiers.grow(&mut capacity.rows[21]);
        self.overrides.grow(&mut capacity.rows[18]);
        capacity.party_countries = capacity.rows[6];
        capacity.idea_countries = capacity.rows[7];
        capacity.decision_countries = capacity.rows[9];
        self.focus_text.grow(&mut capacity.focus_text);
        self.definition_text.grow(&mut capacity.definition_text);
        self.party_text.grow(&mut capacity.party_text);
        self.idea_text.grow(&mut capacity.idea_text);
        self.decision_text.grow(&mut capacity.decision_text);
        self.modifier_text.grow(&mut capacity.modifier_text);
        self.modifier_values.grow(&mut capacity.modifier_values);
        self.tactic_text.grow(&mut capacity.tactic_text);
        self.balance_text.grow(&mut capacity.balance_text);
        self.party_countries.grow(&mut capacity.party_countries);
        self.idea_countries.grow(&mut capacity.idea_countries);
        self.decision_countries
            .grow(&mut capacity.decision_countries);
        grow_capacity(&mut capacity.dictionary, self.dictionary_required);
        grow_vec(&mut self.dictionary, capacity.dictionary);
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
            self.research_inputs.outcome,
            self.research_bonuses.outcome,
            self.research_modifiers.outcome,
        ]
    }

    fn buffers(&mut self) -> Buffers<'_> {
        Buffers {
            focus: &mut self.focus.rows,
            completed: &mut self.completed.rows,
            technologies: &mut self.technologies.rows,
            technology_pointers: &mut self.technology_pointers,
            slots: &mut self.slots.rows,
            definitions: &mut self.definitions.rows,
            politics: &mut self.politics.rows,
            parties: &mut self.parties.rows,
            ideas: &mut self.ideas.rows,
            counts: &mut self.counts.rows,
            decisions: &mut self.decisions.rows,
            modifiers: &mut self.modifiers.rows,
            tactics: &mut self.tactics.rows,
            operands: &mut self.operands.rows,
            sharing: &mut self.sharing.rows,
            sharing_members: &mut self.sharing_members.rows,
            balances: &mut self.balances.rows,
            balance_members: &mut self.balance_members.rows,
            ranges: &mut self.ranges.rows,
            research_inputs: &mut self.research_inputs.rows,
            research_bonuses: &mut self.research_bonuses.rows,
            research_modifiers: &mut self.research_modifiers.rows,
            overrides: &mut self.overrides.rows,
            focus_text: &mut self.focus_text.first,
            definition_text: &mut self.definition_text.first,
            party_text: &mut self.party_text.first,
            idea_text: &mut self.idea_text.first,
            decision_text: &mut self.decision_text.first,
            modifier_text: &mut self.modifier_text.first,
            modifier_values: &mut self.modifier_values.first,
            tactic_text: &mut self.tactic_text.first,
            balance_text: &mut self.balance_text.first,
            party_countries: &mut self.party_countries.first,
            idea_countries: &mut self.idea_countries.first,
            decision_countries: &mut self.decision_countries.first,
            dictionary: &mut self.dictionary,
        }
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.reset();
        let mut walk: [bool; N] =
            std::array::from_fn(|i| replay::cadence::should_walk(TABLES[i], hour, force_full));
        // The World acquisition job stages scheduled Focus via `copy_focus_refresh`;
        // the decode worker fills the typed rows before the ready handoff, so this capture
        // skips the delegated focus tables. Costs come from the owned refresh.
        if self.focus_raw_mode {
            walk[0] = false;
            walk[1] = false;
        }
        let first = acquire(memory, base, game_state, self.buffers(), &walk);
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
        self.research_bonuses.outcome = first.outcomes[20];
        self.research_modifiers.outcome = first.outcomes[21];
        self.research_inputs.outcome = first.outcomes[19];
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
        first.ticks
    }
}

fn elapsed(start: i64) -> u64 {
    u64::try_from(qpc().saturating_sub(start)).unwrap_or(0)
}

fn grow_large<R: Default + Clone>(rows: &mut CaptureRows<R>, capacity: &mut usize) {
    let required = match rows.outcome {
        Outcome::Rows { len } => len,
        Outcome::Truncated { len, unread_rows } => len.saturating_add(unread_rows as usize),
        Outcome::Unavailable { .. } => 0,
    };
    if required > capacity.saturating_sub(*capacity / 4) {
        *capacity = (*capacity).max(required.saturating_add(required.div_ceil(3)));
    }
    grow_vec(&mut rows.rows, *capacity);
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
    technologies: &'a mut Vec<ctechnology::Row>,
    technology_pointers: &'a mut Vec<usize>,
    slots: &'a mut [cresearch_slot::Row],
    definitions: &'a mut Vec<ctechnology_template::Row>,
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
    research_inputs: &'a mut [engine::research_inputs::Row],
    research_bonuses: &'a mut [engine::research_inputs::BonusRow],
    research_modifiers: &'a mut [engine::research_modifiers::Row],
    overrides: &'a mut [power_balance_gfx_override::Row],
    focus_text: &'a mut [u8],
    definition_text: &'a mut Vec<u8>,
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
    dictionary: &'a mut Vec<ctechnology_template::Entry>,
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
    for (outcome, &active) in o.outcomes.iter_mut().zip(enabled) {
        if !active {
            *outcome = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
        }
    }
    let started = qpc();
    let mut unused_entries = Vec::new();
    let mut unused_names = Vec::new();
    let mut definitions = ctechnology_template::Dictionary::new(
        if enabled[2] {
            b.dictionary
        } else {
            &mut unused_entries
        },
        if enabled[2] {
            b.definition_text
        } else {
            &mut unused_names
        },
    );
    if enabled[2] {
        o.ticks[2] += elapsed(started);
    }
    let started = qpc();
    let tactic_context = if enabled[11] {
        let _reads = read_accounting::table(TABLES[11]);
        ccombat_tactic::WeightContext::read(memory, base)
    } else {
        Err("tactic weight not sampled")
    };
    if enabled[11] {
        o.ticks[11] += elapsed(started);
    }
    let started = qpc();
    let research_context = if enabled[19] {
        engine::research_inputs::Context::read(memory, base)
    } else {
        Err("research inputs not sampled")
    };
    if enabled[19] {
        o.ticks[19] += elapsed(started);
    }
    let started = qpc();
    let modifier_context = if enabled[21] { engine::research_modifiers::Context::read(memory, base) } else { Err("research modifiers not sampled") };
    if enabled[21] { o.ticks[19] += elapsed(started); }
    if enabled[..13].iter().chain(&enabled[19..]).any(|&value| value) {
        let started = qpc();
        let countries = read_accounting::with_table(TABLES[5], || {
            ccountry::Reader::new(memory, base, game_state)
        });
        o.ticks[5] += elapsed(started);
        match countries {
            Err(reason) => {
                for index in (0..13).chain(19..N) {
                    if enabled[index] {
                        o.outcomes[index] = Outcome::Unavailable { reason };
                    }
                }
            }
            Ok(countries) => {
                for slot in 1..countries.count() {
                    let started = qpc();
                    let country = read_accounting::with_table(TABLES[5], || countries.at(slot));
                    o.ticks[5] += elapsed(started);
                    let country = match country {
                        Ok(Some(country)) => country,
                        Ok(None) => continue,
                        Err(reason) => {
                            for index in (0..13).chain(19..N) {
                                if enabled[index] {
                                    o.outcomes[index] = Outcome::Unavailable { reason };
                                }
                            }
                            break;
                        }
                    };
                    if enabled[0] {
                        let _reads = read_accounting::table(TABLES[0]);
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
                        let _reads = read_accounting::table(TABLES[2]);
                        let started = qpc();
                        let first = o.outcomes[2].len();
                        let result = ctechnology_status::walk_technologies(
                            memory,
                            base,
                            country.pointer,
                            country.id,
                            (&mut *b.technologies, &mut *b.technology_pointers),
                            first,
                            &mut definitions,
                        );
                        append(&mut o.outcomes[2], result);
                        o.ticks[2] += elapsed(started);
                    }
                    if enabled[3] {
                        let _reads = read_accounting::table(TABLES[3]);
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
                    if enabled[19] {
                        let started = qpc();
                        let input_start = o.outcomes[19].len();
                        let (inputs, bonuses) = engine::research_inputs::walk_country(memory, base,
                            country.pointer, country.id, research_context,
                            &mut b.research_inputs[o.outcomes[19].len()..],
                            &mut b.research_bonuses[o.outcomes[20].len()..]);
                        if enabled[21] {
                            let modifiers = match inputs {
                                Outcome::Rows { len } => engine::research_modifiers::walk_country(memory, base, &countries, country, modifier_context, &b.research_inputs[input_start..input_start + len], &mut b.research_modifiers[o.outcomes[21].len()..]),
                                _ => Outcome::Unavailable { reason: "research modifier assignments incomplete" },
                            };
                            append(&mut o.outcomes[21], modifiers);
                        }
                        append(&mut o.outcomes[19], inputs);
                        append(&mut o.outcomes[20], bonuses);
                        o.ticks[19] += elapsed(started);
                    }
                    if enabled[5] {
                        let _reads = read_accounting::table(TABLES[5]);
                        let started = qpc();
                        match cpolitical_status::Reader::new(memory, base, country.pointer) {
                            Err(reason) => {
                                for (index, &active) in enabled.iter().enumerate().take(8).skip(5) {
                                    if active {
                                        o.outcomes[index] = Outcome::Unavailable { reason };
                                    }
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
                                if enabled[7] {
                                    let first = o.outcomes[7].len();
                                    let text = o.lens[3];
                                    let capacity = b.ideas.len().min(b.idea_countries.len());
                                    let result = reader.ideas(
                                        &mut b.ideas[first..capacity],
                                        &mut b.idea_text[text..],
                                    );
                                    for row in &mut b.ideas[first..first + result.outcome.len()] {
                                        row.token_start += text;
                                    }
                                    b.idea_countries[first..first + result.outcome.len()]
                                        .fill(country.id);
                                    append(&mut o.outcomes[7], result.outcome);
                                    o.arena(3, result.bytes_len, result.required_bytes);
                                }
                            }
                        }
                        o.ticks[5] += elapsed(started);
                    }
                    if enabled[8] {
                        let _reads = read_accounting::table(TABLES[8]);
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
                        let _reads = read_accounting::table(TABLES[10]);
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
                        let _reads = read_accounting::table(TABLES[11]);
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
                        // The context was acquired once before entering this loop.
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
        let count = definitions
            .entries
            .iter()
            .filter(|entry| entry.copied)
            .count();
        b.definitions.clear();
        let copied = if b.definitions.try_reserve(count).is_err() {
            Outcome::Unavailable {
                reason: "technology definition row allocation failed",
            }
        } else {
            b.definitions.extend(
                definitions
                    .entries
                    .iter()
                    .filter(|entry| entry.copied)
                    .map(|entry| entry.row),
            );
            // The old direct-index dictionary emitted increasing raw handles.
            b.definitions
                .sort_unstable_by_key(|row| row.technology_handle);
            Outcome::Rows { len: count }
        };
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
        let _reads = read_accounting::table(TABLES[13]);
        let started = qpc();
        let result =
            ctechnology_sharing_group::walk(memory, base, game_state, b.sharing, b.sharing_members);
        o.outcomes[13] = result.groups;
        o.outcomes[14] = result.members;
        o.ticks[13] += elapsed(started);
    }
    if enabled[15] {
        let _reads = read_accounting::table(TABLES[15]);
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
// Scheduled Focus and CompletedFocuses owned native acquisition.
//
// The callback-bounded World job runs the existing native walker once per pass.
// Rows and referenced token bytes are owned here; the decode worker only installs
// admitted prefixes. No raw index, mirrored census or replayed walker remains.
// Exact frame grants are checked before any source read, so storage refusal
// can use the typed fallback without retrying a consumed read fault.
// ===========================================================================

#[derive(Debug, Default)]
pub(crate) struct FocusJob {
    pub active: bool,
    rows: Vec<cnational_focus_progress::Row>,
    completed: Vec<cnational_focus_progress::CompletedRow>,
    bytes: Vec<u8>,
    outcomes: [Option<Outcome>; 2],
    bytes_len: usize,
    required_bytes: usize,
    row_grants: [usize; 2],
    byte_grant: usize,
    provisioned: bool,
    capacity_charge: crate::raw_memory::Charge,
}

impl FocusJob {
    pub fn new() -> Self {
        let capacities = Capacities::default();
        let mut job = Self {
            rows: vec![Default::default(); capacities.rows[0]],
            completed: vec![Default::default(); capacities.rows[1]],
            bytes: vec![0; capacities.focus_text],
            row_grants: [capacities.rows[0], capacities.rows[1]],
            byte_grant: capacities.focus_text,
            provisioned: true,
            ..Default::default()
        };
        job.update_capacity_charge();
        job
    }

    pub fn is_provisioned(&self) -> bool {
        self.provisioned
    }

    #[cfg(test)]
    pub(crate) fn capacities(&self) -> ([usize; 2], usize) {
        ([self.rows.len(), self.completed.len()], self.bytes.len())
    }

    pub fn clear(&mut self) {
        self.active = false;
        self.outcomes = [None; 2];
        self.bytes_len = 0;
        self.required_bytes = 0;
        self.row_grants = [0; 2];
        self.byte_grant = 0;
    }

    fn prepare(&mut self, rows: [usize; 2], bytes: usize) -> bool {
        self.clear();
        self.active = true;
        self.row_grants = rows;
        self.byte_grant = bytes;
        self.provisioned && !self.grow_for_next()
    }

    pub fn grow_for_next(&self) -> bool {
        self.rows.len() < self.row_grants[0]
            || self.completed.len() < self.row_grants[1]
            || self.bytes.len() < self.byte_grant
    }

    /// Called only after callback ownership ends, on the decode side.
    pub fn grow(&mut self) {
        grow_vec(&mut self.rows, self.row_grants[0]);
        grow_vec(&mut self.completed, self.row_grants[1]);
        grow_vec(&mut self.bytes, self.byte_grant);
        self.update_capacity_charge();
    }

    fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            payload_vectors: vector_bytes(&self.rows)
                + vector_bytes(&self.completed)
                + vector_bytes(&self.bytes),
            ..Default::default()
        });
    }

    fn outcome(&self, slot: usize) -> Outcome {
        self.outcomes[slot].unwrap_or(Outcome::Unavailable {
            reason: "focus acquisition not supplied",
        })
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct FocusCopyReport {
    /// False only for a preflight refusal, before any live source read.
    pub raw_ok: bool,
    /// Shared native traversal is charged once to Focus.
    pub copy_ticks: [u64; 2],
    /// Indexed raw payload bytes only. Owned rows/tokens are charged by capacity.
    pub raw_bytes: [u64; 2],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct FocusCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub active: bool,
    pub rows: [usize; 2],
    pub bytes: usize,
}

fn copy_focus_pass(
    job: &mut FocusJob,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    ticks: &mut [u64; 2],
) {
    let _reads = read_accounting::table(TABLES[0]);
    let started = qpc();
    let mut focus_total = Outcome::Rows { len: 0 };
    let mut completed_total = Outcome::Rows { len: 0 };
    let mut text_len = 0usize;
    let mut text_required = 0usize;
    match ccountry::Reader::new(memory, base, game_state) {
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
                    memory,
                    base,
                    country.pointer,
                    country.id,
                    &mut job.rows[first..job.row_grants[0]],
                    &mut job.completed[completed..job.row_grants[1]],
                    &mut job.bytes[text..job.byte_grant],
                );
                for row in &mut job.rows[first..first + result.outcome.len()] {
                    for span in [&mut row.tree, &mut row.current, &mut row.continuous]
                        .into_iter()
                        .flatten()
                    {
                        span.start += text;
                    }
                }
                for row in &mut job.completed[completed..completed + result.completed_outcome.len()]
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
    job.outcomes = [Some(focus_total), Some(completed_total)];
    job.bytes_len = text_len;
    job.required_bytes = text_required;
    ticks[0] += elapsed(started);
}

pub(crate) fn copy_focus_refresh(
    memory: &impl Memory,
    ctx: FocusCopyContext,
    job: &mut FocusJob,
) -> FocusCopyReport {
    if !ctx.active {
        job.clear();
        return FocusCopyReport {
            raw_ok: true,
            ..Default::default()
        };
    }
    if !job.prepare(ctx.rows, ctx.bytes) {
        return FocusCopyReport::default();
    }
    let mut report = FocusCopyReport {
        raw_ok: true,
        ..Default::default()
    };
    copy_focus_pass(
        job,
        memory,
        ctx.base,
        ctx.game_state,
        &mut report.copy_ticks,
    );
    report
}

impl Captured {
    pub(crate) fn focus_copy_context(
        &self,
        base: usize,
        game_state: usize,
        active: bool,
    ) -> FocusCopyContext {
        FocusCopyContext {
            base,
            game_state,
            active,
            rows: [self.focus.rows.len(), self.completed.rows.len()],
            bytes: self.focus_text.first.len(),
        }
    }

    /// Finish an accepted acquisition failure without retrying live memory.
    pub(crate) fn fail_focus_capture(&mut self, active: bool, reason: &'static str) {
        let reason = if active {
            reason
        } else {
            replay::cadence::SKIPPED_REASON
        };
        self.focus.outcome = Outcome::Unavailable { reason };
        self.completed.outcome = Outcome::Unavailable { reason };
        self.focus_text.reset();
        self.focus_raw_mode = false;
    }

    /// Installs only already-owned admitted prefixes. A changed frame grant is
    /// an ownership error, never a reason to touch the live source again.
    pub(crate) fn decode_focus_payload(
        &mut self,
        _base: usize,
        job: &FocusJob,
        decode_ticks: &mut [u64; 2],
    ) {
        if !job.active {
            self.fail_focus_capture(false, replay::cadence::SKIPPED_REASON);
            return;
        }
        let started = qpc();
        let arena_ok = self.focus_text.first.len() == job.byte_grant
            && job.bytes_len <= job.byte_grant
            && job.bytes_len <= job.bytes.len();
        self.focus.outcome = job.outcome(0);
        if !arena_ok
            || self.focus.rows.len() != job.row_grants[0]
            || self.focus.outcome.len() > job.row_grants[0]
        {
            self.focus.outcome = Outcome::Unavailable {
                reason: "focus owned payload capacity mismatch",
            };
        } else {
            let len = self.focus.outcome.len();
            self.focus.rows[..len].copy_from_slice(&job.rows[..len]);
        }
        if arena_ok {
            self.focus_text.first[..job.bytes_len].copy_from_slice(&job.bytes[..job.bytes_len]);
            self.focus_text.len = job.bytes_len;
            self.focus_text.required = job.required_bytes;
        } else {
            self.focus_text.len = 0;
            self.focus_text.required = job.required_bytes.max(job.byte_grant);
        }
        decode_ticks[0] += elapsed(started);
        let started = qpc();
        self.completed.outcome = job.outcome(1);
        if !arena_ok
            || self.completed.rows.len() != job.row_grants[1]
            || self.completed.outcome.len() > job.row_grants[1]
        {
            self.completed.outcome = Outcome::Unavailable {
                reason: "completed focus owned payload capacity mismatch",
            };
        } else {
            let len = self.completed.outcome.len();
            self.completed.rows[..len].copy_from_slice(&job.completed[..len]);
        }
        decode_ticks[1] += elapsed(started);
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
            Outcome::Rows { len } => self.technologies.rows.get(..len),
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
            Outcome::Rows { len } => self.definitions.rows.get(..len),
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
            wrap(view::<tables::ResearchInputs, _>(&self.research_inputs, |_| true, |r| rows::ResearchInputRow {
                country: r.slot.country,
                index: r.slot.index,
                technology_handle: r.slot.technology_handle,
                technology_present: r.slot.technology_present,
                token: r.token.as_bytes().to_vec(),
                classifier: r.classifier,
                level: r.level,
                progress_raw: r.progress_raw,
                cost_raw: r.cost_raw,
                cost_scale_raw: r.cost_scale_raw,
                cost_debug: r.cost_debug,
                research_speed_raw: r.slot.research_speed_raw,
                banked_days_raw: r.slot.banked_days_raw,
                applied_days_raw: r.slot.applied_days_raw,
                ahead_reduction_raw: r.ahead_reduction_raw,
                speed_bonus_raw: r.speed_bonus_raw,
                bonus_count: r.bonus_count,
            })),

            wrap(view::<tables::ResearchAppliedBonuses, _>(&self.research_bonuses, |_| true, |r| rows::ResearchAppliedBonusRow {
                country: r.country,
                index: r.index,
                ordinal: r.ordinal,
                technology_handle: r.technology_handle,
                id: r.value.id,
                resolved: r.resolved,
                name: r.value.name.as_bytes().to_vec(),
                ahead_reduction_raw: r.value.ahead_reduction_raw,
                speed_bonus_raw: r.value.speed_bonus_raw,
                cost_reduction_raw: r.cost_reduction_raw,
            })),
            wrap(view::<tables::ResearchModifiers, _>(&self.research_modifiers, |_| true, |r| rows::ResearchModifierRow {
                country: r.country, index: r.index, ordinal: r.ordinal, technology_handle: r.technology_handle,
                kind: match r.kind { engine::research_modifiers::Kind::Technology => rows::ResearchModifierRow::TECHNOLOGY, engine::research_modifiers::Kind::CountrySpeed => rows::ResearchModifierRow::COUNTRY_SPEED, engine::research_modifiers::Kind::CategorySpeed => rows::ResearchModifierRow::CATEGORY_SPEED, engine::research_modifiers::Kind::DesignTeam => rows::ResearchModifierRow::DESIGN_TEAM, engine::research_modifiers::Kind::Experience => rows::ResearchModifierRow::EXPERIENCE, engine::research_modifiers::Kind::Sharing => rows::ResearchModifierRow::SHARING, engine::research_modifiers::Kind::License => rows::ResearchModifierRow::LICENSE, engine::research_modifiers::Kind::Scientist => rows::ResearchModifierRow::SCIENTIST, engine::research_modifiers::Kind::Year => rows::ResearchModifierRow::YEAR, engine::research_modifiers::Kind::AheadFactor => rows::ResearchModifierRow::AHEAD_FACTOR, engine::research_modifiers::Kind::MinimumSpeed => rows::ResearchModifierRow::MINIMUM_SPEED, engine::research_modifiers::Kind::CountrySource => rows::ResearchModifierRow::COUNTRY_SOURCE, engine::research_modifiers::Kind::IdeaSource => rows::ResearchModifierRow::IDEA_SOURCE, engine::research_modifiers::Kind::ScriptSource => rows::ResearchModifierRow::SCRIPT_SOURCE, engine::research_modifiers::Kind::ScientistSource => rows::ResearchModifierRow::SCIENTIST_SOURCE, engine::research_modifiers::Kind::SharingMultiplier => rows::ResearchModifierRow::SHARING_MULTIPLIER, engine::research_modifiers::Kind::SharingMaximum => rows::ResearchModifierRow::SHARING_MAXIMUM, engine::research_modifiers::Kind::ModifierMetadata => rows::ResearchModifierRow::MODIFIER_METADATA, engine::research_modifiers::Kind::ScientistCategory => rows::ResearchModifierRow::SCIENTIST_CATEGORY, engine::research_modifiers::Kind::SharingMember => rows::ResearchModifierRow::SHARING_MEMBER, engine::research_modifiers::Kind::SharingPredecessor => rows::ResearchModifierRow::SHARING_PREDECESSOR, engine::research_modifiers::Kind::SharingFilter => rows::ResearchModifierRow::SHARING_FILTER, engine::research_modifiers::Kind::SharingCategory => rows::ResearchModifierRow::SHARING_CATEGORY, engine::research_modifiers::Kind::SharingDoctrine => rows::ResearchModifierRow::SHARING_DOCTRINE, engine::research_modifiers::Kind::SharingMissingMember => rows::ResearchModifierRow::SHARING_MISSING_MEMBER }, raw: r.raw, auxiliary: r.auxiliary, count: r.count, source_id: r.source_id, name: r.name.as_bytes().to_vec(),
            })),

        ]
    }
}

#[cfg(test)]
pub(crate) fn research_shadow_fixture(progress: i64) -> Captured {
    let mut captured = Captured::with_capacities(&Capacities {
        rows: [0; N],
        ..Default::default()
    });
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
    use engine::cnational_focus;

    #[test]
    fn hourly_research_conversion_preserves_native_inputs_and_bonus_failures() {
        let mut captured = Captured::vacant();
        let mut input = engine::research_inputs::Row::default();
        input.slot.country = 11;
        input.slot.index = 2;
        input.slot.technology_present = 1;
        input.slot.technology_handle = 77;
        input.token.bytes[..3].copy_from_slice(b"abc");
        input.token.len = 3;
        input.progress_raw = i64::MAX;
        input.speed_bonus_raw = i64::MIN;
        captured.research_inputs.rows.push(input);
        captured.research_inputs.outcome = Outcome::Rows { len: 1 };
        captured.research_bonuses.outcome = Outcome::Unavailable { reason: "bonus read failed" };
        let tables = captured.tables();
        let TableView::Complete(TableRows::ResearchInputs(rows)) = &tables[19] else { panic!("inputs missing") };
        assert_eq!(rows[0].index, 2);
        assert_eq!(rows[0].token, b"abc");
        assert_eq!(rows[0].progress_raw, i64::MAX);
        assert_eq!(rows[0].speed_bonus_raw, i64::MIN);
        assert!(matches!(&tables[20], TableView::Unavailable { .. }));
        for table in [TableId::ResearchInputs, TableId::ResearchAppliedBonuses, TableId::ResearchModifiers] {
            for hour in 0..8 { assert!(replay::cadence::should_walk(table, hour, false)); }
        }
    }

    #[test]
    fn hourly_modifier_conversion_preserves_identity_signed_operands_and_source_text() {
        let mut captured = Captured::vacant();
        let mut row = engine::research_modifiers::Row {
            country: 11, index: u32::MAX, ordinal: 7,
            kind: engine::research_modifiers::Kind::CountrySource,
            raw: i64::MIN, auxiliary: i64::MAX, count: u32::MAX, source_id: 91,
            ..Default::default()
        };
        row.name.bytes[..4].copy_from_slice(b"idea"); row.name.len = 4;
        captured.research_modifiers.rows.push(row);
        captured.research_modifiers.outcome = Outcome::Rows { len: 1 };
        let tables = captured.tables();
        let TableView::Complete(TableRows::ResearchModifiers(rows)) = &tables[21] else { panic!("modifiers missing") };
        assert_eq!((rows[0].country, rows[0].index, rows[0].ordinal), (11, u32::MAX, 7));
        assert_eq!((rows[0].raw, rows[0].auxiliary, rows[0].count), (i64::MIN, i64::MAX, u32::MAX));
        assert_eq!(rows[0].kind, rows::ResearchModifierRow::COUNTRY_SOURCE);
        assert_eq!(rows[0].name, b"idea");
    }

    #[test]
    fn owned_governance_vacancy_has_no_buffer_allocation() {
        fn assert_send<T: Send>() {}
        assert_send::<Captured>();
        let captured = Captured::vacant();
        assert_eq!(captured.allocated_bytes(), 0);
        assert!(captured.dictionary.is_empty());
        assert!(captured.technology_pointers.is_empty());
        assert_eq!(captured.technology_pointers.capacity(), 0);
        assert!(!captured.focus_raw_mode);
    }

    #[test]
    fn owned_governance_failure_preserves_cadence_and_focus_delegation() {
        for force_full in [false, true] {
            for delegated_focus in [false, true] {
                let mut captured = Captured::vacant();
                captured.focus_raw_mode = delegated_focus;
                captured.definition_text.first = vec![1, 2, 3];
                captured.definition_text.len = 3;
                captured.definition_text.required = 7;
                captured.modifier_values.first = vec![11, 12];
                captured.modifier_values.len = 2;
                captured.modifier_values.required = 4;
                captured.dictionary_required = 9;
                let context = CaptureContext {
                    base: 0,
                    game_state: 0,
                    hour: 1,
                    force_full,
                };
                let report = captured.fail_owned_capture(context, "worker stopped");
                assert_eq!(report.ticks, [0; N]);
                assert_eq!(captured.focus_raw_mode, delegated_focus);
                assert_eq!(captured.definition_text.len, 0);
                assert_eq!(captured.definition_text.required, 0);
                assert_eq!(captured.modifier_values.len, 0);
                assert_eq!(captured.modifier_values.required, 0);
                assert_eq!(captured.dictionary_required, 0);
                for (index, outcome) in captured.outcomes().into_iter().enumerate() {
                    let delegated = delegated_focus && index < 2;
                    let skipped =
                        !replay::cadence::should_walk(TABLES[index], context.hour, force_full);
                    assert_eq!(
                        outcome,
                        Outcome::Unavailable {
                            reason: if delegated {
                                "clock unavailable"
                            } else if skipped {
                                replay::cadence::SKIPPED_REASON
                            } else {
                                "worker stopped"
                            },
                        },
                        "{index}",
                    );
                }
            }
        }
    }

    #[test]
    fn focus_failure_preserves_other_governance_tables() {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.focus_raw_mode = true;
        captured.focus.outcome = Outcome::Rows { len: 1 };
        captured.completed.outcome = Outcome::Rows { len: 1 };
        captured.technologies.outcome = Outcome::Rows { len: 1 };
        let reason = "accepted acquisition worker stopped";
        captured.fail_focus_capture(true, reason);
        assert_eq!(captured.focus.outcome, Outcome::Unavailable { reason });
        assert_eq!(captured.completed.outcome, Outcome::Unavailable { reason });
        assert!(!captured.focus_raw_mode);
        assert_eq!(captured.technologies.outcome, Outcome::Rows { len: 1 });
    }

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
                TableInput::ComparedEqual(_) | TableInput::ComparedChanges { .. } => {
                    panic!("Research does not use comparison tokens")
                }
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
        for fault in [0, 1, 2, 3, 4, 6, 7, 8, 9, 10, 11, 12] {
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
        let mut job = FocusJob {
            rows: vec![Default::default(); 2],
            completed: vec![Default::default(); 3],
            bytes: vec![0; 10],
            provisioned: true,
            ..Default::default()
        };
        job.capacity_charge = Charge::empty_on(&LOCAL);
        job.update_capacity_charge();
        let expected = |job: &FocusJob| {
            vector_bytes(&job.rows) + vector_bytes(&job.completed) + vector_bytes(&job.bytes)
        };
        let initial = LOCAL.snapshot().current.payload_vectors;
        assert_eq!(initial, expected(&job));
        assert!(!job.prepare([4, 6], 20));
        job.grow();
        let grown = LOCAL.snapshot().current.payload_vectors;
        assert!(grown > initial);
        assert_eq!(grown, expected(&job));
        job.clear();
        assert_eq!(LOCAL.snapshot().current.payload_vectors, grown);
        drop(job);
        assert_eq!(LOCAL.snapshot().current, Capacity::default());
    }

    #[test]
    fn large_census_growth_leaves_quarter_capacity_free_without_power_of_two_inflation() {
        let mut capacity = Capacities::default();
        capacity.rows[2] = 4;
        let mut captured = Captured::with_capacities(&capacity);
        assert_eq!(
            captured.technology_pointers.len(),
            captured.technologies.rows.len()
        );
        let research_bytes = |captured: &Captured| {
            captured.technologies.allocated_bytes()
                + captured.technology_pointers.capacity() * size_of::<usize>()
        };
        let other_bytes = captured.allocated_bytes() - research_bytes(&captured);
        captured.technology_pointers[0] = 0x1234;
        captured.technologies.outcome = Outcome::Truncated {
            len: 4,
            unread_rows: 246_996,
        };
        captured.grow(&mut capacity);
        assert_eq!(capacity.rows[2], 329_334);
        assert!(247_000 <= capacity.rows[2] - capacity.rows[2] / 4);
        assert_eq!(captured.technology_pointers.len(), capacity.rows[2]);
        assert_eq!(
            captured.technology_pointers.len(),
            captured.technologies.rows.len()
        );
        assert_eq!(captured.technology_pointers[0], 0x1234);
        assert_eq!(
            captured.allocated_bytes(),
            other_bytes + research_bytes(&captured)
        );
        let pointers = captured.technology_pointers.as_ptr();
        let allocated = captured.allocated_bytes();
        captured.technologies.outcome = Outcome::Rows { len: 247_000 };
        captured.grow(&mut capacity);
        assert_eq!(capacity.rows[2], 329_334);
        captured.reset();
        assert_eq!(captured.technology_pointers.as_ptr(), pointers);
        assert_eq!(captured.technology_pointers[0], 0x1234);
        assert_eq!(captured.allocated_bytes(), allocated);
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
        let mut captured = Captured::with_capacities(&Capacities::default());
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
    /// Shared synthetic address space for the scheduled focus copy/decode split
    /// harness: two authenticated countries with inline, absent and heap focus
    /// tokens plus one- and two-entry completed rosters.
    pub(crate) mod focus_test_kit {
        use super::{Capacities, Captured, FocusJob};
        use crate::extended::governance::copy_focus_refresh;
        use engine::memory::SliceMemory;
        use engine::{
            ccontinuous_national_focus, ccountry, cgame_state, cnational_focus,
            cnational_focus_progress, cnational_focus_tree,
        };

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
                Captured::with_capacities(&Capacities::default())
            }

            pub(crate) fn copy_first(
                captured: &mut Captured,
                memory: &impl engine::Memory,
            ) -> (super::FocusCopyReport, FocusJob) {
                let mut job = FocusJob::new();
                let ctx = captured.focus_copy_context(BASE, GS, true);
                let report = copy_focus_refresh(memory, ctx, &mut job);
                (report, job)
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
        let mut typed = Fixture::captured();
        typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
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
        let (report, first) = Fixture::copy_first(&mut replayed, &memory);
        assert!(report.raw_ok);
        replayed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
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
        let mut typed_small = Captured::with_capacities(&small);
        typed_small.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
        let mut replayed_small = Captured::with_capacities(&small);
        replayed_small.focus_raw_mode = true;
        let (_, small_first) = Fixture::copy_first(&mut replayed_small, &memory);
        replayed_small.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
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
            let mut typed = Captured::with_capacities(&caps);
            typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
            let mut replayed = Captured::with_capacities(&caps);
            replayed.focus_raw_mode = true;
            let (report, first) = Fixture::copy_first(&mut replayed, &memory);
            assert!(
                report.raw_ok,
                "focus={focus_rows} completed={completed_rows}"
            );
            replayed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
            let mut ticks = [0; 2];
            replayed.decode_focus_payload(focus_test_kit::BASE, &first, &mut ticks);
            Fixture::assert_matches_typed(&replayed, &typed);
        }
        // Exact-fit arm pins Rows on both tables.
        let mut caps = Capacities::default();
        caps.rows[0] = 2;
        caps.rows[1] = 3;
        caps.focus_text = 88;
        let mut typed = Captured::with_capacities(&caps);
        typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
        assert_eq!(typed.focus.outcome, Outcome::Rows { len: 2 });
        assert_eq!(typed.completed.outcome, Outcome::Rows { len: 3 });
    }

    #[test]
    fn focus_copy_decode_matches_faults_with_identical_reasons() {
        use focus_test_kit::Fixture;
        // Null status root fails both tables with the same reason.
        let mut broken = Fixture::world();
        broken.w64(0x1_2000 + ccountry::FOCUS_PROGRESS, 0);
        let memory = broken.memory();
        let mut typed = Fixture::captured();
        typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
        assert!(matches!(typed.focus.outcome, Outcome::Unavailable { .. }));
        let mut replayed = Fixture::captured();
        replayed.focus_raw_mode = true;
        let (report, first) = Fixture::copy_first(&mut replayed, &memory);
        assert!(report.raw_ok);
        replayed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
        let mut ticks = [0; 2];
        replayed.decode_focus_payload(focus_test_kit::BASE, &first, &mut ticks);
        assert_eq!(replayed.focus.outcome, typed.focus.outcome);
        assert_eq!(replayed.completed.outcome, typed.completed.outcome);

        // Vtable mismatch fails both tables together.
        let mut broken = Fixture::world();
        broken.w64(0x1_6000, 0);
        let memory = broken.memory();
        let mut typed = Fixture::captured();
        typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
        let mut replayed = Fixture::captured();
        replayed.focus_raw_mode = true;
        let (_, first) = Fixture::copy_first(&mut replayed, &memory);
        replayed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
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
        typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
        assert!(matches!(typed.focus.outcome, Outcome::Unavailable { .. }));
        let mut replayed = Fixture::captured();
        replayed.focus_raw_mode = true;
        let (_, first) = Fixture::copy_first(&mut replayed, &memory);
        replayed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
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
        typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
        assert_eq!(typed.focus.outcome, Outcome::Rows { len: 2 });
        assert!(matches!(
            typed.completed.outcome,
            Outcome::Unavailable { .. }
        ));
        let mut replayed = Fixture::captured();
        replayed.focus_raw_mode = true;
        let (_, first) = Fixture::copy_first(&mut replayed, &memory);
        replayed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
        let mut ticks = [0; 2];
        replayed.decode_focus_payload(focus_test_kit::BASE, &first, &mut ticks);
        assert_eq!(replayed.focus.outcome, typed.focus.outcome);
        assert_eq!(replayed.completed.outcome, typed.completed.outcome);
    }

    struct FocusReads<'a> {
        memory: engine::memory::SliceMemory<'a>,
        reads: std::cell::Cell<usize>,
        fault: Option<usize>,
        failed: std::cell::Cell<usize>,
    }

    impl Memory for FocusReads<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            if self.fault == Some(address) && self.failed.get() == 0 {
                self.failed.set(1);
                return false;
            }
            self.memory.copy(address, destination)
        }
    }

    #[test]
    fn daily_governance_sources_keep_politics_and_research_siblings_independent() {
        use focus_test_kit::{BASE, Fixture};
        struct Sources<'a> {
            memory: engine::memory::SliceMemory<'a>,
            ideas: std::cell::Cell<usize>,
            fail_ideas: bool,
        }
        impl Memory for Sources<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let private = [
                    0x1_9000 + cpolitical_status::IDEAS,
                    0x1_a000 + cpolitical_status::IDEAS,
                ];
                if private
                    .iter()
                    .any(|&at| address <= at && at < address.saturating_add(output.len()))
                {
                    self.ideas.set(self.ideas.get() + 1);
                    if self.fail_ideas {
                        return false;
                    }
                }
                self.memory.copy(address, output)
            }
        }
        let mut fixture = Fixture::world();
        for (country, politics) in [(0x1_2000, 0x1_9000), (0x1_4000, 0x1_a000)] {
            fixture.w64(country + ccountry::POLITICAL_STATUS, politics as u64);
            fixture.w64(politics, (BASE + cpolitical_status::VTABLE_RVA) as u64);
            fixture.w64(politics + cpolitical_status::COUNTRY, country as u64);
        }
        for fail_ideas in [false, true] {
            for hour in 0..24 {
                let memory = engine::memory::CountedMemory::new(Sources {
                    memory: fixture.memory(),
                    ideas: Default::default(),
                    fail_ideas,
                });
                let mut captured = Fixture::captured();
                let accounting = read_accounting::Capture::new();
                captured.capture(&memory, BASE, 0x1_0000, hour, false);
                let counts = accounting.finish();
                assert_eq!(captured.politics.outcome, Outcome::Rows { len: 2 });
                assert_eq!(captured.parties.outcome, Outcome::Rows { len: 0 });
                assert_eq!(memory.0.ideas.get() > 0, hour == 15);
                for (index, slot) in [(0, 19), (2, 0), (3, 11)] {
                    assert_eq!(
                        counts[TABLES[index].index()].spans > 0,
                        hour == slot,
                        "table={:?} hour={hour}",
                        TABLES[index]
                    );
                }
                for (index, outcome) in captured.outcomes().into_iter().enumerate() {
                    if !replay::cadence::should_walk(TABLES[index], hour, false) {
                        assert_eq!(
                            outcome,
                            Outcome::Unavailable {
                                reason: replay::cadence::SKIPPED_REASON
                            }
                        );
                        assert_eq!(counts[TABLES[index].index()].spans, 0);
                    }
                }
                if fail_ideas && hour == 15 {
                    assert!(matches!(
                        captured.ideas.outcome,
                        Outcome::Unavailable { .. }
                    ));
                }
            }
        }
    }

    #[test]
    fn failed_native_technology_name_does_not_request_deferred_arena_growth() {
        use focus_test_kit::{BASE, Fixture};
        const TEMPLATE: usize = 0x1_c000;
        const HEAP: usize = 0x9000_0000;
        const LENGTH: usize = 32768;
        let mut fixture = Fixture::world();
        for (owner, country) in [0x1_2000, 0x1_4000].into_iter().enumerate() {
            let status = 0x1_a000 + owner * 0x200;
            let roster = 0x1_a400 + owner * 0x20;
            let technology = 0x1_b000 + owner * 0x200;
            fixture.w64(country + ccountry::TECHNOLOGY_STATUS, status as u64);
            fixture.w64(status, (BASE + ctechnology_status::VTABLE_RVA) as u64);
            fixture.w64(status + ctechnology_status::COUNTRY, country as u64);
            fixture.w64(status + ctechnology_status::TECHNOLOGIES_DATA, roster as u64);
            for offset in [8, 12] {
                fixture.w32(status + ctechnology_status::TECHNOLOGIES_DATA + offset, 1);
            }
            fixture.w64(roster, technology as u64);
            fixture.w64(technology, (BASE + ctechnology::VTABLE_RVA) as u64);
            fixture.w32(technology + ctechnology::DEFINITION_INDEX, 3);
            fixture.w64(technology + ctechnology::STATUS, status as u64);
            fixture.w64(technology + ctechnology::TEMPLATE, TEMPLATE as u64);
        }
        fixture.w64(TEMPLATE + ctechnology_template::ID, HEAP as u64);
        fixture.w64(TEMPLATE + ctechnology_template::ID + 16, LENGTH as u64);
        fixture.w64(TEMPLATE + ctechnology_template::ID + 24, LENGTH as u64);
        fixture.w32(TEMPLATE + ctechnology_template::DATABASE_INDEX, 1);
        fixture.w32(TEMPLATE + ctechnology_template::TECHNOLOGY_HANDLE, 3);
        struct Fault<'a>(engine::memory::SliceMemory<'a>);
        impl Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == HEAP + LENGTH && output.len() == 1 {
                    output[0] = 0;
                    return true;
                }
                self.0.copy(address, output)
            }
        }
        let mut capacity = Capacities { definition_text: 0, ..Default::default() };
        let mut captured = Captured::with_capacities(&capacity);
        captured.capture(&Fault(fixture.memory()), BASE, 0x1_0000, 0, true);
        assert_eq!(captured.technologies.outcome, Outcome::Rows { len: 2 });
        assert_eq!(captured.definitions.outcome, Outcome::Unavailable {
            reason: "technology id bytes unreadable",
        });
        assert_eq!(captured.definition_text.required, 0);
        captured.grow(&mut capacity);
        assert_eq!(capacity.definition_text, 0);
        assert_eq!(captured.definition_text.first.capacity(), 0);
    }

    #[test]
    fn first_research_capture_grows_all_storage_and_keeps_native_and_definition_order() {
        use focus_test_kit::{BASE, Fixture};
        let mut fixture = Fixture::world();
        for (owner, country) in [0x1_2000, 0x1_4000].into_iter().enumerate() {
            let status = 0x1_a000 + owner * 0x200;
            let roster = 0x1_a400 + owner * 0x20;
            fixture.w64(country + ccountry::TECHNOLOGY_STATUS, status as u64);
            fixture.w64(status, (BASE + ctechnology_status::VTABLE_RVA) as u64);
            fixture.w64(status + ctechnology_status::COUNTRY, country as u64);
            fixture.w64(
                status + ctechnology_status::TECHNOLOGIES_DATA,
                roster as u64,
            );
            for offset in [8, 12] {
                fixture.w32(status + ctechnology_status::TECHNOLOGIES_DATA + offset, 2);
            }
            for index in 0..2 {
                let technology = 0x1_b000 + (owner * 2 + index) * 0x200;
                let definition = (owner + index) % 2;
                let template = 0x1_c000 + definition * 0x600;
                fixture.w64(roster + index * 8, technology as u64);
                fixture.w64(technology, (BASE + ctechnology::VTABLE_RVA) as u64);
                fixture.w32(
                    technology + ctechnology::DEFINITION_INDEX,
                    [0x8000_0001, 3][definition],
                );
                fixture.w64(technology + ctechnology::STATUS, status as u64);
                fixture.w64(technology + ctechnology::TEMPLATE, template as u64);
            }
        }
        for (index, handle) in [0x8000_0001, 3].into_iter().enumerate() {
            let template = 0x1_c000 + index * 0x600;
            fixture.w64(
                template + ctechnology_template::ID,
                u32::from_le_bytes(*b"tech") as u64,
            );
            fixture.w64(template + ctechnology_template::ID + 16, 4);
            fixture.w64(template + ctechnology_template::ID + 24, 15);
            fixture.w32(
                template + ctechnology_template::DATABASE_INDEX,
                index as u32 + 1,
            );
            fixture.w32(template + ctechnology_template::TECHNOLOGY_HANDLE, handle);
        }
        struct Reads<'a> {
            memory: engine::memory::SliceMemory<'a>,
            spans: std::cell::RefCell<Vec<(usize, usize)>>,
        }
        impl Memory for Reads<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.spans.borrow_mut().push((address, destination.len()));
                self.memory.copy(address, destination)
            }
        }
        let memory = Reads {
            memory: fixture.memory(),
            spans: Default::default(),
        };
        let mut captured = Captured::vacant();
        assert_eq!(captured.allocated_bytes(), 0);
        let mut enabled = [false; N];
        enabled[2] = true;
        enabled[4] = true;
        let result = acquire(&memory, BASE, 0x1_0000, captured.buffers(), &enabled);
        assert_eq!(result.outcomes[2], Outcome::Rows { len: 4 });
        assert_eq!(result.outcomes[4], Outcome::Rows { len: 2 });
        assert_eq!(result.lens[1], 8);
        assert_eq!(
            captured.technologies.rows[..4]
                .iter()
                .map(|row| (row.country, row.technology_handle))
                .collect::<Vec<_>>(),
            [(1, 0x8000_0001), (1, 3), (2, 3), (2, 0x8000_0001)]
        );
        assert_eq!(
            captured
                .definitions
                .rows
                .iter()
                .map(|row| row.technology_handle)
                .collect::<Vec<_>>(),
            [3, 0x8000_0001]
        );
        assert_eq!(
            captured.allocated_bytes(),
            captured.technologies.rows.capacity() * size_of::<ctechnology::Row>()
                + captured.technology_pointers.capacity() * size_of::<usize>()
                + captured.definitions.rows.capacity() * size_of::<ctechnology_template::Row>()
                + captured.definition_text.first.capacity()
                + captured.dictionary.capacity() * size_of::<ctechnology_template::Entry>()
        );
        let spans = memory.spans.borrow();
        for owner in 0..2 {
            assert_eq!(
                spans
                    .iter()
                    .filter(|&&span| span
                        == (
                            0x1_a000 + owner * 0x200 + ctechnology_status::TECHNOLOGIES_DATA,
                            16
                        ))
                    .count(),
                1
            );
            assert_eq!(
                spans
                    .iter()
                    .filter(
                        |&&span| span == (0x1_c000 + owner * 0x600 + ctechnology_template::ID, 32)
                    )
                    .count(),
                1
            );
        }
    }

    #[test]
    fn inactive_focus_bypasses_owned_capacity_and_decode_preflights() {
        use focus_test_kit::{BASE, Fixture};
        let fixture = Fixture::world();
        let memory = FocusReads {
            memory: fixture.memory(),
            reads: Default::default(),
            fault: Some(0x1_6000),
            failed: Default::default(),
        };
        for hour in 0..24 {
            if replay::cadence::should_walk(TableId::Focus, hour, false) {
                continue;
            }
            let mut job = FocusJob::default();
            let report = copy_focus_refresh(
                &memory,
                FocusCopyContext {
                    base: BASE,
                    game_state: 0x1_0000,
                    active: false,
                    rows: [usize::MAX; 2],
                    bytes: usize::MAX,
                },
                &mut job,
            );
            assert!(report.raw_ok);
            assert_eq!(report.copy_ticks, [0; 2]);
            assert_eq!(memory.reads.get(), 0);
            let mut captured = Fixture::captured();
            captured.politics.outcome = Outcome::Rows { len: 1 };
            let mut ticks = [0; 2];
            captured.decode_focus_payload(BASE, &job, &mut ticks);
            assert_eq!(ticks, [0; 2]);
            assert_eq!(
                captured.focus.outcome,
                Outcome::Unavailable {
                    reason: replay::cadence::SKIPPED_REASON
                }
            );
            assert_eq!(captured.completed.outcome, captured.focus.outcome);
            assert_eq!(captured.politics.outcome, Outcome::Rows { len: 1 });
        }
    }

    #[test]
    fn focus_preflights_primary_grants_before_any_source_read() {
        use focus_test_kit::{BASE, Fixture};
        let fixture = Fixture::world();
        for storage in 0..3 {
            let memory = engine::memory::CountedMemory::new(FocusReads {
                memory: fixture.memory(),
                reads: Default::default(),
                fault: Some(0x1_6000),
                failed: Default::default(),
            });
            let mut captured = Fixture::captured();
            let ctx = captured.focus_copy_context(BASE, 0x1_0000, true);
            let mut job = FocusJob::new();
            match storage {
                0 => job.rows.truncate(ctx.rows[0] - 1),
                1 => job.completed.truncate(ctx.rows[1] - 1),
                _ => job.bytes.truncate(ctx.bytes - 1),
            }
            let native = read_accounting::Capture::new();
            let report = copy_focus_refresh(&memory, ctx, &mut job);
            assert!(!report.raw_ok, "storage={storage}");
            assert_eq!(native.finish(), read_accounting::ZERO);
            assert_eq!(memory.0.reads.get(), 0);
            assert_eq!(memory.0.failed.get(), 0);
            assert!(job.outcomes.iter().all(Option::is_none));
            let native = read_accounting::Capture::new();
            captured.capture(&memory, BASE, 0x1_0000, 0, true);
            assert!(native.finish()[TABLES[0].index()].spans > 0);
            assert_eq!(memory.0.failed.get(), 1);
            assert!(matches!(
                captured.focus.outcome,
                Outcome::Unavailable { .. }
            ));
            assert!(job.grow_for_next());
            job.grow();
            assert!(!job.grow_for_next());
            job.clear();
            assert!(job.is_provisioned());
            assert_eq!(job.outcomes, [None; 2]);
        }
        let memory = FocusReads {
            memory: fixture.memory(),
            reads: Default::default(),
            fault: None,
            failed: Default::default(),
        };
        let captured = Fixture::captured();
        let ctx = captured.focus_copy_context(BASE, 0x1_0000, true);
        let report = copy_focus_refresh(&memory, ctx, &mut FocusJob::default());
        assert!(!report.raw_ok);
        assert_eq!(memory.reads.get(), 0);
    }

    #[test]
    fn focus_refresh_reads_one_pass_and_owned_decode_never_reads_live_memory() {
        use focus_test_kit::{BASE, Fixture};
        let fixture = Fixture::world();
        let source = engine::memory::CountedMemory::new(FocusReads {
            memory: fixture.memory(),
            reads: Default::default(),
            fault: None,
            failed: Default::default(),
        });
        let reference = engine::memory::CountedMemory::new(FocusReads {
            memory: fixture.memory(),
            reads: Default::default(),
            fault: None,
            failed: Default::default(),
        });
        let mut captured = Fixture::captured();
        let ctx = captured.focus_copy_context(BASE, 0x1_0000, true);
        let mut expected = FocusJob::new();
        assert!(expected.prepare(ctx.rows, ctx.bytes));
        let native = read_accounting::Capture::new();
        copy_focus_pass(&mut expected, &reference, BASE, 0x1_0000, &mut [0; 2]);
        let expected_counts = native.finish();
        let mut job = FocusJob::new();
        let native = read_accounting::Capture::new();
        let report = copy_focus_refresh(&source, ctx, &mut job);
        assert!(report.raw_ok);
        assert!(source.0.reads.get() > 0);
        assert_eq!(source.0.reads.get(), reference.0.reads.get());
        assert_eq!(read_accounting::snapshot(), expected_counts);
        let mut anchored = read_accounting::ZERO;
        anchored[TABLES[0].index()] = expected_counts[TABLES[0].index()];
        assert!(anchored[TABLES[0].index()].spans > 0);
        assert_eq!(expected_counts, anchored);
        assert_eq!(job.outcomes, expected.outcomes);
        assert_eq!(&job.rows[..2], &expected.rows[..2]);
        assert_eq!(&job.completed[..3], &expected.completed[..3]);
        assert_eq!(
            &job.bytes[..job.bytes_len],
            &expected.bytes[..expected.bytes_len]
        );
        let reads = source.0.reads.get();
        captured.decode_focus_payload(BASE, &job, &mut [0; 2]);
        assert_eq!(source.0.reads.get(), reads);
        assert_eq!(native.finish(), expected_counts);
        assert_eq!(captured.focus.outcome, Outcome::Rows { len: 2 });
        assert_eq!(captured.completed.outcome, Outcome::Rows { len: 3 });
        assert_eq!(&captured.focus.rows[..2], &job.rows[..2]);
        assert_eq!(&captured.completed.rows[..3], &job.completed[..3]);
    }

    #[test]
    fn focus_native_faults_are_terminal_and_decode_cannot_retry_sources() {
        use focus_test_kit::{BASE, Fixture};
        let fixture = Fixture::world();
        for (fault, unavailable) in [
            (0x1_0000 + engine::cgame_state::COUNTRIES, [true, true]),
            (0x1_6000, [true, true]),
            (0x1_6000 + cnational_focus_progress::PROGRESS, [true, false]),
            (0x1_8800, [false, true]),
            (0x1_8a00, [false, true]),
        ] {
            let memory = FocusReads {
                memory: fixture.memory(),
                reads: Default::default(),
                fault: Some(fault),
                failed: Default::default(),
            };
            let mut captured = Fixture::captured();
            let (report, first) = Fixture::copy_first(&mut captured, &memory);
            assert!(report.raw_ok);
            assert_eq!(memory.failed.get(), 1);
            let reads = memory.reads.get();
            captured.focus_raw_mode = true;
            captured.decode_focus_payload(BASE, &first, &mut [0; 2]);
            assert_eq!(memory.reads.get(), reads);
            assert!(!captured.focus_raw_mode);
            for (slot, outcome) in [captured.focus.outcome, captured.completed.outcome]
                .into_iter()
                .enumerate()
            {
                assert_eq!(
                    matches!(outcome, Outcome::Unavailable { .. }),
                    unavailable[slot]
                );
            }
        }
    }

    #[test]
    fn focus_install_checks_original_grants_and_keeps_unaffected_table() {
        use focus_test_kit::{BASE, Fixture};
        let fixture = Fixture::world();
        for changed in 0..3 {
            let mut captured = Fixture::captured();
            let (_, first) = Fixture::copy_first(&mut captured, &fixture.memory());
            match changed {
                0 => {
                    captured.focus.rows.pop();
                }
                1 => {
                    captured.completed.rows.pop();
                }
                _ => {
                    captured.focus_text.first.pop();
                }
            }
            captured.decode_focus_payload(BASE, &first, &mut [0; 2]);
            assert_eq!(
                matches!(captured.focus.outcome, Outcome::Unavailable { .. }),
                changed != 1
            );
            assert_eq!(
                matches!(captured.completed.outcome, Outcome::Unavailable { .. }),
                changed != 0
            );
        }
    }

    #[test]
    fn focus_pair_uses_daily_slot_nineteen() {
        for hour in -24_i32..48 {
            assert_eq!(
                replay::cadence::should_walk(TableId::Focus, hour, false),
                hour.rem_euclid(24) == 19,
                "hour {hour}"
            );
            assert_eq!(
                replay::cadence::should_walk(TableId::CompletedFocuses, hour, false),
                hour.rem_euclid(24) == 19,
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
