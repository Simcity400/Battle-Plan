//! Intel acquisition buffers. Allocation, growth and replay conversion are off-thread.

use crate::{
    raw::RawLog,
    read_accounting,
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_vec},
};
use engine::{
    Memory, ccountry, ccountry_intel, ccountry_intel_network, ccountry_operation_manager,
    ccountry_operation_token_manager, ccryptology, ccurrent_autonomy_status, cdiplomacy_status,
    cflag_manager, cgame_state, cintelligence_agency, coperation_instance, coperative_leader,
    country_intel_row,
    cpdx_array::Array,
    crelation_status, cscript_flag, cstate,
    csubject_relation::{self, Name},
    cvariables, incoming_diplomatic_action_queue, operation_assigned_operative, read, read_slice,
    walk::Outcome,
};
use replay::{
    BranchId,
    frame::{BaselineToken, TableInput, TableView},
    rows::{self, TableId, TableRows, tables},
};
use std::{mem::size_of, sync::Arc};

pub const N: usize = 16;
pub const TABLES: [TableId; N] = [
    TableId::Capitulation,
    TableId::Subjects,
    TableId::Autonomy,
    TableId::IntelligenceAgencies,
    TableId::OperativeMemberships,
    TableId::Cryptology,
    TableId::IntelNetworks,
    TableId::ScriptFlags,
    TableId::ScriptVariables,
    TableId::Operations,
    TableId::OperationAssignments,
    TableId::OperationTokens,
    TableId::IncomingDiplomaticActions,
    TableId::RelationOpinions,
    TableId::RelationTuples,
    TableId::CountryIntel,
];

#[derive(Clone, Copy, Debug)]
pub(crate) struct CaptureContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CaptureReport {
    pub ticks: [u64; N],
}

/// The operation walker supplies both tables in one census, and one relation
/// matrix pass supplies the cached opinions and the typed tuples. Each group's
/// entire cost is charged once to its anchor.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("operations and assigned slots", 9, &[9, 10]),
    ("relation opinions and typed tuples", 14, &[13, 14]),
];

pub const GLOBAL_SCOPE: u8 = 0;
pub const COUNTRY_SCOPE: u8 = 1;
pub const STATE_SCOPE: u8 = 2;

/// Spool-owned membership scratch, not another captured-frame lease. Bits are
/// cleared for each comparison, and contain no pointers, names or prior values.
#[derive(Default)]
pub(crate) struct ScriptVariableComparison {
    scopes: Vec<u64>,
    entries: Vec<u64>,
}

fn clear_seen(bits: &mut Vec<u64>, count: usize) {
    bits.resize(count.div_ceil(64), 0);
    bits.fill(0);
}

fn first_seen(bits: &mut [u64], index: usize) -> bool {
    let mask = 1_u64 << (index % 64);
    let word = &mut bits[index / 64];
    let first = *word & mask == 0;
    *word |= mask;
    first
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    /// Bytes of caller-owned scratch for the bulk intel-record copies. This is a
    /// fixed copy window, not a row capacity, so `grow` never raises it.
    pub intel_scratch: usize,
    pub subject_names: usize,
    pub autonomy_names: usize,
    pub variable_entries: usize,
    pub variable_names: usize,
    pub tokens: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: [
                512, 256, 256, 512, 4096, 4096, 16_384, 8192, 2048, 2048, 8192, 4096, 512, 16_384,
                4096, 65_536,
            ],
            intel_scratch: 16_384,
            subject_names: 16_384,
            autonomy_names: 32_768,
            variable_entries: 16_384,
            variable_names: 512 * 1024,
            tokens: 32_768,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ScopedFlag {
    scope_kind: u8,
    scope_id: u32,
    flag: cscript_flag::Row,
}

/// Spool-owned, fieldwise native snapshots. These independently sampled tables have no
/// staggered cache, so the memo owns its immutable replay result directly.
/// Neither capture leases nor live pointers survive conversion.
#[derive(Default)]
struct NativeShadow<R> {
    rows: Vec<R>,
    output: Option<Arc<TableRows>>,
    hits: u64,
    misses: u64,
}

impl<R: Clone + PartialEq> NativeShadow<R> {
    fn clear(&mut self) {
        self.rows.clear();
        self.output = None;
    }

    fn input(
        &mut self,
        id: TableId,
        current: &CaptureRows<R>,
        outcome: Outcome,
        convert: impl FnOnce() -> TableView,
    ) -> TableInput {
        if matches!(
            outcome,
            Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON
            }
        ) {
            return TableInput::Sampled(convert());
        }
        let source = match outcome {
            Outcome::Rows { len } => current.rows.get(..len),
            _ => None,
        };
        if let Some(source) = source
            && let Some(output) = &self.output
            && output.table() == id
            && self.rows == source
        {
            self.hits = self.hits.saturating_add(1);
            return TableInput::SharedComplete(Arc::clone(output));
        }
        self.misses = self.misses.saturating_add(1);
        let converted = convert();
        self.clear();
        if let Some(source) = source
            && let TableView::Complete(rows) = converted
        {
            debug_assert_eq!(rows.table(), id);
            let output = Arc::new(rows);
            self.rows.extend_from_slice(source);
            self.output = Some(Arc::clone(&output));
            return TableInput::SharedComplete(output);
        }
        TableInput::Sampled(converted)
    }

    fn statistics(&self) -> IntelShadowStats {
        let replay_capacity_bytes = match self.output.as_deref() {
            Some(TableRows::CountryIntel(rows)) => {
                rows.capacity() * size_of::<rows::CountryIntelRow>()
            }
            Some(TableRows::ScriptFlags(rows)) => {
                rows.capacity() * size_of::<rows::ScriptFlagRow>()
            }
            _ => 0,
        };
        IntelShadowStats {
            hits: self.hits,
            misses: self.misses,
            row_capacity_bytes: self.rows.capacity() * size_of::<R>(),
            replay_capacity_bytes,
        }
    }
}

#[derive(Default)]
pub(crate) struct IntelShadow {
    branch: Option<BranchId>,
    flags: NativeShadow<ScopedFlag>,
    country_intel: NativeShadow<country_intel_row::Row>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct IntelShadowStats {
    pub hits: u64,
    pub misses: u64,
    pub row_capacity_bytes: usize,
    pub replay_capacity_bytes: usize,
}

impl IntelShadow {
    pub(crate) fn statistics(&self) -> [IntelShadowStats; 2] {
        [self.flags.statistics(), self.country_intel.statistics()]
    }
}

pub(crate) struct IntelConversion<'a> {
    shadow: &'a mut IntelShadow,
}

impl<'a> IntelConversion<'a> {
    pub(crate) fn new(branch: BranchId, shadow: &'a mut IntelShadow) -> Self {
        if shadow.branch != Some(branch) {
            shadow.flags.clear();
            shadow.country_intel.clear();
            shadow.branch = Some(branch);
        }
        Self { shadow }
    }
}

pub struct Captured {
    relation_active: [bool; 2],
    capitulation: CaptureRows<cdiplomacy_status::CapitulationRow>,
    subjects: CaptureRows<csubject_relation::Row>,
    autonomy: CaptureRows<ccurrent_autonomy_status::Row>,
    agencies: CaptureRows<cintelligence_agency::Row>,
    operatives: CaptureRows<coperative_leader::Row>,
    cryptology: CaptureRows<ccryptology::Row>,
    networks: CaptureRows<ccountry_intel_network::Row>,
    flags: CaptureRows<ScopedFlag>,
    variables: CaptureRows<cvariables::ScopeRow>,
    operations: CaptureRows<coperation_instance::Row>,
    assignments: CaptureRows<operation_assigned_operative::Row>,
    operation_tokens: CaptureRows<ccountry_operation_token_manager::Row>,
    incoming_actions: CaptureRows<incoming_diplomatic_action_queue::Row>,
    relation_opinions: CaptureRows<crelation_status::OpinionRow>,
    relation_tuples: CaptureRows<crelation_status::TupleRow>,
    /// Decode-only scratch, rebuilt from the current frozen census. Entries
    /// are first country indices, never pointers retained from an earlier job.
    relation_country_index: Vec<usize>,
    country_intel: CaptureRows<country_intel_row::Row>,
    intel_scratch: Vec<u8>,
    subject_names: Arena<u8>,
    autonomy_names: Arena<u8>,
    variable_entries: Arena<cvariables::Row>,
    variable_names: Arena<u8>,
    tokens: Arena<u32>,
    flag_scratch: Vec<cscript_flag::Row>,
    /// Raw-copy split gate for the relations family. Set by the game
    /// thread on refresh hours with a valid raw payload; `capture` then skips
    /// the typed relations walks and the decode worker fills these rows before
    /// the ready handoff. Preserved across `reset` (set per refresh, cleared
    /// by the worker after decode).
    pub(crate) relations_raw_mode: bool,
    /// Raw-copy split gate for the hourly capitulation table (local index 0).
    /// Set by the game thread every hour with a valid raw payload; `capture`
    /// then skips the typed capitulation walk and the decode worker fills
    /// the rows before the ready handoff. Preserved across `reset` (set per
    /// hour, cleared by the worker after decode).
    pub(crate) capitulation_raw_mode: bool,
}

impl Captured {
    pub(crate) fn relation_activity(&self) -> [bool; 2] {
        self.relation_active
    }

    pub(crate) fn set_relation_activity(&mut self, active: [bool; 2]) {
        self.relation_active = active;
    }
    /// Empty ownership placeholder, with no rows or scratch allocated.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: [0; N],
            intel_scratch: 0,
            subject_names: 0,
            autonomy_names: 0,
            variable_entries: 0,
            variable_names: 0,
            tokens: 0,
        })
    }

    /// Every slice and returned array uses local TABLES order.
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            capitulation: CaptureRows::new(capacity.rows[0]),
            subjects: CaptureRows::new(capacity.rows[1]),
            autonomy: CaptureRows::new(capacity.rows[2]),
            agencies: CaptureRows::new(capacity.rows[3]),
            operatives: CaptureRows::new(capacity.rows[4]),
            cryptology: CaptureRows::new(capacity.rows[5]),
            networks: CaptureRows::new(capacity.rows[6]),
            flags: CaptureRows::new(capacity.rows[7]),
            variables: CaptureRows::new(capacity.rows[8]),
            operations: CaptureRows::new(capacity.rows[9]),
            assignments: CaptureRows::new(capacity.rows[10]),
            operation_tokens: CaptureRows::new(capacity.rows[11]),
            incoming_actions: CaptureRows::new(capacity.rows[12]),
            relation_opinions: CaptureRows::new(capacity.rows[13]),
            relation_tuples: CaptureRows::new(capacity.rows[14]),
            relation_country_index: Vec::new(),
            country_intel: CaptureRows::new(capacity.rows[15]),
            intel_scratch: vec![0; capacity.intel_scratch],
            subject_names: Arena::new(capacity.subject_names),
            autonomy_names: Arena::new(capacity.autonomy_names),
            variable_entries: Arena::new(capacity.variable_entries),
            variable_names: Arena::new(capacity.variable_names),
            tokens: Arena::new(capacity.tokens),
            flag_scratch: vec![cscript_flag::Row::default(); capacity.rows[7]],
            relations_raw_mode: false,
            relation_active: [false; 2],
            capitulation_raw_mode: false,
        }
    }

    pub fn reset(&mut self) {
        self.capitulation.reset();
        self.relation_opinions.reset();
        self.relation_tuples.reset();
        self.reset_residual();
    }

    fn reset_residual(&mut self) {
        self.subjects.reset();
        self.autonomy.reset();
        self.agencies.reset();
        self.operatives.reset();
        self.cryptology.reset();
        self.networks.reset();
        self.flags.reset();
        self.variables.reset();
        self.operations.reset();
        self.assignments.reset();
        self.operation_tokens.reset();
        self.incoming_actions.reset();
        self.country_intel.reset();
        self.subject_names.reset();
        self.autonomy_names.reset();
        self.variable_entries.reset();
        self.variable_names.reset();
        self.tokens.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.capitulation.allocated_bytes()
            + self.subjects.allocated_bytes()
            + self.autonomy.allocated_bytes()
            + self.agencies.allocated_bytes()
            + self.operatives.allocated_bytes()
            + self.cryptology.allocated_bytes()
            + self.networks.allocated_bytes()
            + self.flags.allocated_bytes()
            + self.variables.allocated_bytes()
            + self.operations.allocated_bytes()
            + self.assignments.allocated_bytes()
            + self.operation_tokens.allocated_bytes()
            + self.incoming_actions.allocated_bytes()
            + self.relation_opinions.allocated_bytes()
            + self.relation_tuples.allocated_bytes()
            + self.relation_country_index.capacity() * size_of::<usize>()
            + self.country_intel.allocated_bytes()
            + self.intel_scratch.capacity()
            + self.subject_names.allocated_bytes()
            + self.autonomy_names.allocated_bytes()
            + self.variable_entries.allocated_bytes()
            + self.variable_names.allocated_bytes()
            + self.tokens.allocated_bytes()
            + self.flag_scratch.capacity() * size_of::<cscript_flag::Row>()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.capitulation.grow(&mut capacity.rows[0]);
        self.subjects.grow(&mut capacity.rows[1]);
        self.autonomy.grow(&mut capacity.rows[2]);
        self.agencies.grow(&mut capacity.rows[3]);
        self.operatives.grow(&mut capacity.rows[4]);
        self.cryptology.grow(&mut capacity.rows[5]);
        self.networks.grow(&mut capacity.rows[6]);
        self.flags.grow(&mut capacity.rows[7]);
        self.variables.grow(&mut capacity.rows[8]);
        self.operations.grow(&mut capacity.rows[9]);
        self.assignments.grow(&mut capacity.rows[10]);
        self.operation_tokens.grow(&mut capacity.rows[11]);
        self.incoming_actions.grow(&mut capacity.rows[12]);
        self.relation_opinions.grow(&mut capacity.rows[13]);
        self.relation_tuples.grow(&mut capacity.rows[14]);
        self.country_intel.grow(&mut capacity.rows[15]);
        // The intel scratch is a fixed copy window, not a row capacity: it bounds
        // how many 0x20-byte records one bulk copy moves, and a country longer
        // than the window is copied in several passes rather than truncated. It
        // therefore never grows with observed row counts.
        self.subject_names.grow(&mut capacity.subject_names);
        self.autonomy_names.grow(&mut capacity.autonomy_names);
        self.variable_entries.grow(&mut capacity.variable_entries);
        self.variable_names.grow(&mut capacity.variable_names);
        self.tokens.grow(&mut capacity.tokens);
        grow_vec(&mut self.flag_scratch, capacity.rows[7]);
    }

    /// No allocation, encoding, token-name resolution, engine calls or I/O.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        let report = self.capture_tables(
            memory,
            CaptureContext {
                base,
                game_state: state,
                hour,
                force_full,
            },
            true,
            true,
        );
        report.ticks
    }

    /// Acquire the scheduled tables whose buffers belong to this request. The
    /// independently delegated capitulation and relations slots remain intact.
    pub(crate) fn capture_residual(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
    ) -> CaptureReport {
        self.capture_tables(memory, context, true, false)
    }

    /// Complete only the independently delegated slots after residual capture
    /// has returned. Its rows and arenas remain owned by that result.
    pub(crate) fn capture_delegated(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
    ) -> CaptureReport {
        self.capture_tables(memory, context, false, true)
    }

    /// Reject an accepted residual request without retrying any live reads or
    /// changing the independently delegated tables and their raw-mode gates.
    pub(crate) fn fail_residual_capture(
        &mut self,
        context: CaptureContext,
        reason: &'static str,
    ) -> CaptureReport {
        self.reset_residual();
        macro_rules! fail {
            ($($index:literal => $field:ident),+ $(,)?) => {$(
                self.$field.outcome = Outcome::Unavailable {
                    reason: if replay::cadence::should_walk(TABLES[$index], context.hour, context.force_full) {
                        reason
                    } else {
                        replay::cadence::SKIPPED_REASON
                    },
                };
            )+};
        }
        fail!(
            1 => subjects,
            2 => autonomy,
            3 => agencies,
            4 => operatives,
            5 => cryptology,
            6 => networks,
            7 => flags,
            8 => variables,
            9 => operations,
            10 => assignments,
            11 => operation_tokens,
            12 => incoming_actions,
            15 => country_intel,
        );
        CaptureReport::default()
    }

    fn capture_tables(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        include_residual: bool,
        include_delegated: bool,
    ) -> CaptureReport {
        if include_delegated {
            self.capitulation.reset();
            self.relation_opinions.reset();
            self.relation_tuples.reset();
        }
        if include_residual {
            self.reset_residual();
        }
        let CaptureContext {
            base,
            game_state: state,
            hour,
            force_full,
        } = context;
        let mut active = TABLES.map(|table| replay::cadence::should_walk(table, hour, force_full));
        if self.relations_raw_mode {
            active[13] = self.relation_active[0];
            active[14] = self.relation_active[1];
        } else {
            self.relation_active = [active[13], active[14]];
        }
        macro_rules! skip {
            ($($field:ident = $index:literal),+ $(,)?) => {$(
                if !active[$index] {
                    self.$field.outcome = Outcome::Unavailable {
                        reason: replay::cadence::SKIPPED_REASON,
                    };
                }
            )+};
        }
        if include_residual {
            skip!(
                subjects = 1,
                autonomy = 2,
                agencies = 3,
                operatives = 4,
                cryptology = 5,
                networks = 6,
                flags = 7,
                variables = 8,
                operations = 9,
                assignments = 10,
                operation_tokens = 11,
                incoming_actions = 12,
                country_intel = 15
            );
        }
        if include_delegated {
            skip!(
                capitulation = 0,
                relation_opinions = 13,
                relation_tuples = 14
            );
        }
        let mut ticks = [0; N];
        macro_rules! plain {
            ($index:literal, $field:ident, $walker:path) => {{
                if active[$index] {
                    let _reads = read_accounting::table(TABLES[$index]);
                    let begin = qpc();
                    capture_rows(&mut self.$field, |rows| $walker(memory, base, state, rows));
                    ticks[$index] = elapsed(begin);
                }
            }};
        }
        if include_delegated && !self.capitulation_raw_mode {
            plain!(0, capitulation, cdiplomacy_status::walk_capitulation);
        }
        if include_residual {
            let begin = qpc();
            capture_named(
                &mut self.subjects,
                &mut self.subject_names,
                |rows, names| {
                    let _reads = read_accounting::table(TABLES[1]);
                    ArenaRead::from(csubject_relation::walk(memory, base, state, rows, names))
                },
            );
            ticks[1] = elapsed(begin);
            let begin = qpc();
            capture_named(
                &mut self.autonomy,
                &mut self.autonomy_names,
                |rows, names| {
                    let _reads = read_accounting::table(TABLES[2]);
                    ArenaRead::from(ccurrent_autonomy_status::walk(
                        memory, base, state, rows, names,
                    ))
                },
            );
            ticks[2] = elapsed(begin);
            plain!(3, agencies, cintelligence_agency::walk);
            plain!(4, operatives, coperative_leader::walk);
            plain!(5, cryptology, ccryptology::walk);
            plain!(6, networks, ccountry_intel_network::walk);
            if active[7] {
                let begin = qpc();
                capture_rows(&mut self.flags, |rows| {
                    let _reads = read_accounting::table(TABLES[7]);
                    walk_flags(memory, base, state, rows, &mut self.flag_scratch)
                });
                ticks[7] = elapsed(begin);
            }
            if active[8] {
                let begin = qpc();
                capture_variables(
                    &mut self.variables,
                    &mut self.variable_entries,
                    &mut self.variable_names,
                    |scopes, entries, names| {
                        let _reads = read_accounting::table(TABLES[8]);
                        cvariables::walk_scopes(memory, base, state, scopes, entries, names)
                    },
                );
                ticks[8] = elapsed(begin);
            }
            if active[9] {
                let begin = qpc();
                let result = read_accounting::with_table(TABLES[9], || {
                    ccountry_operation_manager::walk(
                        memory,
                        base,
                        state,
                        &mut self.operations.rows,
                        &mut self.assignments.rows,
                    )
                });
                self.operations.outcome = bounded(result.outcome, self.operations.rows.len());
                self.assignments.outcome = bounded(result.assignments, self.assignments.rows.len());
                ticks[9] = elapsed(begin);
            }
            if active[11] {
                let begin = qpc();
                capture_named(
                    &mut self.operation_tokens,
                    &mut self.tokens,
                    |rows, tokens| {
                        let _reads = read_accounting::table(TABLES[11]);
                        let result = ccountry_operation_token_manager::walk(
                            memory, base, state, rows, tokens,
                        );
                        ArenaRead {
                            outcome: result.outcome,
                            len: result.tokens_len,
                            required: result.required_tokens,
                        }
                    },
                );
                ticks[11] = elapsed(begin);
            }
            plain!(12, incoming_actions, incoming_diplomatic_action_queue::walk);
        }
        let relation_walk =
            include_delegated && (active[13] || active[14]) && !self.relations_raw_mode;
        if relation_walk {
            let anchor = if active[14] { 14 } else { 13 };
            let _reads = read_accounting::table(TABLES[anchor]);
            let begin = qpc();
            let result = crelation_status::walk(
                memory,
                base,
                state,
                &mut self.relation_opinions.rows,
                &mut self.relation_tuples.rows,
                [active[13], active[14]],
            );
            if active[13] {
                self.relation_opinions.outcome =
                    bounded(result.opinions, self.relation_opinions.rows.len());
            }
            if active[14] {
                self.relation_tuples.outcome =
                    bounded(result.tuples, self.relation_tuples.rows.len());
            }
            ticks[anchor] = elapsed(begin);
        }
        if include_residual {
            let begin = qpc();
            capture_rows(&mut self.country_intel, |rows| {
                let _reads = read_accounting::table(TABLES[15]);
                ccountry_intel::walk(memory, base, state, rows, &mut self.intel_scratch)
            });
            ticks[15] = elapsed(begin);
        }
        CaptureReport { ticks }
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.capitulation.outcome,
            self.subjects.outcome,
            self.autonomy.outcome,
            self.agencies.outcome,
            self.operatives.outcome,
            self.cryptology.outcome,
            self.networks.outcome,
            self.flags.outcome,
            self.variables.outcome,
            self.operations.outcome,
            self.assignments.outcome,
            self.operation_tokens.outcome,
            self.incoming_actions.outcome,
            self.relation_opinions.outcome,
            self.relation_tuples.outcome,
            self.country_intel.outcome,
        ]
    }

    /// Called only on the spool thread; this owns all Vec and string conversion.
    pub(crate) fn table_inputs(
        &self,
        baseline: Option<(BaselineToken, &[rows::ScriptVariablesRow])>,
        scratch: &mut ScriptVariableComparison,
        reuse: Option<&mut IntelConversion<'_>>,
    ) -> [TableInput; N] {
        let outcomes = self.outcomes();
        let (flags, country_intel) = if let Some(reuse) = reuse {
            (
                reuse
                    .shadow
                    .flags
                    .input(TableId::ScriptFlags, &self.flags, outcomes[7], || {
                        self.flags_table(outcomes[7])
                    }),
                reuse.shadow.country_intel.input(
                    TableId::CountryIntel,
                    &self.country_intel,
                    outcomes[15],
                    || self.country_intel_table(outcomes[15]),
                ),
            )
        } else {
            (
                TableInput::Sampled(self.flags_table(outcomes[7])),
                TableInput::Sampled(self.country_intel_table(outcomes[15])),
            )
        };
        self.convert_tables(TableInput::Sampled, flags, country_intel, |outcome| {
            if let Some((token, previous)) = baseline
                && self.variables_equal(outcome, previous, scratch)
            {
                TableInput::ComparedEqual(token)
            } else {
                TableInput::Sampled(self.variables_table(outcome))
            }
        })
    }

    #[cfg(test)]
    fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        self.convert_tables(
            std::convert::identity,
            self.flags_table(outcomes[7]),
            self.country_intel_table(outcomes[15]),
            |outcome| self.variables_table(outcome),
        )
    }

    fn convert_tables<V>(
        &self,
        sample: impl Fn(TableView) -> V,
        flags: V,
        country_intel: V,
        variables: impl FnOnce(Outcome) -> V,
    ) -> [V; N] {
        let outcomes = self.outcomes();
        [
            sample(table::<tables::Capitulation, _>(
                outcomes[0],
                &self.capitulation.rows,
                |r| rows::CapitulationRow {
                    country: r.country,
                    capitulated: r.capitulated,
                    capitulated_hour: r.capitulated_hour,
                    last_surrender_hour: r.last_surrender_hour,
                    faction_join_hour: r.faction_join_hour,
                    legitimacy_raw: r.legitimacy_raw,
                    exile_army_leaders: r.exile_army_leaders,
                },
            )),
            sample(self.subjects_table(outcomes[1])),
            sample(self.autonomy_table(outcomes[2])),
            sample(table::<tables::IntelligenceAgencies, _>(
                outcomes[3],
                &self.agencies.rows,
                |r| rows::IntelligenceAgencyRow {
                    country: r.country,
                    upgrade_progress_raw: r.upgrade_progress_raw,
                    defense_raw: r.defense_raw,
                    active_count: r.active_count,
                    admission_ceiling: r.admission_ceiling,
                    unknown_f0: r.unknown_f0,
                },
            )),
            sample(table::<tables::OperativeMemberships, _>(
                outcomes[4],
                &self.operatives.rows,
                |r| rows::OperativeMembershipRow {
                    id: r.id,
                    owner_country: r.owner_country,
                    roster_country: r.roster_country,
                    roster_kind: r.roster_kind,
                    detected: r.detected,
                    assignment: r.assignment,
                    experience_raw: r.experience_raw,
                    skill: r.skill,
                    capturer_country: r.capturer_country,
                    captured_hour: r.captured_hour,
                    action_variant: r.action_variant,
                },
            )),
            sample(table::<tables::Cryptology, _>(
                outcomes[5],
                &self.cryptology.rows,
                |r| rows::CryptologyRow {
                    owner_country: r.owner_country,
                    target_country: r.target_country,
                    amount_raw: r.amount_raw,
                    bonus_days: r.bonus_days,
                    bonus_active: r.bonus_active,
                    decrypting: r.decrypting,
                },
            )),
            sample(table::<tables::IntelNetworks, _>(
                outcomes[6],
                &self.networks.rows,
                |r| rows::IntelNetworkRow {
                    owner_country: r.owner_country,
                    target_country: r.target_country,
                    state_id: r.state_id,
                    strength_raw: r.strength_raw,
                    subnetwork_selector: r.subnetwork_selector,
                },
            )),
            flags,
            variables(outcomes[8]),
            sample(self.operations_table(outcomes[9])),
            sample(table::<tables::OperationAssignments, _>(
                outcomes[10],
                &self.assignments.rows,
                |r| rows::OperationAssignmentRow {
                    operation: r.operation,
                    ordinal: r.ordinal,
                    operative: r.operative,
                    selection: r.selection,
                },
            )),
            sample(self.operation_tokens_table(outcomes[11])),
            sample(table::<tables::IncomingDiplomaticActions, _>(
                outcomes[12],
                &self.incoming_actions.rows,
                |r| rows::IncomingDiplomaticActionRow {
                    country: r.country,
                    ordinal: r.ordinal,
                    handle: r.handle,
                    status: r.status,
                    action_type: r.action_type,
                    actor: r.actor,
                    original_actor: r.original_actor,
                    recipient: r.recipient,
                    original_recipient: r.original_recipient,
                    hour: r.hour,
                    last_command_hour: r.last_command_hour,
                },
            )),
            sample(table::<tables::RelationOpinions, _>(
                outcomes[13],
                &self.relation_opinions.rows,
                |r| rows::RelationOpinionRow {
                    owner_country: r.owner_country,
                    target_country: r.target_country,
                    opinion: r.opinion,
                },
            )),
            sample(self.relation_tuples_table(outcomes[14])),
            country_intel,
        ]
    }
}

