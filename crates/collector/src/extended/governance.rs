//! Governance rows and arenas are allocated off the game thread.
//! Shared-walk costs are charged once to each READ_GROUPS leader, in QPC ticks.

use crate::{
    read_accounting,
    sampling::qpc,
    slicing::{SliceFingerprints, SliceSelection},
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
};
use engine::allocation::{copy_vec, default_vec, owned_vec, reserve, reserve_push_batch};
use engine::{
    Memory, ccombat_tactic, ccountry, cdecision_status, cdynamic_modifier_container,
    cnational_focus_progress, cpolitical_party, cpolitical_status, cresearch_slot, ctechnology,
    ctechnology_sharing_group, ctechnology_status, ctechnology_template,
    power_balance_gfx_override, power_balance_manager, power_balance_range, power_balance_record,
    walk::Outcome,
};
#[cfg(test)]
use replay::rows::TableRows;
use replay::{
    frame::{TableInput, TableView},
    rows::{self, RowCodec, TableId, TableSchema, tables},
};
use std::mem::size_of;

mod demand;
pub(crate) use demand::status as demand_status;

pub const N: usize = 25;

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
    TableId::TechSharingGroups,
    TableId::TechSharingMembers,
    TableId::PowerBalances,
    TableId::PowerBalanceMembers,
    TableId::PowerBalanceRanges,
    TableId::PowerBalanceOverrides,
    TableId::ResearchInputs,
    TableId::ResearchAppliedBonuses,
    TableId::ResearchModifiers,
    TableId::PartyLeaders,
    TableId::PowerBalanceModifierBreakdowns,
    TableId::TimedIdeas,
    TableId::DecisionRandomItems,
];
/// Name, local cost anchor and local members. Reports sum member ticks;
/// physically shared walks charge their anchor once and leave followers zero.
/// Country authentication is included in hourly Politics and shared by tables 0..13.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("Focus", 0, &[0, 1]),
    ("Research technologies and definitions", 2, &[2, 4]),
    ("Research slots", 3, &[3]),
    (
        "Politics, parties, party leaders, ideas and shared country authentication",
        5,
        &[5, 6, 7, 21],
    ),
    ("Decision counts and state", 8, &[8, 9]),
    ("Dynamic modifiers", 10, &[10]),
    ("Preferred tactic", 11, &[11]),
    ("Technology sharing", 12, &[12, 13]),
    ("Power balance and children", 14, &[14, 15, 16, 17, 22]),
    (
        "Sliced research inputs, bonuses and modifiers",
        18,
        &[18, 19, 20],
    ),
    ("Timed ideas", 23, &[23]),
    ("Decision random counts", 24, &[24]),
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

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacities {
    rows: [usize; N],
    focus_text: usize,
    definition_text: usize,
    party_text: usize,
    idea_text: usize,
    timed_idea_text: usize,
    random_item_text: usize,
    decision_text: usize,
    modifier_text: usize,
    modifier_values: usize,
    tactic_text: usize,
    balance_text: usize,
    party_countries: usize,
    idea_countries: usize,
    decision_countries: usize,
    effect_tokens: usize,
    effect_frames: usize,
    effect_names: usize,
    effect_bytes: usize,
    effect_events: usize,
    dictionary: usize,
}

impl Capacities {
    #[cfg(test)]
    pub(super) fn for_test_research(rows: usize) -> Self {
        let mut capacity = Self::default();
        for index in RESEARCH {
            capacity.rows[index] = rows;
        }
        capacity
    }

    /// Off-thread hints from complete live party/modifier headers. Each
    /// independent family raises storage even if a sibling hint fails.
    pub(crate) fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        demand::presize(self, memory, base, state)
    }

    pub(crate) fn raise_for_effect_inputs(
        &mut self,
        timed: cpolitical_status::IdeaDemand,
        random: cdecision_status::RandomItemDemand,
    ) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[23], rows(timed.rows, 1));
        raise(&mut self.rows[24], rows(random.rows, 1));
        raise(&mut self.timed_idea_text, rows(timed.bytes, 1));
        raise(&mut self.random_item_text, rows(random.bytes, 1));
    }

    /// One row per canonical country for Focus, Politics, DecisionCounts and
    /// the preferred-tactic table. Variable rosters and arenas use their
    /// own live demand, including each country's research slot header.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        use crate::country_scale::{raise, rows};
        for index in [0, 5, 8, 11] {
            raise(&mut self.rows[index], rows(slots, 1));
        }
    }

    /// PartyLeaders from the live leader total
    /// (`country_scale::observe_politics`), with the three-quarters growth
    /// rule's headroom. Never lowers.
    pub(crate) fn raise_for_party_leaders(&mut self, leaders: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[21], rows(leaders, 1));
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
    breakdowns: Vec<rows::PowerBalanceModifierBreakdownRow>,
    breakdown_outcome: Outcome,
    focus: CaptureRows<cnational_focus_progress::Row>,
    completed: CaptureRows<cnational_focus_progress::CompletedRow>,
    technologies: CaptureRows<ctechnology::Row>,
    /// Acquisition-only native pointers, parallel to the current technology rows.
    technology_pointers: Vec<usize>,
    slots: CaptureRows<cresearch_slot::Row>,
    definitions: CaptureRows<ctechnology_template::Row>,
    politics: CaptureRows<CountryRow<cpolitical_status::Row>>,
    parties: CaptureRows<cpolitical_party::Row>,
    party_leaders: CaptureRows<cpolitical_party::LeaderRow>,
    ideas: CaptureRows<cpolitical_status::IdeaRow>,
    timed_ideas: CaptureRows<cpolitical_status::TimedIdeaRow>,
    random_items: CaptureRows<cdecision_status::RandomItemRow>,
    counts: CaptureRows<CountryRow<cdecision_status::Counts>>,
    decisions: CaptureRows<cdecision_status::Row>,
    modifiers: CaptureRows<cdynamic_modifier_container::Row>,
    tactics: CaptureRows<TacticRow>,
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
    timed_idea_text: Arena<u8>,
    random_item_text: Arena<u8>,
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
    effect_tokens: Arena<engine::border_effect_demand::Token>,
    effect_frames: Arena<engine::border_effect_demand::Frame>,
    effect_names: Arena<engine::border_effect_demand::Name>,
    effect_bytes: Arena<u8>,
    effect_events: Arena<u32>,
    /// Owned acquisition gate for the scheduled focus pair (Focus,
    /// CompletedFocuses). Set for an accepted World payload; `capture` skips
    /// duplicate focus walks and the decode worker installs the owned rows
    /// before the ready handoff. Preserved across
    /// `reset` (set per hour, cleared by the worker after decode).
    pub(crate) focus_raw_mode: bool,
    /// The hourly research tables are loaned to their own job for this
    /// callback; `capture` leaves them to it and `restore_research` returns them.
    research_loaned: bool,
    /// Which countries the research rows cover (see `ResearchPart`).
    research_selection: SliceSelection,
    /// Which countries the technology rows cover: the `technologies` sliced
    /// family, read in this family's country walk.
    technology_selection: SliceSelection,
}

