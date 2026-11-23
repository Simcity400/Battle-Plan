
use super::daily_intel::{Home, Loan, Walk};
use crate::{
    clock::qpc,
    raw::RawLog,
    read_accounting,
    slicing::{SliceFingerprints, SliceSelection},
    table_capture::{Arena, CaptureRows, bounded, grow_capacity, grow_vec},
    table_conversion::{table, table_iter, unavailable},
};
use collector_projection::{
    projection::{Projection, scalar_table},
    table_clock::timed,
};
use engine_allocation::{copy_vec, default_vec, map_slice, owned_vec, reserve, reserve_exact};
use engine_events::incoming_diplomatic_action_queue;
use engine_intelligence::{
    agency_operation_buffers, ccountry_intel_network, ccountry_operation_manager,
    ccountry_operation_token_manager, strategic_operatives,
};
use engine_native_memory::{Memory, cpdx_array::Array, read, read_slice, walk::Outcome};
use engine_native::{
    ccharacter::{self, manager as ccharacter_manager},
    ccountry,
    cgame_state,
    crelation::{
        status as crelation_status,
        subject::{self as csubject_relation, Name},
    },
    cstate,
    cwar_relation,
};
use engine_native_politics::{ccurrent_autonomy_status, cdiplomacy_status};
use engine_native_intelligence::{
    agency_upgrade_levels,
    ccountry_intel,
    ccryptology,
    cintelligence_agency,
    coperation_instance,
    coperative_leader,
    operation_assigned_operative,
    operative_missions,
};
use engine_scripts::{cflag_manager, cscript_flag, cvariables};
use replay_data::{
    BranchId, Hour, rows, rows::TableId, rows::TableRows, rows::tables, sliced::SliceUpdate,
};
use replay_frame::{frame::BaselineToken, frame::TableInput, frame::TableView};
use std::mem::size_of;
use std::sync::Arc;

struct OperativeRefusals {
    rows: std::sync::atomic::AtomicU64,
    samples: std::sync::atomic::AtomicU64,
    first_state: std::sync::atomic::AtomicU64,
    id: std::sync::atomic::AtomicU64,
    roster: std::sync::atomic::AtomicU64,
    kind: std::sync::atomic::AtomicU64,
    reason: std::sync::atomic::AtomicU64,
}

impl OperativeRefusals {
    const fn new() -> Self {
        Self {
            rows: std::sync::atomic::AtomicU64::new(0),
            samples: std::sync::atomic::AtomicU64::new(0),
            first_state: std::sync::atomic::AtomicU64::new(0),
            id: std::sync::atomic::AtomicU64::new(0),
            roster: std::sync::atomic::AtomicU64::new(0),
            kind: std::sync::atomic::AtomicU64::new(0),
            reason: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn observe(&self, count: u32, first: Option<coperative_leader::Refusal>) {
        use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};
        if count == 0 {
            return;
        }
        self.rows.fetch_add(u64::from(count), Relaxed);
        self.samples.fetch_add(1, Relaxed);
        if let Some(first) = first
            && self
                .first_state
                .compare_exchange(0, 1, Acquire, Relaxed)
                .is_ok()
        {
            self.id.store(first.id, Relaxed);
            self.roster.store(u64::from(first.roster_country), Relaxed);
            self.kind.store(u64::from(first.roster_kind), Relaxed);
            self.reason.store(
                if first.reason == "active operative operation reference is null" {
                    1
                } else {
                    2
                },
                Relaxed,
            );
            self.first_state.store(2, Release);
        }
    }
}

static OPERATIVE_REFUSALS: OperativeRefusals = OperativeRefusals::new();
static NETWORK_RESETS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static NETWORK_CACHE_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
static NETWORK_CAPTURE_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct NetworkSummaryCache {
    values: ccountry_intel_network::SummaryCache,
    epoch: u64,
    resets: u64,
}

pub fn network_summary_boundary(invalidate: bool) {
    if invalidate {
        NETWORK_CACHE_EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}
static ROOT_OUT_KEY: std::sync::Mutex<engine_native_parts::modifier_definition::NamedKey> =
    std::sync::Mutex::new(operative_missions::root_out_key());
static OFFER_SPEED_KEY: std::sync::Mutex<engine_native_parts::modifier_definition::NamedKey> =
    std::sync::Mutex::new(agency_operation_buffers::speed_key());

fn with_held_key<R>(
    held: &std::sync::Mutex<engine_native_parts::modifier_definition::NamedKey>,
    fresh: engine_native_parts::modifier_definition::NamedKey,
    walk: impl FnOnce(&mut engine_native_parts::modifier_definition::NamedKey) -> R,
) -> R {
    match held.try_lock() {
        Ok(mut key) => walk(&mut key),
        Err(std::sync::TryLockError::Poisoned(poisoned)) => {
            let mut key = poisoned.into_inner();
            *key = fresh;
            walk(&mut key)
        }
        Err(std::sync::TryLockError::WouldBlock) => walk(&mut { fresh }),
    }
}

type Rosters = std::sync::Arc<[Vec<u64>; 5]>;

static HELD_ROSTERS: std::sync::Mutex<Option<std::collections::HashMap<u32, Rosters>>> =
    std::sync::Mutex::new(None);

fn held_rosters(
    held: &mut std::collections::HashMap<u32, Rosters>,
    country: u32,
    entries: &[strategic_operatives::Entry],
    counts: [usize; 5],
) -> Rosters {
    let mut start = 0;
    let lists = counts.map(|count| {
        let list = &entries[start..start + count];
        start += count;
        list
    });
    if let Some(rosters) = held.get(&country)
        && rosters.iter().zip(lists).all(|(roster, list)| {
            roster.len() == list.len()
                && roster
                    .iter()
                    .zip(list)
                    .all(|(&id, entry)| id == entry.operative)
        })
    {
        return std::sync::Arc::clone(rosters);
    }
    let rosters: Rosters =
        std::sync::Arc::new(lists.map(|list| list.iter().map(|entry| entry.operative).collect()));
    if engine_allocation::reserve_map(held, 1).is_ok() {
        held.insert(country, std::sync::Arc::clone(&rosters));
    }
    rosters
}

pub fn operative_mission_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> operative_missions::Walk {
    with_held_key(&ROOT_OUT_KEY, operative_missions::root_out_key(), |key| {
        operative_missions::walk_buffer(
            memory,
            base,
            state,
            engine_native_memory::walk::Buffer::Count,
            engine_native_memory::walk::Buffer::Count,
            key,
        )
    })
}

static NETWORK_SUMMARY_CACHE: std::sync::Mutex<NetworkSummaryCache> =
    std::sync::Mutex::new(NetworkSummaryCache {
        values: ccountry_intel_network::SummaryCache::new(),
        epoch: 0,
        resets: 0,
    });

static AGENCY_UPGRADE_REFUSALS: [std::sync::atomic::AtomicU64;
    agency_upgrade_levels::STATUS_COUNT] =
    [const { std::sync::atomic::AtomicU64::new(0) }; agency_upgrade_levels::STATUS_COUNT];
static AGENCY_UPGRADE_STORAGE_OMISSIONS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);
static AGENCY_UPGRADE_PEAK_DEMAND: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

fn operative_storage_outcome(outcome: Outcome, refused: u32) -> Outcome {
    match outcome {
        Outcome::Truncated { len, unread_rows } => match unread_rows.checked_sub(refused) {
            Some(0) => Outcome::Rows { len },
            Some(unread_rows) => Outcome::Truncated { len, unread_rows },
            None => Outcome::Unavailable {
                reason: "operative refusal count exceeds missing rows",
            },
        },
        outcome => outcome,
    }
}

#[cfg(any(test, feature = "conversion_check"))]
pub mod conversion_check;
mod demand;
mod flag_store;
pub use flag_store::{FlagReuse, FlagStore};
mod variable_store;
pub use variable_store::{VariableDelta, VariableReuse, VariableStore};
mod variable_work;
use variable_work::{AllScopes, Complete, Path};
pub use variable_work::{VariableTotals, VariableWork};

pub const N: usize = 26;
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
    TableId::AgencyUpgradeLevels,
    TableId::OperativeMissions,
    TableId::StrategicOperatives,
    TableId::AgencyOperationBuffers,
    TableId::IntelNetworkSummaries,
    TableId::PropagandaEfforts,
    TableId::IntelContributions,
    TableId::IntelSources,
    TableId::IntelNetworkAuxiliaries,
];

const fn table_slot(table: TableId) -> usize {
    let mut index = 0;
    while index < TABLES.len() {
        if TABLES[index].index() == table.index() {
            return index;
        }
        index += 1;
    }
    panic!("intel read group table is absent from TABLES");
}

const CONTRIBUTIONS_SLOT: usize = table_slot(TableId::IntelContributions);
const SOURCES_SLOT: usize = table_slot(TableId::IntelSources);

const UNCAPTURED: Outcome = Outcome::Unavailable {
    reason: "clock unavailable",
};

#[derive(Clone, Copy, Debug)]
pub struct CaptureContext {
    pub base: usize,
    pub game_state: usize,
    pub hour: i32,
    pub force_full: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CaptureReport {
    pub ticks: [u64; N],
}

pub const GLOBAL_SCOPE: u8 = 0;
pub const COUNTRY_SCOPE: u8 = 1;
pub const STATE_SCOPE: u8 = 2;
pub const CHARACTER_SCOPE: u8 = 3;

#[derive(Default)]
pub struct SeedWork {
    captured: u64,
    refused: u64,
}

impl SeedWork {
    pub fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::status!({
            "captured": self.captured,
            "refused": self.refused,
            "refusals": crate::status::map(std::iter::once((
                "script variable random seed unreadable", self.refused,
            )).filter(|(_, count)| *count != 0)),
            "scope": "hourly random seeds for admitted ScriptVariables rows; unread seeds remain absent while the entries and scope rows stand",
        })
    }
}

#[derive(Default)]
pub struct ScriptVariableComparison {
    scopes: Vec<u64>,
    entries: Vec<u64>,
    kept: Option<Kept>,
    work: Option<VariableWork>,
}