impl Captured {
    fn country_intel_table(&self, outcome: Outcome) -> TableView {
        table::<tables::CountryIntel, _>(outcome, &self.country_intel.rows, |r| {
            rows::CountryIntelRow {
                owner_country: r.owner_country,
                target_index: r.target_index,
                civilian: r.civilian,
                army: r.army,
                navy: r.navy,
                air: r.air,
            }
        })
    }

    fn subjects_table(&self, outcome: Outcome) -> TableView {
        if !valid_arena(
            outcome,
            &self.subjects.rows,
            &self.subject_names.first,
            self.subject_names.len,
            self.subject_names.required,
        ) {
            return unavailable("subject name lies outside captured arena");
        }
        table::<tables::Subjects, _>(outcome, &self.subjects.rows, |r| rows::SubjectRow {
            overlord: r.overlord,
            subject: r.subject,
            start_hour: r.start_hour,
            end_hour: r.end_hour,
            cancel: r.cancel,
            autonomy_ratio: r.autonomy_ratio,
            state: literal(&self.subject_names.first, r.state).to_vec(),
        })
    }

    fn autonomy_table(&self, outcome: Outcome) -> TableView {
        if !valid_arena(
            outcome,
            &self.autonomy.rows,
            &self.autonomy_names.first,
            self.autonomy_names.len,
            self.autonomy_names.required,
        ) {
            return unavailable("autonomy name lies outside captured arena");
        }
        table::<tables::Autonomy, _>(outcome, &self.autonomy.rows, |r| rows::AutonomyRow {
            subject: r.subject,
            score: r.score,
            lower_score: r.lower_score,
            upper_score: r.upper_score,
            last_modified_bits: r.last_modified_bits,
            state: literal(&self.autonomy_names.first, r.state).to_vec(),
            previous: r
                .previous
                .map(|name| literal(&self.autonomy_names.first, name).to_vec()),
            next: r
                .next
                .map(|name| literal(&self.autonomy_names.first, name).to_vec()),
        })
    }

    fn flags_table(&self, outcome: Outcome) -> TableView {
        table::<tables::ScriptFlags, _>(outcome, &self.flags.rows, |r| rows::ScriptFlagRow {
            scope_kind: r.scope_kind,
            scope_id: r.scope_id,
            token: r.flag.token,
            set_hour: r.flag.set_hour,
            value: r.flag.value,
            remaining_days: r.flag.remaining_days,
        })
    }

    fn variables_equal(
        &self,
        outcome: Outcome,
        previous: &[rows::ScriptVariablesRow],
        scratch: &mut ScriptVariableComparison,
    ) -> bool {
        let Outcome::Rows { len } = outcome else {
            return false;
        };
        if len != previous.len()
            || !valid_variables(
                outcome,
                &self.variables.rows,
                &self.variable_entries,
                &self.variable_names,
            )
        {
            return false;
        }
        let key = |row: &rows::ScriptVariablesRow| (row.scope_kind, row.scope_id);
        if previous
            .windows(2)
            .any(|pair| key(&pair[0]) >= key(&pair[1]))
        {
            return false;
        }
        clear_seen(&mut scratch.scopes, len);
        for (ordinal, scope) in self.variables.rows[..len].iter().enumerate() {
            let identity = scope_identity(scope.scope);
            // State scopes arrive in ID order, while canonical country slots
            // need not follow raw country IDs. Treat the ordinal only as a
            // checked lookup hint; reordered scopes still use the exact search.
            let index = if key(&previous[ordinal]) == identity {
                ordinal
            } else {
                let Ok(index) = previous.binary_search_by_key(&identity, key) else {
                    return false;
                };
                index
            };
            // Equal counts and successful lookups alone do not exclude duplicate
            // current scopes or names replacing an omitted baseline key.
            if !first_seen(&mut scratch.scopes, index) {
                return false;
            }
            let old = &previous[index].entries;
            if scope.entries_len != old.len()
                || old.windows(2).any(|pair| pair[0].name >= pair[1].name)
            {
                return false;
            }
            clear_seen(&mut scratch.entries, old.len());
            for entry in &self.variable_entries.first
                [scope.entries_start..scope.entries_start + scope.entries_len]
            {
                let name =
                    &self.variable_names.first[entry.name_start..entry.name_start + entry.name_len];
                let Ok(index) = old.binary_search_by(|row| row.name.as_slice().cmp(name)) else {
                    return false;
                };
                if !first_seen(&mut scratch.entries, index)
                    || entry.value_raw != old[index].value_raw
                {
                    return false;
                }
            }
        }
        true
    }

    fn variables_table(&self, outcome: Outcome) -> TableView {
        if !valid_variables(
            outcome,
            &self.variables.rows,
            &self.variable_entries,
            &self.variable_names,
        ) {
            return unavailable("script variable entry or name lies outside captured arena");
        }
        let view = table::<tables::ScriptVariables, _>(outcome, &self.variables.rows, |r| {
            let (scope_kind, scope_id) = scope_identity(r.scope);
            let mut entries: Vec<_> = self.variable_entries.first
                [r.entries_start..r.entries_start + r.entries_len]
                .iter()
                .map(|entry| rows::ScriptVariableEntry {
                    name: self.variable_names.first
                        [entry.name_start..entry.name_start + entry.name_len]
                        .to_vec(),
                    value_raw: entry.value_raw,
                })
                .collect();
            entries.sort_unstable_by(|a, b| a.name.cmp(&b.name));
            rows::ScriptVariablesRow {
                scope_kind,
                scope_id,
                entries,
            }
        });
        let values = match &view {
            TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows,
            _ => return view,
        };
        if values
            .rows::<tables::ScriptVariables>()
            .is_some_and(|rows| {
                rows.iter().any(|row| {
                    row.entries
                        .windows(2)
                        .any(|pair| pair[0].name == pair[1].name)
                })
            })
        {
            return unavailable("duplicate literal script variable key");
        }
        view
    }

    fn operations_table(&self, outcome: Outcome) -> TableView {
        table::<tables::Operations, _>(outcome, &self.operations.rows, |r| rows::OperationRow {
            id: r.id,
            owner: r.owner,
            target: r.target,
            target_province: r.target_province,
            definition_token: r.definition_token,
            preparation_current: r.preparation_current,
            preparation_required: r.preparation_required,
            flag_40: r.flag_40,
            auto_repeat: r.auto_repeat,
            terminal: r.terminal,
            start_hour: r.start_hour,
            duration_days: r.duration_days,
            phase_count: r.phase_count,
            assigned_slots: r.assigned_slots,
        })
    }

    /// The wire ordinal is the native serializable-vector index, so a native
    /// vector longer than the wire width cannot be represented and refuses the
    /// table rather than aliasing two relations onto one key.
    fn relation_tuples_table(&self, outcome: Outcome) -> TableView {
        if self.relation_tuples.rows[..outcome.len()]
            .iter()
            .any(|row| u16::try_from(row.ordinal).is_err())
        {
            return unavailable("relation tuple ordinal exceeds its wire width");
        }
        table::<tables::RelationTuples, _>(outcome, &self.relation_tuples.rows, |r| {
            rows::RelationTupleRow {
                owner_country: r.owner_country,
                target_country: r.target_country,
                type_id: r.type_id,
                ordinal: r.ordinal as u16,
                first_country: r.first_country,
                second_country: r.second_country,
                start_hour: r.start_hour,
                end_hour: r.end_hour,
                cancel: r.cancel,
                retired: r.retired,
            }
        })
    }

    fn operation_tokens_table(&self, outcome: Outcome) -> TableView {
        if !valid_arena(
            outcome,
            &self.operation_tokens.rows,
            &self.tokens.first,
            self.tokens.len,
            self.tokens.required,
        ) {
            return unavailable("operation token vector lies outside captured arena");
        }
        table::<tables::OperationTokens, _>(outcome, &self.operation_tokens.rows, |r| {
            rows::OperationTokensRow {
                owner: r.owner,
                target: r.target,
                tokens: self.tokens.first[r.tokens_start..r.tokens_start + r.tokens_len].to_vec(),
            }
        })
    }
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

fn bounded(outcome: Outcome, capacity: usize) -> Outcome {
    if outcome.len() > capacity {
        Outcome::Unavailable {
            reason: "walker row count exceeds allocated buffer",
        }
    } else if matches!(outcome, Outcome::Truncated { unread_rows: 0, .. }) {
        Outcome::Unavailable {
            reason: "walker marked truncation without missing rows",
        }
    } else {
        outcome
    }
}

fn capture_rows<R>(captured: &mut CaptureRows<R>, walk: impl FnOnce(&mut [R]) -> Outcome) {
    captured.outcome = bounded(walk(&mut captured.rows), captured.rows.len());
}

#[derive(Clone, Copy)]
struct ArenaRead {
    outcome: Outcome,
    len: usize,
    required: usize,
}

impl From<csubject_relation::NameWalk> for ArenaRead {
    fn from(value: csubject_relation::NameWalk) -> Self {
        Self {
            outcome: value.outcome,
            len: value.bytes_len,
            required: value.required_bytes,
        }
    }
}

/// Validate each row against its owned arena.
trait ArenaRow<T> {
    fn valid(&self, len: usize) -> bool;
}

fn fits(start: usize, count: usize, len: usize) -> bool {
    start.checked_add(count).is_some_and(|end| end <= len)
}

fn name_fits(name: Name, len: usize) -> bool {
    fits(name.start, name.len, len)
}

fn literal(bytes: &[u8], name: Name) -> &[u8] {
    &bytes[name.start..name.start + name.len]
}

impl ArenaRow<u8> for csubject_relation::Row {
    fn valid(&self, len: usize) -> bool {
        name_fits(self.state, len)
    }
}

impl ArenaRow<u8> for ccurrent_autonomy_status::Row {
    fn valid(&self, len: usize) -> bool {
        name_fits(self.state, len)
            && self.previous.is_none_or(|name| name_fits(name, len))
            && self.next.is_none_or(|name| name_fits(name, len))
    }
}

impl ArenaRow<u32> for ccountry_operation_token_manager::Row {
    fn valid(&self, len: usize) -> bool {
        fits(self.tokens_start, self.tokens_len, len)
    }
}

fn valid_arena<R: ArenaRow<T>, T>(
    outcome: Outcome,
    rows: &[R],
    values: &[T],
    len: usize,
    required: usize,
) -> bool {
    len <= values.len()
        && required >= len
        && rows
            .get(..outcome.len())
            .is_some_and(|rows| rows.iter().all(|row| row.valid(len)))
}

fn arena_outcome<R: ArenaRow<T>, T>(result: ArenaRead, rows: &[R], values: &[T]) -> Outcome {
    let outcome = bounded(result.outcome, rows.len());
    if valid_arena(outcome, rows, values, result.len, result.required) {
        outcome
    } else {
        Outcome::Unavailable {
            reason: "captured arena range invalid",
        }
    }
}

fn first_named<R: ArenaRow<T>, T>(
    captured: &mut CaptureRows<R>,
    arena: &mut Arena<T>,
    first: ArenaRead,
) {
    captured.outcome = arena_outcome(first, &captured.rows, &arena.first);
    arena.len = first.len;
    arena.required = first.required;
}

fn capture_named<R: ArenaRow<T>, T>(
    captured: &mut CaptureRows<R>,
    arena: &mut Arena<T>,
    walk: impl FnOnce(&mut [R], &mut [T]) -> ArenaRead,
) {
    let result = walk(&mut captured.rows, &mut arena.first);
    first_named(captured, arena, result);
}

fn scope_identity(scope: cvariables::Scope) -> (u8, u32) {
    match scope {
        cvariables::Scope::Global => (GLOBAL_SCOPE, 0),
        cvariables::Scope::Country(id) => (COUNTRY_SCOPE, id),
        cvariables::Scope::State(id) => (STATE_SCOPE, id),
    }
}

fn capture_variables(
    captured: &mut CaptureRows<cvariables::ScopeRow>,
    entries: &mut Arena<cvariables::Row>,
    names: &mut Arena<u8>,
    walk: impl FnOnce(
        &mut [cvariables::ScopeRow],
        &mut [cvariables::Row],
        &mut [u8],
    ) -> cvariables::ScopeWalk,
) {
    let result = walk(&mut captured.rows, &mut entries.first, &mut names.first);
    captured.outcome = variable_outcome(result, &captured.rows, &entries.first, &names.first);
    entries.len = result.entries_len;
    entries.required = result.required_entries;
    names.len = result.bytes_len;
    names.required = result.required_bytes;
}

fn variable_outcome(
    read: cvariables::ScopeWalk,
    rows: &[cvariables::ScopeRow],
    entries: &[cvariables::Row],
    names: &[u8],
) -> Outcome {
    if valid_variable_parts(read, rows, entries, names) {
        read.outcome
    } else {
        Outcome::Unavailable {
            reason: "script variable entry or name lies outside captured arena",
        }
    }
}

fn valid_variables(
    outcome: Outcome,
    rows: &[cvariables::ScopeRow],
    entries: &Arena<cvariables::Row>,
    names: &Arena<u8>,
) -> bool {
    valid_variable_parts(
        cvariables::ScopeWalk {
            outcome,
            entries_len: entries.len,
            required_entries: entries.required,
            bytes_len: names.len,
            required_bytes: names.required,
        },
        rows,
        &entries.first,
        &names.first,
    )
}

fn valid_variable_parts(
    read: cvariables::ScopeWalk,
    rows: &[cvariables::ScopeRow],
    entries: &[cvariables::Row],
    names: &[u8],
) -> bool {
    read.entries_len <= entries.len()
        && read.required_entries >= read.entries_len
        && read.bytes_len <= names.len()
        && read.required_bytes >= read.bytes_len
        && rows.get(..read.outcome.len()).is_some_and(|rows| {
            rows.iter().all(|row| {
                fits(row.entries_start, row.entries_len, read.entries_len)
                    && entries[row.entries_start..row.entries_start + row.entries_len]
                        .iter()
                        .all(|entry| fits(entry.name_start, entry.name_len, read.bytes_len))
            })
        })
}

/// Proven expiry callers dereference all three manager roots. Slot zero and
/// null state holes are not scopes; canonical country aliases are visited once.
fn walk_flags(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut [ScopedFlag],
    scratch: &mut [cscript_flag::Row],
) -> Outcome {
    let result = (|| {
        let mut len = 0;
        let mut unread = 0_u32;
        let mut scope = |kind, id, root, offset| -> Result<(), &'static str> {
            let manager = pointer(memory, root, offset)?;
            if manager == 0 {
                return Err("script flag manager root is null");
            }
            let available = (output.len() - len).min(scratch.len());
            let outcome = bounded(
                cflag_manager::walk(memory, base, manager, &mut scratch[..available]),
                available,
            );
            let count = match outcome {
                Outcome::Rows { len } => len,
                Outcome::Truncated { len, unread_rows } => {
                    unread = unread
                        .checked_add(unread_rows)
                        .ok_or("flag omitted count overflow")?;
                    len
                }
                Outcome::Unavailable { reason } => return Err(reason),
            };
            for flag in &scratch[..count] {
                output[len] = ScopedFlag {
                    scope_kind: kind,
                    scope_id: id,
                    flag: *flag,
                };
                len += 1;
            }
            Ok(())
        };
        scope(GLOBAL_SCOPE, 0, state, cgame_state::GLOBAL_FLAGS)?;
        let countries = ccountry::Reader::new(memory, base, state)?;
        let mut found_country = false;
        for slot in 1..countries.count() {
            if let Some(country) = countries.at(slot)? {
                found_country = true;
                scope(
                    COUNTRY_SCOPE,
                    country.id,
                    country.pointer,
                    ccountry::FLAG_MANAGER,
                )?;
            }
        }
        if !found_country {
            return Err("country table has no readable countries");
        }
        let states = state
            .checked_add(cgame_state::STATES)
            .and_then(|at| Array::read(memory, at, 8))
            .ok_or("state scope array unreadable")?;
        if states.count < 2 {
            return Err("state table has no state slots");
        }
        let state_vtable = base
            .checked_add(cstate::VTABLE_RVA)
            .ok_or("state vtable overflow")?;
        let mut first = 1;
        let mut found_state = false;
        let mut pointers = [0_u8; 512];
        while first < states.count {
            let count = ((states.count - first) as usize).min(64);
            let address = states
                .element(first, 8)
                .ok_or("state scope pointer overflow")?;
            if !read_slice(memory, address, &mut pointers[..count * 8]) {
                return Err("state scope pointer array unreadable");
            }
            for (index, bytes) in pointers[..count * 8].chunks_exact(8).enumerate() {
                let at = usize::try_from(u64::from_le_bytes(bytes.try_into().unwrap()))
                    .map_err(|_| "state scope pointer overflow")?;
                if at == 0 {
                    continue;
                }
                found_state = true;
                if read::<u64>(memory, at) != Some(state_vtable as u64) {
                    return Err("state scope vtable mismatch");
                }
                let id = at
                    .checked_add(cstate::ID)
                    .and_then(|p| read::<u32>(memory, p))
                    .ok_or("state scope id unreadable")?;
                if id != first + index as u32 {
                    return Err("state scope id differs from slot");
                }
                scope(STATE_SCOPE, id, at, cstate::FLAGS)?;
            }
            first += count as u32;
        }
        if !found_state {
            return Err("state table has no readable states");
        }
        Ok(if unread == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: unread,
            }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

fn pointer(memory: &impl Memory, root: usize, offset: usize) -> Result<usize, &'static str> {
    root.checked_add(offset)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .ok_or("script scope root unreadable")
}

// ===========================================================================
// Relations copy/decode split (family 1: opinions + tuples).
//
// Acquisition (`copy_relations_refresh`): raw copies only. It uses
// the factored engine `copy_*` helpers (`copy_slots`, `copy_auth`,
// `copy_rest`) plus plain roster-structure reads (country enumeration,
// diplomacy pointers, matrix/serializable headers, pointer pages) to discover
// what to copy. No vtable, backlink, target, type, ownership or ordering
// validation happens here; discovery interprets only already-copied bytes to
// find the next spans (serializable headers for vector bounds, auth bytes
// best-effort for rest-copy gating). Every span is recorded with its success
// flag, so decode reproduces identical faults with no live access. Rest bytes
// are copied for every serializable element (including skipped war/subject);
// decode ignores rest for skipped elements exactly like the typed walk, which
// never reads them.
//
// Decode worker (`Captured::decode_relations_payload`): replays the same
// validations as `crelation_status::walk` through the factored `decode_*`
// helpers (`decode_slot`, `decode_auth`, `decode_rest`,
// `Array::from_header`) into the identical typed rows and outcomes.
// Unavailable outcomes discard rows exactly like the typed walk, so any
// recorded fault short-circuits to the same reason.
//
// ===========================================================================

/// Frozen country identity for decode-side resolution without live access.
/// Built once per pass from the game-thread country enumeration, which the
/// typed walk also performs; decode looks up targets/endpoints by raw id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CountrySnap {
    pub slot: u32,
    pub pointer: usize,
    pub id: u32,
    pub canonical_slot: u32,
}

/// Exact completion of one tuple's auth fields. Semantic errors stay in the
/// complete auth bytes and are interpreted in chunk order by the decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum TupleAuthRead {
    Complete,
    AddressOverflow,
    VtableUnreadable,
    TypeUnreadable,
}

impl TupleAuthRead {
    fn reason(self) -> Option<&'static str> {
        match self {
            Self::Complete => None,
            Self::AddressOverflow => Some("relation field address overflow"),
            Self::VtableUnreadable => Some("relation vtable unreadable"),
            Self::TypeUnreadable => Some("relation type word unreadable"),
        }
    }
}

/// Exact completion of one tuple's rest fields. Only complete records may
/// supply decoded endpoints or rows; the failing field retains its own reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum TupleRestRead {
    Complete,
    NotRequested,
    AddressOverflow,
    FirstCountryUnreadable,
    SecondCountryUnreadable,
    StartHourUnreadable,
    EndHourUnreadable,
    CancelUnreadable,
    RetiredUnreadable,
}

impl TupleRestRead {
    fn reason(self) -> Option<&'static str> {
        match self {
            Self::Complete => None,
            Self::NotRequested | Self::FirstCountryUnreadable => {
                Some("relation first country unreadable")
            }
            Self::AddressOverflow => Some("relation field address overflow"),
            Self::SecondCountryUnreadable => Some("relation second country unreadable"),
            Self::StartHourUnreadable => Some("relation start hour unreadable"),
            Self::EndHourUnreadable => Some("relation end hour unreadable"),
            Self::CancelUnreadable => Some("relation cancel byte unreadable"),
            Self::RetiredUnreadable => Some("relation retired byte unreadable"),
        }
    }
}

/// One serializable relation element in walk order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TupleOcc {
    pub owner_id: u32,
    pub owner_canonical: u32,
    pub target_country: u32,
    pub ordinal: u32,
    pub auth: [u8; crelation_status::TUPLE_AUTH_BYTES],
    pub auth_read: TupleAuthRead,
    pub rest: [u8; crelation_status::TUPLE_REST_BYTES],
    pub rest_read: TupleRestRead,
}

/// Fixed-size per-owner copy metadata. Blobs live in the job-level pools
/// addressed by the start/len ranges below.
#[derive(Debug, Default)]
pub(crate) struct OwnerMeta {
    pub owner_id: u32,
    pub owner_canonical: u32,
    pub owner_pointer: usize,
    pub diplomacy_vtable: [u8; 8],
    pub diplomacy_backlink: [u8; 8],
    pub diplomacy_scattered: usize,
    pub diplomacy_err: Option<&'static str>,
    pub matrix_header: [u8; 16],
    pub matrix_header_ok: bool,
    pub matrix_err: Option<&'static str>,
    pub slot_start: usize,
    pub slot_len: usize,
    pub tuple_start: usize,
    pub tuple_len: usize,
}

/// One refresh of raw relations payload. Flat pools keep every per-item struct
/// fixed-size so `clear` recycles capacities and the steady state allocates
/// on neither thread; growth happens on the decode side only (see
/// `crate::decode::recycle_relations_job`).
#[derive(Debug, Default)]
pub(crate) struct RelationsJob {
    pub active: [bool; 2],
    pub countries: Vec<CountrySnap>,
    pub links: Vec<Result<Option<u32>, &'static str>>,
    pub highest_slot: u32,
    pub metas: Vec<OwnerMeta>,
    /// Initialized backing storage retained across refreshes. Only the current
    /// `slot_used` prefix belongs to this payload; inactive spans are ignored.
    pub slot_pool: Vec<u8>,
    pub slot_used: usize,
    pub tuples: Vec<TupleOcc>,
    pub reader_err: Option<&'static str>,
    pub country_err: Option<&'static str>,
    pub top_err: Option<&'static str>,
    pub unread_opinions: u32,
    pub unread_tuples: u32,
    pub overflowed: bool,
    capacity_charge: crate::raw_memory::Charge,
}

impl RelationsJob {
    pub fn new() -> Self {
        let mut job = Self {
            active: [true; 2],
            countries: Vec::with_capacity(512),
            links: Vec::with_capacity(512),
            highest_slot: 0,
            metas: Vec::with_capacity(512),
            slot_pool: vec![0; 8 << 20],
            slot_used: 0,
            tuples: Vec::with_capacity(8192),
            reader_err: None,
            country_err: None,
            top_err: None,
            unread_opinions: 0,
            unread_tuples: 0,
            overflowed: false,
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        job.update_capacity_charge();
        job
    }

    /// Reset payload lengths, retaining initialized slot storage and capacities.
    /// No allocation or clearing of bytes that the next slot gather overwrites.
    pub fn clear(&mut self) {
        self.active = [false; 2];
        self.countries.clear();
        self.links.clear();
        self.highest_slot = 0;
        self.metas.clear();
        self.slot_used = 0;
        self.tuples.clear();
        self.reader_err = None;
        self.country_err = None;
        self.top_err = None;
        self.unread_opinions = 0;
        self.unread_tuples = 0;
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
        double(&mut self.countries);
        double(&mut self.links);
        double(&mut self.metas);
        double(&mut self.slot_pool);
        self.slot_pool.resize(self.slot_pool.capacity(), 0);
        double(&mut self.tuples);
        self.update_capacity_charge();
    }

    fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            payload_vectors: vector_bytes(&self.countries)
                + vector_bytes(&self.links)
                + vector_bytes(&self.metas)
                + vector_bytes(&self.slot_pool)
                + vector_bytes(&self.tuples),
            ..Default::default()
        });
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct RelationsCopyReport {
    /// False when staging overflowed: the caller falls back to the typed walk
    /// (identical rows through the existing path) and enqueues no payload.
    pub raw_ok: bool,
    pub copy_ticks: [u64; 2],
    pub raw_bytes: [u64; 2],
}

/// Acquisition parameters, bundled to keep the refresh entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RelationsCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub active: [bool; 2],
    pub opinion_cap: usize,
    pub tuple_cap: usize,
}

