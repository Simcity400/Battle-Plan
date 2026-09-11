//! Governance rows and arenas are allocated off the game thread.
//! Shared-walk costs are charged once to each READ_GROUPS leader, in QPC ticks.

use crate::{
    sampling::{CONSISTENCY_TARGET, qpc},
    table_capture::{Arena, CaptureRows, comparison, grow_capacity, grow_vec},
};
use engine::{
    Memory, ccombat_tactic, ccountry, cdecision_status, cdynamic_modifier_container,
    cnational_focus_progress, cpolitical_party, cpolitical_status, cresearch_slot, ctechnology,
    ctechnology_sharing_group, ctechnology_status, ctechnology_template,
    power_balance_gfx_override, power_balance_manager, power_balance_range, power_balance_record,
    walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{self, TableId, TableSchema, tables},
};
use std::mem::size_of;

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
        let walk: [bool; N] = std::array::from_fn(|i| {
            replay::cadence::should_walk(TABLES[i], hour, force_full)
        });
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
}
