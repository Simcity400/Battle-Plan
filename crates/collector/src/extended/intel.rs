//! Intel acquisition buffers. Allocation, growth and replay conversion are off-thread.

use super::daily_intel::{Home, Loan, Walk};
use crate::{
    raw::RawLog,
    read_accounting,
    sampling::qpc,
    slicing::{SliceFingerprints, SliceSelection},
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, bounded, grow_vec},
};
use engine::allocation::{copy_vec, default_vec, map_slice, owned_vec, reserve, reserve_exact};
use engine::{
    Memory, ccharacter, ccharacter_manager, ccountry, ccountry_intel, ccountry_intel_network,
    ccountry_operation_manager, ccountry_operation_token_manager, ccryptology,
    ccurrent_autonomy_status, cdiplomacy_status, cflag_manager, cgame_state, cintelligence_agency,
    coperation_instance, coperative_leader,
    cpdx_array::Array,
    crelation_status, cscript_flag, cstate,
    csubject_relation::{self, Name},
    cvariables, cwar_relation, incoming_diplomatic_action_queue, operation_assigned_operative,
    read, read_slice,
    walk::Outcome,
};
use replay::{
    BranchId,
    frame::{BaselineToken, TableInput, TableView},
    rows::{self, TableId, TableRows, tables},
};
use std::mem::size_of;
use std::sync::Arc;

pub const N: usize = 17;
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
    TableId::CapturedOperativeReferences,
];

const UNCAPTURED: Outcome = Outcome::Unavailable {
    reason: "clock unavailable",
};

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
/// A character's own flags (`CCharacter+0x110`) and script variables
/// (`CCharacter+0xD8`), keyed by its id (`CCharacter+0xC`, the Characters
/// id).
pub const CHARACTER_SCOPE: u8 = 3;

#[derive(Default)]
pub(crate) struct SeedWork {
    captured: u64,
    refused: u64,
}

impl SeedWork {
    pub(crate) fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::status!({
            "captured": self.captured,
            "refused": self.refused,
            "refusals": crate::status::map(std::iter::once((
                "script variable random seed unreadable", self.refused,
            )).filter(|(_, count)| *count != 0)),
            "scope": "hourly CVariables+8 for admitted ScriptVariables rows; unread seeds remain absent while the entries and scope rows stand",
        })
    }
}

/// Spool-owned membership scratch, not another captured-frame lease. Bits are
/// cleared for each comparison, and contain no pointers, names or prior values.
#[derive(Default)]
pub(crate) struct ScriptVariableComparison {
    scopes: Vec<u64>,
    entries: Vec<u64>,
}

fn clear_seen(bits: &mut Vec<u64>, count: usize) -> bool {
    let words = count.div_ceil(64);
    if reserve(bits, words.saturating_sub(bits.len())).is_err() {
        return false;
    }
    bits.resize(words, 0);
    bits.fill(0);
    true
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
    /// Bytes of scratch for the bulk intel-record copies, held with the shared
    /// CountryIntel rows. This is a fixed copy window, not a row capacity, so
    /// `grow` never raises it.
    pub intel_scratch: usize,
    pub subject_names: usize,
    pub autonomy_names: usize,
    pub agency_names: usize,
    pub variable_entries: usize,
    pub variable_names: usize,
    pub tokens: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: [
                512, 256, 256, 512, 4096, 4096, 16_384, 8192, 2048, 2048, 8192, 4096, 512, 16_384,
                4096, 65_536, 0,
            ],
            intel_scratch: 16_384,
            subject_names: 16_384,
            autonomy_names: 32_768,
            agency_names: 32_768,
            variable_entries: 16_384,
            variable_names: 512 * 1024,
            tokens: 32_768,
        }
    }
}
impl Capacities {
    /// At most one row per canonical country for Capitulation, Subjects,
    /// Autonomy and IntelligenceAgencies; ScriptVariables has one scope row
    /// for the global scope, every country and every state (and every
    /// character, which [`Self::raise_for_script_variables`] counts).
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        use crate::country_scale::{raise, rows};
        for index in [0, 1, 2, 3] {
            raise(&mut self.rows[index], rows(slots, 1));
        }
        let states = Self::default().rows[8];
        raise(&mut self.rows[8], rows(slots, 1).saturating_add(states));
    }

    /// CountryIntel keeps at most one row per native record, and every owner's
    /// vector is indexed by target slot, so the live dense total
    /// (`ccountry_intel::native_rows`) bounds the first hour. Nonzero intel is
    /// dense among live countries (~0.9 x live^2 rows), so the sparse table
    /// needs close to that bound when every state is its own country.
    pub(crate) fn raise_for_country_intel(&mut self, native_rows: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[15], rows(native_rows, 1));
    }

    pub(crate) fn raise_for_captured_operatives(&mut self, count: usize) {
        crate::country_scale::raise(&mut self.rows[16], crate::country_scale::rows(count, 1));
    }

    /// ScriptFlags keeps one row per live flag of every scope, which the
    /// managers' own counts give (`script_flag_demand`); the per-manager
    /// scratch follows the same capacity.
    pub(crate) fn raise_for_script_flags(&mut self, flags: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[7], rows(flags, 1));
    }

    /// ScriptVariables keeps one row per walked store and every store's
    /// entries and key bytes, which the stores' own headers give
    /// (`script_variable_demand`).
    pub(crate) fn raise_for_script_variables(&mut self, demand: VariableDemand) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[8], rows(demand.scopes, 1));
        raise(&mut self.variable_entries, rows(demand.entries, 1));
        raise(&mut self.variable_names, rows(demand.bytes, 1));
    }
}

/// The live ScriptVariables demand: its stores, entries and key bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VariableDemand {
    pub scopes: usize,
    pub entries: usize,
    pub bytes: usize,
}

impl VariableDemand {
    /// The field-wise maximum.
    pub fn max(self, other: Self) -> Self {
        Self {
            scopes: self.scopes.max(other.scopes),
            entries: self.entries.max(other.entries),
            bytes: self.bytes.max(other.bytes),
        }
    }
}

/// Sizing only: every ScriptVariables store, entry and key byte, from each
/// store's header, through the walk itself with no room for rows.
pub(crate) fn script_variable_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Option<VariableDemand> {
    let walk = cvariables::walk_scopes(memory, base, state, &mut [], &mut [], &mut []);
    let scopes = match walk.outcome {
        Outcome::Rows { len } => len,
        Outcome::Truncated { len, unread_rows } => len + unread_rows as usize,
        Outcome::Unavailable { .. } => return None,
    };
    Some(VariableDemand {
        scopes,
        entries: walk.required_entries,
        bytes: walk.required_bytes,
    })
}