#[derive(Debug, Default)]
struct RelationsTallies {
    ticks: [u64; 2],
    bytes: [u64; 2],
}

fn relations_fit_len(used: usize, capacity: usize, need: usize) -> bool {
    need <= capacity.saturating_sub(used)
}

/// Build once on the worker, bounded by the highest authenticated census
/// slot. `Reader::at` publishes only canonical slots inside the country table;
/// raw links (including bad or unused aliases) never determine this allocation.
/// Clearing every slot preserves first-match semantics across job reuse/churn.
fn prepare_relation_country_index(index: &mut Vec<usize>, countries: &[CountrySnap]) {
    let slots = countries
        .iter()
        .map(|country| country.canonical_slot as usize + 1)
        .max()
        .unwrap_or(0);
    index.clear();
    index.resize(slots, usize::MAX);
    for (country_index, country) in countries.iter().enumerate() {
        let first = &mut index[country.canonical_slot as usize];
        if *first == usize::MAX {
            *first = country_index;
        }
    }
}

fn relations_lookup(
    job: &RelationsJob,
    country_index: &[usize],
    raw: u32,
) -> Result<CountrySnap, &'static str> {
    if raw == 0 {
        return Err("country id is zero");
    }
    let canonical = job
        .links
        .get(raw as usize)
        .copied()
        .ok_or("country link unreadable or outside country table")??
        .ok_or("country link unreadable or outside country table")?;
    country_index
        .get(canonical as usize)
        .and_then(|&index| job.countries.get(index))
        .copied()
        .ok_or("country link resolves to null country")
}

fn copy_relations_pass(
    memory: &impl Memory,
    context: RelationsCopyContext,
    job: &mut RelationsJob,
) -> RelationsTallies {
    let RelationsCopyContext {
        base,
        game_state,
        active,
        opinion_cap,
        tuple_cap,
    } = context;
    job.clear();
    job.active = active;
    let mut tallies = RelationsTallies::default();
    if !active.into_iter().any(|active| active) {
        return tallies;
    }
    let anchor = usize::from(active[1]);
    let _reads = read_accounting::table(TABLES[13 + anchor]);
    let fields = crelation_status::slot_fields(active);
    let slot_bytes = fields.iter().map(|field| field.len).sum::<usize>();
    let status_vtable = match base.checked_add(crelation_status::VTABLE_RVA) {
        Some(address) => address as u64,
        None => {
            job.top_err = Some("module address overflow");
            return tallies;
        }
    };
    let _ = status_vtable;
    let countries = match ccountry::Reader::new(memory, base, game_state) {
        Err(reason) => {
            job.reader_err = Some(reason);
            return tallies;
        }
        Ok(countries) => countries,
    };
    let count = countries.count();
    // Highest-slot census mirrors the typed walk: enumeration faults abort
    // the whole pass with the identical reason.
    let mut highest_slot = 0u32;
    for slot in 1..count {
        let started = qpc();
        match countries.at(slot) {
            Err(reason) => {
                job.country_err = Some(reason);
                tallies.ticks[anchor] += elapsed(started);
                return tallies;
            }
            Ok(Some(country)) => {
                highest_slot = slot;
                if job.countries.len() == job.countries.capacity() {
                    job.overflowed = true;
                    return tallies;
                }
                job.countries.push(CountrySnap {
                    slot,
                    pointer: country.pointer,
                    id: country.id,
                    canonical_slot: country.canonical_slot,
                });
            }
            Ok(None) => {}
        }
        tallies.ticks[anchor] += elapsed(started);
    }
    job.highest_slot = highest_slot;
    // Raw references are link-table indices, not the object's current id.
    // Retain every link result independently: a bad unused alias must not
    // invalidate otherwise readable relations.
    if let Some(links) = game_state
        .checked_add(cgame_state::COUNTRY_LINKS)
        .and_then(|p| Array::read(memory, p, 4))
    {
        for raw in 0..links.count {
            if job.links.len() == job.links.capacity() {
                job.overflowed = true;
                return tallies;
            }
            job.links.push(countries.link(raw));
        }
    }
    if job.countries.is_empty() {
        job.top_err = Some("country table has no readable countries");
        return tallies;
    }
    let mut opinion_recorded = 0usize;
    let mut tuple_recorded = 0usize;
    let mut unread_opinions = 0u32;
    let mut unread_tuples = 0u32;
    // Each successful page read and pointer conversion overwrites its entire
    // used prefix. A short read exits before either scratch prefix is used.
    let mut page = [0u8; crelation_status::BATCH * 8];
    let mut pointers = [0usize; crelation_status::BATCH];
    // Borrow the frozen country list by index (no allocation, no live reads
    // after enumeration); entries are `Copy`.
    let country_len = job.countries.len();
    for ci in 0..country_len {
        let entry = job.countries[ci];
        let started = qpc();
        if job.metas.len() == job.metas.capacity() {
            job.overflowed = true;
            return tallies;
        }
        let mut meta = OwnerMeta {
            owner_id: entry.id,
            owner_canonical: entry.canonical_slot,
            owner_pointer: entry.pointer,
            ..OwnerMeta::default()
        };
        // Diplomacy pointer discovery mirrors `diplomacy_root` up to the
        // vtable/backlink identity spans; those spans decode off-thread.
        let diplomacy = match entry
            .pointer
            .checked_add(ccountry::DIPLOMACY_STATUS)
            .and_then(|address| read::<u64>(memory, address))
            .and_then(|value| usize::try_from(value).ok())
        {
            None => {
                meta.diplomacy_err = Some("country diplomacy unreadable");
                job.metas.push(meta);
                tallies.ticks[anchor] += elapsed(started);
                break;
            }
            Some(0) => {
                meta.diplomacy_err = Some("country diplomacy is null");
                job.metas.push(meta);
                tallies.ticks[anchor] += elapsed(started);
                break;
            }
            Some(pointer) => pointer,
        };
        // Identity and matrix metadata are independent observations. Keep their
        // order and failure positions while entering the guarded copier once.
        let header_addr = diplomacy.checked_add(cdiplomacy_status::RELATIONS);
        let header_len = if header_addr.is_some() { 16 } else { 0 };
        let scattered = memory.copy_scattered_prefetched(
            [
                diplomacy,
                diplomacy.saturating_add(cdiplomacy_status::COUNTRY),
                header_addr.unwrap_or(0),
            ],
            &mut [
                &mut meta.diplomacy_vtable,
                &mut meta.diplomacy_backlink,
                &mut meta.matrix_header[..header_len],
            ],
        );
        meta.diplomacy_scattered = scattered.min(2);
        tallies.bytes[anchor] += 16;
        if scattered < 2 {
            meta.diplomacy_err = Some("country diplomacy unreadable");
            job.metas.push(meta);
            tallies.ticks[anchor] += elapsed(started);
            break;
        }
        if header_addr.is_none() || scattered != 3 {
            meta.matrix_err = Some("relation matrix unreadable or invalid");
            job.metas.push(meta);
            tallies.ticks[anchor] += elapsed(started);
            break;
        }
        meta.matrix_header_ok = true;
        tallies.bytes[anchor] += 16;
        let matrix = match Array::from_header(meta.matrix_header, 8) {
            Some(matrix) => matrix,
            None => {
                meta.matrix_err = Some("relation matrix unreadable or invalid");
                job.metas.push(meta);
                tallies.ticks[anchor] += elapsed(started);
                break;
            }
        };
        if matrix.count <= highest_slot {
            meta.matrix_err = Some("relation matrix does not cover country ordinals");
            job.metas.push(meta);
            tallies.ticks[anchor] += elapsed(started);
            break;
        }
        meta.slot_start = job.slot_used;
        meta.tuple_start = job.tuples.len();
        let mut index = 1u32;
        let mut failed = false;
        while index < matrix.count && !failed {
            let len = (matrix.count - index).min(crelation_status::BATCH as u32) as usize;
            let page_addr = match matrix.element(index, 8) {
                Some(address) => address,
                None => {
                    meta.matrix_err = Some("relation matrix pointer address overflow");
                    break;
                }
            };
            if !read_slice(memory, page_addr, &mut page[..len * 8]) {
                meta.matrix_err = Some("relation matrix pointer batch unreadable");
                break;
            }
            for (slot, chunk) in pointers[..len]
                .iter_mut()
                .zip(page[..len * 8].chunks_exact(8))
            {
                match usize::try_from(u64::from_le_bytes(chunk.try_into().unwrap())) {
                    Err(_) => {
                        meta.matrix_err = Some("relation matrix pointer overflow");
                        failed = true;
                        break;
                    }
                    Ok(0) => {
                        meta.matrix_err = Some("null relation matrix entry");
                        failed = true;
                        break;
                    }
                    Ok(pointer) => {
                        *slot = pointer;
                        // The whole pointer page is already copied. Hint only
                        // the exact fields the following gather will read;
                        // actual reads, validation and fault order stay there.
                        engine::prefetch::fields(pointer, fields);
                    }
                }
            }
            if failed {
                break;
            }
            if !relations_fit_len(
                job.slot_used,
                job.slot_pool.len(),
                len * crelation_status::RECORD_BYTES,
            ) {
                job.overflowed = true;
                return tallies;
            }
            let slot_base = job.slot_used;
            job.slot_used += len * crelation_status::RECORD_BYTES;
            let copied = crelation_status::copy_slots(
                memory,
                &pointers[..len],
                &mut job.slot_pool[slot_base..slot_base + len * crelation_status::RECORD_BYTES],
                active,
            );
            tallies.bytes[anchor] += (len * slot_bytes) as u64;
            if copied < len * fields.len() {
                meta.matrix_err = Some("relation matrix slot fields unreadable");
                // Publish only completed records. Partial fields and retained
                // suffix bytes never belong to the current payload.
                let completed = copied / fields.len();
                job.slot_used = slot_base + completed * crelation_status::RECORD_BYTES;
                break;
            }
            // Tuple census per slot, in native vector order. Serializable
            // headers come from already-copied slot bytes (discovery only).
            for offset in 0..len {
                let record: &[u8; crelation_status::RECORD_BYTES] = job.slot_pool[slot_base
                    + offset * crelation_status::RECORD_BYTES
                    ..slot_base + (offset + 1) * crelation_status::RECORD_BYTES]
                    .try_into()
                    .unwrap();
                let serializable = if active[1] {
                    let header: [u8; 16] = record[16..].try_into().unwrap();
                    match Array::from_header(header, 8) {
                        Some(array) => array,
                        None => {
                            meta.matrix_err = Some("serializable relation vector invalid");
                            failed = true;
                            break;
                        }
                    }
                } else {
                    Array { data: 0, count: 0 }
                };
                // Opinion capacity accounting mirrors `push_opinion`: the
                // opinion filter itself decodes off-thread, so count slots
                // with nonzero opinions here from already-copied bytes.
                let opinion = i32::from_le_bytes(record[12..16].try_into().unwrap());
                if active[0] && opinion != 0 {
                    if opinion_recorded == opinion_cap {
                        match unread_opinions.checked_add(1) {
                            Some(next) => unread_opinions = next,
                            None => {
                                meta.matrix_err =
                                    Some("relation opinion unread row count overflow");
                                failed = true;
                                break;
                            }
                        }
                    } else {
                        opinion_recorded += 1;
                    }
                }
                if active[1]
                    && !copy_slot_tuples(
                        memory,
                        base,
                        serializable,
                        entry.id,
                        entry.canonical_slot,
                        u32::from_le_bytes(record[8..12].try_into().unwrap()),
                        tuple_cap,
                        &mut tuple_recorded,
                        &mut unread_tuples,
                        job,
                        &mut meta.matrix_err,
                        &mut tallies,
                    )
                {
                    failed = true;
                    break;
                }
                if job.overflowed {
                    return tallies;
                }
                if meta.matrix_err.is_some() {
                    failed = true;
                    break;
                }
            }
            index += len as u32;
        }
        meta.slot_len = job.slot_used - meta.slot_start;
        meta.tuple_len = job.tuples.len() - meta.tuple_start;
        tallies.ticks[anchor] += elapsed(started);
        let terminal = failed || meta.matrix_err.is_some();
        job.metas.push(meta);
        if terminal {
            // A retained terminal owner decides every active partition. Later
            // admission must not discard that failure and retry live memory.
            break;
        }
    }
    job.unread_opinions = unread_opinions;
    job.unread_tuples = unread_tuples;
    tallies
}

#[allow(clippy::too_many_arguments)]
fn copy_slot_tuples(
    memory: &impl Memory,
    base: usize,
    serializable: Array,
    owner_id: u32,
    owner_canonical: u32,
    target_country: u32,
    tuple_cap: usize,
    tuple_recorded: &mut usize,
    unread_tuples: &mut u32,
    job: &mut RelationsJob,
    meta_err: &mut Option<&'static str>,
    tallies: &mut RelationsTallies,
) -> bool {
    let mut first = 0u32;
    while first < serializable.count {
        let chunk = (serializable.count - first).min(crelation_status::BATCH as u32) as usize;
        // Decline an unprovisioned chunk before observing its pointer, auth or
        // rest bytes. Once a native fault is observed, retaining its full auth
        // chunk and exact failure sentinel must never trigger a live fallback.
        if !relations_fit_len(job.tuples.len(), job.tuples.capacity(), chunk) {
            job.overflowed = true;
            return false;
        }
        let first_address = match serializable.element(first, 8) {
            Some(address) => address,
            None => {
                *meta_err = Some("serializable relation entry unreadable");
                return false;
            }
        };
        let mut pointer_bytes = [0u8; crelation_status::BATCH * 8];
        if !read_slice(memory, first_address, &mut pointer_bytes[..chunk * 8]) {
            *meta_err = Some("serializable relation entry unreadable");
            return false;
        }
        let mut pointers = [0usize; crelation_status::BATCH];
        for (slot, chunk_bytes) in pointers[..chunk]
            .iter_mut()
            .zip(pointer_bytes[..chunk * 8].chunks_exact(8))
        {
            match usize::try_from(u64::from_le_bytes(chunk_bytes.try_into().unwrap())) {
                Err(_) => {
                    *meta_err = Some("serializable relation entry unreadable");
                    return false;
                }
                Ok(0) => {
                    *meta_err = Some("null serializable relation entry");
                    return false;
                }
                Ok(pointer) => *slot = pointer,
            }
        }
        // Auth spans first, exactly like the walk: a short gather attributes
        // the identical field-level fault before any rest copy runs.
        let mut auth_records = [0u8; crelation_status::BATCH * crelation_status::TUPLE_AUTH_BYTES];
        let auth_copied = crelation_status::copy_auth(
            memory,
            &pointers[..chunk],
            &mut auth_records[..chunk * crelation_status::TUPLE_AUTH_BYTES],
        );
        tallies.bytes[1] += (chunk * crelation_status::TUPLE_AUTH_BYTES) as u64;
        // The typed reader interprets each completed auth row before a later
        // short-gather fault. Retain the exact terminal occurrence so decode
        // can keep that precedence without consulting any rest in this chunk.
        let mut live_pointers = [0usize; crelation_status::BATCH];
        let mut live_source = [0usize; crelation_status::BATCH];
        let mut live_len = 0usize;
        let mut failed_auth: Option<(usize, TupleAuthRead)> = None;
        for index in 0..chunk {
            if auth_copied < (index + 1) * crelation_status::TUPLE_AUTH_FIELDS.len() {
                let failed = auth_copied % crelation_status::TUPLE_AUTH_FIELDS.len();
                let pointer = pointers[auth_copied / crelation_status::TUPLE_AUTH_FIELDS.len()];
                let field = &crelation_status::TUPLE_AUTH_FIELDS[failed];
                let status = if pointer
                    .checked_add(field.source_offset)
                    .and_then(|address| address.checked_add(field.len))
                    .is_none()
                {
                    TupleAuthRead::AddressOverflow
                } else if failed == 0 {
                    TupleAuthRead::VtableUnreadable
                } else {
                    TupleAuthRead::TypeUnreadable
                };
                failed_auth = Some((index, status));
                break;
            }
            let auth: &[u8; crelation_status::TUPLE_AUTH_BYTES] = auth_records[index
                * crelation_status::TUPLE_AUTH_BYTES
                ..(index + 1) * crelation_status::TUPLE_AUTH_BYTES]
                .try_into()
                .unwrap();
            match crelation_status::decode_auth(base, auth) {
                Err(_) => {
                    // The complete invalid blob itself supplies the decode error.
                    failed_auth = Some((index, TupleAuthRead::Complete));
                    break;
                }
                Ok(decoded) if decoded.skipped => {}
                Ok(_) => {
                    live_pointers[live_len] = pointers[index];
                    live_source[live_len] = index;
                    live_len += 1;
                }
            }
        }
        // The entire auth chunk must succeed before any rest is requested.
        // Skipped elements and a terminal auth chunk carry unread rest bytes;
        // decode authenticates the whole chunk before consulting those bytes.
        let mut rest_records = [0u8; crelation_status::BATCH * crelation_status::TUPLE_REST_BYTES];
        let mut failed_rest = None;
        let rest_copied = if failed_auth.is_none() && live_len > 0 {
            let copied = crelation_status::copy_rest(
                memory,
                &live_pointers[..live_len],
                &mut rest_records[..live_len * crelation_status::TUPLE_REST_BYTES],
            );
            tallies.bytes[1] += (live_len * crelation_status::TUPLE_REST_BYTES) as u64;
            if copied < live_len * crelation_status::TUPLE_REST_FIELDS.len() {
                let failed = copied % crelation_status::TUPLE_REST_FIELDS.len();
                let pointer = live_pointers[copied / crelation_status::TUPLE_REST_FIELDS.len()];
                let field = &crelation_status::TUPLE_REST_FIELDS[failed];
                let status = if pointer
                    .checked_add(field.source_offset)
                    .and_then(|address| address.checked_add(field.len))
                    .is_none()
                {
                    TupleRestRead::AddressOverflow
                } else {
                    match failed {
                        0 => TupleRestRead::FirstCountryUnreadable,
                        1 => TupleRestRead::SecondCountryUnreadable,
                        2 => TupleRestRead::StartHourUnreadable,
                        3 => TupleRestRead::EndHourUnreadable,
                        4 => TupleRestRead::CancelUnreadable,
                        _ => TupleRestRead::RetiredUnreadable,
                    }
                };
                failed_rest = Some((copied / crelation_status::TUPLE_REST_FIELDS.len(), status));
            }
            copied
        } else {
            0
        };
        let complete_rest = rest_copied / crelation_status::TUPLE_REST_FIELDS.len();
        let mut live_cursor = 0usize;
        let staged = failed_auth.map_or(chunk, |(index, _)| index + 1);
        for index in 0..staged {
            let auth: [u8; crelation_status::TUPLE_AUTH_BYTES] = auth_records[index
                * crelation_status::TUPLE_AUTH_BYTES
                ..(index + 1) * crelation_status::TUPLE_AUTH_BYTES]
                .try_into()
                .unwrap();
            // Rest failures retain every auth occurrence for the decoder's
            // complete-chunk auth pass, but publish only complete earlier rest
            // records. The exact failed live record owns the fault sentinel.
            let live = failed_auth.is_none()
                && live_source[..live_len].get(live_cursor) == Some(&index);
            let emitted = live && live_cursor < complete_rest;
            let rest_read = if emitted {
                TupleRestRead::Complete
            } else if live {
                failed_rest
                    .filter(|(failed, _)| *failed == live_cursor)
                    .map_or(TupleRestRead::NotRequested, |(_, status)| status)
            } else {
                TupleRestRead::NotRequested
            };
            let rest = if emitted {
                rest_records[live_cursor * crelation_status::TUPLE_REST_BYTES
                    ..(live_cursor + 1) * crelation_status::TUPLE_REST_BYTES]
                    .try_into()
                    .unwrap()
            } else {
                [0; crelation_status::TUPLE_REST_BYTES]
            };
            live_cursor += usize::from(live);
            job.tuples.push(TupleOcc {
                owner_id,
                owner_canonical,
                target_country,
                ordinal: first + index as u32,
                auth,
                auth_read: failed_auth
                    .filter(|(failed, _)| *failed == index)
                    .map_or(TupleAuthRead::Complete, |(_, status)| status),
                rest,
                rest_read,
            });
            // Only complete live rest records before a failed field can
            // consume an output row or unread count. Skipped and failed records
            // never advance that count; decode retains endpoint validation.
            if emitted {
                if *tuple_recorded == tuple_cap {
                    match unread_tuples.checked_add(1) {
                        Some(next) => *unread_tuples = next,
                        None => {
                            *meta_err = Some("relation tuple unread row count overflow");
                            return false;
                        }
                    }
                } else {
                    *tuple_recorded += 1;
                }
            }
        }
        if failed_auth.is_some() || failed_rest.is_some() {
            // Keep the terminal occurrence local to its original slot/chunk.
            // An owner-wide error would outrank earlier decoded slot or endpoint
            // failures. No later chunk or slot is requested after this failure.
            return false;
        }
        first += chunk as u32;
    }
    true
}

/// Raw acquisition for a relations refresh with exclusively owned jobs.
/// Captures one current payload and returns copy-only costs. Staging overflow
/// retains the existing typed fallback and enqueues no payload.
pub(crate) fn copy_relations_refresh(
    memory: &impl Memory,
    ctx: RelationsCopyContext,
    job: &mut RelationsJob,
) -> RelationsCopyReport {
    let tallies = copy_relations_pass(memory, ctx, job);
    RelationsCopyReport {
        raw_ok: !job.overflowed,
        copy_ticks: tallies.ticks,
        raw_bytes: tallies.bytes,
    }
}

impl Captured {
    pub(crate) fn opinion_rows_cap(&self) -> usize {
        self.relation_opinions.rows.len()
    }

    pub(crate) fn tuple_rows_cap(&self) -> usize {
        self.relation_tuples.rows.len()
    }

    /// Terminal failure of an accepted relations refresh, without another live
    /// read. Other Intel tables and their independent acquisition gates survive.
    pub(crate) fn fail_relations_capture(&mut self, active: [bool; 2], reason: &'static str) {
        self.relation_active = active;
        self.relation_opinions.outcome = Outcome::Unavailable {
            reason: if active[0] {
                reason
            } else {
                replay::cadence::SKIPPED_REASON
            },
        };
        self.relation_tuples.outcome = Outcome::Unavailable {
            reason: if active[1] {
                reason
            } else {
                replay::cadence::SKIPPED_REASON
            },
        };
        self.relations_raw_mode = false;
    }

    /// Decode-worker interpretation of a raw refresh payload into the pooled
    /// typed rows. Outcomes match the typed walk exactly. Clears
    /// `relations_raw_mode` so the next capture starts clean.
    pub(crate) fn decode_relations_payload(
        &mut self,
        base: usize,
        job: &RelationsJob,
        decode_ticks: &mut [u64; 2],
    ) {
        self.relation_active = job.active;
        if !job.active.into_iter().any(|active| active) {
            self.fail_relations_capture(job.active, replay::cadence::SKIPPED_REASON);
            return;
        }
        let anchor = usize::from(job.active[1]);
        let started = qpc();
        let status_vtable = match base.checked_add(crelation_status::VTABLE_RVA) {
            Some(address) => address as u64,
            None => {
                self.fail_relations_capture(job.active, "module address overflow");
                decode_ticks[anchor] += elapsed(started);
                return;
            }
        };
        let mut opinions_total = Outcome::Rows { len: 0 };
        let mut tuples_total = Outcome::Rows { len: 0 };
        let mut opinion_len = 0usize;
        let mut tuple_len = 0usize;
        // Skipped-element tuple reservations made by the copy are released
        // here: decode counts only emitted rows against the typed capacity,
        // so truncation accounting matches the walk exactly.
        let mut tuple_reserved_released = 0usize;
        if job.reader_err.is_none() && job.country_err.is_none() && job.top_err.is_none() {
            prepare_relation_country_index(&mut self.relation_country_index, &job.countries);
            for meta in &job.metas {
                let (opinion_outcome, tuple_outcome) = decode_owner_relations(
                    base,
                    status_vtable,
                    meta,
                    job,
                    &self.relation_country_index,
                    &mut self.relation_opinions.rows[opinion_len..],
                    &mut self.relation_tuples.rows[tuple_len..],
                    &mut tuple_reserved_released,
                );
                append_relations(&mut opinions_total, opinion_outcome);
                append_relations(&mut tuples_total, tuple_outcome);
                opinion_len = opinions_total.len();
                tuple_len = tuples_total.len();
                if matches!(
                    opinions_total,
                    Outcome::Unavailable { .. } | Outcome::Truncated { .. }
                ) && matches!(
                    tuples_total,
                    Outcome::Unavailable { .. } | Outcome::Truncated { .. }
                ) {
                    // Both partitions already decided; remaining owners cannot
                    // change either outcome, mirroring the walk's early return.
                }
            }
        }
        decode_ticks[anchor] += elapsed(started);
        let started = qpc();
        if let Some(reason) = job.reader_err {
            opinions_total = Outcome::Unavailable { reason };
            tuples_total = Outcome::Unavailable { reason };
        } else if let Some(reason) = job.country_err {
            opinions_total = Outcome::Unavailable { reason };
            tuples_total = Outcome::Unavailable { reason };
        } else if let Some(reason) = job.top_err {
            opinions_total = Outcome::Unavailable { reason };
            tuples_total = Outcome::Unavailable { reason };
        }
        // Reconcile copy-time capacity reservations with decoded rows: skipped
        // war/subject elements never consumed typed rows, so unread counts
        // computed against every element over-count. Recompute truncation
        // from decoded lengths exactly like the walk's counters would.
        let _ = tuple_reserved_released;
        let skipped = Outcome::Unavailable {
            reason: replay::cadence::SKIPPED_REASON,
        };
        self.relation_opinions.outcome = if job.active[0] {
            opinions_total
        } else {
            skipped
        };
        self.relation_tuples.outcome = if job.active[1] { tuples_total } else { skipped };
        decode_ticks[anchor] += elapsed(started);
        self.relations_raw_mode = false;
    }
}