/// Research inputs, applied bonuses and modifiers: the governance family's
/// largest walk, acquired by its own job so it overlaps the rest of the
/// family. The tables are [`replay::cadence::Cadence::Sliced`]: each hour reads
/// one country slice, plus every country whose assignment fingerprint changed
/// since it was last observed, and the recorder merges the result into the
/// previous complete tables. Rows keep each read's own order and outcomes.
pub(crate) struct ResearchPart {
    inputs: CaptureRows<engine::research_inputs::Row>,
    bonuses: CaptureRows<engine::research_inputs::BonusRow>,
    modifiers: CaptureRows<engine::research_modifiers::Row>,
    selection: SliceSelection,
}

/// Indices of the research tables in `TABLES`.
pub(crate) const RESEARCH: [usize; 3] = [18, 19, 20];

/// Indices of the technologies family's tables in `TABLES`.
pub(crate) const TECHNOLOGIES: [usize; 2] = [2, 4];

impl ResearchPart {
    /// Read this hour's research rows: every country on a full hour, else
    /// the hour's country slice and every country whose assignment
    /// fingerprint changed since it was last observed. The fingerprint only
    /// adds reads; it never replaces one. Returns ticks indexed like
    /// `RESEARCH`; the scan and walk are charged to ResearchInputs.
    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        fingerprints: &mut SliceFingerprints,
    ) -> [u64; 3] {
        self.selection.reset(context.hour, context.force_full);
        let enabled = RESEARCH.map(|index| {
            replay::cadence::should_walk(TABLES[index], context.hour, context.force_full)
        });
        let mut outcomes = [Outcome::Rows { len: 0 }; 3];
        for (outcome, &active) in outcomes.iter_mut().zip(&enabled) {
            if !active {
                *outcome = Outcome::Unavailable {
                    reason: replay::cadence::SKIPPED_REASON,
                };
            }
        }
        let mut ticks = [0; 3];
        if enabled[0] {
            let _reads = read_accounting::table(TABLES[RESEARCH[0]]);
            let started = qpc();
            let research_context = engine::research_inputs::Context::read(memory, context.base);
            let modifier_context = if enabled[2] {
                engine::research_modifiers::Context::read(memory, context.base)
            } else {
                Err("research modifiers not sampled")
            };
            let fail = |outcomes: &mut [Outcome; 3], reason| {
                for (outcome, &active) in outcomes.iter_mut().zip(&enabled) {
                    if active {
                        *outcome = Outcome::Unavailable { reason };
                    }
                }
            };
            match ccountry::Reader::new(memory, context.base, context.game_state) {
                Err(reason) => fail(&mut outcomes, reason),
                Ok(countries) => {
                    // Scan first, so a changed country joins this hour's read.
                    let scan = qpc();
                    let mut census = true;
                    for slot in 1..countries.count() {
                        let country = match countries.at(slot) {
                            Ok(Some(country)) => country,
                            Ok(None) => continue,
                            Err(_) => {
                                census = false;
                                continue;
                            }
                        };
                        let value = engine::research_inputs::fingerprint_country(
                            memory,
                            context.base,
                            country.pointer,
                        )
                        .ok();
                        self.selection.scan(fingerprints, country.id, value);
                    }
                    fingerprints.finish_scan();
                    if census {
                        self.selection.finish_census(fingerprints);
                    }
                    self.selection.scan_ticks = elapsed(scan);
                    for slot in 1..countries.count() {
                        let country = match countries.at(slot) {
                            Ok(Some(country)) => country,
                            Ok(None) => continue,
                            Err(reason) => {
                                fail(&mut outcomes, reason);
                                break;
                            }
                        };
                        if !self.selection.contains(country.id) {
                            continue;
                        }
                        let input_start = outcomes[0].len();
                        let (inputs, bonuses) = engine::research_inputs::walk_country_buffer(
                            memory,
                            context.base,
                            country.pointer,
                            country.id,
                            research_context,
                            engine::walk::Buffer::Growing(&mut self.inputs.rows)
                                .suffix(outcomes[0].len()),
                            engine::walk::Buffer::Growing(&mut self.bonuses.rows)
                                .suffix(outcomes[1].len()),
                        );
                        if enabled[2] {
                            let modifiers = match inputs {
                                Outcome::Rows { len } => {
                                    engine::research_modifiers::walk_country_buffer(
                                        memory,
                                        context.base,
                                        &countries,
                                        country,
                                        modifier_context,
                                        &self.inputs.rows[input_start..input_start + len],
                                        engine::walk::Buffer::Growing(&mut self.modifiers.rows)
                                            .suffix(outcomes[2].len()),
                                    )
                                }
                                _ => Outcome::Unavailable {
                                    reason: "research modifier assignments incomplete",
                                },
                            };
                            append(&mut outcomes[2], modifiers);
                        }
                        append(&mut outcomes[0], inputs);
                        append(&mut outcomes[1], bonuses);
                    }
                }
            }
            ticks[0] = elapsed(started);
        }
        self.inputs.outcome = outcomes[0];
        self.bonuses.outcome = outcomes[1];
        self.modifiers.outcome = outcomes[2];
        ticks
    }

    /// Terminal completion after an accepted worker failure. Partial rows
    /// cannot become publishable, cadence skips stay skips, and nothing is read.
    pub(crate) fn fail(&mut self, context: CaptureContext, reason: &'static str) {
        let [inputs, bonuses, modifiers] = RESEARCH.map(|index| {
            let active =
                replay::cadence::should_walk(TABLES[index], context.hour, context.force_full);
            Outcome::Unavailable {
                reason: if active {
                    reason
                } else {
                    replay::cadence::SKIPPED_REASON
                },
            }
        });
        self.inputs.reset();
        self.bonuses.reset();
        self.modifiers.reset();
        self.inputs.outcome = inputs;
        self.bonuses.outcome = bonuses;
        self.modifiers.outcome = modifiers;
        self.selection.reset(context.hour, context.force_full);
    }
}

impl Captured {
    /// Loan the research rows to their own job. Allocation-free: the
    /// placeholders have no capacity, and `capture` leaves them untouched.
    pub(crate) fn take_research(&mut self) -> ResearchPart {
        debug_assert!(!self.research_loaned, "research part already loaned");
        self.research_loaned = true;
        ResearchPart {
            inputs: std::mem::replace(&mut self.research_inputs, CaptureRows::new(0)),
            bonuses: std::mem::replace(&mut self.research_bonuses, CaptureRows::new(0)),
            modifiers: std::mem::replace(&mut self.research_modifiers, CaptureRows::new(0)),
            selection: std::mem::replace(&mut self.research_selection, SliceSelection::new(false)),
        }
    }

    /// Return the loaned research rows with their outcomes. Call after the
    /// family is restored and before any view, growth or publication.
    pub(crate) fn restore_research(&mut self, part: ResearchPart) {
        debug_assert!(self.research_loaned, "research part was not loaned");
        self.research_loaned = false;
        self.research_inputs = part.inputs;
        self.research_bonuses = part.bonuses;
        self.research_modifiers = part.modifiers;
        self.research_selection = part.selection;
    }