/// Sizing only: the live flag count of every ScriptFlags scope, from each
/// manager's header, through the walk itself with no room for rows.
pub(crate) fn script_flag_demand(memory: &impl Memory, base: usize, state: usize) -> Option<usize> {
    match walk_flags(memory, base, state, &mut [], &mut []) {
        Outcome::Rows { len } => Some(len),
        Outcome::Truncated { unread_rows, .. } => Some(unread_rows as usize),
        Outcome::Unavailable { .. } => None,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ScopedFlag {
    scope_kind: u8,
    scope_id: u32,
    flag: cscript_flag::Row,
}

/// Spool-owned, fieldwise native snapshot of ScriptFlags, which has no
/// staggered cache, so the memo owns its immutable replay result directly.
/// Neither capture leases nor live pointers survive conversion.
#[derive(Default)]
struct NativeShadow<R> {
    rows: Vec<R>,
    output: Option<Arc<TableRows>>,
    hits: u64,
    misses: u64,
}

impl<R: Copy + PartialEq> NativeShadow<R> {
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
            if reserve(&mut self.rows, source.len()).is_err() {
                return unavailable("collector allocation failed").into();
            }
            self.rows.extend_from_slice(source);
            self.output = Some(Arc::clone(&output));
            return TableInput::SharedComplete(output);
        }
        TableInput::Sampled(converted)
    }

    fn statistics(&self) -> IntelShadowStats {
        let replay_capacity_bytes = match self.output.as_deref() {
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
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct IntelShadowStats {
    pub hits: u64,
    pub misses: u64,
    pub row_capacity_bytes: usize,
    pub replay_capacity_bytes: usize,
}

impl IntelShadow {
    pub(crate) fn statistics(&self) -> IntelShadowStats {
        self.flags.statistics()
    }
}

pub(crate) struct IntelConversion<'a> {
    shadow: &'a mut IntelShadow,
}

impl<'a> IntelConversion<'a> {
    pub(crate) fn new(branch: BranchId, shadow: &'a mut IntelShadow) -> Self {
        if shadow.branch != Some(branch) {
            shadow.flags.clear();
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
    captured_operatives: CaptureRows<cintelligence_agency::CapturedOperativeRow>,
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
    /// Dense country authentication cells for the typed relation walk, sized
    /// by `grow` from the census and link headers the last walk observed.
    relation_resolver: crelation_status::Resolver,
    /// Owned-membership snapshot and resolver for the subject walk, sized by
    /// `grow` from the last walk's observed demand. Empty storage is correct.
    subject_snapshot: cwar_relation::OwnedSnapshotScratch,
    /// The pool's shared CountryIntel rows, held from the walk until the spool
    /// grows this frame; see `daily_intel`.
    country_intel: Loan,
    country_intel_outcome: Outcome,
    subject_names: Arena<u8>,
    autonomy_names: Arena<u8>,
    agency_names: Arena<u8>,
    variable_entries: Arena<cvariables::Row>,
    variable_names: Arena<u8>,
    tokens: Arena<u32>,
    flag_scratch: Vec<cscript_flag::Row>,
    /// Worker-payload gate for the relations family. Set by the game thread
    /// on refresh hours with a valid payload; `capture` then skips the typed
    /// relations walk and the decode worker installs these rows before the
    /// ready handoff. Preserved across `reset` (set per refresh, cleared
    /// by the worker after decode).
    pub(crate) relations_raw_mode: bool,
    /// Which owners this hour's relation rows cover (`slicing::RELATIONS`).
    /// Loaned to the relations job from dispatch to its join.
    relations_selection: SliceSelection,
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
        Self::build(
            &Capacities {
                rows: [0; N],
                intel_scratch: 0,
                subject_names: 0,
                autonomy_names: 0,
                agency_names: 0,
                variable_entries: 0,
                variable_names: 0,
                tokens: 0,
            },
            Loan::default(),
        )
    }

    /// Every slice and returned array uses local TABLES order. The frame gets
    /// its own CountryIntel buffer until `share_country_intel`.
    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self::build(
            capacity,
            Home::try_new(capacity.rows[15], capacity.intel_scratch)
                .map(Loan::new)
                .unwrap_or_default(),
        )
    }

    /// The CountryIntel buffer this frame uses.
    pub(crate) fn country_intel_home(&self) -> Option<&Home> {
        self.country_intel.home()
    }

    /// Use the pool's CountryIntel buffer instead of this frame's own.
    pub(crate) fn share_country_intel(&mut self, home: &Home) {
        self.country_intel = Loan::new(home.clone());
    }

    fn build(capacity: &Capacities, country_intel: Loan) -> Self {
        Self {
            capitulation: CaptureRows::new(capacity.rows[0]),
            subjects: CaptureRows::new(capacity.rows[1]),
            autonomy: CaptureRows::new(capacity.rows[2]),
            agencies: CaptureRows::new(capacity.rows[3]),
            operatives: CaptureRows::new(capacity.rows[4]),
            captured_operatives: CaptureRows::new(capacity.rows[16]),
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
            relation_resolver: crelation_status::Resolver::default(),
            subject_snapshot: cwar_relation::OwnedSnapshotScratch::default(),
            country_intel,
            country_intel_outcome: UNCAPTURED,
            subject_names: Arena::new(capacity.subject_names),
            autonomy_names: Arena::new(capacity.autonomy_names),
            agency_names: Arena::new(capacity.agency_names),
            variable_entries: Arena::new(capacity.variable_entries),
            variable_names: Arena::new(capacity.variable_names),
            tokens: Arena::new(capacity.tokens),
            flag_scratch: owned_vec(default_vec(capacity.rows[7])),
            relations_raw_mode: false,
            relations_selection: SliceSelection::new(capacity.rows[13] != 0),
            relation_active: [false; 2],
            capitulation_raw_mode: false,
        }
    }

    /// The owners whose relation rows this frame's read replaced.
    pub(crate) fn relations_selection(&self) -> &SliceSelection {
        &self.relations_selection
    }

    /// Start this hour's relations read and loan its selection to the job.
    /// Allocation-free: the pack keeps a placeholder until the join.
    pub(crate) fn take_relations_selection(
        &mut self,
        hour: i32,
        force_full: bool,
    ) -> SliceSelection {
        self.relations_selection.reset(hour, force_full);
        std::mem::replace(&mut self.relations_selection, SliceSelection::new(false))
    }

    /// Return the job's selection at its join, before any view or publication.
    pub(crate) fn restore_relations_selection(&mut self, selection: SliceSelection) {
        self.relations_selection = selection;
    }

    /// Before IntelResidual borrows this pack: the relations job's context
    /// (row capacities, activity) and, on a refresh hour, this hour's owner
    /// slice. During the loan the frame holds a vacant placeholder with no rows
    /// and no selection storage, so this must run first.
    pub(crate) fn begin_relations(
        &mut self,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        refresh: bool,
    ) -> (RelationsCopyContext, Option<SliceSelection>) {
        let context = RelationsCopyContext {
            base,
            game_state,
            opinion_cap: self.opinion_rows_cap(),
            tuple_cap: self.tuple_rows_cap(),
            active: [TABLES[13], TABLES[14]]
                .map(|table| replay::cadence::should_walk(table, hour, force_full)),
            epoch: relation_epoch(),
        };
        self.set_relation_activity(context.active);
        let selection = refresh.then(|| self.take_relations_selection(hour, force_full));
        (context, selection)
    }

    /// The relations join as `acquire` runs it, after IntelResidual returned
    /// this pack: take back the slice and return the store (`end_relations`)
    /// and set the raw-mode gate. `Ok` is a payload to publish. `Err` is one
    /// to recycle: after a short payload the typed fallback reads the tables
    /// from the returned slice; after a failure (a panicked or stopped job)
    /// the gate keeps the typed capture off them, and
    /// `fail_relations_capture` publishes them unavailable with no further
    /// native read.
    #[allow(clippy::type_complexity)]
    pub(crate) fn join_relations(
        &mut self,
        mut job: RelationsJob,
        report: RelationsCopyReport,
        failure: Option<&'static str>,
    ) -> (
        Option<Box<SliceFingerprints>>,
        Result<RelationsJob, RelationsJob>,
    ) {
        // A read not published whole (failed, truncated or left to the typed
        // fallback) is read again at the next hour.
        let published = report.raw_ok && failure.is_none() && job.published();
        let fingerprints = self.end_relations(&mut job, published);
        self.relations_raw_mode = report.raw_ok || failure.is_some();
        let payload = if report.raw_ok && failure.is_none() {
            Ok(job)
        } else {
            Err(job)
        };
        (fingerprints, payload)
    }

    /// At the relations join, after IntelResidual returned this pack: take
    /// back the job's owner slice and return its fingerprint store. A read not
    /// `published` whole is read again at the next hour.
    pub(crate) fn end_relations(
        &mut self,
        job: &mut RelationsJob,
        published: bool,
    ) -> Option<Box<SliceFingerprints>> {
        let mut fingerprints = job.fingerprints.take();
        if let Some(selection) = job.selection.take() {
            if !published && let Some(fingerprints) = fingerprints.as_deref_mut() {
                selection.invalidate(fingerprints);
            }
            self.restore_relations_selection(selection);
        }
        fingerprints
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
        self.captured_operatives.reset();
        self.cryptology.reset();
        self.networks.reset();
        self.flags.reset();
        self.variables.reset();
        self.operations.reset();
        self.assignments.reset();
        self.operation_tokens.reset();
        self.incoming_actions.reset();
        self.country_intel.release();
        self.country_intel_outcome = UNCAPTURED;
        self.subject_names.reset();
        self.autonomy_names.reset();
        self.agency_names.reset();
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
            + self.captured_operatives.allocated_bytes()
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
            + self.relation_resolver.allocated_bytes()
            + self.subject_snapshot.allocated_bytes()
            + self.subject_names.allocated_bytes()
            + self.autonomy_names.allocated_bytes()
            + self.agency_names.allocated_bytes()
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
        self.captured_operatives.grow(&mut capacity.rows[16]);
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
        // Relation storage is sized only from what its own walks observed, so
        // a vacant or never-walked pack allocates nothing here.
        if self
            .relation_resolver
            .grow()
            .map_err(crate::allocation::io)
            .is_err()
        {
            return;
        }
        if self
            .subject_snapshot
            .grow(0)
            .map_err(crate::allocation::io)
            .is_err()
        {
            return;
        }
        // Returns the shared buffer. Its scratch is a fixed copy window: a
        // country longer than it is copied in several passes, not truncated.
        self.country_intel.grow(&mut capacity.rows[15]);
        self.subject_names.grow(&mut capacity.subject_names);
        self.autonomy_names.grow(&mut capacity.autonomy_names);
        self.agency_names.grow(&mut capacity.agency_names);
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
        );
        self.country_intel_outcome = Outcome::Unavailable {
            reason: if replay::cadence::should_walk(TABLES[15], context.hour, context.force_full) {
                reason
            } else {
                replay::cadence::SKIPPED_REASON
            },
        };
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
                captured_operatives = 16,
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
                    ArenaRead::from(csubject_relation::walk_owned(
                        memory,
                        base,
                        state,
                        rows,
                        names,
                        &mut self.subject_snapshot,
                    ))
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
            let begin = qpc();
            capture_named(&mut self.agencies, &mut self.agency_names, |rows, names| {
                let _reads = read_accounting::table(TABLES[3]);
                ArenaRead::from(cintelligence_agency::walk(memory, base, state, rows, names))
            });
            ticks[3] = elapsed(begin);
            plain!(4, operatives, coperative_leader::walk);
            plain!(
                16,
                captured_operatives,
                cintelligence_agency::walk_captured_operatives
            );
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
            let result = crelation_status::walk_with(
                memory,
                base,
                state,
                &mut self.relation_opinions.rows,
                &mut self.relation_tuples.rows,
                [active[13], active[14]],
                &mut self.relation_resolver,
                &mut crelation_status::Baseline::default(),
                None,
                |owner| self.relations_selection.contains(owner),
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
            // Walked only by the frame holding the pool's buffer; a due hour
            // that finds it held carries the last sample and the next hour walks.
            self.country_intel_outcome = match self.country_intel.borrow(active[15]) {
                Walk::Skip | Walk::Busy => Outcome::Unavailable {
                    reason: replay::cadence::SKIPPED_REASON,
                },
                Walk::Ready(buffer) => {
                    let begin = qpc();
                    capture_rows(&mut buffer.rows, |rows| {
                        let _reads = read_accounting::table(TABLES[15]);
                        ccountry_intel::walk(memory, base, state, rows, &mut buffer.scratch)
                    });
                    ticks[15] = elapsed(begin);
                    buffer.rows.outcome
                }
            };
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
            self.country_intel_outcome,
            self.captured_operatives.outcome,
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
        let flags = if let Some(reuse) = reuse {
            reuse
                .shadow
                .flags
                .input(TableId::ScriptFlags, &self.flags, outcomes[7], || {
                    self.flags_table(outcomes[7])
                })
        } else {
            TableInput::Sampled(self.flags_table(outcomes[7]))
        };
        // Converted once a day; the spool's staggered cache carries it between.
        let country_intel = TableInput::Sampled(self.country_intel_table(outcomes[15]));
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
            sample(self.agencies_table(outcomes[3])),
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
                    ordinal: rows::Recorded::Value(r.ordinal),
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
                    local_strength_raw: r.local_strength_raw,
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
            sample(table::<tables::CapturedOperativeReferences, _>(
                outcomes[16],
                &self.captured_operatives.rows,
                |r| rows::CapturedOperativeReferenceRow {
                    country: r.country,
                    ordinal: r.ordinal,
                    operative: r.operative,
                    owner: r.owner,
                },
            )),
        ]
    }
}

impl Captured {
    fn country_intel_table(&self, outcome: Outcome) -> TableView {
        let source = self.country_intel.rows().unwrap_or_default();
        table::<tables::CountryIntel, _>(outcome, source, |r| rows::CountryIntelRow {
            owner_country: r.owner_country,
            target_index: r.target_index,
            army: r.army,
            air: r.air,
            civilian: r.civilian,
            navy: r.navy,
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
            state: owned_vec(copy_vec(literal(&self.subject_names.first, r.state))),
        })
    }

    fn agencies_table(&self, outcome: Outcome) -> TableView {
        if !valid_arena(
            outcome,
            &self.agencies.rows,
            &self.agency_names.first,
            self.agency_names.len,
            self.agency_names.required,
        ) {
            return unavailable("agency name lies outside captured arena");
        }
        table::<tables::IntelligenceAgencies, _>(outcome, &self.agencies.rows, |r| {
            rows::IntelligenceAgencyRow {
                country: r.country,
                upgrade_progress_raw: r.upgrade_progress_raw,
                defense_raw: r.defense_raw,
                active_count: r.active_count,
                admission_ceiling: r.admission_ceiling,
                unknown_f0: r.unknown_f0,
                name: owned_vec(copy_vec(literal(&self.agency_names.first, r.name))),
                created: r.created,
            }
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
            state: owned_vec(copy_vec(literal(&self.autonomy_names.first, r.state))),
            previous: r
                .previous
                .map(|name| owned_vec(copy_vec(literal(&self.autonomy_names.first, name)))),
            next: r
                .next
                .map(|name| owned_vec(copy_vec(literal(&self.autonomy_names.first, name)))),
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
        if !clear_seen(&mut scratch.scopes, len) {
            return false;
        }
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
            let Ok(seed) = previous[index].seed.require("ScriptVariables.seed") else {
                return false;
            };
            if *seed != scope.seed {
                return false;
            }
            let old = &previous[index].entries;
            if scope.entries_len != old.len()
                || old.windows(2).any(|pair| pair[0].name >= pair[1].name)
            {
                return false;
            }
            if !clear_seen(&mut scratch.entries, old.len()) {
                return false;
            }
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

    pub(crate) fn observe_seeds(&self, work: &mut SeedWork) {
        let len = match self.variables.outcome {
            Outcome::Rows { len } | Outcome::Truncated { len, .. } => len,
            Outcome::Unavailable { .. } => return,
        };
        if let Some(rows) = self.variables.rows.get(..len) {
            for row in rows {
                if row.seed.is_some() {
                    work.captured = work.captured.saturating_add(1);
                } else {
                    work.refused = work.refused.saturating_add(1);
                }
            }
        }
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
            let mut entries = owned_vec(map_slice(
                &self.variable_entries.first[r.entries_start..r.entries_start + r.entries_len],
                |entry| rows::ScriptVariableEntry {
                    name: owned_vec(copy_vec(
                        &self.variable_names.first
                            [entry.name_start..entry.name_start + entry.name_len],
                    )),
                    value_raw: entry.value_raw,
                },
            ));
            entries.sort_unstable_by(|a, b| a.name.cmp(&b.name));
            rows::ScriptVariablesRow {
                scope_kind,
                scope_id,
                entries,
                seed: rows::Recorded::Value(r.seed),
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
                tokens: owned_vec(copy_vec(
                    &self.tokens.first[r.tokens_start..r.tokens_start + r.tokens_len],
                )),
            }
        })
    }
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
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

impl ArenaRow<u8> for cintelligence_agency::Row {
    fn valid(&self, len: usize) -> bool {
        name_fits(self.name, len)
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
        cvariables::Scope::Character(id) => (CHARACTER_SCOPE, id),
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
/// Every character in both serialized rosters is a scope too, with its flag
/// manager embedded in the character (`ccharacter::FLAGS`); each manager's
/// live count (+0x14) sizes its rows, and an omitted count is kept for growth.
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
        let root = |root, offset| -> Result<usize, &'static str> {
            let manager = pointer(memory, root, offset)?;
            if manager == 0 {
                return Err("script flag manager root is null");
            }
            Ok(manager)
        };
        let mut scope = |kind, id, manager| -> Result<(), &'static str> {
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
        scope(GLOBAL_SCOPE, 0, root(state, cgame_state::GLOBAL_FLAGS)?)?;
        let countries = ccountry::Reader::new(memory, base, state)?;
        let mut found_country = false;
        for slot in 1..countries.count() {
            if let Some(country) = countries.at(slot)? {
                found_country = true;
                scope(
                    COUNTRY_SCOPE,
                    country.id,
                    root(country.pointer, ccountry::FLAG_MANAGER)?,
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
                scope(STATE_SCOPE, id, root(at, cstate::FLAGS)?)?;
            }
            first += count as u32;
        }
        if !found_state {
            return Err("state table has no readable states");
        }
        // Each authenticated roster occurrence's identity, then its manager.
        let mut characters = ccharacter_manager::Roster::read(memory, base, state)?.cursor();
        while let Some(view) = characters.next(memory, [false; 4])? {
            let (character, id) = view.reference();
            let manager = character
                .checked_add(ccharacter::FLAGS)
                .ok_or("character flag manager address overflow")?;
            scope(CHARACTER_SCOPE, id, manager)?;
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
// Relations job (RelationOpinions, RelationTuples).
//
// The worker runs the typed walk (`crelation_status::walk_with`) into the
// job's own rows; the decode worker copies them into the pack. One shared
// store keeps the opinion baseline between refreshes, so a refresh that
// observed every opinion write since the last one (`crate::opinion_changes`)
// reads only the statuses a writer changed.
// ===========================================================================

/// Drains the opinion writes observed since `consumed`: see
/// [`crate::opinion_changes::drain`].
type Drain =
    fn(&mut u64, &mut [crelation_status::Mark], &mut usize) -> Option<(u64, Option<usize>)>;

/// The opinion baseline every relations job shares. Pooled jobs borrow the one
/// store: a job takes it at its refresh and returns it, grown off the game
/// thread, when decode releases the job. A refresh that finds it lent reads
/// every status, which yields the same rows.
struct SharedOpinions {
    baseline: crelation_status::Baseline,
    /// The baseline's counts last reported to `opinion_changes`.
    reported: crelation_status::BaselineCounts,
    /// The opinion-write log position this baseline has accounted for.
    consumed: u64,
    /// Each mark is a status and its owning diplomacy, so a mark waits for
    /// its owner's slice.
    marks: Vec<crelation_status::Mark>,
    required_marks: usize,
    /// Rows of the verify walk.
    verify: Vec<crelation_status::OpinionRow>,
    required_verify: usize,
    drain: Drain,
    /// Verify mode: where a comparison's result goes.
    report: Option<fn(Option<bool>)>,
    charge: crate::raw_memory::Charge,
}

impl SharedOpinions {
    fn new() -> Self {
        Self {
            baseline: crelation_status::Baseline::default(),
            reported: crelation_status::BaselineCounts::default(),
            // Marks appended before this store existed are no one's demand.
            consumed: default_cursor(),
            marks: Vec::new(),
            required_marks: 0,
            verify: Vec::new(),
            required_verify: 0,
            drain: default_drain(),
            report: crate::opinion_changes::verify()
                .then_some(crate::opinion_changes::verified as fn(Option<bool>)),
            charge: crate::raw_memory::Charge::default(),
        }
    }

    /// Off the game thread.
    fn grow(&mut self) {
        fn fit<T: Default + Clone>(vector: &mut Vec<T>, required: usize) {
            if required > vector.len() {
                let len = required.saturating_add(required / 4);
                if reserve_exact(vector, len - vector.len()).is_ok() {
                    vector.resize(len, T::default());
                }
            }
        }
        // Room for the marks the last drain saw, in the baseline as well as
        // here, so they are not refused twice.
        if self
            .baseline
            .reserve_marks(self.required_marks)
            .map_err(crate::allocation::io)
            .is_err()
        {
            return;
        }
        fit(
            &mut self.marks,
            self.required_marks
                .max(crelation_status::Baseline::MIN_MARKS),
        );
        fit(&mut self.verify, self.required_verify);
        self.charge.set(crate::raw_memory::Capacity {
            payload_vectors: (self.baseline.allocated_bytes()
                + self.marks.capacity() * size_of::<crelation_status::Mark>()
                + self.verify.capacity() * size_of::<crelation_status::OpinionRow>())
                as u64,
            ..Default::default()
        });
    }
}

/// Tests never observe the process's native writers.
#[cfg(not(test))]
fn default_cursor() -> u64 {
    crate::opinion_changes::cursor()
}

#[cfg(test)]
fn default_cursor() -> u64 {
    0
}

#[cfg(not(test))]
fn default_drain() -> Drain {
    crate::opinion_changes::drain
}

#[cfg(test)]
fn default_drain() -> Drain {
    |_, _, _| None
}

// The slot is a static: boxing the store would allocate on its first
// borrow, which happens in the callback.
#[allow(clippy::large_enum_variant)]
enum OpinionSlot {
    Unmade,
    Home(SharedOpinions),
    Lent,
}

type OpinionStore = std::sync::Mutex<OpinionSlot>;

#[cfg(not(test))]
static RELATION_OPINIONS: OpinionStore = std::sync::Mutex::new(OpinionSlot::Unmade);

/// Tests keep a private store per job, so parallel tests never share one.
#[cfg(not(test))]
fn default_opinion_store() -> &'static OpinionStore {
    &RELATION_OPINIONS
}

#[cfg(test)]
fn default_opinion_store() -> &'static OpinionStore {
    Box::leak(Box::new(std::sync::Mutex::new(OpinionSlot::Unmade)))
}

/// A borrowed store, returned to its slot when dropped.
struct LentOpinions {
    store: &'static OpinionStore,
    shared: Option<SharedOpinions>,
}

impl LentOpinions {
    /// Never blocks and never allocates: the callback may call it.
    fn borrow(store: &'static OpinionStore) -> Option<Self> {
        let mut slot = store.try_lock().ok()?;
        let shared = match std::mem::replace(&mut *slot, OpinionSlot::Lent) {
            OpinionSlot::Unmade => SharedOpinions::new(),
            OpinionSlot::Home(shared) => shared,
            OpinionSlot::Lent => return None,
        };
        Some(Self {
            store,
            shared: Some(shared),
        })
    }

    fn shared(&mut self) -> &mut SharedOpinions {
        self.shared.as_mut().expect("present until dropped")
    }
}

impl Drop for LentOpinions {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.take()
            && let Ok(mut slot) = self.store.lock()
        {
            *slot = OpinionSlot::Home(shared);
        }
    }
}

/// Bumped on every load, gap or recovery; a baseline recorded under an older
/// value is not replayed.
static RELATION_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The world may have been rebuilt: no recorded opinion may be replayed.
pub(crate) fn forget_relation_opinions() {
    RELATION_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn relation_epoch() -> u64 {
    RELATION_EPOCH.load(std::sync::atomic::Ordering::Relaxed)
}

/// One relations refresh: its rows, outcomes and the walk's retained storage.
/// Storage grows only off the game thread (`crate::decode`); a refresh that
/// needs more rows than the job holds reports it and takes the typed fallback.
#[derive(Default)]
pub(crate) struct RelationsJob {
    pub active: [bool; 2],
    pub opinions: Vec<crelation_status::OpinionRow>,
    pub tuples: Vec<crelation_status::TupleRow>,
    walk: Option<crelation_status::Walk>,
    resolver: crelation_status::Resolver,
    /// Where this job borrows the opinion baseline.
    store: Option<&'static OpinionStore>,
    /// The shared store while this job holds it, from its refresh until decode
    /// releases the job.
    shared: Option<LentOpinions>,
    /// Rows the last refresh needed, `[opinions, tuples]`.
    required: [usize; 2],
    overflowed: bool,
    capacity_charge: crate::raw_memory::Charge,
    /// The pack's owner selection for this hour, loaned from dispatch to the
    /// join (`Captured::take_relations_selection`).
    pub(crate) selection: Option<SliceSelection>,
    /// The relations family's fingerprint store, loaned with the selection.
    pub(crate) fingerprints: Option<Box<SliceFingerprints>>,
}

impl std::fmt::Debug for RelationsJob {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RelationsJob")
            .field("active", &self.active)
            .field("walk", &self.walk)
            .field("overflowed", &self.overflowed)
            .finish_non_exhaustive()
    }
}

impl RelationsJob {
    /// Seeded for a late campaign at the vanilla country count (8,639
    /// opinions and 1,512 tuples); growth follows each refresh.
    pub fn new() -> Self {
        let mut job = Self {
            opinions: owned_vec(default_vec(16_384)),
            tuples: owned_vec(default_vec(4_096)),
            ..Self::default()
        };
        job.update_capacity_charge();
        job
    }

    /// Reset the payload, retaining storage.
    pub fn clear(&mut self) {
        self.active = [false; 2];
        self.walk = None;
        self.overflowed = false;
    }

    /// Whether the refresh's walk published both tables whole: rows, not a
    /// failure or truncation.
    pub(crate) fn published(&self) -> bool {
        self.walk.is_some_and(|walk| {
            matches!(walk.opinions, Outcome::Rows { .. })
                && matches!(walk.tuples, Outcome::Rows { .. })
        })
    }

    /// Whether this is pooled storage rather than a vacant placeholder.
    pub fn is_allocated(&self) -> bool {
        !self.opinions.is_empty()
    }

    pub fn grow_for_next(&self) -> bool {
        self.overflowed || self.resolver.required() > self.resolver.capacity()
    }

    /// Off the game thread: grow the shared store to what this job's refresh
    /// observed and hand it back for the next job.
    pub fn return_opinions(&mut self) {
        if let Some(mut lent) = self.shared.take() {
            lent.shared().grow();
        }
    }

    /// Off the game thread: grow rows to the last refresh's demand (with
    /// headroom) and the walk's storage to what it observed.
    pub fn grow(&mut self) {
        fn fit<T: Default + Clone>(rows: &mut Vec<T>, required: usize) {
            if required > rows.len() {
                let len = required.saturating_add(required / 4);
                if reserve_exact(rows, len - rows.len()).is_ok() {
                    rows.resize(len, T::default());
                }
            }
        }
        fit(&mut self.opinions, self.required[0]);
        fit(&mut self.tuples, self.required[1]);
        if self.resolver.grow().map_err(crate::allocation::io).is_err() {
            return;
        }
        self.update_capacity_charge();
    }

    fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            payload_vectors: vector_bytes(&self.opinions)
                + vector_bytes(&self.tuples)
                + self.resolver.allocated_bytes() as u64,
            ..Default::default()
        });
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct RelationsCopyReport {
    /// False when the job's rows were too short: the caller takes the typed
    /// fallback and enqueues no payload.
    pub raw_ok: bool,
    pub copy_ticks: [u64; 2],
    /// Row bytes handed to decode.
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
    /// `relation_epoch` at dispatch.
    pub epoch: u64,
}

/// Verify mode: compare an observed walk's opinions with a walk that reads
/// every status of the same owner slice. A difference poisons the
/// observation for the rest of the attachment and the reference rows are
/// kept.
fn verify_opinions(
    memory: &impl Memory,
    ctx: RelationsCopyContext,
    shared: &mut SharedOpinions,
    resolver: &mut crelation_status::Resolver,
    rows: &mut [crelation_status::OpinionRow],
    outcome: Outcome,
    select: impl Fn(u32) -> bool,
) -> Outcome {
    let Some(report) = shared.report else {
        return outcome;
    };
    shared.required_verify = rows.len();
    let Some(reference) = shared.verify.get_mut(..rows.len()) else {
        report(None);
        return outcome;
    };
    let expected = crelation_status::walk_with(
        memory,
        ctx.base,
        ctx.game_state,
        reference,
        &mut [],
        [true, false],
        resolver,
        &mut crelation_status::Baseline::default(),
        None,
        select,
    )
    .opinions;
    let len = expected.len();
    let agrees = expected == outcome && reference[..len] == rows[..len];
    report(Some(agrees));
    if agrees {
        return outcome;
    }
    shared.baseline.clear();
    rows[..len].copy_from_slice(&reference[..len]);
    expected
}

/// The relations walk on the worker, into the job's rows. Rows and outcomes
/// equal a walk into the pack's own buffers: each table is walked into at
/// most the pack's capacity, and a job shorter than that which truncates
/// reports `raw_ok: false` instead.
/// Scan every live country into this hour's relations selection with a
/// constant fingerprint: only new owners and unpublished reads join the
/// hour's slice. The census reads go through the callback's cached country
/// view. An incomplete census keeps every held owner live.
fn scan_relation_owners(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    selection: &mut SliceSelection,
    fingerprints: &mut SliceFingerprints,
) {
    let scan = qpc();
    let mut census = true;
    match ccountry::Reader::new(memory, base, game_state) {
        Err(_) => census = false,
        Ok(countries) => {
            for slot in 1..countries.count() {
                match countries.at(slot) {
                    Ok(Some(country)) => selection.scan(fingerprints, country.id, Some(0)),
                    Ok(None) => {}
                    Err(_) => census = false,
                }
            }
        }
    }
    fingerprints.finish_scan();
    if census {
        selection.finish_census(fingerprints);
    }
    selection.scan_ticks = elapsed(scan);
}

pub(crate) fn copy_relations_refresh(
    memory: &impl Memory,
    ctx: RelationsCopyContext,
    job: &mut RelationsJob,
) -> RelationsCopyReport {
    job.clear();
    job.active = ctx.active;
    if !ctx.active.into_iter().any(|active| active) {
        return RelationsCopyReport {
            raw_ok: true,
            ..Default::default()
        };
    }
    if job.shared.is_none() {
        let store = *job.store.get_or_insert_with(default_opinion_store);
        job.shared = LentOpinions::borrow(store);
    }
    let anchor = usize::from(ctx.active[1]);
    let _reads = read_accounting::table(TABLES[13 + anchor]);
    let started = qpc();
    let lens = [
        job.opinions.len().min(ctx.opinion_cap),
        job.tuples.len().min(ctx.tuple_cap),
    ];
    if let (Some(selection), Some(fingerprints)) = (&mut job.selection, &mut job.fingerprints) {
        scan_relation_owners(memory, ctx.base, ctx.game_state, selection, fingerprints);
    }
    // Lent to another job: read every status, as after a load.
    let mut unshared = crelation_status::Baseline::default();
    let (baseline, observation) = match &mut job.shared {
        Some(lent) => {
            let SharedOpinions {
                baseline,
                consumed,
                marks,
                required_marks,
                drain,
                report,
                verify,
                required_verify,
                ..
            } = lent.shared();
            // Only opinion refreshes account for the log.
            let mut pending = 0;
            let drained = if ctx.active[0] {
                drain(consumed, marks.as_mut_slice(), &mut pending)
            } else {
                None
            };
            // Marks from before the baseline's epoch (a load's, say) are
            // never replayed: they are no demand for storage.
            if drained.is_some_and(|(epoch, _)| baseline.token() == Some((epoch, ctx.epoch))) {
                *required_marks = pending;
            }
            // Verify mode publishes no replay it cannot compare: until its
            // storage holds this job's rows, the observation hands over no
            // marks, so the walk reads every selected owner in full.
            let verifiable = report.is_none() || verify.len() >= lens[0];
            if report.is_some() {
                *required_verify = (*required_verify).max(lens[0]);
            }
            let observation = drained.map(|(epoch, count)| crelation_status::Observation {
                token: (epoch, ctx.epoch),
                marks: count.filter(|_| verifiable).map(|count| &marks[..count]),
            });
            (baseline, observation)
        }
        None => {
            crate::opinion_changes::store_lent();
            (&mut unshared, None)
        }
    };
    // Without a loaned selection (a test harness) every owner is read, as on
    // a full hour: the pack's placeholder selection is a full one.
    let selection = job.selection.as_ref();
    let select = |owner| selection.is_none_or(|selection| selection.contains(owner));
    // Verify only walks that replayed an owner: a walk that read every
    // selected owner in full has nothing to compare.
    let replayed = baseline.counts().reused_owners;
    let mut walk = crelation_status::walk_with(
        memory,
        ctx.base,
        ctx.game_state,
        &mut job.opinions[..lens[0]],
        &mut job.tuples[..lens[1]],
        ctx.active,
        &mut job.resolver,
        baseline,
        observation,
        select,
    );
    let replayed = baseline.counts().reused_owners != replayed;
    if let Some(lent) = &mut job.shared {
        let shared = lent.shared();
        crate::opinion_changes::record(shared.baseline.counts(), &mut shared.reported);
        if replayed && shared.report.is_some() {
            walk.opinions = verify_opinions(
                memory,
                ctx,
                shared,
                &mut job.resolver,
                &mut job.opinions[..lens[0]],
                walk.opinions,
                select,
            );
        }
    }
    let mut copy_ticks = [0; 2];
    copy_ticks[anchor] = elapsed(started);
    // A truncation the pack's capacity would have held needs a longer job.
    let short = |outcome: Outcome, len: usize, cap: usize| match outcome {
        Outcome::Truncated { unread_rows, .. } if len < cap => {
            Some(len.saturating_add(unread_rows as usize))
        }
        _ => None,
    };
    let demand = [
        short(walk.opinions, lens[0], ctx.opinion_cap),
        short(walk.tuples, lens[1], ctx.tuple_cap),
    ];
    job.overflowed = demand.iter().any(Option::is_some);
    job.required = std::array::from_fn(|table| demand[table].unwrap_or(0));
    job.walk = Some(walk);
    RelationsCopyReport {
        raw_ok: !job.overflowed,
        copy_ticks,
        raw_bytes: [
            (walk.opinions.len() * size_of::<crelation_status::OpinionRow>()) as u64,
            (walk.tuples.len() * size_of::<crelation_status::TupleRow>()) as u64,
        ],
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

    /// Installs a refresh's rows and outcomes into the pack. Clears
    /// `relations_raw_mode` so the next capture starts clean.
    pub(crate) fn decode_relations_payload(
        &mut self,
        job: &RelationsJob,
        decode_ticks: &mut [u64; 2],
    ) {
        self.relation_active = job.active;
        let Some(walk) = job
            .walk
            .filter(|_| job.active.into_iter().any(|active| active))
        else {
            self.fail_relations_capture(job.active, replay::cadence::SKIPPED_REASON);
            return;
        };
        let started = qpc();
        let skipped = Outcome::Unavailable {
            reason: replay::cadence::SKIPPED_REASON,
        };
        install(
            job.active[0],
            walk.opinions,
            &job.opinions,
            &mut self.relation_opinions,
            skipped,
        );
        install(
            job.active[1],
            walk.tuples,
            &job.tuples,
            &mut self.relation_tuples,
            skipped,
        );
        decode_ticks[usize::from(job.active[1])] += elapsed(started);
        self.relations_raw_mode = false;
    }
}

/// Copies one table's rows, which the refresh walked into at most `rows`'s
/// capacity.
fn install<R: Copy>(
    active: bool,
    outcome: Outcome,
    source: &[R],
    rows: &mut CaptureRows<R>,
    skipped: Outcome,
) {
    if !active {
        rows.outcome = skipped;
        return;
    }
    let len = outcome.len();
    rows.outcome = match (source.get(..len), rows.rows.get_mut(..len)) {
        (Some(source), Some(destination)) => {
            destination.copy_from_slice(source);
            outcome
        }
        _ => Outcome::Unavailable {
            reason: "relation rows exceed their buffer",
        },
    };
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
        agency_names: 0,
        variable_entries: 0,
        variable_names: 0,
        tokens: 0,
    };
    let mut capture = Captured::with_capacities(&capacities);
    let Walk::Ready(buffer) = capture.country_intel.borrow(true) else {
        unreachable!("a new frame holds its own buffer")
    };
    for (index, row) in buffer.rows.rows.iter_mut().enumerate() {
        *row = engine::country_intel_row::Row {
            owner_country: 7,
            target_index: index as u32,
            army: value,
            air: i64::MIN,
            civilian: 0,
            navy: 0,
        };
    }
    for (index, row) in capture.flags.rows.iter_mut().enumerate() {
        *row = ScopedFlag {
            scope_kind: COUNTRY_SCOPE,
            scope_id: 7,
            flag: cscript_flag::Row {
                token: index as u32,
                set_hour: -(value as i32),
                value: i16::MIN,
                remaining_days: i16::MAX,
            },
        };
    }
    capture.flags.outcome = Outcome::Rows { len: 2 };
    capture.country_intel_outcome = Outcome::Rows { len: 2 };
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
            let country_intel = captured.country_intel.rows().unwrap().to_vec();
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
            assert_eq!(captured.country_intel.rows().unwrap(), country_intel);
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
            agency_names: 0,
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

    /// Pooled frames share one CountryIntel buffer. A daily slot that finds it
    /// still held (the spool has not returned the last walk) returns at once
    /// with a carried skip, and the next hour walks instead.
    #[test]
    fn a_held_intel_buffer_defers_the_daily_walk_one_hour_without_waiting() {
        struct Unreadable;
        impl Memory for Unreadable {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                false
            }
        }
        let mut first = Captured::with_capacities(&Capacities::default());
        let mut second = Captured::with_capacities(&Capacities::default());
        second.share_country_intel(first.country_intel_home().unwrap());
        let capture = |captured: &mut Captured, hour| {
            captured.capture_residual(
                &Unreadable,
                CaptureContext {
                    base: 0,
                    game_state: 0x1000,
                    hour,
                    force_full: false,
                },
            );
            // Any outcome but the carried skip means the walk ran.
            captured.outcomes()[15]
                != Outcome::Unavailable {
                    reason: replay::cadence::SKIPPED_REASON,
                }
        };
        assert!(capture(&mut first, 0));
        assert!(!capture(&mut second, 1));
        // The first frame is still unconverted at the next daily slot.
        assert!(!capture(&mut second, 24));
        assert!(!capture(&mut second, 25));
        first.grow(&mut Capacities::default());
        assert!(capture(&mut second, 26));
        second.grow(&mut Capacities::default());
        assert!(!capture(&mut first, 27));
        assert!(capture(&mut first, 48));
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
        assert!(Arc::ptr_eq(shared(&first[7]), shared(&second[7])));
        assert_eq!(
            TableView::Complete((**shared(&first[7])).clone()),
            original[7]
        );
        // CountryIntel is converted once a day and carried by the spool's
        // staggered cache, so it keeps no native shadow.
        assert_eq!(second[15], TableInput::Sampled(original[15].clone()));
        let stats = shadow.statistics();
        assert_eq!((stats.hits, stats.misses), (1, 1));
        assert_eq!(
            stats.row_capacity_bytes,
            shadow.flags.rows.capacity() * size_of::<ScopedFlag>()
        );
        assert_eq!(
            stats.replay_capacity_bytes,
            std::mem::size_of_val(shared(&first[7]).rows::<tables::ScriptFlags>().unwrap())
        );
        // Unused capacity is not data. Only the current successful prefix is compared.
        capture.flags.rows.push(ScopedFlag::default());
        let third = memo_inputs(&capture, &mut shadow, 1);
        assert!(Arc::ptr_eq(shared(&first[7]), shared(&third[7])));
        drop(capture);
        assert_eq!(shadow.flags.rows.len(), 2);
    }

    #[test]
    fn intel_shadow_compares_every_native_field_and_order_before_reuse() {
        for change in 0..8 {
            let mut capture = intel_shadow_fixture(42);
            let mut shadow = IntelShadow::default();
            let first = memo_inputs(&capture, &mut shadow, 1);
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
            let expected = capture.tables();
            let next = memo_inputs(&capture, &mut shadow, 1);
            assert!(
                !Arc::ptr_eq(shared(&first[7]), shared(&next[7])),
                "change {change}"
            );
            assert_eq!(
                TableView::Complete((**shared(&next[7])).clone()),
                expected[7]
            );
        }
    }

    #[test]
    fn intel_shadow_current_faults_truncation_and_duplicate_keys_clear_only_affected_table() {
        for fault in 0..5 {
            let mut capture = intel_shadow_fixture(42);
            let mut shadow = IntelShadow::default();
            let first = memo_inputs(&capture, &mut shadow, 1);
            capture.flags.outcome = match fault {
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
            if fault == 4 {
                capture.flags.rows[1] = capture.flags.rows[0];
            }
            let expected = capture.tables();
            let next = memo_inputs(&capture, &mut shadow, 1);
            assert_eq!(next[7], TableInput::Sampled(expected[7].clone()));
            assert!(matches!(
                next[7],
                TableInput::Sampled(TableView::Unavailable { .. } | TableView::Truncated { .. })
            ));
            assert_eq!(shadow.statistics().replay_capacity_bytes, 0);
            let recovered = memo_inputs(&intel_shadow_fixture(42), &mut shadow, 1);
            assert!(!Arc::ptr_eq(shared(&first[7]), shared(&recovered[7])));
        }
    }

    #[test]
    fn intel_shadow_branch_changes_and_empty_success_never_resurrect_old_rows() {
        let mut capture = intel_shadow_fixture(42);
        let mut shadow = IntelShadow::default();
        let first = memo_inputs(&capture, &mut shadow, 1);
        let next = memo_inputs(&capture, &mut shadow, 2);
        assert!(!Arc::ptr_eq(shared(&first[7]), shared(&next[7])));
        capture.flags.outcome = Outcome::Rows { len: 0 };
        let empty = memo_inputs(&capture, &mut shadow, 2);
        let repeated = memo_inputs(&capture, &mut shadow, 2);
        assert!(Arc::ptr_eq(shared(&empty[7]), shared(&repeated[7])));
        assert!(!Arc::ptr_eq(shared(&next[7]), shared(&empty[7])));
    }

    #[test]
    fn comparison_bitsets_keep_the_original_resize_capacity_trace() {
        let mut previous = Vec::<u64>::new();
        let mut checked = Vec::<u64>::new();
        for count in [1_usize, 64, 257, 65, 513, 1_025, 0] {
            previous.resize(count.div_ceil(64), 0);
            previous.fill(0);
            assert!(clear_seen(&mut checked, count));
            assert_eq!(checked, previous);
            assert_eq!(checked.capacity(), previous.capacity());
            checked.fill(u64::MAX);
        }
    }

    #[test]
    fn relations_job_capacity_accounting_tracks_all_vectors_through_growth_clear_and_drop() {
        use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
        static LOCAL: Tracker = Tracker::new();
        let mut job = RelationsJob::new();
        job.capacity_charge = Charge::empty_on(&LOCAL);
        job.update_capacity_charge();
        let expected = |job: &RelationsJob| {
            vector_bytes(&job.opinions)
                + vector_bytes(&job.tuples)
                + job.resolver.allocated_bytes() as u64
        };
        let initial = LOCAL.snapshot().current.payload_vectors;
        assert_eq!(initial, expected(&job));
        // A refresh records its demand; growth follows it off the capture.
        let bytes = relations_kit::opinionated_fixture();
        let ctx = RelationsCopyContext {
            base: relations_kit::BASE,
            game_state: relations_kit::GS,
            active: [true; 2],
            opinion_cap: 16,
            tuple_cap: 16,
            epoch: 0,
        };
        assert!(copy_relations_refresh(&relations_kit::memory(&bytes), ctx, &mut job).raw_ok);
        assert!(job.grow_for_next());
        job.grow();
        let grown = LOCAL.snapshot().current.payload_vectors;
        assert!(grown > initial);
        assert_eq!(grown, expected(&job));
        // The shared store grows as the job returns it, on its own charge.
        job.return_opinions();
        let mut lent = LentOpinions::borrow(job.store.unwrap()).unwrap();
        let baseline = &lent.shared().baseline;
        assert!(baseline.allocated_bytes() > 0);
        assert!(!baseline.needs_growth());
        drop(lent);
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
            agency_names: 8,
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
            seed: Some(u32::MAX),
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
            ordinal: u32::MAX,
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
            local_strength_raw: i64::MIN,
        };
        capture.networks.rows[1] = ccountry_intel_network::Row {
            owner_country: 1,
            target_country: 2,
            state_id: 4,
            strength_raw: Some(i64::MIN),
            subnetwork_selector: 0,
            local_strength_raw: i64::MAX,
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
            seed: Some(u32::MAX),
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
        assert_eq!(networks[0].local_strength_raw, i64::MIN);
        assert_eq!(networks[1].local_strength_raw, i64::MAX);
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
            operative
                .ordinal
                .require("OperativeMemberships.ordinal")
                .unwrap(),
            &u32::MAX
        );
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
                seed: Some(0),
            },
            cvariables::ScopeRow {
                scope: cvariables::Scope::Country(9),
                entries_start: 0,
                entries_len: 3,
                seed: Some(u32::MAX),
            },
            cvariables::ScopeRow {
                scope: cvariables::Scope::Global,
                entries_start: 4,
                entries_len: 0,
                seed: Some(1),
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
                    tables: views.map(Into::into),
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
        for change in 0..5 {
            let (mut capture, previous) = variable_comparison_fixture();
            let encoder = variable_baseline(previous);
            match change {
                0 => capture.variable_entries.first[0].value_raw += 1,
                1 => capture.variable_names.first[2] = b'A',
                2 => capture.variables.rows[0].scope = cvariables::Scope::State(5),
                3 => capture.variable_entries.first[2].name_len = 1,
                _ => capture.variables.rows[0].seed = Some(u32::MAX),
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
    fn variable_precomparison_distinguishes_unrecorded_seed_from_captured_absence() {
        let (mut capture, _) = variable_comparison_fixture();
        capture.variables.rows[0].seed = None;
        let view = capture.variables_table(capture.outcomes()[8]);
        let mut previous = complete::<tables::ScriptVariables>(&view).to_vec();
        let index = previous
            .iter()
            .position(|row| (row.scope_kind, row.scope_id) == (STATE_SCOPE, 4))
            .unwrap();
        assert_eq!(previous[index].seed, rows::Recorded::Value(None));
        let mut scratch = ScriptVariableComparison::default();
        assert!(capture.variables_equal(capture.outcomes()[8], &previous, &mut scratch));
        previous[index].seed = rows::Recorded::NotRecorded;
        assert_eq!(
            previous[index].seed.require("ScriptVariables.seed"),
            Err("ScriptVariables.seed not recorded".into())
        );
        assert!(!capture.variables_equal(capture.outcomes()[8], &previous, &mut scratch));
    }

    #[test]
    fn variable_seeds_use_live_scope_capacity_and_detect_seed_only_changes_at_5000_countries() {
        let (mut capacities, mut capture) = small();
        const COUNTRIES: usize = 5_000;
        let count = COUNTRIES + 1;
        capacities.raise_for_script_variables(VariableDemand {
            scopes: count,
            entries: 0,
            bytes: 0,
        });
        capture.grow(&mut capacities);
        assert!(capture.variables.rows.len() >= count);
        capture.variables.outcome = Outcome::Rows { len: count };
        for (index, scope) in capture.variables.rows[..count].iter_mut().enumerate() {
            *scope = cvariables::ScopeRow {
                scope: if index == 0 {
                    cvariables::Scope::Global
                } else {
                    cvariables::Scope::Country(index as u32)
                },
                entries_start: 0,
                entries_len: 0,
                seed: Some(u32::MAX - index as u32),
            };
        }
        let view = capture.variables_table(capture.outcomes()[8]);
        let previous = complete::<tables::ScriptVariables>(&view);
        assert_eq!(previous.len(), count);
        assert_eq!(previous[0].seed, rows::Recorded::Value(Some(u32::MAX)));
        assert_eq!(
            previous[COUNTRIES].seed,
            rows::Recorded::Value(Some(u32::MAX - COUNTRIES as u32))
        );
        let mut scratch = ScriptVariableComparison::default();
        assert!(capture.variables_equal(capture.outcomes()[8], previous, &mut scratch));
        capture.variables.rows[COUNTRIES].seed = Some(0);
        assert!(!capture.variables_equal(capture.outcomes()[8], previous, &mut scratch));
        let changed = capture.variables_table(capture.outcomes()[8]);
        assert_eq!(
            complete::<tables::ScriptVariables>(&changed)[COUNTRIES].seed,
            rows::Recorded::Value(Some(0))
        );
        capture.variables.rows[COUNTRIES].seed = None;
        let mut work = SeedWork::default();
        capture.observe_seeds(&mut work);
        assert_eq!((work.captured, work.refused), (COUNTRIES as u64, 1));
        let status = serde_json::to_value(work.status()).unwrap();
        assert_eq!(
            status["refusals"]["script variable random seed unreadable"],
            1
        );
        assert!(!capture.variables_equal(capture.outcomes()[8], previous, &mut scratch));
        let unknown = capture.variables_table(capture.outcomes()[8]);
        assert_eq!(
            complete::<tables::ScriptVariables>(&unknown)[COUNTRIES].seed,
            rows::Recorded::Value(None)
        );
    }

    #[test]
    fn variable_precomparison_scope_hint_accepts_ordered_and_reordered_country_and_state_ids() {
        let (mut capture, _) = variable_comparison_fixture();
        capture.variables.rows.extend_from_slice(&[
            cvariables::ScopeRow {
                scope: cvariables::Scope::Country(2),
                entries_start: 4,
                entries_len: 0,
                seed: Some(2),
            },
            cvariables::ScopeRow {
                scope: cvariables::Scope::State(1),
                entries_start: 4,
                entries_len: 0,
                seed: Some(1),
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
        // Two historical characters, ids 44 and 45: 44 owns one flag, 45 an
        // empty manager. Their managers are embedded, not pointed to.
        put(
            &mut bytes,
            GAME + cgame_state::CHARACTER_MANAGER,
            &0x5000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x5000,
            &((BASE + ccharacter_manager::VTABLE_RVA) as u64).to_le_bytes(),
        );
        let historical = 0x5000 + ccharacter_manager::HISTORICAL_CHARACTERS;
        put(&mut bytes, historical, &0x5100_u64.to_le_bytes());
        put(&mut bytes, historical + 8, &2_u32.to_le_bytes());
        put(&mut bytes, historical + 12, &2_u32.to_le_bytes());
        for (index, (character, id)) in [(0x6000_usize, 44_u32), (0x6200, 45)]
            .into_iter()
            .enumerate()
        {
            put(
                &mut bytes,
                0x5100 + index * 8,
                &(character as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                character,
                &((BASE + ccharacter::VTABLE_RVA) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                character + ccharacter::TYPE,
                &ccharacter::REFERENCE_TYPE.to_le_bytes(),
            );
            put(&mut bytes, character + ccharacter::ID, &id.to_le_bytes());
        }
        put(
            &mut bytes,
            0x6200 + ccharacter::FLAGS,
            &((BASE + cflag_manager::VTABLE_RVA) as u64).to_le_bytes(),
        );
        for (index, (root, offset)) in [
            (Some(GAME), cgame_state::GLOBAL_FLAGS),
            (Some(0x4000), ccountry::FLAG_MANAGER),
            (Some(0x8000), cstate::FLAGS),
            (None, 0x6000 + ccharacter::FLAGS),
        ]
        .into_iter()
        .enumerate()
        {
            let row = 0xb000 + index * cscript_flag::STRIDE;
            let manager = match root {
                Some(root) => {
                    let manager = 0xa000 + index * 0x100;
                    put(&mut bytes, root + offset, &(manager as u64).to_le_bytes());
                    manager
                }
                None => offset,
            };
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
        let mut rows = [ScopedFlag::default(); 4];
        let mut scratch = [cscript_flag::Row::default(); 4];
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            walk_flags(&memory, BASE, GAME, &mut rows, &mut scratch),
            Outcome::Rows { len: 4 }
        );
        // Character 45's empty manager is a scope without rows.
        assert_eq!(
            rows.map(|row| (row.scope_kind, row.scope_id)),
            [
                (GLOBAL_SCOPE, 0),
                (COUNTRY_SCOPE, 2),
                (STATE_SCOPE, 1),
                (CHARACTER_SCOPE, 44)
            ]
        );
        assert!(
            rows.iter()
                .all(|row| row.flag.value == i16::MIN && row.flag.remaining_days == -3)
        );
        // Every later manager's own count is the omitted demand, characters'
        // included; an empty buffer still counts the whole table.
        assert_eq!(
            walk_flags(&memory, BASE, GAME, &mut rows[..1], &mut scratch),
            Outcome::Truncated {
                len: 1,
                unread_rows: 3
            }
        );
        assert_eq!(
            walk_flags(&memory, BASE, GAME, &mut [], &mut []),
            Outcome::Truncated {
                len: 0,
                unread_rows: 4
            }
        );
        // That count sizes the first full hour.
        assert_eq!(script_flag_demand(&memory, BASE, GAME), Some(4));
        let mut capacities = Capacities::default();
        capacities.rows[7] = 1;
        capacities.raise_for_script_flags(4);
        let mut sized = vec![ScopedFlag::default(); capacities.rows[7]];
        let mut scratch = vec![cscript_flag::Row::default(); capacities.rows[7]];
        assert_eq!(
            walk_flags(&memory, BASE, GAME, &mut sized, &mut scratch),
            Outcome::Rows { len: 4 }
        );
        // A character whose manager is not a CFlagManager fails the table.
        let mut broken = bytes.clone();
        put(
            &mut broken,
            0x6200 + ccharacter::FLAGS,
            &0_u64.to_le_bytes(),
        );
        assert!(matches!(
            walk_flags(
                &SliceMemory {
                    base: 0,
                    bytes: &broken
                },
                BASE,
                GAME,
                &mut rows,
                &mut scratch
            ),
            Outcome::Unavailable {
                reason: "flag manager vtable mismatch"
            }
        ));
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

    /// Shared synthetic address space for the relations job harness.
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
        /// The owned-relation vector of owner index `owner`.
        pub(crate) fn own(bytes: &mut [u8], owner: usize, relations: &[usize]) {
            let relations: Vec<u64> = relations.iter().map(|&r| r as u64).collect();
            array(
                bytes,
                DIPLOMACY[owner] + cdiplomacy_status::OWNED_RELATIONS,
                0x3400 + owner * 0x40,
                &relations,
            );
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

        /// Job path under test: the worker walk on `memory`, installed into `intel`.
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
                epoch: 0,
            };
            let report = copy_relations_refresh(memory, ctx, &mut job);
            assert!(report.raw_ok);
            intel.decode_relations_payload(&job, &mut [0; 2]);
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
            for owner in [0, 2] {
                own(&mut bytes, owner, &RELATION);
            }
            bytes
        }
    }

    use relations_test_kit as relations_kit;

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
    fn relations_job_matches_typed_rows_including_skips_and_ordinals() {
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
        assert_eq!(
            report.raw_bytes,
            [
                2 * size_of::<crelation_status::OpinionRow>() as u64,
                2 * size_of::<crelation_status::TupleRow>() as u64
            ]
        );
        relations_kit::assert_matches_direct(&intel, &direct);
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
                        epoch: 0,
                    },
                    &mut job,
                );
                assert!(report.raw_ok, "hour={hour} fault={fault}");
                let mut captured = relations_kit::intel_captured(opinion_cap, tuple_cap);
                let mut ticks = [0; 2];
                captured.decode_relations_payload(&job, &mut ticks);
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
    fn relations_job_matches_truncation_with_exact_missing_counts() {
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
        // Zero, one, exact and ample caps: the job walks into at most the
        // pack's capacity, so its outcome (unread_rows included) and rows
        // equal a typed walk into the pack's own buffers.
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
    fn relations_job_matches_faults_with_identical_reasons() {
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
        for owner in [0, 2] {
            relations_kit::own(
                &mut bytes,
                owner,
                &[
                    relations_kit::RELATION[0],
                    relations_kit::RELATION[1],
                    subject,
                    market,
                ],
            );
        }
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

    thread_local! {
        static MARKS: std::cell::RefCell<Vec<crelation_status::Mark>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    /// Observes this thread's `MARKS` as every opinion write since the last
    /// drain, under one epoch.
    fn test_drain(
        _: &mut u64,
        output: &mut [crelation_status::Mark],
        required: &mut usize,
    ) -> Option<(u64, Option<usize>)> {
        MARKS.with_borrow_mut(|marks| {
            *required = marks.len();
            let fits = marks.len() <= output.len();
            if fits {
                output[..marks.len()].copy_from_slice(marks);
            }
            let count = fits.then_some(marks.len());
            marks.clear();
            Some((1, count))
        })
    }

    fn observed_store(verify: bool) -> &'static OpinionStore {
        let mut shared = SharedOpinions::new();
        shared.drain = test_drain;
        shared.marks = vec![crelation_status::Mark::default(); 8];
        shared.report = verify.then_some(crate::opinion_changes::verified as fn(Option<bool>));
        Box::leak(Box::new(std::sync::Mutex::new(OpinionSlot::Home(shared))))
    }

    fn relations_ctx(epoch: u64) -> RelationsCopyContext {
        RelationsCopyContext {
            base: relations_kit::BASE,
            game_state: relations_kit::GS,
            active: [true; 2],
            opinion_cap: 16,
            tuple_cap: 16,
            epoch,
        }
    }

    /// One refresh released as decode releases it; its opinion outcome and rows.
    fn observed_refresh(
        bytes: &[u8],
        job: &mut RelationsJob,
        epoch: u64,
    ) -> (Outcome, Vec<crelation_status::OpinionRow>) {
        let report =
            copy_relations_refresh(&relations_kit::memory(bytes), relations_ctx(epoch), job);
        assert!(report.raw_ok);
        if job.grow_for_next() {
            job.grow();
        }
        job.return_opinions();
        let outcome = job.walk.unwrap().opinions;
        (outcome, job.opinions[..outcome.len()].to_vec())
    }

    /// Pooled jobs share one opinion store. A refresh that finds it lent
    /// reads every status and yields the same rows; the store comes home,
    /// grown, when decode releases the job holding it.
    #[test]
    fn relations_jobs_share_one_opinion_store() {
        let bytes = relations_kit::opinionated_fixture();
        let store = observed_store(false);
        let [mut first, mut second] = [(); 2].map(|()| RelationsJob {
            store: Some(store),
            ..RelationsJob::new()
        });
        let memory = relations_kit::memory(&bytes);
        let run = |job: &mut RelationsJob| {
            assert!(copy_relations_refresh(&memory, relations_ctx(0), job).raw_ok);
            (
                job.walk.unwrap(),
                job.opinions[..2].to_vec(),
                job.tuples[..2].to_vec(),
            )
        };
        let rows = run(&mut first);
        assert!(first.shared.is_some());
        // Lent: the second job walks without it, to the same rows.
        assert_eq!(run(&mut second), rows);
        assert!(second.shared.is_none());
        first.return_opinions();
        let mut lent = LentOpinions::borrow(store).unwrap();
        assert!(lent.shared().baseline.allocated_bytes() > 0);
        drop(lent);
        // Home and grown: the first job's refresh sized the owners; two more,
        // each returned so the store grows, size and record their rows. The
        // next replays, so an unobserved change to an unmarked status is not
        // read.
        let mut cleared = bytes.clone();
        relations_kit::p64(&mut cleared, relations_kit::status(0, 2), 0);
        for _ in 0..2 {
            assert!(copy_relations_refresh(&memory, relations_ctx(0), &mut second).raw_ok);
            second.return_opinions();
        }
        assert!(
            copy_relations_refresh(
                &relations_kit::memory(&cleared),
                relations_ctx(0),
                &mut second
            )
            .raw_ok
        );
        assert_eq!(second.walk.unwrap().opinions, rows.0.opinions);
    }

    /// The relations dispatch runs across IntelResidual's loan of the pack, as
    /// `acquire` runs it: the context and slice come from the real pack before
    /// the loan, the join (`join_relations`) returns them to it, then the
    /// typed capture and the failure path run as they do there. Published,
    /// short (typed fallback, from the returned slice) and failed reads (no
    /// typed read) all return the slice and the store; an unpublished read is
    /// read again at the next hour. Rejected-inline execution is covered in
    /// `acquisition_jobs`.
    #[test]
    fn relations_dispatch_spans_the_intel_loan_and_returns_its_slice() {
        let bytes = relations_kit::opinionated_fixture();
        let memory = relations_kit::memory(&bytes);
        let mut pack = relations_kit::intel_captured(16, 16);
        let mut store = Some(Box::new(SliceFingerprints::new()));
        // One hour: begin before the loan, walk while it is out, then the
        // join, the typed capture and the failure path. Returns the owners
        // the hour's published rows came from, and whether they were
        // published by the job itself.
        let hour = |pack: &mut Captured,
                    store: &mut Option<Box<SliceFingerprints>>,
                    hour: i32,
                    rows: usize,
                    // A panicked (walked) or stopped (never walked) job.
                    failure: Option<(&'static str, bool)>| {
            let (context, selection) =
                pack.begin_relations(relations_kit::BASE, relations_kit::GS, hour, false, true);
            assert_eq!((context.opinion_cap, context.tuple_cap), (16, 16));
            assert_eq!(pack.relation_activity(), [true; 2]);
            let loaned = std::mem::replace(pack, Captured::vacant());
            // What the old dispatch order read: the placeholder has nothing.
            assert_eq!((pack.opinion_rows_cap(), pack.tuple_rows_cap()), (0, 0));
            let mut job = RelationsJob {
                opinions: vec![crelation_status::OpinionRow::default(); rows],
                tuples: vec![crelation_status::TupleRow::default(); rows],
                selection,
                fingerprints: store.take(),
                ..RelationsJob::default()
            };
            let report = match failure {
                None => copy_relations_refresh(&memory, context, &mut job),
                Some((_, walked)) => {
                    if walked {
                        copy_relations_refresh(&memory, context, &mut job);
                    }
                    // `completed!` maps a failed completion to the default
                    // report, as `acquire` sees it.
                    RelationsCopyReport::default()
                }
            };
            *pack = loaned;
            let failure = failure.map(|(reason, _)| reason);
            let published = report.raw_ok && failure.is_none() && job.published();
            let (fingerprints, payload) = pack.join_relations(job, report, failure);
            *store = fingerprints;
            assert!(store.is_some(), "the store comes back at hour {hour}");
            // The typed capture, as `acquire` runs it after the joins. It
            // resets the relation outcomes to UNCAPTURED; one it walked would
            // hold this fixture's rows.
            pack.capture_delegated(
                &memory,
                CaptureContext {
                    base: relations_kit::BASE,
                    game_state: relations_kit::GS,
                    hour,
                    force_full: false,
                },
            );
            let rows = |rows: &[crelation_status::OpinionRow], outcome: Outcome| {
                rows[..outcome.len()]
                    .iter()
                    .map(|row| row.owner_country)
                    .collect::<std::collections::BTreeSet<u32>>()
            };
            let owners = match payload {
                Ok(job) => {
                    assert!(job.selection.is_none() && job.fingerprints.is_none());
                    assert_eq!(pack.relation_opinions.outcome, UNCAPTURED, "no typed walk");
                    rows(&job.opinions, job.walk.unwrap().opinions)
                }
                Err(job) => {
                    assert!(job.selection.is_none() && job.fingerprints.is_none());
                    if let Some(reason) = failure {
                        // The raw-mode gate kept the typed capture off the
                        // tables; the failure path publishes them unavailable.
                        assert_eq!(pack.relation_opinions.outcome, UNCAPTURED, "no typed walk");
                        assert_eq!(pack.relation_tuples.outcome, UNCAPTURED, "no typed walk");
                        pack.fail_relations_capture(context.active, reason);
                        for outcome in
                            [pack.relation_opinions.outcome, pack.relation_tuples.outcome]
                        {
                            assert_eq!(outcome, Outcome::Unavailable { reason });
                        }
                        Default::default()
                    } else {
                        // The typed fallback read from the returned slice.
                        let outcome = pack.relation_opinions.outcome;
                        assert!(matches!(outcome, Outcome::Rows { .. }), "{outcome:?}");
                        rows(&pack.relation_opinions.rows, outcome)
                    }
                }
            };
            (owners, published)
        };
        // First sight reads every owner; the slice then reads its own.
        let (all, published) = hour(&mut pack, &mut store, 1, 16, None);
        assert!(published && all.len() >= 2);
        let slice = |hour: i32| {
            all.iter()
                .copied()
                .filter(|&owner| replay::cadence::slice_of(owner) == hour.rem_euclid(24) as u32)
                .collect::<std::collections::BTreeSet<_>>()
        };
        let one = *all.iter().next().unwrap();
        let own = replay::cadence::slice_of(one) as i32 + 24;
        assert_eq!(hour(&mut pack, &mut store, own, 16, None).0, slice(own));
        // A short job falls back to the typed walk, which reads exactly this
        // hour's slice from the returned selection. Not published by the job,
        // so the next hour reads this slice again.
        let (typed, published) = hour(&mut pack, &mut store, own + 24, 0, None);
        assert!(!published);
        assert_eq!(typed, slice(own));
        assert_eq!(
            hour(&mut pack, &mut store, own + 25, 16, None).0,
            &slice(own) | &slice(own + 25)
        );
        // A panicked job (it walked) or a stopped one (it never walked)
        // publishes nothing, reads nothing more, and is read again too.
        let (failed, published) = hour(
            &mut pack,
            &mut store,
            own + 48,
            16,
            Some(("acquisition worker panicked", true)),
        );
        assert!(!published && failed.is_empty());
        assert_eq!(
            hour(&mut pack, &mut store, own + 49, 16, None).0,
            &slice(own) | &slice(own + 49)
        );
        let (failed, published) = hour(
            &mut pack,
            &mut store,
            own + 72,
            16,
            Some(("acquisition job stopped", false)),
        );
        assert!(!published && failed.is_empty());
        assert_eq!(
            hour(&mut pack, &mut store, own + 73, 16, None).0,
            &slice(own) | &slice(own + 73)
        );
        // The pack's own selection, with its storage, is back after each join.
        assert!(pack.relations_selection().contains(one) || !slice(own + 49).contains(&one));
    }

    /// Relations are read once a day, one owner slice per hour: a first scan
    /// reads every owner (each is new), a later hour reads only its slice,
    /// and a read left unpublished is read again at the next hour.
    #[test]
    fn a_relations_job_reads_new_owners_then_only_its_hours_slice() {
        let bytes = relations_kit::opinionated_fixture();
        let memory = relations_kit::memory(&bytes);
        let ctx = RelationsCopyContext {
            base: relations_kit::BASE,
            game_state: relations_kit::GS,
            active: [true; 2],
            opinion_cap: 16,
            tuple_cap: 16,
            epoch: 0,
        };
        let mut job = RelationsJob::new();
        let mut store = Some(Box::new(SliceFingerprints::new()));
        let mut read = |hour: i32, store: &mut Option<Box<SliceFingerprints>>| {
            let mut selection = SliceSelection::new(true);
            selection.reset(hour, false);
            job.selection = Some(selection);
            job.fingerprints = store.take();
            assert!(copy_relations_refresh(&memory, ctx, &mut job).raw_ok);
            *store = job.fingerprints.take();
            let walk = job.walk.unwrap();
            let owners: std::collections::BTreeSet<u32> = job.opinions[..walk.opinions.len()]
                .iter()
                .map(|row| row.owner_country)
                .chain(
                    job.tuples[..walk.tuples.len()]
                        .iter()
                        .map(|row| row.owner_country),
                )
                .collect();
            (owners, job.selection.take().unwrap())
        };
        let (all, _) = read(1, &mut store);
        assert!(all.len() >= 2, "{all:?}");
        let slice = |hour: i32| {
            all.iter()
                .copied()
                .filter(|&owner| replay::cadence::slice_of(owner) == hour.rem_euclid(24) as u32)
                .collect::<std::collections::BTreeSet<_>>()
        };
        let (owners, unpublished) = read(25, &mut store);
        assert!(!owners.is_empty());
        assert_eq!(owners, slice(25));
        // Unpublished: the next hour reads its own slice and this one again.
        unpublished.invalidate(store.as_deref_mut().unwrap());
        let (owners, _) = read(26, &mut store);
        assert_eq!(owners, &slice(25) | &slice(26));
        let (owners, _) = read(27, &mut store);
        assert_eq!(owners, slice(27));
    }

    /// A baseline is replayed until the epoch moves (a load, gap or
    /// recovery), and then every status is read again.
    #[test]
    fn relation_baselines_last_until_the_epoch_moves() {
        let bytes = relations_kit::opinionated_fixture();
        let mut job = RelationsJob {
            store: Some(observed_store(false)),
            ..RelationsJob::new()
        };
        // Owners, then their rows, get storage; the third refresh records.
        for _ in 0..3 {
            assert_eq!(
                observed_refresh(&bytes, &mut job, 1).0,
                Outcome::Rows { len: 2 }
            );
        }
        let mut moved = bytes.clone();
        relations_kit::p64(&mut moved, relations_kit::status(0, 2), 0);
        assert_eq!(
            observed_refresh(&moved, &mut job, 1).0,
            Outcome::Rows { len: 2 }
        );
        assert_eq!(
            observed_refresh(&moved, &mut job, 2).0,
            Outcome::Unavailable {
                reason: "relation status vtable mismatch"
            }
        );
    }

    /// Observed refreshes re-read the marked statuses; verify mode compares
    /// each replay with a walk of every status and keeps the reference rows
    /// when an unobserved write made them differ.
    #[test]
    fn observed_refreshes_read_marked_statuses_and_verify_catches_a_missed_write() {
        let _lock = crate::opinion_changes::test_lock();
        crate::opinion_changes::observe_for_test(true);
        let verified = || {
            let status = serde_json::to_value(crate::opinion_changes::status()).unwrap();
            ["unverified_refreshes", "verified_refreshes", "mismatches"]
                .map(|key| status[key].as_u64().unwrap())
        };
        let before = verified();
        let mut bytes = relations_kit::opinionated_fixture();
        let mut job = RelationsJob {
            store: Some(observed_store(true)),
            ..RelationsJob::new()
        };
        let opinion =
            |owner, target| relations_kit::status(owner, target) + crelation_status::OPINION;
        let initial = observed_refresh(&bytes, &mut job, 1);
        assert_eq!(initial.0, Outcome::Rows { len: 2 });
        // Records, then replays (the first replay sizes the verify rows).
        for _ in 0..3 {
            assert_eq!(observed_refresh(&bytes, &mut job, 1), initial);
        }
        // A marked write is read.
        relations_kit::p32(&mut bytes, opinion(1, 3), 61);
        MARKS.with_borrow_mut(|marks| {
            marks.push(crelation_status::Mark {
                status: relations_kit::status(1, 3),
                diplomacy: relations_kit::DIPLOMACY[1],
            })
        });
        let marked = observed_refresh(&bytes, &mut job, 1);
        assert_eq!(marked.1[1].opinion, 61);
        // An unmarked write is caught by verify, which keeps the reference.
        relations_kit::p32(&mut bytes, opinion(1, 3), 62);
        let missed = observed_refresh(&bytes, &mut job, 1);
        assert_eq!(missed.1[1].opinion, 62);
        let delta = |now: [u64; 3]| std::array::from_fn(|index| now[index] - before[index]);
        // Never unverified (until the verify rows existed the walks read in
        // full), agreed three times, then disagreed.
        assert_eq!(delta(verified()), [0, 3, 1]);
        // The mismatch poisoned the real observation: a drain now observes
        // nothing, so every refresh reads every status.
        let (mut consumed, mut output, mut required) =
            (0, [crelation_status::Mark::default(); 4], 0);
        assert_eq!(
            crate::opinion_changes::drain(&mut consumed, &mut output, &mut required),
            None
        );
        assert_eq!(
            serde_json::to_value(crate::opinion_changes::status()).unwrap()["poisoned"],
            true
        );
        // The baseline was dropped: the next refresh reads every status.
        assert_eq!(observed_refresh(&bytes, &mut job, 1), missed);
        assert_eq!(delta(verified()), [0, 3, 1]);
        crate::opinion_changes::observe_for_test(false);
    }

    #[test]
    fn a_short_relations_job_takes_the_typed_fallback_and_grows() {
        let bytes = relations_kit::opinionated_fixture();
        let memory = relations_kit::memory(&bytes);
        let intel = relations_kit::intel_captured(16, 16);
        // A job shorter than the pack's buffers cannot hold the rows: the
        // report declines the payload so the caller falls back to the typed walk.
        let mut tiny = RelationsJob::default();
        let ctx = RelationsCopyContext {
            base: relations_kit::BASE,
            game_state: relations_kit::GS,
            active: [true; 2],
            opinion_cap: intel.opinion_rows_cap(),
            tuple_cap: intel.tuple_rows_cap(),
            epoch: 0,
        };
        let report = copy_relations_refresh(&memory, ctx, &mut tiny);
        assert!(!report.raw_ok);
        assert!(tiny.grow_for_next());
        tiny.grow();
        assert!(copy_relations_refresh(&memory, ctx, &mut tiny).raw_ok);
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
            agency_names: 8,
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