/// Decode one owner's matrix slots + tuple occs into the typed row slices.
/// Returns per-owner (opinions, tuples) outcomes to append; row slices are
/// filled in walk order with identical ordinals and skip semantics.
#[allow(clippy::too_many_arguments)]
fn decode_owner_relations(
    base: usize,
    status_vtable: u64,
    meta: &OwnerMeta,
    job: &RelationsJob,
    country_index: &[usize],
    opinions: &mut [crelation_status::OpinionRow],
    tuples: &mut [crelation_status::TupleRow],
    _released: &mut usize,
) -> (Outcome, Outcome) {
    if let Some(reason) = meta.diplomacy_err {
        let unavailable = Outcome::Unavailable { reason };
        return (unavailable, unavailable);
    }
    // Diplomacy identity spans decode with the identical faults as
    // `diplomacy_root`: scattered copy first, then vtable, then backlink.
    if meta.diplomacy_scattered != 2 {
        let unavailable = Outcome::Unavailable {
            reason: "country diplomacy unreadable",
        };
        return (unavailable, unavailable);
    }
    let expected_diplomacy = match base.checked_add(cdiplomacy_status::VTABLE_RVA) {
        Some(address) => address as u64,
        None => {
            let unavailable = Outcome::Unavailable {
                reason: "module address overflow",
            };
            return (unavailable, unavailable);
        }
    };
    if u64::from_le_bytes(meta.diplomacy_vtable) != expected_diplomacy {
        let unavailable = Outcome::Unavailable {
            reason: "diplomacy vtable mismatch",
        };
        return (unavailable, unavailable);
    }
    if u64::from_le_bytes(meta.diplomacy_backlink) != meta.owner_pointer as u64 {
        let unavailable = Outcome::Unavailable {
            reason: "diplomacy country mismatch",
        };
        return (unavailable, unavailable);
    }
    if let Some(reason) = meta.matrix_err {
        // Matrix/serializable structural faults fail both tables together,
        // exactly like the typed walk's single `Walk` error.
        let unavailable = Outcome::Unavailable { reason };
        return (unavailable, unavailable);
    }
    if !meta.matrix_header_ok {
        let unavailable = Outcome::Unavailable {
            reason: "relation matrix unreadable or invalid",
        };
        return (unavailable, unavailable);
    }
    let matrix = match Array::from_header(meta.matrix_header, 8) {
        Some(matrix) => matrix,
        None => {
            let unavailable = Outcome::Unavailable {
                reason: "relation matrix unreadable or invalid",
            };
            return (unavailable, unavailable);
        }
    };
    if matrix.count <= job.highest_slot {
        let unavailable = Outcome::Unavailable {
            reason: "relation matrix does not cover country ordinals",
        };
        return (unavailable, unavailable);
    }
    let slots = match job
        .slot_pool
        .get(..job.slot_used)
        .and_then(|slots| slots.get(meta.slot_start..meta.slot_start + meta.slot_len))
    {
        Some(slots) => slots,
        None => {
            let unavailable = Outcome::Unavailable {
                reason: "relation matrix slot fields unreadable",
            };
            return (unavailable, unavailable);
        }
    };
    let occs = match job
        .tuples
        .get(meta.tuple_start..meta.tuple_start + meta.tuple_len)
    {
        Some(occs) => occs,
        None => {
            let unavailable = Outcome::Unavailable {
                reason: "serializable relation entry unreadable",
            };
            return (unavailable, unavailable);
        }
    };
    let mut opinion_len = 0usize;
    let mut opinion_unread = 0u32;
    let mut tuple_len = 0usize;
    let mut tuple_unread = 0u32;
    let mut occ_index = 0usize;
    // Slot order is matrix order; tuple occs are grouped per slot in vector
    // order. Walk the slots and consume occs in the same order the copy
    // recorded them.
    for (slot_offset, slot_bytes) in slots
        .chunks_exact(crelation_status::RECORD_BYTES)
        .enumerate()
    {
        let record: &[u8; crelation_status::RECORD_BYTES] = match slot_bytes.try_into() {
            Ok(record) => record,
            Err(_) => {
                let unavailable = Outcome::Unavailable {
                    reason: "relation matrix slot fields unreadable",
                };
                return (unavailable, unavailable);
            }
        };
        let decoded = match crelation_status::decode_slot(status_vtable, record) {
            Ok(decoded) => decoded,
            Err(reason) => {
                let unavailable = Outcome::Unavailable { reason };
                return (unavailable, unavailable);
            }
        };
        let target = match relations_lookup(job, country_index, decoded.target_country) {
            Ok(target) => target,
            Err(reason) => {
                let unavailable = Outcome::Unavailable { reason };
                return (unavailable, unavailable);
            }
        };
        // Matrix slot index mirrors the walk's `index + offset` census.
        let slot_index = 1 + slot_offset as u32;
        if target.canonical_slot != slot_index {
            let unavailable = Outcome::Unavailable {
                reason: "relation status target differs from matrix slot",
            };
            return (unavailable, unavailable);
        }
        if job.active[0] && decoded.opinion != 0 {
            if opinion_len == opinions.len() {
                opinion_unread = opinion_unread.saturating_add(1);
            } else if let Some(slot) = opinions.get_mut(opinion_len) {
                *slot = crelation_status::OpinionRow {
                    owner_country: meta.owner_id,
                    target_country: decoded.target_country,
                    opinion: decoded.opinion,
                };
                opinion_len += 1;
            }
        }
        if !job.active[1] {
            continue;
        }
        let serializable = match Array::from_header(decoded.serializable_header, 8) {
            Some(array) => array,
            None => {
                let unavailable = Outcome::Unavailable {
                    reason: "serializable relation vector invalid",
                };
                return (unavailable, unavailable);
            }
        };
        let mut first = 0u32;
        while first < serializable.count {
            let chunk = (serializable.count - first).min(crelation_status::BATCH as u32) as usize;
            let mut auth = [(0_u32, false); crelation_status::BATCH];
            // Match read_tuples: authenticate the complete owned chunk before
            // any endpoint/rest interpretation. A terminal occurrence can end
            // this prefix early and retains its exact native or semantic error.
            for (offset, decoded_auth) in auth[..chunk].iter_mut().enumerate() {
                let ordinal = first + offset as u32;
                let occ = match occs.get(occ_index + offset) {
                    Some(occ) => occ,
                    None => {
                        let unavailable = Outcome::Unavailable {
                            reason: "serializable relation entry unreadable",
                        };
                        return (unavailable, unavailable);
                    }
                };
                if occ.ordinal != ordinal
                    || occ.owner_id != meta.owner_id
                    || occ.target_country != decoded.target_country
                {
                    let unavailable = Outcome::Unavailable {
                        reason: "serializable relation entry unreadable",
                    };
                    return (unavailable, unavailable);
                }
                if let Some(reason) = occ.auth_read.reason() {
                    let unavailable = Outcome::Unavailable { reason };
                    return (unavailable, unavailable);
                }
                match crelation_status::decode_auth(base, &occ.auth) {
                    Ok(value) => *decoded_auth = (value.type_id, value.skipped),
                    Err(reason) => {
                        let unavailable = Outcome::Unavailable { reason };
                        return (unavailable, unavailable);
                    }
                }
            }
            for (offset, &(type_id, skipped)) in auth[..chunk].iter().enumerate() {
                let ordinal = first + offset as u32;
                let occ = &occs[occ_index + offset];
                if skipped {
                    continue;
                }
                if let Some(reason) = occ.rest_read.reason() {
                    let unavailable = Outcome::Unavailable { reason };
                    return (unavailable, unavailable);
                }
                let rest = crelation_status::decode_rest(&occ.rest);
                let first = match relations_lookup(job, country_index, rest.first_country) {
                    Ok(first) => first,
                    Err(reason) => {
                        let unavailable = Outcome::Unavailable { reason };
                        return (unavailable, unavailable);
                    }
                };
                if first.canonical_slot != meta.owner_canonical {
                    let unavailable = Outcome::Unavailable {
                        reason: "serializable relation is not owned by its first endpoint",
                    };
                    return (unavailable, unavailable);
                }
                if let Err(reason) = relations_lookup(job, country_index, rest.second_country) {
                    let unavailable = Outcome::Unavailable { reason };
                    return (unavailable, unavailable);
                }
                if tuple_len == tuples.len() {
                    tuple_unread = tuple_unread.saturating_add(1);
                } else if let Some(slot) = tuples.get_mut(tuple_len) {
                    *slot = crelation_status::TupleRow {
                        owner_country: meta.owner_id,
                        target_country: decoded.target_country,
                        type_id,
                        ordinal,
                        first_country: rest.first_country,
                        second_country: rest.second_country,
                        start_hour: rest.start_hour,
                        end_hour: rest.end_hour,
                        cancel: rest.cancel,
                        retired: rest.retired,
                    };
                    tuple_len += 1;
                }
            }
            occ_index += chunk;
            first += chunk as u32;
        }
    }
    if occ_index != occs.len() {
        let unavailable = Outcome::Unavailable {
            reason: "serializable relation entry unreadable",
        };
        return (unavailable, unavailable);
    }
    let opinions = if opinion_unread == 0 {
        Outcome::Rows { len: opinion_len }
    } else {
        Outcome::Truncated {
            len: opinion_len,
            unread_rows: opinion_unread,
        }
    };
    let tuples = if tuple_unread == 0 {
        Outcome::Rows { len: tuple_len }
    } else {
        Outcome::Truncated {
            len: tuple_len,
            unread_rows: tuple_unread,
        }
    };
    (opinions, tuples)
}

fn append_relations(total: &mut Outcome, next: Outcome) {
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
                    reason: "relations omitted row count overflow",
                },
            };
        }
    }
}

// ===========================================================================
// Hourly capitulation raw-copy split (Capitulation).
//
// The game thread copies raw bytes only: one shared country census plus, per
// country, the diplomacy pointer and one bulk diplomacy object enclosing the
// capitulation scalars. No row is decoded on the game thread. The single FIFO
// decode worker replays `cdiplomacy_status::walk_capitulation` against the
// recorded spans (`crate::raw::Recorded`), so rows, order, truncation and
// failure reasons are identical by construction (see `crate::raw` for the
// mirror rules).
//
// The mirror traverses structurally and records every discoverable span,
// never aborting except on overflow: validation, precedence and error scope
// are local parses, so the replay derives every outcome from the same
// recorded spans and later extras are harmless.
// ===========================================================================

/// Fault-attribution mask: the single capitulation table. The shared census
/// carries the same bit, so tearing the census blocks the check like the
/// pilot's conservative rule.
const M_CAPITULATION: u32 = 0x01;

/// One hour of raw capitulation payload. Flat pools recycle capacities and
/// the steady state allocates on neither thread; growth happens on the
/// decode side only (see `crate::decode::recycle_capitulation_job`).
#[derive(Debug, Default)]
pub(crate) struct CapitulationJob {
    pub log: RawLog<1>,
    pub game_state: usize,
}