    /// Start a frame's sliced selections at its hour; the jobs reset them again.
    pub(crate) fn reset_slice_selections(&mut self, hour: i32, force_full: bool) {
        if !self.research_loaned {
            self.research_selection.reset(hour, force_full);
        }
        self.technology_selection.reset(hour, force_full);
    }

    /// The countries whose technology rows this frame's read replaced.
    pub(crate) fn technology_selection(&self) -> &SliceSelection {
        &self.technology_selection
    }

    /// The countries whose research rows this frame's read replaced.
    pub(crate) fn research_selection(&self) -> &SliceSelection {
        &self.research_selection
    }
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
            timed_idea_text: 0,
            random_item_text: 0,
            decision_text: 0,
            modifier_text: 0,
            modifier_values: 0,
            tactic_text: 0,
            balance_text: 0,
            party_countries: 0,
            idea_countries: 0,
            decision_countries: 0,
            effect_tokens: 0,
            effect_frames: 0,
            effect_names: 0,
            effect_bytes: 0,
            effect_events: 0,
            dictionary: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            focus: CaptureRows::new(capacity.rows[0]),
            completed: CaptureRows::new(capacity.rows[1]),
            technologies: CaptureRows::new(capacity.rows[2]),
            technology_pointers: owned_vec(default_vec(capacity.rows[2])),
            slots: CaptureRows::new(capacity.rows[3]),
            definitions: CaptureRows::new(capacity.rows[4]),
            politics: CaptureRows::new(capacity.rows[5]),
            parties: CaptureRows::new(capacity.rows[6]),
            party_leaders: CaptureRows::new(capacity.rows[21]),
            ideas: CaptureRows::new(capacity.rows[7]),
            timed_ideas: CaptureRows::new(capacity.rows[23]),
            random_items: CaptureRows::new(capacity.rows[24]),
            counts: CaptureRows::new(capacity.rows[8]),
            decisions: CaptureRows::new(capacity.rows[9]),
            effect_tokens: Arena::new(capacity.effect_tokens),
            effect_frames: Arena::new(capacity.effect_frames),
            effect_names: Arena::new(capacity.effect_names),
            effect_bytes: Arena::new(capacity.effect_bytes),
            effect_events: Arena::new(capacity.effect_events),
            modifiers: CaptureRows::new(capacity.rows[10]),
            tactics: CaptureRows::new(capacity.rows[11]),
            sharing: CaptureRows::new(capacity.rows[12]),
            sharing_members: CaptureRows::new(capacity.rows[13]),
            balances: CaptureRows::new(capacity.rows[14]),
            breakdowns: Vec::new(),
            breakdown_outcome: Outcome::Rows { len: 0 },
            balance_members: CaptureRows::new(capacity.rows[15]),
            ranges: CaptureRows::new(capacity.rows[16]),
            research_inputs: CaptureRows::new(capacity.rows[18]),
            research_bonuses: CaptureRows::new(capacity.rows[19]),
            research_modifiers: CaptureRows::new(capacity.rows[20]),
            overrides: CaptureRows::new(capacity.rows[17]),
            focus_text: Arena::new(capacity.focus_text),
            definition_text: Arena::new(capacity.definition_text),
            party_text: Arena::new(capacity.party_text),
            idea_text: Arena::new(capacity.idea_text),
            timed_idea_text: Arena::new(capacity.timed_idea_text),
            random_item_text: Arena::new(capacity.random_item_text),
            decision_text: Arena::new(capacity.decision_text),
            modifier_text: Arena::new(capacity.modifier_text),
            modifier_values: Arena::new(capacity.modifier_values),
            tactic_text: Arena::new(capacity.tactic_text),
            balance_text: Arena::new(capacity.balance_text),
            party_countries: Arena::new(capacity.party_countries),
            idea_countries: Arena::new(capacity.idea_countries),
            decision_countries: Arena::new(capacity.decision_countries),
            dictionary: owned_vec(default_vec(capacity.dictionary)),
            dictionary_required: 0,
            focus_raw_mode: false,
            research_loaned: false,
            research_selection: SliceSelection::new(capacity.rows[18] != 0),
            technology_selection: SliceSelection::new(capacity.rows[2] != 0),
        }
    }

    /// Run the unchanged complete family capture on its owning thread. Focus
    /// delegation must already describe this callback before dispatch.
    pub(crate) fn capture_owned(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        technology_fingerprints: &mut SliceFingerprints,
    ) -> CaptureReport {
        let ticks = self.capture_sliced(
            memory,
            context.base,
            context.game_state,
            context.hour,
            context.force_full,
            technology_fingerprints,
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
        self.technology_selection
            .reset(context.hour, context.force_full);
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
            sharing = 12,
            sharing_members = 13,
            balances = 14,
            balance_members = 15,
            ranges = 16,
            overrides = 17,
            research_bonuses = 19,
            research_modifiers = 20,
            research_inputs = 18,
            party_leaders = 21,
            timed_ideas = 23,
            random_items = 24,
        );
        self.breakdown_outcome = Outcome::Unavailable { reason };
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
        self.timed_ideas.reset();
        self.random_items.reset();
        self.counts.reset();
        self.decisions.reset();
        self.effect_tokens.reset();
        self.effect_frames.reset();
        self.effect_names.reset();
        self.effect_bytes.reset();
        self.effect_events.reset();
        self.modifiers.reset();
        self.tactics.reset();
        self.sharing.reset();
        self.sharing_members.reset();
        self.balances.reset();
        self.breakdowns.clear();
        self.breakdown_outcome = Outcome::Rows { len: 0 };
        self.balance_members.reset();
        self.ranges.reset();
        self.overrides.reset();
        self.research_bonuses.reset();
        self.research_modifiers.reset();
        self.research_inputs.reset();
        self.party_leaders.reset();
        self.focus_text.reset();
        self.definition_text.reset();
        self.party_text.reset();
        self.idea_text.reset();
        self.timed_idea_text.reset();
        self.random_item_text.reset();
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
            + self.timed_ideas.allocated_bytes()
            + self.random_items.allocated_bytes()
            + self.counts.allocated_bytes()
            + self.decisions.allocated_bytes()
            + self.effect_tokens.allocated_bytes()
            + self.effect_frames.allocated_bytes()
            + self.effect_names.allocated_bytes()
            + self.effect_bytes.allocated_bytes()
            + self.effect_events.allocated_bytes()
            + self.modifiers.allocated_bytes()
            + self.tactics.allocated_bytes()
            + self.sharing.allocated_bytes()
            + self.sharing_members.allocated_bytes()
            + self.balances.allocated_bytes()
            + self.breakdowns.capacity() * size_of::<rows::PowerBalanceModifierBreakdownRow>()
            + self
                .breakdowns
                .iter()
                .map(|r| {
                    r.sources.capacity() * size_of::<Vec<u8>>()
                        + r.sources.iter().map(Vec::capacity).sum::<usize>()
                        + [&r.daily, &r.weekly]
                            .into_iter()
                            .map(|nodes| {
                                nodes.capacity() * size_of::<engine::modifier_breakdown::Node>()
                                    + nodes.iter().map(|n| n.name.capacity()).sum::<usize>()
                            })
                            .sum::<usize>()
                })
                .sum::<usize>()
            + self.balance_members.allocated_bytes()
            + self.ranges.allocated_bytes()
            + self.overrides.allocated_bytes()
            + self.research_bonuses.allocated_bytes()
            + self
                .research_bonuses
                .rows
                .iter()
                .map(|row| row.value.name.allocated_bytes())
                .sum::<usize>()
            + self.research_modifiers.allocated_bytes()
            + self
                .research_modifiers
                .rows
                .iter()
                .map(|row| row.name.allocated_bytes())
                .sum::<usize>()
            + self.research_inputs.allocated_bytes()
            + self
                .research_inputs
                .rows
                .iter()
                .map(|row| row.token.allocated_bytes())
                .sum::<usize>()
            + self.party_leaders.allocated_bytes()
            + self.focus_text.allocated_bytes()
            + self.definition_text.allocated_bytes()
            + self.party_text.allocated_bytes()
            + self.idea_text.allocated_bytes()
            + self.timed_idea_text.allocated_bytes()
            + self.random_item_text.allocated_bytes()
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
        // Worker-grown storage is demand for every recycled frame, including
        // an empty or failed later capture. Pooled headroom never sizes hooks.
        capacity.rows[6] = capacity.rows[6].max(self.parties.rows.len());
        capacity.rows[10] = capacity.rows[10].max(self.modifiers.rows.len());
        capacity.party_text = capacity.party_text.max(self.party_text.first.len());
        capacity.modifier_text = capacity.modifier_text.max(self.modifier_text.first.len());
        capacity.modifier_values = capacity
            .modifier_values
            .max(self.modifier_values.first.len());
        capacity.party_countries = capacity
            .party_countries
            .max(self.party_countries.first.len());
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
        self.timed_ideas.grow(&mut capacity.rows[23]);
        self.random_items.grow(&mut capacity.rows[24]);
        self.counts.grow(&mut capacity.rows[8]);
        self.decisions.grow(&mut capacity.rows[9]);
        self.effect_tokens.grow(&mut capacity.effect_tokens);
        self.effect_frames.grow(&mut capacity.effect_frames);
        self.effect_names.grow(&mut capacity.effect_names);
        self.effect_bytes.grow(&mut capacity.effect_bytes);
        self.effect_events.grow(&mut capacity.effect_events);
        self.modifiers.grow(&mut capacity.rows[10]);
        self.tactics.grow(&mut capacity.rows[11]);
        self.sharing.grow(&mut capacity.rows[12]);
        self.sharing_members.grow(&mut capacity.rows[13]);
        self.balances.grow(&mut capacity.rows[14]);
        self.balance_members.grow(&mut capacity.rows[15]);
        self.ranges.grow(&mut capacity.rows[16]);
        self.research_inputs.grow(&mut capacity.rows[18]);
        self.research_bonuses.grow(&mut capacity.rows[19]);
        self.research_modifiers.grow(&mut capacity.rows[20]);
        self.party_leaders.grow(&mut capacity.rows[21]);
        self.overrides.grow(&mut capacity.rows[17]);
        capacity.party_countries = capacity.party_countries.max(capacity.rows[6]);
        capacity.idea_countries = capacity.rows[7];
        capacity.decision_countries = capacity.rows[9];
        self.focus_text.grow(&mut capacity.focus_text);
        self.definition_text.grow(&mut capacity.definition_text);
        self.party_text.grow(&mut capacity.party_text);
        self.idea_text.grow(&mut capacity.idea_text);
        self.timed_idea_text.grow(&mut capacity.timed_idea_text);
        self.random_item_text.grow(&mut capacity.random_item_text);
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
            self.sharing.outcome,
            self.sharing_members.outcome,
            self.balances.outcome,
            self.balance_members.outcome,
            self.ranges.outcome,
            self.overrides.outcome,
            self.research_inputs.outcome,
            self.research_bonuses.outcome,
            self.research_modifiers.outcome,
            self.party_leaders.outcome,
            self.breakdown_outcome,
            self.timed_ideas.outcome,
            self.random_items.outcome,
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
            effect_tokens: &mut self.effect_tokens,
            effect_frames: &mut self.effect_frames,
            effect_names: &mut self.effect_names,
            effect_bytes: &mut self.effect_bytes,
            effect_events: &mut self.effect_events,
            modifiers: &mut self.modifiers.rows,
            tactics: &mut self.tactics.rows,
            sharing: &mut self.sharing.rows,
            sharing_members: &mut self.sharing_members.rows,
            balances: &mut self.balances.rows,
            balance_members: &mut self.balance_members.rows,
            ranges: &mut self.ranges.rows,
            overrides: &mut self.overrides.rows,
            party_leaders: &mut self.party_leaders.rows,
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
            technology_selection: &mut self.technology_selection,
        }
    }

    /// A capture whose technology store is new: every country is first seen.
    #[cfg(test)]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        let mut fingerprints = SliceFingerprints::new();
        self.capture_sliced(
            memory,
            base,
            game_state,
            hour,
            force_full,
            &mut fingerprints,
        )
    }

    /// Read the family. ResearchTechnologies reads this hour's country slice,
    /// every country on a full hour, and every country `technology_fingerprints`
    /// reports as new or unpublished; TechnologyDefinitions holds the templates
    /// those rosters reach.
    pub(crate) fn capture_sliced(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        technology_fingerprints: &mut SliceFingerprints,
    ) -> [u64; N] {
        self.reset();
        self.technology_selection.reset(hour, force_full);
        let mut walk: [bool; N] =
            std::array::from_fn(|i| replay::cadence::should_walk(TABLES[i], hour, force_full));
        // The World acquisition job stages scheduled Focus via `copy_focus_refresh`;
        // the decode worker fills the typed rows before the ready handoff, so this capture
        // skips the delegated focus tables. Costs come from the owned refresh.
        if self.focus_raw_mode {
            walk[0] = false;
            walk[1] = false;
        }
        // The research tables belong to `ResearchPart`, walked by its own job.
        for index in RESEARCH {
            walk[index] = false;
        }
        let first = acquire(
            memory,
            base,
            game_state,
            self.buffers(),
            &walk,
            technology_fingerprints,
        );
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
        self.sharing.outcome = first.outcomes[12];
        self.sharing_members.outcome = first.outcomes[13];
        self.balances.outcome = first.outcomes[14];
        self.balance_members.outcome = first.outcomes[15];
        self.ranges.outcome = first.outcomes[16];
        self.overrides.outcome = first.outcomes[17];
        self.party_leaders.outcome = first.outcomes[21];
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
        let mut ticks = first.ticks;
        let started = qpc();
        let _reads = read_accounting::table(TABLES[22]);
        self.capture_breakdowns(memory, base, game_state);
        ticks[22] = elapsed(started);
        for index in [23, 24] {
            if !walk[index] {
                let outcome = Outcome::Unavailable {
                    reason: replay::cadence::SKIPPED_REASON,
                };
                if index == 23 {
                    self.timed_ideas.outcome = outcome;
                } else {
                    self.random_items.outcome = outcome;
                }
                continue;
            }
            let _reads = read_accounting::table(TABLES[index]);
            let started = qpc();
            if index == 23 {
                let result = cpolitical_status::walk_timed_ideas_growing(
                    memory,
                    base,
                    game_state,
                    &mut self.timed_ideas.rows,
                    &mut self.timed_idea_text.first,
                );
                self.timed_ideas.outcome = result.outcome;
                self.timed_idea_text.len = result.bytes_len;
                self.timed_idea_text.required = result.required_bytes;
            } else {
                let result = cdecision_status::walk_random_items_growing(
                    memory,
                    base,
                    game_state,
                    &mut self.random_items.rows,
                    &mut self.random_item_text.first,
                );
                self.random_items.outcome = result.outcome;
                self.random_item_text.len = result.bytes_len;
                self.random_item_text.required = result.required_bytes;
            }
            ticks[index] = elapsed(started);
        }
        ticks
    }
    fn capture_breakdowns(&mut self, memory: &impl Memory, base: usize, state: usize) {
        let result = (|| {
            if !matches!(self.balances.outcome, Outcome::Rows { .. }) {
                return Err("power balance modifier breakdown record census incomplete");
            }
            let manager = state
                .checked_add(engine::cgame_state::POWER_BALANCE_MANAGER)
                .and_then(|p| engine::read::<u64>(memory, p))
                .filter(|&p| p != 0)
                .ok_or("power balance modifier breakdown manager unreadable or null")?
                as usize;
            let roster = manager
                .checked_add(power_balance_manager::RECORDS)
                .and_then(|p| {
                    engine::cpdx_array::Array::read(memory, p, power_balance_record::STRIDE)
                })
                .ok_or("power balance modifier breakdown roster unreadable or invalid")?;
            if roster.count as usize != self.balances.outcome.len() {
                return Err("power balance modifier breakdown roster count mismatch");
            }
            reserve_push_batch(&mut self.breakdowns, roster.count as usize)
                .map_err(crate::allocation::io)
                .map_err(|_| "collector allocation failed")?;
            for record in 0..roster.count {
                let address = roster
                    .element(record, power_balance_record::STRIDE)
                    .ok_or("power balance modifier breakdown record address overflow")?;
                let tree = engine::modifier_breakdown::balance(memory, base, address)?;
                self.breakdowns
                    .push(rows::PowerBalanceModifierBreakdownRow {
                        record,
                        sources: tree.sources,
                        daily: tree.daily,
                        weekly: tree.weekly,
                    });
            }
            Ok(())
        })();
        self.breakdown_outcome = match result {
            Ok(()) => Outcome::Rows {
                len: self.breakdowns.len(),
            },
            Err(reason) => Outcome::Unavailable { reason },
        };
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
    focus: &'a mut Vec<cnational_focus_progress::Row>,
    completed: &'a mut Vec<cnational_focus_progress::CompletedRow>,
    technologies: &'a mut Vec<ctechnology::Row>,
    technology_pointers: &'a mut Vec<usize>,
    slots: &'a mut Vec<cresearch_slot::Row>,
    definitions: &'a mut Vec<ctechnology_template::Row>,
    politics: &'a mut Vec<CountryRow<cpolitical_status::Row>>,
    parties: &'a mut Vec<cpolitical_party::Row>,
    ideas: &'a mut Vec<cpolitical_status::IdeaRow>,
    counts: &'a mut Vec<CountryRow<cdecision_status::Counts>>,
    decisions: &'a mut Vec<cdecision_status::Row>,
    effect_tokens: &'a mut Arena<engine::border_effect_demand::Token>,
    effect_frames: &'a mut Arena<engine::border_effect_demand::Frame>,
    effect_names: &'a mut Arena<engine::border_effect_demand::Name>,
    effect_bytes: &'a mut Arena<u8>,
    effect_events: &'a mut Arena<u32>,
    modifiers: &'a mut Vec<cdynamic_modifier_container::Row>,
    tactics: &'a mut Vec<TacticRow>,
    sharing: &'a mut Vec<ctechnology_sharing_group::Row>,
    sharing_members: &'a mut Vec<ctechnology_sharing_group::MemberRow>,
    balances: &'a mut Vec<power_balance_record::Row>,
    balance_members: &'a mut Vec<power_balance_record::MemberRow>,
    ranges: &'a mut Vec<power_balance_range::Row>,
    overrides: &'a mut Vec<power_balance_gfx_override::Row>,
    party_leaders: &'a mut Vec<cpolitical_party::LeaderRow>,
    focus_text: &'a mut Vec<u8>,
    definition_text: &'a mut Vec<u8>,
    party_text: &'a mut Vec<u8>,
    idea_text: &'a mut Vec<u8>,
    decision_text: &'a mut Vec<u8>,
    modifier_text: &'a mut Vec<u8>,
    modifier_values: &'a mut Vec<i64>,
    tactic_text: &'a mut Vec<u8>,
    balance_text: &'a mut Vec<u8>,
    party_countries: &'a mut Vec<u32>,
    idea_countries: &'a mut Vec<u32>,
    decision_countries: &'a mut Vec<u32>,
    dictionary: &'a mut Vec<ctechnology_template::Entry>,
    technology_selection: &'a mut SliceSelection,
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
    technology_fingerprints: &mut SliceFingerprints,
) -> Observation {
    let mut o = Observation::new();
    for (outcome, &active) in o.outcomes.iter_mut().zip(enabled) {
        if !active {
            *outcome = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
        }
    }
    let demand_failure = if enabled[9] {
        let started = qpc();
        let _reads = read_accounting::table(TABLES[9]);
        let demand = engine::border_effect_demand::walk_growing(
            memory,
            base,
            game_state,
            &mut b.effect_tokens.first,
            &mut b.effect_frames.first,
            &mut b.effect_events.first,
        );
        b.effect_tokens.len = demand.len;
        b.effect_tokens.required = demand.required_rows;
        b.effect_frames.required = demand.required_stack;
        b.effect_events.required = demand.required_events;
        b.effect_names.required = demand.required_rows;
        let failure = match demand.outcome {
            Outcome::Rows { .. } => {
                let tokens = &b.effect_tokens.first[..demand.len];
                match tokens
                    .iter()
                    .try_fold(0_usize, |total, token| total.checked_add(token.len))
                {
                    Some(required) => {
                        b.effect_bytes.required = required;
                        if !grow_vec(&mut b.effect_names.first, demand.len)
                            || !grow_vec(&mut b.effect_bytes.first, required)
                        {
                            Some("border demand text allocation failed")
                        } else {
                            match engine::border_effect_demand::copy_names(
                                memory,
                                tokens,
                                &mut b.effect_names.first,
                                &mut b.effect_bytes.first,
                            ) {
                                Ok(len) => {
                                    b.effect_names.len = demand.len;
                                    b.effect_bytes.len = len;
                                    None
                                }
                                Err(reason) => Some(reason),
                            }
                        }
                    }
                    None => Some("border demand byte count overflow"),
                }
            }
            Outcome::Unavailable { reason } => Some(reason),
            Outcome::Truncated { .. } => Some("border effect demand token capacity"),
        };
        o.ticks[9] += elapsed(started);
        failure
    } else {
        None
    };
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
    // Every live country was visited without a census error.
    let mut census = true;
    if enabled[..12].iter().any(|&value| value) || enabled[21] {
        let started = qpc();
        let countries = read_accounting::with_table(TABLES[5], || {
            ccountry::Reader::new(memory, base, game_state)
        });
        o.ticks[5] += elapsed(started);
        match countries {
            Err(reason) => {
                census = false;
                for (outcome, _) in o.outcomes[..12]
                    .iter_mut()
                    .zip(&enabled[..12])
                    .filter(|(_, active)| **active)
                {
                    *outcome = Outcome::Unavailable { reason };
                }
                if enabled[21] {
                    o.outcomes[21] = Outcome::Unavailable { reason };
                }
            }
            Ok(countries) => {
                for index in [0, 5, 8, 11] {
                    if !enabled[index] {
                        continue;
                    }
                    let required = countries.count() as usize;
                    let ready = match index {
                        0 => grow_vec(b.focus, required),
                        5 => grow_vec(b.politics, required),
                        8 => grow_vec(b.counts, required),
                        11 => grow_vec(b.tactics, required),
                        _ => unreachable!(),
                    };
                    if !ready {
                        o.outcomes[index] = Outcome::Unavailable {
                            reason: "governance country row allocation failed",
                        };
                    }
                }
                for slot in 1..countries.count() {
                    let started = qpc();
                    let country = read_accounting::with_table(TABLES[5], || countries.at(slot));
                    o.ticks[5] += elapsed(started);
                    let country = match country {
                        Ok(Some(country)) => country,
                        Ok(None) => continue,
                        Err(reason) => {
                            census = false;
                            for (outcome, _) in o.outcomes[..12]
                                .iter_mut()
                                .zip(&enabled[..12])
                                .filter(|(_, active)| **active)
                            {
                                *outcome = Outcome::Unavailable { reason };
                            }
                            if enabled[21] {
                                o.outcomes[21] = Outcome::Unavailable { reason };
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
                        let result = cnational_focus_progress::walk_buffer(
                            memory,
                            base,
                            country.pointer,
                            country.id,
                            engine::walk::Buffer::Growing(b.focus).suffix(first),
                            engine::walk::Buffer::Growing(b.completed).suffix(completed),
                            engine::walk::Buffer::Growing(b.focus_text).suffix(text),
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
                    // A constant fingerprint: only new owners and unpublished
                    // reads join the hour's slice.
                    if enabled[2] {
                        b.technology_selection
                            .scan(technology_fingerprints, country.id, Some(0));
                    }
                    if enabled[2] && b.technology_selection.contains(country.id) {
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
                        let result = ctechnology_status::walk_slots_buffer(
                            memory,
                            base,
                            country.pointer,
                            country.id,
                            engine::walk::Buffer::Growing(b.slots).suffix(first),
                        );
                        append(&mut o.outcomes[3], result);
                        o.ticks[3] += elapsed(started);
                    }
                    if enabled[5] || enabled[21] {
                        let _reads = read_accounting::table(TABLES[5]);
                        let started = qpc();
                        match cpolitical_status::Reader::new(memory, base, country.pointer) {
                            Err(reason) => {
                                for (index, &active) in enabled.iter().enumerate().take(8).skip(5) {
                                    if active {
                                        o.outcomes[index] = Outcome::Unavailable { reason };
                                    }
                                }
                                if enabled[21] {
                                    o.outcomes[21] = Outcome::Unavailable { reason };
                                }
                            }
                            Ok(reader) => {
                                // PartyLeaders shares Politics' hourly walk and
                                // authenticated status, charged to its own table.
                                if enabled[21] {
                                    let _reads = read_accounting::table(TABLES[21]);
                                    let leaders = qpc();
                                    let first = o.outcomes[21].len();
                                    let result = reader.party_leaders_buffer(
                                        country.id,
                                        engine::walk::Buffer::Growing(b.party_leaders)
                                            .suffix(first),
                                    );
                                    append(&mut o.outcomes[21], result);
                                    o.ticks[21] += elapsed(leaders);
                                }
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
                                let mut result =
                                    reader.parties_growing(b.parties, first, b.party_text, text);
                                if !grow_vec(b.party_countries, first + result.outcome.len()) {
                                    result.outcome = Outcome::Unavailable {
                                        reason: "political party country arena allocation failed",
                                    };
                                    result.bytes_len = 0;
                                }
                                for row in &mut b.parties[first..first + result.outcome.len()] {
                                    row.token_start += text;
                                    row.name_start += text;
                                    row.long_name_start += text;
                                }
                                b.party_countries[first..first + result.outcome.len()]
                                    .fill(country.id);
                                append(&mut o.outcomes[6], result.outcome);
                                o.arena(2, result.bytes_len, result.required_bytes);
                                if enabled[7] {
                                    let first = o.outcomes[7].len();
                                    let text = o.lens[3];
                                    let mut result = reader.ideas_buffer(
                                        engine::walk::Buffer::Growing(b.ideas).suffix(first),
                                        engine::walk::Buffer::Growing(b.idea_text).suffix(text),
                                    );
                                    if engine::walk::Buffer::Growing(b.idea_countries)
                                        .ensure(first + result.outcome.len())
                                        .is_err()
                                    {
                                        result.outcome = Outcome::Unavailable {
                                            reason: "active idea country arena allocation failed",
                                        };
                                        result.bytes_len = 0;
                                    }
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
                        let result = match demand_failure {
                            None => cdecision_status::walk_with_description_demand_buffer(
                                memory,
                                base,
                                game_state,
                                country.pointer,
                                engine::walk::Buffer::Growing(b.decisions).suffix(first),
                                engine::walk::Buffer::Growing(b.decision_text).suffix(text),
                                engine::border_effect_demand::Demand {
                                    names: &b.effect_names.first[..b.effect_names.len],
                                    bytes: &b.effect_bytes.first[..b.effect_bytes.len],
                                },
                            ),
                            Some(_) => cdecision_status::walk_buffer(
                                memory,
                                base,
                                country.pointer,
                                engine::walk::Buffer::Growing(b.decisions).suffix(first),
                                engine::walk::Buffer::Growing(b.decision_text).suffix(text),
                            ),
                        };
                        let owners_ready =
                            grow_vec(b.decision_countries, first + result.outcome.len());
                        if owners_ready {
                            for row in &mut b.decisions[first..first + result.outcome.len()] {
                                row.token_start += text;
                            }
                            b.decision_countries[first..first + result.outcome.len()]
                                .fill(country.id);
                        }
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
                        append(
                            &mut o.outcomes[9],
                            if owners_ready {
                                result.outcome
                            } else {
                                Outcome::Unavailable {
                                    reason: "decision owner row allocation failed",
                                }
                            },
                        );
                        o.arena(4, result.bytes_len, result.required_bytes);
                        o.ticks[8] += elapsed(started);
                    }
                    if enabled[10] {
                        let _reads = read_accounting::table(TABLES[10]);
                        let started = qpc();
                        let first = o.outcomes[10].len();
                        let text = o.lens[5];
                        let values = o.lens[6];
                        let result = cdynamic_modifier_container::walk_growing(
                            memory,
                            base,
                            country.pointer,
                            country.id,
                            b.modifiers,
                            first,
                            b.modifier_text,
                            text,
                            b.modifier_values,
                            values,
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
                        let selected = ccombat_tactic::preferred_buffer(
                            memory,
                            base,
                            country.pointer,
                            country.id,
                            engine::walk::Buffer::Growing(b.tactic_text).suffix(text),
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
                        o.ticks[11] += elapsed(started);
                    }
                }
            }
        }
    }
    if let Some(reason) = demand_failure {
        o.outcomes[9] = Outcome::Unavailable { reason };
    }
    if enabled[2] {
        if census {
            technology_fingerprints.finish_scan();
            b.technology_selection
                .finish_census(technology_fingerprints);
        }
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
        let copied = if reserve(b.definitions, count).is_err() {
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
            // Templates are identified by their positive database index.
            b.definitions.sort_unstable_by_key(|row| row.database_index);
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
    if enabled[12] {
        let _reads = read_accounting::table(TABLES[12]);
        let started = qpc();
        let result = ctechnology_sharing_group::walk_growing(
            memory,
            base,
            game_state,
            b.sharing,
            b.sharing_members,
        );
        o.outcomes[12] = result.groups;
        o.outcomes[13] = result.members;
        o.ticks[12] += elapsed(started);
    }
    if enabled[14] {
        let _reads = read_accounting::table(TABLES[14]);
        let started = qpc();
        let result = power_balance_manager::walk_buffer(
            memory,
            base,
            game_state,
            power_balance_manager::GrowingBuffers {
                records: engine::walk::Buffer::Growing(b.balances),
                members: engine::walk::Buffer::Growing(b.balance_members),
                ranges: engine::walk::Buffer::Growing(b.ranges),
                overrides: engine::walk::Buffer::Growing(b.overrides),
                text: engine::walk::Buffer::Growing(b.balance_text),
            },
        );
        o.outcomes[14] = result.records;
        o.outcomes[15] = result.members;
        o.outcomes[16] = result.ranges;
        o.outcomes[17] = result.overrides;
        o.lens[8] = result.text_len;
        o.required[8] = result.required_text;
        o.ticks[14] += elapsed(started);
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

mod focus;

#[cfg(test)]
use focus::copy_focus_pass;
pub(crate) use focus::{FocusCopyContext, FocusCopyReport, FocusJob, copy_focus_refresh};

fn valid<T>(arena: &[T], start: usize, len: usize) -> bool {
    start.checked_add(len).is_some_and(|end| end <= arena.len())
}
fn copy_span<T: Copy>(arena: &[T], start: usize, len: usize) -> Vec<T> {
    owned_vec(copy_vec(&arena[start..start + len]))
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

    pub(crate) fn table_inputs(&self) -> [TableInput; N] {
        self.tables().map(TableInput::Sampled)
    }

    fn breakdowns_table(&self) -> TableView {
        let clone_failed = std::cell::Cell::new(false);
        let view = crate::spooling::table::<tables::PowerBalanceModifierBreakdowns, _>(
            self.breakdown_outcome,
            &self.breakdowns,
            |row| {
                if clone_failed.get() {
                    return Default::default();
                }
                match row.try_clone() {
                    Ok(row) => row,
                    Err(error) => {
                        crate::allocation::observe(&error);
                        clone_failed.set(true);
                        Default::default()
                    }
                }
            },
        );
        if clone_failed.get() {
            crate::spooling::unavailable("power balance modifier breakdown copy failed")
        } else {
            view
        }
    }

    fn technologies_table(&self) -> TableView {
        view::<tables::ResearchTechnologies, _>(
            &self.technologies,
            |_| true,
            |r| rows::ResearchTechnologyRow {
                country: r.country,
                database_index: r.database_index,
                applied_bonus_count: r.applied_bonus_count,
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

    fn convert_tables<T>(
        &self,
        wrap: impl Fn(TableView) -> T,
        technologies: T,
        definitions: T,
    ) -> [T; N] {
        let focus = &self.focus_text.first[..self.focus_text.len];
        let parties = &self.party_text.first[..self.party_text.len];
        let ideas = &self.idea_text.first[..self.idea_text.len];
        let timed_ideas = &self.timed_idea_text.first[..self.timed_idea_text.len];
        let random_items = &self.random_item_text.first[..self.random_item_text.len];
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
                    database_index: rows::Recorded::Value(r.database_index),
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
                    original_capital: r.row.original_capital,
                },
            )),
            wrap(country_view::<tables::PoliticalParties, _>(
                &self.parties,
                &self.party_countries.first[..self.party_countries.len],
                |r| {
                    valid(parties, r.token_start, r.token_len)
                        && valid(parties, r.name_start, r.name_len)
                        && valid(parties, r.long_name_start, r.long_name_len)
                },
                |country, r| rows::PoliticalPartyRow {
                    country,
                    id: r.id,
                    popularity: r.popularity,
                    is_ruling: r.is_ruling,
                    token: copy_span(parties, r.token_start, r.token_len),
                    tag: r.tag,
                    name_default: r.name_default,
                    name: copy_span(parties, r.name_start, r.name_len),
                    long_name: copy_span(parties, r.long_name_start, r.long_name_len),
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
                    description_seed: replay::rows::Recorded::Value(r.description_seed),
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
            wrap(view::<tables::ResearchInputs, _>(
                &self.research_inputs,
                |_| true,
                |r| rows::ResearchInputRow {
                    country: r.slot.country,
                    index: r.slot.index,
                    technology_handle: r.slot.technology_handle,
                    technology_present: r.slot.technology_present,
                    database_index: rows::Recorded::Value(r.template_index),
                    token: owned_vec(copy_vec(r.token.as_bytes())),
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
                },
            )),
            wrap(view::<tables::ResearchAppliedBonuses, _>(
                &self.research_bonuses,
                |_| true,
                |r| rows::ResearchAppliedBonusRow {
                    country: r.country,
                    index: r.index,
                    ordinal: r.ordinal,
                    technology_handle: r.technology_handle,
                    id: r.value.id,
                    resolved: r.resolved,
                    name: owned_vec(copy_vec(r.value.name.as_bytes())),
                    ahead_reduction_raw: r.value.ahead_reduction_raw,
                    speed_bonus_raw: r.value.speed_bonus_raw,
                    cost_reduction_raw: r.cost_reduction_raw,
                },
            )),
            wrap(view::<tables::ResearchModifiers, _>(
                &self.research_modifiers,
                |_| true,
                |r| rows::ResearchModifierRow {
                    country: r.country,
                    index: r.index,
                    ordinal: r.ordinal,
                    technology_handle: r.technology_handle,
                    kind: match r.kind {
                        engine::research_modifiers::Kind::Technology => {
                            rows::ResearchModifierRow::TECHNOLOGY
                        }
                        engine::research_modifiers::Kind::CountrySpeed => {
                            rows::ResearchModifierRow::COUNTRY_SPEED
                        }
                        engine::research_modifiers::Kind::CategorySpeed => {
                            rows::ResearchModifierRow::CATEGORY_SPEED
                        }
                        engine::research_modifiers::Kind::DesignTeam => {
                            rows::ResearchModifierRow::DESIGN_TEAM
                        }
                        engine::research_modifiers::Kind::Experience => {
                            rows::ResearchModifierRow::EXPERIENCE
                        }
                        engine::research_modifiers::Kind::Sharing => {
                            rows::ResearchModifierRow::SHARING
                        }
                        engine::research_modifiers::Kind::License => {
                            rows::ResearchModifierRow::LICENSE
                        }
                        engine::research_modifiers::Kind::Scientist => {
                            rows::ResearchModifierRow::SCIENTIST
                        }
                        engine::research_modifiers::Kind::Year => rows::ResearchModifierRow::YEAR,
                        engine::research_modifiers::Kind::AheadFactor => {
                            rows::ResearchModifierRow::AHEAD_FACTOR
                        }
                        engine::research_modifiers::Kind::MinimumSpeed => {
                            rows::ResearchModifierRow::MINIMUM_SPEED
                        }
                        engine::research_modifiers::Kind::CountrySource => {
                            rows::ResearchModifierRow::COUNTRY_SOURCE
                        }
                        engine::research_modifiers::Kind::IdeaSource => {
                            rows::ResearchModifierRow::IDEA_SOURCE
                        }
                        engine::research_modifiers::Kind::ScriptSource => {
                            rows::ResearchModifierRow::SCRIPT_SOURCE
                        }
                        engine::research_modifiers::Kind::ScientistSource => {
                            rows::ResearchModifierRow::SCIENTIST_SOURCE
                        }
                        engine::research_modifiers::Kind::SharingMultiplier => {
                            rows::ResearchModifierRow::SHARING_MULTIPLIER
                        }
                        engine::research_modifiers::Kind::SharingMaximum => {
                            rows::ResearchModifierRow::SHARING_MAXIMUM
                        }
                        engine::research_modifiers::Kind::ModifierMetadata => {
                            rows::ResearchModifierRow::MODIFIER_METADATA
                        }
                        engine::research_modifiers::Kind::ScientistCategory => {
                            rows::ResearchModifierRow::SCIENTIST_CATEGORY
                        }
                        engine::research_modifiers::Kind::SharingMember => {
                            rows::ResearchModifierRow::SHARING_MEMBER
                        }
                        engine::research_modifiers::Kind::SharingPredecessor => {
                            rows::ResearchModifierRow::SHARING_PREDECESSOR
                        }
                        engine::research_modifiers::Kind::SharingFilter => {
                            rows::ResearchModifierRow::SHARING_FILTER
                        }
                        engine::research_modifiers::Kind::SharingCategory => {
                            rows::ResearchModifierRow::SHARING_CATEGORY
                        }
                        engine::research_modifiers::Kind::SharingDoctrine => {
                            rows::ResearchModifierRow::SHARING_DOCTRINE
                        }
                        engine::research_modifiers::Kind::SharingMissingMember => {
                            rows::ResearchModifierRow::SHARING_MISSING_MEMBER
                        }
                    },
                    raw: r.raw,
                    auxiliary: r.auxiliary,
                    count: r.count,
                    source_id: r.source_id,
                    name: owned_vec(copy_vec(r.name.as_bytes())),
                },
            )),
            wrap(view::<tables::PartyLeaders, _>(
                &self.party_leaders,
                |_| true,
                |r| rows::PartyLeaderRow {
                    country: r.country,
                    party: r.party,
                    ordinal: r.ordinal,
                    character: r.character,
                    leader_id: r.leader_id,
                    role: r.role,
                },
            )),
            wrap(self.breakdowns_table()),
            wrap(view::<tables::TimedIdeas, _>(
                &self.timed_ideas,
                |r| valid(timed_ideas, r.token_start, r.token_len),
                |r| rows::TimedIdeaRow {
                    country: r.country,
                    ordinal: r.ordinal,
                    token: copy_span(timed_ideas, r.token_start, r.token_len),
                    days: r.days,
                },
            )),
            wrap(view::<tables::DecisionRandomItems, _>(
                &self.random_items,
                |r| valid(random_items, r.token_start, r.token_len),
                |r| rows::DecisionRandomItemRow {
                    country: r.country,
                    ordinal: r.ordinal,
                    token: copy_span(random_items, r.token_start, r.token_len),
                    count: r.count,
                    target: r.target as u32,
                },
            )),
        ]
    }
}

/// Build before arming the allocator; conversion follows the populated spool path.
#[cfg(test)]
pub(crate) fn allocation_failure_fixture() -> impl FnOnce() -> bool {
    let mut captured = Captured::vacant();
    captured.breakdowns = vec![rows::PowerBalanceModifierBreakdownRow {
        record: 7,
        sources: vec![b"first source".to_vec(), b"second source".to_vec()],
        daily: vec![
            rows::ModifierBreakdownNode {
                children: 1,
                value: 11,
                name: b"daily root".to_vec(),
            },
            rows::ModifierBreakdownNode {
                children: 0,
                value: 12,
                name: b"daily leaf".to_vec(),
            },
        ],
        weekly: vec![rows::ModifierBreakdownNode {
            children: 0,
            value: 21,
            name: b"weekly root".to_vec(),
        }],
    }];
    captured.breakdown_outcome = Outcome::Rows { len: 1 };
    move || {
        let view = captured.breakdowns_table();
        if crate::allocation::failed() {
            return matches!(view, TableView::Unavailable { .. });
        }
        let TableView::Complete(TableRows::PowerBalanceModifierBreakdowns(converted)) = view else {
            return false;
        };
        let [row] = converted.as_slice() else {
            return false;
        };
        row.record == 7
            && row.sources.len() == 2
            && row.sources[0] == b"first source"
            && row.sources[1] == b"second source"
            && row.daily.len() == 2
            && row.daily[0].children == 1
            && row.daily[0].value == 11
            && row.daily[0].name == b"daily root"
            && row.daily[1].children == 0
            && row.daily[1].value == 12
            && row.daily[1].name == b"daily leaf"
            && row.weekly.len() == 1
            && row.weekly[0].children == 0
            && row.weekly[0].value == 21
            && row.weekly[0].name == b"weekly root"
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod border_effects {
    mod tests;
}

#[cfg(test)]
#[path = "governance_research_tests.rs"]
mod research_tests;