impl ScriptVariableComparison {
    pub fn take_work(&mut self) -> Option<VariableWork> {
        self.work.take()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Kept {
    at: (u64, BranchId, Hour),
    generation: u64,
}

type Edits = Vec<(u128, Option<rows::ScriptVariablesRow>)>;

#[cfg(feature = "conversion_check")]
fn sorted_scopes(previous: &[rows::ScriptVariablesRow]) -> bool {
    previous
        .windows(2)
        .all(|pair| (pair[0].scope_kind, pair[0].scope_id) < (pair[1].scope_kind, pair[1].scope_id))
}

fn baseline_index(
    previous: &[rows::ScriptVariablesRow],
    ordinal: usize,
    identity: (u8, u32),
) -> Option<usize> {
    let key = |row: &rows::ScriptVariablesRow| (row.scope_kind, row.scope_id);
    if previous.get(ordinal).map(key) == Some(identity) {
        return Some(ordinal);
    }
    previous.binary_search_by_key(&identity, key).ok()
}

fn elapsed_ns(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn keyed_update(token: BaselineToken, mut edits: Edits) -> Option<TableInput> {
    if edits.is_empty() {
        return Some(TableInput::ComparedEqual(token));
    }
    edits.sort_unstable_by_key(|&(key, _)| key);
    if edits.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return None;
    }
    let mut keys = Vec::new();
    let mut rows = Vec::new();
    reserve_exact(&mut keys, edits.len()).ok()?;
    reserve_exact(&mut rows, edits.len()).ok()?;
    for (key, row) in edits {
        keys.push(key);
        rows.extend(row);
    }
    Some(TableInput::SliceUpdate {
        token,
        update: Arc::new(SliceUpdate::keyed(keys, TableRows::ScriptVariables(rows))),
    })
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

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub intel_scratch: usize,
    pub subject_names: usize,
    pub autonomy_names: usize,
    pub agency_names: usize,
    pub agency_upgrade_definitions: usize,
    pub operation_descriptions: usize,
    pub operative_names: usize,
    pub operative_mission_bytes: usize,
    pub operation_parts: usize,
    pub strategic_operative_entries: usize,
    pub agency_operation_entries: usize,
    pub variable_entries: usize,
    pub variable_names: usize,
    pub tokens: usize,
}

collector_primitives::capacity_max!(Capacities {
    rows,
    intel_scratch,
    subject_names,
    autonomy_names,
    agency_names,
    agency_upgrade_definitions,
    operation_descriptions,
    operative_names,
    operative_mission_bytes,
    operation_parts,
    strategic_operative_entries,
    agency_operation_entries,
    variable_entries,
    variable_names,
    tokens,
});

impl Capacities {
    pub fn presize_relations(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let walk = crelation_status::count(memory, base, state, [true, true]);
        let mut failure = None;
        for (index, outcome) in [(13, walk.opinions), (14, walk.tuples)] {
            match outcome {
                Outcome::Rows { len } => self.rows[index] = self.rows[index].max(len),
                Outcome::Truncated { len, unread_rows } => {
                    let count = len
                        .checked_add(unread_rows as usize)
                        .ok_or("relation sizing count overflow")?;
                    self.rows[index] = self.rows[index].max(count);
                }
                Outcome::Unavailable { reason } => {
                    failure.get_or_insert(reason);
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }

    pub fn raise_for_countries(&mut self, slots: usize) {
        use crate::country_scale::{raise, rows};
        for index in [0, 1, 2, 3] {
            raise(&mut self.rows[index], rows(slots, 1));
        }
        let states = Self::default().rows[8];
        raise(&mut self.rows[8], rows(slots, 1).saturating_add(states));
    }

    pub fn raise_for_country_intel(&mut self, native_rows: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[15], rows(native_rows, 1));
    }

    pub fn raise_for_captured_operatives(&mut self, count: usize) {
        crate::country_scale::raise(&mut self.rows[16], crate::country_scale::rows(count, 1));
    }

    pub fn raise_for_script_flags(&mut self, flags: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[7], rows(flags, 1));
    }

    pub fn raise_for_script_variables(&mut self, demand: VariableDemand) {
        use crate::country_scale::{raise, rows};
        crate::variable_changes::offer_scopes(demand.scopes);
        raise(&mut self.rows[8], rows(demand.scopes, 1));
        raise(&mut self.variable_entries, rows(demand.entries, 1));
        raise(&mut self.variable_names, rows(demand.bytes, 1));
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VariableDemand {
    pub scopes: usize,
    pub entries: usize,
    pub bytes: usize,
}

impl VariableDemand {
    pub fn max(self, other: Self) -> Self {
        Self {
            scopes: self.scopes.max(other.scopes),
            entries: self.entries.max(other.entries),
            bytes: self.bytes.max(other.bytes),
        }
    }
}

pub fn script_variable_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Option<VariableDemand> {
    let walk = cvariables::count_scopes(memory, base, state);
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

pub fn script_flag_demand(memory: &impl Memory, base: usize, state: usize) -> Option<usize> {
    match flag_demand(memory, base, state) {
        Outcome::Rows { len } => Some(len),
        Outcome::Truncated { unread_rows, .. } => Some(unread_rows as usize),
        Outcome::Unavailable { .. } => None,
    }
}

fn flag_demand(memory: &impl Memory, base: usize, state: usize) -> Outcome {
    let mut scopes = 0;
    let outcome = walk_flags_buffer(
        memory,
        base,
        state,
        engine_native_memory::walk::Buffer::Count,
        engine_native_memory::walk::Buffer::Count,
        None,
        Some(&mut scopes),
    );
    if !matches!(outcome, Outcome::Unavailable { .. }) {
        crate::flag_changes::offer_scopes(scopes);
    }
    outcome
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ScopedFlag {
    scope_kind: u8,
    scope_id: u32,
    flag: cscript_flag::Row,
}

#[derive(Default)]
struct NativeShadow<R> {
    rows: Vec<R>,
    output: Option<Arc<TableRows>>,
    hour: Option<i32>,
    witness: Option<u64>,
    hits: u64,
    misses: u64,
    republished: u64,
}

impl<R: Copy + PartialEq> NativeShadow<R> {
    fn clear(&mut self) {
        self.rows.clear();
        self.output = None;
        self.hour = None;
        self.witness = None;
    }

    fn input(
        &mut self,
        id: TableId,
        current: &CaptureRows<R>,
        outcome: Outcome,
        (hour, witness): (i32, Option<u64>),
        convert: impl FnOnce() -> TableView,
    ) -> TableInput {
        if matches!(
            outcome,
            Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON
            }
        ) {
            return TableInput::Sampled(convert());
        }
        if witness.is_some()
            && self.witness == witness
            && self.hour.and_then(|previous| previous.checked_add(1)) == Some(hour)
            && let Some(output) = &self.output
            && output.table() == id
        {
            self.hour = Some(hour);
            self.republished = self.republished.saturating_add(1);
            return TableInput::SharedComplete(Arc::clone(output));
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
            self.hour = Some(hour);
            self.witness = witness;
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
            self.hour = Some(hour);
            self.witness = witness;
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
            republished: self.republished,
            row_capacity_bytes: self.rows.capacity() * size_of::<R>(),
            replay_capacity_bytes,
        }
    }
}

#[derive(Default)]
pub struct IntelShadow {
    branch: Option<BranchId>,
    flags: NativeShadow<ScopedFlag>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IntelShadowStats {
    pub hits: u64,
    pub misses: u64,
    pub republished: u64,
    pub row_capacity_bytes: usize,
    pub replay_capacity_bytes: usize,
}

impl IntelShadow {
    pub fn statistics(&self) -> IntelShadowStats {
        self.flags.statistics()
    }

    pub fn forget(&mut self, table: TableId) {
        if table == TableId::ScriptFlags {
            self.flags.clear();
        }
    }
}

pub struct IntelConversion<'a> {
    shadow: &'a mut IntelShadow,
    hour: i32,
}

impl<'a> IntelConversion<'a> {
    pub fn new(branch: BranchId, hour: i32, shadow: &'a mut IntelShadow) -> Self {
        if shadow.branch != Some(branch) {
            shadow.flags.clear();
            shadow.branch = Some(branch);
        }
        Self { shadow, hour }
    }
}

pub struct ResidualAux {
    context: CaptureContext,
    left_out: [bool; N],
    ticks: [u64; N],
    subjects: CaptureRows<csubject_relation::Row>,
    subject_names: Arena<u8>,
    subject_snapshot: cwar_relation::OwnedSnapshotScratch,
    autonomy: CaptureRows<ccurrent_autonomy_status::Row>,
    autonomy_names: Arena<u8>,
    agencies: CaptureRows<cintelligence_agency::Row>,
    agency_names: Arena<u8>,
    agency_upgrades: CaptureRows<agency_upgrade_levels::Row>,
    agency_upgrade_catalog: agency_upgrade_levels::Catalog,
    agency_upgrade_required: usize,
    operatives: CaptureRows<coperative_leader::Row>,
    operative_names: Arena<u8>,
    operative_failure: Option<(u64, u32, u8)>,
    operative_refused_rows: u32,
    operative_first_refused: Option<coperative_leader::Refusal>,
    captured_operatives: CaptureRows<cintelligence_agency::CapturedOperativeRow>,
    operative_missions: CaptureRows<operative_missions::Row>,
    operative_mission_bytes: Arena<u8>,
    operative_mission_failure: Option<(u64, u32)>,
    operative_effects: Vec<crate::operative_effects::Effects>,
    cryptology: CaptureRows<ccryptology::Row>,
    agency_operation_buffers: CaptureRows<agency_operation_buffers::Row>,
    agency_operation_entries: Arena<agency_operation_buffers::Entry>,
    operations: CaptureRows<coperation_instance::Row>,
    assignments: CaptureRows<operation_assigned_operative::Row>,
    operation_descriptions: Arena<u8>,
    operation_parts: Arena<coperation_instance::Part>,
    operation_tokens: CaptureRows<ccountry_operation_token_manager::Row>,
    tokens: Arena<u32>,
    incoming_actions: CaptureRows<incoming_diplomatic_action_queue::Row>,
}

impl ResidualAux {
    pub fn capture(&mut self, memory: &impl Memory) {
        self.subjects.reset();
        self.autonomy.reset();
        self.agencies.reset();
        self.agency_upgrades.reset();
        self.agency_upgrade_required = 0;
        self.operatives.reset();
        self.captured_operatives.reset();
        self.operative_failure = None;
        self.operative_refused_rows = 0;
        self.operative_first_refused = None;
        self.operative_missions.reset();
        self.operative_effects.clear();
        self.operative_mission_failure = None;
        self.agency_operation_buffers.reset();
        self.cryptology.reset();
        self.operations.reset();
        self.assignments.reset();
        self.operation_tokens.reset();
        self.incoming_actions.reset();
        self.subject_names.reset();
        self.autonomy_names.reset();
        self.agency_names.reset();
        self.operative_names.reset();
        self.operative_mission_bytes.reset();
        self.agency_operation_entries.reset();
        self.operation_descriptions.reset();
        self.operation_parts.reset();
        self.tokens.reset();
        let CaptureContext {
            base,
            game_state: state,
            hour,
            force_full,
        } = self.context;
        let mut active =
            TABLES.map(|table| replay_data::cadence::should_walk(table, hour, force_full));
        for (active, &left_out) in active.iter_mut().zip(&self.left_out) {
            *active &= !left_out;
        }
        macro_rules! skip {
            ($($field:ident = $index:literal),+ $(,)?) => {$(
                if !active[$index] {
                    self.$field.outcome = Outcome::Unavailable {
                        reason: replay_data::cadence::SKIPPED_REASON,
                    };
                }
            )+};
        }
        skip!(
            subjects = 1,
            autonomy = 2,
            agencies = 3,
            operatives = 4,
            cryptology = 5,
            operations = 9,
            assignments = 10,
            operation_tokens = 11,
            incoming_actions = 12,
            captured_operatives = 16,
            agency_upgrades = 17,
            operative_missions = 18,
            agency_operation_buffers = 20,
        );
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
        if !(self.left_out[3] && self.left_out[17]) {
            let begin = qpc();
            let result = read_accounting::with_table(TABLES[3], || {
                cintelligence_agency::walk_with_upgrades(
                    memory,
                    base,
                    state,
                    &mut self.agencies.rows,
                    &mut self.agency_names.first,
                    &mut self.agency_upgrades.rows,
                    &mut self.agency_upgrade_catalog,
                )
            });
            first_named(
                &mut self.agencies,
                &mut self.agency_names,
                ArenaRead::from(result.agencies),
            );
            self.agency_upgrades.outcome =
                bounded(result.upgrades.outcome, self.agency_upgrades.rows.len());
            self.agency_upgrade_required = result.upgrades.required_rows;
            AGENCY_UPGRADE_PEAK_DEMAND.fetch_max(
                result.upgrades.required_rows,
                std::sync::atomic::Ordering::Relaxed,
            );
            AGENCY_UPGRADE_STORAGE_OMISSIONS.fetch_add(
                u64::from(result.upgrades.omitted_rows),
                std::sync::atomic::Ordering::Relaxed,
            );
            for (counter, count) in AGENCY_UPGRADE_REFUSALS
                .iter()
                .zip(result.upgrades.refusal_counts)
            {
                counter.fetch_add(u64::from(count), std::sync::atomic::Ordering::Relaxed);
            }
            ticks[3] = elapsed(begin);
        }
        if active[4] {
            let begin = qpc();
            capture_named(
                &mut self.operatives,
                &mut self.operative_names,
                |rows, names| {
                    let _reads = read_accounting::table(TABLES[4]);
                    let read = coperative_leader::walk(memory, base, state, rows, names);
                    self.operative_failure = read.failed_membership;
                    self.operative_refused_rows = read.refused_rows;
                    self.operative_first_refused = read.first_refused;
                    OPERATIVE_REFUSALS.observe(read.refused_rows, read.first_refused);
                    ArenaRead {
                        outcome: read.outcome,
                        len: read.bytes_len,
                        required: read.required_bytes,
                    }
                },
            );
            ticks[4] = elapsed(begin);
        }
        plain!(
            16,
            captured_operatives,
            cintelligence_agency::walk_captured_operatives
        );
        if active[18] {
            let begin = qpc();
            capture_named(
                &mut self.operative_missions,
                &mut self.operative_mission_bytes,
                |rows, bytes| {
                    let _reads = read_accounting::table(TABLES[18]);
                    let read = with_held_key(
                        &ROOT_OUT_KEY,
                        operative_missions::root_out_key(),
                        |key| operative_missions::walk(memory, base, state, rows, bytes, key),
                    );
                    self.operative_mission_failure = read.failed_operative;
                    ArenaRead {
                        outcome: read.outcome,
                        len: read.bytes_len,
                        required: read.required_bytes,
                    }
                },
            );
            {
                let _reads = read_accounting::table(TABLES[18]);
                crate::operative_effects::snapshot(
                    memory,
                    base,
                    state,
                    &self.operative_missions.rows[..self.operative_missions.outcome.len()],
                    &mut self.operative_effects,
                );
            }
            ticks[18] = elapsed(begin);
        }
        plain!(5, cryptology, ccryptology::walk);
        if active[20] {
            let begin = qpc();
            capture_named(
                &mut self.agency_operation_buffers,
                &mut self.agency_operation_entries,
                |rows, entries| {
                    let _reads = read_accounting::table(TABLES[20]);
                    let read = with_held_key(
                        &OFFER_SPEED_KEY,
                        agency_operation_buffers::speed_key(),
                        |key| {
                            agency_operation_buffers::walk(
                                memory, base, state, rows, entries, key,
                            )
                        },
                    );
                    ArenaRead {
                        outcome: read.outcome,
                        len: read.entries_len,
                        required: read.required_entries,
                    }
                },
            );
            ticks[20] = elapsed(begin);
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
                    &mut self.operation_descriptions.first,
                    &mut self.operation_parts.first,
                )
            });
            first_named(
                &mut self.operations,
                &mut self.operation_descriptions,
                ArenaRead {
                    outcome: result.outcome,
                    len: result.bytes_len,
                    required: result.required_bytes,
                },
            );
            self.operation_parts.len = result.parts_len;
            self.operation_parts.required = result.required_parts;
            if !valid_arena(
                self.operations.outcome,
                &self.operations.rows,
                &self.operation_parts.first,
                result.parts_len,
                result.required_parts,
            ) {
                self.operations.outcome = Outcome::Unavailable {
                    reason: "captured arena range invalid",
                };
            }
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
        plain!(
            12,
            incoming_actions,
            incoming_diplomatic_action_queue::walk
        );
        self.ticks = ticks;
    }
}

pub struct Captured {
    relation_active: [bool; 2],
    left_out: [bool; N],
    variables_apart: bool,
    residual_aux_apart: bool,
    capitulation: CaptureRows<cdiplomacy_status::CapitulationRow>,
    war_goal_recipients: Vec<u32>,
    subjects: CaptureRows<csubject_relation::Row>,
    autonomy: CaptureRows<ccurrent_autonomy_status::Row>,
    agencies: CaptureRows<cintelligence_agency::Row>,
    agency_upgrades: CaptureRows<agency_upgrade_levels::Row>,
    agency_upgrade_catalog: agency_upgrade_levels::Catalog,
    agency_upgrade_required: usize,
    operatives: CaptureRows<coperative_leader::Row>,
    captured_operatives: CaptureRows<cintelligence_agency::CapturedOperativeRow>,
    operative_failure: Option<(u64, u32, u8)>,
    operative_refused_rows: u32,
    operative_first_refused: Option<coperative_leader::Refusal>,
    operative_missions: CaptureRows<operative_missions::Row>,
    operative_effects: Vec<crate::operative_effects::Effects>,
    operative_mission_failure: Option<(u64, u32)>,
    strategic_operatives: CaptureRows<strategic_operatives::Row>,
    agency_operation_buffers: CaptureRows<agency_operation_buffers::Row>,
    cryptology: CaptureRows<ccryptology::Row>,
    networks: CaptureRows<ccountry_intel_network::Row>,
    network_summaries: CaptureRows<ccountry_intel_network::Summary>,
    propaganda: CaptureRows<crate::propaganda_changes::Row>,
    source_contributions: CaptureRows<rows::IntelContributionRow>,
    source_values: CaptureRows<rows::IntelSourceRow>,
    network_auxiliary: CaptureRows<rows::IntelNetworkAuxiliaryRow>,
    certified_snapshots: [Option<Arc<crate::retained_snapshot::Snapshot>>; 4],
    source_seeds: Vec<crate::source_state::Seed>,
    source_census_complete: bool,
    flags: CaptureRows<ScopedFlag>,
    variables: CaptureRows<cvariables::ScopeRow>,
    variable_delta: VariableDelta,
    operations: CaptureRows<coperation_instance::Row>,
    assignments: CaptureRows<operation_assigned_operative::Row>,
    operation_tokens: CaptureRows<ccountry_operation_token_manager::Row>,
    incoming_actions: CaptureRows<incoming_diplomatic_action_queue::Row>,
    relation_opinions: CaptureRows<crelation_status::OpinionRow>,
    relation_tuples: CaptureRows<crelation_status::TupleRow>,
    relation_resolver: crelation_status::Resolver,
    subject_snapshot: cwar_relation::OwnedSnapshotScratch,
    country_intel: Loan,
    country_intel_outcome: Outcome,
    country_intel_sample: Option<Result<rows::ReservedCountryIntelSample, &'static str>>,
    subject_names: Arena<u8>,
    autonomy_names: Arena<u8>,
    agency_names: Arena<u8>,
    operative_names: Arena<u8>,
    operative_mission_bytes: Arena<u8>,
    strategic_operative_entries: Arena<strategic_operatives::Entry>,
    agency_operation_entries: Arena<agency_operation_buffers::Entry>,
    operation_descriptions: Arena<u8>,
    operation_parts: Arena<coperation_instance::Part>,
    variable_entries: Arena<cvariables::Row>,
    variable_names: Arena<u8>,
    tokens: Arena<u32>,
    flag_scratch: Vec<cscript_flag::Row>,
    flags_witness: Option<u64>,
    pub relations_raw_mode: bool,
    relations_selection: SliceSelection,
    pub capitulation_raw_mode: bool,
}

impl Captured {
    pub fn relation_activity(&self) -> [bool; 2] {
        self.relation_active
    }

    pub fn set_relation_activity(&mut self, active: [bool; 2]) {
        self.relation_active = active;
    }
    pub fn leave_out(&mut self, left_out: [bool; N]) {
        self.left_out = left_out;
    }

    pub fn vacant() -> Self {
        Self::build(
            &Capacities {
                rows: [0; N],
                intel_scratch: 0,
                subject_names: 0,
                autonomy_names: 0,
                agency_names: 0,
                agency_upgrade_definitions: 0,
                operation_descriptions: 0,
                operative_names: 0,
                operative_mission_bytes: 0,
                operation_parts: 0,
                strategic_operative_entries: 0,
                agency_operation_entries: 0,
                variable_entries: 0,
                variable_names: 0,
                tokens: 0,
            },
            Loan::default(),
        )
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self::build(
            capacity,
            Home::try_new(capacity.rows[15], capacity.intel_scratch)
                .map(Loan::new)
                .unwrap_or_default(),
        )
    }

    pub fn without_home(capacity: &Capacities) -> Self {
        Self::build(capacity, Loan::default())
    }

    pub fn country_intel_home(&self) -> Option<&Home> {
        self.country_intel.home()
    }

    pub fn share_country_intel(&mut self, home: &Home) {
        self.country_intel = Loan::new(home.clone());
    }

    pub fn take_country_intel_loan(&mut self) -> Loan {
        std::mem::take(&mut self.country_intel)
    }

    pub fn restore_country_intel_loan(&mut self, loan: Loan) {
        self.country_intel = loan;
    }

    pub fn release_country_intel(&mut self) {
        self.country_intel.release();
    }

    fn build(capacity: &Capacities, country_intel: Loan) -> Self {
        Self {
            capitulation: CaptureRows::new(capacity.rows[0]),
            war_goal_recipients: Vec::new(),
            subjects: CaptureRows::new(capacity.rows[1]),
            autonomy: CaptureRows::new(capacity.rows[2]),
            agencies: CaptureRows::new(capacity.rows[3]),
            agency_upgrades: CaptureRows::new(capacity.rows[17]),
            agency_upgrade_catalog: agency_upgrade_levels::Catalog::new(
                capacity.agency_upgrade_definitions,
            ),
            agency_upgrade_required: 0,
            operatives: CaptureRows::new(capacity.rows[4]),
            captured_operatives: CaptureRows::new(capacity.rows[16]),
            operative_failure: None,
            operative_refused_rows: 0,
            operative_first_refused: None,
            operative_missions: CaptureRows::new(capacity.rows[18]),
            operative_effects: Vec::new(),
            operative_mission_failure: None,
            strategic_operatives: CaptureRows::new(capacity.rows[19]),
            agency_operation_buffers: CaptureRows::new(capacity.rows[20]),
            cryptology: CaptureRows::new(capacity.rows[5]),
            networks: CaptureRows::new(capacity.rows[6]),
            network_summaries: CaptureRows::new(capacity.rows[21]),
            propaganda: CaptureRows::new(capacity.rows[22]),
            source_contributions: CaptureRows::new(capacity.rows[23]),
            source_values: CaptureRows::new(capacity.rows[24]),
            network_auxiliary: CaptureRows::new(capacity.rows[25]),
            certified_snapshots: std::array::from_fn(|_| None),
            source_seeds: Vec::new(),
            source_census_complete: false,
            flags: CaptureRows::new(capacity.rows[7]),
            variables: CaptureRows::new(capacity.rows[8]),
            variable_delta: VariableDelta::default(),
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
            country_intel_sample: None,
            subject_names: Arena::new(capacity.subject_names),
            autonomy_names: Arena::new(capacity.autonomy_names),
            agency_names: Arena::new(capacity.agency_names),
            operative_names: Arena::new(capacity.operative_names),
            operative_mission_bytes: Arena::new(capacity.operative_mission_bytes),
            strategic_operative_entries: Arena::new(capacity.strategic_operative_entries),
            agency_operation_entries: Arena::new(capacity.agency_operation_entries),
            operation_descriptions: Arena::new(capacity.operation_descriptions),
            operation_parts: Arena::new(capacity.operation_parts),
            variable_entries: Arena::new(capacity.variable_entries),
            variable_names: Arena::new(capacity.variable_names),
            tokens: Arena::new(capacity.tokens),
            flag_scratch: owned_vec(default_vec(capacity.rows[7])),
            flags_witness: None,
            relations_raw_mode: false,
            relations_selection: SliceSelection::new(),
            relation_active: [false; 2],
            left_out: [false; N],
            variables_apart: false,
            residual_aux_apart: false,
            capitulation_raw_mode: false,
        }
    }

    pub fn relations_selection(&self) -> &SliceSelection {
        &self.relations_selection
    }

    pub fn take_relations_selection(&mut self, hour: i32, force_full: bool) -> SliceSelection {
        self.relations_selection.reset(hour, force_full);
        std::mem::replace(&mut self.relations_selection, SliceSelection::new())
    }

    pub fn restore_relations_selection(&mut self, selection: SliceSelection) {
        self.relations_selection = selection;
    }

    pub fn begin_relations(
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
            active: [TABLES[13], TABLES[14]]
                .map(|table| replay_data::cadence::should_walk(table, hour, force_full)),
            epoch: relation_epoch(),
        };
        self.set_relation_activity(context.active);
        let selection = refresh.then(|| self.take_relations_selection(hour, force_full));
        (context, selection)
    }

    #[allow(clippy::type_complexity)]
    pub fn join_relations(
        &mut self,
        mut job: RelationsJob,
        report: RelationsCopyReport,
        failure: Option<&'static str>,
    ) -> (
        Option<Box<SliceFingerprints>>,
        Result<RelationsJob, RelationsJob>,
    ) {
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

    pub fn end_relations(
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
        self.flags_witness = None;
        if !self.residual_aux_apart {
            self.subjects.reset();
            self.autonomy.reset();
            self.agencies.reset();
            self.agency_upgrades.reset();
            self.agency_upgrade_required = 0;
            self.operatives.reset();
            self.captured_operatives.reset();
            self.operative_failure = None;
            self.operative_refused_rows = 0;
            self.operative_first_refused = None;
            self.operative_missions.reset();
            self.operative_effects.clear();
            self.operative_mission_failure = None;
            self.agency_operation_buffers.reset();
            self.cryptology.reset();
            self.operations.reset();
            self.assignments.reset();
            self.operation_tokens.reset();
            self.incoming_actions.reset();
            self.subject_names.reset();
            self.autonomy_names.reset();
            self.agency_names.reset();
            self.operative_names.reset();
            self.operative_mission_bytes.reset();
            self.agency_operation_entries.reset();
            self.operation_descriptions.reset();
            self.operation_parts.reset();
            self.tokens.reset();
        }
        self.strategic_operatives.reset();
        self.networks.reset();
        self.network_summaries.reset();
        self.propaganda.reset();
        self.source_contributions.reset();
        self.source_values.reset();
        self.network_auxiliary.reset();
        self.certified_snapshots.fill(None);
        self.source_census_complete = false;
        self.flags.reset();
        self.variables.reset();
        self.variable_delta.clear();
        self.country_intel.release();
        self.country_intel_outcome = UNCAPTURED;
        self.country_intel_sample = None;
        self.strategic_operative_entries.reset();
        self.variable_entries.reset();
        self.variable_names.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        let handed_off = self
            .certified_snapshots
            .iter()
            .zip(&self.outcomes()[22..26])
            .filter_map(|(snapshot, outcome)| {
                matches!(outcome, Outcome::Rows { .. })
                    .then_some(snapshot.as_ref())
                    .flatten()
            })
            .map(|snapshot| snapshot.acquired_bytes())
            .sum::<usize>();
        handed_off
            + self.capitulation.allocated_bytes()
            + self.war_goal_recipients.capacity() * size_of::<u32>()
            + self.subjects.allocated_bytes()
            + self.autonomy.allocated_bytes()
            + self.agencies.allocated_bytes()
            + self.agency_upgrades.allocated_bytes()
            + self.agency_upgrade_catalog.allocated_bytes()
            + self.operatives.allocated_bytes()
            + self.captured_operatives.allocated_bytes()
            + self.operative_missions.allocated_bytes()
            + self.operative_effects.capacity() * size_of::<crate::operative_effects::Effects>()
            + self.strategic_operatives.allocated_bytes()
            + self.agency_operation_buffers.allocated_bytes()
            + self.cryptology.allocated_bytes()
            + self.networks.allocated_bytes()
            + self.network_summaries.allocated_bytes()
            + self.propaganda.allocated_bytes()
            + self.source_contributions.allocated_bytes()
            + self.source_values.allocated_bytes()
            + self.network_auxiliary.allocated_bytes()
            + self.source_seeds.capacity() * size_of::<crate::source_state::Seed>()
            + self.flags.allocated_bytes()
            + self.variables.allocated_bytes()
            + self.variable_delta.allocated_bytes()
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
            + self.operative_names.allocated_bytes()
            + self.operative_mission_bytes.allocated_bytes()
            + self.strategic_operative_entries.allocated_bytes()
            + self.agency_operation_entries.allocated_bytes()
            + self.operation_descriptions.allocated_bytes()
            + self.operation_parts.allocated_bytes()
            + self.variable_entries.allocated_bytes()
            + self.variable_names.allocated_bytes()
            + self.tokens.allocated_bytes()
            + self.flag_scratch.capacity() * size_of::<cscript_flag::Row>()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        for (index, outcome) in [
            self.propaganda.outcome,
            self.source_contributions.outcome,
            self.source_values.outcome,
            self.network_auxiliary.outcome,
        ]
        .into_iter()
        .enumerate()
        {
            if !matches!(outcome, Outcome::Rows { .. }) {
                continue;
            }
            if let Some(snapshot) = &self.certified_snapshots[index]
                && let Some(acquired) = snapshot.recycle_acquired()
            {
                use crate::retained_snapshot::AcquiredRows;
                match acquired {
                    AcquiredRows::Propaganda(rows) => self.propaganda.rows = rows,
                    AcquiredRows::Contributions(rows) => self.source_contributions.rows = rows,
                    AcquiredRows::Sources(rows) => self.source_values.rows = rows,
                    AcquiredRows::NetworkAuxiliaries(rows) => self.network_auxiliary.rows = rows,
                }
            }
        }
        self.capitulation.grow(&mut capacity.rows[0]);
        self.subjects.grow(&mut capacity.rows[1]);
        self.autonomy.grow(&mut capacity.rows[2]);
        self.agencies.grow(&mut capacity.rows[3]);
        grow_capacity(&mut capacity.rows[17], self.agency_upgrade_required);
        grow_vec(&mut self.agency_upgrades.rows, capacity.rows[17]);
        grow_capacity(
            &mut capacity.agency_upgrade_definitions,
            self.agency_upgrade_catalog.required_capacity(),
        );
        self.agency_upgrade_catalog
            .resize(capacity.agency_upgrade_definitions);
        let operative_outcome = self.operatives.outcome;
        self.operatives.outcome =
            operative_storage_outcome(operative_outcome, self.operative_refused_rows);
        self.operatives.grow(&mut capacity.rows[4]);
        self.captured_operatives.grow(&mut capacity.rows[16]);
        self.operatives.outcome = operative_outcome;
        self.operative_missions.grow(&mut capacity.rows[18]);
        self.agency_operation_entries
            .grow(&mut capacity.agency_operation_entries);
        self.strategic_operatives.grow(&mut capacity.rows[19]);
        self.agency_operation_buffers.grow(&mut capacity.rows[20]);
        self.cryptology.grow(&mut capacity.rows[5]);
        self.networks.grow(&mut capacity.rows[6]);
        self.network_summaries.grow(&mut capacity.rows[21]);
        self.propaganda.grow(&mut capacity.rows[22]);
        self.source_contributions.grow(&mut capacity.rows[23]);
        self.source_values.grow(&mut capacity.rows[24]);
        self.network_auxiliary.grow(&mut capacity.rows[25]);
        self.flags.grow(&mut capacity.rows[7]);
        self.variables.grow(&mut capacity.rows[8]);
        self.operations.grow(&mut capacity.rows[9]);
        self.assignments.grow(&mut capacity.rows[10]);
        self.operation_tokens.grow(&mut capacity.rows[11]);
        self.incoming_actions.grow(&mut capacity.rows[12]);
        self.relation_opinions.grow(&mut capacity.rows[13]);
        self.relation_tuples.grow(&mut capacity.rows[14]);
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
        self.country_intel.grow(&mut capacity.rows[15]);
        self.subject_names.grow(&mut capacity.subject_names);
        self.autonomy_names.grow(&mut capacity.autonomy_names);
        self.agency_names.grow(&mut capacity.agency_names);
        self.operative_names.grow(&mut capacity.operative_names);
        self.operative_mission_bytes
            .grow(&mut capacity.operative_mission_bytes);
        self.strategic_operative_entries
            .grow(&mut capacity.strategic_operative_entries);
        self.operation_descriptions
            .grow(&mut capacity.operation_descriptions);
        self.operation_parts.grow(&mut capacity.operation_parts);
        self.variable_entries.grow(&mut capacity.variable_entries);
        self.variable_names.grow(&mut capacity.variable_names);
        self.tokens.grow(&mut capacity.tokens);
        grow_vec(&mut self.flag_scratch, capacity.rows[7]);
    }

    #[cfg(any(test, feature = "test-support"))]
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
            None,
            None,
        );
        report.ticks
    }

    pub fn capture_residual(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        flags: Option<&mut FlagStore>,
        variables: Option<&mut VariableStore>,
    ) -> CaptureReport {
        self.capture_tables(memory, context, true, false, flags, variables)
    }

    pub fn take_residual_aux(&mut self, context: CaptureContext) -> ResidualAux {
        self.residual_aux_apart = true;
        ResidualAux {
            context,
            left_out: self.left_out,
            ticks: [0; N],
            subjects: std::mem::replace(&mut self.subjects, CaptureRows::new(0)),
            subject_names: std::mem::replace(&mut self.subject_names, Arena::new(0)),
            subject_snapshot: std::mem::take(&mut self.subject_snapshot),
            autonomy: std::mem::replace(&mut self.autonomy, CaptureRows::new(0)),
            autonomy_names: std::mem::replace(&mut self.autonomy_names, Arena::new(0)),
            agencies: std::mem::replace(&mut self.agencies, CaptureRows::new(0)),
            agency_names: std::mem::replace(&mut self.agency_names, Arena::new(0)),
            agency_upgrades: std::mem::replace(&mut self.agency_upgrades, CaptureRows::new(0)),
            agency_upgrade_catalog: std::mem::replace(
                &mut self.agency_upgrade_catalog,
                agency_upgrade_levels::Catalog::new(0),
            ),
            agency_upgrade_required: std::mem::take(&mut self.agency_upgrade_required),
            operatives: std::mem::replace(&mut self.operatives, CaptureRows::new(0)),
            operative_names: std::mem::replace(&mut self.operative_names, Arena::new(0)),
            operative_failure: self.operative_failure.take(),
            operative_refused_rows: std::mem::take(&mut self.operative_refused_rows),
            operative_first_refused: self.operative_first_refused.take(),
            captured_operatives: std::mem::replace(
                &mut self.captured_operatives,
                CaptureRows::new(0),
            ),
            operative_missions: std::mem::replace(
                &mut self.operative_missions,
                CaptureRows::new(0),
            ),
            operative_mission_bytes: std::mem::replace(
                &mut self.operative_mission_bytes,
                Arena::new(0),
            ),
            operative_mission_failure: self.operative_mission_failure.take(),
            operative_effects: std::mem::take(&mut self.operative_effects),
            cryptology: std::mem::replace(&mut self.cryptology, CaptureRows::new(0)),
            agency_operation_buffers: std::mem::replace(
                &mut self.agency_operation_buffers,
                CaptureRows::new(0),
            ),
            agency_operation_entries: std::mem::replace(
                &mut self.agency_operation_entries,
                Arena::new(0),
            ),
            operations: std::mem::replace(&mut self.operations, CaptureRows::new(0)),
            assignments: std::mem::replace(&mut self.assignments, CaptureRows::new(0)),
            operation_descriptions: std::mem::replace(
                &mut self.operation_descriptions,
                Arena::new(0),
            ),
            operation_parts: std::mem::replace(&mut self.operation_parts, Arena::new(0)),
            operation_tokens: std::mem::replace(&mut self.operation_tokens, CaptureRows::new(0)),
            tokens: std::mem::replace(&mut self.tokens, Arena::new(0)),
            incoming_actions: std::mem::replace(&mut self.incoming_actions, CaptureRows::new(0)),
        }
    }

    pub fn restore_residual_aux(
        &mut self,
        aux: Option<ResidualAux>,
        context: CaptureContext,
        reason: &'static str,
    ) -> [u64; N] {
        self.residual_aux_apart = false;
        let Some(aux) = aux else {
            macro_rules! fail_aux {
                ($($index:literal => $field:ident),+ $(,)?) => {$(
                    self.$field.outcome = Outcome::Unavailable {
                        reason: if replay_data::cadence::should_walk(TABLES[$index], context.hour, context.force_full) {
                            reason
                        } else {
                            replay_data::cadence::SKIPPED_REASON
                        },
                    };
                )+};
            }
            fail_aux!(
                1 => subjects,
                2 => autonomy,
                3 => agencies,
                4 => operatives,
                5 => cryptology,
                9 => operations,
                10 => assignments,
                11 => operation_tokens,
                12 => incoming_actions,
                16 => captured_operatives,
                17 => agency_upgrades,
                18 => operative_missions,
                20 => agency_operation_buffers,
            );
            return [0; N];
        };
        self.subjects = aux.subjects;
        self.subject_names = aux.subject_names;
        self.subject_snapshot = aux.subject_snapshot;
        self.autonomy = aux.autonomy;
        self.autonomy_names = aux.autonomy_names;
        self.agencies = aux.agencies;
        self.agency_names = aux.agency_names;
        self.agency_upgrades = aux.agency_upgrades;
        self.agency_upgrade_catalog = aux.agency_upgrade_catalog;
        self.agency_upgrade_required = aux.agency_upgrade_required;
        self.operatives = aux.operatives;
        self.operative_names = aux.operative_names;
        self.operative_failure = aux.operative_failure;
        self.operative_refused_rows = aux.operative_refused_rows;
        self.operative_first_refused = aux.operative_first_refused;
        self.captured_operatives = aux.captured_operatives;
        self.operative_missions = aux.operative_missions;
        self.operative_mission_bytes = aux.operative_mission_bytes;
        self.operative_mission_failure = aux.operative_mission_failure;
        self.operative_effects = aux.operative_effects;
        self.cryptology = aux.cryptology;
        self.agency_operation_buffers = aux.agency_operation_buffers;
        self.agency_operation_entries = aux.agency_operation_entries;
        self.operations = aux.operations;
        self.assignments = aux.assignments;
        self.operation_descriptions = aux.operation_descriptions;
        self.operation_parts = aux.operation_parts;
        self.operation_tokens = aux.operation_tokens;
        self.tokens = aux.tokens;
        self.incoming_actions = aux.incoming_actions;
        aux.ticks
    }

    pub fn capture_delegated(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
    ) -> CaptureReport {
        self.capture_tables(memory, context, false, true, None, None)
    }

    pub fn fail_delegated_capture(&mut self, context: CaptureContext, reason: &'static str) {
        if !self.capitulation_raw_mode {
            self.fail_capitulation_capture(
                if replay_data::cadence::should_walk(TABLES[0], context.hour, context.force_full) {
                    reason
                } else {
                    replay_data::cadence::SKIPPED_REASON
                },
            );
        }
        if !self.relations_raw_mode {
            self.fail_relations_capture(
                [13, 14].map(|index| {
                    replay_data::cadence::should_walk(
                        TABLES[index],
                        context.hour,
                        context.force_full,
                    )
                }),
                reason,
            );
        }
    }

    pub fn fail_residual_capture(
        &mut self,
        context: CaptureContext,
        reason: &'static str,
    ) -> CaptureReport {
        let intel_due =
            replay_data::cadence::should_walk(TABLES[15], context.hour, context.force_full)
                || self.country_intel_sample.is_some();
        self.reset_residual();
        macro_rules! fail {
            ($($index:literal => $field:ident),+ $(,)?) => {$(
                self.$field.outcome = Outcome::Unavailable {
                    reason: if replay_data::cadence::should_walk(TABLES[$index], context.hour, context.force_full) {
                        reason
                    } else {
                        replay_data::cadence::SKIPPED_REASON
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
            16 => captured_operatives,
            17 => agency_upgrades,
            18 => operative_missions,
            19 => strategic_operatives,
            20 => agency_operation_buffers,
            21 => network_summaries,
            22 => propaganda,
            23 => source_contributions,
            24 => source_values,
            25 => network_auxiliary,
        );
        self.country_intel_outcome = Outcome::Unavailable {
            reason: if intel_due {
                reason
            } else {
                replay_data::cadence::SKIPPED_REASON
            },
        };
        self.country_intel_sample = intel_due.then_some(Err(reason));
        CaptureReport::default()
    }

    fn capture_tables(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        include_residual: bool,
        include_delegated: bool,
        flags: Option<&mut FlagStore>,
        variables: Option<&mut VariableStore>,
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
        let mut active =
            TABLES.map(|table| replay_data::cadence::should_walk(table, hour, force_full));
        let resets = crate::country_input_changes::intel_network_writes();
        if include_residual
            && (resets != Some(NETWORK_RESETS.load(std::sync::atomic::Ordering::SeqCst))
                || NETWORK_CAPTURE_EPOCH.load(std::sync::atomic::Ordering::SeqCst)
                    != NETWORK_CACHE_EPOCH.load(std::sync::atomic::Ordering::SeqCst))
        {
            active[6] = true;
        }
        active[21] = active[6];
        if self.relations_raw_mode {
            active[13] = self.relation_active[0];
            active[14] = self.relation_active[1];
        } else {
            self.relation_active = [active[13], active[14]];
        }
        if include_residual {
            for (active, &left_out) in active.iter_mut().zip(&self.left_out) {
                *active &= !left_out;
            }
        }
        macro_rules! skip {
            ($($field:ident = $index:literal),+ $(,)?) => {$(
                if !active[$index] {
                    self.$field.outcome = Outcome::Unavailable {
                        reason: replay_data::cadence::SKIPPED_REASON,
                    };
                }
            )+};
        }
        if include_residual {
            if !self.residual_aux_apart {
                skip!(
                    subjects = 1,
                    autonomy = 2,
                    agencies = 3,
                    operatives = 4,
                    cryptology = 5,
                    operations = 9,
                    assignments = 10,
                    operation_tokens = 11,
                    incoming_actions = 12,
                    captured_operatives = 16,
                    agency_upgrades = 17,
                    operative_missions = 18,
                    agency_operation_buffers = 20,
                );
            }
            skip!(
                networks = 6,
                flags = 7,
                variables = 8,
                strategic_operatives = 19,
                network_summaries = 21,
                propaganda = 22,
                source_contributions = 23,
                source_values = 24,
                network_auxiliary = 25,
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
        if include_delegated && !self.capitulation_raw_mode && active[0] {
            let _reads = read_accounting::table(TABLES[0]);
            let begin = qpc();
            let recipients = &mut self.war_goal_recipients;
            capture_rows(&mut self.capitulation, |rows| {
                cdiplomacy_status::walk_capitulation(memory, base, state, rows, recipients)
            });
            ticks[0] = elapsed(begin);
        }
        if include_residual {
            if !self.residual_aux_apart {
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
                if !(self.left_out[3] && self.left_out[17]) {
                    let begin = qpc();
                    let result = read_accounting::with_table(TABLES[3], || {
                        cintelligence_agency::walk_with_upgrades(
                            memory,
                            base,
                            state,
                            &mut self.agencies.rows,
                            &mut self.agency_names.first,
                            &mut self.agency_upgrades.rows,
                            &mut self.agency_upgrade_catalog,
                        )
                    });
                    first_named(
                        &mut self.agencies,
                        &mut self.agency_names,
                        ArenaRead::from(result.agencies),
                    );
                    self.agency_upgrades.outcome =
                        bounded(result.upgrades.outcome, self.agency_upgrades.rows.len());
                    self.agency_upgrade_required = result.upgrades.required_rows;
                    AGENCY_UPGRADE_PEAK_DEMAND.fetch_max(
                        result.upgrades.required_rows,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    AGENCY_UPGRADE_STORAGE_OMISSIONS.fetch_add(
                        u64::from(result.upgrades.omitted_rows),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    for (counter, count) in AGENCY_UPGRADE_REFUSALS
                        .iter()
                        .zip(result.upgrades.refusal_counts)
                    {
                        counter.fetch_add(u64::from(count), std::sync::atomic::Ordering::Relaxed);
                    }
                    ticks[3] = elapsed(begin);
                }
                if active[4] {
                    let begin = qpc();
                    capture_named(
                        &mut self.operatives,
                        &mut self.operative_names,
                        |rows, names| {
                            let _reads = read_accounting::table(TABLES[4]);
                            let read =
                                coperative_leader::walk(memory, base, state, rows, names);
                            self.operative_failure = read.failed_membership;
                            self.operative_refused_rows = read.refused_rows;
                            self.operative_first_refused = read.first_refused;
                            OPERATIVE_REFUSALS.observe(read.refused_rows, read.first_refused);
                            ArenaRead {
                                outcome: read.outcome,
                                len: read.bytes_len,
                                required: read.required_bytes,
                            }
                        },
                    );
                    ticks[4] = elapsed(begin);
                }
                plain!(
                    16,
                    captured_operatives,
                    cintelligence_agency::walk_captured_operatives
                );
                if active[18] {
                    let begin = qpc();
                    capture_named(
                        &mut self.operative_missions,
                        &mut self.operative_mission_bytes,
                        |rows, bytes| {
                            let _reads = read_accounting::table(TABLES[18]);
                            let read = with_held_key(
                                &ROOT_OUT_KEY,
                                operative_missions::root_out_key(),
                                |key| {
                                    operative_missions::walk(
                                        memory, base, state, rows, bytes, key,
                                    )
                                },
                            );
                            self.operative_mission_failure = read.failed_operative;
                            ArenaRead {
                                outcome: read.outcome,
                                len: read.bytes_len,
                                required: read.required_bytes,
                            }
                        },
                    );
                    {
                        let _reads = read_accounting::table(TABLES[18]);
                        crate::operative_effects::snapshot(
                            memory,
                            base,
                            state,
                            &self.operative_missions.rows[..self.operative_missions.outcome.len()],
                            &mut self.operative_effects,
                        );
                    }
                    ticks[18] = elapsed(begin);
                }
                plain!(5, cryptology, ccryptology::walk);
            }
            if active[6] {
                let _reads = read_accounting::table(TABLES[6]);
                let begin = qpc();
                let membership_epoch =
                    NETWORK_CACHE_EPOCH.load(std::sync::atomic::Ordering::SeqCst);
                let scheduled = replay_data::cadence::should_walk(TABLES[21], hour, force_full);
                if scheduled || resets.is_some() {
                    let mut cache = NETWORK_SUMMARY_CACHE
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let epoch = NETWORK_CACHE_EPOCH.load(std::sync::atomic::Ordering::SeqCst);
                    let lifetimes = crate::country_input_changes::intel_network_resets();
                    let lifetime_changed = cache.epoch != epoch || lifetimes != Some(cache.resets);
                    let refresh = scheduled || lifetime_changed;
                    let read = ccountry_intel_network::walk_with_cached_summaries(
                        memory,
                        base,
                        state,
                        &mut self.networks.rows,
                        &mut self.network_summaries.rows,
                        &mut cache.values,
                        refresh,
                    );
                    if matches!(read.summaries, Outcome::Rows { .. }) {
                        cache.epoch = epoch;
                        if let Some(lifetimes) = lifetimes {
                            cache.resets = lifetimes;
                        }
                    }
                    self.networks.outcome = bounded(read.states, self.networks.rows.len());
                    self.network_summaries.outcome =
                        bounded(read.summaries, self.network_summaries.rows.len());
                    if active[25] {
                        if let Outcome::Rows { len } = self.network_summaries.outcome {
                            if crate::network_auxiliary::membership(
                                &self.network_summaries.rows[..len],
                                &self.networks.rows[..match self.networks.outcome {
                                    Outcome::Rows { len } => len,
                                    _ => 0,
                                }],
                                lifetime_changed,
                            )
                            .is_err()
                            {
                                crate::network_auxiliary::refused();
                            }
                        } else {
                            crate::network_auxiliary::refused();
                        }
                    }
                    if let Outcome::Rows { len } = self.network_summaries.outcome {
                        if crate::source_changes::network_membership(
                            &self.network_summaries.rows[..len],
                            lifetime_changed,
                        )
                        .is_err()
                        {
                            crate::source_changes::network_refused();
                        }
                    } else {
                        crate::source_changes::network_refused();
                    }
                } else {
                    self.networks.outcome = ccountry_intel_network::walk(
                        memory,
                        base,
                        state,
                        &mut self.networks.rows,
                    );
                    self.network_summaries.outcome = Outcome::Unavailable {
                        reason: "intel network summary writer certificate unavailable",
                    };
                    crate::source_changes::network_refused();
                    NETWORK_SUMMARY_CACHE
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .values
                        .invalidate();
                }
                ticks[6] = elapsed(begin);
                NETWORK_CAPTURE_EPOCH.store(membership_epoch, std::sync::atomic::Ordering::SeqCst);
            }
            if let (Some(resets), Outcome::Rows { .. }) = (resets, self.networks.outcome) {
                NETWORK_RESETS.store(resets, std::sync::atomic::Ordering::SeqCst);
            }
            if !self.residual_aux_apart && active[20] {
                let begin = qpc();
                capture_named(
                    &mut self.agency_operation_buffers,
                    &mut self.agency_operation_entries,
                    |rows, entries| {
                        let _reads = read_accounting::table(TABLES[20]);
                        let read = with_held_key(
                            &OFFER_SPEED_KEY,
                            agency_operation_buffers::speed_key(),
                            |key| {
                                agency_operation_buffers::walk(
                                    memory, base, state, rows, entries, key,
                                )
                            },
                        );
                        ArenaRead {
                            outcome: read.outcome,
                            len: read.entries_len,
                            required: read.required_entries,
                        }
                    },
                );
                ticks[20] = elapsed(begin);
            }
            if active[19] {
                let begin = qpc();
                let mut objects = None;
                capture_named(
                    &mut self.strategic_operatives,
                    &mut self.strategic_operative_entries,
                    |rows, entries| {
                        let _reads = read_accounting::table(TABLES[19]);
                        let read =
                            strategic_operatives::walk(memory, base, state, rows, entries);
                        objects = read.objects;
                        ArenaRead {
                            outcome: read.outcome,
                            len: read.entries_len,
                            required: read.required_entries,
                        }
                    },
                );
                ticks[19] = elapsed(begin);
                let begin = qpc();
                let _reads = read_accounting::table(TABLES[22]);
                self.propaganda.outcome = match self.strategic_operatives.outcome {
                    Outcome::Rows { len } => crate::propaganda_changes::capture(
                        memory,
                        base,
                        objects,
                        &self.strategic_operatives.rows[..len],
                        force_full,
                        replay_data::cadence::should_walk(TABLES[6], hour, false),
                        &mut self.propaganda.rows,
                        &mut self.certified_snapshots[0],
                    ),
                    _ => Outcome::Unavailable {
                        reason: "propaganda strategic census incomplete",
                    },
                };
                ticks[22] = elapsed(begin);
            }
            if active[7] {
                let begin = qpc();
                let mut flags = flags;
                capture_rows(&mut self.flags, |rows| {
                    let _reads = read_accounting::table(TABLES[7]);
                    let reuse = flags
                        .as_deref_mut()
                        .map(|store| FlagReuse::begin(store, &crate::flag_changes::Observer));
                    walk_flags(memory, base, state, rows, &mut self.flag_scratch, reuse)
                });
                self.flags_witness = flags
                    .as_deref()
                    .and_then(FlagStore::witnessed)
                    .filter(|_| !force_full && matches!(self.flags.outcome, Outcome::Rows { .. }));
                ticks[7] = elapsed(begin);
            }
            if active[8] && !self.variables_apart {
                let begin = qpc();
                walk_variables(
                    memory,
                    base,
                    state,
                    &mut self.variables,
                    &mut self.variable_entries,
                    &mut self.variable_names,
                    &mut self.variable_delta,
                    variables,
                );
                ticks[8] = elapsed(begin);
            }
            if !self.residual_aux_apart {
                if active[9] {
                    let begin = qpc();
                    let result = read_accounting::with_table(TABLES[9], || {
                        ccountry_operation_manager::walk(
                            memory,
                            base,
                            state,
                            &mut self.operations.rows,
                            &mut self.assignments.rows,
                            &mut self.operation_descriptions.first,
                            &mut self.operation_parts.first,
                        )
                    });
                    first_named(
                        &mut self.operations,
                        &mut self.operation_descriptions,
                        ArenaRead {
                            outcome: result.outcome,
                            len: result.bytes_len,
                            required: result.required_bytes,
                        },
                    );
                    self.operation_parts.len = result.parts_len;
                    self.operation_parts.required = result.required_parts;
                    if !valid_arena(
                        self.operations.outcome,
                        &self.operations.rows,
                        &self.operation_parts.first,
                        result.parts_len,
                        result.required_parts,
                    ) {
                        self.operations.outcome = Outcome::Unavailable {
                            reason: "captured arena range invalid",
                        };
                    }
                    self.assignments.outcome =
                        bounded(result.assignments, self.assignments.rows.len());
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
                plain!(
                    12,
                    incoming_actions,
                    incoming_diplomatic_action_queue::walk
                );
            }
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
            self.country_intel_outcome = match self.country_intel.borrow(active[15]) {
                Walk::Skip | Walk::Busy => Outcome::Unavailable {
                    reason: replay_data::cadence::SKIPPED_REASON,
                },
                Walk::Ready(buffer) => {
                    self.country_intel_sample = Some(Err("country intel acquisition incomplete"));
                    let begin = qpc();
                    self.source_seeds.clear();
                    let mut source_error = false;
                    let result = {
                        let _reads = read_accounting::table(TABLES[15]);
                        if grow_vec(
                            &mut buffer.scratch,
                            engine_native_intelligence::country_intel_row::STRIDE,
                        ) {
                            ccountry_intel::walk_with_reserved_visit(
                                memory,
                                base,
                                state,
                                engine_native_memory::walk::Buffer::Growing(&mut buffer.rows.rows),
                                &mut buffer.scratch,
                                &mut |owner, root, country| {
                                    if !source_error {
                                        if engine_allocation::reserve(&mut self.source_seeds, 1)
                                            .is_ok()
                                        {
                                            self.source_seeds.push(crate::source_state::Seed {
                                                owner,
                                                root,
                                                country,
                                            });
                                        } else {
                                            source_error = true;
                                        }
                                    }
                                },
                            )
                        } else {
                            ccountry_intel::ReservedCapture {
                                outcome: Outcome::Unavailable {
                                    reason: "country intel scratch allocation failed",
                                },
                                vector_count: Err("country intel scratch allocation failed"),
                                reserved_required: 0,
                                reserved_reached: false,
                            }
                        }
                    };
                    self.source_census_complete = !source_error
                        && matches!(result.outcome, Outcome::Rows { .. })
                        && result.vector_count.is_ok();
                    self.source_seeds.sort_unstable_by_key(|seed| seed.root);
                    self.source_seeds.dedup();
                    buffer.rows.outcome = bounded(result.outcome, buffer.rows.rows.len());
                    buffer.reserved_required =
                        buffer.reserved_required.max(result.reserved_required);
                    self.country_intel_sample = Some(result.vector_count.map(|vector_count| {
                        rows::ReservedCountryIntelSample {
                            sample_hour: hour,
                            vector_count,
                        }
                    }));
                    ticks[15] = elapsed(begin);
                    buffer.rows.outcome
                }
            };
        }
        if include_residual && (active[CONTRIBUTIONS_SLOT] || active[SOURCES_SLOT]) {
            let begin = qpc();
            let _reads = read_accounting::table(TABLES[CONTRIBUTIONS_SLOT]);
            let seeds = self
                .source_census_complete
                .then_some(self.source_seeds.as_slice());
            let (contributions, sources) = crate::source_changes::capture(
                memory,
                base,
                seeds,
                context.force_full,
                &mut self.source_contributions.rows,
                &mut self.source_values.rows,
                &mut self.certified_snapshots[1..3],
            );
            self.source_contributions.outcome = contributions;
            self.source_values.outcome = sources;
            ticks[CONTRIBUTIONS_SLOT] = elapsed(begin);
        }
        if include_residual && active[25] {
            let begin = qpc();
            let _reads = read_accounting::table(TABLES[25]);
            self.network_auxiliary.outcome = crate::network_auxiliary::capture(
                memory,
                base,
                &mut self.network_auxiliary.rows,
                &mut self.certified_snapshots[3],
            );
            ticks[25] = elapsed(begin);
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
            self.agency_upgrades.outcome,
            self.operative_missions.outcome,
            self.strategic_operatives.outcome,
            self.agency_operation_buffers.outcome,
            self.network_summaries.outcome,
            self.propaganda.outcome,
            self.source_contributions.outcome,
            self.source_values.outcome,
            self.network_auxiliary.outcome,
        ]
    }

    pub fn country_intel_sample(
        &self,
    ) -> Option<Result<rows::ReservedCountryIntelSample, &'static str>> {
        self.country_intel_sample
    }

    pub fn table_inputs(
        &self,
        baseline: Option<(BaselineToken, &[rows::ScriptVariablesRow])>,
        scratch: &mut ScriptVariableComparison,
        reuse: Option<&mut IntelConversion<'_>>,
    ) -> [TableInput; N] {
        self.table_inputs_projected(baseline, scratch, reuse, None)
    }

    pub fn table_inputs_projected(
        &self,
        baseline: Option<(BaselineToken, &[rows::ScriptVariablesRow])>,
        scratch: &mut ScriptVariableComparison,
        reuse: Option<&mut IntelConversion<'_>>,
        mut projection: Option<&mut Projection<'_>>,
    ) -> [TableInput; N] {
        use replay_data::rows::RowCodec;
        let outcomes = self.outcomes();
        let flags = timed(TableId::ScriptFlags, || {
            if let Some(reuse) = reuse {
                reuse.shadow.flags.input(
                    TableId::ScriptFlags,
                    &self.flags,
                    outcomes[7],
                    (reuse.hour, self.flags_witness),
                    || self.flags_table(outcomes[7]),
                )
            } else {
                TableInput::Sampled(self.flags_table(outcomes[7]))
            }
        });
        let cryptology_row = |r: &ccryptology::Row| rows::CryptologyRow {
            owner_country: r.owner_country,
            target_country: r.target_country,
            amount_raw: r.amount_raw,
            bonus_days: r.bonus_days,
            bonus_active: r.bonus_active,
            decrypting: r.decrypting,
            ordinal: rows::Recorded::Value(r.ordinal),
        };
        let network_row = |r: &ccountry_intel_network::Row| rows::IntelNetworkRow {
            owner_country: r.owner_country,
            target_country: r.target_country,
            state_id: r.state_id,
            strength_raw: r.strength_raw,
            subnetwork_selector: r.subnetwork_selector,
            local_strength_raw: r.local_strength_raw,
            gain_raw: r.gain_raw,
            gain_from_operatives_raw: r.gain_from_operatives_raw,
            gain_from_adjacency_raw: r.gain_from_adjacency_raw,
            local_quiet: r.local_quiet,
            gain_factor_raw: r.gain_factor_raw,
            strength_target_raw: r.strength_target_raw,
            target_from_operatives_raw: r.target_from_operatives_raw,
            target_from_counterintelligence_raw: r.target_from_counterintelligence_raw,
            subnetwork_quiet: r.subnetwork_quiet,
        };
        let assignment_row = |r: &operation_assigned_operative::Row| rows::OperationAssignmentRow {
            operation: r.operation,
            ordinal: r.ordinal,
            operative: r.operative,
            selection: r.selection,
        };
        let action_row =
            |r: &incoming_diplomatic_action_queue::Row| rows::IncomingDiplomaticActionRow {
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
            };
        let opinion_row = |r: &crelation_status::OpinionRow| rows::RelationOpinionRow {
            owner_country: r.owner_country,
            target_country: r.target_country,
            opinion: r.opinion,
        };
        let country_intel_row = |r: &engine_native::country_intel_row::Row| rows::CountryIntelRow {
            owner_country: r.owner_country,
            target_index: r.target_index,
            army: r.army,
            air: r.air,
            civilian: r.civilian,
            navy: r.navy,
        };
        let captured_row =
            |r: &cintelligence_agency::CapturedOperativeRow| rows::CapturedOperativeReferenceRow {
                country: r.country,
                ordinal: r.ordinal,
                operative: r.operative,
                owner: r.owner,
            };
        let summary_row = |r: &ccountry_intel_network::Summary| rows::IntelNetworkSummaryRow {
            owner_country: r.owner_country,
            target_country: r.target_country,
            covered_core_states: r.covered_core_states,
            ordinal: r.ordinal,
            covered_controlled_states: r.covered_controlled_states,
            covered_worth: r.covered_worth,
            controlled_strength_sum: r.controlled_strength_sum,
            core_strength_sum: r.core_strength_sum,
            max_state_strength: r.max_state_strength,
        };
        let propaganda_row = |r: &crate::propaganda_changes::Row| rows::PropagandaEffortRow {
            slot: r.slot,
            country: r.country,
            sequence: r.sequence,
            stability: r.stability,
            war_support: r.war_support,
            derived_drift: r.derived_drift,
        };
        let certified_or = |slot: usize, fallback: &mut dyn FnMut() -> TableInput| match &self
            .certified_snapshots[slot - 22]
        {
            Some(snapshot) => snapshot.input(),
            None => fallback(),
        };
        [
            TableInput::Sampled(self.capitulation_table(outcomes[0])),
            TableInput::Sampled(self.subjects_table(outcomes[1])),
            TableInput::Sampled(self.autonomy_table(outcomes[2])),
            TableInput::Sampled(self.agencies_table(outcomes[3])),
            TableInput::Sampled(self.operatives_table(outcomes[4])),
            scalar_table::<tables::Cryptology, _>(
                projection.as_deref_mut(),
                outcomes[5],
                &self.cryptology.rows,
                |r| RowCodec::key(&cryptology_row(r)),
                cryptology_row,
            ),
            scalar_table::<tables::IntelNetworks, _>(
                projection.as_deref_mut(),
                outcomes[6],
                &self.networks.rows,
                |r| RowCodec::key(&network_row(r)),
                network_row,
            ),
            flags,
            timed(TableId::ScriptVariables, || {
                self.variables_input(outcomes[8], baseline, scratch)
            }),
            TableInput::Sampled(self.operations_table(outcomes[9])),
            scalar_table::<tables::OperationAssignments, _>(
                projection.as_deref_mut(),
                outcomes[10],
                &self.assignments.rows,
                |r| RowCodec::key(&assignment_row(r)),
                assignment_row,
            ),
            TableInput::Sampled(self.operation_tokens_table(outcomes[11])),
            scalar_table::<tables::IncomingDiplomaticActions, _>(
                projection.as_deref_mut(),
                outcomes[12],
                &self.incoming_actions.rows,
                |r| RowCodec::key(&action_row(r)),
                action_row,
            ),
            scalar_table::<tables::RelationOpinions, _>(
                projection.as_deref_mut(),
                outcomes[13],
                &self.relation_opinions.rows,
                |r| RowCodec::key(&opinion_row(r)),
                opinion_row,
            ),
            TableInput::Sampled(self.relation_tuples_table(outcomes[14])),
            scalar_table::<tables::CountryIntel, _>(
                projection.as_deref_mut(),
                outcomes[15],
                self.country_intel.rows().unwrap_or_default(),
                |r| RowCodec::key(&country_intel_row(r)),
                country_intel_row,
            ),
            scalar_table::<tables::CapturedOperativeReferences, _>(
                projection.as_deref_mut(),
                outcomes[16],
                &self.captured_operatives.rows,
                |r| RowCodec::key(&captured_row(r)),
                captured_row,
            ),
            TableInput::Sampled(self.agency_upgrades_table(outcomes[17])),
            TableInput::Sampled(self.operative_missions_table(outcomes[18])),
            TableInput::Sampled(self.strategic_operatives_table(outcomes[19])),
            TableInput::Sampled(self.agency_operation_buffers_table(outcomes[20])),
            scalar_table::<tables::IntelNetworkSummaries, _>(
                projection.as_deref_mut(),
                outcomes[21],
                &self.network_summaries.rows,
                |r| RowCodec::key(&summary_row(r)),
                summary_row,
            ),
            certified_or(22, &mut || {
                scalar_table::<tables::PropagandaEfforts, _>(
                    projection.as_deref_mut(),
                    outcomes[22],
                    &self.propaganda.rows,
                    |r| RowCodec::key(&propaganda_row(r)),
                    propaganda_row,
                )
            }),
            certified_or(23, &mut || {
                scalar_table::<tables::IntelContributions, _>(
                    projection.as_deref_mut(),
                    outcomes[23],
                    &self.source_contributions.rows,
                    |row| RowCodec::key(row),
                    |row| *row,
                )
            }),
            certified_or(24, &mut || {
                scalar_table::<tables::IntelSources, _>(
                    projection.as_deref_mut(),
                    outcomes[24],
                    &self.source_values.rows,
                    |row| RowCodec::key(row),
                    |row| *row,
                )
            }),
            certified_or(25, &mut || {
                scalar_table::<tables::IntelNetworkAuxiliaries, _>(
                    projection.as_deref_mut(),
                    outcomes[25],
                    &self.network_auxiliary.rows,
                    |row| RowCodec::key(row),
                    |row| *row,
                )
            }),
        ]
    }

    #[cfg(test)]
    fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        self.convert_tables(
            std::convert::identity,
            |rows| TableView::Complete(rows.try_clone().expect("synthetic snapshot clone")),
            self.flags_table(outcomes[7]),
            self.country_intel_table(outcomes[15]),
            |outcome| self.variables_table(outcome),
        )
    }

    #[cfg(test)]
    fn certified_table<V>(
        &self,
        slot: usize,
        sample: impl FnOnce(TableView) -> V,
        shared: impl FnOnce(Arc<TableRows>) -> V,
        project: impl FnOnce() -> TableView,
    ) -> V {
        match &self.certified_snapshots[slot - 22] {
            Some(snapshot) => match snapshot.input() {
                TableInput::SharedComplete(rows) => shared(rows),
                TableInput::Sampled(view) => sample(view),
                _ => unreachable!("certified snapshot only projects complete rows or refusal"),
            },
            None => sample(project()),
        }
    }

    #[cfg(test)]
    fn convert_tables<V>(
        &self,
        sample: impl Fn(TableView) -> V,
        shared: impl Fn(Arc<TableRows>) -> V,
        flags: V,
        country_intel: V,
        variables: impl FnOnce(Outcome) -> V,
    ) -> [V; N] {
        let outcomes = self.outcomes();
        [
            sample(self.capitulation_table(outcomes[0])),
            sample(self.subjects_table(outcomes[1])),
            sample(self.autonomy_table(outcomes[2])),
            sample(self.agencies_table(outcomes[3])),
            sample(self.operatives_table(outcomes[4])),
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
                    ordinal: rows::Recorded::Value(r.ordinal),
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
                    gain_raw: r.gain_raw,
                    gain_from_operatives_raw: r.gain_from_operatives_raw,
                    gain_from_adjacency_raw: r.gain_from_adjacency_raw,
                    local_quiet: r.local_quiet,
                    gain_factor_raw: r.gain_factor_raw,
                    strength_target_raw: r.strength_target_raw,
                    target_from_operatives_raw: r.target_from_operatives_raw,
                    target_from_counterintelligence_raw: r.target_from_counterintelligence_raw,
                    subnetwork_quiet: r.subnetwork_quiet,
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
            sample(self.agency_upgrades_table(outcomes[17])),
            sample(self.operative_missions_table(outcomes[18])),
            sample(self.strategic_operatives_table(outcomes[19])),
            sample(self.agency_operation_buffers_table(outcomes[20])),
            sample(table::<tables::IntelNetworkSummaries, _>(
                outcomes[21],
                &self.network_summaries.rows,
                |r| rows::IntelNetworkSummaryRow {
                    owner_country: r.owner_country,
                    target_country: r.target_country,
                    covered_core_states: r.covered_core_states,
                    ordinal: r.ordinal,
                    covered_controlled_states: r.covered_controlled_states,
                    covered_worth: r.covered_worth,
                    controlled_strength_sum: r.controlled_strength_sum,
                    core_strength_sum: r.core_strength_sum,
                    max_state_strength: r.max_state_strength,
                },
            )),
            self.certified_table(22, &sample, &shared, || {
                table::<tables::PropagandaEfforts, _>(outcomes[22], &self.propaganda.rows, |r| {
                    rows::PropagandaEffortRow {
                        slot: r.slot,
                        country: r.country,
                        sequence: r.sequence,
                        stability: r.stability,
                        war_support: r.war_support,
                        derived_drift: r.derived_drift,
                    }
                })
            }),
            self.certified_table(23, &sample, &shared, || {
                table::<tables::IntelContributions, _>(
                    outcomes[23],
                    &self.source_contributions.rows,
                    |row| *row,
                )
            }),
            self.certified_table(24, &sample, &shared, || {
                table::<tables::IntelSources, _>(outcomes[24], &self.source_values.rows, |row| *row)
            }),
            self.certified_table(25, &sample, &shared, || {
                table::<tables::IntelNetworkAuxiliaries, _>(
                    outcomes[25],
                    &self.network_auxiliary.rows,
                    |row| *row,
                )
            }),
        ]
    }
}

impl Captured {
    #[cfg(test)]
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

    fn operatives_table(&self, outcome: Outcome) -> TableView {
        if let Outcome::Unavailable { reason } = outcome
            && let Some((id, roster, kind)) = self.operative_failure
        {
            let mut detail = String::new();
            if crate::roster::add_reason(
                &mut detail,
                format_args!(
                    "{reason}; operative={id} roster={roster} kind={kind} refused_reads=1"
                ),
            )
            .is_err()
            {
                return unavailable("operative refusal description allocation failed");
            }
            return TableView::Unavailable { reason: detail };
        }
        if !valid_arena(
            outcome,
            &self.operatives.rows,
            &self.operative_names.first,
            self.operative_names.len,
            self.operative_names.required,
        ) {
            return unavailable("operative name lies outside captured arena");
        }
        table::<tables::OperativeMemberships, _>(outcome, &self.operatives.rows, |r| {
            rows::OperativeMembershipRow {
                id: r.id,
                owner_country: r.owner_country,
                roster_country: r.roster_country,
                roster_kind: r.roster_kind,
                legacy_description_length: 0,
                assignment: r.assignment,
                experience_raw: r.experience_raw,
                skill: r.skill,
                capturer_country: r.capturer_country,
                captured_hour: r.captured_hour,
                action_variant: r.action_variant,
                ordinal: rows::Recorded::Value(r.ordinal),
                target_country: rows::Recorded::Value(r.target_country),
                operation: rows::Recorded::Value(r.operation),
                name: rows::Recorded::Value(owned_vec(copy_vec(literal(
                    &self.operative_names.first,
                    r.name,
                )))),
                cooldown_kind: rows::Recorded::Value(r.cooldown_kind),
                cooldown_start_hour: rows::Recorded::Value(r.cooldown_start_hour),
                cooldown_end_hour: rows::Recorded::Value(r.cooldown_end_hour),
            }
        })
    }

    fn operative_missions_table(&self, outcome: Outcome) -> TableView {
        if let Outcome::Unavailable { reason } = outcome
            && let Some((id, roster)) = self.operative_mission_failure
        {
            let mut detail = String::new();
            if crate::roster::add_reason(
                &mut detail,
                format_args!("{reason}; operative={id} roster={roster} refused_reads=1"),
            )
            .is_err()
            {
                return unavailable("operative mission refusal description allocation failed");
            }
            return TableView::Unavailable { reason: detail };
        }
        if !valid_arena(
            outcome,
            &self.operative_missions.rows,
            &self.operative_mission_bytes.first,
            self.operative_mission_bytes.len,
            self.operative_mission_bytes.required,
        ) {
            return unavailable("operative mission bytes lie outside captured arena");
        }
        let bytes = &self.operative_mission_bytes.first;
        table_iter::<tables::OperativeMissions, _>(
            outcome,
            self.operative_missions
                .rows
                .iter()
                .take(outcome.len())
                .enumerate(),
            |(index, r)| rows::OperativeMissionRow {
                active_effect: rows::Recorded::Value(
                    self.operative_effects
                        .get(index)
                        .and_then(|effects| effects[0].as_ref())
                        .cloned(),
                ),
                pending_effect: rows::Recorded::Value(
                    self.operative_effects
                        .get(index)
                        .and_then(|effects| effects[1].as_ref())
                        .cloned(),
                ),
                map_selected: rows::Recorded::Value(r.map_selected),
                map_warning: rows::Recorded::Value(r.map_warning),
                roster_country: r.roster_country,
                id: r.id,
                ordinal: r.ordinal,
                owner_country: r.owner_country,
                assignment: r.assignment,
                cooldown_kind: r.cooldown_kind,
                cooldown_start_hour: r.cooldown_start_hour,
                cooldown_end_hour: r.cooldown_end_hour,
                variant: r.variant,
                target_country: r.target_country,
                target_state: r.target_state,
                ideology: owned_vec(copy_vec(literal(bytes, r.ideology))),
                pending_variant: r.pending_variant,
                pending_country: r.pending_country,
                pending_state: r.pending_state,
                pending_ideology: owned_vec(copy_vec(literal(bytes, r.pending_ideology))),
                portrait: owned_vec(copy_vec(literal(bytes, r.portrait))),
                root_out_country_factor: r.root_out_country_factor,
                root_out_operative_factor: r.root_out_operative_factor,
                root_out_nationality: r.root_out_nationality,
            },
        )
    }

    fn agency_operation_buffers_table(&self, outcome: Outcome) -> TableView {
        if !valid_arena(
            outcome,
            &self.agency_operation_buffers.rows,
            &self.agency_operation_entries.first,
            self.agency_operation_entries.len,
            self.agency_operation_entries.required,
        ) {
            return unavailable("agency operation buffer entries lie outside captured arena");
        }
        let entries = &self.agency_operation_entries.first;
        table::<tables::AgencyOperationBuffers, _>(
            outcome,
            &self.agency_operation_buffers.rows,
            |r| {
                let mut start = r.entries_start;
                let mut list = |count: usize| {
                    let part = &entries[start..start + count];
                    start += count;
                    owned_vec(map_slice(part, |entry| rows::AgencyBufferedOperation {
                        operation: entry.operation,
                        state: entry.state,
                    }))
                };
                let completed = list(r.counts[0]);
                let running = list(r.counts[1]);
                let ready = list(r.counts[2]);
                let unready = list(r.counts[3]);
                let offers = owned_vec(map_slice(&entries[start..start + r.counts[4]], |entry| {
                    rows::AgencyBufferedOffer {
                        definition: entry.definition,
                        country: entry.country,
                        state: entry.state,
                        duration: entry.duration,
                    }
                }));
                rows::AgencyOperationBuffersRow {
                    snapshot: 0,
                    completed,
                    running,
                    ready,
                    unready,
                    offers: rows::Recorded::Value(offers),
                }
            },
        )
    }

    fn strategic_operatives_table(&self, outcome: Outcome) -> TableView {
        if !valid_arena(
            outcome,
            &self.strategic_operatives.rows,
            &self.strategic_operative_entries.first,
            self.strategic_operative_entries.len,
            self.strategic_operative_entries.required,
        ) {
            return unavailable("strategic operative entries lie outside captured arena");
        }
        let entries = &self.strategic_operative_entries.first;
        let mut guard = HELD_ROSTERS.lock().unwrap_or_else(|poisoned| {
            let mut held = poisoned.into_inner();
            *held = None;
            held
        });
        let held = std::cell::RefCell::new(guard.get_or_insert_with(Default::default));
        let view = table::<tables::StrategicOperatives, _>(
            outcome,
            &self.strategic_operatives.rows,
            |r| {
                let trade_end = r.entries_start + r.trade_len;
                rows::StrategicOperativeRow {
                    mission_rosters: rows::Recorded::Value(Some(held_rosters(
                        &mut held.borrow_mut(),
                        r.country,
                        &entries[trade_end + r.pressure_len..],
                        r.mission_counts,
                    ))),
                    country: r.country,
                    slot: r.slot,
                    mission_activity: r.mission_activity,
                    operation_activity: r.operation_activity,
                    trade_influence: owned_vec(map_slice(
                        &entries[r.entries_start..trade_end],
                        |entry| rows::TradeInfluence {
                            owner: entry.owner,
                            value: entry.value,
                        },
                    )),
                    diplomatic_pressure: owned_vec(map_slice(
                        &entries[trade_end..trade_end + r.pressure_len],
                        |entry| rows::DiplomaticPressure {
                            owner: entry.owner,
                            acceptance: entry.value,
                            tension: entry.tension,
                        },
                    )),
                }
            },
        );
        let held = held.into_inner();
        if let Outcome::Rows { len } = outcome
            && held.len() > len
            && let Some(live) = self.strategic_operatives.rows.get(..len)
        {
            held.retain(|country, _| live.iter().any(|row| row.country == *country));
        }
        view
    }

    fn agency_upgrades_table(&self, outcome: Outcome) -> TableView {
        table::<tables::AgencyUpgradeLevels, _>(outcome, &self.agency_upgrades.rows, |r| {
            rows::AgencyUpgradeLevelRow {
                country: r.country,
                ordinal: r.ordinal,
                definition_pointer: r.definition_pointer,
                branch_pointer: r.branch_pointer,
                upgrade_token: r.upgrade_token,
                branch_token: r.branch_token,
                level: r.level,
                identity_status: r.identity_status,
            }
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

    #[cfg(test)]
    fn variables_equal(
        &self,
        outcome: Outcome,
        previous: &[rows::ScriptVariablesRow],
        scratch: &mut ScriptVariableComparison,
    ) -> bool {
        let Some(scopes) = self.variables.rows.get(..outcome.len()) else {
            return false;
        };
        let mut edits = Edits::new();
        matches!(outcome, Outcome::Rows { .. })
            && self
                .mapped_edits(
                    scopes,
                    None,
                    previous,
                    scratch,
                    &mut VariableWork::default(),
                    &mut edits,
                )
                .is_some()
            && edits.is_empty()
    }

    fn variables_input(
        &self,
        outcome: Outcome,
        baseline: Option<(BaselineToken, &[rows::ScriptVariablesRow])>,
        scratch: &mut ScriptVariableComparison,
    ) -> TableInput {
        #[cfg(feature = "conversion_check")]
        let reference = self.reference_variables(outcome, baseline, scratch);
        let input = self.changed_variables(outcome, baseline, scratch);
        #[cfg(feature = "conversion_check")]
        conversion_check::record(baseline, &reference, &input);
        input
    }

    fn changed_variables(
        &self,
        outcome: Outcome,
        baseline: Option<(BaselineToken, &[rows::ScriptVariablesRow])>,
        scratch: &mut ScriptVariableComparison,
    ) -> TableInput {
        let started = std::time::Instant::now();
        let mut work = VariableWork::default();
        let input = self.variable_edits(outcome, baseline, scratch, &mut work);
        let holds_capture = matches!(
            input,
            TableInput::ComparedEqual(_)
                | TableInput::SliceUpdate { .. }
                | TableInput::SharedComplete(_)
        );
        scratch.kept = match (baseline, self.variable_delta.generation) {
            (Some((token, _)), Some(generation)) if holds_capture => Some(Kept {
                at: token.destination(),
                generation,
            }),
            _ => None,
        };
        work.ns = elapsed_ns(started);
        scratch.work = Some(work);
        input
    }

    fn variable_edits(
        &self,
        outcome: Outcome,
        baseline: Option<(BaselineToken, &[rows::ScriptVariablesRow])>,
        scratch: &mut ScriptVariableComparison,
        work: &mut VariableWork,
    ) -> TableInput {
        let Some((token, previous)) = baseline else {
            return self.complete_variables(outcome, Complete::NoBaseline, work);
        };
        let Outcome::Rows { len } = outcome else {
            return self.complete_variables(outcome, Complete::NotComplete, work);
        };
        let (entries, names) = (&self.variable_entries, &self.variable_names);
        let Some(scopes) = self.variables.rows.get(..len).filter(|_| {
            entries.len <= entries.first.len()
                && names.len <= names.first.len()
                && entries.required >= entries.len
                && names.required >= names.len
        }) else {
            return self.complete_variables(outcome, Complete::Refused, work);
        };
        work.scopes = len as u64;
        let delta = &self.variable_delta;
        let changed = match scratch.kept {
            Some(kept) if kept.at == token.baseline() => {
                if delta.since == Some(kept.generation) {
                    Ok(&delta.changed[..])
                } else {
                    Err(AllScopes::NoCaptureDelta)
                }
            }
            _ => Err(AllScopes::NoKeptHour),
        };
        let mut edits = Edits::new();
        let decided = match changed {
            Ok(changed) if delta.same_scopes && len == previous.len() => {
                self.listed_edits(scopes, changed, previous, scratch, work, &mut edits)
            }
            Ok(changed) => {
                work.reordered = true;
                self.mapped_edits(scopes, Some(changed), previous, scratch, work, &mut edits)
            }
            Err(reason) => {
                work.path = Path::AllScopes(reason);
                self.mapped_edits(scopes, None, previous, scratch, work, &mut edits)
            }
        };
        let built = std::time::Instant::now();
        let input = decided.and_then(|()| keyed_update(token, edits));
        work.build_ns = work.build_ns.saturating_add(elapsed_ns(built));
        input.unwrap_or_else(|| self.complete_variables(outcome, Complete::Refused, work))
    }

    fn listed_edits(
        &self,
        scopes: &[cvariables::ScopeRow],
        changed: &[u32],
        previous: &[rows::ScriptVariablesRow],
        scratch: &mut ScriptVariableComparison,
        work: &mut VariableWork,
        edits: &mut Edits,
    ) -> Option<()> {
        let mut next = 0;
        for &ordinal in changed {
            let ordinal = ordinal as usize;
            if ordinal < next {
                return None;
            }
            next = ordinal + 1;
            let scope = scopes.get(ordinal)?;
            let index = baseline_index(previous, ordinal, scope_identity(scope.scope))?;
            self.scope_edit(
                scope,
                Some(&previous[index]),
                &mut scratch.entries,
                work,
                edits,
            )?;
        }
        Some(())
    }

    fn mapped_edits(
        &self,
        scopes: &[cvariables::ScopeRow],
        changed: Option<&[u32]>,
        previous: &[rows::ScriptVariablesRow],
        scratch: &mut ScriptVariableComparison,
        work: &mut VariableWork,
        edits: &mut Edits,
    ) -> Option<()> {
        if !clear_seen(&mut scratch.scopes, previous.len()) {
            return None;
        }
        let mut listed = changed.map(|changed| changed.iter().peekable());
        for (ordinal, scope) in scopes.iter().enumerate() {
            let compared = match &mut listed {
                Some(listed) => listed.next_if(|&&next| next as usize == ordinal).is_some(),
                None => true,
            };
            let index = baseline_index(previous, ordinal, scope_identity(scope.scope));
            if let Some(index) = index
                && !first_seen(&mut scratch.scopes, index)
            {
                return None;
            }
            let row = index.map(|index| &previous[index]);
            match (row, compared) {
                (_, true) => self.scope_edit(scope, row, &mut scratch.entries, work, edits)?,
                (Some(_), false) => {}
                (None, false) => return None,
            }
        }
        if listed.is_some_and(|mut listed| listed.next().is_some()) {
            return None;
        }
        for (index, row) in previous.iter().enumerate() {
            if scratch.scopes[index / 64] & (1_u64 << (index % 64)) == 0 {
                reserve(edits, 1).ok()?;
                edits.push((row.key(), None));
                work.scopes_removed += 1;
            }
        }
        Some(())
    }

    fn scope_edit(
        &self,
        scope: &cvariables::ScopeRow,
        previous: Option<&rows::ScriptVariablesRow>,
        seen: &mut Vec<u64>,
        work: &mut VariableWork,
        edits: &mut Edits,
    ) -> Option<()> {
        let started = std::time::Instant::now();
        let (entries, names) = (&self.variable_entries, &self.variable_names);
        if !fits(scope.entries_start, scope.entries_len, entries.len) {
            return None;
        }
        let current = &entries.first[scope.entries_start..scope.entries_start + scope.entries_len];
        let mut bytes = 0;
        for entry in current {
            if !fits(entry.name_start, entry.name_len, names.len) {
                return None;
            }
            bytes += entry.name_len;
        }
        work.scopes_compared += 1;
        work.entries_compared += current.len() as u64;
        work.bytes_compared += (size_of_val(current) + bytes) as u64;
        let matched = previous
            .filter(|previous| previous.entries.len() == scope.entries_len)
            .and_then(|previous| self.scope_matches(scope, previous, seen));
        let mut build_ns = 0;
        if matched != Some(true) {
            let built = std::time::Instant::now();
            let row = self.variable_row(scope);
            work.rows_built += 1;
            if row
                .entries
                .windows(2)
                .any(|pair| pair[0].name == pair[1].name)
            {
                return None;
            }
            if previous != Some(&row) {
                match (previous, matched) {
                    (None, _) => work.scopes_added += 1,
                    (Some(_), None) => work.entry_sets_changed += 1,
                    (Some(_), Some(_)) => {}
                }
                work.rows_changed += u64::from(previous.is_some());
                reserve(edits, 1).ok()?;
                edits.push((row.key(), Some(row)));
            }
            build_ns = elapsed_ns(built);
        }
        work.build_ns = work.build_ns.saturating_add(build_ns);
        work.compare_ns = work
            .compare_ns
            .saturating_add(elapsed_ns(started).saturating_sub(build_ns));
        Some(())
    }

    fn complete_variables(
        &self,
        outcome: Outcome,
        reason: Complete,
        work: &mut VariableWork,
    ) -> TableInput {
        work.path = Path::Complete(reason);
        let started = std::time::Instant::now();
        let input = match self.variables_table(outcome) {
            TableView::Complete(rows) => TableInput::SharedComplete(Arc::new(rows)),
            view => TableInput::Sampled(view),
        };
        work.build_ns = work.build_ns.saturating_add(elapsed_ns(started));
        input
    }

    #[cfg(feature = "conversion_check")]
    fn reference_variables(
        &self,
        outcome: Outcome,
        baseline: Option<(BaselineToken, &[rows::ScriptVariablesRow])>,
        scratch: &mut ScriptVariableComparison,
    ) -> TableInput {
        let Some((token, previous)) = baseline else {
            return TableInput::Sampled(self.variables_table(outcome));
        };
        let mut changed = Vec::new();
        let matches = self.compare_variables(outcome, previous, scratch, |index, ordinal| {
            if reserve(&mut changed, 1).is_err() {
                return false;
            }
            changed.push((index, ordinal));
            true
        });
        if !matches {
            return TableInput::Sampled(self.variables_table(outcome));
        }
        if changed.is_empty() {
            return TableInput::ComparedEqual(token);
        }
        changed.sort_unstable_by_key(|&(index, _)| index);
        TableInput::ComparedChanges {
            token,
            indices: changed.iter().map(|&(index, _)| index).collect(),
            rows: TableRows::ScriptVariables(
                changed
                    .iter()
                    .map(|&(_, ordinal)| self.variable_row(&self.variables.rows[ordinal]))
                    .collect(),
            ),
        }
    }

    #[cfg(feature = "conversion_check")]
    fn compare_variables(
        &self,
        outcome: Outcome,
        previous: &[rows::ScriptVariablesRow],
        scratch: &mut ScriptVariableComparison,
        mut changed: impl FnMut(usize, usize) -> bool,
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
            || !sorted_scopes(previous)
            || !clear_seen(&mut scratch.scopes, len)
        {
            return false;
        }
        for (ordinal, scope) in self.variables.rows[..len].iter().enumerate() {
            let Some(index) = baseline_index(previous, ordinal, scope_identity(scope.scope)) else {
                return false;
            };
            if !first_seen(&mut scratch.scopes, index) {
                return false;
            }
            match self.scope_matches(scope, &previous[index], &mut scratch.entries) {
                Some(true) => {}
                Some(false) if changed(index, ordinal) => {}
                _ => return false,
            }
        }
        true
    }

    fn scope_matches(
        &self,
        scope: &cvariables::ScopeRow,
        previous: &rows::ScriptVariablesRow,
        seen: &mut Vec<u64>,
    ) -> Option<bool> {
        let seed = previous.seed.require("ScriptVariables.seed").ok()?;
        let mut equal = *seed == scope.seed;
        let old = &previous.entries;
        if scope.entries_len != old.len()
            || old.windows(2).any(|pair| pair[0].name >= pair[1].name)
            || !clear_seen(seen, old.len())
        {
            return None;
        }
        for entry in &self.variable_entries.first
            [scope.entries_start..scope.entries_start + scope.entries_len]
        {
            let name =
                &self.variable_names.first[entry.name_start..entry.name_start + entry.name_len];
            let index = old
                .binary_search_by(|row| row.name.as_slice().cmp(name))
                .ok()?;
            if !first_seen(seen, index) {
                return None;
            }
            equal &= entry.value_raw == old[index].value_raw;
        }
        Some(equal)
    }

    pub fn observe_seeds(&self, work: &mut SeedWork) {
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
            self.variable_row(r)
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

    fn variable_row(&self, r: &cvariables::ScopeRow) -> rows::ScriptVariablesRow {
        let (scope_kind, scope_id) = scope_identity(r.scope);
        let mut entries = owned_vec(map_slice(
            &self.variable_entries.first[r.entries_start..r.entries_start + r.entries_len],
            |entry| rows::ScriptVariableEntry {
                name: owned_vec(copy_vec(
                    &self.variable_names.first[entry.name_start..entry.name_start + entry.name_len],
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
    }

    fn operations_table(&self, outcome: Outcome) -> TableView {
        if !valid_arena(
            outcome,
            &self.operations.rows,
            &self.operation_descriptions.first,
            self.operation_descriptions.len,
            self.operation_descriptions.required,
        ) {
            return unavailable("operation description lies outside captured arena");
        }
        let parts = &self.operation_parts.first;
        let in_order =
            |r: &coperation_instance::Row| operation_parts(r, parts, self.operation_parts.len);
        if !valid_arena(
            outcome,
            &self.operations.rows,
            parts,
            self.operation_parts.len,
            self.operation_parts.required,
        ) || !self
            .operations
            .rows
            .get(..outcome.len())
            .is_some_and(|rows| rows.iter().all(|r| in_order(r).is_some()))
        {
            return unavailable("operation parts lie outside captured arena or out of order");
        }
        table::<tables::Operations, _>(outcome, &self.operations.rows, |r| {
            let [phases, requirements, equipment, totals] = in_order(r).unwrap_or_default();
            rows::OperationRow {
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
                bonus: rows::Recorded::Value(r.bonus),
                detected: rows::Recorded::Value(r.detected),
                effect_description: rows::Recorded::Value(owned_vec(copy_vec(literal(
                    &self.operation_descriptions.first,
                    r.effect_description,
                )))),
                phases: owned_vec(map_slice(phases, |part| match part {
                    coperation_instance::Part::Phase(token) => *token,
                    _ => 0,
                })),
                requirements: owned_vec(map_slice(requirements, |part| match part {
                    coperation_instance::Part::Requirement(requirement) => {
                        rows::OperationRequirement {
                            amount: requirement.amount,
                            days: requirement.days,
                            archetype: requirement.archetype,
                            equipment: requirement.equipment,
                            uses_time: requirement.uses_time,
                        }
                    }
                    _ => Default::default(),
                })),
                equipment: owned_vec(map_slice(equipment, |part| match part {
                    coperation_instance::Part::Equipment(entry) => rows::OperationEquipment {
                        variant: entry.variant,
                        archetype: entry.archetype,
                        amount: entry.amount,
                    },
                    _ => Default::default(),
                })),
                equipment_totals: owned_vec(map_slice(totals, |part| match part {
                    coperation_instance::Part::EquipmentTotal(total) => {
                        rows::OperationEquipmentTotal {
                            archetype: total.archetype,
                            amount: total.amount,
                        }
                    }
                    _ => Default::default(),
                })),
                equipment_allow_zero: r.equipment_allow_zero,
            }
        })
    }

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

fn capture_rows<R>(captured: &mut CaptureRows<R>, walk: impl FnOnce(&mut Vec<R>) -> Outcome) {
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

impl ArenaRow<u8> for coperation_instance::Row {
    fn valid(&self, len: usize) -> bool {
        name_fits(self.effect_description, len)
    }
}

impl ArenaRow<coperation_instance::Part> for coperation_instance::Row {
    fn valid(&self, len: usize) -> bool {
        fits(self.parts_start, self.part_count(), len)
    }
}

fn operation_parts<'a>(
    row: &coperation_instance::Row,
    parts: &'a [coperation_instance::Part],
    len: usize,
) -> Option<[&'a [coperation_instance::Part]; 4]> {
    use coperation_instance::Part;
    let counts = [
        row.phase_count,
        row.requirement_count,
        row.equipment_count,
        row.equipment_total_count,
    ];
    let mut spans: [&[Part]; 4] = [&[]; 4];
    let mut start = row.parts_start;
    for (kind, (span, count)) in spans.iter_mut().zip(counts).enumerate() {
        let end = start
            .checked_add(count as usize)
            .filter(|&end| end <= len)?;
        let slice = parts.get(start..end)?;
        if !slice.iter().all(|part| {
            matches!(
                (kind, part),
                (0, Part::Phase(_))
                    | (1, Part::Requirement(_))
                    | (2, Part::Equipment(_))
                    | (3, Part::EquipmentTotal(_))
            )
        }) {
            return None;
        }
        *span = slice;
        start = end;
    }
    Some(spans)
}

impl ArenaRow<agency_operation_buffers::Entry> for agency_operation_buffers::Row {
    fn valid(&self, len: usize) -> bool {
        self.counts
            .iter()
            .try_fold(0usize, |sum, n| sum.checked_add(*n))
            .is_some_and(|count| fits(self.entries_start, count, len))
    }
}

impl ArenaRow<strategic_operatives::Entry> for strategic_operatives::Row {
    fn valid(&self, len: usize) -> bool {
        self.trade_len
            .checked_add(self.pressure_len)
            .and_then(|count| {
                self.mission_counts
                    .into_iter()
                    .try_fold(count, usize::checked_add)
            })
            .is_some_and(|count| fits(self.entries_start, count, len))
    }
}

impl ArenaRow<u8> for cintelligence_agency::Row {
    fn valid(&self, len: usize) -> bool {
        name_fits(self.name, len)
    }
}

impl ArenaRow<u8> for coperative_leader::Row {
    fn valid(&self, len: usize) -> bool {
        name_fits(self.name, len)
    }
}

impl ArenaRow<u8> for operative_missions::Row {
    fn valid(&self, len: usize) -> bool {
        name_fits(self.ideology, len)
            && name_fits(self.pending_ideology, len)
            && name_fits(self.portrait, len)
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
    walk: impl FnOnce(&mut Vec<R>, &mut Vec<T>) -> ArenaRead,
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

#[allow(
    clippy::too_many_arguments,
    reason = "the walk's operands, its row arenas, the delta and the held store"
)]
fn walk_variables(
    memory: &impl Memory,
    base: usize,
    state: usize,
    rows: &mut CaptureRows<cvariables::ScopeRow>,
    entries: &mut Arena<cvariables::Row>,
    names: &mut Arena<u8>,
    delta: &mut VariableDelta,
    store: Option<&mut VariableStore>,
) {
    let mut reuse =
        store.map(|store| VariableReuse::begin(store, &crate::variable_changes::Observer));
    delta.clear();
    capture_variables(rows, entries, names, |scopes, entries, names| {
        let _reads = read_accounting::table(TABLES[8]);
        if let Some(reuse) = reuse.as_mut()
            && let Some(walk) = reuse.quiet_hour(memory, base, state, scopes, entries, names)
        {
            return walk;
        }
        let (scopes, entries, names) = (
            engine_native_memory::walk::Buffer::Growing(scopes),
            engine_native_memory::walk::Buffer::Growing(entries),
            engine_native_memory::walk::Buffer::Growing(names),
        );
        match reuse.as_mut() {
            Some(reuse) => {
                cvariables::walk_scopes_carried(memory, base, state, scopes, entries, names, reuse)
            }
            None => cvariables::walk_scopes_buffer(memory, base, state, scopes, entries, names),
        }
    });
    if let Some(reuse) = reuse {
        if matches!(rows.outcome, Outcome::Rows { .. }) {
            let len = rows.outcome.len();
            reuse.finish(
                true,
                &rows.rows[..len],
                &entries.first[..entries.len],
                &names.first[..names.len],
                delta,
            );
        } else {
            reuse.finish(false, &[], &[], &[], delta);
        }
    }
}

pub struct VariablePart {
    rows: CaptureRows<cvariables::ScopeRow>,
    entries: Arena<cvariables::Row>,
    names: Arena<u8>,
    delta: VariableDelta,
    left_out: bool,
}

impl VariablePart {
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.entries.allocated_bytes()
            + self.names.allocated_bytes()
            + self.delta.allocated_bytes()
    }

    fn due(&self, context: CaptureContext) -> bool {
        !self.left_out
            && replay_data::cadence::should_walk(TABLES[8], context.hour, context.force_full)
    }

    fn reset(&mut self) {
        self.rows.reset();
        self.entries.reset();
        self.names.reset();
        self.delta.clear();
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        context: CaptureContext,
        store: Option<&mut VariableStore>,
    ) -> u64 {
        self.reset();
        if !self.due(context) {
            self.rows.outcome = Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON,
            };
            return 0;
        }
        let begin = qpc();
        walk_variables(
            memory,
            context.base,
            context.game_state,
            &mut self.rows,
            &mut self.entries,
            &mut self.names,
            &mut self.delta,
            store,
        );
        elapsed(begin)
    }

    pub fn fail(&mut self, context: CaptureContext, reason: &'static str) {
        self.reset();
        self.rows.outcome = Outcome::Unavailable {
            reason: if self.due(context) {
                reason
            } else {
                replay_data::cadence::SKIPPED_REASON
            },
        };
    }
}

impl Captured {
    pub fn take_variables(&mut self) -> VariablePart {
        self.variables_apart = true;
        VariablePart {
            rows: std::mem::replace(&mut self.variables, CaptureRows::new(0)),
            entries: std::mem::replace(&mut self.variable_entries, Arena::new(0)),
            names: std::mem::replace(&mut self.variable_names, Arena::new(0)),
            delta: std::mem::take(&mut self.variable_delta),
            left_out: self.left_out[8],
        }
    }

    pub fn restore_variables(&mut self, part: VariablePart) {
        self.variables = part.rows;
        self.variable_entries = part.entries;
        self.variable_names = part.names;
        self.variable_delta = part.delta;
        self.variables_apart = false;
    }
}

fn capture_variables(
    captured: &mut CaptureRows<cvariables::ScopeRow>,
    entries: &mut Arena<cvariables::Row>,
    names: &mut Arena<u8>,
    walk: impl FnOnce(
        &mut Vec<cvariables::ScopeRow>,
        &mut Vec<cvariables::Row>,
        &mut Vec<u8>,
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

fn walk_flags(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<ScopedFlag>,
    scratch: &mut Vec<cscript_flag::Row>,
    reuse: Option<FlagReuse<'_>>,
) -> Outcome {
    walk_flags_buffer(
        memory,
        base,
        state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(scratch),
        reuse,
        None,
    )
}

fn walk_flags_buffer(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, ScopedFlag>,
    mut scratch: engine_native_memory::walk::Buffer<'_, cscript_flag::Row>,
    mut reuse: Option<FlagReuse<'_>>,
    mut scopes: Option<&mut usize>,
) -> Outcome {
    if let Some(reuse) = reuse.as_mut()
        && let Some(outcome) = reuse.quiet_hour(memory, base, state, &mut output, |held| {
            same_countries(memory, base, state, held)
        })
    {
        return outcome;
    }
    let result = (|| {
        let mut len = 0_usize;
        let mut unread = 0_u32;
        let root = |root, offset| -> Result<usize, &'static str> {
            let manager = pointer(memory, root, offset)?;
            if manager == 0 {
                return Err("script flag manager root is null");
            }
            Ok(manager)
        };
        let mut scope = |kind,
                         id,
                         manager: usize,
                         gathered: Option<Option<[u8; cflag_manager::HEADER]>>,
                         country: Option<usize>|
         -> Result<(), &'static str> {
            if let Some(scopes) = scopes.as_mut() {
                **scopes += 1;
            }
            if let (Some(reuse), Some(country)) = (reuse.as_mut(), country) {
                reuse.country(country, id);
            }
            let plan = reuse.as_mut().map(|reuse| reuse.plan(kind, id, manager));
            if let (Some(reuse), Some(plan)) = (reuse.as_mut(), plan.as_ref())
                && let Some(rows) = plan.carry_rows()
            {
                let end = len
                    .checked_add(rows)
                    .ok_or("scoped flag row demand overflow")?;
                let room = if output.is_growing() {
                    output.ensure(end)?;
                    true
                } else {
                    rows <= (output.len() - len).min(scratch.len())
                };
                if room {
                    for flag in reuse.carry(plan) {
                        output[len] = ScopedFlag {
                            scope_kind: kind,
                            scope_id: id,
                            flag: *flag,
                        };
                        len += 1;
                    }
                    return Ok(());
                }
            }
            let head = gathered.unwrap_or_else(|| cflag_manager::read_header(memory, manager));
            let available = (output.len() - len).min(scratch.len());
            let outcome = if output.is_growing() {
                let read = cflag_manager::walk_header_buffer(memory, base, head, scratch.suffix(0));
                output.ensure(
                    len.checked_add(read.len())
                        .ok_or("scoped flag row demand overflow")?,
                )?;
                read
            } else {
                bounded(
                    cflag_manager::walk_header_buffer(
                        memory,
                        base,
                        head,
                        engine_native_memory::walk::Buffer::Count,
                    ),
                    available,
                )
            };
            if let (Some(reuse), Some(plan)) = (reuse.as_mut(), plan.as_ref()) {
                reuse.read(plan, outcome, &scratch[..outcome.len()]);
            }
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
        scope(
            GLOBAL_SCOPE,
            0,
            root(state, cgame_state::GLOBAL_FLAGS)?,
            None,
            None,
        )?;
        let countries = ccountry::Reader::new(memory, base, state)?;
        let mut found_country = false;
        for slot in 1..countries.count() {
            if let Some(country) = countries.at(slot)? {
                found_country = true;
                scope(
                    COUNTRY_SCOPE,
                    country.id,
                    root(country.pointer, ccountry::FLAG_MANAGER)?,
                    None,
                    Some(country.pointer),
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
        let state_vtable =
            engine_sites::at(base, cstate::VTABLE_RVA).ok_or("state vtable overflow")?;
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
                let id = first + index as u32;
                let manager = state_flag_manager(memory, at, state_vtable, id)?;
                scope(STATE_SCOPE, id, manager, None, None)?;
            }
            first += count as u32;
        }
        if !found_state {
            return Err("state table has no readable states");
        }
        let mut characters = ccharacter_manager::Roster::read(memory, base, state)?.cursor();
        while let Some(character) = characters.next_occurrence(memory)? {
            let mut head = [0; cflag_manager::HEADER];
            let (id, copied) = ccharacter::identity_with_spans(
                memory,
                base,
                character,
                [ccharacter::FLAGS, 0, 0],
                [&mut head, &mut [], &mut []],
            )?;
            let manager = character
                .checked_add(ccharacter::FLAGS)
                .ok_or("character flag manager address overflow")?;
            scope(
                CHARACTER_SCOPE,
                id,
                manager,
                Some((copied != 0).then_some(head)),
                None,
            )?;
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
    let outcome = result.unwrap_or_else(|reason| Outcome::Unavailable { reason });
    if let Some(reuse) = reuse {
        let complete = matches!(outcome, Outcome::Rows { .. });
        let rows = if complete {
            &output[..outcome.len()]
        } else {
            &[]
        };
        reuse.finish(complete, rows);
    }
    outcome
}

fn same_countries(memory: &impl Memory, base: usize, state: usize, held: &[(usize, u32)]) -> bool {
    let Ok(countries) = ccountry::Reader::new(memory, base, state) else {
        return false;
    };
    let mut held = held.iter();
    for slot in 1..countries.count() {
        match countries.at(slot) {
            Ok(Some(country)) => {
                if held.next() != Some(&(country.pointer, country.id)) {
                    return false;
                }
            }
            Ok(None) => {}
            Err(_) => return false,
        }
    }
    held.next().is_none()
}

fn state_flag_manager(
    memory: &impl Memory,
    state: usize,
    vtable: usize,
    slot: u32,
) -> Result<usize, &'static str> {
    let manager = |root: Option<[u8; 8]>| {
        let manager = root
            .and_then(|root| usize::try_from(u64::from_le_bytes(root)).ok())
            .ok_or("script scope root unreadable")?;
        if manager == 0 {
            return Err("script flag manager root is null");
        }
        Ok(manager)
    };
    let (Some(id_at), Some(root_at)) = (
        state.checked_add(cstate::ID),
        state.checked_add(cstate::FLAGS),
    ) else {
        if read::<u64>(memory, state) != Some(vtable as u64) {
            return Err("state scope vtable mismatch");
        }
        let id = state
            .checked_add(cstate::ID)
            .and_then(|p| read::<u32>(memory, p))
            .ok_or("state scope id unreadable")?;
        if id != slot {
            return Err("state scope id differs from slot");
        }
        return manager(
            state
                .checked_add(cstate::FLAGS)
                .and_then(|p| read::<[u8; 8]>(memory, p)),
        );
    };
    let mut identity = [0; 8];
    let mut id = [0; 4];
    let mut root = [0; 8];
    let completed = memory.copy_scattered(
        [state, id_at, root_at],
        &mut [&mut identity, &mut id, &mut root],
    );
    if completed == 0 || u64::from_le_bytes(identity) != vtable as u64 {
        return Err("state scope vtable mismatch");
    }
    if completed == 1 {
        return Err("state scope id unreadable");
    }
    if u32::from_le_bytes(id) != slot {
        return Err("state scope id differs from slot");
    }
    manager((completed == 3).then_some(root))
}

fn pointer(memory: &impl Memory, root: usize, offset: usize) -> Result<usize, &'static str> {
    root.checked_add(offset)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .ok_or("script scope root unreadable")
}


type Drain =
    fn(&mut u64, &mut [crelation_status::Mark], &mut usize) -> Option<(u64, Option<usize>)>;

struct SharedOpinions {
    baseline: crelation_status::Baseline,
    reported: crelation_status::BaselineCounts,
    consumed: u64,
    marks: Vec<crelation_status::Mark>,
    required_marks: usize,
    required_log: usize,
    drain: Drain,
    offer: fn(usize),
    charge: crate::raw_memory::Charge,
}

impl SharedOpinions {
    fn new() -> Self {
        Self {
            baseline: crelation_status::Baseline::default(),
            reported: crelation_status::BaselineCounts::default(),
            consumed: default_cursor(),
            marks: Vec::new(),
            required_marks: 0,
            required_log: 0,
            drain: default_drain(),
            offer: default_offer(),
            charge: crate::raw_memory::Charge::default(),
        }
    }

    fn grow(&mut self) {
        fn fit<T: Default + Clone>(vector: &mut Vec<T>, required: usize) {
            if required > vector.len() {
                let len = required.saturating_add(required / 4);
                if reserve_exact(vector, len - vector.len()).is_ok() {
                    vector.resize(len, T::default());
                }
            }
        }
        if self
            .baseline
            .reserve_marks(self.required_marks)
            .map_err(crate::allocation::io)
            .is_err()
        {
            return;
        }
        fit(&mut self.marks, self.required_marks);
        (self.offer)(self.required_log.max(self.baseline.rows()));
        self.charge.set(crate::raw_memory::Capacity {
            payload_vectors: (self.baseline.allocated_bytes()
                + self.marks.capacity() * size_of::<crelation_status::Mark>())
                as u64,
            ..Default::default()
        });
    }
}

#[cfg(not(any(test, feature = "test-support")))]
fn default_cursor() -> u64 {
    crate::opinion_changes::cursor()
}

#[cfg(any(test, feature = "test-support"))]
fn default_cursor() -> u64 {
    0
}

#[cfg(not(any(test, feature = "test-support")))]
fn default_drain() -> Drain {
    crate::opinion_changes::drain
}

#[cfg(not(any(test, feature = "test-support")))]
fn default_offer() -> fn(usize) {
    crate::opinion_changes::offer
}

#[cfg(any(test, feature = "test-support"))]
fn default_offer() -> fn(usize) {
    |_| {}
}

#[cfg(any(test, feature = "test-support"))]
fn default_drain() -> Drain {
    |_, _, _| None
}

#[allow(clippy::large_enum_variant)]
enum OpinionSlot {
    Unmade,
    Home(SharedOpinions),
    Lent,
}

type OpinionStore = std::sync::Mutex<OpinionSlot>;

#[derive(Clone)]
enum StoreRef {
    #[cfg(not(any(test, feature = "test-support")))]
    Shared(&'static OpinionStore),
    #[cfg(any(test, feature = "test-support"))]
    Owned(std::sync::Arc<OpinionStore>),
}

impl StoreRef {
    fn get(&self) -> &OpinionStore {
        match self {
            #[cfg(not(any(test, feature = "test-support")))]
            Self::Shared(store) => store,
            #[cfg(any(test, feature = "test-support"))]
            Self::Owned(store) => store,
        }
    }
}

#[cfg(not(any(test, feature = "test-support")))]
static RELATION_OPINIONS: OpinionStore = std::sync::Mutex::new(OpinionSlot::Unmade);

#[cfg(not(any(test, feature = "test-support")))]
fn default_opinion_store() -> StoreRef {
    StoreRef::Shared(&RELATION_OPINIONS)
}

#[cfg(any(test, feature = "test-support"))]
fn default_opinion_store() -> StoreRef {
    StoreRef::Owned(std::sync::Arc::new(std::sync::Mutex::new(OpinionSlot::Unmade)))
}

struct LentOpinions {
    store: StoreRef,
    shared: Option<SharedOpinions>,
}

impl LentOpinions {
    fn borrow(store: StoreRef) -> Option<Self> {
        let mut slot = store.get().try_lock().ok()?;
        let shared = match std::mem::replace(&mut *slot, OpinionSlot::Lent) {
            OpinionSlot::Unmade => SharedOpinions::new(),
            OpinionSlot::Home(shared) => shared,
            OpinionSlot::Lent => return None,
        };
        drop(slot);
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
            && let Ok(mut slot) = self.store.get().lock()
        {
            *slot = OpinionSlot::Home(shared);
        }
    }
}

static RELATION_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn forget_relation_opinions() {
    RELATION_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

pub fn relation_epoch() -> u64 {
    RELATION_EPOCH.load(std::sync::atomic::Ordering::Relaxed)
}

#[derive(Default)]
pub struct RelationsJob {
    pub active: [bool; 2],
    pub opinions: Vec<crelation_status::OpinionRow>,
    pub tuples: Vec<crelation_status::TupleRow>,
    provisioned: bool,
    walk: Option<crelation_status::Walk>,
    resolver: crelation_status::Resolver,
    store: Option<StoreRef>,
    shared: Option<LentOpinions>,
    capacity_charge: crate::raw_memory::Charge,
    pub selection: Option<SliceSelection>,
    pub fingerprints: Option<Box<SliceFingerprints>>,
}

impl std::fmt::Debug for RelationsJob {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RelationsJob")
            .field("active", &self.active)
            .field("walk", &self.walk)
            .finish_non_exhaustive()
    }
}

impl RelationsJob {
    pub fn new() -> Self {
        let mut job = Self {
            provisioned: true,
            ..Self::default()
        };
        job.update_capacity_charge();
        job
    }

    pub fn clear(&mut self) {
        self.active = [false; 2];
        self.walk = None;
    }

    pub fn published(&self) -> bool {
        self.walk.is_some_and(|walk| {
            matches!(walk.opinions, Outcome::Rows { .. })
                && matches!(walk.tuples, Outcome::Rows { .. })
        })
    }

    pub fn is_allocated(&self) -> bool {
        self.provisioned
    }

    pub fn grow_for_next(&self) -> bool {
        self.resolver.required() > self.resolver.capacity()
    }

    pub fn return_opinions(&mut self) {
        if let Some(mut lent) = self.shared.take() {
            lent.shared().grow();
        }
    }

    pub fn grow(&mut self) {
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
pub struct RelationsCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; 2],
    pub raw_bytes: [u64; 2],
}

#[derive(Debug, Clone, Copy)]
pub struct RelationsCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub active: [bool; 2],
    pub epoch: u64,
}

fn select_marked(
    memory: &impl Memory,
    baseline: &crelation_status::Baseline,
    marks: &[crelation_status::Mark],
    selection: &mut SliceSelection,
) -> bool {
    marks.iter().all(|mark| {
        let owner = baseline.owner(mark.diplomacy).or_else(|| {
            let country = read::<u64>(
                memory,
                mark.diplomacy.checked_add(engine_native_parts::cdiplomacy_status_layout::COUNTRY)?,
            )?;
            let country = usize::try_from(country)
                .ok()
                .filter(|&country| country != 0)?;
            read::<u32>(memory, country.checked_add(ccountry::ID)?)
        });
        owner.is_some_and(|owner| selection.select(owner))
    })
}

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

pub fn copy_relations_refresh(
    memory: &impl Memory,
    ctx: RelationsCopyContext,
    job: &mut RelationsJob,
) -> RelationsCopyReport {
    job.clear();
    job.provisioned = true;
    job.active = ctx.active;
    if !ctx.active.into_iter().any(|active| active) {
        return RelationsCopyReport {
            raw_ok: true,
            ..Default::default()
        };
    }
    if job.shared.is_none() {
        let store = job.store.get_or_insert_with(default_opinion_store).clone();
        job.shared = LentOpinions::borrow(store);
    }
    let anchor = usize::from(ctx.active[1]);
    let _reads = read_accounting::table(TABLES[13 + anchor]);
    let started = qpc();
    if let (Some(selection), Some(fingerprints)) = (&mut job.selection, &mut job.fingerprints) {
        scan_relation_owners(memory, ctx.base, ctx.game_state, selection, fingerprints);
    }
    let mut unshared = crelation_status::Baseline::default();
    let (baseline, observation) = match &mut job.shared {
        Some(lent) => {
            let SharedOpinions {
                baseline,
                consumed,
                marks,
                required_marks,
                required_log,
                drain,
                ..
            } = lent.shared();
            let mut pending = 0;
            let drained = if ctx.active[0] {
                drain(consumed, marks.as_mut_slice(), &mut pending)
            } else {
                None
            };
            let continues =
                drained.is_some_and(|(epoch, _)| baseline.token() == Some((epoch, ctx.epoch)));
            if continues {
                *required_marks = pending;
            }
            *required_log = pending;
            let observation = drained.map(|(epoch, count)| crelation_status::Observation {
                token: (epoch, ctx.epoch),
                marks: count.map(|count| &marks[..count]),
            });
            if let Some(selection) = &mut job.selection {
                let marked = observation
                    .and_then(|observation| observation.marks)
                    .filter(|_| continues);
                if !marked.is_some_and(|marks| select_marked(memory, baseline, marks, selection)) {
                    selection.force_full = true;
                }
            }
            (baseline, observation)
        }
        None => {
            crate::opinion_changes::store_lent();
            (&mut unshared, None)
        }
    };
    let selection = job.selection.as_ref();
    let select = |owner| selection.is_none_or(|selection| selection.contains(owner));
    let walk = crelation_status::walk_with(
        memory,
        ctx.base,
        ctx.game_state,
        &mut job.opinions,
        &mut job.tuples,
        ctx.active,
        &mut job.resolver,
        baseline,
        observation,
        select,
    );
    if let Some(lent) = &mut job.shared {
        let shared = lent.shared();
        crate::opinion_changes::record(shared.baseline.counts(), &mut shared.reported);
    }
    let mut copy_ticks = [0; 2];
    copy_ticks[anchor] = elapsed(started);
    job.walk = Some(walk);
    job.update_capacity_charge();
    RelationsCopyReport {
        raw_ok: true,
        copy_ticks,
        raw_bytes: [
            (walk.opinions.len() * size_of::<crelation_status::OpinionRow>()) as u64,
            (walk.tuples.len() * size_of::<crelation_status::TupleRow>()) as u64,
        ],
    }
}

impl Captured {
    pub fn opinion_rows_cap(&self) -> usize {
        self.relation_opinions.rows.len()
    }

    pub fn tuple_rows_cap(&self) -> usize {
        self.relation_tuples.rows.len()
    }

    pub fn fail_relations_capture(&mut self, active: [bool; 2], reason: &'static str) {
        self.relation_active = active;
        self.relation_opinions.outcome = Outcome::Unavailable {
            reason: if active[0] {
                reason
            } else {
                replay_data::cadence::SKIPPED_REASON
            },
        };
        self.relation_tuples.outcome = Outcome::Unavailable {
            reason: if active[1] {
                reason
            } else {
                replay_data::cadence::SKIPPED_REASON
            },
        };
        self.relations_raw_mode = false;
    }

    pub fn decode_relations_payload(&mut self, job: &RelationsJob, decode_ticks: &mut [u64; 2]) {
        self.relation_active = job.active;
        let Some(walk) = job
            .walk
            .filter(|_| job.active.into_iter().any(|active| active))
        else {
            self.fail_relations_capture(job.active, replay_data::cadence::SKIPPED_REASON);
            return;
        };
        let started = qpc();
        let skipped = Outcome::Unavailable {
            reason: replay_data::cadence::SKIPPED_REASON,
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

fn install<R: Copy + Default>(
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
    if rows.rows.len() < len && engine_allocation::resize_default(&mut rows.rows, len).is_err() {
        rows.outcome = Outcome::Unavailable {
            reason: "relation row storage allocation failed",
        };
        return;
    }
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


const M_CAPITULATION: u32 = 0x01;

#[derive(Debug, Default)]
pub struct CapitulationJob {
    pub log: RawLog<1>,
    pub game_state: usize,
}

impl CapitulationJob {
    pub fn new() -> Self {
        Self {
            log: RawLog::new(0, 0),
            game_state: 0,
        }
    }

    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    pub fn grow(&mut self) {
        self.log.grow();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct CapitulationCopyReport {
    pub raw_ok: bool,
    pub copy_ticks: [u64; 1],
    pub raw_bytes: [u64; 1],
}

#[derive(Debug, Clone, Copy)]
pub struct CapitulationCopyContext {
    pub base: usize,
    pub game_state: usize,
}

fn cap_pool_u64(log: &RawLog<1>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn capspan(
    log: &mut RawLog<1>,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

fn copy_capitulation_pass(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    log: &mut RawLog<1>,
) {
    use engine_native::cgame_state;
    let _reads = read_accounting::table(TABLES[0]);
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
    let Some(mut slots) = crate::raw::CountrySlots::new(countries, links, base) else {
        return;
    };
    for slot in 1..slots.count() {
        let pointer = match slots.take(log, memory, slot, M_CAPITULATION) {
            Some(Some(pointer)) => pointer,
            Some(None) => continue,
            None => return,
        };
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
            diplomacy.checked_add(engine_native_parts::cdiplomacy_status_layout::COUNTRY),
            8,
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
        let goals = capspan(
            log,
            memory,
            diplomacy.checked_add(engine_native_parts::cdiplomacy_status_layout::OWNED_WARGOALS),
            16,
            M_CAPITULATION,
        );
        if log.overflowed {
            return;
        }
        if let Some(at) = goals {
            let mut header = [0u8; 16];
            header.copy_from_slice(&log.bytes[at..at + 16]);
            if let Some(goals) = Array::from_header(header, 8) {
                copy_war_goals(log, memory, base, goals);
            }
        }
        if log.overflowed {
            return;
        }
    }
}

fn copy_war_goals(log: &mut RawLog<1>, memory: &impl Memory, base: usize, goals: Array) {
    let Some(vtable) = engine_sites::at(base, engine_native::cwar_relation::goals::VTABLE_RVA) else {
        return;
    };
    for ordinal in 0..goals.count {
        let Some(start) = capspan(log, memory, goals.element(ordinal, 8), 8, M_CAPITULATION) else {
            return;
        };
        let Some(goal) = usize::try_from(cap_pool_u64(log, start))
            .ok()
            .filter(|&goal| goal != 0)
        else {
            return;
        };
        let Some(start) = capspan(log, memory, Some(goal), 8, M_CAPITULATION) else {
            return;
        };
        if cap_pool_u64(log, start) != vtable as u64 {
            return;
        }
        if capspan(
            log,
            memory,
            goal.checked_add(engine_native::cwar_relation::goals::RECIPIENT),
            4,
            M_CAPITULATION,
        )
        .is_none()
        {
            return;
        }
    }
}

pub fn copy_capitulation_refresh(
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
    fn capitulation_table(&self, outcome: Outcome) -> TableView {
        let goals = |row: &cdiplomacy_status::CapitulationRow| match row.war_goals {
            None => Some(None),
            Some(count) => row
                .war_goals_start
                .checked_add(count as usize)
                .and_then(|end| self.war_goal_recipients.get(row.war_goals_start..end))
                .map(Some),
        };
        let rows = self
            .capitulation
            .rows
            .get(..outcome.len())
            .unwrap_or_default();
        if !rows.iter().all(|row| goals(row).is_some()) {
            return unavailable("war goal recipients outside the captured list");
        }
        table::<tables::Capitulation, _>(outcome, &self.capitulation.rows, |r| {
            rows::CapitulationRow {
                country: r.country,
                capitulated: r.capitulated,
                capitulated_hour: r.capitulated_hour,
                war_goal_recipients: rows::Recorded::Value(
                    goals(r).unwrap().map(|list| owned_vec(copy_vec(list))),
                ),
                ..Default::default()
            }
        })
    }

    pub fn fail_capitulation_capture(&mut self, reason: &'static str) {
        self.capitulation.outcome = Outcome::Unavailable { reason };
        self.capitulation_raw_mode = false;
    }

    pub fn decode_capitulation_payload(
        &mut self,
        base: usize,
        job: &CapitulationJob,
        decode_ticks: &mut [u64; 1],
    ) {
        let recorded = match crate::raw::Recorded::build(&job.log) {
            Ok(recorded) => recorded,
            Err(reason) => {
                self.fail_capitulation_capture(reason);
                return;
            }
        };
        let started = qpc();
        self.capitulation.outcome = bounded(
            cdiplomacy_status::walk_capitulation(
                &recorded,
                base,
                job.game_state,
                &mut self.capitulation.rows,
                &mut self.war_goal_recipients,
            ),
            self.capitulation.rows.len(),
        );
        decode_ticks[0] += elapsed(started);
        self.capitulation_raw_mode = false;
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod relations_test_kit {
    use super::*;
        use engine_native::{cgame_state, crelation};
    use engine_native_parts::{cguarantee_relation, cmarket_access_rights_relation};
    use engine_native_memory::memory::SliceMemory;

    pub const BASE: usize = 0;
    pub const GS: usize = 0x1000;
    pub const COUNTRIES: [usize; 3] = [0x4000, 0x5000, 0x6000];
    pub const DIPLOMACY: [usize; 3] = [0x8000, 0x9000, 0xa000];
    pub const RELATION: [usize; 2] = [0x1c000, 0x1c800];

    pub fn p32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    pub fn p64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    pub fn array(bytes: &mut [u8], at: usize, data: usize, values: &[u64]) {
        p64(bytes, at, data as u64);
        p32(bytes, at + 8, values.len() as u32);
        p32(bytes, at + 12, values.len() as u32);
        for (index, value) in values.iter().enumerate() {
            p64(bytes, data + index * 8, *value);
        }
    }
    pub fn status(owner: usize, target: usize) -> usize {
        0x10000 + owner * 0x4000 + target * 0x1000
    }
    pub fn own(bytes: &mut [u8], owner: usize, relations: &[usize]) {
        let relations: Vec<u64> = relations.iter().map(|&r| r as u64).collect();
        array(
            bytes,
            DIPLOMACY[owner] + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS,
            0x3400 + owner * 0x40,
            &relations,
        );
    }
    pub fn matrix_data(owner: usize) -> usize {
        0x3200 + owner * 0x40
    }
    pub fn serializable_data(owner: usize, target: usize) -> usize {
        0x2800 + owner * 0x80 + target * 0x20
    }

    pub fn fixture() -> Vec<u8> {
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
            p64(&mut bytes, country, (BASE + ccountry::VTABLE_RVA.0) as u64);
            p32(&mut bytes, country + ccountry::ID, owner as u32 + 1);
            p64(
                &mut bytes,
                country + ccountry::DIPLOMACY_STATUS,
                diplomacy as u64,
            );
            p64(
                &mut bytes,
                diplomacy,
                (BASE + engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0) as u64,
            );
            p64(
                &mut bytes,
                diplomacy + engine_native_parts::cdiplomacy_status_layout::COUNTRY,
                country as u64,
            );
            let slots: Vec<u64> = (0..4).map(|t| status(owner, t) as u64).collect();
            array(
                &mut bytes,
                diplomacy + engine_native_parts::cdiplomacy_status_layout::RELATIONS,
                matrix_data(owner),
                &slots,
            );
            for target in 0..4 {
                let object = status(owner, target);
                p64(
                    &mut bytes,
                    object,
                    (BASE + crelation_status::VTABLE_RVA.0) as u64,
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

    pub fn relation(
        bytes: &mut [u8],
        object: usize,
        vtable: engine_sites::Rva,
        type_id: u32,
        first: u32,
        second: u32,
    ) {
        p64(bytes, object, (BASE + vtable.0) as u64);
        p32(bytes, object + crelation::TYPE_ID, type_id);
        p32(bytes, object + crelation::FIRST_COUNTRY, first);
        p32(bytes, object + crelation::SECOND_COUNTRY, second);
    }

    pub fn memory(bytes: &[u8]) -> SliceMemory<'_> {
        SliceMemory { base: 0, bytes }
    }

    pub struct Direct {
        pub opinion_rows: Vec<crelation_status::OpinionRow>,
        pub opinion_outcome: Outcome,
        pub tuple_rows: Vec<crelation_status::TupleRow>,
        pub tuple_outcome: Outcome,
    }

    pub fn direct(memory: &impl Memory, opinion_cap: usize, tuple_cap: usize) -> Direct {
        let mut opinions = vec![crelation_status::OpinionRow::default(); opinion_cap];
        let mut tuples = vec![crelation_status::TupleRow::default(); tuple_cap];
        let result =
            crelation_status::walk(memory, BASE, GS, &mut opinions, &mut tuples, [true; 2]);
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

    pub fn split(
        memory: &impl Memory,
        intel: &mut Captured,
    ) -> (RelationsCopyReport, RelationsJob) {
        let mut job = RelationsJob::new();
        let ctx = RelationsCopyContext {
            base: BASE,
            game_state: GS,
            active: [true; 2],
            epoch: 0,
        };
        let report = copy_relations_refresh(memory, ctx, &mut job);
        assert!(report.raw_ok);
        intel.decode_relations_payload(&job, &mut [0; 2]);
        (report, job)
    }

    pub fn assert_matches_direct(intel: &Captured, direct: &Direct) {
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

    pub fn intel_captured(opinion_cap: usize, tuple_cap: usize) -> Captured {
        let mut capacities = Capacities::default();
        capacities.rows[13] = opinion_cap;
        capacities.rows[14] = tuple_cap;
        Captured::with_capacities(&capacities)
    }

    pub fn opinionated_fixture() -> Vec<u8> {
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

#[cfg(any(test, feature = "test-support"))]
pub fn intel_shadow_fixture(value: i64) -> Captured {
    let capacities = Capacities {
        rows: [2; N],
        intel_scratch: 0,
        subject_names: 0,
        autonomy_names: 0,
        agency_names: 0,
        agency_upgrade_definitions: 0,
        operation_descriptions: 0,
        operative_names: 0,
        operative_mission_bytes: 0,
        operation_parts: 0,
        strategic_operative_entries: 0,
        agency_operation_entries: 0,
        variable_entries: 0,
        variable_names: 0,
        tokens: 0,
    };
    let mut capture = Captured::with_capacities(&capacities);
    let Walk::Ready(buffer) = capture.country_intel.borrow(true) else {
        unreachable!("a new frame holds its own buffer")
    };
    for (index, row) in buffer.rows.rows.iter_mut().enumerate() {
        *row = engine_native_intelligence::country_intel_row::Row {
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
    #[test]
    fn certified_quiet_tables_seed_first_off_slot_periodic_and_post_gap_keyframes() {
        use crate::retained_snapshot::RetainedSnapshot;
        use replay_frame::frame::{
            FrameDecoder, FrameEncoder, FrameInput, Metrics, TABLE_COUNT, TableUpdate,
        };
        let expected = [
            TableRows::PropagandaEfforts(vec![
                rows::PropagandaEffortRow {
                    slot: 0,
                    country: 0,
                    sequence: 7,
                    stability: -17,
                    war_support: 29,
                    derived_drift: 0,
                },
                rows::PropagandaEffortRow {
                    slot: 0,
                    country: 0,
                    sequence: u64::MAX,
                    stability: -31,
                    war_support: 43,
                    derived_drift: 0,
                },
            ]),
            TableRows::IntelContributions(vec![rows::IntelContributionRow {
                owner: 0,
                kind: 1,
                pool: 2,
                ordinal: 3,
                target: 4,
                civilian: -5,
                army: 6,
                navy: 7,
                air: 8,
            }]),
            TableRows::IntelSources(vec![
                rows::IntelSourceRow {
                    owner: 0,
                    pool: 1,
                    source: 2,
                    ordinal: 3,
                    target: 4,
                    civilian: -5,
                    army: 6,
                    navy: 7,
                    air: 8,
                    id: 9,
                    removed: 1,
                    next_id: 10,
                    discriminant: 11,
                    count: 12,
                    other_count: 13,
                    reference_country: 14,
                    reference_pool: -15,
                },
                rows::IntelSourceRow {
                    owner: 0,
                    pool: u32::MAX,
                    source: 0,
                    ordinal: 0,
                    reference_country: 7,
                    reference_pool: 9,
                    ..Default::default()
                },
                rows::IntelSourceRow {
                    owner: 0,
                    pool: u32::MAX,
                    source: u32::MAX,
                    ordinal: u32::MAX,
                    discriminant: 19,
                    count: 4,
                    other_count: 2,
                    ..Default::default()
                },
            ]),
            TableRows::IntelNetworkAuxiliaries(vec![
                rows::IntelNetworkAuxiliaryRow {
                    owner: 1,
                    target: 2,
                    id: 3,
                    kind: 11,
                    subnetwork: 4,
                    ordinal: 5,
                    country: 6,
                    count_a: 7,
                    count_b: -8,
                    count_c: 9,
                    first: -10,
                    second: 11,
                    third: 12,
                },
                rows::IntelNetworkAuxiliaryRow {
                    owner: 2,
                    target: 1,
                    id: 1,
                    kind: 6,
                    country: 9,
                    ..Default::default()
                },
            ]),
        ];
        assert_eq!(TABLES[22..26], expected.each_ref().map(TableRows::table));
        let mut retained = std::array::from_fn::<_, 4, _>(|_| RetainedSnapshot::new());
        let mut snapshots = std::array::from_fn::<_, 4, _>(|_| None);
        for (index, rows) in expected.iter().enumerate() {
            assert_eq!(
                retained[index].publish(
                    rows.table(),
                    Outcome::Rows { len: rows.len() },
                    || crate::retained_snapshot::fixture(rows.try_clone().unwrap()),
                    &mut snapshots[index]
                ),
                Outcome::Rows { len: rows.len() }
            );
        }
        let mut captured = Captured::with_capacities(&Capacities::default());
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        for (hour, reset, force, keyframe) in [
            (1, false, false, true),
            (2, false, false, false),
            (721, false, false, true),
            (723, true, false, true),
            (724, false, true, true),
        ] {
            if reset {
                encoder = FrameEncoder::new();
                decoder = FrameDecoder::new();
            }
            captured.reset_residual();
            for (index, expected) in expected.iter().enumerate() {
                crate::test_support::with_allocations_denied(|| {
                    let quiet = Outcome::Unavailable {
                        reason: replay_data::cadence::SKIPPED_REASON,
                    };
                    assert_eq!(
                        retained[index].publish(
                            expected.table(),
                            quiet,
                            || panic!("quiet cut must not project rows"),
                            &mut captured.certified_snapshots[index]
                        ),
                        quiet
                    );
                    assert!(Arc::ptr_eq(
                        snapshots[index].as_ref().unwrap(),
                        captured.certified_snapshots[index].as_ref().unwrap()
                    ));
                });
            }
            let inputs =
                captured.table_inputs(None, &mut ScriptVariableComparison::default(), None);
            let mut tables = std::array::from_fn::<_, TABLE_COUNT, _>(|_| {
                TableInput::Sampled(TableView::Unavailable {
                    reason: "outside intel regression".into(),
                })
            });
            for (table, input) in TABLES.into_iter().zip(inputs) {
                if expected.iter().any(|rows| rows.table() == table) {
                    let TableInput::SharedComplete(rows) = &input else {
                        panic!("{table:?} lost baseline")
                    };
                    let TableInput::SharedComplete(projected) =
                        snapshots[table_slot(table) - 22].as_ref().unwrap().input()
                    else {
                        panic!()
                    };
                    assert!(Arc::ptr_eq(rows, &projected));
                    tables[table.index()] = input;
                }
            }
            let mut prepared = encoder
                .prepare_bytes(
                    FrameInput {
                        branch: 1,
                        hour,
                        tables,
                        metrics: Metrics::default(),
                    },
                    force,
                )
                .unwrap();
            let encoded = prepared.take_frame();
            encoder.commit(prepared).unwrap();
            let frame = encoded.decode().unwrap();
            assert_eq!(frame.keyframe, keyframe);
            let decoded = decoder.apply(&frame).unwrap();
            for expected in &expected {
                let actual = &*decoded.tables[expected.table().index()];
                assert_eq!(actual, &TableView::Complete(expected.try_clone().unwrap()));
                assert!(expected.field_gaps().next().is_none());
                for row in expected {
                    assert_eq!(row.not_recorded_fields(), 0);
                    row.visit_nested_field_gaps(&mut |name| panic!("missing {name}"));
                }
                assert!(
                    matches!(
                        frame.tables[expected.table().index()],
                        TableUpdate::Snapshot(_)
                    ) == keyframe
                );
                if !keyframe {
                    assert!(matches!(
                        frame.tables[expected.table().index()],
                        TableUpdate::Unchanged { .. }
                    ));
                }
            }
        }
    }

    #[test]
    fn spool_return_recycles_original_native_buffer_and_preserves_an_unadmitted_baseline() {
        use crate::retained_snapshot::{AcquiredRows, RetainedSnapshot};
        let mut capacities = Capacities::default();
        capacities.rows[24] = 16_384;
        let mut captured = Captured::with_capacities(&capacities);
        for (index, row) in captured.source_values.rows[..5000].iter_mut().enumerate() {
            *row = rows::IntelSourceRow {
                owner: (5000 - index) as u32,
                pool: u32::MAX,
                source: u32::MAX,
                ordinal: u32::MAX,
                count: 3,
                ..Default::default()
            };
        }
        let pointer = captured.source_values.rows.as_ptr();
        let allocated = captured.allocated_bytes();
        let mut retained = RetainedSnapshot::new();
        captured.source_values.outcome = Outcome::Rows { len: 5000 };
        assert_eq!(
            retained.publish(
                TableId::IntelSources,
                captured.source_values.outcome,
                || AcquiredRows::Sources(std::mem::take(&mut captured.source_values.rows)),
                &mut captured.certified_snapshots[2]
            ),
            Outcome::Rows { len: 5000 }
        );
        assert_eq!(captured.source_values.rows.capacity(), 0);
        assert_eq!(
            captured.allocated_bytes(),
            allocated,
            "handoff must not lose/double frame accounting"
        );
        captured.grow(&mut capacities);
        assert_eq!(captured.source_values.rows.as_ptr(), pointer);
        assert_eq!(captured.source_values.rows.capacity(), 16_384);
        captured.reset_residual();
        crate::test_support::with_allocations_denied(|| {
            let quiet = Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON,
            };
            assert_eq!(
                retained.publish(
                    TableId::IntelSources,
                    quiet,
                    || panic!("quiet copy"),
                    &mut captured.certified_snapshots[2]
                ),
                quiet
            );
        });
        let inputs = captured.table_inputs(None, &mut ScriptVariableComparison::default(), None);
        let TableInput::SharedComplete(rows) = &inputs[24] else {
            panic!("lost unadmitted baseline")
        };
        assert_eq!(rows.len(), 5000);
        rows.validate_keys().unwrap();
        assert_eq!(rows.rows::<tables::IntelSources>().unwrap()[0].owner, 1);
        assert_eq!(
            rows.rows::<tables::IntelSources>().unwrap()[4999].owner,
            5000
        );
    }

    #[test]
    fn network_summary_without_a_writer_certificate_never_adds_off_cadence_payload_reads() {
        use engine_intelligence::{
            clocal_intel_network as local, cstrategic_operative as strategic,
            cstrategic_operative_manager as manager,
        };
        use engine_native::ccountry::synthetic::World;
        let mut world = World::new(0, 2, 0x1100);
        let manager_root = world.alloc(0x40);
        let objects = world.alloc(3 * 8);
        let network_pointers = world.alloc(8);
        let network = world.alloc(0x108);
        let graph = world.alloc(engine_intelligence::intel_network_graph::SIZE);
        world.u64(
            world.game_state() + cgame_state::STRATEGIC_OPERATIVE_MANAGER,
            manager_root as u64,
        );
        world.array(manager_root + manager::OPERATIVES_DATA, objects, 3);
        for slot in 1..=2 {
            let object = world.alloc(strategic::SIZE);
            world.u64(objects + slot * 8, object as u64);
            world.u64(object, strategic::VTABLE_RVA.0 as u64);
            world.u32(object + strategic::OWNER, slot as u32);
            if slot == 1 {
                world.array(object + strategic::NETWORKS_DATA, network_pointers, 1);
            }
        }
        world.u64(network_pointers, network as u64);
        world.u64(network, ccountry_intel_network::VTABLE_RVA.0 as u64);
        world.u32(network + ccountry_intel_network::TARGET, 2);
        world.u64(
            network + ccountry_intel_network::LOCAL_NETWORK,
            local::VTABLE_RVA.0 as u64,
        );
        world.u64(
            network + ccountry_intel_network::LOCAL_NETWORK + local::GRAPH,
            graph as u64,
        );
        world.u32(network + ccountry_intel_network::COVERED_CORE_STATES, 17);
        let mut captured = Captured::with_capacities(&Capacities {
            rows: [1; N],
            ..Default::default()
        });
        let context = CaptureContext {
            base: 0,
            game_state: world.game_state(),
            hour: 1,
            force_full: true,
        };
        captured.capture_residual(&world.memory(), context, None, None);
        assert_eq!(captured.network_summaries.outcome, Outcome::Rows { len: 1 });
        assert_eq!(captured.network_summaries.rows[0].covered_core_states, 17);
        assert!(crate::country_input_changes::intel_network_writes().is_none());
        fn audit_admission(captured: &Captured) {
            for (id, view) in TABLES.into_iter().zip(captured.tables()) {
                if ![
                    TableId::IntelNetworkSummaries,
                    TableId::PropagandaEfforts,
                    TableId::IntelContributions,
                    TableId::IntelSources,
                    TableId::IntelNetworkAuxiliaries,
                ]
                .contains(&id)
                {
                    continue;
                }
                match view {
                    TableView::Complete(rows) => {
                        assert!(rows.field_gaps().next().is_none(), "{id:?}");
                        for row in &rows {
                            assert_eq!(row.not_recorded_fields(), 0, "{id:?}");
                            row.visit_nested_field_gaps(&mut |name| panic!("{id:?}: {name}"));
                        }
                    }
                    TableView::Unavailable { reason } => {
                        assert!(!reason.is_empty(), "{id:?}");
                        assert_ne!(reason, replay_data::cadence::SKIPPED_REASON, "{id:?}");
                    }
                    TableView::Truncated { .. } => panic!("unexpected incomplete {id:?}"),
                    TableView::NotRecorded { .. } => panic!("unexpected unrecorded {id:?}"),
                }
            }
        }
        audit_admission(&captured);
        captured.fail_residual_capture(context, "synthetic spool gap");
        captured.capture_residual(
            &world.memory(),
            CaptureContext { hour: 3, ..context },
            None,
            None,
        );
        assert_eq!(captured.network_summaries.outcome, Outcome::Rows { len: 1 });
        assert_eq!(captured.network_summaries.rows[0].covered_core_states, 17);
        audit_admission(&captured);

        struct DenyPayload<'a, M> {
            inner: &'a M,
            start: usize,
            end: usize,
            attempts: std::cell::Cell<usize>,
        }
        impl<M: Memory> Memory for DenyPayload<'_, M> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address < self.end && address.saturating_add(output.len()) > self.start {
                    self.attempts.set(self.attempts.get() + 1);
                    return false;
                }
                self.inner.copy(address, output)
            }
        }
        let memory = world.memory();
        let denied = DenyPayload {
            inner: &memory,
            start: network + ccountry_intel_network::COVERED_CORE_STATES,
            end: network + ccountry_intel_network::MAX_STATE_STRENGTH + 8,
            attempts: std::cell::Cell::new(0),
        };
        captured.capture_residual(
            &denied,
            CaptureContext {
                hour: 4,
                force_full: false,
                ..context
            },
            None,
            None,
        );
        assert_eq!(denied.attempts.get(), 0);
        assert_eq!(captured.networks.outcome, Outcome::Rows { len: 0 });
        assert_eq!(
            captured.network_summaries.outcome,
            Outcome::Unavailable {
                reason: "intel network summary writer certificate unavailable"
            }
        );
    }
    use super::*;
    mod flag_store_tests;
    mod variable_projection_tests;
    mod variable_store_tests;

    #[test]
    fn intel_catalog_growth_and_refusal_projection_handle_allocator_denial() {
        let mut catalog = agency_upgrade_levels::Catalog::new(1);
        crate::test_support::with_allocations_denied(|| catalog.resize(5000));
        assert_eq!(catalog.capacity(), 1);
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.operative_failure = Some((u64::MAX, 5000, 3));
        let view = crate::test_support::with_allocations_denied(|| {
            captured.operatives_table(Outcome::Unavailable {
                reason: "synthetic operative read failed",
            })
        });
        assert!(matches!(view, TableView::Unavailable { .. }));
    }

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
                replay_data::cadence::should_walk(TableId::RelationOpinions, hour, force_full),
                replay_data::cadence::should_walk(TableId::RelationTuples, hour, force_full),
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
                                reason: replay_data::cadence::SKIPPED_REASON,
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
            agency_upgrade_definitions: 0,
            operation_descriptions: 0,
            operative_names: 0,
            operative_mission_bytes: 0,
            operation_parts: 0,
            strategic_operative_entries: 0,
            agency_operation_entries: 0,
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
            let report = captured.capture_residual(&Unreadable, context, None, None);
            assert_eq!(report.ticks[10], 0, "operations owns the shared read cost");
            for slot in [0, 13, 14] {
                assert_eq!(captured.outcomes()[slot], before[slot]);
                assert_eq!(report.ticks[slot], 0);
            }
            let report = captured.fail_residual_capture(context, "residual worker failed");
            assert_eq!(report.ticks, [0; N]);
            for slot in (1..=12).chain(15..N) {
                assert_eq!(
                    captured.outcomes()[slot],
                    Outcome::Unavailable {
                        reason: if replay_data::cadence::should_walk(
                            TABLES[slot],
                            context.hour,
                            context.force_full
                        ) {
                            "residual worker failed"
                        } else {
                            replay_data::cadence::SKIPPED_REASON
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
            Some(&mut IntelConversion::new(branch, 0, shadow)),
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
        let memory = engine_native_memory::memory::CountedMemory::new(Unreadable);
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
                None,
                None,
            );
            let counts = accounting.finish();
            assert!(crate::country_input_changes::intel_network_writes().is_none());
            assert_ne!(
                captured.outcomes()[6],
                Outcome::Unavailable {
                    reason: replay_data::cadence::SKIPPED_REASON
                }
            );
            for index in (1..=12).chain(std::iter::once(15)) {
                if index != 6 && !replay_data::cadence::should_walk(TABLES[index], hour, false) {
                    assert_eq!(
                        captured.outcomes()[index],
                        Outcome::Unavailable {
                            reason: replay_data::cadence::SKIPPED_REASON
                        }
                    );
                    assert_eq!(counts[TABLES[index].index()].spans, 0);
                }
            }
        }
    }

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
                None,
                None,
            );
            captured.outcomes()[15]
                != Outcome::Unavailable {
                    reason: replay_data::cadence::SKIPPED_REASON,
                }
        };
        assert!(capture(&mut first, 0));
        assert!(matches!(first.country_intel_sample(), Some(Err(_))));
        assert!(!capture(&mut second, 1));
        assert_eq!(second.country_intel_sample(), None);
        assert!(!capture(&mut second, 24));
        assert_eq!(second.country_intel_sample(), None);
        assert!(!capture(&mut second, 25));
        first.grow(&mut Capacities::default());
        assert!(capture(&mut second, 26));
        assert!(matches!(second.country_intel_sample(), Some(Err(_))));
        second.fail_residual_capture(
            CaptureContext {
                base: 0,
                game_state: 0x1000,
                hour: 26,
                force_full: false,
            },
            "deferred residual worker failed",
        );
        assert_eq!(
            second.country_intel_sample(),
            Some(Err("deferred residual worker failed"))
        );
        assert_eq!(
            second.outcomes()[15],
            Outcome::Unavailable {
                reason: "deferred residual worker failed"
            }
        );
        second.grow(&mut Capacities::default());
        assert!(!capture(&mut first, 27));
        assert_eq!(first.country_intel_sample(), None);
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
    fn a_witnessed_flags_hour_republishes_without_comparing_and_any_other_hour_compares() {
        let inputs = |capture: &Captured, shadow: &mut IntelShadow, hour| {
            capture.table_inputs(
                None,
                &mut ScriptVariableComparison::default(),
                Some(&mut IntelConversion::new(1, hour, shadow)),
            )
        };
        let mut capture = intel_shadow_fixture(42);
        capture.flags_witness = Some(9);
        let mut shadow = IntelShadow::default();
        let first = inputs(&capture, &mut shadow, 0);
        capture.flags.rows[0].flag.value += 1;
        let republished = inputs(&capture, &mut shadow, 1);
        assert!(Arc::ptr_eq(shared(&first[7]), shared(&republished[7])));
        let stats = shadow.statistics();
        assert_eq!((stats.republished, stats.hits, stats.misses), (1, 0, 1));
        let gap = inputs(&capture, &mut shadow, 3);
        assert!(!Arc::ptr_eq(shared(&first[7]), shared(&gap[7])));
        assert_eq!(
            TableView::Complete((**shared(&gap[7])).clone()),
            capture.tables()[7]
        );
        capture.flags_witness = Some(10);
        let moved = inputs(&capture, &mut shadow, 4);
        capture.flags_witness = None;
        let unwitnessed = inputs(&capture, &mut shadow, 5);
        assert!(Arc::ptr_eq(shared(&gap[7]), shared(&moved[7])));
        assert!(Arc::ptr_eq(shared(&gap[7]), shared(&unwitnessed[7])));
        let stats = shadow.statistics();
        assert_eq!((stats.republished, stats.hits, stats.misses), (1, 2, 2));
        shadow.forget(TableId::ScriptFlags);
        capture.flags_witness = Some(10);
        let refused = inputs(&capture, &mut shadow, 6);
        assert!(!Arc::ptr_eq(shared(&gap[7]), shared(&refused[7])));
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
        let bytes = relations_kit::opinionated_fixture();
        let ctx = RelationsCopyContext {
            base: relations_kit::BASE,
            game_state: relations_kit::GS,
            active: [true; 2],
            epoch: 0,
        };
        assert!(job.store.is_none(), "no store is made before a refresh");
        assert!(copy_relations_refresh(&relations_kit::memory(&bytes), ctx, &mut job).raw_ok);
        assert!(job.grow_for_next());
        job.grow();
        let grown = LOCAL.snapshot().current.payload_vectors;
        assert!(grown > initial);
        assert_eq!(grown, expected(&job));
        job.return_opinions();
        let Some(StoreRef::Owned(store)) = &job.store else {
            panic!("the refresh made the job its own opinion store");
        };
        let store = std::sync::Arc::downgrade(store);
        let mut lent = LentOpinions::borrow(job.store.clone().unwrap()).unwrap();
        let baseline = &lent.shared().baseline;
        assert!(baseline.allocated_bytes() > 0);
        assert!(!baseline.needs_growth());
        drop(lent);
        job.clear();
        assert_eq!(LOCAL.snapshot().current.payload_vectors, grown);
        drop(job);
        assert_eq!(LOCAL.snapshot().current, Capacity::default());
        assert!(
            store.upgrade().is_none(),
            "the job's own store and the baseline its refresh grew are released with it"
        );
    }

    use engine_native_memory::memory::SliceMemory;
    use replay_data::rows::TableSchema;

    fn small() -> (Capacities, Captured) {
        let capacities = Capacities {
            rows: [3; N],
            intel_scratch: 2 * engine_native_intelligence::country_intel_row::STRIDE,
            subject_names: 8,
            autonomy_names: 8,
            agency_names: 8,
            agency_upgrade_definitions: 0,
            operation_descriptions: 0,
            operative_names: 0,
            operative_mission_bytes: 0,
            operation_parts: 0,
            strategic_operative_entries: 0,
            agency_operation_entries: 0,
            variable_entries: 4,
            variable_names: 8,
            tokens: 8,
        };
        let captured = Captured::with_capacities(&capacities);
        (capacities, captured)
    }

    #[test]
    fn intel_outcomes_preserve_raw_refused_entries_and_description_bytes() {
        let mut capacity = Capacities {
            rows: [2; N],
            operation_descriptions: 8,
            ..Capacities::default()
        };
        let mut capture = Captured::with_capacities(&capacity);
        capture.agency_upgrades.rows[0] = agency_upgrade_levels::Row {
            country: 5000,
            ordinal: 0,
            definition_pointer: 0x1234,
            branch_pointer: 0x5678,
            upgrade_token: Some(90001),
            branch_token: Some(90002),
            level: -7,
            identity_status: agency_upgrade_levels::AUTHENTICATED,
        };
        capture.agency_upgrades.rows[1] = agency_upgrade_levels::Row {
            country: 5000,
            ordinal: 1,
            definition_pointer: 0x9999,
            level: i32::MIN,
            identity_status: agency_upgrade_levels::DEFINITION_NOT_REGISTERED,
            ..Default::default()
        };
        capture.agency_upgrades.outcome = Outcome::Rows { len: 2 };
        capture.agency_upgrade_required = 2;
        capture.operations.rows[0] = coperation_instance::Row {
            id: 44,
            terminal: 1,
            bonus: 0xfe,
            detected: 0xfd,
            effect_description: Name { start: 0, len: 4 },
            ..Default::default()
        };
        capture.operations.outcome = Outcome::Rows { len: 1 };
        capture.operation_descriptions.first[..4].copy_from_slice(b"a\r\xffz");
        capture.operation_descriptions.len = 4;
        capture.operation_descriptions.required = 4;
        let views = capture.tables();
        let TableView::Complete(rows) = &views[17] else {
            panic!("complete raw acquired census must retain identity refusals")
        };
        let upgrades = rows.rows::<tables::AgencyUpgradeLevels>().unwrap();
        assert_eq!(upgrades.len(), 2);
        assert_eq!(upgrades[0].level, -7);
        assert_eq!(upgrades[1].level, i32::MIN);
        assert_eq!(upgrades[1].definition_pointer, 0x9999);
        assert_eq!(upgrades[1].upgrade_token, None);
        let operations = complete::<tables::Operations>(&views[9]);
        assert_eq!(
            (&operations[0].bonus, &operations[0].detected),
            (&rows::Recorded::Value(0xfe), &rows::Recorded::Value(0xfd))
        );
        assert_eq!(
            operations[0].effect_description,
            rows::Recorded::Value(b"a\r\xffz".to_vec())
        );
        capture.grow(&mut capacity);
        assert_eq!(capacity.rows[17], 2);
        capture.operations.rows[0].effect_description.len = 9;
        assert!(matches!(capture.tables()[9], TableView::Unavailable { .. }));
    }

    #[test]
    fn agency_upgrade_capture_retains_refused_raw_rows_and_counts_only_omissions() {
        const AGENCY: usize = 0xc000;
        const ENTRIES: usize = 0xc200;
        let mut bytes = flag_scopes();
        bytes.resize(0xd000, 0);
        put(
            &mut bytes,
            0x4000 + ccountry::INTELLIGENCE_AGENCY,
            &(AGENCY as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            AGENCY,
            &((BASE + cintelligence_agency::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            AGENCY + cintelligence_agency::OWNER_COUNTRY,
            &0x4000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            AGENCY + cintelligence_agency::NAME + engine_native_memory::msvc_string::CAPACITY,
            &engine_native_memory::msvc_string::INLINE_CAPACITY.to_le_bytes(),
        );
        put(
            &mut bytes,
            AGENCY + agency_upgrade_levels::ACQUIRED,
            &(ENTRIES as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            AGENCY + agency_upgrade_levels::ACQUIRED + 8,
            &2_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            AGENCY + agency_upgrade_levels::ACQUIRED + 12,
            &2_u32.to_le_bytes(),
        );
        put(&mut bytes, ENTRIES + 8, &i32::MIN.to_le_bytes());
        put(
            &mut bytes,
            ENTRIES + agency_upgrade_levels::ENTRY_STRIDE,
            &0xfeed_cafe_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            ENTRIES + agency_upgrade_levels::ENTRY_STRIDE + 8,
            &i32::MAX.to_le_bytes(),
        );
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };

        for (capacity, growing) in [(1, false), (2, false), (1, true), (2, true)] {
            let (mut capacities, _) = small();
            capacities.rows[17] = capacity;
            let mut captured = Captured::with_capacities(&capacities);
            if growing {
                captured.capture_residual(
                    &memory,
                    CaptureContext {
                        base: BASE,
                        game_state: GAME,
                        hour: 0,
                        force_full: true,
                    },
                    None,
                    None,
                );
            } else {
                let result = cintelligence_agency::walk_with_upgrades(
                    &memory,
                    BASE,
                    GAME,
                    &mut captured.agencies.rows,
                    &mut captured.agency_names.first,
                    &mut captured.agency_upgrades.rows,
                    &mut captured.agency_upgrade_catalog,
                );
                first_named(
                    &mut captured.agencies,
                    &mut captured.agency_names,
                    ArenaRead::from(result.agencies),
                );
                captured.agency_upgrades.outcome = result.upgrades.outcome;
                captured.agency_upgrade_required = result.upgrades.required_rows;
                assert_eq!(result.upgrades.omitted_rows, 0, "capacity {capacity}");
                assert_eq!(result.upgrades.refused_rows, 2);
                assert_eq!(
                    result.upgrades.refusal_counts[agency_upgrade_levels::NULL_DEFINITION as usize],
                    1
                );
                assert_eq!(
                    result.upgrades.refusal_counts
                        [agency_upgrade_levels::DATABASE_UNAVAILABLE as usize],
                    1
                );
            }
            let tables = captured.tables();
            let (rows, missing) = match &tables[17] {
                TableView::Complete(rows) => (rows, 0),
                TableView::Truncated { rows, missing } => (rows, *missing),
                other => panic!("raw refused entries must survive acquisition: {other:?}"),
            };
            let upgrades = rows.rows::<tables::AgencyUpgradeLevels>().unwrap();
            let retained = 2;
            assert_eq!(upgrades.len(), retained);
            assert_eq!(upgrades.len() + missing as usize, 2);
            assert_eq!(captured.agency_upgrade_required, 2);
            assert_eq!(
                upgrades[0].identity_status,
                agency_upgrade_levels::NULL_DEFINITION
            );
            assert_eq!(upgrades[0].level, i32::MIN);
            if retained == 2 {
                assert!(matches!(tables[17], TableView::Complete(_)));
                assert_eq!(missing, 0);
                assert_eq!(upgrades[1].definition_pointer, 0xfeed_cafe);
                assert_eq!(
                    upgrades[1].identity_status,
                    agency_upgrade_levels::DATABASE_UNAVAILABLE
                );
                assert_eq!(upgrades[1].level, i32::MAX);
            }

            let mut encoder = replay_frame::frame::FrameEncoder::new();
            let first = encoder
                .encode(
                    replay_frame::frame::DecodedFrame {
                        branch: 1,
                        hour: 0,
                        tables: std::array::from_fn(|_| {
                            unavailable("fixture table not acquired").into()
                        }),
                        metrics: replay_frame::frame::Metrics::default(),
                    },
                    false,
                )
                .unwrap();
            let mut views = std::array::from_fn(|_| unavailable("fixture table not acquired"));
            views[TableId::AgencyUpgradeLevels.index()] = tables[17].clone();
            let source = replay_frame::frame::DecodedFrame {
                branch: 1,
                hour: 1,
                tables: views.map(Into::into),
                metrics: replay_frame::frame::Metrics::default(),
            };
            let decoded_wire = encoder
                .encode(source.clone(), false)
                .unwrap()
                .encoded()
                .unwrap()
                .decode()
                .unwrap();
            let mut decoder = replay_frame::frame::FrameDecoder::new();
            decoder.advance(&first).unwrap();
            let round_trip = decoder.advance(&decoded_wire).unwrap();
            assert_eq!(
                round_trip.tables[TableId::AgencyUpgradeLevels.index()],
                source.tables[TableId::AgencyUpgradeLevels.index()]
            );
        }
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
            war_goals: Some(0),
            war_goals_start: 0,
        };
        capture.operatives.outcome = Outcome::Rows { len: 1 };
        capture.operatives.rows[0] = coperative_leader::Row {
            id: u64::MAX,
            owner_country: u32::MAX,
            roster_country: u32::MAX - 1,
            roster_kind: u8::MAX,
            ordinal: u32::MAX,
            assignment: u32::MAX,
            experience_raw: i64::MIN,
            skill: Some(i32::MIN),
            capturer_country: 0,
            captured_hour: i32::MAX,
            action_variant: None,
            target_country: Some(2),
            operation: Some(u64::MAX),
            name: Name { start: 0, len: 3 },
            cooldown_kind: Some(u32::MAX),
            cooldown_start_hour: Some(i32::MIN),
            cooldown_end_hour: Some(i32::MAX),
        };
        capture.operative_names.first = vec![0xff, 0, b'x'];
        capture.operative_names.len = 3;
        capture.operative_names.required = 3;
        capture.networks.outcome = Outcome::Rows { len: 2 };
        capture.networks.rows[0] = ccountry_intel_network::Row {
            owner_country: 1,
            target_country: 2,
            state_id: 3,
            strength_raw: None,
            subnetwork_selector: i32::MIN,
            local_strength_raw: i64::MIN,
            gain_raw: -175_000,
            gain_from_operatives_raw: 0,
            gain_from_adjacency_raw: 0,
            ..Default::default()
        };
        capture.networks.rows[1] = ccountry_intel_network::Row {
            native_ordinal: 1,
            owner_country: 1,
            target_country: 2,
            state_id: 4,
            strength_raw: Some(i64::MIN),
            subnetwork_selector: 0,
            local_strength_raw: i64::MAX,
            gain_raw: 85_000,
            gain_from_operatives_raw: 42_500,
            gain_from_adjacency_raw: 21_250,
            local_quiet: 1,
            gain_factor_raw: Some(100_000),
            strength_target_raw: Some(7_250_000),
            target_from_operatives_raw: Some(3_000_000),
            target_from_counterintelligence_raw: Some(-4_750_000),
            subnetwork_quiet: 1,
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
            (
                networks[0].gain_raw,
                networks[0].local_quiet,
                networks[0].subnetwork_quiet,
                networks[0].strength_target_raw,
            ),
            (-175_000, 0, 0, None)
        );
        assert_eq!(
            (
                networks[1].gain_raw,
                networks[1].gain_from_operatives_raw,
                networks[1].gain_from_adjacency_raw,
                networks[1].local_quiet,
                networks[1].gain_factor_raw,
                networks[1].strength_target_raw,
                networks[1].target_from_operatives_raw,
                networks[1].target_from_counterintelligence_raw,
                networks[1].subnetwork_quiet,
            ),
            (
                85_000,
                42_500,
                21_250,
                1,
                Some(100_000),
                Some(7_250_000),
                Some(3_000_000),
                Some(-4_750_000),
                1
            )
        );
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
            (u32::MAX, 0xfe, replay_data::rows::RetiredI64::default())
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
        assert_eq!(operative.target_country, rows::Recorded::Value(Some(2)));
        assert_eq!(operative.operation, rows::Recorded::Value(Some(u64::MAX)));
        assert_eq!(operative.name, rows::Recorded::Value(vec![0xff, 0, b'x']));
        assert_eq!(
            operative.cooldown_kind,
            rows::Recorded::Value(Some(u32::MAX))
        );
        assert_eq!(
            operative.cooldown_start_hour,
            rows::Recorded::Value(Some(i32::MIN))
        );
        assert_eq!(
            operative.cooldown_end_hour,
            rows::Recorded::Value(Some(i32::MAX))
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
        capture.operatives.rows[0].cooldown_kind = None;
        capture.operatives.rows[0].cooldown_start_hour = None;
        capture.operatives.rows[0].cooldown_end_hour = None;
        let views = capture.tables();
        let operative = &complete::<tables::OperativeMemberships>(&views[4])[0];
        assert_eq!(operative.cooldown_kind, rows::Recorded::Value(None));
        assert_eq!(operative.cooldown_start_hour, rows::Recorded::Value(None));
        assert_eq!(operative.cooldown_end_hour, rows::Recorded::Value(None));
    }

    #[test]
    fn operative_missions_convert_every_field_and_name_their_refusals() {
        let (_, mut capture) = small();
        capture.operative_missions.outcome = Outcome::Rows { len: 1 };
        capture.operative_missions.rows[0] = operative_missions::Row {
            native_pointer: 0,
            map_selected: None,
            map_warning: None,
            roster_country: u32::MAX,
            id: u64::MAX,
            ordinal: 2,
            owner_country: 7,
            assignment: 1,
            cooldown_kind: Some(3),
            cooldown_start_hour: Some(i32::MIN),
            cooldown_end_hour: Some(i32::MAX),
            variant: Some(5),
            target_country: Some(u32::MAX),
            target_state: Some(-1),
            ideology: Name { start: 0, len: 2 },
            pending_variant: Some(1),
            pending_country: Some(0),
            pending_state: None,
            pending_ideology: Name { start: 2, len: 1 },
            portrait: Name { start: 3, len: 3 },
            root_out_country_factor: Some(i64::MIN),
            root_out_operative_factor: Some(i64::MAX),
            root_out_nationality: Some(1),
        };
        capture.operative_mission_bytes.first = vec![b'f', b'a', b'c', b'G', 0, b'x'];
        capture.operative_mission_bytes.len = 6;
        capture.operative_mission_bytes.required = 6;
        let views = capture.tables();
        assert_eq!(
            complete::<tables::OperativeMissions>(&views[18]),
            [rows::OperativeMissionRow {
                active_effect: rows::Recorded::Value(None),
                pending_effect: rows::Recorded::Value(None),
                map_selected: rows::Recorded::Value(None),
                map_warning: rows::Recorded::Value(None),
                roster_country: u32::MAX,
                id: u64::MAX,
                ordinal: 2,
                owner_country: 7,
                assignment: 1,
                cooldown_kind: Some(3),
                cooldown_start_hour: Some(i32::MIN),
                cooldown_end_hour: Some(i32::MAX),
                variant: Some(5),
                target_country: Some(u32::MAX),
                target_state: Some(-1),
                ideology: b"fa".to_vec(),
                pending_variant: Some(1),
                pending_country: Some(0),
                pending_state: None,
                pending_ideology: b"c".to_vec(),
                portrait: vec![b'G', 0, b'x'],
                root_out_country_factor: Some(i64::MIN),
                root_out_operative_factor: Some(i64::MAX),
                root_out_nationality: Some(1),
            }]
        );
        capture.operative_missions.rows[0].portrait = Name { start: 5, len: 2 };
        assert!(matches!(
            capture.tables()[18],
            TableView::Unavailable { .. }
        ));
        capture.operative_mission_failure = Some((99, 7));
        capture.operative_missions.outcome = Outcome::Unavailable {
            reason: "operative portrait unreadable or invalid",
        };
        let TableView::Unavailable { reason } = &capture.tables()[18] else {
            panic!()
        };
        assert!(reason.contains("operative=99 roster=7 refused_reads=1"));
        capture.operative_mission_failure = None;
        capture.operative_mission_bytes.required = 40_000;
        let mut capacity = Capacities::default();
        capture.grow(&mut capacity);
        assert!(capacity.operative_mission_bytes >= 40_000);
    }

    #[test]
    fn strategic_operatives_convert_both_maps_from_their_entry_spans() {
        let (_, mut capture) = small();
        capture.strategic_operatives.outcome = Outcome::Rows { len: 2 };
        capture.strategic_operatives.rows[0] = strategic_operatives::Row {
            mission_counts: [0; 5],
            native_pointer: 0,
            country: u32::MAX,
            slot: 7,
            mission_activity: i64::MIN,
            operation_activity: i64::MAX,
            entries_start: 0,
            trade_len: 2,
            pressure_len: 1,
        };
        capture.strategic_operatives.rows[1] = strategic_operatives::Row {
            country: 2,
            slot: 2,
            entries_start: 3,
            ..Default::default()
        };
        capture.strategic_operative_entries.first = vec![
            strategic_operatives::Entry {
                operative: 0,
                owner: 2,
                value: 250_000,
                tension: 0,
            },
            strategic_operatives::Entry {
                operative: 0,
                owner: 3,
                value: 0,
                tension: 0,
            },
            strategic_operatives::Entry {
                operative: 0,
                owner: 2,
                value: 32_768,
                tension: -16_384,
            },
        ];
        capture.strategic_operative_entries.len = 3;
        capture.strategic_operative_entries.required = 3;
        let views = capture.tables();
        assert_eq!(
            complete::<tables::StrategicOperatives>(&views[19]),
            [
                rows::StrategicOperativeRow {
                    mission_rosters: rows::Recorded::Value(Some(Default::default())),
                    country: 2,
                    slot: 2,
                    ..Default::default()
                },
                rows::StrategicOperativeRow {
                    mission_rosters: rows::Recorded::Value(Some(Default::default())),
                    country: u32::MAX,
                    slot: 7,
                    mission_activity: i64::MIN,
                    operation_activity: i64::MAX,
                    trade_influence: vec![
                        rows::TradeInfluence {
                            owner: 2,
                            value: 250_000,
                        },
                        rows::TradeInfluence { owner: 3, value: 0 },
                    ],
                    diplomatic_pressure: vec![rows::DiplomaticPressure {
                        owner: 2,
                        acceptance: 32_768,
                        tension: -16_384,
                    }],
                },
            ]
        );
        capture.strategic_operatives.rows[0].pressure_len = 2;
        assert!(matches!(
            capture.tables()[19],
            TableView::Unavailable { .. }
        ));
        capture.strategic_operative_entries.required = 40_000;
        let mut capacity = Capacities::default();
        capture.grow(&mut capacity);
        assert!(capacity.strategic_operative_entries >= 40_000);
    }

    #[test]
    fn operations_convert_their_parts_by_kind_and_refuse_spans_out_of_order() {
        use coperation_instance::{Equipment, EquipmentTotal, Part, Requirement};
        let (_, mut capture) = small();
        capture.operations.outcome = Outcome::Rows { len: 1 };
        capture.operations.rows[0] = coperation_instance::Row {
            id: 9,
            phase_count: 2,
            requirement_count: 1,
            equipment_count: 1,
            equipment_total_count: 1,
            equipment_allow_zero: 1,
            parts_start: 1,
            ..Default::default()
        };
        capture.operation_parts.first = vec![
            Part::Empty,
            Part::Phase(0x11),
            Part::Phase(0x11),
            Part::Requirement(Requirement {
                amount: 250_000,
                days: 3,
                archetype: 0,
                equipment: 0,
                uses_time: 1,
            }),
            Part::Equipment(Equipment {
                variant: u64::MAX,
                archetype: 0x77,
                amount: i64::MIN,
            }),
            Part::EquipmentTotal(EquipmentTotal {
                archetype: 0x77,
                amount: i64::MAX,
            }),
        ];
        capture.operation_parts.len = 6;
        capture.operation_parts.required = 6;
        let views = capture.tables();
        let operation = &complete::<tables::Operations>(&views[9])[0];
        assert_eq!(
            (
                &operation.phases,
                &operation.requirements,
                &operation.equipment,
                &operation.equipment_totals,
                operation.equipment_allow_zero
            ),
            (
                &vec![0x11_u32, 0x11],
                &vec![rows::OperationRequirement {
                    amount: 250_000,
                    days: 3,
                    archetype: 0,
                    equipment: 0,
                    uses_time: 1,
                }],
                &vec![rows::OperationEquipment {
                    variant: u64::MAX,
                    archetype: 0x77,
                    amount: i64::MIN,
                }],
                &vec![rows::OperationEquipmentTotal {
                    archetype: 0x77,
                    amount: i64::MAX,
                }],
                1_u8
            )
        );
        capture.operations.rows[0].parts_start = 0;
        assert!(matches!(capture.tables()[9], TableView::Unavailable { .. }));
        capture.operations.rows[0].parts_start = 2;
        assert!(matches!(capture.tables()[9], TableView::Unavailable { .. }));
        capture.operation_parts.required = 40_000;
        let mut capacity = Capacities::default();
        capture.grow(&mut capacity);
        assert!(capacity.operation_parts >= 40_000);
    }

    #[test]
    fn invalid_arenas_duplicate_literal_keys_and_unavailable_scopes_never_become_empty_tables() {
        let (_, mut capture) = small();
        capture.operatives.outcome = Outcome::Rows { len: 1 };
        capture.operatives.rows[0].name = Name {
            start: usize::MAX,
            len: 1,
        };
        assert!(matches!(capture.tables()[4], TableView::Unavailable { .. }));
        capture.operative_failure = Some((99, 7, 2));
        capture.operatives.outcome = Outcome::Unavailable {
            reason: "operative saved name unreadable or invalid",
        };
        let TableView::Unavailable { reason } = &capture.tables()[4] else {
            panic!()
        };
        assert!(reason.contains("operative=99 roster=7 kind=2 refused_reads=1"));
        capture.operative_failure = None;
        capture.operative_names.required = 40_000;
        let mut capacity = Capacities::default();
        capture.grow(&mut capacity);
        assert!(capacity.operative_names >= 40_000);
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

    #[test]
    fn operative_refusals_keep_rows_without_storage_growth() {
        let (_, mut capture) = small();
        capture.operatives.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 2,
        };
        capture.operatives.rows[0].id = 88;
        capture.operative_refused_rows = 2;
        let refusal = coperative_leader::Refusal {
            id: 99,
            roster_country: 7,
            roster_kind: 2,
            reason: "active operative operation reference is stale",
        };
        capture.operative_first_refused = Some(refusal);
        let TableView::Truncated {
            rows: TableRows::OperativeMemberships(rows),
            missing,
        } = &capture.tables()[4]
        else {
            panic!()
        };
        assert_eq!(*missing, 2);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, 88);
        assert_eq!(
            operative_storage_outcome(capture.operatives.outcome, 2),
            Outcome::Rows { len: 1 }
        );
        let mut capacity = Capacities {
            rows: [8; N],
            ..Capacities::default()
        };
        capture.grow(&mut capacity);
        assert_eq!(capacity.rows[4], 8);
        assert_eq!(
            capture.operatives.outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 2
            }
        );
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

    fn variable_baseline(
        previous: Vec<rows::ScriptVariablesRow>,
    ) -> replay_frame::frame::FrameEncoder {
        let mut encoder = replay_frame::frame::FrameEncoder::new();
        let mut tables: [TableInput; replay_data::rows::TABLE_COUNT] =
            std::array::from_fn(|_| unavailable("fixture table not acquired").into());
        tables[TableId::ScriptVariables.index()] =
            TableInput::SharedComplete(Arc::new(rows::TableRows::ScriptVariables(previous)));
        encoder
            .encode_input(
                replay_frame::frame::FrameInput {
                    branch: 1,
                    hour: 0,
                    tables,
                    metrics: replay_frame::frame::Metrics::default(),
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
            let TableInput::SliceUpdate { update, .. } = &views[8] else {
                panic!("change {change}: {:?}", views[8]);
            };
            assert_eq!(
                update.keys.len(),
                if change == 2 { 2 } else { 1 },
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
    fn variable_seeds_use_live_scope_capacity_and_detect_seed_only_changes() {
        let (mut capacities, mut capture) = small();
        const COUNTRIES: usize = 5;
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
            [0, 1, 2, 3, 4],
            [0, 2, 1, 3, 4],
            [0, 1, 2, 4, 3],
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
            &((BASE + ccountry::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x4000 + ccountry::ID, &2_u32.to_le_bytes());
        put(&mut bytes, 0x2200, &u64::MAX.to_le_bytes());
        put(&mut bytes, 0x2208, &0x8000_u64.to_le_bytes());
        put(
            &mut bytes,
            0x8000,
            &((BASE + cstate::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x8000 + cstate::ID, &1_u32.to_le_bytes());
        put(
            &mut bytes,
            GAME + cgame_state::CHARACTER_MANAGER,
            &0x5000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x5000,
            &((BASE + ccharacter_manager::VTABLE_RVA.0) as u64).to_le_bytes(),
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
                &((BASE + ccharacter::VTABLE_RVA.0) as u64).to_le_bytes(),
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
            &((BASE + cflag_manager::VTABLE_RVA.0) as u64).to_le_bytes(),
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
                &((BASE + cflag_manager::VTABLE_RVA.0) as u64).to_le_bytes(),
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
                    &((BASE + vtable.0) as u64).to_le_bytes(),
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
        let _globals = crate::flag_changes::test_globals();
        let mut bytes = flag_scopes();
        let mut rows = vec![ScopedFlag::default(); 4];
        let mut scratch = vec![cscript_flag::Row::default(); 4];
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            walk_flags(&memory, BASE, GAME, &mut rows, &mut scratch, None),
            Outcome::Rows { len: 4 }
        );
        assert_eq!(
            rows.iter()
                .map(|row| (row.scope_kind, row.scope_id))
                .collect::<Vec<_>>(),
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
        assert_eq!(script_flag_demand(&memory, BASE, GAME), Some(4));
        let mut capacities = Capacities::default();
        capacities.rows[7] = 1;
        capacities.raise_for_script_flags(4);
        let mut sized = vec![ScopedFlag::default(); capacities.rows[7]];
        let mut scratch = vec![cscript_flag::Row::default(); capacities.rows[7]];
        assert_eq!(
            walk_flags(&memory, BASE, GAME, &mut sized, &mut scratch, None),
            Outcome::Rows { len: 4 }
        );
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
            ,
                None),
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
            ,
                None),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn flag_scopes_read_each_character_and_state_once_with_sequential_verdicts() {
        use std::cell::RefCell;
        struct Calls<'a> {
            bytes: &'a [u8],
            denied: Option<usize>,
            calls: RefCell<Vec<usize>>,
        }
        impl Calls<'_> {
            fn span(&self, address: usize, output: &mut [u8]) -> bool {
                if self
                    .denied
                    .is_some_and(|at| (address..address + output.len()).contains(&at))
                {
                    return false;
                }
                SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(address, output)
            }
        }
        impl Memory for Calls<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.calls.borrow_mut().push(address);
                self.span(address, output)
            }
            fn copy_scattered<const N: usize>(
                &self,
                addresses: [usize; N],
                destinations: &mut [&mut [u8]; N],
            ) -> usize {
                self.calls.borrow_mut().push(addresses[0]);
                for (index, (address, output)) in addresses
                    .into_iter()
                    .zip(destinations.iter_mut())
                    .enumerate()
                {
                    if !output.is_empty() && !self.span(address, output) {
                        return index;
                    }
                }
                N
            }
        }
        let bytes = flag_scopes();
        let walk = |denied| {
            let memory = Calls {
                bytes: &bytes,
                denied,
                calls: RefCell::default(),
            };
            let mut rows = vec![ScopedFlag::default(); 4];
            let mut scratch = vec![cscript_flag::Row::default(); 4];
            let outcome = walk_flags(&memory, BASE, GAME, &mut rows, &mut scratch, None);
            (outcome, memory.calls.into_inner())
        };
        let (outcome, calls) = walk(None);
        assert_eq!(outcome, Outcome::Rows { len: 4 });
        let within = |start: usize, len: usize| {
            calls
                .iter()
                .filter(|address| (start..start + len).contains(address))
                .count()
        };
        assert_eq!(within(0x6000, 0x200), 1);
        assert_eq!(within(0x6200, 0x200), 1);
        assert_eq!(within(0x8000, 0x1000), 1);
        for (denied, reason) in [
            (0x8000, "state scope vtable mismatch"),
            (0x8000 + cstate::ID, "state scope id unreadable"),
            (0x8000 + cstate::FLAGS, "script scope root unreadable"),
            (0x6000, "character identity unreadable"),
            (0x6200 + ccharacter::FLAGS, "flag manager header unreadable"),
            (
                0xb000 + 3 * cscript_flag::STRIDE + cscript_flag::DATE,
                "script flag fields unreadable",
            ),
        ] {
            assert_eq!(walk(Some(denied)).0, Outcome::Unavailable { reason });
        }
    }

    use super::relations_test_kit as relations_kit;

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
        let memory = engine_native_memory::memory::CountedMemory::new(memory);
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
                let active = [
                    replay_data::cadence::should_walk(TABLES[13], hour, false),
                    true,
                ];
                let opinion_cap = if active[0] { 16 } else { 0 };
                let tuple_cap = hour as usize % 3;
                let make_memory = || OpinionReads {
                    memory: relations_kit::memory(&bytes),
                    fault,
                    reads: std::cell::Cell::new(0),
                };
                let typed_memory = make_memory();
                let mut opinions = vec![crelation_status::OpinionRow::default(); opinion_cap];
                let mut tuples = vec![crelation_status::TupleRow::default(); 16];
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
                        epoch: 0,
                    },
                    &mut job,
                );
                assert!(report.raw_ok, "hour={hour} fault={fault}");
                let mut captured = relations_kit::intel_captured(opinion_cap, tuple_cap);
                let mut ticks = [0; 2];
                captured.decode_relations_payload(&job, &mut ticks);
                let skipped = Outcome::Unavailable {
                    reason: replay_data::cadence::SKIPPED_REASON,
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
    fn growing_relation_destinations_match_complete_rows_with_skipped_boundary() {
        let bytes = relations_kit::opinionated_fixture();
        let memory = relations_kit::memory(&bytes);
        for (opinion_cap, tuple_cap) in [(0, 0), (1, 1), (2, 2), (64, 64)] {
            let direct = relations_kit::direct(&memory, 2, 2);
            let mut intel = relations_kit::intel_captured(opinion_cap, tuple_cap);
            let (_, _) = relations_kit::split(&memory, &mut intel);
            relations_kit::assert_matches_direct(&intel, &direct);
        }
    }

    #[test]
    fn relations_job_matches_faults_with_identical_reasons() {
        use engine_native::{crelation::subject as csubject_relation, cwar_relation};
        let valid = relations_kit::opinionated_fixture();
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
                        relations_kit::DIPLOMACY[0] + engine_native_parts::cdiplomacy_status_layout::RELATIONS + 12,
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
                        relations_kit::RELATION[0] + engine_native::crelation::TYPE_ID,
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
            let _ = (cwar_relation::VTABLE_RVA, csubject_relation::VTABLE_RVA);
        }
    }

    #[test]
    fn relations_war_and_subject_elements_are_skipped_but_consume_ordinals() {
        use engine_native_parts::cguarantee_relation;
        use engine_native::{crelation::subject as csubject_relation, cwar_relation};
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
            engine_native_parts::cmarket_access_rights_relation::VTABLE_RVA,
            engine_native_parts::cmarket_access_rights_relation::TYPE,
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
                (engine_native_parts::cmarket_access_rights_relation::TYPE, 3),
            ]
        );
        let mut intel = relations_kit::intel_captured(16, 16);
        let (_, _) = relations_kit::split(&memory, &mut intel);
        relations_kit::assert_matches_direct(&intel, &direct);
    }

    #[test]
    fn relations_invalid_primary_bytes_preserve_unavailable_rows() {
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

    fn observed_store() -> StoreRef {
        let mut shared = SharedOpinions::new();
        shared.drain = test_drain;
        shared.marks = vec![crelation_status::Mark::default(); 8];
        StoreRef::Owned(std::sync::Arc::new(std::sync::Mutex::new(OpinionSlot::Home(shared))))
    }

    fn relations_ctx(epoch: u64) -> RelationsCopyContext {
        RelationsCopyContext {
            base: relations_kit::BASE,
            game_state: relations_kit::GS,
            active: [true; 2],
            epoch,
        }
    }

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

    #[test]
    fn relations_jobs_share_one_opinion_store() {
        let bytes = relations_kit::opinionated_fixture();
        let store = observed_store();
        let [mut first, mut second] = [(); 2].map(|()| RelationsJob {
            store: Some(store.clone()),
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
        assert_eq!(run(&mut second), rows);
        assert!(second.shared.is_none());
        first.return_opinions();
        let mut lent = LentOpinions::borrow(store.clone()).unwrap();
        assert!(lent.shared().baseline.allocated_bytes() > 0);
        drop(lent);
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

    #[test]
    fn relations_dispatch_spans_the_intel_loan_and_returns_its_slice() {
        let bytes = relations_kit::opinionated_fixture();
        let memory = relations_kit::memory(&bytes);
        let mut pack = relations_kit::intel_captured(16, 16);
        let mut store = Some(Box::new(SliceFingerprints::new()));
        let opinions = observed_store();
        let hour = |pack: &mut Captured,
                    store: &mut Option<Box<SliceFingerprints>>,
                    hour: i32,
                    rows: usize,
                    failure: Option<(&'static str, bool)>| {
            let (context, selection) =
                pack.begin_relations(relations_kit::BASE, relations_kit::GS, hour, false, true);
            assert_eq!(pack.relation_activity(), [true; 2]);
            let loaned = std::mem::replace(pack, Captured::vacant());
            assert_eq!((pack.opinion_rows_cap(), pack.tuple_rows_cap()), (0, 0));
            let mut job = RelationsJob {
                opinions: vec![crelation_status::OpinionRow::default(); rows],
                tuples: vec![crelation_status::TupleRow::default(); rows],
                selection,
                fingerprints: store.take(),
                store: Some(opinions.clone()),
                ..RelationsJob::default()
            };
            let report = match failure {
                None => copy_relations_refresh(&memory, context, &mut job),
                Some((_, walked)) => {
                    if walked {
                        copy_relations_refresh(&memory, context, &mut job);
                    }
                    RelationsCopyReport::default()
                }
            };
            *pack = loaned;
            let failure = failure.map(|(reason, _)| reason);
            let published = report.raw_ok && failure.is_none() && job.published();
            let (fingerprints, payload) = pack.join_relations(job, report, failure);
            *store = fingerprints;
            assert!(store.is_some(), "the store comes back at hour {hour}");
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
                Ok(mut job) => {
                    assert!(job.selection.is_none() && job.fingerprints.is_none());
                    assert_eq!(pack.relation_opinions.outcome, UNCAPTURED, "no typed walk");
                    assert_eq!(pack.relation_tuples.outcome, UNCAPTURED, "no typed walk");
                    job.return_opinions();
                    rows(&job.opinions, job.walk.unwrap().opinions)
                }
                Err(mut job) => {
                    job.return_opinions();
                    assert!(job.selection.is_none() && job.fingerprints.is_none());
                    let reason = failure.expect("accepted workers return their owned payload");
                    assert_eq!(pack.relation_opinions.outcome, UNCAPTURED, "no typed walk");
                    assert_eq!(pack.relation_tuples.outcome, UNCAPTURED, "no typed walk");
                    pack.fail_relations_capture(context.active, reason);
                    for outcome in [pack.relation_opinions.outcome, pack.relation_tuples.outcome] {
                        assert_eq!(outcome, Outcome::Unavailable { reason });
                    }
                    Default::default()
                }
            };
            (owners, published)
        };
        let (all, published) = hour(&mut pack, &mut store, 1, 16, None);
        assert!(published && all.len() >= 2);
        let slice = |hour: i32| {
            all.iter()
                .copied()
                .filter(|&owner| {
                    replay_data::cadence::slice_of(owner) == hour.rem_euclid(24) as u32
                })
                .collect::<std::collections::BTreeSet<_>>()
        };
        let one = *all.iter().next().unwrap();
        assert_eq!(hour(&mut pack, &mut store, 2, 16, None).0, all);
        let own = replay_data::cadence::slice_of(one) as i32 + 24;
        assert_eq!(hour(&mut pack, &mut store, own, 16, None).0, slice(own));
        let (grown, published) = hour(&mut pack, &mut store, own + 24, 0, None);
        assert!(published);
        assert_eq!(grown, slice(own));
        assert_eq!(
            hour(&mut pack, &mut store, own + 25, 16, None).0,
            slice(own + 25)
        );
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
        assert!(pack.relations_selection().contains(one) || !slice(own + 49).contains(&one));
    }

    #[test]
    fn a_relations_job_reads_new_owners_then_only_its_hours_slice() {
        let bytes = relations_kit::opinionated_fixture();
        let memory = relations_kit::memory(&bytes);
        let ctx = RelationsCopyContext {
            base: relations_kit::BASE,
            game_state: relations_kit::GS,
            active: [true; 2],
            epoch: 0,
        };
        let mut job = RelationsJob {
            store: Some(observed_store()),
            ..RelationsJob::new()
        };
        let mut unobserved = RelationsJob::new();
        let mut store = Some(Box::new(SliceFingerprints::new()));
        let mut read = |hour: i32, store: &mut Option<Box<SliceFingerprints>>, observed: bool| {
            let job = if observed { &mut job } else { &mut unobserved };
            let mut selection = SliceSelection::new();
            selection.reset(hour, false);
            job.selection = Some(selection);
            job.fingerprints = store.take();
            assert!(copy_relations_refresh(&memory, ctx, job).raw_ok);
            job.return_opinions();
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
        let (all, _) = read(1, &mut store, true);
        assert!(all.len() >= 2, "{all:?}");
        assert_eq!(read(2, &mut store, true).0, all);
        let slice = |hour: i32| {
            all.iter()
                .copied()
                .filter(|&owner| {
                    replay_data::cadence::slice_of(owner) == hour.rem_euclid(24) as u32
                })
                .collect::<std::collections::BTreeSet<_>>()
        };
        let (owners, unpublished) = read(25, &mut store, true);
        assert!(!owners.is_empty());
        assert_eq!(owners, slice(25));
        unpublished.invalidate(store.as_deref_mut().unwrap());
        let (owners, _) = read(26, &mut store, true);
        assert_eq!(owners, &slice(25) | &slice(26));
        let (owners, _) = read(27, &mut store, true);
        assert_eq!(owners, slice(27));
        let (owners, _) = read(28, &mut store, false);
        assert_eq!(owners, all);
    }

    #[test]
    fn relation_baselines_last_until_the_epoch_moves() {
        let bytes = relations_kit::opinionated_fixture();
        let mut job = RelationsJob {
            store: Some(observed_store()),
            ..RelationsJob::new()
        };
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

    #[test]
    fn observed_refreshes_read_marked_statuses() {
        let mut bytes = relations_kit::opinionated_fixture();
        let mut job = RelationsJob {
            store: Some(observed_store()),
            ..RelationsJob::new()
        };
        let opinion =
            |owner, target| relations_kit::status(owner, target) + crelation_status::OPINION;
        let initial = observed_refresh(&bytes, &mut job, 1);
        assert_eq!(initial.0, Outcome::Rows { len: 2 });
        for _ in 0..3 {
            assert_eq!(observed_refresh(&bytes, &mut job, 1), initial);
        }
        relations_kit::p32(&mut bytes, opinion(1, 3), 61);
        MARKS.with_borrow_mut(|marks| {
            marks.push(crelation_status::Mark {
                status: relations_kit::status(1, 3),
                diplomacy: relations_kit::DIPLOMACY[1],
            })
        });
        let marked = observed_refresh(&bytes, &mut job, 1);
        assert_eq!(marked.1[1].opinion, 61);
    }

    fn sliced_refresh(
        bytes: &[u8],
        job: &mut RelationsJob,
        fingerprints: &mut Option<Box<SliceFingerprints>>,
        hour: i32,
    ) -> (Vec<u32>, Vec<crelation_status::OpinionRow>) {
        let mut selection = SliceSelection::new();
        selection.reset(hour, false);
        job.selection = Some(selection);
        job.fingerprints = fingerprints.take();
        let (outcome, rows) = observed_refresh(bytes, job, 1);
        assert!(matches!(outcome, Outcome::Rows { .. }), "{outcome:?}");
        *fingerprints = job.fingerprints.take();
        let owners = job.selection.take().unwrap().read_owners().collect();
        (owners, rows)
    }

    #[test]
    fn marked_owners_are_read_the_next_hour_outside_their_slice() {
        let mut bytes = relations_kit::opinionated_fixture();
        let mut job = RelationsJob {
            store: Some(observed_store()),
            ..RelationsJob::new()
        };
        let mut fingerprints = Some(Box::new(SliceFingerprints::new()));
        let (owners, _) = sliced_refresh(&bytes, &mut job, &mut fingerprints, 24);
        assert_eq!(owners, [1, 2, 3]);
        let (owners, _) = sliced_refresh(&bytes, &mut job, &mut fingerprints, 25);
        assert_eq!(owners, [1, 2, 3]);
        assert!(
            sliced_refresh(&bytes, &mut job, &mut fingerprints, 29)
                .0
                .is_empty()
        );
        relations_kit::p32(
            &mut bytes,
            relations_kit::status(1, 3) + crelation_status::OPINION,
            61,
        );
        MARKS.with_borrow_mut(|marks| {
            marks.push(crelation_status::Mark {
                status: relations_kit::status(1, 3),
                diplomacy: relations_kit::DIPLOMACY[1],
            })
        });
        let (owners, rows) = sliced_refresh(&bytes, &mut job, &mut fingerprints, 30);
        assert_eq!(owners, [2]);
        assert!(
            rows.iter()
                .any(|row| row.owner_country == 2 && row.opinion == 61),
            "{rows:?}"
        );
        MARKS.with_borrow_mut(|marks| {
            marks.extend(std::iter::repeat_n(
                crelation_status::Mark {
                    status: relations_kit::status(0, 2),
                    diplomacy: relations_kit::DIPLOMACY[0],
                },
                9,
            ))
        });
        let (owners, _) = sliced_refresh(&bytes, &mut job, &mut fingerprints, 31);
        assert_eq!(owners, [1, 2, 3]);
        MARKS.with_borrow_mut(|marks| {
            marks.push(crelation_status::Mark {
                status: relations_kit::status(2, 0),
                diplomacy: relations_kit::DIPLOMACY[2],
            })
        });
        let (owners, _) = sliced_refresh(&bytes, &mut job, &mut fingerprints, 32);
        assert_eq!(owners, [3]);
    }

    #[test]
    fn a_short_relations_job_grows_during_its_original_worker_walk() {
        let bytes = relations_kit::opinionated_fixture();
        let memory = relations_kit::memory(&bytes);
        let mut intel = relations_kit::intel_captured(0, 0);
        let mut tiny = RelationsJob::default();
        let ctx = RelationsCopyContext {
            base: relations_kit::BASE,
            game_state: relations_kit::GS,
            active: [true; 2],
            epoch: 0,
        };
        let report = copy_relations_refresh(&memory, ctx, &mut tiny);
        assert!(report.raw_ok);
        intel.decode_relations_payload(&tiny, &mut [0; 2]);
        let direct = relations_kit::direct(&memory, 16, 16);
        relations_kit::assert_matches_direct(&intel, &direct);
        assert_eq!(direct.opinion_outcome, Outcome::Rows { len: 2 });
    }


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
            &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
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
            &(engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        cap_put(
            &mut bytes,
            CAP_DIPLO + engine_native_parts::cdiplomacy_status_layout::COUNTRY,
            &(CAP_COUNTRY as u64).to_le_bytes(),
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
            intel_scratch: 2 * engine_native_intelligence::country_intel_row::STRIDE,
            subject_names: 8,
            autonomy_names: 8,
            agency_names: 8,
            agency_upgrade_definitions: 0,
            operation_descriptions: 0,
            operative_names: 0,
            operative_mission_bytes: 0,
            operation_parts: 0,
            strategic_operative_entries: 0,
            agency_operation_entries: 0,
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
        captured.capitulation.outcome = bounded(
            cdiplomacy_status::walk_capitulation(
                memory,
                CAP_BASE,
                CAP_GAME,
                &mut captured.capitulation.rows,
                &mut Vec::new(),
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
        assert_eq!(split.capitulation.rows[0].capitulated_hour, i32::MIN);
    }

    #[test]
    fn capitulation_split_copy_admits_reserved_and_positive_country_aliases() {
        const RESERVED: usize = 0x2800;
        let mut bytes = cap_fixture();
        cap_array(
            &mut bytes,
            CAP_GAME + cgame_state::COUNTRIES,
            CAP_COUNTRIES,
            4,
        );
        cap_array(
            &mut bytes,
            CAP_GAME + cgame_state::COUNTRY_LINKS,
            CAP_LINKS,
            4,
        );
        for (slot, object) in [(0, RESERVED), (2, RESERVED), (3, CAP_COUNTRY)] {
            cap_put(
                &mut bytes,
                CAP_COUNTRIES + slot * 8,
                &(object as u64).to_le_bytes(),
            );
        }
        cap_put(&mut bytes, CAP_LINKS + 3 * 4, &1_u32.to_le_bytes());
        cap_put(
            &mut bytes,
            RESERVED,
            &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        let memory = SliceMemory {
            base: CAP_GAME,
            bytes: &bytes,
        };
        let direct = cap_direct(&memory);
        assert_eq!(direct.capitulation.outcome, Outcome::Rows { len: 1 });
        let (split, report) = cap_split(&memory);
        assert!(report.raw_ok);
        assert_eq!(split.capitulation.outcome, direct.capitulation.outcome);
        assert_eq!(split.capitulation.rows[0], direct.capitulation.rows[0]);
    }

    #[test]
    fn capitulation_split_copy_reads_owned_war_goal_recipients() {
        let mut bytes = cap_fixture();
        bytes.resize(0x5000 - CAP_GAME + 0x1000, 0);
        cap_array(
            &mut bytes,
            CAP_DIPLO + engine_native_parts::cdiplomacy_status_layout::OWNED_WARGOALS,
            0x4400,
            2,
        );
        for (index, (goal, recipient)) in
            [(0x4500_usize, 7_u32), (0x4600, 9)].into_iter().enumerate()
        {
            cap_put(&mut bytes, 0x4400 + index * 8, &(goal as u64).to_le_bytes());
            cap_put(
                &mut bytes,
                goal,
                &(engine_native::cwar_relation::goals::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            cap_put(
                &mut bytes,
                goal + engine_native::cwar_relation::goals::RECIPIENT,
                &recipient.to_le_bytes(),
            );
        }
        for (refused, expected) in [(false, Some(vec![7, 9])), (true, None)] {
            if refused {
                cap_put(&mut bytes, 0x4600, &0_u64.to_le_bytes());
            }
            let memory = SliceMemory {
                base: CAP_GAME,
                bytes: &bytes,
            };
            let mut rows = Vec::new();
            let mut recipients = Vec::new();
            let direct = cdiplomacy_status::walk_capitulation(
                &memory,
                CAP_BASE,
                CAP_GAME,
                &mut rows,
                &mut recipients,
            );
            let (split, report) = cap_split(&memory);
            assert!(report.raw_ok);
            assert_eq!(split.capitulation.outcome, direct);
            assert_eq!(split.capitulation.rows[0], rows[0]);
            let table = split.capitulation_table(split.capitulation.outcome);
            let TableView::Complete(replay_data::rows::TableRows::Capitulation(converted)) = table
            else {
                panic!("complete capitulation table");
            };
            assert_eq!(
                converted[0].war_goal_recipients,
                rows::Recorded::Value(expected)
            );
        }
    }

    #[test]
    fn capitulation_split_copy_reads_war_goals_beside_an_unreadable_diplomacy_tail() {
        struct Hole<'a> {
            memory: SliceMemory<'a>,
            hole: std::ops::Range<usize>,
        }
        impl Memory for Hole<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let end = address.saturating_add(output.len());
                address.max(self.hole.start) >= end.min(self.hole.end)
                    && self.memory.copy(address, output)
            }
        }
        let mut bytes = cap_fixture();
        bytes.resize(0x5000 - CAP_GAME + 0x1000, 0);
        cap_array(
            &mut bytes,
            CAP_DIPLO + engine_native_parts::cdiplomacy_status_layout::OWNED_WARGOALS,
            0x4400,
            1,
        );
        cap_put(&mut bytes, 0x4400, &0x4500_u64.to_le_bytes());
        cap_put(
            &mut bytes,
            0x4500,
            &(engine_native::cwar_relation::goals::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        cap_put(
            &mut bytes,
            0x4500 + engine_native::cwar_relation::goals::RECIPIENT,
            &7_u32.to_le_bytes(),
        );
        let memory = Hole {
            memory: SliceMemory {
                base: CAP_GAME,
                bytes: &bytes,
            },
            hole: CAP_DIPLO + 0x340..CAP_DIPLO + 0x360,
        };
        let mut rows = Vec::new();
        let mut recipients = Vec::new();
        let direct = cdiplomacy_status::walk_capitulation(
            &memory,
            CAP_BASE,
            CAP_GAME,
            &mut rows,
            &mut recipients,
        );
        assert_eq!(direct, Outcome::Rows { len: 1 });
        assert_eq!(rows[0].war_goals, Some(1));
        let capacities = cap_capacities();
        let mut split = Captured::with_capacities(&capacities);
        let mut job = CapitulationJob::new();
        let ctx = CapitulationCopyContext {
            base: CAP_BASE,
            game_state: CAP_GAME,
        };
        let report = copy_capitulation_refresh(&memory, ctx, &mut job);
        assert!(report.raw_ok);
        split.capitulation_raw_mode = true;
        split.decode_capitulation_payload(CAP_BASE, &job, &mut [0; 1]);
        assert_eq!(split.capitulation.outcome, direct);
        assert_eq!(split.capitulation.rows[0], rows[0]);
        let TableView::Complete(replay_data::rows::TableRows::Capitulation(converted)) =
            split.capitulation_table(split.capitulation.outcome)
        else {
            panic!("complete capitulation table");
        };
        assert_eq!(
            converted[0].war_goal_recipients,
            rows::Recorded::Value(Some(vec![7]))
        );
    }

    #[test]
    fn capitulation_split_copy_reads_only_what_the_walk_reads() {
        struct Trace<'a> {
            memory: SliceMemory<'a>,
            reads: std::cell::RefCell<std::collections::BTreeSet<(usize, usize)>>,
        }
        impl Memory for Trace<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.reads.borrow_mut().insert((address, output.len()));
                self.memory.copy(address, output)
            }
        }
        let mut bytes = cap_fixture();
        bytes.resize(0x5000 - CAP_GAME + 0x1000, 0);
        cap_array(
            &mut bytes,
            CAP_DIPLO + engine_native_parts::cdiplomacy_status_layout::OWNED_WARGOALS,
            0x4400,
            1,
        );
        cap_put(&mut bytes, 0x4400, &0x4500_u64.to_le_bytes());
        cap_put(
            &mut bytes,
            0x4500,
            &(engine_native::cwar_relation::goals::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        cap_put(
            &mut bytes,
            0x4500 + engine_native::cwar_relation::goals::RECIPIENT,
            &7_u32.to_le_bytes(),
        );
        let trace = || Trace {
            memory: SliceMemory {
                base: CAP_GAME,
                bytes: &bytes,
            },
            reads: Default::default(),
        };
        let walked = trace();
        let mut rows = Vec::new();
        let mut recipients = Vec::new();
        let direct = cdiplomacy_status::walk_capitulation(
            &walked,
            CAP_BASE,
            CAP_GAME,
            &mut rows,
            &mut recipients,
        );
        assert_eq!(direct, Outcome::Rows { len: 1 });
        assert_eq!(recipients, [7]);
        let copied = trace();
        let mut job = CapitulationJob::new();
        let ctx = CapitulationCopyContext {
            base: CAP_BASE,
            game_state: CAP_GAME,
        };
        let report = copy_capitulation_refresh(&copied, ctx, &mut job);
        assert!(report.raw_ok);
        assert_eq!(copied.reads.into_inner(), walked.reads.into_inner());
        assert!(!job.log.has_bulk);
        assert!(job.log.spans.iter().all(|span| span.ok && !span.bulk));
        let exact: u64 = job.log.spans.iter().map(|span| span.len as u64).sum();
        assert_eq!(report.raw_bytes, [exact]);
    }

    #[test]
    fn capitulation_split_copy_grows_original_shortage_before_publication() {
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
                &mut Vec::new(),
            ),
            direct.capitulation.rows.len(),
        );
        assert_eq!(direct.capitulation.outcome, Outcome::Rows { len: 1 });
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
        let direct = cap_direct(&memory);
        assert_eq!(split.capitulation.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.capitulation.outcome, direct.capitulation.outcome);
        assert_eq!(split.capitulation.rows[..1], direct.capitulation.rows[..1]);
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
}