impl CapitulationJob {
    pub fn new() -> Self {
        Self {
            // Late-campaign worst case is 439 countries with one diplomacy
            // object each (~0.9 KiB per country); both pools start above
            // that so steady state never overflows.
            log: RawLog::new(1 << 15, 4 << 20),
            game_state: 0,
        }
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    /// Double every pool off the game thread until copies fit. Converges like
    /// the typed capacity growth; overflowed hours fall back to the typed
    /// walk in the meantime, so sizing is a pure performance knob.
    pub fn grow(&mut self) {
        self.log.grow();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct CapitulationCopyReport {
    /// False when staging overflowed: the caller falls back to the typed walk
    /// (identical rows through the existing path) and enqueues no payload.
    pub raw_ok: bool,
    pub copy_ticks: [u64; 1],
    pub raw_bytes: [u64; 1],
}

/// Game-thread copy parameters, bundled to keep the tick entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CapitulationCopyContext {
    pub base: usize,
    pub game_state: usize,
}

fn cap_pool_u64(log: &RawLog<1>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn cap_pool_u32(log: &RawLog<1>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
fn capspan(
    log: &mut RawLog<1>,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// Bulk variant for contiguous diplomacy objects. Enclosed sub-reads serve
/// from the bulk span at replay.
fn capsbulk(
    log: &mut RawLog<1>,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take_bulk(memory, addr?, len, mask)
}

/// Mirror of a link-table read with local bounds checks.
fn cap_link(
    log: &mut RawLog<1>,
    memory: &impl Memory,
    links: &Array,
    countries: u32,
    raw: u32,
) -> Option<Option<u32>> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = capspan(
        log,
        memory,
        links.data.checked_add(offset),
        4,
        M_CAPITULATION,
    )?;
    let slot = cap_pool_u32(log, start);
    if slot == 0 {
        return Some(None);
    }
    if slot >= countries {
        return None;
    }
    Some(Some(slot))
}

/// One structural copy pass: shared census plus, per country, the diplomacy
/// pointer and one bulk diplomacy object (0x360 bytes enclosing 0x188..0x350
/// capitulation scalars). Stops at the first typed abort point; later extras
/// are harmless.
fn copy_capitulation_pass(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    log: &mut RawLog<1>,
) {
    use engine::cgame_state;
    let _reads = read_accounting::table(TABLES[0]);
    log.has_bulk = true;
    let Some(countries_start) = capspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRIES),
        16,
        M_CAPITULATION,
    ) else {
        return;
    };
    let Some(links_start) = capspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRY_LINKS),
        16,
        M_CAPITULATION,
    ) else {
        return;
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries_start..countries_start + 16]);
    let countries = match Array::from_header(raw, 8) {
        Some(a) => a,
        None => return,
    };
    raw.copy_from_slice(&log.bytes[links_start..links_start + 16]);
    let links = match Array::from_header(raw, 4) {
        Some(a) => a,
        None => return,
    };
    if countries.count < 2 || links.count == 0 {
        return;
    }
    let country_vtable = match base.checked_add(ccountry::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    for slot in 1..countries.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|o| countries.data.checked_add(o));
        let Some(pointer_start) = capspan(log, memory, pointer_addr, 8, M_CAPITULATION) else {
            return;
        };
        let pointer = match usize::try_from(cap_pool_u64(log, pointer_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        let Some(vtable_start) = capspan(log, memory, Some(pointer), 8, M_CAPITULATION) else {
            return;
        };
        if cap_pool_u64(log, vtable_start) != country_vtable {
            return;
        }
        let Some(id_start) = capspan(
            log,
            memory,
            pointer.checked_add(ccountry::ID),
            4,
            M_CAPITULATION,
        ) else {
            return;
        };
        let id = cap_pool_u32(log, id_start);
        // Authenticate through the link table like `ccountry::Reader`.
        let canonical = match cap_link(log, memory, &links, countries.count, id) {
            Some(Some(canonical)) => canonical,
            _ => return,
        };
        let canonical_slot = match cap_link(log, memory, &links, countries.count, slot) {
            Some(Some(canonical_slot)) => canonical_slot,
            _ => return,
        };
        if canonical_slot != canonical {
            return;
        }
        // Per-country diplomacy pointer plus one bulk object.
        let Some(diplo_start) = capspan(
            log,
            memory,
            pointer.checked_add(ccountry::DIPLOMACY_STATUS),
            8,
            M_CAPITULATION,
        ) else {
            return;
        };
        let diplomacy = match usize::try_from(cap_pool_u64(log, diplo_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        let _ = capspan(log, memory, Some(diplomacy), 8, M_CAPITULATION);
        let _ = capspan(
            log,
            memory,
            diplomacy.checked_add(cdiplomacy_status::COUNTRY),
            8,
            M_CAPITULATION,
        );
        let _ = capspan(
            log,
            memory,
            diplomacy.checked_add(cdiplomacy_status::LEGITIMACY),
            12,
            M_CAPITULATION,
        );
        let _ = capspan(
            log,
            memory,
            diplomacy.checked_add(cdiplomacy_status::LAST_SURRENDER_HOUR),
            4,
            M_CAPITULATION,
        );
        let _ = capspan(
            log,
            memory,
            diplomacy.checked_add(cdiplomacy_status::FACTION_JOIN_HOUR),
            4,
            M_CAPITULATION,
        );
        let _ = capspan(
            log,
            memory,
            diplomacy.checked_add(cdiplomacy_status::CAPITULATED),
            1,
            M_CAPITULATION,
        );
        let _ = capspan(
            log,
            memory,
            diplomacy.checked_add(cdiplomacy_status::CAPITULATED_HOUR),
            4,
            M_CAPITULATION,
        );
        // Bulk superset encloses the scattered reads above for replay
        // containment; exact spans above preserve fault attribution.
        let _ = capsbulk(log, memory, Some(diplomacy), 0x360, M_CAPITULATION);
        if log.overflowed {
            return;
        }
    }
}

/// Hourly acquisition into one owned job. Staging overflow retains the existing
/// typed fallback and enqueues no payload.
pub(crate) fn copy_capitulation_refresh(
    memory: &impl Memory,
    ctx: CapitulationCopyContext,
    job: &mut CapitulationJob,
) -> CapitulationCopyReport {
    let started = qpc();
    job.clear();
    job.game_state = ctx.game_state;
    copy_capitulation_pass(memory, ctx.base, ctx.game_state, &mut job.log);
    CapitulationCopyReport {
        raw_ok: !job.log.overflowed,
        copy_ticks: [elapsed(started)],
        raw_bytes: job.log.raw_bytes,
    }
}

impl Captured {
    /// Terminal failure of accepted Capitulation acquisition without retrying
    /// live memory or invalidating the other Intel tables.
    pub(crate) fn fail_capitulation_capture(&mut self, reason: &'static str) {
        self.capitulation.outcome = Outcome::Unavailable { reason };
        self.capitulation_raw_mode = false;
    }

    /// Decode-worker interpretation of a raw hourly payload into the pooled
    /// typed rows. Outcomes match the typed walk exactly because the
    /// identical walk runs against the recorded spans. Clears
    /// `capitulation_raw_mode` so the next capture starts clean.
    pub(crate) fn decode_capitulation_payload(
        &mut self,
        base: usize,
        job: &CapitulationJob,
        decode_ticks: &mut [u64; 1],
    ) {
        let recorded = crate::raw::Recorded::build(&job.log);
        let started = qpc();
        self.capitulation.outcome = bounded(
            cdiplomacy_status::walk_capitulation(
                &recorded,
                base,
                job.game_state,
                &mut self.capitulation.rows,
            ),
            self.capitulation.rows.len(),
        );
        decode_ticks[0] += elapsed(started);
        self.capitulation_raw_mode = false;
    }
}

// Test-only re-export so the decode worker tests can reuse the same harness.
#[cfg(test)]
pub(crate) use tests::relations_test_kit;

#[cfg(test)]
pub(crate) fn intel_shadow_fixture(value: i64) -> Captured {
    let capacities = Capacities {
        rows: [2; N],
        intel_scratch: 0,
        subject_names: 0,
        autonomy_names: 0,
        variable_entries: 0,
        variable_names: 0,
        tokens: 0,
    };
    let mut capture = Captured::with_capacities(&capacities);
    for (index, row) in capture.country_intel.rows.iter_mut().enumerate() {
        *row = country_intel_row::Row {
            owner_country: 7,
            target_index: index as u32,
            civilian: value,
            army: i64::MIN,
            navy: i64::MAX,
            air: -1,
        };
    }
    for (index, row) in capture.flags.rows.iter_mut().enumerate() {
        *row = ScopedFlag {
            scope_kind: COUNTRY_SCOPE,
            scope_id: 7,
            flag: cscript_flag::Row {
                token: index as u32,
                set_hour: -3,
                value: i16::MIN,
                remaining_days: i16::MAX,
            },
        };
    }
    capture.flags.outcome = Outcome::Rows { len: 2 };
    capture.country_intel.outcome = Outcome::Rows { len: 2 };
    capture
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegated_capture_preserves_residual_results_and_obeys_raw_gates_and_cadence() {
        struct GuardedReads {
            allowed: bool,
            calls: std::cell::Cell<usize>,
        }
        impl Memory for GuardedReads {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                assert!(
                    self.allowed,
                    "delegated raw or skipped tables must not read again"
                );
                self.calls.set(self.calls.get() + 1);
                false
            }
        }
        for (capitulation_raw, relations_raw, hour, force_full) in [
            (true, true, 1, true),
            (true, false, 0, false),
            (true, false, 1, false),
            (false, true, 2, false),
            (false, false, 0, true),
        ] {
            let mut captured = intel_shadow_fixture(42);
            let active = [
                replay::cadence::should_walk(TableId::RelationOpinions, hour, force_full),
                replay::cadence::should_walk(TableId::RelationTuples, hour, force_full),
            ];
            captured.set_relation_activity(active);
            captured.capitulation_raw_mode = capitulation_raw;
            captured.relations_raw_mode = relations_raw;
            let outcomes = captured.outcomes();
            let flags = captured.flags.rows.clone();
            let country_intel = captured.country_intel.rows.clone();
            let allocated = captured.allocated_bytes();
            let context = CaptureContext {
                base: 0,
                game_state: 0x1000,
                hour,
                force_full,
            };
            let relation_walk = !relations_raw && active.into_iter().any(|active| active);
            let memory = GuardedReads {
                allowed: !capitulation_raw || relation_walk,
                calls: std::cell::Cell::new(0),
            };
            let report = captured.capture_delegated(&memory, context);
            assert_eq!(memory.calls.get() != 0, memory.allowed);
            assert_eq!(report.ticks[13], 0, "tuples owns the shared relation cost");
            if !relation_walk {
                assert_eq!(report.ticks[14], 0);
                if !relations_raw {
                    for slot in [13, 14] {
                        if active[slot - 13] {
                            continue;
                        }
                        assert_eq!(
                            captured.outcomes()[slot],
                            Outcome::Unavailable {
                                reason: replay::cadence::SKIPPED_REASON,
                            }
                        );
                    }
                }
            }
            for slot in (1..=12).chain(std::iter::once(15)) {
                assert_eq!(captured.outcomes()[slot], outcomes[slot]);
                assert_eq!(report.ticks[slot], 0);
            }
            assert_eq!(captured.flags.rows, flags);
            assert_eq!(captured.country_intel.rows, country_intel);
            assert_eq!(captured.capitulation_raw_mode, capitulation_raw);
            assert_eq!(captured.relations_raw_mode, relations_raw);
            assert_eq!(captured.allocated_bytes(), allocated);
        }
    }

    #[test]
    fn residual_capture_and_failure_preserve_delegated_slots() {
        struct Unreadable;
        impl Memory for Unreadable {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                false
            }
        }
        let capacities = Capacities {
            rows: [1; N],
            intel_scratch: 32,
            subject_names: 0,
            autonomy_names: 0,
            variable_entries: 0,
            variable_names: 0,
            tokens: 0,
        };
        for raw_modes in 0..4 {
            let mut captured = Captured::with_capacities(&capacities);
            captured.capitulation.outcome = Outcome::Rows { len: 1 };
            captured.relation_opinions.outcome = Outcome::Rows { len: 1 };
            captured.relation_tuples.outcome = Outcome::Rows { len: 1 };
            captured.capitulation_raw_mode = raw_modes & 1 != 0;
            captured.relations_raw_mode = raw_modes & 2 != 0;
            let before = captured.outcomes();
            let allocated = captured.allocated_bytes();
            let context = CaptureContext {
                base: 0,
                game_state: 0x1000,
                hour: raw_modes,
                force_full: raw_modes == 0,
            };
            let report = captured.capture_residual(&Unreadable, context);
            assert_eq!(report.ticks[10], 0, "operations owns the shared read cost");
            for slot in [0, 13, 14] {
                assert_eq!(captured.outcomes()[slot], before[slot]);
                assert_eq!(report.ticks[slot], 0);
            }
            let report = captured.fail_residual_capture(context, "residual worker failed");
            assert_eq!(report.ticks, [0; N]);
            for slot in (1..=12).chain(std::iter::once(15)) {
                assert_eq!(
                    captured.outcomes()[slot],
                    Outcome::Unavailable {
                        reason: if replay::cadence::should_walk(
                            TABLES[slot],
                            context.hour,
                            context.force_full
                        ) {
                            "residual worker failed"
                        } else {
                            replay::cadence::SKIPPED_REASON
                        },
                    }
                );
            }
            for slot in [0, 13, 14] {
                assert_eq!(captured.outcomes()[slot], before[slot]);
            }
            assert_eq!(captured.capitulation_raw_mode, raw_modes & 1 != 0);
            assert_eq!(captured.relations_raw_mode, raw_modes & 2 != 0);
            assert_eq!(captured.allocated_bytes(), allocated);
        }
    }

    fn memo_inputs(
        capture: &Captured,
        shadow: &mut IntelShadow,
        branch: BranchId,
    ) -> [TableInput; N] {
        capture.table_inputs(
            None,
            &mut ScriptVariableComparison::default(),
            Some(&mut IntelConversion::new(branch, shadow)),
        )
    }

    #[test]
    fn residual_daily_gates_keep_skips_and_zero_native_work_through_faults() {
        struct Unreadable;
        impl Memory for Unreadable {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                false
            }
        }
        let memory = engine::memory::CountedMemory::new(Unreadable);
        let mut captured = Captured::with_capacities(&Capacities::default());
        for hour in 0..24 {
            let accounting = read_accounting::Capture::new();
            captured.capture_residual(
                &memory,
                CaptureContext {
                    base: 0,
                    game_state: 0x1000,
                    hour,
                    force_full: false,
                },
            );
            let counts = accounting.finish();
            for index in (1..=12).chain(std::iter::once(15)) {
                if !replay::cadence::should_walk(TABLES[index], hour, false) {
                    assert_eq!(
                        captured.outcomes()[index],
                        Outcome::Unavailable {
                            reason: replay::cadence::SKIPPED_REASON
                        }
                    );
                    assert_eq!(counts[TABLES[index].index()].spans, 0);
                }
            }
        }
    }

    fn shared(input: &TableInput) -> &Arc<TableRows> {
        let TableInput::SharedComplete(rows) = input else {
            panic!("expected complete shared rows")
        };
        rows
    }

    #[test]
    fn intel_shadow_exact_hits_own_rows_and_account_actual_capacities() {
        let mut capture = intel_shadow_fixture(42);
        let mut shadow = IntelShadow::default();
        let original = capture.tables();
        let first = memo_inputs(&capture, &mut shadow, 1);
        let second = memo_inputs(&capture, &mut shadow, 1);
        for slot in [7, 15] {
            assert!(Arc::ptr_eq(shared(&first[slot]), shared(&second[slot])));
            assert_eq!(
                TableView::Complete((**shared(&first[slot])).clone()),
                original[slot]
            );
        }
        let stats = shadow.statistics();
        assert!(stats.iter().all(|stat| stat.hits == 1 && stat.misses == 1));
        assert_eq!(
            stats[0].row_capacity_bytes,
            shadow.flags.rows.capacity() * size_of::<ScopedFlag>()
        );
        assert_eq!(
            stats[1].row_capacity_bytes,
            shadow.country_intel.rows.capacity() * size_of::<country_intel_row::Row>()
        );
        assert_eq!(
            stats[0].replay_capacity_bytes,
            std::mem::size_of_val(shared(&first[7]).rows::<tables::ScriptFlags>().unwrap())
        );
        // Unused capacity is not data. Only the current successful prefix is compared.
        capture.flags.rows.push(ScopedFlag::default());
        capture
            .country_intel
            .rows
            .push(country_intel_row::Row::default());
        let third = memo_inputs(&capture, &mut shadow, 1);
        assert!(Arc::ptr_eq(shared(&first[7]), shared(&third[7])));
        assert!(Arc::ptr_eq(shared(&first[15]), shared(&third[15])));
        drop(capture);
        assert_eq!(shadow.flags.rows.len(), 2);
        assert_eq!(shadow.country_intel.rows.len(), 2);
    }

    #[test]
    fn intel_shadow_compares_every_native_field_and_order_before_reuse() {
        for slot in [7, 15] {
            for change in 0..8 {
                let mut capture = intel_shadow_fixture(42);
                let mut shadow = IntelShadow::default();
                let first = memo_inputs(&capture, &mut shadow, 1);
                if slot == 7 {
                    let row = &mut capture.flags.rows[0];
                    match change {
                        0 => row.scope_kind = STATE_SCOPE,
                        1 => row.scope_id += 1,
                        2 => row.flag.token += 10,
                        3 => row.flag.set_hour += 1,
                        4 => row.flag.value += 1,
                        5 => row.flag.remaining_days -= 1,
                        6 => capture.flags.rows.swap(0, 1),
                        _ => capture.flags.outcome = Outcome::Rows { len: 1 },
                    }
                } else {
                    let row = &mut capture.country_intel.rows[0];
                    match change {
                        0 => row.owner_country += 1,
                        1 => row.target_index += 10,
                        2 => row.civilian += 1,
                        3 => row.army += 1,
                        4 => row.navy -= 1,
                        5 => row.air += 1,
                        6 => capture.country_intel.rows.swap(0, 1),
                        _ => capture.country_intel.outcome = Outcome::Rows { len: 1 },
                    }
                }
                let expected = capture.tables();
                let next = memo_inputs(&capture, &mut shadow, 1);
                assert!(
                    !Arc::ptr_eq(shared(&first[slot]), shared(&next[slot])),
                    "slot {slot}, change {change}"
                );
                assert_eq!(
                    TableView::Complete((**shared(&next[slot])).clone()),
                    expected[slot]
                );
                assert!(Arc::ptr_eq(
                    shared(&first[22 - slot]),
                    shared(&next[22 - slot])
                ));
            }
        }
    }

    #[test]
    fn intel_shadow_current_faults_truncation_and_duplicate_keys_clear_only_affected_table() {
        for slot in [7, 15] {
            for fault in 0..5 {
                let mut capture = intel_shadow_fixture(42);
                let mut shadow = IntelShadow::default();
                let first = memo_inputs(&capture, &mut shadow, 1);
                let outcome = match fault {
                    0 => Outcome::Unavailable {
                        reason: "fixture unreadable",
                    },
                    1 => Outcome::Truncated {
                        len: 1,
                        unread_rows: 3,
                    },
                    2 => Outcome::Rows { len: 3 },
                    3 => Outcome::Truncated {
                        len: 1,
                        unread_rows: 0,
                    },
                    _ => Outcome::Rows { len: 2 },
                };
                if slot == 7 {
                    capture.flags.outcome = outcome;
                    if fault == 4 {
                        capture.flags.rows[1] = capture.flags.rows[0];
                    }
                } else {
                    capture.country_intel.outcome = outcome;
                    if fault == 4 {
                        capture.country_intel.rows[1] = capture.country_intel.rows[0];
                    }
                }
                let expected = capture.tables();
                let next = memo_inputs(&capture, &mut shadow, 1);
                assert_eq!(next[slot], TableInput::Sampled(expected[slot].clone()));
                assert!(matches!(
                    next[slot],
                    TableInput::Sampled(
                        TableView::Unavailable { .. } | TableView::Truncated { .. }
                    )
                ));
                assert!(Arc::ptr_eq(
                    shared(&first[22 - slot]),
                    shared(&next[22 - slot])
                ));
                assert_eq!(
                    shadow.statistics()[usize::from(slot == 15)].replay_capacity_bytes,
                    0
                );
                let recovered = memo_inputs(&intel_shadow_fixture(42), &mut shadow, 1);
                assert!(!Arc::ptr_eq(shared(&first[slot]), shared(&recovered[slot])));
            }
        }
    }

    #[test]
    fn intel_shadow_branch_changes_and_empty_success_never_resurrect_old_rows() {
        let mut capture = intel_shadow_fixture(42);
        let mut shadow = IntelShadow::default();
        let first = memo_inputs(&capture, &mut shadow, 1);
        let next = memo_inputs(&capture, &mut shadow, 2);
        for slot in [7, 15] {
            assert!(!Arc::ptr_eq(shared(&first[slot]), shared(&next[slot])));
        }
        capture.flags.outcome = Outcome::Rows { len: 0 };
        capture.country_intel.outcome = Outcome::Rows { len: 0 };
        let empty = memo_inputs(&capture, &mut shadow, 2);
        let repeated = memo_inputs(&capture, &mut shadow, 2);
        for slot in [7, 15] {
            assert!(Arc::ptr_eq(shared(&empty[slot]), shared(&repeated[slot])));
            assert!(!Arc::ptr_eq(shared(&next[slot]), shared(&empty[slot])));
        }
    }

    #[test]
    fn relations_job_capacity_accounting_tracks_all_vectors_through_growth_clear_and_drop() {
        use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
        static LOCAL: Tracker = Tracker::new();
        let mut job = RelationsJob::default();
        job.countries.reserve_exact(3);
        job.links.reserve_exact(3);
        job.metas.reserve_exact(3);
        job.slot_pool.reserve_exact(3);
        job.tuples.reserve_exact(3);
        job.capacity_charge = Charge::empty_on(&LOCAL);
        job.update_capacity_charge();
        let expected = |job: &RelationsJob| {
            vector_bytes(&job.countries)
                + vector_bytes(&job.links)
                + vector_bytes(&job.metas)
                + vector_bytes(&job.slot_pool)
                + vector_bytes(&job.tuples)
        };
        let initial = LOCAL.snapshot().current.payload_vectors;
        assert_eq!(initial, expected(&job));
        job.grow();
        let grown = LOCAL.snapshot().current.payload_vectors;
        assert!(grown > initial);
        assert_eq!(grown, expected(&job));
        job.clear();
        assert_eq!(LOCAL.snapshot().current.payload_vectors, grown);
        drop(job);
        assert_eq!(LOCAL.snapshot().current, Capacity::default());
    }
    use engine::memory::SliceMemory;
    use replay::rows::TableSchema;

    fn small() -> (Capacities, Captured) {
        let capacities = Capacities {
            rows: [3; N],
            intel_scratch: 2 * engine::country_intel_row::STRIDE,
            subject_names: 8,
            autonomy_names: 8,
            variable_entries: 4,
            variable_names: 8,
            tokens: 8,
        };
        let captured = Captured::with_capacities(&capacities);
        (capacities, captured)
    }

    fn complete<S: TableSchema>(view: &TableView) -> &[S::Row] {
        let TableView::Complete(rows) = view else {
            panic!("expected complete table");
        };
        rows.rows::<S>().unwrap()
    }

    fn variable_fixture(
        scopes: &mut [cvariables::ScopeRow],
        entries: &mut [cvariables::Row],
        names: &mut [u8],
    ) -> cvariables::ScopeWalk {
        scopes[0] = cvariables::ScopeRow {
            scope: cvariables::Scope::Country(u32::MAX),
            entries_start: 0,
            entries_len: 2,
        };
        entries[0] = cvariables::Row {
            name_start: 0,
            name_len: 2,
            value_raw: i64::MIN,
        };
        entries[1] = cvariables::Row {
            name_start: 2,
            name_len: 1,
            value_raw: i64::MAX,
        };
        names[..3].copy_from_slice(&[0xff, 0, b'x']);
        cvariables::ScopeWalk {
            outcome: Outcome::Rows { len: 1 },
            entries_len: 2,
            required_entries: 2,
            bytes_len: 3,
            required_bytes: 3,
        }
    }

    #[test]
    fn variable_capture_reads_once_and_preserves_scope_values_and_literal_bytes() {
        let (_, mut capture) = small();
        let mut calls = 0;
        capture_variables(
            &mut capture.variables,
            &mut capture.variable_entries,
            &mut capture.variable_names,
            |scopes, entries, names| {
                calls += 1;
                variable_fixture(scopes, entries, names)
            },
        );
        assert_eq!(calls, 1);
        assert_eq!(capture.variables.outcome, Outcome::Rows { len: 1 });
        assert_eq!(capture.variable_entries.len, 2);
        assert_eq!(
            &capture.variable_names.first[..capture.variable_names.len],
            &[0xff, 0, b'x']
        );
        assert!(valid_variables(
            capture.variables.outcome,
            &capture.variables.rows,
            &capture.variable_entries,
            &capture.variable_names
        ));
    }

    #[test]
    fn variable_primary_truncation_grows_both_arenas_and_recovers() {
        let (mut capacities, mut capture) = small();
        capture_variables(
            &mut capture.variables,
            &mut capture.variable_entries,
            &mut capture.variable_names,
            |scopes, entries, names| {
                let mut result = variable_fixture(scopes, entries, names);
                result.outcome = Outcome::Truncated {
                    len: 1,
                    unread_rows: 1,
                };
                result.required_entries = 17;
                result.required_bytes = 65;
                result
            },
        );
        assert_eq!(
            capture.outcomes()[8],
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        capture.grow(&mut capacities);
        assert!(capacities.variable_entries >= 17);
        assert!(capacities.variable_names >= 65);
        capture.reset();
        let mut calls = 0;
        capture_variables(
            &mut capture.variables,
            &mut capture.variable_entries,
            &mut capture.variable_names,
            |scopes, entries, names| {
                calls += 1;
                variable_fixture(scopes, entries, names)
            },
        );
        assert_eq!(calls, 1);
        assert_eq!(capture.variables.outcome, Outcome::Rows { len: 1 });
    }

    #[test]
    fn replay_conversion_retains_native_widths_nullability_literal_keys_and_vector_order() {
        let (_, mut capture) = small();
        capture.capitulation.outcome = Outcome::Rows { len: 1 };
        capture.capitulation.rows[0] = cdiplomacy_status::CapitulationRow {
            country: u32::MAX,
            capitulated: 0xfe,
            capitulated_hour: i32::MIN,
            last_surrender_hour: i32::MAX,
            faction_join_hour: -1,
            legitimacy_raw: i64::MIN,
            exile_army_leaders: i32::MIN,
        };
        capture.operatives.outcome = Outcome::Rows { len: 1 };
        capture.operatives.rows[0] = coperative_leader::Row {
            id: u64::MAX,
            owner_country: u32::MAX,
            roster_country: u32::MAX - 1,
            roster_kind: u8::MAX,
            detected: 0xfe,
            assignment: u32::MAX,
            experience_raw: i64::MIN,
            skill: Some(i32::MIN),
            capturer_country: 0,
            captured_hour: i32::MAX,
            action_variant: None,
        };
        capture.networks.outcome = Outcome::Rows { len: 2 };
        capture.networks.rows[0] = ccountry_intel_network::Row {
            owner_country: 1,
            target_country: 2,
            state_id: 3,
            strength_raw: None,
            subnetwork_selector: i32::MIN,
        };
        capture.networks.rows[1] = ccountry_intel_network::Row {
            owner_country: 1,
            target_country: 2,
            state_id: 4,
            strength_raw: Some(i64::MIN),
            subnetwork_selector: 0,
        };
        capture.operations.outcome = Outcome::Rows { len: 1 };
        capture.operations.rows[0] = coperation_instance::Row {
            id: u64::MAX,
            definition_token: u32::MAX,
            preparation_current: i64::MIN,
            ..Default::default()
        };
        capture.operation_tokens.outcome = Outcome::Rows { len: 2 };
        capture.operation_tokens.rows[0] = ccountry_operation_token_manager::Row {
            owner: u32::MAX,
            target: 2,
            tokens_start: 0,
            tokens_len: 4,
        };
        capture.operation_tokens.rows[1] = ccountry_operation_token_manager::Row {
            owner: u32::MAX,
            target: 0,
            tokens_start: 4,
            tokens_len: 0,
        };
        capture.tokens.first[..4].copy_from_slice(&[u32::MAX, 2, 2, 0]);
        capture.tokens.len = 4;
        capture.tokens.required = 4;
        capture.autonomy.outcome = Outcome::Rows { len: 1 };
        capture.autonomy.rows[0] = ccurrent_autonomy_status::Row {
            subject: 9,
            state: Name { start: 0, len: 1 },
            previous: None,
            next: Some(Name { start: 1, len: 0 }),
            ..Default::default()
        };
        capture.autonomy_names.first[0] = 0xff;
        capture.autonomy_names.len = 1;
        capture.autonomy_names.required = 1;
        capture.variables.outcome = Outcome::Rows { len: 1 };
        capture.variables.rows[0] = cvariables::ScopeRow {
            scope: cvariables::Scope::Country(u32::MAX),
            entries_start: 0,
            entries_len: 2,
        };
        capture.variable_entries.first[0] = cvariables::Row {
            name_start: 0,
            name_len: 2,
            value_raw: i64::MIN,
        };
        capture.variable_entries.first[1] = cvariables::Row {
            name_start: 2,
            name_len: 1,
            value_raw: i64::MAX,
        };
        capture.variable_entries.len = 2;
        capture.variable_entries.required = 2;
        capture.variable_names.first[..3].copy_from_slice(&[0xff, 0, b'x']);
        capture.variable_names.len = 3;
        capture.variable_names.required = 3;

        let views = capture.tables();
        let networks = complete::<tables::IntelNetworks>(&views[6]);
        assert_eq!(
            (networks[0].subnetwork_selector, networks[0].strength_raw),
            (i32::MIN, None)
        );
        assert_eq!(
            (networks[1].subnetwork_selector, networks[1].strength_raw),
            (0, Some(i64::MIN))
        );
        let operation = &complete::<tables::Operations>(&views[9])[0];
        assert_eq!(
            (
                operation.id,
                operation.definition_token,
                operation.preparation_current
            ),
            (u64::MAX, u32::MAX, i64::MIN)
        );
        let capitulation = &complete::<tables::Capitulation>(&views[0])[0];
        assert_eq!(
            (
                capitulation.country,
                capitulation.capitulated,
                capitulation.legitimacy_raw
            ),
            (u32::MAX, 0xfe, i64::MIN)
        );
        let operative = &complete::<tables::OperativeMemberships>(&views[4])[0];
        assert_eq!(
            (
                operative.id,
                operative.roster_country,
                operative.roster_kind
            ),
            (u64::MAX, u32::MAX - 1, u8::MAX)
        );
        assert_eq!(
            (operative.skill, operative.action_variant),
            (Some(i32::MIN), None)
        );
        let autonomy = &complete::<tables::Autonomy>(&views[2])[0];
        assert_eq!(
            (autonomy.previous.as_deref(), autonomy.next.as_deref()),
            (None, Some(&[][..]))
        );
        let tokens = complete::<tables::OperationTokens>(&views[11]);
        assert_eq!(tokens[0].target, 0);
        assert!(tokens[0].tokens.is_empty());
        assert_eq!(tokens[1].tokens, [u32::MAX, 2, 2, 0]);
        let variables = &complete::<tables::ScriptVariables>(&views[8])[0].entries;
        assert_eq!(variables[0].name, b"x");
        assert_eq!(variables[1].name, [0xff, 0]);
        assert_eq!(
            (variables[0].value_raw, variables[1].value_raw),
            (i64::MAX, i64::MIN)
        );
    }

    #[test]
    fn invalid_arenas_duplicate_literal_keys_and_unavailable_scopes_never_become_empty_tables() {
        let (_, mut capture) = small();
        capture.operation_tokens.outcome = Outcome::Rows { len: 1 };
        capture.operation_tokens.rows[0].tokens_start = usize::MAX;
        capture.operation_tokens.rows[0].tokens_len = 2;
        assert!(matches!(
            capture.tables()[11],
            TableView::Unavailable { .. }
        ));

        capture.variables.outcome = Outcome::Rows { len: 1 };
        capture.variables.rows[0].entries_len = 2;
        capture.variable_entries.len = 2;
        capture.variable_entries.required = 2;
        capture.variable_entries.first[0] = cvariables::Row {
            name_start: 0,
            name_len: 1,
            value_raw: 1,
        };
        capture.variable_entries.first[1] = cvariables::Row {
            name_start: 1,
            name_len: 1,
            value_raw: 2,
        };
        capture.variable_names.first[..2].copy_from_slice(b"aa");
        capture.variable_names.len = 2;
        capture.variable_names.required = 2;
        assert!(matches!(capture.tables()[8], TableView::Unavailable { .. }));
        let unavailable = Outcome::Unavailable {
            reason: "script variable scope root is null",
        };
        capture.variables.outcome = unavailable;
        assert!(matches!(capture.tables()[8], TableView::Unavailable { .. }));
        capture.reset();
        assert_eq!(capture.tokens.len, 0);
    }

    fn variable_comparison_fixture() -> (Captured, Vec<rows::ScriptVariablesRow>) {
        let (_, mut capture) = small();
        capture.variables.outcome = Outcome::Rows { len: 3 };
        capture.variables.rows[..3].copy_from_slice(&[
            cvariables::ScopeRow {
                scope: cvariables::Scope::State(4),
                entries_start: 3,
                entries_len: 1,
            },
            cvariables::ScopeRow {
                scope: cvariables::Scope::Country(9),
                entries_start: 0,
                entries_len: 3,
            },
            cvariables::ScopeRow {
                scope: cvariables::Scope::Global,
                entries_start: 4,
                entries_len: 0,
            },
        ]);
        capture.variable_entries.first[..4].copy_from_slice(&[
            cvariables::Row {
                name_start: 0,
                name_len: 2,
                value_raw: i64::MIN,
            },
            cvariables::Row {
                name_start: 2,
                name_len: 1,
                value_raw: i64::MAX,
            },
            cvariables::Row {
                name_start: 3,
                name_len: 0,
                value_raw: -1,
            },
            cvariables::Row {
                name_start: 3,
                name_len: 2,
                value_raw: 42,
            },
        ]);
        capture.variable_entries.len = 4;
        capture.variable_entries.required = 4;
        capture.variable_names.first[..5].copy_from_slice(&[0xff, 0, b'a', b'A', 0]);
        capture.variable_names.len = 5;
        capture.variable_names.required = 5;
        let converted = capture.variables_table(capture.outcomes()[8]);
        let previous = complete::<tables::ScriptVariables>(&converted).to_vec();
        (capture, previous)
    }

    fn variable_baseline(previous: Vec<rows::ScriptVariablesRow>) -> replay::frame::FrameEncoder {
        let mut encoder = replay::frame::FrameEncoder::new();
        let mut views = std::array::from_fn(|_| unavailable("fixture table not acquired"));
        views[TableId::ScriptVariables.index()] =
            TableView::Complete(rows::TableRows::ScriptVariables(previous));
        encoder
            .encode(
                replay::frame::DecodedFrame {
                    branch: 1,
                    hour: 0,
                    tables: views,
                    metrics: replay::frame::Metrics::default(),
                },
                false,
            )
            .unwrap();
        encoder
    }

    #[test]
    fn variable_precomparison_handles_hash_order_empty_scopes_and_literal_bytes_without_row_leases()
    {
        let (mut capture, previous) = variable_comparison_fixture();
        let encoder = variable_baseline(previous.clone());
        let mut scratch = ScriptVariableComparison::default();
        let bytes = capture.allocated_bytes();
        assert!(capture.variables_equal(capture.outcomes()[8], &previous, &mut scratch));
        let capacities = (scratch.scopes.capacity(), scratch.entries.capacity());
        // Arena relocation and unused bytes do not change a literal key.
        capture.variable_names.first[6..8].copy_from_slice(&[0xff, 0]);
        capture.variable_entries.first[0].name_start = 6;
        capture.variable_names.len = 8;
        capture.variable_names.required = 8;
        capture.variables.rows.swap(0, 2);
        let views = capture.table_inputs(
            encoder.comparison_baseline::<tables::ScriptVariables>(1, 1, false),
            &mut scratch,
            None,
        );
        assert!(matches!(views[8], TableInput::ComparedEqual(_)));
        assert_eq!(
            capacities,
            (scratch.scopes.capacity(), scratch.entries.capacity())
        );
        assert_eq!(capture.allocated_bytes(), bytes);
        capture.variables.outcome = Outcome::Rows { len: 0 };
        capture.variable_entries.len = 0;
        capture.variable_names.len = 0;
        assert!(capture.variables_equal(capture.outcomes()[8], &[], &mut scratch));
        assert!(!capture.variables_equal(capture.outcomes()[8], &previous, &mut scratch));
    }

    #[test]
    fn variable_precomparison_preserves_changed_values_names_and_scope_identity() {
        for change in 0..4 {
            let (mut capture, previous) = variable_comparison_fixture();
            let encoder = variable_baseline(previous);
            match change {
                0 => capture.variable_entries.first[0].value_raw += 1,
                1 => capture.variable_names.first[2] = b'A',
                2 => capture.variables.rows[0].scope = cvariables::Scope::State(5),
                _ => capture.variable_entries.first[2].name_len = 1,
            }
            let views = capture.table_inputs(
                encoder.comparison_baseline::<tables::ScriptVariables>(1, 1, false),
                &mut ScriptVariableComparison::default(),
                None,
            );
            assert!(
                matches!(views[8], TableInput::Sampled(TableView::Complete(_))),
                "change {change}"
            );
        }
    }

    #[test]
    fn variable_precomparison_scope_hint_accepts_ordered_and_reordered_country_and_state_ids() {
        let (mut capture, _) = variable_comparison_fixture();
        capture.variables.rows.extend_from_slice(&[
            cvariables::ScopeRow {
                scope: cvariables::Scope::Country(2),
                entries_start: 4,
                entries_len: 0,
            },
            cvariables::ScopeRow {
                scope: cvariables::Scope::State(1),
                entries_start: 4,
                entries_len: 0,
            },
        ]);
        capture.variables.outcome = Outcome::Rows { len: 5 };
        capture
            .variables
            .rows
            .sort_unstable_by_key(|row| scope_identity(row.scope));
        let ordered = capture.variables.rows.clone();
        let converted = capture.variables_table(capture.outcomes()[8]);
        let previous = complete::<tables::ScriptVariables>(&converted).to_vec();
        let encoder = variable_baseline(previous.clone());
        let mut scratch = ScriptVariableComparison::default();
        for order in [
            [0, 1, 2, 3, 4], // Every checked ordinal matches.
            [0, 2, 1, 3, 4], // Country canonical-slot order differs from raw IDs.
            [0, 1, 2, 4, 3], // State permutation must also use the fallback.
            [0, 2, 1, 4, 3],
        ] {
            for (row, index) in capture.variables.rows.iter_mut().zip(order) {
                *row = ordered[index];
            }
            let views = capture.table_inputs(
                encoder.comparison_baseline::<tables::ScriptVariables>(1, 1, false),
                &mut scratch,
                None,
            );
            assert!(
                matches!(views[8], TableInput::ComparedEqual(_)),
                "scope order {order:?}"
            );
            let converted = capture.variables_table(capture.outcomes()[8]);
            assert_eq!(
                complete::<tables::ScriptVariables>(&converted),
                previous.as_slice(),
                "scope order {order:?}"
            );
        }
    }

    #[test]
    fn variable_precomparison_rejects_duplicate_names_and_scopes_instead_of_hiding_missing_keys() {
        for duplicate_scope in [false, true] {
            let (mut capture, previous) = variable_comparison_fixture();
            let encoder = variable_baseline(previous);
            if duplicate_scope {
                capture.variables.rows[0] = capture.variables.rows[1];
            } else {
                capture.variable_entries.first[1] = capture.variable_entries.first[0];
            }
            let views = capture.table_inputs(
                encoder.comparison_baseline::<tables::ScriptVariables>(1, 1, false),
                &mut ScriptVariableComparison::default(),
                None,
            );
            assert!(matches!(
                views[8],
                TableInput::Sampled(TableView::Unavailable { .. })
            ));
        }
    }

    #[test]
    fn variable_precomparison_arena_faults_retain_unavailable_verdicts() {
        for fault in 0..8 {
            let (mut capture, previous) = variable_comparison_fixture();
            let encoder = variable_baseline(previous);
            match fault {
                0 => capture.variables.rows[0].entries_start = usize::MAX,
                1 => capture.variable_entries.len = capture.variable_entries.first.len() + 1,
                2 => capture.variable_entries.required = 0,
                3 => capture.variable_names.len = capture.variable_names.first.len() + 1,
                4 => capture.variable_names.required = 0,
                5 => capture.variable_entries.first[0].name_start = usize::MAX,
                6 => capture.variable_entries.first[0].name_len = usize::MAX,
                _ => capture.variables.outcome = Outcome::Rows { len: 4 },
            }
            let views = capture.table_inputs(
                encoder.comparison_baseline::<tables::ScriptVariables>(1, 1, false),
                &mut ScriptVariableComparison::default(),
                None,
            );
            assert!(
                matches!(views[8], TableInput::Sampled(TableView::Unavailable { .. })),
                "fault {fault}"
            );
        }
    }

    #[test]
    fn variable_precomparison_noncomplete_sources_never_reuse() {
        for fault in 0..2 {
            let (mut capture, previous) = variable_comparison_fixture();
            let encoder = variable_baseline(previous);
            match fault {
                0 => {
                    capture.variables.outcome = Outcome::Truncated {
                        len: 3,
                        unread_rows: 1,
                    }
                }
                1 => {
                    capture.variables.outcome = Outcome::Unavailable {
                        reason: "fixture source unreadable",
                    }
                }
                _ => unreachable!(),
            }
            let views = capture.table_inputs(
                encoder.comparison_baseline::<tables::ScriptVariables>(1, 1, false),
                &mut ScriptVariableComparison::default(),
                None,
            );
            if fault == 0 {
                assert!(matches!(
                    views[8],
                    TableInput::Sampled(TableView::Truncated { .. })
                ));
            } else {
                assert!(matches!(
                    views[8],
                    TableInput::Sampled(TableView::Unavailable { .. })
                ));
            }
        }
    }

    #[test]
    fn primary_growth_preserves_row_and_arena_requirements() {
        let (mut capacities, mut capture) = small();
        capture.subjects.outcome = Outcome::Truncated {
            len: 3,
            unread_rows: 12,
        };
        capture.subject_names.required = 33;
        capture.grow(&mut capacities);
        assert!(capacities.rows[1] >= 15);
        assert!(capacities.subject_names >= 33);
        let before = capture.allocated_bytes();
        capture.reset();
        capture.grow(&mut capacities);
        assert_eq!(capture.allocated_bytes(), before);
    }

    const BASE: usize = 0x0001_4000_0000;
    const GAME: usize = 0x1000;

    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }

    fn flag_scopes() -> Vec<u8> {
        let mut bytes = vec![0; 0xc000];
        for (offset, data, count) in [
            (cgame_state::COUNTRIES, 0x2000_u64, 3_u32),
            (cgame_state::COUNTRY_LINKS, 0x2100, 3),
            (cgame_state::STATES, 0x2200, 3),
        ] {
            put(&mut bytes, GAME + offset, &data.to_le_bytes());
            put(&mut bytes, GAME + offset + 8, &count.to_le_bytes());
            put(&mut bytes, GAME + offset + 12, &count.to_le_bytes());
        }
        put(&mut bytes, 0x2008, &0x4000_u64.to_le_bytes());
        put(&mut bytes, 0x2010, &0x4000_u64.to_le_bytes());
        put(&mut bytes, 0x2104, &1_u32.to_le_bytes());
        put(&mut bytes, 0x2108, &1_u32.to_le_bytes());
        put(
            &mut bytes,
            0x4000,
            &((BASE + ccountry::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x4000 + ccountry::ID, &2_u32.to_le_bytes());
        put(&mut bytes, 0x2200, &u64::MAX.to_le_bytes());
        put(&mut bytes, 0x2208, &0x8000_u64.to_le_bytes());
        put(
            &mut bytes,
            0x8000,
            &((BASE + cstate::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x8000 + cstate::ID, &1_u32.to_le_bytes());
        for (index, (root, offset)) in [
            (GAME, cgame_state::GLOBAL_FLAGS),
            (0x4000, ccountry::FLAG_MANAGER),
            (0x8000, cstate::FLAGS),
        ]
        .into_iter()
        .enumerate()
        {
            let manager = 0xa000 + index * 0x100;
            let row = 0xb000 + index * cscript_flag::STRIDE;
            put(&mut bytes, root + offset, &(manager as u64).to_le_bytes());
            put(
                &mut bytes,
                manager,
                &((BASE + cflag_manager::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                manager + cflag_manager::ROWS,
                &(row as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                manager + cflag_manager::SLOT_COUNT,
                &1_i32.to_le_bytes(),
            );
            put(
                &mut bytes,
                manager + cflag_manager::ENTRY_COUNT,
                &1_i32.to_le_bytes(),
            );
            for (field, vtable) in [
                (0, cscript_flag::VTABLE_RVA),
                (cscript_flag::DATE, cscript_flag::DATE_VTABLE_RVA),
                (
                    cscript_flag::DATE_SECONDARY,
                    cscript_flag::DATE_SECONDARY_VTABLE_RVA,
                ),
            ] {
                put(
                    &mut bytes,
                    row + field,
                    &((BASE + vtable) as u64).to_le_bytes(),
                );
            }
            put(
                &mut bytes,
                row + cscript_flag::TOKEN_ID,
                &u32::MAX.to_le_bytes(),
            );
            put(
                &mut bytes,
                row + cscript_flag::SET_HOUR,
                &i32::MIN.to_le_bytes(),
            );
            put(
                &mut bytes,
                row + cscript_flag::VALUE,
                &i16::MIN.to_le_bytes(),
            );
            put(
                &mut bytes,
                row + cscript_flag::REMAINING_DAYS,
                &(-3_i16).to_le_bytes(),
            );
        }
        bytes
    }

    #[test]
    fn flag_census_preserves_all_scope_identities_aliases_and_exact_truncation() {
        let mut bytes = flag_scopes();
        let mut rows = [ScopedFlag::default(); 3];
        let mut scratch = [cscript_flag::Row::default(); 3];
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            walk_flags(&memory, BASE, GAME, &mut rows, &mut scratch),
            Outcome::Rows { len: 3 }
        );
        assert_eq!(
            rows.map(|row| (row.scope_kind, row.scope_id)),
            [(GLOBAL_SCOPE, 0), (COUNTRY_SCOPE, 2), (STATE_SCOPE, 1)]
        );
        assert!(
            rows.iter()
                .all(|row| row.flag.value == i16::MIN && row.flag.remaining_days == -3)
        );
        assert_eq!(
            walk_flags(&memory, BASE, GAME, &mut rows[..1], &mut scratch),
            Outcome::Truncated {
                len: 1,
                unread_rows: 2
            }
        );
        put(&mut bytes, 0x8000 + cstate::FLAGS, &0_u64.to_le_bytes());
        assert!(matches!(
            walk_flags(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                BASE,
                GAME,
                &mut rows,
                &mut scratch
            ),
            Outcome::Unavailable { .. }
        ));
    }

    /// Shared synthetic address space for the relations copy/decode harness.
    /// Three countries in slots 1..3, each with a complete four-slot matrix;
    /// slot zero is the reserved sentinel the walk never reaches.
    pub(crate) mod relations_test_kit {
        use super::super::*;
        use engine::{
            cdiplomacy_status, cgame_state, cguarantee_relation, cmarket_access_rights_relation,
            crelation, memory::SliceMemory,
        };

        pub(crate) const BASE: usize = 0;
        pub(crate) const GS: usize = 0x1000;
        pub(crate) const COUNTRIES: [usize; 3] = [0x4000, 0x5000, 0x6000];
        pub(crate) const DIPLOMACY: [usize; 3] = [0x8000, 0x9000, 0xa000];
        pub(crate) const RELATION: [usize; 2] = [0x1c000, 0x1c800];

        pub(crate) fn p32(bytes: &mut [u8], at: usize, value: u32) {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        pub(crate) fn p64(bytes: &mut [u8], at: usize, value: u64) {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        pub(crate) fn array(bytes: &mut [u8], at: usize, data: usize, values: &[u64]) {
            p64(bytes, at, data as u64);
            p32(bytes, at + 8, values.len() as u32);
            p32(bytes, at + 12, values.len() as u32);
            for (index, value) in values.iter().enumerate() {
                p64(bytes, data + index * 8, *value);
            }
        }
        pub(crate) fn status(owner: usize, target: usize) -> usize {
            0x10000 + owner * 0x4000 + target * 0x1000
        }
        pub(crate) fn matrix_data(owner: usize) -> usize {
            0x3200 + owner * 0x40
        }
        pub(crate) fn serializable_data(owner: usize, target: usize) -> usize {
            0x2800 + owner * 0x80 + target * 0x20
        }

        pub(crate) fn fixture() -> Vec<u8> {
            let mut bytes = vec![0_u8; 0x24000];
            array(
                &mut bytes,
                GS + cgame_state::COUNTRIES,
                0x3000,
                &[
                    u64::MAX,
                    COUNTRIES[0] as u64,
                    COUNTRIES[1] as u64,
                    COUNTRIES[2] as u64,
                ],
            );
            p64(&mut bytes, GS + cgame_state::COUNTRY_LINKS, 0x3100);
            p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 8, 4);
            p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 12, 4);
            for (index, value) in [0_u32, 1, 2, 3].into_iter().enumerate() {
                p32(&mut bytes, 0x3100 + index * 4, value);
            }
            for owner in 0..3 {
                let country = COUNTRIES[owner];
                let diplomacy = DIPLOMACY[owner];
                p64(&mut bytes, country, (BASE + ccountry::VTABLE_RVA) as u64);
                p32(&mut bytes, country + ccountry::ID, owner as u32 + 1);
                p64(
                    &mut bytes,
                    country + ccountry::DIPLOMACY_STATUS,
                    diplomacy as u64,
                );
                p64(
                    &mut bytes,
                    diplomacy,
                    (BASE + cdiplomacy_status::VTABLE_RVA) as u64,
                );
                p64(
                    &mut bytes,
                    diplomacy + cdiplomacy_status::COUNTRY,
                    country as u64,
                );
                let slots: Vec<u64> = (0..4).map(|t| status(owner, t) as u64).collect();
                array(
                    &mut bytes,
                    diplomacy + cdiplomacy_status::RELATIONS,
                    matrix_data(owner),
                    &slots,
                );
                for target in 0..4 {
                    let object = status(owner, target);
                    p64(
                        &mut bytes,
                        object,
                        (BASE + crelation_status::VTABLE_RVA) as u64,
                    );
                    p64(
                        &mut bytes,
                        object + crelation_status::DIPLOMACY_STATUS,
                        diplomacy as u64,
                    );
                    p32(
                        &mut bytes,
                        object + crelation_status::TARGET_COUNTRY,
                        target as u32,
                    );
                    array(
                        &mut bytes,
                        object + crelation_status::SERIALIZABLE_RELATIONS,
                        serializable_data(owner, target),
                        &[],
                    );
                }
                p32(
                    &mut bytes,
                    status(owner, 0) + crelation_status::TARGET_COUNTRY,
                    1,
                );
            }
            bytes
        }

        pub(crate) fn relation(
            bytes: &mut [u8],
            object: usize,
            vtable: usize,
            type_id: u32,
            first: u32,
            second: u32,
        ) {
            p64(bytes, object, (BASE + vtable) as u64);
            p32(bytes, object + crelation::TYPE_ID, type_id);
            p32(bytes, object + crelation::FIRST_COUNTRY, first);
            p32(bytes, object + crelation::SECOND_COUNTRY, second);
        }

        pub(crate) fn memory(bytes: &[u8]) -> SliceMemory<'_> {
            SliceMemory { base: 0, bytes }
        }

        pub(crate) struct Direct {
            pub opinion_rows: Vec<crelation_status::OpinionRow>,
            pub opinion_outcome: Outcome,
            pub tuple_rows: Vec<crelation_status::TupleRow>,
            pub tuple_outcome: Outcome,
        }

        /// Typed reference path: the existing `crelation_status::walk`.
        pub(crate) fn direct(memory: &impl Memory, opinion_cap: usize, tuple_cap: usize) -> Direct {
            let mut opinions = vec![crelation_status::OpinionRow::default(); opinion_cap];
            let mut tuples = vec![crelation_status::TupleRow::default(); tuple_cap];
            let result =
                crelation_status::walk(memory, BASE, GS, &mut opinions, &mut tuples, [true; 2]);
            // The collector bounds row counts against its buffers.
            let opinions = bounded_for_test(result.opinions, opinions);
            let tuples = bounded_for_test(result.tuples, tuples);
            Direct {
                opinion_rows: opinions.1,
                opinion_outcome: opinions.0,
                tuple_rows: tuples.1,
                tuple_outcome: tuples.0,
            }
        }

        fn bounded_for_test<R: Default + Clone>(
            outcome: Outcome,
            mut rows: Vec<R>,
        ) -> (Outcome, Vec<R>) {
            let outcome = if outcome.len() > rows.len() {
                Outcome::Unavailable {
                    reason: "walker row count exceeds allocated buffer",
                }
            } else {
                outcome
            };
            let kept = rows.drain(..outcome.len().min(rows.len())).collect();
            (outcome, kept)
        }

        /// Split path under test: raw copy on `memory`, decode into `intel`.
        pub(crate) fn split(
            memory: &impl Memory,
            intel: &mut Captured,
        ) -> (RelationsCopyReport, RelationsJob) {
            let mut job = RelationsJob::new();
            let ctx = RelationsCopyContext {
                base: BASE,
                game_state: GS,
                active: [true; 2],
                opinion_cap: intel.opinion_rows_cap(),
                tuple_cap: intel.tuple_rows_cap(),
            };
            let report = copy_relations_refresh(memory, ctx, &mut job);
            assert!(report.raw_ok);
            intel.decode_relations_payload(BASE, &job, &mut [0; 2]);
            (report, job)
        }

        pub(crate) fn assert_matches_direct(intel: &Captured, direct: &Direct) {
            assert_eq!(intel.relation_opinions.outcome, direct.opinion_outcome);
            assert_eq!(
                &intel.relation_opinions.rows[..direct.opinion_outcome.len()],
                direct.opinion_rows.as_slice()
            );
            assert_eq!(intel.relation_tuples.outcome, direct.tuple_outcome);
            assert_eq!(
                &intel.relation_tuples.rows[..direct.tuple_outcome.len()],
                direct.tuple_rows.as_slice()
            );
        }

        pub(crate) fn intel_captured(opinion_cap: usize, tuple_cap: usize) -> Captured {
            let mut capacities = Capacities::default();
            capacities.rows[13] = opinion_cap;
            capacities.rows[14] = tuple_cap;
            Captured::with_capacities(&capacities)
        }

        pub(crate) fn opinionated_fixture() -> Vec<u8> {
            let mut bytes = fixture();
            p32(
                &mut bytes,
                status(0, 2) + crelation_status::OPINION,
                (-100_i32) as u32,
            );
            p32(&mut bytes, status(1, 3) + crelation_status::OPINION, 60);
            p32(&mut bytes, status(2, 0) + crelation_status::OPINION, 7);
            relation(
                &mut bytes,
                RELATION[0],
                cmarket_access_rights_relation::VTABLE_RVA,
                cmarket_access_rights_relation::TYPE,
                1,
                3,
            );
            p32(&mut bytes, RELATION[0] + crelation::START_HOUR, 100);
            p32(
                &mut bytes,
                RELATION[0] + crelation::END_HOUR,
                (-5_i32) as u32,
            );
            bytes[RELATION[0] + crelation::CANCEL] = 0x80;
            bytes[RELATION[0] + crelation::RETIRED] = 0x02;
            relation(
                &mut bytes,
                RELATION[1],
                cguarantee_relation::VTABLE_RVA,
                cguarantee_relation::TYPE,
                1,
                3,
            );
            array(
                &mut bytes,
                status(0, 3) + crelation_status::SERIALIZABLE_RELATIONS,
                serializable_data(0, 3),
                &[RELATION[0] as u64, RELATION[1] as u64],
            );
            bytes
        }
    }

    use relations_test_kit as relations_kit;

    fn linear_relation_lookup(job: &RelationsJob, raw: u32) -> Result<CountrySnap, &'static str> {
        if raw == 0 {
            return Err("country id is zero");
        }
        let canonical = job
            .links
            .get(raw as usize)
            .copied()
            .ok_or("country link unreadable or outside country table")??
            .ok_or("country link unreadable or outside country table")?;
        job.countries
            .iter()
            .find(|entry| entry.canonical_slot == canonical)
            .copied()
            .ok_or("country link resolves to null country")
    }

    #[test]
    fn relations_country_index_keeps_first_duplicate_and_native_identity() {
        let first = CountrySnap {
            slot: 3,
            pointer: 0x1234,
            id: 19,
            canonical_slot: 3,
        };
        let second = CountrySnap {
            slot: 1,
            pointer: 0x5678,
            id: 7,
            canonical_slot: 1,
        };
        let job = RelationsJob {
            countries: vec![
                first,
                second,
                CountrySnap {
                    pointer: 0x9999,
                    ..first
                },
            ],
            links: vec![Ok(Some(3)), Ok(Some(3)), Ok(Some(1)), Ok(Some(3))],
            ..RelationsJob::default()
        };
        let mut index = Vec::new();
        prepare_relation_country_index(&mut index, &job.countries);
        assert_eq!(index, [usize::MAX, 1, usize::MAX, 0]);
        assert_eq!(relations_lookup(&job, &index, 1), Ok(first));
        assert_eq!(relations_lookup(&job, &index, 2), Ok(second));
        assert_eq!(relations_lookup(&job, &index, 3), Ok(first));
        for raw in [0, 1, 2, 3, 4, u32::MAX] {
            assert_eq!(
                relations_lookup(&job, &index, raw),
                linear_relation_lookup(&job, raw)
            );
        }
    }

    #[test]
    fn relations_country_index_preserves_link_faults_missing_slots_and_zero_precedence() {
        let country = CountrySnap {
            slot: 2,
            pointer: 0x1234,
            id: 20,
            canonical_slot: 2,
        };
        let job = RelationsJob {
            countries: vec![country],
            links: vec![
                Err("sentinel fault must not replace zero-id error"),
                Ok(Some(2)),
                Err("recorded link fault"),
                Ok(None),
                Ok(Some(1)),
                Ok(Some(u32::MAX)),
            ],
            ..RelationsJob::default()
        };
        let mut index = Vec::new();
        prepare_relation_country_index(&mut index, &job.countries);
        // Neither the highest raw alias nor a missing canonical target sizes
        // the index: only the authenticated frozen countries do.
        assert_eq!(index.len(), 3);
        for raw in [0, 1, 2, 3, 4, 5, 6, u32::MAX] {
            assert_eq!(
                relations_lookup(&job, &index, raw),
                linear_relation_lookup(&job, raw)
            );
        }
        assert_eq!(relations_lookup(&job, &index, 0), Err("country id is zero"));
        assert_eq!(
            relations_lookup(&job, &index, 2),
            Err("recorded link fault")
        );
        assert_eq!(
            relations_lookup(&job, &index, 3),
            Err("country link unreadable or outside country table")
        );
        assert_eq!(
            relations_lookup(&job, &index, 4),
            Err("country link resolves to null country")
        );
    }

    #[test]
    fn relations_country_index_rebuilds_current_job_without_accumulating_slots() {
        let mut job = RelationsJob::default();
        job.countries.push(CountrySnap {
            slot: 63,
            pointer: 0x1000,
            id: 63,
            canonical_slot: 63,
        });
        let mut index = Vec::new();
        prepare_relation_country_index(&mut index, &job.countries);
        let capacity = index.capacity();
        for generation in 0..128 {
            job.clear();
            let canonical = generation % 63 + 1;
            let current = CountrySnap {
                slot: canonical,
                pointer: 0x2000 + generation as usize * 16,
                id: 1000 + generation,
                canonical_slot: canonical,
            };
            job.countries.extend([
                current,
                CountrySnap {
                    pointer: 0xffff,
                    ..current
                },
            ]);
            job.links.extend([
                Ok(None),
                Ok(Some(canonical)),
                Ok(Some(63)),
                Err("current fault"),
            ]);
            prepare_relation_country_index(&mut index, &job.countries);
            assert_eq!(index.len(), canonical as usize + 1);
            assert_eq!(index.capacity(), capacity);
            assert_eq!(relations_lookup(&job, &index, 1), Ok(current));
            for raw in [0, 1, 2, 3, 4, u32::MAX] {
                assert_eq!(
                    relations_lookup(&job, &index, raw),
                    linear_relation_lookup(&job, raw)
                );
            }
        }
        job.clear();
        job.links.push(Ok(Some(1)));
        prepare_relation_country_index(&mut index, &job.countries);
        assert!(index.is_empty());
        assert_eq!(index.capacity(), capacity);
    }

    #[test]
    fn relations_country_index_reused_decode_matches_typed_and_accounts_scratch() {
        let mut bytes = relations_kit::opinionated_fixture();
        let mut intel = relations_kit::intel_captured(16, 16);
        let allocated = intel.allocated_bytes();
        let mut job = RelationsJob::new();
        let storage = (job.slot_pool.as_ptr(), job.slot_pool.len());
        let context = RelationsCopyContext {
            base: relations_kit::BASE,
            game_state: relations_kit::GS,
            active: [true; 2],
            opinion_cap: 16,
            tuple_cap: 16,
        };
        for generation in 0..3 {
            // Removing country 3 leaves referenced endpoints unresolved; the
            // following refresh restores it. No old index may survive either.
            relations_kit::p64(
                &mut bytes,
                0x3000 + 3 * 8,
                if generation == 1 {
                    0
                } else {
                    relations_kit::COUNTRIES[2] as u64
                },
            );
            let memory = relations_kit::memory(&bytes);
            let direct = relations_kit::direct(&memory, 16, 16);
            let report = copy_relations_refresh(&memory, context, &mut job);
            assert!(report.raw_ok);
            assert_eq!((job.slot_pool.as_ptr(), job.slot_pool.len()), storage);
            let slots = if generation == 1 { 6 } else { 9 };
            assert_eq!(job.slot_used, slots * crelation_status::RECORD_BYTES);
            if generation == 0 {
                assert!(intel.relation_country_index.is_empty());
                assert_eq!(intel.allocated_bytes(), allocated);
            }
            intel.decode_relations_payload(relations_kit::BASE, &job, &mut [0; 2]);
            relations_kit::assert_matches_direct(&intel, &direct);
            assert_eq!(
                intel.allocated_bytes() - allocated,
                intel.relation_country_index.capacity() * size_of::<usize>()
            );
            // The index must be reconstructed, even when a pooled payload's
            // country ordering changes without its length changing.
            job.countries.reverse();
            intel.decode_relations_payload(relations_kit::BASE, &job, &mut [0; 2]);
            relations_kit::assert_matches_direct(&intel, &direct);
        }
    }

    #[test]
    fn relations_slot_storage_grows_off_capture_and_discards_short_gather_bytes() {
        struct ShortSlot<'a> {
            memory: SliceMemory<'a>,
        }
        impl Memory for ShortSlot<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == relations_kit::status(0, 2) + crelation_status::OPINION
                    && output.len() == 4
                {
                    let _ = self.memory.copy(address, &mut output[..2]);
                    return false;
                }
                self.memory.copy(address, output)
            }
        }
        let bytes = relations_kit::opinionated_fixture();
        let memory = relations_kit::memory(&bytes);
        let mut job = RelationsJob::new();
        job.slot_pool = vec![0xa5; 5 * crelation_status::RECORD_BYTES];
        job.update_capacity_charge();
        let context = RelationsCopyContext {
            base: relations_kit::BASE,
            game_state: relations_kit::GS,
            active: [true; 2],
            opinion_cap: 16,
            tuple_cap: 16,
        };
        // A failed page reservation leaves allocation and initialized storage
        // unchanged. Only the explicit off-capture grow can add room.
        let storage = (job.slot_pool.as_ptr(), job.slot_pool.len());
        assert!(!copy_relations_refresh(&memory, context, &mut job).raw_ok);
        assert_eq!((job.slot_pool.as_ptr(), job.slot_pool.len()), storage);
        assert_eq!(job.slot_used, 3 * crelation_status::RECORD_BYTES);
        job.grow();
        let storage = (job.slot_pool.as_ptr(), job.slot_pool.len());
        let mut captured = relations_kit::intel_captured(16, 16);
        assert!(copy_relations_refresh(&memory, context, &mut job).raw_ok);
        captured.decode_relations_payload(relations_kit::BASE, &job, &mut [0; 2]);
        relations_kit::assert_matches_direct(&captured, &relations_kit::direct(&memory, 16, 16));

        let short = ShortSlot { memory };
        assert!(copy_relations_refresh(&short, context, &mut job).raw_ok);
        assert_eq!(job.metas[0].slot_len, crelation_status::RECORD_BYTES);
        assert_eq!(job.slot_used, crelation_status::RECORD_BYTES);
        assert_eq!(job.metas.len(), 1);
        job.slot_pool[job.slot_used..].fill(0xff);
        captured.decode_relations_payload(relations_kit::BASE, &job, &mut [0; 2]);
        relations_kit::assert_matches_direct(&captured, &relations_kit::direct(&short, 16, 16));

        // All scheduled bytes are replaced after the short gather; poisoned
        // inactive storage cannot become an opinion, tuple or error.
        assert!(copy_relations_refresh(&short.memory, context, &mut job).raw_ok);
        assert_eq!(job.slot_used, 9 * crelation_status::RECORD_BYTES);
        assert_eq!((job.slot_pool.as_ptr(), job.slot_pool.len()), storage);
        captured.decode_relations_payload(relations_kit::BASE, &job, &mut [0; 2]);
        relations_kit::assert_matches_direct(
            &captured,
            &relations_kit::direct(&short.memory, 16, 16),
        );
    }

    #[test]
    fn staged_relations_skip_the_game_thread_typed_walk() {
        struct CountOpinion<'a> {
            memory: SliceMemory<'a>,
            reads: std::cell::Cell<usize>,
        }
        impl Memory for CountOpinion<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                let opinion = relations_kit::status(0, 2) + crelation_status::OPINION;
                if address <= opinion && opinion < address.saturating_add(destination.len()) {
                    self.reads.set(self.reads.get() + 1);
                }
                self.memory.copy(address, destination)
            }
        }
        let bytes = relations_kit::opinionated_fixture();
        let memory = CountOpinion {
            memory: relations_kit::memory(&bytes),
            reads: std::cell::Cell::new(0),
        };
        let mut capture = relations_kit::intel_captured(16, 16);
        capture.capture(&memory, relations_kit::BASE, relations_kit::GS, 21, false);
        assert!(memory.reads.get() > 0);
        memory.reads.set(0);
        capture.relations_raw_mode = true;
        capture.capture(&memory, relations_kit::BASE, relations_kit::GS, 21, false);
        assert_eq!(memory.reads.get(), 0);
    }

    #[test]
    fn relations_copy_decode_matches_typed_rows_including_skips_and_ordinals() {
        let bytes = relations_kit::opinionated_fixture();
        let memory = relations_kit::memory(&bytes);
        let direct = relations_kit::direct(&memory, 16, 16);
        assert_eq!(direct.opinion_outcome, Outcome::Rows { len: 2 });
        assert_eq!(direct.tuple_outcome, Outcome::Rows { len: 2 });
        // Slot zero is never serialized even with a nonzero opinion.
        assert_eq!(
            direct
                .opinion_rows
                .iter()
                .map(|row| (row.owner_country, row.target_country, row.opinion))
                .collect::<Vec<_>>(),
            [(1, 2, -100), (2, 3, 60)]
        );
        assert_eq!(direct.tuple_rows[0].ordinal, 0);
        assert_eq!(direct.tuple_rows[1].ordinal, 1);
        let mut intel = relations_kit::intel_captured(16, 16);
        let memory = engine::memory::CountedMemory::new(memory);
        let native = read_accounting::Capture::new();
        let (report, _) = relations_kit::split(&memory, &mut intel);
        let counts = native.finish();
        let mut expected_counts = read_accounting::ZERO;
        expected_counts[TABLES[14].index()] = counts[TABLES[14].index()];
        assert!(counts[TABLES[14].index()].spans > 0);
        assert!(counts[TABLES[14].index()].bytes > 0);
        assert_eq!(counts, expected_counts);
        assert!(report.raw_ok);
        assert!(report.raw_ok);
        assert_eq!(report.raw_bytes[0], 0);
        assert!(report.raw_bytes[1] > 0);
        relations_kit::assert_matches_direct(&intel, &direct);
    }

    #[test]
    fn relation_owner_gather_preserves_every_header_fault_and_saves_one_guard() {
        use std::cell::{Cell, RefCell};
        let owner = relations_kit::DIPLOMACY[0];
        let fields = [
            (owner, 8),
            (owner + cdiplomacy_status::COUNTRY, 8),
            (owner + cdiplomacy_status::RELATIONS, 16),
        ];
        struct Headers<'a> {
            source: SliceMemory<'a>,
            fields: [(usize, usize); 3],
            fault: Option<usize>,
            reads: RefCell<Vec<(usize, usize)>>,
            guards: Cell<usize>,
        }
        impl Headers<'_> {
            fn span(&self, address: usize, output: &mut [u8]) -> bool {
                if self.fields.contains(&(address, output.len())) {
                    self.reads.borrow_mut().push((address, output.len()));
                }
                if let Some(fault) = self.fault
                    && address <= fault
                    && fault - address < output.len()
                {
                    let prefix = fault - address;
                    let _ = self.source.copy(address, &mut output[..prefix]);
                    return false;
                }
                self.source.copy(address, output)
            }
        }
        impl Memory for Headers<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if self.fields.contains(&(address, output.len())) {
                    self.guards.set(self.guards.get() + 1);
                }
                self.span(address, output)
            }
            fn copy_scattered<const M: usize>(
                &self,
                addresses: [usize; M],
                outputs: &mut [&mut [u8]; M],
            ) -> usize {
                if addresses
                    .iter()
                    .zip(outputs.iter())
                    .any(|(&address, output)| self.fields.contains(&(address, output.len())))
                {
                    self.guards.set(self.guards.get() + 1);
                }
                for (index, (address, output)) in addresses.into_iter().zip(outputs).enumerate() {
                    if !output.is_empty() && !self.span(address, output) {
                        return index;
                    }
                }
                M
            }
        }
        let bytes = relations_kit::opinionated_fixture();
        for fault in std::iter::once(None).chain(
            fields
                .into_iter()
                .flat_map(|(address, len)| (address..address + len).map(Some)),
        ) {
            let memory = || Headers {
                source: relations_kit::memory(&bytes),
                fields,
                fault,
                reads: RefCell::new(Vec::new()),
                guards: Cell::new(0),
            };
            let direct_memory = memory();
            let direct = relations_kit::direct(&direct_memory, 16, 16);
            let source = memory();
            let mut captured = relations_kit::intel_captured(16, 16);
            let (report, _) = relations_kit::split(&source, &mut captured);
            assert!(report.raw_ok);
            assert_eq!(*source.reads.borrow(), *direct_memory.reads.borrow());
            assert_eq!(source.guards.get(), 1);
            // The old raw path already grouped the two identity fields. The
            // typed walker uses separate scalar identity reads, so it is the
            // row oracle, not the two-guard acquisition reference.
            let previous = memory();
            let (mut vtable, mut backlink, mut header) = ([0; 8], [0; 8], [0; 16]);
            let identity = previous.copy_scattered_prefetched(
                [fields[0].0, fields[1].0],
                &mut [&mut vtable, &mut backlink],
            );
            let previous_error = if identity != 2 {
                Some("country diplomacy unreadable")
            } else if !previous.copy(fields[2].0, &mut header) {
                Some("relation matrix unreadable or invalid")
            } else {
                None
            };
            assert_eq!(*source.reads.borrow(), *previous.reads.borrow());
            assert_eq!(previous.guards.get(), 1 + usize::from(identity == 2));
            assert_eq!(
                direct_memory.guards.get(),
                direct_memory.reads.borrow().len()
            );
            if let Some(reason) = previous_error {
                // Raw identity read faults have always used this reason;
                // the typed reader calls them vtable/backlink mismatches.
                assert_eq!(
                    captured.relation_opinions.outcome,
                    Outcome::Unavailable { reason }
                );
                assert_eq!(
                    captured.relation_tuples.outcome,
                    Outcome::Unavailable { reason }
                );
                assert!(matches!(
                    direct.opinion_outcome,
                    Outcome::Unavailable { .. }
                ));
                assert!(matches!(direct.tuple_outcome, Outcome::Unavailable { .. }));
            } else {
                relations_kit::assert_matches_direct(&captured, &direct);
            }
        }
    }

    #[test]
    fn daily_opinion_gate_preserves_tuple_rows_faults_and_capacity() {
        struct OpinionReads<'a> {
            memory: SliceMemory<'a>,
            fault: bool,
            reads: std::cell::Cell<usize>,
        }
        impl Memory for OpinionReads<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let private = relations_kit::status(0, 2) + crelation_status::OPINION;
                if address <= private && private < address.saturating_add(output.len()) {
                    self.reads.set(self.reads.get() + 1);
                    if self.fault {
                        return false;
                    }
                }
                self.memory.copy(address, output)
            }
        }
        let bytes = relations_kit::opinionated_fixture();
        let mut job = RelationsJob::new();
        for fault in [false, true] {
            for hour in 0..24 {
                let active = [replay::cadence::should_walk(TABLES[13], hour, false), true];
                let opinion_cap = if active[0] { 16 } else { 0 };
                let tuple_cap = hour as usize % 3;
                let make_memory = || OpinionReads {
                    memory: relations_kit::memory(&bytes),
                    fault,
                    reads: std::cell::Cell::new(0),
                };
                let typed_memory = make_memory();
                let mut opinions = vec![crelation_status::OpinionRow::default(); opinion_cap];
                let mut tuples = vec![crelation_status::TupleRow::default(); tuple_cap];
                let typed = crelation_status::walk(
                    &typed_memory,
                    relations_kit::BASE,
                    relations_kit::GS,
                    &mut opinions,
                    &mut tuples,
                    active,
                );
                let raw_memory = make_memory();
                let report = copy_relations_refresh(
                    &raw_memory,
                    RelationsCopyContext {
                        base: relations_kit::BASE,
                        game_state: relations_kit::GS,
                        active,
                        opinion_cap,
                        tuple_cap,
                    },
                    &mut job,
                );
                assert!(report.raw_ok, "hour={hour} fault={fault}");
                let mut captured = relations_kit::intel_captured(opinion_cap, tuple_cap);
                let mut ticks = [0; 2];
                captured.decode_relations_payload(relations_kit::BASE, &job, &mut ticks);
                let skipped = Outcome::Unavailable {
                    reason: replay::cadence::SKIPPED_REASON,
                };
                assert_eq!(
                    captured.relation_opinions.outcome,
                    if active[0] { typed.opinions } else { skipped }
                );
                assert_eq!(captured.relation_tuples.outcome, typed.tuples);
                assert_eq!(
                    &captured.relation_tuples.rows[..typed.tuples.len()],
                    &tuples[..typed.tuples.len()]
                );
                if active[0] {
                    assert_eq!(
                        &captured.relation_opinions.rows[..typed.opinions.len()],
                        &opinions[..typed.opinions.len()]
                    );
                    assert!(typed_memory.reads.get() > 0 && raw_memory.reads.get() > 0);
                } else {
                    assert_eq!(typed_memory.reads.get(), 0);
                    assert_eq!(raw_memory.reads.get(), 0);
                    assert_eq!(report.copy_ticks[0], 0);
                    assert_eq!(report.raw_bytes[0], 0);
                    assert_eq!(ticks[0], 0);
                }
                assert_eq!(captured.relation_activity(), active);
                captured.fail_relations_capture(active, "accepted worker stopped");
                assert_eq!(
                    captured.relation_opinions.outcome,
                    if active[0] {
                        Outcome::Unavailable {
                            reason: "accepted worker stopped",
                        }
                    } else {
                        skipped
                    }
                );
                assert_eq!(
                    captured.relation_tuples.outcome,
                    Outcome::Unavailable {
                        reason: "accepted worker stopped"
                    }
                );
            }
        }
    }

    #[test]
    fn relations_copy_decode_matches_truncation_with_exact_missing_counts() {
        let bytes = relations_kit::opinionated_fixture();
        let memory = relations_kit::memory(&bytes);
        let direct = relations_kit::direct(&memory, 1, 1);
        assert_eq!(
            direct.opinion_outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(
            direct.tuple_outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        let mut intel = relations_kit::intel_captured(1, 1);
        let (_, _) = relations_kit::split(&memory, &mut intel);
        relations_kit::assert_matches_direct(&intel, &direct);
    }

    #[test]
    fn relations_truncation_matrix_matches_direct_with_skipped_boundary() {
        // B3: zero/one/exact/ample tuple caps run typed AND copy-decode
        // against the SAME memory with the SAME pooled caps, asserting
        // identical outcome (incl. unread_rows) and identical rows. The
        // opinionated fixture interleaves war/subject (skipped) with
        // emitted tuples; caps that fill before a skipped vs before an
        // emitted element must agree on unread_rows because skipped
        // reservations are released identically on both paths.
        let bytes = relations_kit::opinionated_fixture();
        let memory = relations_kit::memory(&bytes);
        for (opinion_cap, tuple_cap) in [(0, 0), (1, 1), (2, 2), (64, 64)] {
            let direct = relations_kit::direct(&memory, opinion_cap, tuple_cap);
            let mut intel = relations_kit::intel_captured(opinion_cap, tuple_cap);
            let (_, _) = relations_kit::split(&memory, &mut intel);
            relations_kit::assert_matches_direct(&intel, &direct);
        }
    }

    #[test]
    fn relations_tuple_auth_tail_preserves_fault_precedence_capacity_and_gates() {
        use engine::{crelation, csubject_relation, cwar_relation};
        use std::cell::RefCell;

        struct TupleReads<'a> {
            memory: SliceMemory<'a>,
            fault: Option<usize>,
            reads: RefCell<Vec<(usize, usize)>>,
        }

        impl Memory for TupleReads<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.reads.borrow_mut().push((address, output.len()));
                if self
                    .fault
                    .is_some_and(|fault| address <= fault && fault < address + output.len())
                {
                    let prefix = output.len().min(3);
                    output[..prefix].fill(0xa5);
                    return false;
                }
                self.memory.copy(address, output)
            }
        }

        fn rest_address(address: usize) -> bool {
            relations_kit::RELATION.iter().any(|pointer| {
                crelation_status::TUPLE_REST_FIELDS
                    .iter()
                    .any(|field| address == pointer + field.source_offset)
            })
        }

        fn tuple_reads(memory: &TupleReads<'_>) -> Vec<(usize, usize)> {
            memory
                .reads
                .borrow()
                .iter()
                .copied()
                .filter(|(address, _)| {
                    *address == relations_kit::serializable_data(0, 3)
                        || relations_kit::RELATION.iter().any(|pointer| {
                            crelation_status::TUPLE_AUTH_FIELDS
                                .iter()
                                .chain(crelation_status::TUPLE_REST_FIELDS.iter())
                                .any(|field| *address == pointer + field.source_offset)
                        })
                })
                .collect()
        }

        fn check(
            bytes: &[u8],
            fault: Option<(usize, bool)>,
            reason: Option<&'static str>,
            case: &str,
        ) {
            // Reuse staging across capacities and gates to exercise its reset too.
            let mut job = RelationsJob::new();
            for cap in [0, 1, 16] {
                for active in [[true, true], [false, true], [true, false]] {
                    let context = format!("{case}, cap={cap}, active={active:?}");
                    let make_memory = || TupleReads {
                        memory: relations_kit::memory(bytes),
                        fault: fault.map(|(address, _)| address),
                        reads: RefCell::new(Vec::new()),
                    };
                    let typed_memory = make_memory();
                    let mut opinions = vec![crelation_status::OpinionRow::default(); cap];
                    let mut tuples = vec![crelation_status::TupleRow::default(); cap];
                    let typed = crelation_status::walk(
                        &typed_memory,
                        relations_kit::BASE,
                        relations_kit::GS,
                        &mut opinions,
                        &mut tuples,
                        active,
                    );
                    for (enabled, outcome) in active.into_iter().zip([typed.opinions, typed.tuples]) {
                        if enabled {
                            if let Some(reason) = reason.filter(|_| active[1]) {
                                assert_eq!(outcome, Outcome::Unavailable { reason }, "{context}");
                            } else {
                                assert!(
                                    matches!(outcome, Outcome::Rows { .. } | Outcome::Truncated { .. }),
                                    "{context}: {outcome:?}"
                                );
                            }
                        }
                    }
                    let raw_memory = make_memory();
                    let report = copy_relations_refresh(
                        &raw_memory,
                        RelationsCopyContext {
                            base: relations_kit::BASE,
                            game_state: relations_kit::GS,
                            active,
                            opinion_cap: cap,
                            tuple_cap: cap,
                        },
                        &mut job,
                    );
                    assert!(report.raw_ok, "{context}");
                    let mut captured = relations_kit::intel_captured(cap, cap);
                    let mut ticks = [0; 2];
                    captured.decode_relations_payload(relations_kit::BASE, &job, &mut ticks);
                    let skipped = Outcome::Unavailable {
                        reason: replay::cadence::SKIPPED_REASON,
                    };
                    assert_eq!(
                        captured.relation_opinions.outcome,
                        if active[0] { typed.opinions } else { skipped },
                        "{context}"
                    );
                    assert_eq!(
                        captured.relation_tuples.outcome,
                        if active[1] { typed.tuples } else { skipped },
                        "{context}"
                    );
                    assert_eq!(
                        &captured.relation_opinions.rows[..typed.opinions.len()],
                        &opinions[..typed.opinions.len()],
                        "{context}"
                    );
                    assert_eq!(
                        &captured.relation_tuples.rows[..typed.tuples.len()],
                        &tuples[..typed.tuples.len()],
                        "{context}"
                    );
                    assert_eq!(captured.relation_activity(), active, "{context}");

                    // Country and matrix acquisition differ; tuple reads must not.
                    let typed_reads = tuple_reads(&typed_memory);
                    let raw_reads = tuple_reads(&raw_memory);
                    assert_eq!(raw_reads, typed_reads, "{context}");
                    if active[1] {
                        if matches!(
                            reason,
                            Some("serializable relation has an unclassified vtable")
                                | Some("relation type word differs from its class")
                        ) {
                            assert!(
                                typed_reads.iter().all(|&(address, _)| !rest_address(address)),
                                "{context}: invalid chunk auth must precede every rest read"
                            );
                        }
                    } else {
                        assert!(typed_reads.is_empty(), "{context}");
                        assert_eq!(report.raw_bytes[1], 0, "{context}");
                        assert_eq!(report.copy_ticks[1], 0, "{context}");
                        assert_eq!(ticks[1], 0, "{context}");
                    }
                    if let Some((fault, attempted)) = fault {
                        for reads in [&typed_reads, &raw_reads] {
                            assert_eq!(
                                reads.iter().any(|&(address, len)| address <= fault && fault < address + len),
                                active[1] && attempted,
                                "{context}: fault attempt"
                            );
                        }
                    }
                }
            }
        }

        fn invalidate(bytes: &mut [u8], index: usize, vtable: bool) -> &'static str {
            let pointer = relations_kit::RELATION[index];
            if vtable {
                relations_kit::p64(bytes, pointer, 0x1234);
                "serializable relation has an unclassified vtable"
            } else {
                relations_kit::p32(bytes, pointer + crelation::TYPE_ID, u32::MAX);
                "relation type word differs from its class"
            }
        }

        for (case, index, skipped) in [
            ("first", 0, None),
            ("tail", 1, None),
            ("war prefix", 1, Some((cwar_relation::VTABLE_RVA, cwar_relation::TYPE))),
            ("subject prefix", 1, Some((csubject_relation::VTABLE_RVA, csubject_relation::TYPE_VALUE))),
        ] {
            let mut valid = relations_kit::opinionated_fixture();
            if let Some((vtable, type_id)) = skipped {
                relations_kit::relation(&mut valid, relations_kit::RELATION[0], vtable, type_id, 1, 3);
            }
            // Valid controls exercise retained fields, ordinal 1 after a skipped
            // predecessor, and exact truncation counts, not just empty error rows.
            check(&valid, None, None, &format!("{case} valid"));
            for vtable in [true, false] {
                let mut bytes = valid.clone();
                let reason = invalidate(&mut bytes, index, vtable);
                check(&bytes, None, Some(reason), &format!("{case}, vtable={vtable}"));
            }
        }

        let valid = relations_kit::opinionated_fixture();
        let first_rest = relations_kit::RELATION[0] + crelation::FIRST_COUNTRY;
        for (fault, reason) in [
            (first_rest, "relation first country unreadable"),
            (relations_kit::RELATION[1], "relation vtable unreadable"),
            (relations_kit::RELATION[1] + crelation::TYPE_ID, "relation type word unreadable"),
        ] {
            check(&valid, Some((fault, true)), Some(reason), "read fault control");
        }
        for vtable in [true, false] {
            let mut bytes = valid.clone();
            let reason = invalidate(&mut bytes, 1, vtable);
            check(&bytes, Some((first_rest, false)), Some(reason), "later auth beats earlier rest read fault");
            relations_kit::p32(&mut bytes, first_rest, 0);
            check(&bytes, None, Some(reason), "later auth beats earlier endpoint error");

            let mut bytes = valid.clone();
            let reason = invalidate(&mut bytes, 0, vtable);
            for fault in [relations_kit::RELATION[1], relations_kit::RELATION[1] + crelation::TYPE_ID] {
                check(&bytes, Some((fault, true)), Some(reason), "earlier complete auth beats later auth read fault");
            }
        }
        let mut bytes = valid;
        invalidate(&mut bytes, 0, true);
        check(
            &bytes,
            Some((relations_kit::RELATION[0] + crelation::TYPE_ID, true)),
            Some("relation type word unreadable"),
            "same-element incomplete auth precedes vtable interpretation",
        );

        // Admit the complete auth chunk before touching any tuple bytes.
        // Insufficient staging declines the optimization before a one-shot
        // native fault could be consumed and then retried by the fallback.
        let mut bytes = relations_kit::opinionated_fixture();
        invalidate(&mut bytes, 1, true);
        for capacity in [0, 1, 2] {
            for active in [[true, true], [false, true], [true, false]] {
                let memory = TupleReads {
                    memory: relations_kit::memory(&bytes),
                    fault: Some(first_rest),
                    reads: RefCell::new(Vec::new()),
                };
                let mut job = RelationsJob::new();
                job.tuples = Vec::with_capacity(capacity);
                let storage = (job.tuples.as_ptr(), job.tuples.capacity());
                let report = copy_relations_refresh(
                    &memory,
                    RelationsCopyContext {
                        base: relations_kit::BASE,
                        game_state: relations_kit::GS,
                        active,
                        opinion_cap: 16,
                        tuple_cap: 16,
                    },
                    &mut job,
                );
                assert_eq!(
                    report.raw_ok,
                    !active[1] || capacity >= 2,
                    "capacity={capacity}, active={active:?}"
                );
                assert_eq!(job.overflowed, active[1] && capacity < 2);
                assert_eq!((job.tuples.as_ptr(), job.tuples.capacity()), storage);
                if active[1] && capacity < 2 {
                    assert!(job.tuples.is_empty());
                    assert!(tuple_reads(&memory).is_empty());
                }
                assert!(
                    tuple_reads(&memory)
                        .iter()
                        .all(|&(address, _)| !rest_address(address))
                );
                if active[1] && capacity >= 2 {
                    assert_eq!(job.tuples.len(), 2);
                    assert_eq!(job.tuples[1].ordinal, 1);
                    assert_eq!(job.tuples[1].auth_read, TupleAuthRead::Complete);
                    assert!(crelation_status::decode_auth(
                        relations_kit::BASE,
                        &job.tuples[1].auth,
                    )
                    .is_err());
                }
                if !active[1] {
                    assert!(job.tuples.is_empty());
                }
            }
        }
    }

    #[test]
    fn relations_tuple_rest_prefix_preserves_fault_precedence_and_staging_bounds() {
        use engine::{crelation, csubject_relation, cwar_relation};
        use std::cell::RefCell;

        const REST: [(usize, &str, TupleRestRead); 6] = [
            (crelation::FIRST_COUNTRY, "relation first country unreadable", TupleRestRead::FirstCountryUnreadable),
            (crelation::SECOND_COUNTRY, "relation second country unreadable", TupleRestRead::SecondCountryUnreadable),
            (crelation::START_HOUR, "relation start hour unreadable", TupleRestRead::StartHourUnreadable),
            (crelation::END_HOUR, "relation end hour unreadable", TupleRestRead::EndHourUnreadable),
            (crelation::CANCEL, "relation cancel byte unreadable", TupleRestRead::CancelUnreadable),
            (crelation::RETIRED, "relation retired byte unreadable", TupleRestRead::RetiredUnreadable),
        ];

        struct TupleReads<'a> {
            memory: SliceMemory<'a>,
            fault: Option<usize>,
            reads: RefCell<Vec<(usize, usize)>>,
        }

        impl Memory for TupleReads<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.reads.borrow_mut().push((address, output.len()));
                if self
                    .fault
                    .is_some_and(|fault| address <= fault && fault < address + output.len())
                {
                    let prefix = output.len().min(3);
                    output[..prefix].fill(0xa5);
                    return false;
                }
                self.memory.copy(address, output)
            }
        }

        fn tuple_reads(memory: &TupleReads<'_>) -> Vec<(usize, usize)> {
            memory
                .reads
                .borrow()
                .iter()
                .copied()
                .filter(|(address, _)| {
                    *address == relations_kit::serializable_data(0, 3)
                        || relations_kit::RELATION.iter().any(|pointer| {
                            crelation_status::TUPLE_AUTH_FIELDS
                                .iter()
                                .chain(crelation_status::TUPLE_REST_FIELDS.iter())
                                .any(|field| *address == pointer + field.source_offset)
                        })
                })
                .collect()
        }

        fn check(
            bytes: &[u8],
            skipped: Option<usize>,
            fault: Option<(usize, usize)>,
            reason: Option<&'static str>,
            case: &str,
        ) {
            let mut job = RelationsJob::new();
            for staging in [0, 1, 2] {
                job.tuples = Vec::with_capacity(staging);
                // Assert actual allocator-visible capacity, not just the request.
                assert_eq!(job.tuples.capacity(), staging);
                job.update_capacity_charge();
                for cap in [0, 1, 16] {
                    let context = format!("{case}, cap={cap}, staging={staging}");
                    let fault_address = fault.map(|(row, field)| relations_kit::RELATION[row] + REST[field].0);
                    let make_memory = || TupleReads {
                        memory: relations_kit::memory(bytes),
                        fault: fault_address,
                        reads: RefCell::new(Vec::new()),
                    };
                    let typed_memory = make_memory();
                    let direct = relations_kit::direct(&typed_memory, cap, cap);
                    if let Some(reason) = reason {
                        assert_eq!(direct.opinion_outcome, Outcome::Unavailable { reason }, "{context}");
                        assert_eq!(direct.tuple_outcome, Outcome::Unavailable { reason }, "{context}");
                    } else {
                        let available = 2 - usize::from(skipped.is_some());
                        assert_eq!(
                            direct.tuple_outcome,
                            if cap < available {
                                Outcome::Truncated { len: cap, unread_rows: (available - cap) as u32 }
                            } else {
                                Outcome::Rows { len: available }
                            },
                            "{context}"
                        );
                        assert_eq!(
                            direct.tuple_rows.iter().map(|row| row.ordinal).collect::<Vec<_>>(),
                            (0..2).filter(|index| Some(*index) != skipped).take(cap).map(|index| index as u32).collect::<Vec<_>>(),
                            "{context}"
                        );
                        if let Some(row) = direct.tuple_rows.iter().find(|row| row.ordinal == 0) {
                            assert_eq!((row.start_hour, row.end_hour, row.cancel, row.retired), (100, -5, 0x80, 0x02));
                        }
                    }
                    let raw_memory = make_memory();
                    let report = copy_relations_refresh(
                        &raw_memory,
                        RelationsCopyContext {
                            base: relations_kit::BASE,
                            game_state: relations_kit::GS,
                            active: [true; 2],
                            opinion_cap: cap,
                            tuple_cap: cap,
                        },
                        &mut job,
                    );
                    assert_eq!(job.tuples.capacity(), staging, "{context}: capture grew staging");
                    assert_eq!(report.raw_ok, staging == 2, "{context}");
                    assert_eq!(job.overflowed, staging < 2, "{context}");
                    assert_eq!(job.tuples.len(), if staging == 2 { 2 } else { 0 }, "{context}");

                    let typed_reads = tuple_reads(&typed_memory);
                    let raw_reads = tuple_reads(&raw_memory);
                    if staging < 2 {
                        // Refuse before touching any tuple pointer/auth/rest byte:
                        // a caller fallback must receive a one-shot fault first.
                        assert!(raw_reads.is_empty(), "{context}: read before admission");
                        continue;
                    }
                    assert_eq!(raw_reads, typed_reads, "{context}: tuple read prefix");
                    for (index, read) in raw_reads.iter().enumerate() {
                        assert!(!raw_reads[..index].contains(read), "{context}: repeated tuple read {read:?}");
                    }
                    if let Some(address) = fault_address {
                        assert_eq!(raw_reads.last(), Some(&(address, crelation_status::TUPLE_REST_FIELDS[fault.unwrap().1].len)), "{context}");
                    }

                    if report.raw_ok {
                        // Both auth blobs remain available even when the first
                        // rest row fails; decode still authenticates this chunk first.
                        for (index, occ) in job.tuples.iter().enumerate() {
                            assert_eq!(occ.ordinal, index as u32, "{context}");
                            assert_eq!(occ.auth_read, TupleAuthRead::Complete, "{context}");
                            let expected = if Some(index) == skipped {
                                TupleRestRead::NotRequested
                            } else if let Some((failed, field)) = fault {
                                if index < failed {
                                    TupleRestRead::Complete
                                } else if index == failed {
                                    REST[field].2
                                } else {
                                    TupleRestRead::NotRequested
                                }
                            } else {
                                TupleRestRead::Complete
                            };
                            assert_eq!(occ.rest_read, expected, "{context}");
                            if expected != TupleRestRead::Complete {
                                assert_eq!(occ.rest, [0; crelation_status::TUPLE_REST_BYTES], "{context}: incomplete bytes retained");
                            }
                        }
                        let mut captured = relations_kit::intel_captured(cap, cap);
                        captured.decode_relations_payload(relations_kit::BASE, &job, &mut [0; 2]);
                        relations_kit::assert_matches_direct(&captured, &direct);
                    }
                    // Insufficient staging was refused above without consuming
                    // tuple reads; only admitted payloads reach decode here.
                }
            }
        }

        let valid = relations_kit::opinionated_fixture();
        check(&valid, None, None, None, "complete rest control");
        for (field, _) in REST.iter().enumerate() {
            for row in 0..2 {
                check(&valid, None, Some((row, field)), Some(REST[field].1), "native rest fault control");
            }
            for (offset, value, reason) in [
                (crelation::FIRST_COUNTRY, 0, "country id is zero"),
                (crelation::FIRST_COUNTRY, 2, "serializable relation is not owned by its first endpoint"),
                (crelation::SECOND_COUNTRY, 0, "country id is zero"),
            ] {
                let mut bytes = valid.clone();
                relations_kit::p32(&mut bytes, relations_kit::RELATION[0] + offset, value);
                check(&bytes, None, Some((1, field)), Some(reason), "earlier complete endpoint error beats later rest fault");
            }
            let mut bytes = valid.clone();
            relations_kit::p32(&mut bytes, relations_kit::RELATION[0] + crelation::FIRST_COUNTRY, 0);
            check(&bytes, None, Some((0, field)), Some(REST[field].1), "same-row incomplete rest precedes endpoint interpretation");
        }
        for (vtable, type_id) in [
            (cwar_relation::VTABLE_RVA, cwar_relation::TYPE),
            (csubject_relation::VTABLE_RVA, csubject_relation::TYPE_VALUE),
        ] {
            for skipped in 0..2 {
                let mut bytes = valid.clone();
                relations_kit::relation(&mut bytes, relations_kit::RELATION[skipped], vtable, type_id, 1, 3);
                check(&bytes, Some(skipped), None, None, "skipped prefix or tail control");
                for (field, _) in REST.iter().enumerate() {
                    check(&bytes, Some(skipped), Some((1 - skipped, field)), Some(REST[field].1), "rest fault beside skipped prefix or tail");
                }
            }
        }
    }

    // Compile-time proof on the tested target that the richer status preserves
    // the previous occurrence size/alignment, including its unchanged auth status.
    #[test]
    fn relations_tuple_admission_preserves_one_shot_fault_through_caller_fallback() {
        use engine::crelation;
        use std::cell::{Cell, RefCell};

        struct OneShot<'a> {
            memory: SliceMemory<'a>,
            fault: usize,
            armed: Cell<bool>,
            attempts: Cell<usize>,
            reads: RefCell<Vec<(usize, usize)>>,
        }

        impl Memory for OneShot<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.reads.borrow_mut().push((address, output.len()));
                if address <= self.fault && self.fault < address + output.len() {
                    self.attempts.set(self.attempts.get() + 1);
                    if self.armed.replace(false) {
                        let prefix = output.len().min(3);
                        output[..prefix].fill(0xa5);
                        return false;
                    }
                }
                self.memory.copy(address, output)
            }
        }

        fn tuple_reads(memory: &OneShot<'_>) -> Vec<(usize, usize)> {
            memory
                .reads
                .borrow()
                .iter()
                .copied()
                .filter(|(address, _)| {
                    *address == relations_kit::serializable_data(0, 3)
                        || relations_kit::RELATION.iter().any(|pointer| {
                            crelation_status::TUPLE_AUTH_FIELDS
                                .iter()
                                .chain(crelation_status::TUPLE_REST_FIELDS.iter())
                                .any(|field| *address == pointer + field.source_offset)
                        })
                })
                .collect()
        }

        let bytes = relations_kit::opinionated_fixture();
        for row in 0..2 {
            for (offset, reason) in [
                (0, "relation vtable unreadable"),
                (crelation::TYPE_ID, "relation type word unreadable"),
                (crelation::FIRST_COUNTRY, "relation first country unreadable"),
                (crelation::SECOND_COUNTRY, "relation second country unreadable"),
                (crelation::START_HOUR, "relation start hour unreadable"),
                (crelation::END_HOUR, "relation end hour unreadable"),
                (crelation::CANCEL, "relation cancel byte unreadable"),
                (crelation::RETIRED, "relation retired byte unreadable"),
            ] {
                let make_memory = || OneShot {
                    memory: relations_kit::memory(&bytes),
                    fault: relations_kit::RELATION[row] + offset,
                    armed: Cell::new(true),
                    attempts: Cell::new(0),
                    reads: RefCell::new(Vec::new()),
                };
                let reference_memory = make_memory();
                let direct = relations_kit::direct(&reference_memory, 16, 16);
                assert_eq!(direct.opinion_outcome, Outcome::Unavailable { reason });
                assert_eq!(direct.tuple_outcome, Outcome::Unavailable { reason });
                assert_eq!(reference_memory.attempts.get(), 1);
                let reference_reads = tuple_reads(&reference_memory);

                for staging in [0, 1, 2] {
                    let context = format!("row={row}, offset={offset:#x}, staging={staging}");
                    let memory = make_memory();
                    let mut job = RelationsJob::new();
                    job.tuples = Vec::with_capacity(staging);
                    // A captured fault in owner zero must finish the refresh.
                    // Trying owner one would exhaust metadata and request fallback.
                    job.metas = Vec::with_capacity(1);
                    job.update_capacity_charge();
                    assert_eq!((job.tuples.capacity(), job.metas.capacity()), (staging, 1));
                    let report = copy_relations_refresh(
                        &memory,
                        RelationsCopyContext {
                            base: relations_kit::BASE,
                            game_state: relations_kit::GS,
                            active: [true; 2],
                            opinion_cap: 16,
                            tuple_cap: 16,
                        },
                        &mut job,
                    );
                    assert_eq!(report.raw_ok, staging == 2, "{context}");
                    assert_eq!(job.overflowed, staging < 2, "{context}");
                    assert_eq!((job.tuples.capacity(), job.metas.capacity()), (staging, 1));

                    if report.raw_ok {
                        assert_eq!(job.metas.len(), 1, "{context}");
                        assert_eq!(memory.attempts.get(), 1, "{context}");
                        let mut captured = relations_kit::intel_captured(16, 16);
                        captured.decode_relations_payload(relations_kit::BASE, &job, &mut [0; 2]);
                        relations_kit::assert_matches_direct(&captured, &direct);
                    } else {
                        assert!(job.tuples.is_empty(), "{context}");
                        assert!(tuple_reads(&memory).is_empty(), "{context}: tuple read before admission");
                        assert_eq!(memory.attempts.get(), 0, "{context}: consumed fault before fallback");
                        // Emulate the real caller: only a refused raw payload takes
                        // the typed fallback, on the SAME observation-bearing Memory.
                        let fallback = relations_kit::direct(&memory, 16, 16);
                        assert_eq!(fallback.opinion_outcome, direct.opinion_outcome, "{context}");
                        assert_eq!(fallback.tuple_outcome, direct.tuple_outcome, "{context}");
                        assert_eq!(fallback.opinion_rows, direct.opinion_rows, "{context}");
                        assert_eq!(fallback.tuple_rows, direct.tuple_rows, "{context}");
                    }
                    assert_eq!(memory.attempts.get(), 1, "{context}: fault address retried");
                    assert!(!memory.armed.get(), "{context}: fault was never observed");
                    assert_eq!(tuple_reads(&memory), reference_reads, "{context}: tuple prefix differs");
                }
            }
        }
    }

    #[test]
    fn relations_tuple_rest_status_preserves_occurrence_layout() {
        #[allow(dead_code)]
        struct BeforeRestStatus {
            owner_id: u32,
            owner_canonical: u32,
            target_country: u32,
            ordinal: u32,
            auth: [u8; crelation_status::TUPLE_AUTH_BYTES],
            auth_read: TupleAuthRead,
            rest: [u8; crelation_status::TUPLE_REST_BYTES],
            rest_ok: bool,
        }
        const _: () = {
            assert!(std::mem::size_of::<TupleRestRead>() == std::mem::size_of::<bool>());
            assert!(std::mem::align_of::<TupleRestRead>() == std::mem::align_of::<bool>());
            assert!(std::mem::size_of::<TupleOcc>() == std::mem::size_of::<BeforeRestStatus>());
            assert!(std::mem::align_of::<TupleOcc>() == std::mem::align_of::<BeforeRestStatus>());
        };
    }

    #[test]
    fn relations_copy_decode_matches_faults_with_identical_reasons() {
        use engine::{csubject_relation, cwar_relation};
        let valid = relations_kit::opinionated_fixture();
        // Each mutation faults the whole family; both paths must agree on the
        // outcome reason while the other partition stays consistent.
        type Fault = (&'static str, fn(&mut Vec<u8>));
        let cases: &[Fault] = &[
            ("relation status vtable mismatch", |bytes: &mut Vec<u8>| {
                relations_kit::p64(bytes, relations_kit::status(0, 2), 0);
            }),
            ("null relation matrix entry", |bytes: &mut Vec<u8>| {
                relations_kit::p64(bytes, relations_kit::matrix_data(0) + 2 * 8, 0);
            }),
            (
                "relation status target differs from matrix slot",
                |bytes: &mut Vec<u8>| {
                    relations_kit::p32(
                        bytes,
                        relations_kit::status(0, 2) + crelation_status::TARGET_COUNTRY,
                        3,
                    );
                },
            ),
            (
                "relation matrix does not cover country ordinals",
                |bytes: &mut Vec<u8>| {
                    relations_kit::p32(
                        bytes,
                        relations_kit::DIPLOMACY[0] + cdiplomacy_status::RELATIONS + 12,
                        2,
                    );
                },
            ),
            ("diplomacy vtable mismatch", |bytes: &mut Vec<u8>| {
                relations_kit::p64(bytes, relations_kit::DIPLOMACY[1], 0);
            }),
            ("country diplomacy is null", |bytes: &mut Vec<u8>| {
                relations_kit::p64(
                    bytes,
                    relations_kit::COUNTRIES[1] + ccountry::DIPLOMACY_STATUS,
                    0,
                );
            }),
            (
                "serializable relation vector invalid",
                |bytes: &mut Vec<u8>| {
                    relations_kit::p32(
                        bytes,
                        relations_kit::status(0, 1) + crelation_status::SERIALIZABLE_RELATIONS + 12,
                        9,
                    );
                },
            ),
            (
                "serializable relation has an unclassified vtable",
                |bytes: &mut Vec<u8>| {
                    relations_kit::p64(bytes, relations_kit::RELATION[0], 0x1234);
                },
            ),
            (
                "relation type word differs from its class",
                |bytes: &mut Vec<u8>| {
                    relations_kit::p32(
                        bytes,
                        relations_kit::RELATION[0] + engine::crelation::TYPE_ID,
                        1,
                    );
                },
            ),
            ("null serializable relation entry", |bytes: &mut Vec<u8>| {
                relations_kit::p64(bytes, relations_kit::serializable_data(0, 3), 0);
            }),
        ];
        for (reason, break_it) in cases {
            let mut bytes = valid.clone();
            break_it(&mut bytes);
            let memory = relations_kit::memory(&bytes);
            let direct = relations_kit::direct(&memory, 16, 16);
            let mut intel = relations_kit::intel_captured(16, 16);
            let (report, _) = relations_kit::split(&memory, &mut intel);
            assert!(report.raw_ok, "{reason}");
            assert_eq!(
                intel.relation_opinions.outcome, direct.opinion_outcome,
                "{reason}"
            );
            assert_eq!(
                intel.relation_tuples.outcome, direct.tuple_outcome,
                "{reason}"
            );
            // War/subject skip faults still authenticate: exercised below.
            let _ = (cwar_relation::VTABLE_RVA, csubject_relation::VTABLE_RVA);
        }
    }

    #[test]
    fn relations_war_and_subject_elements_are_skipped_but_consume_ordinals() {
        use engine::{cguarantee_relation, csubject_relation, cwar_relation};
        let mut bytes = relations_kit::fixture();
        relations_kit::relation(
            &mut bytes,
            relations_kit::RELATION[0],
            cwar_relation::VTABLE_RVA,
            cwar_relation::TYPE,
            1,
            3,
        );
        relations_kit::relation(
            &mut bytes,
            relations_kit::RELATION[1],
            cguarantee_relation::VTABLE_RVA,
            cguarantee_relation::TYPE,
            1,
            3,
        );
        let subject = 0x1d000;
        let market = 0x1d800;
        relations_kit::relation(
            &mut bytes,
            subject,
            csubject_relation::VTABLE_RVA,
            csubject_relation::TYPE_VALUE,
            1,
            3,
        );
        relations_kit::relation(
            &mut bytes,
            market,
            engine::cmarket_access_rights_relation::VTABLE_RVA,
            engine::cmarket_access_rights_relation::TYPE,
            1,
            3,
        );
        relations_kit::array(
            &mut bytes,
            relations_kit::status(0, 3) + crelation_status::SERIALIZABLE_RELATIONS,
            relations_kit::serializable_data(0, 3),
            &[
                relations_kit::RELATION[0] as u64,
                relations_kit::RELATION[1] as u64,
                subject as u64,
                market as u64,
            ],
        );
        let memory = relations_kit::memory(&bytes);
        let direct = relations_kit::direct(&memory, 16, 16);
        assert_eq!(direct.tuple_outcome, Outcome::Rows { len: 2 });
        assert_eq!(
            direct
                .tuple_rows
                .iter()
                .map(|row| (row.type_id, row.ordinal))
                .collect::<Vec<_>>(),
            vec![
                (cguarantee_relation::TYPE, 1),
                (engine::cmarket_access_rights_relation::TYPE, 3),
            ]
        );
        let mut intel = relations_kit::intel_captured(16, 16);
        let (_, _) = relations_kit::split(&memory, &mut intel);
        relations_kit::assert_matches_direct(&intel, &direct);
    }

    #[test]
    fn relations_invalid_primary_bytes_preserve_unavailable_rows() {
        // A readable but invalid diplomacy vtable retains the typed error.
        let (mut bytes, ()) = (relations_kit::fixture(), ());
        bytes[relations_kit::DIPLOMACY[0]] ^= 0xff;
        let memory = relations_kit::memory(&bytes);
        let direct = relations_kit::direct(&memory, 16, 16);
        assert!(matches!(
            direct.opinion_outcome,
            Outcome::Unavailable { .. }
        ));
        let mut intel = relations_kit::intel_captured(16, 16);
        let (report, _) = relations_kit::split(&memory, &mut intel);
        assert!(report.raw_ok);
        assert_eq!(intel.relation_opinions.outcome, direct.opinion_outcome);
    }

    #[test]
    fn relations_staging_overflow_reports_fallback_without_blocking() {
        let bytes = relations_kit::opinionated_fixture();
        let memory = relations_kit::memory(&bytes);
        let intel = relations_kit::intel_captured(16, 16);
        // Zero-capacity staging cannot hold any copy: the report declines the
        // payload so the caller falls back to the typed walk.
        let mut tiny = RelationsJob {
            active: [true; 2],
            countries: Vec::new(),
            links: Vec::new(),
            highest_slot: 0,
            metas: Vec::new(),
            slot_pool: Vec::new(),
            slot_used: 0,
            tuples: Vec::new(),
            reader_err: None,
            country_err: None,
            top_err: None,
            unread_opinions: 0,
            unread_tuples: 0,
            overflowed: false,
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        let ctx = RelationsCopyContext {
            base: relations_kit::BASE,
            game_state: relations_kit::GS,
            active: [true; 2],
            opinion_cap: intel.opinion_rows_cap(),
            tuple_cap: intel.tuple_rows_cap(),
        };
        let report = copy_relations_refresh(&memory, ctx, &mut tiny);
        assert!(!report.raw_ok);
        // The typed path still reads the same fixture cleanly.
        let direct = relations_kit::direct(&memory, 16, 16);
        assert_eq!(direct.opinion_outcome, Outcome::Rows { len: 2 });
    }

    // ---- Hourly capitulation split (Capitulation) ----

    const CAP_BASE: usize = 0;
    const CAP_GAME: usize = 0x1000;
    const CAP_COUNTRIES: usize = 0x2000;
    const CAP_LINKS: usize = 0x2100;
    const CAP_COUNTRY: usize = 0x3000;
    const CAP_DIPLO: usize = 0x4000;

    fn cap_put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address - CAP_GAME..address - CAP_GAME + value.len()].copy_from_slice(value);
    }

    fn cap_array(bytes: &mut [u8], header: usize, data: usize, count: u32) {
        cap_put(bytes, header, &(data as u64).to_le_bytes());
        cap_put(bytes, header + 8, &count.to_le_bytes());
        cap_put(bytes, header + 12, &count.to_le_bytes());
    }

    fn cap_fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x4000];
        cap_array(
            &mut bytes,
            CAP_GAME + cgame_state::COUNTRIES,
            CAP_COUNTRIES,
            2,
        );
        cap_array(
            &mut bytes,
            CAP_GAME + cgame_state::COUNTRY_LINKS,
            CAP_LINKS,
            2,
        );
        cap_put(&mut bytes, CAP_COUNTRIES, &0_u64.to_le_bytes());
        cap_put(
            &mut bytes,
            CAP_COUNTRIES + 8,
            &(CAP_COUNTRY as u64).to_le_bytes(),
        );
        cap_put(&mut bytes, CAP_LINKS, &0_u32.to_le_bytes());
        cap_put(&mut bytes, CAP_LINKS + 4, &1_u32.to_le_bytes());
        cap_put(
            &mut bytes,
            CAP_COUNTRY,
            &(ccountry::VTABLE_RVA as u64).to_le_bytes(),
        );
        cap_put(&mut bytes, CAP_COUNTRY + ccountry::ID, &1_u32.to_le_bytes());
        cap_put(
            &mut bytes,
            CAP_COUNTRY + ccountry::DIPLOMACY_STATUS,
            &(CAP_DIPLO as u64).to_le_bytes(),
        );
        cap_put(
            &mut bytes,
            CAP_DIPLO,
            &(cdiplomacy_status::VTABLE_RVA as u64).to_le_bytes(),
        );
        cap_put(
            &mut bytes,
            CAP_DIPLO + cdiplomacy_status::COUNTRY,
            &(CAP_COUNTRY as u64).to_le_bytes(),
        );
        cap_put(
            &mut bytes,
            CAP_DIPLO + cdiplomacy_status::LEGITIMACY,
            &(-125_682_i64).to_le_bytes(),
        );
        cap_put(
            &mut bytes,
            CAP_DIPLO + cdiplomacy_status::LEGITIMACY + 8,
            &42_i32.to_le_bytes(),
        );
        cap_put(
            &mut bytes,
            CAP_DIPLO + cdiplomacy_status::LAST_SURRENDER_HOUR,
            &(-1_i32).to_le_bytes(),
        );
        cap_put(
            &mut bytes,
            CAP_DIPLO + cdiplomacy_status::FACTION_JOIN_HOUR,
            &7_i32.to_le_bytes(),
        );
        cap_put(&mut bytes, CAP_DIPLO + cdiplomacy_status::CAPITULATED, &[1]);
        cap_put(
            &mut bytes,
            CAP_DIPLO + cdiplomacy_status::CAPITULATED_HOUR,
            &i32::MIN.to_le_bytes(),
        );
        bytes
    }

    fn cap_capacities() -> Capacities {
        let mut capacities = Capacities {
            rows: [2; N],
            intel_scratch: 2 * engine::country_intel_row::STRIDE,
            subject_names: 8,
            autonomy_names: 8,
            variable_entries: 4,
            variable_names: 8,
            tokens: 8,
        };
        capacities.rows[0] = 2;
        capacities
    }

    fn cap_direct(memory: &SliceMemory<'_>) -> Captured {
        let capacities = cap_capacities();
        let mut captured = Captured::with_capacities(&capacities);
        // Typed reference path for slot 0 only; other slots stay reset.
        captured.capitulation.outcome = bounded(
            cdiplomacy_status::walk_capitulation(
                memory,
                CAP_BASE,
                CAP_GAME,
                &mut captured.capitulation.rows,
            ),
            captured.capitulation.rows.len(),
        );
        captured
    }

    fn cap_split(memory: &SliceMemory<'_>) -> (Captured, CapitulationCopyReport) {
        let capacities = cap_capacities();
        let mut captured = Captured::with_capacities(&capacities);
        let ctx = CapitulationCopyContext {
            base: CAP_BASE,
            game_state: CAP_GAME,
        };
        let mut first = CapitulationJob::new();
        let report = copy_capitulation_refresh(memory, ctx, &mut first);
        assert!(report.raw_ok);
        captured.capitulation_raw_mode = report.raw_ok;
        let mut ticks = [0u64; 1];
        captured.decode_capitulation_payload(CAP_BASE, &first, &mut ticks);
        (captured, report)
    }

    #[test]
    fn owned_intel_failures_preserve_other_tables_and_capacities() {
        let mut captured = Captured::with_capacities(&cap_capacities());
        captured.capitulation.outcome = Outcome::Rows { len: 1 };
        captured.relation_opinions.outcome = Outcome::Rows { len: 1 };
        captured.relation_tuples.outcome = Outcome::Rows { len: 1 };
        captured.subjects.outcome = Outcome::Rows { len: 1 };
        captured.capitulation_raw_mode = true;
        captured.relations_raw_mode = true;
        let bytes = captured.allocated_bytes();
        let mut outcomes = captured.outcomes();
        captured.fail_capitulation_capture("capitulation worker failed");
        outcomes[0] = Outcome::Unavailable {
            reason: "capitulation worker failed",
        };
        assert_eq!(captured.outcomes(), outcomes);
        assert!(!captured.capitulation_raw_mode);
        assert!(captured.relations_raw_mode);
        captured.fail_relations_capture([true; 2], "relations worker failed");
        for slot in [13, 14] {
            outcomes[slot] = Outcome::Unavailable {
                reason: "relations worker failed",
            };
        }
        assert_eq!(captured.outcomes(), outcomes);
        assert!(!captured.relations_raw_mode);
        assert_eq!(captured.allocated_bytes(), bytes);
    }

    #[test]
    fn capitulation_split_copy_matches_direct_walk() {
        let bytes = cap_fixture();
        let memory = SliceMemory {
            base: CAP_GAME,
            bytes: &bytes,
        };
        let direct = cap_direct(&memory);
        let (split, report) = cap_split(&memory);
        assert!(report.raw_ok);
        assert_eq!(split.capitulation.outcome, direct.capitulation.outcome);
        let len = split.capitulation.outcome.len();
        assert_eq!(
            &split.capitulation.rows[..len],
            &direct.capitulation.rows[..len]
        );
        assert_eq!(split.capitulation.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.capitulation.rows[0].country, 1);
        assert_eq!(split.capitulation.rows[0].capitulated, 1);
        assert_eq!(split.capitulation.rows[0].legitimacy_raw, -125_682);
    }

    #[test]
    fn capitulation_split_copy_matches_direct_walk_under_truncation() {
        let bytes = cap_fixture();
        let memory = SliceMemory {
            base: CAP_GAME,
            bytes: &bytes,
        };
        let mut capacities = cap_capacities();
        capacities.rows[0] = 0;
        let mut direct = Captured::with_capacities(&capacities);
        direct.capitulation.outcome = bounded(
            cdiplomacy_status::walk_capitulation(
                &memory,
                CAP_BASE,
                CAP_GAME,
                &mut direct.capitulation.rows,
            ),
            direct.capitulation.rows.len(),
        );
        assert!(matches!(
            direct.capitulation.outcome,
            Outcome::Truncated { .. }
        ));
        let mut split = Captured::with_capacities(&capacities);
        let ctx = CapitulationCopyContext {
            base: CAP_BASE,
            game_state: CAP_GAME,
        };
        let mut first = CapitulationJob::new();
        let report = copy_capitulation_refresh(&memory, ctx, &mut first);
        assert!(report.raw_ok);
        split.capitulation_raw_mode = true;
        let mut ticks = [0u64; 1];
        split.decode_capitulation_payload(CAP_BASE, &first, &mut ticks);
        assert_eq!(split.capitulation.outcome, direct.capitulation.outcome);
    }

    #[test]
    fn capitulation_split_copy_matches_direct_walk_on_fault_matrix() {
        let mut bytes = cap_fixture();
        cap_put(&mut bytes, CAP_DIPLO, &0_u64.to_le_bytes());
        let memory = SliceMemory {
            base: CAP_GAME,
            bytes: &bytes,
        };
        let direct = cap_direct(&memory);
        assert!(matches!(
            direct.capitulation.outcome,
            Outcome::Unavailable { .. }
        ));
        let (split, report) = cap_split(&memory);
        assert!(report.raw_ok);
        assert_eq!(split.capitulation.outcome, direct.capitulation.outcome);
    }

    #[test]
    fn capitulation_staging_overflow_falls_back_to_typed_walk() {
        let bytes = cap_fixture();
        let memory = SliceMemory {
            base: CAP_GAME,
            bytes: &bytes,
        };
        let ctx = CapitulationCopyContext {
            base: CAP_BASE,
            game_state: CAP_GAME,
        };
        let mut tiny = CapitulationJob {
            log: crate::raw::RawLog::new(8, 8),
            game_state: 0,
        };
        let report = copy_capitulation_refresh(&memory, ctx, &mut tiny);
        assert!(!report.raw_ok);
        assert!(tiny.log.overflowed);
        assert!(tiny.grow_for_next());
    }

    #[test]
    fn capitulation_gate_skips_typed_walk_when_raw_mode_set() {
        let capacities = cap_capacities();
        let mut captured = Captured::with_capacities(&capacities);
        captured.capitulation_raw_mode = true;
        let bytes = cap_fixture();
        let memory = SliceMemory {
            base: CAP_GAME,
            bytes: &bytes,
        };
        let ticks = captured.capture(&memory, CAP_BASE, CAP_GAME, 0, false);
        assert_eq!(ticks[0], 0);
    }

    #[test]
    fn capitulation_worst_case_pools_start_above_late_campaign_caps() {
        let job = CapitulationJob::new();
        assert!(job.log.spans.capacity() >= (1 << 14));
        assert!(job.log.bytes.capacity() >= (4 << 20));
        let capacities = Capacities::default();
        assert!(capacities.rows[0] >= 512);
    }
}
