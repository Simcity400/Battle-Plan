//! Conversion, differencing and durable output. Nothing here runs on the game thread.

use crate::decode;
use crate::extended::characters::{CharacterConversion, CharacterShadow, CharacterShadowStats};
use crate::extended::intel::{
    IntelConversion, IntelShadow, IntelShadowStats, ScriptVariableComparison,
};
use crate::extended::lexical::{LexicalConversion, LexicalConversionStats};
use crate::history::PreparedCampaigns;
use crate::projection::{Projection, ProjectionScratch, project_table, scalar_table};
use crate::sampling::{Captured, LossBatch, LostObservation, qpc as current_qpc};
use crate::status::{
    Choice, DebugValue, LossyBytes, map as status_map, owned_sequence, sequence, status,
};
use engine_native::{
    cgame_state::Clock, memory::ReadCounts, save_allocation_counters::AllocationCounters,
    walk::Outcome,
};
use replay::{
    spool::ControlRecord, spool::FileBackend, spool::Record, spool::SegmentBackend,
    spool::SegmentedSpool,
};
use replay_data::rows::TABLE_COUNT;
use replay_data::{
    BranchId, codec::invalid_static as invalid, identity::SessionStart, identity::is_campaign_uuid,
    rows::BorderConflictRow, rows::CampaignRow, rows::ClockRow, rows::CountryLinkRow,
    rows::CountryRow, rows::DivisionRow, rows::FactionMemberRow, rows::FactionRow,
    rows::ProvinceRow, rows::ScriptVariablesRow, rows::StateRow, rows::TableId, rows::TableRows,
    rows::WarRelationRow, rows::tables,
};
use replay_frame::{
    branches::BranchStart, branches::BranchTracker, branches::Gap, frame::BaselineToken,
    frame::FrameEncoder, frame::FrameInput, frame::Metrics, frame::TableInput, frame::TableView,
};
use serde::Serialize;
#[cfg(test)]
use serde_json::{Value, json};
#[cfg(test)]
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::{
    borrow::Cow,
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

/// Baseline and borrowed conversion caches for one spool frame.
pub(crate) struct SpoolConversion<'a, 'cache> {
    pub(crate) variables_baseline: Option<(BaselineToken, &'a [ScriptVariablesRow])>,
    pub(crate) variables_scratch: &'a mut ScriptVariableComparison,
    pub(crate) characters: Option<&'a mut CharacterConversion<'cache>>,
    pub(crate) lexical: Option<&'a mut LexicalConversion<'cache>>,
    pub(crate) intel: Option<&'a mut IntelConversion<'cache>>,
    pub(crate) projection: Option<&'a mut Projection<'cache>>,
    /// The conversion helpers and the spool thread's backlog verdict.
    pub(crate) helper: Option<crate::spool_helper::Helpers<'a>>,
}

/// One hour's sliced read: the merged complete rows, or an update of the
/// cached rows the encoder shares.
enum Merged {
    Rows(TableRows),
    Update(BaselineToken, replay_data::sliced::SliceUpdate),
}

struct TableCache<T>([Option<T>; TABLE_COUNT]);
impl<T> TableCache<T> {
    fn new() -> Self {
        Self(std::array::from_fn(|_| None))
    }
    fn get(&self, table: &TableId) -> Option<&T> {
        self.0[table.index()].as_ref()
    }
    fn insert(&mut self, table: TableId, value: T) -> Option<T> {
        self.0[table.index()].replace(value)
    }
    fn remove(&mut self, table: &TableId) -> Option<T> {
        self.0[table.index()].take()
    }
    fn clear(&mut self) {
        self.0.iter_mut().for_each(|slot| *slot = None);
    }
    #[cfg(test)]
    fn contains_key(&self, table: &TableId) -> bool {
        self.get(table).is_some()
    }
}

/// Sorted owner reads per table. Updates reserve a whole batch, then merge linearly.
struct ReadHours([Vec<(u32, i32)>; TABLE_COUNT]);
impl ReadHours {
    fn new() -> Self {
        Self(std::array::from_fn(|_| Vec::new()))
    }
    fn clear(&mut self) {
        self.0.iter_mut().for_each(Vec::clear);
    }
    fn get(&self, &(table, owner): &(u32, u32)) -> Option<&i32> {
        let rows = self.0.get(table as usize)?;
        rows.binary_search_by_key(&owner, |entry| entry.0)
            .ok()
            .map(|index| &rows[index].1)
    }
    #[cfg(test)]
    fn keys(&self) -> impl Iterator<Item = (u32, u32)> {
        self.0
            .iter()
            .enumerate()
            .flat_map(|(table, rows)| rows.iter().map(move |&(owner, _)| (table as u32, owner)))
    }
}

struct Campaigns(Vec<(String, Campaign)>);
impl Campaigns {
    fn new() -> Self {
        Self(Vec::new())
    }
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    fn position(&self, id: &str) -> Result<usize, usize> {
        self.0.binary_search_by(|entry| entry.0.as_str().cmp(id))
    }
    fn contains_key(&self, id: &str) -> bool {
        self.position(id).is_ok()
    }
    fn get_mut(&mut self, id: &str) -> Option<&mut Campaign> {
        let index = self.position(id).ok()?;
        Some(&mut self.0[index].1)
    }
    fn insert(&mut self, id: String, campaign: Campaign) -> io::Result<()> {
        match self.position(&id) {
            Ok(index) => self.0[index].1 = campaign,
            Err(index) => {
                engine_allocation::reserve(&mut self.0, 1)?;
                self.0.insert(index, (id, campaign));
            }
        }
        Ok(())
    }
    fn iter(&self) -> impl Clone + Iterator<Item = (&String, &Campaign)> {
        self.0.iter().map(|(id, campaign)| (id, campaign))
    }
    fn values(&self) -> impl Clone + Iterator<Item = &Campaign> {
        self.0.iter().map(|(_, campaign)| campaign)
    }
    fn keys(&self) -> impl DoubleEndedIterator<Item = &String> {
        self.0.iter().map(|(id, _)| id)
    }
}
impl<'a> IntoIterator for &'a mut Campaigns {
    type Item = (&'a String, &'a mut Campaign);
    type IntoIter = std::iter::Map<
        std::slice::IterMut<'a, (String, Campaign)>,
        fn(&mut (String, Campaign)) -> (&String, &mut Campaign),
    >;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut().map(|(id, campaign)| (&*id, campaign))
    }
}

fn error_reason(error: &io::Error) -> Cow<'static, str> {
    match error.kind() {
        io::ErrorKind::OutOfMemory => Cow::Borrowed("collector allocation failed"),
        io::ErrorKind::InvalidData => {
            // Retained by the spool worker, after the original error is dropped.
            // Checked formatting keeps allocation failure on its existing path.
            replay_data::codec::try_format(format_args!("spool validation failed: {error}"))
                .map(Cow::Owned)
                .unwrap_or(Cow::Borrowed("collector allocation failed"))
        }
        _ => Cow::Borrowed("spool I/O failed"),
    }
}

fn committed_spool_record(segment: u32, bytes: u64, complete_hour: bool) {
    if complete_hour {
        crate::allocation::attempted_hour(None);
    }
    crate::allocation::committed(segment, bytes);
}

pub(crate) use collector_extended_support::distribution::Distribution;

/// Cumulative wall of a cost reported every hour without percentiles: its
/// storage and status cost stay fixed however many hours are recorded.
#[derive(Clone, Copy, Default)]
pub(crate) struct WallTotals {
    calls: u64,
    total_ns: u64,
    max_ns: u64,
}

impl WallTotals {
    pub(crate) fn merge(&mut self, other: Self) {
        self.calls = self.calls.saturating_add(other.calls);
        self.total_ns = self.total_ns.saturating_add(other.total_ns);
        self.max_ns = self.max_ns.max(other.max_ns);
    }

    pub(crate) fn record_ticks(&mut self, ticks: i64, frequency: u64) {
        let ns = ((ticks.max(0) as u128 * 1_000_000_000) / u128::from(frequency)) as u64;
        self.calls = self.calls.saturating_add(1);
        self.total_ns = self.total_ns.saturating_add(ns);
        self.max_ns = self.max_ns.max(ns);
    }

    pub(crate) fn status(&self) -> impl Serialize + use<> {
        status!({
            "calls": self.calls,
            "total_ns": self.total_ns,
            "mean_ns": if self.calls == 0 { 0.0 } else { self.total_ns as f64 / self.calls as f64 },
            "max_ns": self.max_ns,
        })
    }
}

struct PendingBranch {
    start: BranchStart,
    gaps: Vec<Gap>,
    /// The reads a resumed session's unobserved event interval lies between
    /// ([`ControlRecord::EventLoss`]), written with the branch's context.
    event_loss: Option<(i32, i32)>,
}

#[derive(Default)]
struct WorkSample {
    validation_refused: bool,
    validation_families: u32,
    conversion: i64,
    frame_encoding: Option<i64>,
    record_writes: Option<i64>,
}

enum CachedTable {
    Complete(Arc<TableRows>),
    Truncated { rows: TableRows, missing: u32 },
    Unavailable { reason: String },
}

impl CachedTable {
    fn input(&self) -> io::Result<TableInput> {
        Ok(match self {
            Self::Complete(rows) => TableInput::SharedComplete(Arc::clone(rows)),
            Self::Truncated { rows, missing } => TableInput::Sampled(TableView::Truncated {
                rows: rows.try_clone()?,
                missing: *missing,
            }),
            Self::Unavailable { reason } => TableInput::Sampled(TableView::Unavailable {
                reason: engine_allocation::string(reason)?,
            }),
        })
    }
}

fn lexical_conversion_status<'a>(
    statistics: impl IntoIterator<Item = &'a LexicalConversionStats>,
) -> impl Serialize {
    let mut total = LexicalConversionStats::default();
    for current in statistics {
        total.hits = total.hits.saturating_add(current.hits);
        total.misses = total.misses.saturating_add(current.misses);
    }
    status!({
        "hits": total.hits,
        "misses": total.misses,
        "counter_scope": "cumulative conversion decisions across retained campaigns, including later rejected or failed writes; genuine skipped inputs excluded",
        "cost_scope": "comparison and fallback conversion included in spool_work.conversion_packs lexical",
        "additional_native_dictionary_capacity_bytes": 0,
        "allocation_scope": "no native shadow or extra dictionary allocation; shares existing complete spool/encoder replay storage",
    })
}

fn lexical_cache(
    cache: &TableCache<CachedTable>,
    cached_branch: Option<BranchId>,
    branch: BranchId,
) -> Option<&Arc<TableRows>> {
    if cached_branch != Some(branch) {
        return None;
    }
    match cache.get(&replay_data::rows::TableId::LexicalTokens) {
        Some(CachedTable::Complete(rows)) => Some(rows),
        _ => None,
    }
}

fn intel_shadow_status<'a>(shadows: impl IntoIterator<Item = &'a IntelShadow>) -> impl Serialize {
    let mut total = IntelShadowStats::default();
    let mut campaigns = 0usize;
    for shadow in shadows {
        campaigns += 1;
        let current = shadow.statistics();
        total.hits = total.hits.saturating_add(current.hits);
        total.misses = total.misses.saturating_add(current.misses);
        total.row_capacity_bytes += current.row_capacity_bytes;
        total.replay_capacity_bytes += current.replay_capacity_bytes;
    }
    status!({
        "campaigns": campaigns,
        "table": "ScriptFlags",
        "hits": total.hits,
        "misses": total.misses,
        "retained_native_capacity_bytes": total.row_capacity_bytes,
        "retained_replay_capacity_bytes": total.replay_capacity_bytes,
        "allocation_scope": "current native row Vec capacities plus strongly retained immutable replay Vec capacities across retained campaigns; replay bytes may also be shared with encoder and are not additive to its storage; excludes Arc control blocks and allocator overhead; separate from raw/pool accounting",
        "counter_scope": "cumulative hourly conversion decisions including later rejected or failed writes; faults count as misses when conversion runs; enclosing clock failure clears shadows without conversion",
        "cost_scope": "native comparison, snapshot installation and fallback conversion included in spool_work.conversion_packs intel",
    })
}

fn character_shadow_status<'a>(
    shadows: impl IntoIterator<Item = &'a CharacterShadow>,
) -> impl Serialize {
    let mut totals = [CharacterShadowStats::default(); 4];
    let mut campaigns = 0usize;
    for shadow in shadows {
        campaigns += 1;
        for (total, current) in totals.iter_mut().zip(shadow.statistics()) {
            total.hits = total.hits.saturating_add(current.hits);
            total.misses = total.misses.saturating_add(current.misses);
            total.row_capacity_bytes += current.row_capacity_bytes;
            total.arena_capacity_bytes += current.arena_capacity_bytes;
        }
    }
    let bytes: usize = totals
        .iter()
        .map(|table| table.row_capacity_bytes + table.arena_capacity_bytes)
        .sum();
    let tables = sequence(
        [
            "Characters",
            "CountryLeaderRoles",
            "AdvisorRoles",
            "UnitLeaderTraits",
        ]
        .into_iter()
        .zip(totals)
        .map(|(name, table)| {
            status!({
                "name": name,
                "hits": table.hits,
                "misses": table.misses,
                "row_capacity_bytes": table.row_capacity_bytes,
                "arena_capacity_bytes": table.arena_capacity_bytes,
            })
        }),
    );
    status!({
        "allocation_scope": "current native row/text Vec capacities across retained campaigns; excludes shared replay rows, Arc control blocks and allocator overhead; separate from raw/pool accounting",
        "counter_scope": "cumulative conversion decisions across retained campaigns, including later rejected or failed writes; genuine skipped inputs excluded; non-skip fallback/fault inputs count as misses",
        "campaigns": campaigns,
        "retained_capacity_bytes": bytes,
        "tables": tables,
    })
}

fn character_cache(
    cache: &TableCache<CachedTable>,
    cached_branch: Option<BranchId>,
    branch: BranchId,
) -> [Option<&Arc<TableRows>>; 4] {
    crate::extended::characters::SHADOW_TABLES.map(|table| {
        if cached_branch != Some(branch) {
            return None;
        }
        match cache.get(&table) {
            Some(CachedTable::Complete(rows)) => Some(rows),
            _ => None,
        }
    })
}

#[derive(Serialize)]
struct ValidationRefusal {
    table: String,
    column: Option<String>,
    reason: String,
    detail: String,
    first_hour: i32,
    last_hour: i32,
    count: u64,
}

#[derive(Clone, Default)]
pub(crate) struct ValidationRefusals(Arc<std::sync::Mutex<Vec<ValidationRefusal>>>);
impl Serialize for ValidationRefusals {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .serialize(serializer)
    }
}

fn validation_refusal(
    failures: &mut Vec<ValidationRefusal>,
    directory: &Path,
    table: TableId,
    hour: i32,
    error: io::Error,
) -> io::Result<String> {
    let detail = replay_data::codec::try_format(format_args!("{error}"))?;
    let (column, reason) = detail
        .split_once(" column ")
        .and_then(|(_, rest)| rest.split_once(": "))
        .map_or(
            (
                None,
                detail.split_once(": ").map_or(detail.as_str(), |(_, r)| r),
            ),
            |(column, reason)| (Some(column), reason),
        );
    let table_name = replay_data::codec::try_format(format_args!("{table:?}"))?;
    if let Some(failure) = failures
        .iter_mut()
        .find(|f| f.table == table_name && f.column.as_deref() == column && f.reason == reason)
    {
        failure.last_hour = hour;
        failure.count += 1;
    } else {
        engine_allocation::reserve(failures, 1)?;
        failures.push(ValidationRefusal {
            table: table_name,
            column: column.map(engine_allocation::string).transpose()?,
            reason: engine_allocation::string(reason)?,
            detail: engine_allocation::string(&detail)?,
            first_hour: hour,
            last_hour: hour,
            count: 1,
        });
        crate::hook_failures::log(
            directory.parent().unwrap_or(directory),
            format_args!("spool validation refused at hour {hour}: {detail}"),
        )?;
    }
    replay_data::codec::try_format(format_args!("spool validation failed: {detail}"))
}

struct Campaign<B: SegmentBackend = FileBackend> {
    validation_refusals: ValidationRefusals,
    world_generation: Option<u64>,
    directory: PathBuf,
    session: SessionStart,
    tracker: BranchTracker,
    /// A reattached process session's recorded tail (branch, last hour)
    /// until its first observation: event completeness was lost for part of
    /// the interval since the read of that hour, while the recorder was
    /// detached.
    resumed: Option<(BranchId, i32)>,
    encoder: FrameEncoder,
    spool: SegmentedSpool<B>,
    disk_branch: Option<BranchId>,
    last_written: Option<i32>,
    pending: Vec<PendingBranch>,
    staggered_cache: TableCache<CachedTable>,
    staggered_branch: Option<BranchId>,
    /// Paired with the latest CountryIntel cache, even before durable publication.
    country_intel_sample: Option<replay_data::rows::ReservedCountryIntelSample>,
    /// Acquisition hour of the latest CountryIntel cache when it is complete,
    /// the sample certified spotting navy lanes cite.
    country_intel_hour: Option<i32>,
    /// The latest complete rows of each sliced table on `sliced_branch`.
    sliced_cache: TableCache<Arc<TableRows>>,
    /// The read hour of each (sliced table, owner) in `sliced_cache`.
    sliced_reads: ReadHours,
    sliced_branch: Option<BranchId>,
    /// This frame's slice updates of `sliced_cache` rows, held until the
    /// frame's write succeeds (the encoder then holds the updated rows) or
    /// fails (the cache then takes the merged rows).
    pending_slices: Vec<(TableId, Arc<replay_data::sliced::SliceUpdate>)>,
    /// What this campaign's commits no longer hold, for freeing elsewhere.
    retired: Vec<replay_frame::frame::Retired>,
    /// Each production line's output on the current branch.
    line_output: crate::extended::line_output::Totals,
    operation_outcomes: crate::extended::operation_outcomes::Ledger,
    map_events: crate::extended::map_events::Ledger,
    politics: crate::extended::politics::Ledger,
    /// Each construction line's output on the current branch.
    construction_ic: crate::extended::construction_ic::Totals,
    fuel_awards: crate::extended::fuel_awards::Windows,
    variables_scratch: ScriptVariableComparison,
    character_shadow: CharacterShadow,
    intel_shadow: IntelShadow,
    projection_scratch: ProjectionScratch,
    lexical_conversion: LexicalConversionStats,
    _history_file: Option<File>,
}

impl Campaign {
    fn open(
        root: &Path,
        history: &mut PreparedCampaigns,
        id: &str,
        session: &SessionStart,
    ) -> io::Result<Self> {
        let directory = root.join(id);
        // An attach has its own root. Reusing an unfinalized root would conceal
        // history from the tracker and could assign existing branch ids again.
        if directory.exists()
            && fs::read_dir(&directory)
                .map_err(engine_allocation::allocation_error)?
                .any(|entry| {
                    entry.is_ok_and(|entry| {
                        entry.file_name().to_string_lossy().starts_with("spool.")
                    })
                })
        {
            return Err(invalid(
                "attach output already contains spool segments; use a fresh attach directory",
            ));
        }
        let mut spool = SegmentedSpool::new(FileBackend::new(&directory)?);
        if engine_allocation::capture_active() {
            spool.set_commit_hook(committed_spool_record);
            spool.set_write_start_hook(crate::allocation::record_start);
        }
        let world_generation = history.campaign_world_generation(id);
        let (tracker, history_file) = history.take(id, session)?;
        Ok(Self {
            validation_refusals: ValidationRefusals::default(),
            directory,
            session: session.try_clone()?,
            resumed: tracker.last_observation(),
            tracker,
            encoder: FrameEncoder::new(),
            spool,
            disk_branch: None,
            world_generation,
            last_written: None,
            pending: Vec::new(),
            staggered_cache: TableCache::new(),
            staggered_branch: None,
            country_intel_sample: None,
            country_intel_hour: None,
            sliced_cache: TableCache::new(),
            sliced_reads: ReadHours::new(),
            sliced_branch: None,
            pending_slices: Vec::new(),
            retired: Vec::new(),
            line_output: Default::default(),
            operation_outcomes: Default::default(),
            map_events: Default::default(),
            politics: Default::default(),
            construction_ic: Default::default(),
            fuel_awards: Default::default(),
            variables_scratch: ScriptVariableComparison::default(),
            character_shadow: CharacterShadow::default(),
            intel_shadow: IntelShadow::default(),
            projection_scratch: ProjectionScratch::default(),
            lexical_conversion: LexicalConversionStats::default(),
            _history_file: history_file,
        })
    }
}

impl<B: SegmentBackend> Campaign<B> {
    fn pending_branch(&mut self, branch: BranchId) -> io::Result<&mut PendingBranch> {
        if self
            .pending
            .last()
            .is_none_or(|pending| pending.start.id != branch)
        {
            let start = self
                .tracker
                .branch_start(branch)
                .expect("observed branch exists")
                .clone();
            engine_allocation::reserve(&mut self.pending, 1)?;
            self.pending.push(PendingBranch {
                start,
                gaps: Vec::new(),
                event_loss: None,
            });
        }
        Ok(self.pending.last_mut().unwrap())
    }

    fn gap(
        &mut self,
        branch: BranchId,
        first_hour: i32,
        last_hour: i32,
        reason: String,
    ) -> io::Result<()> {
        let gaps = &mut self.pending_branch(branch)?.gaps;
        if let Some(previous) = gaps.last_mut()
            && previous.reason == reason
            && previous.last_hour.checked_add(1) == Some(first_hour)
        {
            previous.last_hour = last_hour;
        } else {
            engine_allocation::reserve(gaps, 1)?;
            gaps.push(Gap {
                branch,
                first_hour,
                last_hour,
                reason,
            });
        }
        Ok(())
    }

    fn observe(
        &mut self,
        loaded: Option<i32>,
        hour: i32,
        signal: Option<i64>,
    ) -> io::Result<BranchId> {
        let gaps = self
            .pending
            .last()
            .filter(|pending| Some(pending.start.id) == self.tracker.current_branch())
            .map_or(&[][..], |pending| pending.gaps.as_slice());
        let observation = self.tracker.observe(loaded, hour, signal, gaps)?;
        // A resumed session's first observation that continues its recorded
        // branch ends an interval whose event completeness was lost while the
        // recorder was detached, between the read of `after` and this attach's
        // hooks. A new branch starts its own first interval instead.
        let event_loss = self
            .resumed
            .take()
            .filter(|&(branch, _)| observation.start.is_none() && observation.branch == branch)
            .map(|(_, after)| (after, hour));
        let pending = self.pending_branch(observation.branch)?;
        if event_loss.is_some() {
            pending.event_loss = event_loss;
        }
        Ok(observation.branch)
    }

    /// Replace the rows of the owners this frame read in each sliced table's
    /// latest complete rows on this branch. A full read replaces everything.
    /// When those rows are the encoder's shared baseline for this hour, the
    /// frame carries the read as a slice update of them instead of a merged
    /// copy: the encoder visits and replaces only the read owners' rows, and
    /// the cache follows the write's outcome ([`Self::write`]).
    /// Without earlier complete rows a slice cannot stand alone: the table is
    /// unavailable and the next hour must read every slice (returns true).
    /// A failed acquisition publishes its failure and keeps earlier rows, whose
    /// read hours show their age once a later slice completes the table.
    /// SliceReads is assembled here: the read hour of every owner with rows
    /// in each owned sliced table, and of every owner a complete read covered
    /// that returned none, replaced only for the owners read now.
    ///
    /// `walked` is whether the walk published rows for a table. Such a table
    /// that conversion or merging made unavailable was never published although
    /// the game thread saw a successful read, so its family is returned in
    /// the second value (bit per [`crate::slicing::FAMILIES`] index) for the
    /// game thread to forget ([`crate::slicing::SliceFingerprints::forget`]).
    /// Frames already captured before that forget took effect trusted the
    /// unpublished read, so the family's earlier rows are dropped here too:
    /// until a full read replaces them, no hour merges rows from before the
    /// failure, and a full refresh is requested.
    fn try_merge_sliced_inputs<'a>(
        &mut self,
        source: &mut FrameInput,
        selection: impl Fn(TableId) -> Option<&'a crate::slicing::SliceSelection>,
        walked: impl Fn(TableId) -> bool,
    ) -> io::Result<(bool, u32)> {
        // An update whose write never ran leaves the cache as a failure does.
        self.settle_failed_slices();
        if self.sliced_branch != Some(source.branch) {
            self.sliced_cache.clear();
            self.sliced_reads.clear();
            self.sliced_branch = Some(source.branch);
        }
        let force = !self.spool.is_open();
        let mut retry = false;
        let mut unpublished = 0_u32;
        let mut frame_reads: [Option<Vec<(u32, i32)>>; TABLE_COUNT] = std::array::from_fn(|_| None);
        let hour = source.hour;
        for table in TableId::ALL {
            if !replay_data::cadence::is_sliced(table) {
                continue;
            }
            let input = &mut source.tables[table.index()];
            if walked(table)
                && matches!(input, TableInput::Sampled(TableView::Unavailable { reason })
                    if *reason != replay_data::cadence::SKIPPED_REASON)
                && let Some(family) = crate::slicing::family_of(table)
            {
                unpublished |= 1 << family;
            }
            let Some(selection) = selection(table) else {
                *input = TableInput::Sampled(unavailable("sliced table has no owner selection"));
                continue;
            };
            let captured = |owner| selection.contains(owner);
            let live = |owner| selection.live(owner);
            match input {
                TableInput::Sampled(TableView::Complete(rows)) => {
                    let fresh = std::mem::replace(rows, TableRows::empty(table));
                    let update = (!selection.force_full
                        && !replay_data::sliced::is_keyed(table)
                        && !replay_data::sliced::replaces_whole(&fresh))
                    .then(|| self.sliced_cache.get(&table))
                    .flatten()
                    .and_then(|previous| {
                        self.encoder
                            .slice_baseline(table, source.branch, hour, force)
                            .filter(|(_, baseline)| Arc::ptr_eq(baseline, previous))
                            .map(|(token, _)| (token, previous))
                    });
                    let merged = if let Some((token, previous)) = update {
                        match replay_data::sliced::SliceUpdate::new(previous, fresh, captured, live)
                        {
                            Ok(update) => Merged::Update(token, update),
                            Err(error) if error.kind() == io::ErrorKind::OutOfMemory => {
                                return Err(error);
                            }
                            Err(_) => {
                                *input =
                                    TableInput::Sampled(unavailable("sliced table merge failed"));
                                if let Some(family) = crate::slicing::family_of(table) {
                                    unpublished |= 1 << family;
                                }
                                retry = true;
                                continue;
                            }
                        }
                    } else if selection.force_full || replay_data::sliced::replaces_whole(&fresh) {
                        Merged::Rows(fresh)
                    } else if let Some(previous) = self.sliced_cache.get(&table) {
                        match replay_data::sliced::merge_slice(previous, fresh, captured, live) {
                            Ok(merged) => Merged::Rows(merged),
                            Err(error) if error.kind() == io::ErrorKind::OutOfMemory => {
                                return Err(error);
                            }
                            Err(_) => {
                                *input =
                                    TableInput::Sampled(unavailable("sliced table merge failed"));
                                if let Some(family) = crate::slicing::family_of(table) {
                                    unpublished |= 1 << family;
                                }
                                retry = true;
                                continue;
                            }
                        }
                    } else {
                        *input = TableInput::Sampled(unavailable(
                            "sliced table has no earlier complete rows on this branch; full refresh requested",
                        ));
                        retry = true;
                        continue;
                    };
                    if !replay_data::sliced::is_keyed(table) {
                        let owners = match &merged {
                            Merged::Rows(rows) => replay_data::sliced::try_owners(rows)?,
                            // Unread live owners already have their provenance
                            // in `previous`, including owners with no rows. The
                            // final loop below keeps it. Only the fresh owners
                            // need a new sample hour; do not rescan the complete
                            // baseline merely to enumerate its carried owners.
                            Merged::Update(_, update) => {
                                replay_data::sliced::try_owners(update.rows())?
                            }
                        };
                        let previous = &self.sliced_reads.0[table.index()];
                        let capacity = owners
                            .len()
                            .checked_add(selection.read_owners().count())
                            .and_then(|count| count.checked_add(previous.len()))
                            .ok_or_else(engine_allocation::oom)?;
                        let mut reads = engine_allocation::vec(capacity)?;
                        for &owner in &owners {
                            let read = if selection.force_full || captured(owner) {
                                Some(hour)
                            } else {
                                self.sliced_reads
                                    .get(&(table.index() as u32, owner))
                                    .copied()
                            };
                            if let Some(read) = read {
                                reads.push((owner, read));
                            }
                        }
                        for owner in selection.read_owners() {
                            if owners.binary_search(&owner).is_err() {
                                reads.push((owner, hour));
                            }
                        }
                        for &(owner, read) in previous {
                            if !selection.force_full
                                && !captured(owner)
                                && live(owner)
                                && owners.binary_search(&owner).is_err()
                            {
                                reads.push((owner, read));
                            }
                        }
                        reads.sort_unstable_by_key(|entry| entry.0);
                        reads.dedup_by_key(|entry| entry.0);
                        self.sliced_reads.0[table.index()] = reads;
                    }
                    match merged {
                        Merged::Rows(merged) => {
                            let merged = Arc::new(merged);
                            self.sliced_cache.insert(table, Arc::clone(&merged));
                            *input = TableInput::SharedComplete(merged);
                        }
                        Merged::Update(token, update) => {
                            let update = Arc::new(update);
                            engine_allocation::reserve(&mut self.pending_slices, 1)?;
                            self.pending_slices.push((table, Arc::clone(&update)));
                            *input = TableInput::SliceUpdate { token, update };
                        }
                    }
                }
                TableInput::Sampled(TableView::Truncated { rows, missing }) => {
                    let missing = *missing;
                    let fresh = std::mem::replace(rows, TableRows::empty(table));
                    let returned = replay_data::sliced::try_owners(&fresh)?;
                    let replaced =
                        |owner: u32| captured(owner) && returned.binary_search(&owner).is_ok();
                    if !replay_data::sliced::is_keyed(table) {
                        let previous = &self.sliced_reads.0[table.index()];
                        let capacity = previous
                            .len()
                            .checked_add(returned.len())
                            .ok_or_else(engine_allocation::oom)?;
                        let mut reads = engine_allocation::vec(capacity)?;
                        let mut old = previous.iter().peekable();
                        for &owner in returned.iter().filter(|&&owner| captured(owner)) {
                            while old.peek().is_some_and(|entry| entry.0 < owner) {
                                reads.push(*old.next().unwrap());
                            }
                            if old.peek().is_some_and(|entry| entry.0 == owner) {
                                old.next();
                            }
                            reads.push((owner, hour));
                        }
                        reads.extend(old.copied());
                        frame_reads[table.index()] = Some(reads);
                    }
                    if let Some(previous) = self.sliced_cache.get(&table) {
                        *input = TableInput::Sampled(
                            match replay_data::sliced::merge_slice(previous, fresh, replaced, live)
                            {
                                Ok(rows) => TableView::Truncated { rows, missing },
                                Err(error) if error.kind() == io::ErrorKind::OutOfMemory => {
                                    return Err(error);
                                }
                                Err(_) => {
                                    if let Some(family) = crate::slicing::family_of(table) {
                                        unpublished |= 1 << family;
                                    }
                                    unavailable("sliced table merge failed")
                                }
                            },
                        );
                    } else {
                        *input = TableInput::Sampled(unavailable(
                            "sliced table has no earlier complete rows on this branch; full refresh requested",
                        ));
                        retry = true;
                    }
                }
                _ => {}
            }
        }
        if unpublished != 0 {
            for table in TableId::ALL {
                if crate::slicing::family_of(table)
                    .is_some_and(|family| unpublished & (1 << family) != 0)
                {
                    self.sliced_cache.remove(&table);
                    self.pending_slices.retain(|(pending, _)| *pending != table);
                    let previous = std::mem::take(&mut self.sliced_reads.0[table.index()]);
                    if frame_reads[table.index()].is_none() {
                        frame_reads[table.index()] = Some(previous);
                    }
                }
            }
            retry = true;
        }
        let count = frame_reads
            .iter()
            .zip(&self.sliced_reads.0)
            .try_fold(0_usize, |count, (view, base)| {
                count.checked_add(view.as_ref().unwrap_or(base).len())
            })
            .ok_or_else(engine_allocation::oom)?;
        let mut reads = engine_allocation::vec(count)?;
        for (table, (view, base)) in frame_reads.iter().zip(&self.sliced_reads.0).enumerate() {
            reads.extend(
                view.as_ref()
                    .unwrap_or(base)
                    .iter()
                    .map(|&(owner, sample_hour)| replay_data::rows::SliceReadRow {
                        table: table as u32,
                        owner,
                        sample_hour,
                    }),
            );
        }
        source.tables[TableId::SliceReads.index()] =
            TableInput::SharedComplete(Arc::new(TableRows::SliceReads(reads)));
        Ok((retry, unpublished))
    }

    /// Before this frame's commit: the producer lets go of the rows its slice
    /// updates replace, so the encoder replaces them in place. Returns the
    /// tables whose cache [`Self::adopt_slices`] refills.
    fn release_slices(&mut self) -> [bool; TABLE_COUNT] {
        let mut released = [false; TABLE_COUNT];
        for (table, _) in self.pending_slices.drain(..) {
            self.sliced_cache.remove(&table);
            released[table.index()] = true;
        }
        released
    }

    /// After this frame's commit: the cache holds the encoder's updated rows.
    fn adopt_slices(&mut self, released: [bool; TABLE_COUNT]) {
        for table in TableId::ALL
            .into_iter()
            .filter(|table| released[table.index()])
        {
            if let Some(rows) = self.encoder.shared_baseline(table) {
                self.sliced_cache.insert(table, Arc::clone(rows));
            }
        }
    }

    /// A frame whose slice updates were not committed: the cache takes the
    /// merged rows they describe, as it holds the latest merge whatever the
    /// write's outcome.
    fn settle_failed_slices(&mut self) {
        for (table, update) in std::mem::take(&mut self.pending_slices) {
            let Some(previous) = self.sliced_cache.get(&table) else {
                continue;
            };
            match update.merged(previous) {
                Ok(merged) => {
                    self.sliced_cache.insert(table, Arc::new(merged));
                }
                // Allocation failure ends the capture; nothing more is merged.
                Err(_) => {
                    self.sliced_cache.remove(&table);
                }
            }
        }
    }

    #[cfg(test)]
    fn merge_sliced_inputs<'a>(
        &mut self,
        source: &mut FrameInput,
        selection: impl Fn(TableId) -> Option<&'a crate::slicing::SliceSelection>,
        walked: impl Fn(TableId) -> bool,
    ) -> (bool, u32) {
        self.try_merge_sliced_inputs(source, selection, walked)
            .unwrap()
    }

    /// Add this frame's production ticks to the branch's per-line totals and
    /// publish ProductionLineOutput beside the frame's ProductionLines. Before
    /// the staggered carry, so an off-slot hour's skip is carried.
    fn assemble_line_output(&mut self, source: &mut FrameInput, captured: &Captured) {
        self.line_output.assemble(
            source.branch,
            source.hour,
            captured.extended.line_output(),
            &mut source.tables,
        );
    }

    /// Add this frame's construction output to the branch's per-line totals and
    /// publish ConstructionLineOutput beside the frame's construction queues.
    /// Before the staggered carry, so an off-slot hour's skip is carried.
    fn assemble_construction_ic(&mut self, source: &mut FrameInput, captured: &Captured) {
        self.construction_ic.assemble(
            source.branch,
            source.hour,
            captured.extended.construction_ic(),
            &mut source.tables,
        );
    }

    // Spool-internal carry-forward keeps format-neutral byte-identical output.
    fn try_apply_staggered_inputs(
        &mut self,
        source: &mut FrameInput,
        intel_sample: Option<Result<replay_data::rows::ReservedCountryIntelSample, &'static str>>,
    ) -> io::Result<[bool; TABLE_COUNT]> {
        if self.staggered_branch != Some(source.branch) {
            self.staggered_cache.clear();
            self.country_intel_sample = None;
            self.country_intel_hour = None;
            self.staggered_branch = Some(source.branch);
        }
        let mut skipped = [false; TABLE_COUNT];
        for table in replay_data::rows::TableId::ALL {
            let index = table.index();
            let may_carry = replay_data::cadence::is_staggered(table);
            let conversion_cache = table == TableId::LexicalTokens
                || crate::extended::characters::SHADOW_TABLES.contains(&table);
            if !may_carry && !conversion_cache {
                continue;
            }
            let input = &mut source.tables[index];
            let is_skip = matches!(
                &*input,
                TableInput::Sampled(TableView::Unavailable { reason })
                if reason == replay_data::cadence::SKIPPED_REASON
            );
            if is_skip {
                skipped[index] = true;
                if !may_carry {
                    *input = TableInput::Sampled(unavailable("hourly table was not acquired"));
                    self.staggered_cache.insert(
                        table,
                        CachedTable::Unavailable {
                            reason: engine_allocation::string("hourly table was not acquired")?,
                        },
                    );
                    continue;
                }
                if self
                    .encoder
                    .can_carry(table, source.branch, source.hour, !self.spool.is_open())
                {
                    // Keep the small marker. The encoder already owns a
                    // validated complete baseline; no row clone or recheck.
                } else if let Some(cached) = self.staggered_cache.get(&table) {
                    *input = cached.input()?;
                } else {
                    *input = TableInput::Sampled(TableView::Unavailable {
                        reason: engine_allocation::string(
                            "staggered cache empty on new branch; full refresh required",
                        )?,
                    });
                }
            } else {
                let cached = match input {
                    TableInput::Sampled(TableView::Complete(rows)) => {
                        let rows = Arc::new(std::mem::replace(rows, TableRows::empty(table)));
                        *input = TableInput::SharedComplete(Arc::clone(&rows));
                        CachedTable::Complete(rows)
                    }
                    TableInput::SharedComplete(rows) => CachedTable::Complete(Arc::clone(rows)),
                    TableInput::Sampled(TableView::Truncated { rows, missing }) => {
                        CachedTable::Truncated {
                            rows: rows.try_clone()?,
                            missing: *missing,
                        }
                    }
                    TableInput::Sampled(TableView::Unavailable { reason }) => {
                        CachedTable::Unavailable {
                            reason: engine_allocation::string(reason)?,
                        }
                    }
                    // Only hourly tables are compared natively.
                    TableInput::ComparedEqual(_)
                    | TableInput::ComparedChanges { .. }
                    | TableInput::SliceUpdate { .. } => CachedTable::Unavailable {
                        reason: engine_allocation::string(
                            "a native comparison cannot supply a shared recovery snapshot",
                        )?,
                    },
                };
                // Keep the latest sample even if encoding or writing fails.
                // Recovery resets the encoder, but can snapshot this allocation
                // on an off hour without retaining a native capture buffer.
                if table == TableId::CountryIntel {
                    self.country_intel_hour =
                        matches!(&cached, CachedTable::Complete(_)).then_some(source.hour);
                    self.country_intel_sample = if matches!(&cached, CachedTable::Complete(_)) {
                        intel_sample
                            .and_then(Result::ok)
                            .filter(|sample| sample.sample_hour == source.hour)
                    } else {
                        None
                    };
                }
                self.staggered_cache.insert(table, cached);
            }
        }
        Ok(skipped)
    }

    /// RCN is acquired hourly, while this certificate follows the exact daily
    /// CountryIntel sample selected above. Never fill a refused naming prefix.
    fn assemble_reserved_intel_sample(&self, source: &mut FrameInput) {
        let TableInput::Sampled(TableView::Complete(TableRows::ReservedCountryNaming(rows))) =
            &mut source.tables[TableId::ReservedCountryNaming.index()]
        else {
            return;
        };
        if let [row] = rows.as_mut_slice() {
            row.country_intel_sample =
                replay_data::rows::Recorded::Value(self.country_intel_sample);
        }
    }

    #[cfg(test)]
    fn apply_staggered_inputs(&mut self, source: &mut FrameInput) -> [bool; TABLE_COUNT] {
        self.try_apply_staggered_inputs(source, None).unwrap()
    }

    #[cfg(test)]
    fn apply_staggered_cache(
        &mut self,
        source: &mut replay_frame::frame::DecodedFrame,
    ) -> [bool; TABLE_COUNT] {
        let mut input = source.clone().into();
        let skipped = self.apply_staggered_inputs(&mut input);
        *source = sampled_frame(input);
        skipped
    }

    fn lost(&mut self, loss: &LostObservation, suppress_origin: bool) -> io::Result<()> {
        if loss.first_hour > loss.last_hour
            || i64::from(loss.last_hour) - i64::from(loss.first_hour) + 1
                != i64::try_from(loss.count).unwrap_or(-1)
        {
            return Err(invalid(
                "queue loss witness has inconsistent hours and count",
            ));
        }
        let origin = if suppress_origin || self.tracker.current_branch().is_none() {
            None
        } else {
            loss.loaded_hour
        };
        crate::allocation::set_context(crate::allocation::Context {
            campaign_id: loss.campaign_id.as_bytes().try_into().ok(),
            branch: None,
            load_signal: loss.load_signal,
            failed_hour: Some(loss.first_hour),
        });
        let branch = self.observe(origin, loss.first_hour, loss.load_signal)?;
        crate::allocation::set_context(crate::allocation::Context {
            campaign_id: loss.campaign_id.as_bytes().try_into().ok(),
            branch: Some(branch),
            load_signal: loss.load_signal,
            failed_hour: Some(loss.first_hour),
        });
        if loss.first_hour != loss.last_hour {
            self.gap(
                branch,
                loss.first_hour,
                loss.last_hour - 1,
                engine_allocation::string("collector queue overflow")?,
            )?;
            crate::allocation::set_context(crate::allocation::Context {
                campaign_id: loss.campaign_id.as_bytes().try_into().ok(),
                branch: None,
                load_signal: loss.load_signal,
                failed_hour: Some(loss.last_hour),
            });
            self.observe(None, loss.last_hour, loss.load_signal)?;
            crate::allocation::set_context(crate::allocation::Context {
                campaign_id: loss.campaign_id.as_bytes().try_into().ok(),
                branch: Some(branch),
                load_signal: loss.load_signal,
                failed_hour: Some(loss.last_hour),
            });
            self.gap(
                branch,
                loss.last_hour,
                loss.last_hour,
                engine_allocation::string("collector queue overflow")?,
            )?;
        } else {
            self.gap(
                branch,
                loss.first_hour,
                loss.last_hour,
                engine_allocation::string("collector queue overflow")?,
            )?;
        }
        Ok(())
    }

    fn context(&self, restarting: bool, extra: usize) -> io::Result<Vec<Record>> {
        let count = self
            .pending
            .iter()
            .try_fold(usize::from(restarting) + extra, |count, pending| {
                count
                    .checked_add(1)?
                    .checked_add(pending.gaps.len())?
                    .checked_add(usize::from(pending.event_loss.is_some()))
            })
            .ok_or_else(engine_allocation::oom)?;
        let mut records = engine_allocation::vec(count)?;
        let mut active = self.disk_branch;
        if restarting {
            records.push(Record::SessionStart(self.session.try_clone()?));
            active = None;
        }
        for pending in &self.pending {
            if active != Some(pending.start.id) {
                records.push(Record::BranchStart(pending.start.clone()));
                active = Some(pending.start.id);
            }
            for gap in &pending.gaps {
                records.push(Record::Gap(gap.try_clone()?));
            }
            if let Some((after_hour, hour)) = pending.event_loss {
                records.push(Record::Control(ControlRecord::EventLoss {
                    session: self.session.id,
                    branch: pending.start.id,
                    after_hour,
                    hour,
                }));
            }
        }
        Ok(records)
    }

    #[cfg(test)]
    fn write(
        &mut self,
        source: impl Into<FrameInput>,
        qpc: i64,
        signal: Option<i64>,
        unknown: u64,
        work: &mut WorkSample,
    ) -> io::Result<()> {
        self.write_with(source.into(), qpc, signal, unknown, work, None)
    }

    /// [`Self::write`], encoding the frame's larger tables on the spool
    /// thread and `helpers` at once.
    fn write_with(
        &mut self,
        source: FrameInput,
        qpc: i64,
        signal: Option<i64>,
        unknown: u64,
        work: &mut WorkSample,
        helpers: Option<crate::spool_helper::Helpers<'_>>,
    ) -> io::Result<()> {
        let _details = replay_data::codec::error_details_scope();
        let (branch, hour) = (source.branch, source.hour);
        let held_line_output = crate::extended::line_output::holds_publication(
            &source.tables[TableId::ProductionLineOutput.index()],
        );
        let held_construction_ic = crate::extended::construction_ic::holds_publication(
            &source.tables[TableId::ConstructionLineOutput.index()],
        );
        let result = self.write_inner(source, qpc, signal, unknown, work, helpers);
        if result.is_ok() {
            self.line_output.written(held_line_output);
            self.construction_ic.written(held_construction_ic);
        }
        if let Err(error) = &result {
            if let Some((segment, bytes)) = self.spool.committed_watermark() {
                crate::allocation::committed(segment, bytes);
            }
            // Codec/validation errors are failures too. Closing also prevents a
            // partially published context from being reused after such an error.
            let _span = crate::spool_phases::SPOOL.span(crate::spool_phases::Phase::Write);
            let close_started = current_qpc();
            let _ = self.spool.finish_segment();
            *work.record_writes.get_or_insert(0) += current_qpc() - close_started;
            self.disk_branch = None;
            if error.kind() != io::ErrorKind::OutOfMemory && !crate::allocation::failed() {
                self.encoder = FrameEncoder::new();
                self.settle_failed_slices();
                self.gap(
                    branch,
                    hour,
                    hour,
                    engine_allocation::string("spool write failed")?,
                )?;
            }
        }
        result
    }

    fn write_inner(
        &mut self,
        source: FrameInput,
        qpc: i64,
        signal: Option<i64>,
        unknown: u64,
        work: &mut WorkSample,
        helpers: Option<crate::spool_helper::Helpers<'_>>,
    ) -> io::Result<()> {
        let (branch, hour) = (source.branch, source.hour);
        let restarting = !self.spool.is_open();
        if restarting {
            self.encoder = FrameEncoder::new();
        }
        let encoding_started = current_qpc();
        let span = crate::spool_phases::SPOOL.span(crate::spool_phases::Phase::Encode);
        let frame = {
            let _phase = crate::allocation::enter(crate::allocation::Phase::Encode);
            let mut segments = match helpers {
                Some(helpers) => crate::spool_segments::encode(
                    &self.encoder,
                    &source,
                    (branch, hour, restarting),
                    helpers,
                ),
                None => std::array::from_fn(|_| None),
            };
            self.encoder
                .prepare_contained(source, restarting, &mut segments, |table, error| {
                    let reason = validation_refusal(
                        &mut self
                            .validation_refusals
                            .0
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner),
                        &self.directory,
                        table,
                        hour,
                        error,
                    )?;
                    // No refused table may survive in a producer's cached rows.
                    self.staggered_cache.remove(&table);
                    self.sliced_cache.remove(&table);
                    self.pending_slices.retain(|(id, _)| *id != table);
                    work.validation_refused = true;
                    if let Some(family) = crate::slicing::family_of(table) {
                        work.validation_families |= 1 << family;
                    }
                    Ok(reason)
                })
        };
        work.frame_encoding = Some(current_qpc() - encoding_started);
        let mut prepared = frame?;
        let span = span.switch(crate::spool_phases::Phase::Write);
        let write_started = current_qpc();
        let result = (|| {
            let mut context = self.context(restarting, 2)?;
            if unknown != 0 {
                context.push(Record::Control(ControlRecord::GapSummary {
                    session: self.session.id,
                    dropped_frames: unknown,
                    reason: engine_allocation::string(
                        "collector lost clock metadata; campaign and hour range are unknown",
                    )?,
                }));
            }
            context.push(Record::Control(ControlRecord::ClockWitness {
                session: self.session.id,
                qpc: u64::try_from(qpc).map_err(|_| invalid("negative callback QPC"))?,
                hour,
                load_signal: signal,
            }));
            #[cfg(test)]
            crate::allocation_failure_tests::before_spool_publication();
            if crate::allocation::failed() {
                return Err(io::ErrorKind::OutOfMemory.into());
            }
            if restarting {
                self.spool
                    .start_encoded_segment(&context, prepared.take_frame())
            } else {
                for record in &context {
                    if crate::allocation::failed() {
                        return Err(io::ErrorKind::OutOfMemory.into());
                    }
                    self.spool.append(record)?;
                }
                if crate::allocation::failed() {
                    return Err(io::ErrorKind::OutOfMemory.into());
                }
                self.spool.append_encoded_frame(prepared.take_frame())
            }
        })();
        work.record_writes = Some(current_qpc() - write_started);
        let _span = span.switch(crate::spool_phases::Phase::Commit);
        match result {
            Ok(()) => {
                if crate::allocation::failed() {
                    return Err(io::ErrorKind::OutOfMemory.into());
                }
                let released = self.release_slices();
                let retired = self.encoder.commit_retiring(prepared)?;
                self.adopt_slices(released);
                engine_allocation::reserve(&mut self.retired, 1)?;
                self.retired.push(retired);
                self.disk_branch = Some(branch);
                self.last_written = Some(hour);
                self.pending.clear();
                crate::allocation::attempted_hour(None);
                self.tracker.record_hour(branch, hour)?;
                #[cfg(feature = "alloc-fault-injection")]
                crate::allocation::committed_hour();
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn detach(&mut self, unknown: u64) -> io::Result<()> {
        if !self.spool.is_open() {
            if self.pending.is_empty() && unknown == 0 {
                return Ok(());
            }
            return Err(invalid(
                "spool unavailable at detach; pending gaps need another observed keyframe",
            ));
        }
        for record in self.context(false, 0)? {
            self.spool.append(&record)?;
        }
        if unknown != 0 {
            self.spool
                .append(&Record::Control(ControlRecord::GapSummary {
                    session: self.session.id,
                    dropped_frames: unknown,
                    reason: engine_allocation::string(
                        "collector lost clock metadata; campaign and hour range are unknown",
                    )?,
                }))?;
        }
        self.spool.append(&Record::Control(ControlRecord::Detach {
            session: self.session.id,
            last_hour: self.last_written,
        }))?;
        self.pending.clear();
        self.spool.finish_segment()
    }
}

#[derive(Default)]
struct OrderCounterWitness {
    sample: Option<BoundOrderCounterSample>,
    rows: Vec<engine_native::ccountry::OrderInstanceCounter>,
}

struct BoundOrderCounterSample {
    seq: u64,
    branch: Option<BranchId>,
    clock: Option<Clock>,
    callback_qpc: i64,
    begin: i64,
    end: i64,
    elapsed_ns: u64,
    allocation_counters: Option<AllocationCounters>,
    read: Option<engine_native::ccountry::OrderInstanceCounterRead>,
    source_range_valid: bool,
    source_capacity_bytes: usize,
    cache_status: engine_native::country_memory::SeedStatus,
    frame_write_succeeded: bool,
    frame_write_error: Option<Cow<'static, str>>,
    subsequent_io_error: Option<Cow<'static, str>>,
}

impl OrderCounterWitness {
    fn observe(&mut self, captured: &Captured, branch: Option<BranchId>, result: &io::Result<()>) {
        self.observe_error(captured, branch, result.as_ref().err());
    }

    fn observe_error(
        &mut self,
        captured: &Captured,
        branch: Option<BranchId>,
        error: Option<&io::Error>,
    ) {
        if error.is_some_and(|error| error.kind() == io::ErrorKind::OutOfMemory) {
            if let Some(sample) = &mut self.sample {
                sample.frame_write_succeeded = false;
                sample
                    .frame_write_error
                    .get_or_insert(Cow::Borrowed("collector allocation failed"));
            }
            return;
        }
        let source = &captured.order_counters;
        if !source.forced {
            // A failed later append can also fail while closing its segment.
            // Keep the old sample's identity, but conservatively revoke its
            // write binding just as periodic sync/detach failures do.
            if let Some(error) = error
                && let Some(clock) = captured.clock.as_ref()
                && let Ok(campaign) = std::str::from_utf8(clock.campaign_id.as_bytes())
            {
                self.invalidate_write(campaign, error);
            }
            return;
        }
        let rows = source.read.and_then(|read| source.rows.get(..read.len));
        self.rows.clear();
        let source_range_valid = rows.is_some();
        let rows = rows.unwrap_or_default();
        if engine_allocation::reserve(&mut self.rows, rows.len()).is_err() {
            return;
        }
        self.rows.extend_from_slice(rows);
        self.sample = Some(BoundOrderCounterSample {
            seq: captured.capture_seq,
            branch,
            clock: captured
                .clock
                .as_ref()
                .and_then(|clock| clock.copy_owned().ok()),
            callback_qpc: captured.qpc,
            begin: source.qpc_begin,
            end: source.qpc_end,
            elapsed_ns: source.elapsed_ns,
            allocation_counters: captured.counters,
            read: source.read,
            source_range_valid,
            source_capacity_bytes: source.allocated_bytes(),
            cache_status: captured.country_cache_status,
            frame_write_succeeded: error.is_none(),
            frame_write_error: error.map(error_reason),
            subsequent_io_error: None,
        });
    }

    fn invalidate_write(&mut self, campaign: &str, error: &io::Error) {
        if let Some(sample) = &mut self.sample
            && sample
                .clock
                .as_ref()
                .is_some_and(|clock| clock.campaign_id.as_bytes() == campaign.as_bytes())
        {
            sample
                .subsequent_io_error
                .get_or_insert_with(|| error_reason(error));
        }
    }

    fn status(&self) -> impl Serialize + '_ {
        let capacity = self.rows.capacity()
            * std::mem::size_of::<engine_native::ccountry::OrderInstanceCounter>();
        let Some(sample) = &self.sample else {
            return Choice::First(
                status!({"sample": Option::<()>::None, "retained_capacity_bytes": capacity}),
            );
        };
        let cache_valid = sample.cache_status == engine_native::country_memory::SeedStatus::Ready;
        let read_complete = sample.source_range_valid
            && sample.read.is_some_and(|read| {
                read.complete()
                    && read.len == self.rows.len()
                    && self.rows.iter().all(|row| row.value.is_some())
            });
        let sample_valid = sample.clock.is_some()
            && cache_valid
            && read_complete
            && sample.begin >= sample.callback_qpc
            && sample.begin > 0
            && sample.end >= sample.begin;
        let clock = sample.clock.as_ref().map(|clock| {
            status!({
                "campaign": LossyBytes(clock.campaign_id.as_bytes()),
                "game_state": clock.game_state, "hour": clock.hour,
                "load_signal": clock.play_session_started_at,
                "play_time": clock.play_time, "tick_busy": clock.tick_busy,
            })
        });
        let allocation_counters = |counters: Option<AllocationCounters>| {
            counters.map(|counters| {
                status!({
                    "debug_current_ref_id": counters.debug_current_ref_id,
                    "unit": counters.unit,
                    "order_index": counters.order_index,
                    "front_index": counters.front_index,
                    "theatre_index": counters.theatre_index,
                    "theater_group_index": counters.theater_group_index,
                    "military_deployment_line_index": counters.military_deployment_line_index,
                    "military_deployment_conveyor_index": counters.military_deployment_conveyor_index,
                    "equipment_variant_index": counters.equipment_variant_index,
                    "country_leader_index": counters.country_leader_index,
                    "navy_id": counters.navy_id,
                    "land_combat_id": counters.land_combat_id,
                    "multiplayer_random_count": counters.multiplayer_random_count,
                })
            })
        };
        Choice::Second(status!({
            "scope": "latest consumed force_full callback, bound to its sequence, entry clock, branch and frame-write result; not the latest status hour",
            "counter_epoch": "no universal no-reset, no-load or no-wrap proof",
            "allocation_counters_scope": "the recorded beginning-of-acquisition sample; missing counter values are null. Counter metadata does not change the order-counter sample validity checks.",
            "retained_capacity_bytes": capacity,
            "sample": {
                "sequence": sample.seq, "branch": sample.branch,
                "clock": clock,
                "callback_qpc": sample.callback_qpc,
                "read_begin_qpc": sample.begin, "read_end_qpc": sample.end,
                "read_ns": sample.elapsed_ns,
                "allocation_counters": allocation_counters(sample.allocation_counters),
                "country_cache_status": DebugValue(sample.cache_status),
                "country_cache_valid": cache_valid,
                "read_complete": read_complete,
                "stored": self.rows.len(),
                "required": sample.read.map(|read| read.required),
                "unreadable": sample.read.map(|read| read.unreadable),
                "source_range_valid": sample.source_range_valid,
                "source_capacity_bytes": sample.source_capacity_bytes,
                "sample_valid": sample_valid,
                "frame_write_succeeded": sample.frame_write_succeeded,
                "frame_write_error": &sample.frame_write_error,
                "subsequent_io_error": &sample.subsequent_io_error,
                "usable_with_recorded_frame": sample_valid && sample.branch.is_some()
                    && sample.frame_write_succeeded && sample.subsequent_io_error.is_none(),
                "durability": "frame write result; not an independent replay-recovery verification",
                "countries": sequence(self.rows.iter().map(|row| status!({
                    "country": row.country, "canonical_slot": row.canonical_slot,
                    "pointer": row.pointer, "counter": row.value,
                }))),
            },
        }))
    }
    #[cfg(test)]
    fn json(&self) -> Value {
        serde_json::to_value(self.status()).unwrap()
    }
}

#[derive(Default)]
struct WarSnapshotWork {
    passes: u64,
    totals: [u64; 6],
    max_required: [usize; 3],
    peak_frame_bytes: usize,
    last: Option<(
        engine_native::cwar_relation::OwnedSnapshotStats,
        engine_native::cwar_relation::OwnedSnapshotCapacity,
    )>,
}

impl WarSnapshotWork {
    fn observe(
        &mut self,
        sampled: bool,
        stats: engine_native::cwar_relation::OwnedSnapshotStats,
        capacity: engine_native::cwar_relation::OwnedSnapshotCapacity,
    ) {
        if !sampled {
            return;
        }
        self.passes = self.passes.saturating_add(1);
        for (total, current) in self.totals.iter_mut().zip([
            stats.queries,
            stats.hits,
            stats.installed,
            stats.fallbacks,
            stats.acquisition_headers,
            stats.acquisition_words,
        ]) {
            *total = total.saturating_add(current as u64);
        }
        for (maximum, current) in self.max_required.iter_mut().zip([
            stats.required_vectors,
            stats.required_words,
            stats.required_cells,
        ]) {
            *maximum = (*maximum).max(current);
        }
        self.peak_frame_bytes = self.peak_frame_bytes.max(capacity.allocated_bytes);
        self.last = Some((stats, capacity));
    }

    fn status(&self) -> impl Serialize + '_ {
        status!({
            "scope": "processor-lifetime cumulative across campaigns: consumed optimized first-pass work, including failed/unwritten frames; not replay acceptance",
            "passes": self.passes,
            "queries": self.totals[0], "hits": self.totals[1],
            "installed": self.totals[2], "fallbacks": self.totals[3],
            "acquisition_header_attempts": self.totals[4],
            "acquisition_consumed_nonnull_words": self.totals[5],
            "word_counter_scope": "interpreted prefixes, not physical native reads; guarded batches may read a later suffix",
            "maximum_required": {
                "vectors": self.max_required[0], "words": self.max_required[1],
                "count_cells": self.max_required[2],
            },
            "requirement_scope": "observed growth demand; may be a lower bound when root metadata is full",
            "latest": self.last.map(|(stats, capacity)| status!({
                "required_vectors": stats.required_vectors,
                "required_words": stats.required_words,
                "required_count_cells": stats.required_cells,
                "capacity": {
                    "vectors": capacity.vectors,
                    "initialized_root_slots": capacity.root_slots,
                    "root_capacity": capacity.root_capacity,
                    "words": capacity.words,
                    "initialized_count_cells": capacity.initialized_count_cells,
                    "count_capacity": capacity.count_capacity,
                    "allocated_bytes": capacity.allocated_bytes,
                },
            })),
            "peak_sampled_frame_capacity_bytes": self.peak_frame_bytes,
            "capacity_scope": "last sampled frame before off-thread growth; peak is one frame, not whole-pool ownership; heap bytes also included in pool_bytes",
            "accounted_in": "WarRelations callback cost",
        })
    }
    #[cfg(test)]
    fn json(&self) -> Value {
        serde_json::to_value(self.status()).unwrap()
    }
}

fn read_counts_status(counts: ReadCounts) -> impl Serialize {
    status!({
        "copy_calls": counts.copy_calls,
        "scattered_calls": counts.scattered_calls,
        "record_calls": counts.record_calls,
        "plan_calls": counts.plan_calls,
        "spans": counts.spans,
        "bytes": counts.bytes,
    })
}

#[cfg(test)]
fn read_counts_json(counts: ReadCounts) -> Value {
    serde_json::to_value(read_counts_status(counts)).unwrap()
}

/// One sliced family's reads and fingerprint scans, per recorded frame.
#[derive(Default)]
struct SliceWork {
    frames: u64,
    full_frames: u64,
    scanned: u64,
    out_of_range: u64,
    out_of_range_max: u32,
    early: u64,
    early_max: u32,
    scan_ticks: u64,
    scan_ticks_max: u64,
}

impl SliceWork {
    fn observe(&mut self, selection: &crate::slicing::SliceSelection) {
        self.frames += 1;
        self.full_frames += u64::from(selection.force_full);
        self.scanned += u64::from(selection.scanned);
        self.out_of_range += u64::from(selection.out_of_range);
        self.out_of_range_max = self.out_of_range_max.max(selection.out_of_range);
        self.early += u64::from(selection.early_count);
        self.early_max = self.early_max.max(selection.early_count);
        self.scan_ticks = self.scan_ticks.saturating_add(selection.scan_ticks);
        self.scan_ticks_max = self.scan_ticks_max.max(selection.scan_ticks);
    }

    fn status(&self, frequency: u64) -> crate::status::Object<impl crate::status::Fields + '_> {
        let ns = |ticks: u64| {
            (u128::from(ticks) * 1_000_000_000)
                .checked_div(u128::from(frequency))
                .unwrap_or(0)
                .min(u128::from(u64::MAX)) as u64
        };
        status!({
            "frames": self.frames,
            "full_frames": self.full_frames,
            "owners_scanned": self.scanned,
            "owners_out_of_fingerprint_range": self.out_of_range,
            "owners_out_of_fingerprint_range_max": self.out_of_range_max,
            "early_reads": self.early,
            "early_reads_max": self.early_max,
            "scan_ns": ns(self.scan_ticks),
            "scan_mean_ns": ns(self.scan_ticks).checked_div(self.frames).unwrap_or(0),
            "scan_max_ns": ns(self.scan_ticks_max),
            "scope": "a sliced family reads one owner slice per hour plus owners whose change fingerprint changed (early reads); scan time is the fingerprint pass inside the family's worker job, not game-thread time; full frames read every owner; owners out of fingerprint range (ID >= 65536) have no fingerprint and are read at every hour",
        })
    }
}

/// A daily table that truncated waits for its slot otherwise; a static table
/// that failed would wait for the next full hour.
fn needs_retry(tables: &[TableInput; TABLE_COUNT]) -> bool {
    TableId::ALL.into_iter().any(|table| {
        let input = &tables[table.index()];
        match replay_data::cadence::cadence(table) {
            replay_data::cadence::Cadence::Static => !matches!(
                input,
                TableInput::Sampled(TableView::Complete(_)) | TableInput::SharedComplete(_)
            ) && !matches!(input, TableInput::Sampled(TableView::Unavailable { reason }) if reason == replay_data::cadence::SKIPPED_REASON),
            replay_data::cadence::Cadence::Daily(_) => {
                matches!(input, TableInput::Sampled(TableView::Truncated { .. }))
            }
            // A failed, truncated or dropped slice read is read again at the
            // next hour (SliceSelection::invalidate); the merge requests a full
            // refresh only when it has no earlier rows to merge into.
            replay_data::cadence::Cadence::Hourly | replay_data::cadence::Cadence::Sliced => false,
        }
    })
}

/// The country the recording client views and the operands that pick its flag
/// (own tag, cosmetic tag, original-or-own tag, ruling ideology group), copied
/// from committed rows for the recorder's panel. The status omits it while any
/// of those rows is unavailable.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
struct ViewedFlag {
    tag: String,
    cosmetic: String,
    original: Option<String>,
    group: String,
}

impl ViewedFlag {
    fn same(&self, operands: &replay_data::country_flags::FlagOperands<'_>) -> bool {
        self.tag == operands.tag
            && self.cosmetic == operands.cosmetic
            && self.original.as_deref() == operands.original
            && self.group == operands.group
    }

    fn copy(operands: &replay_data::country_flags::FlagOperands<'_>) -> io::Result<Self> {
        Ok(Self {
            tag: engine_allocation::string(operands.tag)?,
            cosmetic: engine_allocation::string(operands.cosmetic)?,
            original: operands
                .original
                .map(engine_allocation::string)
                .transpose()?,
            group: engine_allocation::string(operands.group)?,
        })
    }
}

/// The viewed country's flag operands from the encoder's committed baselines.
fn viewed_flag(encoder: &FrameEncoder) -> Option<replay_data::country_flags::FlagOperands<'_>> {
    replay_data::country_flags::viewed_country(
        encoder.committed::<tables::LocalPlayer>()?.1,
        encoder.committed::<tables::CountryLinks>()?.1,
        encoder.committed::<tables::Countries>()?.1,
        encoder.committed::<tables::CountryAppearance>()?.1,
        encoder.committed::<tables::Politics>()?.1,
        encoder.committed::<tables::PoliticalParties>()?.1,
    )
    .ok()
}

/// Keep the published viewed country current, copying only when it changes.
fn set_viewed(
    slot: &mut Option<ViewedFlag>,
    operands: Option<replay_data::country_flags::FlagOperands<'_>>,
) -> io::Result<()> {
    match operands {
        None => *slot = None,
        Some(operands) if slot.as_ref().is_some_and(|current| current.same(&operands)) => {}
        Some(operands) => *slot = Some(ViewedFlag::copy(&operands)?),
    }
    Ok(())
}

struct Statistics {
    validation_refusals: ValidationRefusals,
    game_score: crate::extended::game_score::Work,
    country_rosters: crate::extended::country_counters::RosterWork,
    reserved_visibility: crate::extended::reserved_visibility::Work,
    state_map_inputs: crate::acquisition_states::InputWork,
    railway_map_inputs: crate::acquisition_states::InputWork,
    physical_navies: crate::extended::PhysicalNavyWork,
    battle_plan_child_fronts: crate::extended::plans::ChildFrontWork,
    battle_plan_execution: crate::extended::plans::ExecutionWork,
    division_naming: crate::extended::division_presentation::NamingWork,
    counter_tooltip_inputs: crate::extended::division_presentation::CounterWork,
    character_naming: crate::extended::character_inputs::NamingWork,
    supply_vehicles: crate::extended::supply_trains::VehicleWork,
    variable_seeds: crate::extended::intel::SeedWork,
    war_snapshots: WarSnapshotWork,
    order_counters: OrderCounterWitness,
    hours: u64,
    campaign: Option<String>,
    last_hour: Option<i32>,
    /// The recording client's viewed country, for the recorder's panel.
    player: Option<ViewedFlag>,
    total_ns: u64,
    callback: Distribution,
    callback_tail: crate::slow_hours::Recorder,
    table_ns: [u64; TABLE_COUNT],
    table_total_ns: [u64; TABLE_COUNT],
    /// Cumulative successful-frame Stability job phase ticks, including its
    /// countries that refused. Inputs includes setup and per-country folding.
    stability_walk_ticks: [u64; 4],
    table_reads: [ReadCounts; TABLE_COUNT],
    table_total_reads: [ReadCounts; TABLE_COUNT],
    dropped_frames: u64,
    truncated_tables: usize,
    country_cache_fallbacks: u64,
    country_cache_entries: usize,
    country_cache_status: engine_native::country_memory::SeedStatus,
    country_cache_cost: Distribution,
    pool_bytes: u64,
    sampled_buffer_bytes: usize,
    sampled_buffer_parts: Vec<(&'static str, usize)>,
    staggered_skipped: [u64; TABLE_COUNT],
    staggered_walked: [u64; TABLE_COUNT],
    previous: Option<(String, BranchId, i32, i64)>,
    interval_hours: u64,
    interval_qpc: u64,
    errors: u64,
    last_error: Option<Cow<'static, str>>,
    input_demand_errors: u64,
    input_demand_last_error: Option<&'static str>,
    conversion: Distribution,
    conversion_packs: [WallTotals; crate::extended::PACK_NAMES.len()],
    frame_encoding: Distribution,
    record_writes: Distribution,
    slices: [SliceWork; crate::slicing::FAMILIES.len()],
    /// Cumulative torn memo copies and refused raid rows.
    raid_torn_memo_reads: u64,
    raid_refused_leaders: u64,
    weather_suspend_failures: u64,
    weather_suspend_last_failure: Option<&'static str>,
    weather_display_failures: u64,
    weather_display_last_failure: Option<&'static str>,
}

impl Default for Statistics {
    fn default() -> Self {
        Self {
            validation_refusals: ValidationRefusals::default(),
            country_rosters: crate::extended::country_counters::RosterWork::default(),
            reserved_visibility: crate::extended::reserved_visibility::Work::default(),
            state_map_inputs: crate::acquisition_states::InputWork::default(),
            railway_map_inputs: crate::acquisition_states::InputWork::default(),
            physical_navies: crate::extended::PhysicalNavyWork::default(),
            battle_plan_child_fronts: crate::extended::plans::ChildFrontWork::default(),
            battle_plan_execution: crate::extended::plans::ExecutionWork::default(),
            division_naming: crate::extended::division_presentation::NamingWork::default(),
            counter_tooltip_inputs: crate::extended::division_presentation::CounterWork::default(),
            character_naming: crate::extended::character_inputs::NamingWork::default(),
            supply_vehicles: crate::extended::supply_trains::VehicleWork::default(),
            variable_seeds: crate::extended::intel::SeedWork::default(),
            game_score: Default::default(),
            order_counters: OrderCounterWitness::default(),
            war_snapshots: WarSnapshotWork::default(),
            hours: 0,
            campaign: None,
            last_hour: None,
            player: None,
            total_ns: 0,
            callback: Distribution::default(),
            callback_tail: crate::slow_hours::Recorder::default(),
            table_ns: [0; TABLE_COUNT],
            table_total_ns: [0; TABLE_COUNT],
            stability_walk_ticks: [0; 4],
            table_reads: [ReadCounts::ZERO; TABLE_COUNT],
            table_total_reads: [ReadCounts::ZERO; TABLE_COUNT],
            dropped_frames: 0,
            truncated_tables: 0,
            country_cache_fallbacks: 0,
            country_cache_entries: 0,
            country_cache_status: engine_native::country_memory::SeedStatus::Unavailable {
                reason: "no observation",
            },
            country_cache_cost: Distribution::default(),
            pool_bytes: 0,
            sampled_buffer_bytes: 0,
            sampled_buffer_parts: Vec::new(),
            previous: None,
            staggered_skipped: [0; TABLE_COUNT],
            staggered_walked: [0; TABLE_COUNT],
            interval_hours: 0,
            interval_qpc: 0,
            errors: 0,
            last_error: None,
            input_demand_errors: 0,
            input_demand_last_error: None,
            conversion: Distribution::default(),
            conversion_packs: [WallTotals::default(); crate::extended::PACK_NAMES.len()],
            frame_encoding: Distribution::default(),
            record_writes: Distribution::default(),
            slices: std::array::from_fn(|_| SliceWork::default()),
            raid_torn_memo_reads: 0,
            raid_refused_leaders: 0,
            weather_suspend_failures: 0,
            weather_suspend_last_failure: None,
            weather_display_failures: 0,
            weather_display_last_failure: None,
        }
    }
}

impl Statistics {
    fn observe_stability_walk_ticks(&mut self, latest: [u64; 4]) {
        for (total, ticks) in self.stability_walk_ticks.iter_mut().zip(latest) {
            *total = total.saturating_add(ticks);
        }
    }

    fn observe_table_costs(&mut self, latest: [u64; TABLE_COUNT]) {
        self.table_ns = latest;
        for (total, ns) in self.table_total_ns.iter_mut().zip(latest) {
            *total = total.saturating_add(ns);
        }
    }

    fn observe_table_reads(&mut self, latest: [ReadCounts; TABLE_COUNT]) {
        self.table_reads = latest;
        for (total, counts) in self.table_total_reads.iter_mut().zip(latest) {
            *total += counts;
        }
    }
}

/// Owned conversion for exactly one hour. Write it before preparing another
/// hour, flushing or detaching: comparison tokens belong to the current encoder.
#[must_use = "prepared observations must be written before processing another hour"]
pub(crate) struct PreparedFrame {
    source: FrameInput,
    id: String,
    clock: Clock,
    qpc: i64,
    unknown: u64,
    retry_full: bool,
    /// Sliced families whose read conversion or merging withheld (bit per family).
    unpublished_families: u32,
    skipped: [bool; TABLE_COUNT],
    work: WorkSample,
    pack_ticks: [Option<i64>; crate::extended::PACK_NAMES.len()],
    sampled: CapturedStatistics,
    witness: PreparedOrderWitness,
    operation_publication: Option<crate::extended::operation_outcomes::Publication>,
    map_event_publication: Option<crate::extended::map_events::Publication>,
    event_feed_publication: Option<crate::event_feed_history::Prepared>,
}

impl PreparedFrame {
    pub(crate) fn needs_retry(&self) -> bool {
        self.retry_full
    }

    pub(crate) fn allows_early_recycle(&self) -> bool {
        !self.needs_retry()
    }
}

/// Everything success accounting needs after the capture slot has been reused.
struct CapturedStatistics {
    total_ns: u64,
    hour: i32,
    qpc: i64,
    hour_cost: crate::sampling::HourCost,
    table_ns: [u64; TABLE_COUNT],
    table_reads: [ReadCounts; TABLE_COUNT],
    stability_walk_ticks: [u64; 4],
    pool_bytes: u64,
    dropped_frames: u64,
    sampled_buffer_bytes: usize,
    sampled_buffer_parts: Vec<(&'static str, usize)>,
    truncated_tables: usize,
    country_cache_status: engine_native::country_memory::SeedStatus,
    country_cache_entries: usize,
    country_cache_ns: u64,
    raid_reads: crate::extended::RaidReadCounts,
    weather_suspend_failure: Option<&'static str>,
    weather_display_failure: Option<&'static str>,
}

impl CapturedStatistics {
    fn capture(captured: &Captured) -> io::Result<Self> {
        let mut sampled_buffer_parts = captured.extended.allocated_by_pack()?;
        let extended_bytes: usize = sampled_buffer_parts.iter().map(|(_, bytes)| *bytes).sum();
        let map_bytes = captured.map.allocated_bytes();
        let core_bytes = captured.core_allocated_bytes();
        let sampled_buffer_bytes = core_bytes + map_bytes + extended_bytes;
        engine_allocation::reserve(&mut sampled_buffer_parts, 2)?;
        sampled_buffer_parts.push(("map", map_bytes));
        sampled_buffer_parts.push(("core", core_bytes));
        let raid_reads = captured.extended.raid_reads();
        let extended_outcomes = captured.extended.outcomes();
        let truncated_tables = [
            captured.countries_result,
            captured.links_result,
            captured.provinces_result,
            captured.provinces.conflict_walk.outcome,
            captured.states_result,
            captured.divisions_result,
            captured.wars_result,
            captured.factions_result,
            captured.faction_members_result,
        ]
        .into_iter()
        .chain(captured.map.outcomes())
        .chain(crate::extended::PACK_TABLES.map(|table| {
            if table == TableId::StateResistance {
                captured.state_inputs.occupation.outcome()
            } else {
                extended_outcomes[table.index()]
            }
        }))
        .filter(|outcome| matches!(outcome, Outcome::Truncated { .. }))
        .count();
        Ok(Self {
            total_ns: captured.total_ns,
            hour: captured
                .clock
                .as_ref()
                .expect("recorded frame has a clock")
                .hour,
            qpc: captured.qpc,
            hour_cost: captured.hour_cost,
            table_ns: captured.table_ns,
            table_reads: captured.table_reads,
            stability_walk_ticks: captured.extended.stability_phase_ticks(),
            pool_bytes: captured.pool_bytes,
            dropped_frames: captured.dropped_frames,
            sampled_buffer_bytes,
            sampled_buffer_parts,
            truncated_tables,
            country_cache_status: captured.country_cache_status,
            country_cache_entries: captured.country_cache_entries,
            country_cache_ns: captured.country_cache_ns,
            raid_reads,
            weather_suspend_failure: captured.extended.weather_suspend_failure(),
            weather_display_failure: captured.extended.weather_display_failure(),
        })
    }

    fn record(self, stats: &mut Statistics) {
        stats.total_ns = self.total_ns;
        stats.callback.record_ns(self.total_ns);
        stats
            .callback_tail
            .record(self.hour, self.qpc, self.total_ns, self.hour_cost);
        stats.observe_table_costs(self.table_ns);
        stats.observe_table_reads(self.table_reads);
        stats.observe_stability_walk_ticks(self.stability_walk_ticks);
        stats.pool_bytes = self.pool_bytes;
        stats.sampled_buffer_bytes = self.sampled_buffer_bytes;
        stats.sampled_buffer_parts = self.sampled_buffer_parts;
        stats.dropped_frames = self.dropped_frames;
        stats.country_cache_fallbacks += u64::from(
            self.country_cache_status != engine_native::country_memory::SeedStatus::Ready,
        );
        stats.country_cache_entries = self.country_cache_entries;
        stats.country_cache_status = self.country_cache_status;
        stats.country_cache_cost.record_ns(self.country_cache_ns);
        stats.truncated_tables = self.truncated_tables;
        if let Some(reason) = self.weather_display_failure {
            stats.weather_display_failures = stats.weather_display_failures.saturating_add(1);
            stats.weather_display_last_failure = Some(reason);
        }
        if let Some(reason) = self.weather_suspend_failure {
            stats.weather_suspend_failures = stats.weather_suspend_failures.saturating_add(1);
            stats.weather_suspend_last_failure = Some(reason);
        }
        stats.raid_torn_memo_reads = stats
            .raid_torn_memo_reads
            .saturating_add(u64::from(self.raid_reads.torn_memo_reads));
        stats.raid_refused_leaders = stats
            .raid_refused_leaders
            .saturating_add(u64::from(self.raid_reads.refused_leaders));
    }
}

/// A forced diagnostic owns its row copy before the capture slot is released.
/// Ordinary observations retain only the clock needed to invalidate an old witness.
struct PreparedOrderWitness {
    clock: Option<Clock>,
    forced: Option<OrderCounterWitness>,
}

impl PreparedOrderWitness {
    fn capture(captured: &Captured, branch: Option<BranchId>) -> Self {
        let forced = captured.order_counters.forced.then(|| {
            let mut witness = OrderCounterWitness::default();
            witness.observe(captured, branch, &Ok(()));
            witness
        });
        Self {
            clock: captured
                .clock
                .as_ref()
                .and_then(|clock| clock.copy_owned().ok()),
            forced,
        }
    }

    fn bind(self, target: &mut OrderCounterWitness, result: &io::Result<()>) {
        if let Some(mut witness) = self.forced {
            let sample = witness
                .sample
                .as_mut()
                .expect("forced witness owns its sample");
            sample.frame_write_succeeded = result.is_ok();
            sample.frame_write_error = result.as_ref().err().map(error_reason);
            *target = witness;
        } else if let Err(error) = result
            && let Some(clock) = self.clock
            && let Ok(campaign) = std::str::from_utf8(clock.campaign_id.as_bytes())
        {
            target.invalidate_write(campaign, error);
        }
    }
}

pub struct Processor {
    root: PathBuf,
    history: PreparedCampaigns,
    session: SessionStart,
    campaigns: Campaigns,
    stats: Statistics,
    unknown_pending: u64,
    suppress_origin: bool,
    /// A staggered table truncated or a static table failed this hour; the
    /// runtime asks the collector to walk everything again next hour.
    retry_full: bool,
    /// Sliced families a conversion withheld; the runtime has the collector
    /// forget their fingerprint stores.
    unpublished_families: u32,
    /// Converts map tables and lane-0 helper packs beside this thread, and
    /// lane 1 too when the second helper is absent.
    helper: Option<crate::spool_helper::SpoolHelper>,
    /// Converts lane-1 helper packs beside this thread and the first helper.
    second_helper: Option<crate::spool_helper::SpoolHelper>,
    /// The spool thread's latest backlog verdict, which the helper follows.
    backlogged: bool,
    /// Decisions of the comparison before encoding.
    comparison: Box<crate::spool_compare::Counters>,
}

impl Processor {
    pub(crate) fn validation_refusals(&self) -> ValidationRefusals {
        self.stats.validation_refusals.clone()
    }
    pub(crate) fn observe_game_score_sizing(
        &mut self,
        sizing: &engine_native::game_score_inputs::ArenaSizing,
    ) -> io::Result<()> {
        let _details = replay_data::codec::error_details_scope();
        self.stats.game_score.observe_sizing(sizing)
    }
    pub fn new(root: &Path, history: PreparedCampaigns, session: SessionStart) -> io::Result<Self> {
        let _details = replay_data::codec::error_details_scope();
        session.validate()?;
        fs::create_dir_all(root).map_err(engine_allocation::allocation_error)?;
        Ok(Self {
            root: root.into(),
            history,
            session,
            campaigns: Campaigns::new(),
            stats: Statistics::default(),
            unknown_pending: 0,
            suppress_origin: false,
            retry_full: false,
            unpublished_families: 0,
            helper: None,
            second_helper: None,
            backlogged: false,
            comparison: Box::default(),
        })
    }

    /// Hand a conversion helper to this processor: the first converts the map
    /// tables and both helper lanes, a second takes lane 1. Without one, the
    /// spool thread converts every table itself with identical output.
    pub(crate) fn set_helper(&mut self, helper: crate::spool_helper::SpoolHelper) {
        if self.helper.is_none() {
            self.helper = Some(helper);
        } else {
            self.second_helper = Some(helper);
        }
    }

    /// The capture-buffer backlog verdict for the next conversion.
    pub(crate) fn set_backlogged(&mut self, backlogged: bool) {
        self.backlogged = backlogged;
    }

    pub fn set_verified_session(&mut self, session: SessionStart) {
        // The spool worker calls this exactly once, before its first observation.
        assert!(self.campaigns.is_empty());
        self.session = session;
    }

    fn campaign(&mut self, id: &str) -> io::Result<&mut Campaign> {
        if !self.campaigns.contains_key(id) {
            let mut campaign = Campaign::open(&self.root, &mut self.history, id, &self.session)?;
            campaign.validation_refusals = self.stats.validation_refusals.clone();
            self.campaigns
                .insert(engine_allocation::string(id)?, campaign)?;
        }
        Ok(self.campaigns.get_mut(id).unwrap())
    }

    fn error(&mut self, error: &io::Error) {
        self.stats.errors += 1;
        // Preserve the cause of recovery when later sync/detach work also fails.
        self.stats
            .last_error
            .get_or_insert_with(|| error_reason(error));
    }

    /// A failed or racing off-thread sizing hint loses no captured data; it is
    /// counted here and never becomes the session warning.
    pub(crate) fn observe_input_demand_error(&mut self, reason: &'static str) {
        self.observe_input_demand_errors(1, reason);
    }

    /// `count` such errors, `last` the latest; the sizing thread reports them
    /// together.
    pub(crate) fn observe_input_demand_errors(&mut self, count: u64, last: &'static str) {
        self.stats.input_demand_errors = self.stats.input_demand_errors.saturating_add(count);
        self.stats.input_demand_last_error = Some(last);
    }

    /// Off-thread observations prepare storage without affecting session warnings.
    pub(crate) fn observe_input_demand<T>(&mut self, result: Result<T, &'static str>) -> Option<T> {
        match result {
            Ok(demand) => Some(demand),
            Err(reason) => {
                self.observe_input_demand_error(reason);
                None
            }
        }
    }

    /// The caller retains the buffer until this succeeds. Opening a campaign may
    /// do I/O, but this preflight never observes or acknowledges a callback.
    /// Once prepared, consume its loss witnesses and then call `frame` once.
    pub fn prepare(&mut self, captured: &Captured) -> io::Result<()> {
        let _details = replay_data::codec::error_details_scope();
        let _phase = crate::allocation::enter(crate::allocation::Phase::Spool);
        if crate::allocation::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        crate::allocation::set_context(Default::default());
        let clock = captured.clock.as_ref();
        let ids = captured
            .dropped_before
            .iter()
            .map(|loss| {
                crate::allocation::set_context(crate::allocation::Context {
                    campaign_id: loss.campaign_id.as_bytes().try_into().ok(),
                    branch: None,
                    load_signal: loss.load_signal,
                    failed_hour: Some(loss.first_hour),
                });
                loss.campaign_id.as_bytes()
            })
            .chain(clock.into_iter().map(|clock| {
                crate::allocation::set_context(crate::allocation::Context {
                    campaign_id: clock.campaign_id.as_bytes().try_into().ok(),
                    branch: None,
                    load_signal: Some(clock.play_session_started_at),
                    failed_hour: Some(clock.hour),
                });
                clock.campaign_id.as_bytes()
            }));
        let result = prepare_campaigns(ids, |id| self.campaign(id).map(|_| ()));
        if let Err(error) = &result {
            self.error(error);
        }
        result
    }

    /// Quiescent detach uses the same preflight before consuming its final batch.
    pub fn prepare_losses(&mut self, losses: &LossBatch) -> io::Result<()> {
        let _details = replay_data::codec::error_details_scope();
        let _phase = crate::allocation::enter(crate::allocation::Phase::Teardown);
        if crate::allocation::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        crate::allocation::set_context(Default::default());
        let result = prepare_campaigns(
            losses.iter().map(|loss| {
                crate::allocation::set_context(crate::allocation::Context {
                    campaign_id: loss.campaign_id.as_bytes().try_into().ok(),
                    branch: None,
                    load_signal: loss.load_signal,
                    failed_hour: Some(loss.first_hour),
                });
                loss.campaign_id.as_bytes()
            }),
            |id| self.campaign(id).map(|_| ()),
        );
        if let Err(error) = &result {
            self.error(error);
        }
        result
    }

    pub fn lost(&mut self, loss: &LostObservation) -> io::Result<()> {
        let _details = replay_data::codec::error_details_scope();
        if crate::allocation::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        crate::allocation::set_context(crate::allocation::Context {
            campaign_id: loss.campaign_id.as_bytes().try_into().ok(),
            branch: None,
            load_signal: loss.load_signal,
            failed_hour: Some(loss.first_hour),
        });
        let result = (|| {
            let id = match campaign_name(loss.campaign_id.as_bytes()) {
                Ok(id) => id,
                Err(error) => {
                    self.unknown_loss(loss.count);
                    return Err(error);
                }
            };
            let suppress = self.suppress_origin;
            self.campaign(&id)?.lost(loss, suppress)
        })();
        if let Err(error) = &result {
            self.error(error);
        }
        result
    }

    pub fn unknown_loss(&mut self, count: u64) {
        self.unknown_pending = self.unknown_pending.saturating_add(count);
        self.suppress_origin |= count != 0;
    }

    #[cfg(test)]
    pub fn frame(&mut self, captured: &Captured) -> io::Result<()> {
        let prepared = self.prepare_frame(captured)?;
        self.write_prepared(prepared)
    }

    pub(crate) fn prepare_frame(&mut self, captured: &Captured) -> io::Result<PreparedFrame> {
        let _details = replay_data::codec::error_details_scope();
        let _phase = crate::allocation::enter(crate::allocation::Phase::Spool);
        if crate::allocation::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        let span = crate::spool_phases::SPOOL.span(crate::spool_phases::Phase::Observe);
        if let Some(clock) = captured.clock.as_ref() {
            crate::allocation::set_context(crate::allocation::Context {
                campaign_id: clock.campaign_id.as_bytes().try_into().ok(),
                load_signal: Some(clock.play_session_started_at),
                failed_hour: Some(clock.hour),
                ..Default::default()
            });
        } else {
            crate::allocation::set_context(Default::default());
        }
        if captured.clock.is_some() {
            captured
                .state_inputs
                .observe(&mut self.stats.state_map_inputs)?;
            captured
                .extended
                .observe_railway_inputs(&mut self.stats.railway_map_inputs)?;
            captured
                .extended
                .observe_physical_navies(&mut self.stats.physical_navies)?;
            captured
                .extended
                .observe_battle_plan_child_fronts(&mut self.stats.battle_plan_child_fronts);
            captured
                .extended
                .observe_battle_plan_execution(&mut self.stats.battle_plan_execution);
            self.stats
                .game_score
                .observe(captured.extended.game_score_status())?;
            captured.extended.observe_country_rosters(
                &mut self.stats.country_rosters,
                self.session.qpc_frequency,
            )?;
            captured
                .extended
                .observe_reserved_visibility(&mut self.stats.reserved_visibility)?;
            captured
                .extended
                .observe_division_naming(&mut self.stats.division_naming)?;
            captured
                .extended
                .observe_counter_tooltip_inputs(&mut self.stats.counter_tooltip_inputs)?;
            captured
                .extended
                .observe_character_naming(&mut self.stats.character_naming)?;
            captured
                .extended
                .observe_supply_vehicles(&mut self.stats.supply_vehicles);
            captured
                .extended
                .observe_variable_seeds(&mut self.stats.variable_seeds);
        }
        // A clockless frame read nothing; its selections belong to an older hour.
        for (index, _family) in crate::slicing::FAMILIES
            .iter()
            .enumerate()
            .filter(|_| captured.clock.is_some())
        {
            self.stats.slices[index].observe(captured.extended.family_selection(index));
        }
        self.stats.war_snapshots.observe(
            captured.wars_snapshot_sampled,
            captured.wars_snapshot.stats(),
            captured.wars_snapshot.capacity(),
        );
        let mut branch = None;
        let result = self.prepare_frame_inner(captured, &mut branch, span);
        if let Err(error) = &result {
            self.stats
                .order_counters
                .observe_error(captured, branch, Some(error));
            self.error(error);
        }
        result
    }

    fn prepare_frame_inner(
        &mut self,
        captured: &Captured,
        bound_branch: &mut Option<BranchId>,
        span: crate::spool_phases::Span<'static>,
    ) -> io::Result<PreparedFrame> {
        let Some(clock) = captured.clock.as_ref() else {
            self.unknown_loss(1);
            return Err(invalid(
                "entry clock unreadable; campaign and hour cannot be recorded",
            ));
        };
        let id = match campaign_name(clock.campaign_id.as_bytes()) {
            Ok(id) => id,
            Err(error) => {
                self.unknown_loss(1);
                return Err(error);
            }
        };
        let suppress = self.suppress_origin;
        let unknown = self.unknown_pending;
        self.campaign(&id)?;
        let helper = self
            .helper
            .as_ref()
            .map(|first| crate::spool_helper::Helpers {
                first,
                second: self.second_helper.as_ref(),
                backlogged: self.backlogged,
            });
        let campaign = self
            .campaigns
            .get_mut(&id)
            .expect("an opened campaign remains admitted");
        crate::allocation::set_context(crate::allocation::Context {
            campaign_id: clock.campaign_id.as_bytes().try_into().ok(),
            branch: None,
            load_signal: Some(clock.play_session_started_at),
            failed_hour: Some(clock.hour),
        });
        let origin = if suppress || campaign.tracker.current_branch().is_none() {
            None
        } else {
            Some(clock.hour)
        };
        // Native invalidation can leave every stored clock operand unchanged.
        if campaign
            .world_generation
            .is_some_and(|previous| previous != captured.world_generation)
        {
            campaign.tracker.world_changed();
        }
        campaign.world_generation = Some(captured.world_generation);
        let branch = campaign.observe(origin, clock.hour, Some(clock.play_session_started_at))?;
        crate::allocation::set_context(crate::allocation::Context {
            campaign_id: clock.campaign_id.as_bytes().try_into().ok(),
            branch: Some(branch),
            load_signal: Some(clock.play_session_started_at),
            failed_hour: Some(clock.hour),
        });
        *bound_branch = Some(branch);
        let conversion_started = current_qpc();
        let variables_baseline = campaign
            .encoder
            .comparison_baseline::<tables::ScriptVariables>(
                branch,
                clock.hour,
                !campaign.spool.is_open(),
            );
        let mut characters = CharacterConversion::new(
            branch,
            &mut campaign.character_shadow,
            character_cache(&campaign.staggered_cache, campaign.staggered_branch, branch),
        );
        let mut lexical = LexicalConversion::new(
            lexical_cache(&campaign.staggered_cache, campaign.staggered_branch, branch),
            &mut campaign.lexical_conversion,
        );
        let mut intel = IntelConversion::new(branch, &mut campaign.intel_shadow);
        let mut projection = Projection::new(
            &campaign.encoder,
            branch,
            clock.hour,
            !campaign.spool.is_open(),
            &campaign.projection_scratch,
        );
        span.finish();
        let (mut source, pack_ticks) = decoded_with_costs(
            captured,
            clock.borrowed(),
            branch,
            &mut SpoolConversion {
                variables_baseline,
                variables_scratch: &mut campaign.variables_scratch,
                characters: Some(&mut characters),
                lexical: Some(&mut lexical),
                intel: Some(&mut intel),
                projection: Some(&mut projection),
                helper,
            },
        );
        let span = crate::spool_phases::SPOOL.span(crate::spool_phases::Phase::Merge);
        if crate::allocation::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        let outcomes = captured.extended.outcomes();
        let (sliced_retry, unpublished_families) = campaign.try_merge_sliced_inputs(
            &mut source,
            |table| captured.extended.slice_selection(table),
            |table| {
                matches!(
                    outcomes[table.index()],
                    Outcome::Rows { .. } | Outcome::Truncated { .. }
                )
            },
        )?;
        campaign.assemble_line_output(&mut source, captured);
        let event_feed_publication = captured.extended.event_feed().prepare_pack()?;
        if let Some(publication) = &event_feed_publication {
            let mut rows = engine_allocation::vec(1)?;
            rows.push(replay_data::rows::RowCodec::try_clone(publication.row())?);
            source.tables[TableId::EventFeedHistory.index()] = TableInput::Sampled(
                TableView::Complete(replay_data::rows::TableRows::EventFeedHistory(rows)),
            );
        }
        let operation_publication = campaign
            .operation_outcomes
            .assemble_owned(captured.extended.operation_outcomes(), &mut source.tables);
        let map_event_publication = campaign
            .map_events
            .assemble_owned(captured.extended.map_events(), &mut source.tables);
        if crate::allocation::failed() {
            return Err(crate::allocation::oom());
        }
        campaign.politics.assemble_owned(
            source.branch,
            source.hour,
            captured.extended.politics(),
            &captured.countries,
            captured.countries_result,
            &mut source.tables,
        )?;
        campaign.assemble_construction_ic(&mut source, captured);
        campaign.fuel_awards.assemble(
            source.branch,
            source.hour,
            captured.extended.fuel_awards(),
            &mut source.tables,
        )?;
        let retry_full = needs_retry(&source.tables) || sliced_retry;
        let skipped = campaign
            .try_apply_staggered_inputs(&mut source, captured.extended.country_intel_sample())?;
        campaign.assemble_reserved_intel_sample(&mut source);
        if let Some(layout) = &captured.fixed_layout {
            crate::fixed_layout::apply(&mut source.tables, layout)?;
        }
        let work = WorkSample {
            conversion: current_qpc() - conversion_started,
            ..WorkSample::default()
        };
        let span = span.switch(crate::spool_phases::Phase::Statistics);
        let sampled = CapturedStatistics::capture(captured)?;
        let witness = PreparedOrderWitness::capture(captured, Some(branch));
        if crate::allocation::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        let clock = clock.copy_owned()?;
        span.finish();
        Ok(PreparedFrame {
            source,
            id,
            clock,
            qpc: captured.qpc,
            unknown,
            retry_full,
            unpublished_families,
            skipped,
            work,
            pack_ticks,
            sampled,
            witness,
            operation_publication,
            map_event_publication,
            event_feed_publication,
        })
    }

    pub(crate) fn write_prepared(&mut self, prepared: PreparedFrame) -> io::Result<()> {
        let _details = replay_data::codec::error_details_scope();
        let _phase = crate::allocation::enter(crate::allocation::Phase::Spool);
        if crate::allocation::failed() {
            if let Some(publication) = &prepared.event_feed_publication {
                crate::event_feed_observer::discarded(publication.publication());
            }
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        let PreparedFrame {
            mut source,
            id,
            clock,
            qpc,
            unknown,
            retry_full,
            unpublished_families,
            skipped,
            mut work,
            pack_ticks,
            sampled,
            witness,
            operation_publication,
            map_event_publication,
            event_feed_publication,
        } = prepared;
        self.unpublished_families |= unpublished_families;
        let branch = source.branch;
        crate::allocation::set_context(crate::allocation::Context {
            campaign_id: clock.campaign_id.as_bytes().try_into().ok(),
            branch: Some(branch),
            load_signal: Some(clock.play_session_started_at),
            failed_hour: Some(clock.hour),
        });
        // Preparation already admitted this campaign. No other processor operation
        // may intervene between preparation and writing its baseline-bound input.
        let campaign = self
            .campaigns
            .get_mut(&id)
            .expect("prepared campaign remains admitted");
        // Compare before encoding with the same restart verdict the write uses.
        let helpers = self
            .helper
            .as_ref()
            .map(|first| crate::spool_helper::Helpers {
                first,
                second: self.second_helper.as_ref(),
                backlogged: self.backlogged,
            });
        crate::hook_failures::apply(&mut source)?;
        crate::spool_compare::compare_frame(
            &campaign.encoder,
            crate::spool_compare::Target {
                branch,
                hour: source.hour,
                restarting: !campaign.spool.is_open(),
            },
            &mut source,
            helpers,
            &self.comparison,
        );
        let result = campaign.write_with(
            source,
            qpc,
            Some(clock.play_session_started_at),
            unknown,
            &mut work,
            helpers,
        );
        let _span = crate::spool_phases::SPOOL.span(crate::spool_phases::Phase::Commit);
        witness.bind(&mut self.stats.order_counters, &result);
        if let Err(error) = &result {
            self.error(error);
        }
        for table in replay_data::rows::TableId::ALL {
            let index = table.index();
            if replay_data::cadence::is_staggered(table) {
                if skipped[index] {
                    self.stats.staggered_skipped[index] += 1;
                } else {
                    self.stats.staggered_walked[index] += 1;
                }
            }
        }
        let frequency = self.session.qpc_frequency;
        self.stats
            .conversion
            .record_ticks(work.conversion, frequency);
        for (cost, ticks) in self.stats.conversion_packs.iter_mut().zip(pack_ticks) {
            if let Some(ticks) = ticks {
                cost.record_ticks(ticks, frequency);
            }
        }
        if let Some(ticks) = work.frame_encoding {
            self.stats.frame_encoding.record_ticks(ticks, frequency);
        }
        if let Some(ticks) = work.record_writes {
            self.stats.record_writes.record_ticks(ticks, frequency);
        }
        result?;
        if let Some(publication) = event_feed_publication {
            crate::event_feed_observer::accepted(&publication)?;
        }
        if let Some(publication) = operation_publication {
            publication.commit();
        }
        if let Some(publication) = map_event_publication {
            publication.commit();
        }
        self.retry_full |= retry_full || work.validation_refused;
        self.unpublished_families |= work.validation_families;
        self.unknown_pending = 0;
        self.suppress_origin = false;
        // The status names the viewed country's flag operands from rows just
        // committed; nothing is read from the game for it.
        let viewed = self
            .campaigns
            .get_mut(&id)
            .and_then(|campaign| viewed_flag(&campaign.encoder));
        set_viewed(&mut self.stats.player, viewed)?;
        let stats = &mut self.stats;
        stats.hours += 1;
        stats.campaign = Some(engine_allocation::string(&id)?);
        stats.last_hour = Some(clock.hour);
        sampled.record(stats);
        if let Some((old_id, old_branch, hour, previous_qpc)) = &stats.previous
            && old_id == &id
            && *old_branch == branch
            && hour.checked_add(1) == Some(clock.hour)
            && qpc > *previous_qpc
        {
            stats.interval_hours += 1;
            stats.interval_qpc += (qpc - previous_qpc) as u64;
        }
        stats.previous = Some((id, branch, clock.hour, qpc));
        if crate::allocation::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        Ok(())
    }

    pub fn flush(&mut self) -> io::Result<()> {
        let _details = replay_data::codec::error_details_scope();
        let _phase = crate::allocation::enter(crate::allocation::Phase::Spool);
        crate::allocation::attempted_hour(None);
        if crate::allocation::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        let mut first_error = None;
        for (id, campaign) in &mut self.campaigns {
            crate::allocation::set_context(crate::allocation::Context {
                campaign_id: id.as_bytes().try_into().ok(),
                branch: campaign.tracker.current_branch(),
                load_signal: None,
                failed_hour: None,
            });
            if let Err(error) = campaign.spool.sync_if_due() {
                self.stats.order_counters.invalidate_write(id, &error);
                campaign.disk_branch = None;
                campaign.encoder = FrameEncoder::new();
                first_error.get_or_insert(error);
            }
        }
        if let Some(error) = first_error {
            self.error(&error);
            Err(error)
        } else {
            Ok(())
        }
    }

    pub fn detach(&mut self) -> io::Result<()> {
        let _details = replay_data::codec::error_details_scope();
        let _phase = crate::allocation::enter(crate::allocation::Phase::Teardown);
        crate::allocation::attempted_hour(None);
        if crate::allocation::failed() {
            for (_, campaign) in &mut self.campaigns {
                let _ = campaign.spool.finish_segment();
            }
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        let mut first_error = None;
        let summary_campaign = self
            .stats
            .campaign
            .as_deref()
            .or_else(|| self.campaigns.keys().next_back().map(String::as_str))
            .and_then(|id| <[u8; 36]>::try_from(id.as_bytes()).ok());
        for (id, campaign) in &mut self.campaigns {
            crate::allocation::set_context(crate::allocation::Context {
                campaign_id: id.as_bytes().try_into().ok(),
                branch: campaign.tracker.current_branch(),
                load_signal: None,
                failed_hour: None,
            });
            let unknown = if summary_campaign
                .as_ref()
                .is_some_and(|summary| summary.as_slice() == id.as_bytes())
            {
                self.unknown_pending
            } else {
                0
            };
            match campaign.detach(unknown) {
                Ok(()) if unknown != 0 => self.unknown_pending = 0,
                Ok(()) => {}
                Err(error) => {
                    self.stats.order_counters.invalidate_write(id, &error);
                    first_error.get_or_insert(error);
                }
            }
        }
        if self.unknown_pending != 0 {
            first_error
                .get_or_insert_with(|| invalid("unattributed clock losses remain at detach"));
        }
        if let Some(error) = first_error {
            self.error(&error);
            Err(error)
        } else {
            Ok(())
        }
    }

    /// What the campaigns' commits no longer hold, for freeing off this thread.
    pub(crate) fn take_retired(&mut self) -> Vec<replay_frame::frame::Retired> {
        let mut retired = Vec::new();
        for (_, campaign) in &mut self.campaigns {
            if retired.is_empty() {
                retired = std::mem::take(&mut campaign.retired);
            } else if engine_allocation::reserve(&mut retired, campaign.retired.len()).is_ok() {
                retired.append(&mut campaign.retired);
            } else {
                campaign.retired.clear();
            }
        }
        retired
    }

    pub fn take_retry_full(&mut self) -> bool {
        std::mem::take(&mut self.retry_full)
    }

    /// Sliced families (bit per [`crate::slicing::FAMILIES`] index) whose
    /// walked read conversion or merging withheld since the last call.
    pub fn take_unpublished_families(&mut self) -> u32 {
        std::mem::take(&mut self.unpublished_families)
    }

    /// The campaign directories whose `spool.*` files `status.json` reports
    /// as `spool_bytes`. The status writer measures them off this thread.
    pub fn spool_directories(&self) -> io::Result<Vec<std::path::PathBuf>> {
        let _details = replay_data::codec::error_details_scope();
        let mut directories = engine_allocation::vec(self.campaigns.0.len())?;
        for campaign in self.campaigns.values() {
            directories.push(campaign.directory.clone());
        }
        Ok(directories)
    }

    /// Status without file-system I/O; `spool_bytes` is added by the status
    /// writer (`status_writer::StatusHandle`).
    #[cfg(test)]
    pub fn status(&self) -> io::Result<impl Serialize + '_> {
        self.status_with_work(status!({}))
    }

    pub(crate) fn recorded_hours(&self) -> u64 {
        self.stats.hours
    }

    #[cfg(test)]
    pub(crate) fn allocation_failure_publication_state(
        &self,
        last_hour: i32,
    ) -> Option<(BaselineToken, (u32, u64))> {
        let campaign = self.campaigns.values().next()?;
        let branch = campaign.tracker.current_branch()?;
        let (baseline, _) = campaign.encoder.comparison_baseline::<tables::Campaign>(
            branch,
            last_hour.checked_add(1)?,
            false,
        )?;
        Some((baseline, campaign.spool.committed_watermark()?))
    }

    pub(crate) fn status_with_work<'a, T: crate::status::Fields + 'a>(
        &'a self,
        work: crate::status::Object<T>,
    ) -> io::Result<impl Serialize + 'a> {
        let _details = replay_data::codec::error_details_scope();
        let _phase = crate::allocation::enter(crate::allocation::Phase::Status);
        crate::allocation::attempted_hour(None);
        if crate::allocation::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        let stats = &self.stats;
        let callback = stats.callback.try_percentiles()?;
        let callback_work = stats.callback.status()?;
        let country_cache_cost = stats.country_cache_cost.status()?;
        let conversion = stats.conversion.status()?;
        let frame_encoding = stats.frame_encoding.status()?;
        let record_writes = stats.record_writes.status()?;
        let country_rosters = stats.country_rosters.status()?;
        // Per-family copy-only mirrors, built outside the status `json!` so
        // the macro stays under its recursion limit as families move over.
        let research_walk_ns = [
            stats.table_ns[replay_data::rows::TableId::ResearchTechnologies.index()],
            stats.table_ns[replay_data::rows::TableId::ResearchSlots.index()],
            stats.table_ns[replay_data::rows::TableId::TechnologyDefinitions.index()],
        ];
        let stability_walk_ns = stats.stability_walk_ticks.map(|ticks| {
            (u128::from(ticks) * 1_000_000_000)
                .checked_div(u128::from(self.session.qpc_frequency))
                .unwrap_or(0)
                .min(u128::from(u64::MAX)) as u64
        });
        let relations_copy_ns = [
            stats.table_ns[replay_data::rows::TableId::RelationOpinions.index()],
            stats.table_ns[replay_data::rows::TableId::RelationTuples.index()],
        ];
        let characters_copy_ns = [
            stats.table_ns[replay_data::rows::TableId::Characters.index()],
            stats.table_ns[replay_data::rows::TableId::CountryLeaderRoles.index()],
            stats.table_ns[replay_data::rows::TableId::AdvisorRoles.index()],
            stats.table_ns[replay_data::rows::TableId::UnitLeaders.index()],
            stats.table_ns[replay_data::rows::TableId::UnitLeaderTraits.index()],
        ];
        let projects_copy_ns = decode::PROJECTS.map(|table| stats.table_ns[table.index()]);
        let lexical_copy_ns = [stats.table_ns[replay_data::rows::TableId::LexicalTokens.index()]];
        let variants_walk_ns = [
            stats.table_ns[replay_data::rows::TableId::VariantDefinitions.index()],
            stats.table_ns[replay_data::rows::TableId::VariantModules.index()],
            stats.table_ns[replay_data::rows::TableId::VariantStats.index()],
            stats.table_ns[replay_data::rows::TableId::VariantUpgrades.index()],
        ];
        // The shared state/province walk is charged to StateBuildings; the
        // hourly detection walk beside it is the third entry.
        let buildings_walk_ns = [
            stats.table_ns[replay_data::rows::TableId::StateBuildings.index()],
            stats.table_ns[replay_data::rows::TableId::ProvinceBuildings.index()],
            stats.table_ns[replay_data::rows::TableId::RaidTargetStatuses.index()],
        ];
        // The owned war-vector acquisition is charged to WarRelations.
        let wars_walk_ns = [stats.table_ns[replay_data::rows::TableId::WarRelations.index()]];
        let divisions_copy_ns = [
            stats.table_ns[decode::DIVISIONS[0].index()],
            stats.table_ns[decode::DIVISIONS[1].index()],
            stats.table_ns[decode::DIVISIONS[2].index()],
            stats.table_ns[decode::DIVISIONS[3].index()],
            stats.table_ns[decode::DIVISIONS[4].index()],
            stats.table_ns[decode::DIVISIONS[5].index()],
        ];
        let production_rest_copy_ns = [
            stats.table_ns[decode::PRODUCTION_REST[0].index()],
            stats.table_ns[decode::PRODUCTION_REST[1].index()],
            stats.table_ns[decode::PRODUCTION_REST[2].index()],
            stats.table_ns[decode::PRODUCTION_REST[3].index()],
            stats.table_ns[decode::PRODUCTION_REST[4].index()],
            stats.table_ns[decode::PRODUCTION_REST[5].index()],
            stats.table_ns[decode::PRODUCTION_REST[6].index()],
            stats.table_ns[decode::PRODUCTION_REST[7].index()],
        ];
        let construction_rest_copy_ns = [
            stats.table_ns[decode::CONSTRUCTION_REST[0].index()],
            stats.table_ns[decode::CONSTRUCTION_REST[1].index()],
            stats.table_ns[decode::CONSTRUCTION_REST[2].index()],
            stats.table_ns[decode::CONSTRUCTION_REST[3].index()],
            stats.table_ns[decode::CONSTRUCTION_REST[4].index()],
            stats.table_ns[decode::CONSTRUCTION_REST[5].index()],
            stats.table_ns[decode::CONSTRUCTION_REST[6].index()],
            stats.table_ns[decode::CONSTRUCTION_REST[7].index()],
            stats.table_ns[decode::CONSTRUCTION_REST[8].index()],
            stats.table_ns[decode::CONSTRUCTION_REST[9].index()],
        ];
        // Map-pack coupled unit, wing-fed air pair, source-fed naval block,
        // players pair and session family: game-thread copy-only mirrors.
        // Each mirrors its `table_ns` slots (copy-only ns for moved
        // families; `decode_ns` lives in the decode calibration, so this is
        // not comparable across threads — see the PLAN appendix).
        let map_walk_ns = [
            stats.table_ns[replay_data::rows::TableId::LandCombats.index()],
            stats.table_ns[replay_data::rows::TableId::Fleets.index()],
            stats.table_ns[replay_data::rows::TableId::TaskForces.index()],
            stats.table_ns[replay_data::rows::TableId::Ships.index()],
            stats.table_ns[replay_data::rows::TableId::NavalCombats.index()],
            stats.table_ns[replay_data::rows::TableId::NavalCombatParticipants.index()],
            stats.table_ns[replay_data::rows::TableId::AirWings.index()],
            stats.table_ns[replay_data::rows::TableId::ArmyOrders.index()],
        ];
        let air_wing_copy_ns = [
            stats.table_ns[decode::AIR_WING[0].index()],
            stats.table_ns[decode::AIR_WING[1].index()],
        ];
        let air_stats_copy_ns = [stats.table_ns[decode::AIR_STATS[0].index()]];
        let naval_copy_ns = [
            stats.table_ns[decode::NAVAL_ORDER[0].index()],
            stats.table_ns[decode::NAVAL_ORDER[1].index()],
            stats.table_ns[decode::NAVAL_ORDER[2].index()],
            stats.table_ns[decode::NAVAL_ORDER[3].index()],
            stats.table_ns[decode::NAVAL_ORDER[4].index()],
            stats.table_ns[decode::NAVAL_ORDER[5].index()],
            stats.table_ns[decode::NAVAL_ORDER[6].index()],
            stats.table_ns[decode::NAVAL_ORDER[7].index()],
            stats.table_ns[decode::NAVAL_ORDER[8].index()],
        ];
        let players_copy_ns = [
            stats.table_ns[decode::PLAYERS[0].index()],
            stats.table_ns[decode::PLAYERS[1].index()],
        ];
        let session_copy_ns = [
            stats.table_ns[decode::SESSION[0].index()],
            stats.table_ns[decode::SESSION[1].index()],
            stats.table_ns[decode::SESSION[2].index()],
        ];
        let mut background_sync = engine_allocation::vec(self.campaigns.0.len())?;
        for (id, campaign) in self.campaigns.iter() {
            if let Some(work) = campaign.spool.background_sync_work() {
                background_sync.push(status!({
                    "campaign": id.as_str(), "requested": work.requested, "completed": work.completed,
                    "pending": work.pending, "total_ns": work.total_ns, "max_ns": work.max_ns,
                    "threads": work.threads,
                }));
            }
        }
        let spool_work = status!({
                "conversion": conversion,
                "conversion_packs": sequence(crate::extended::PACK_NAMES.iter().zip(&stats.conversion_packs)
                    .map(|(name, cost)| status!({"name": name, "cost": cost.status()}))),
                "conversion_packs_scope": "cumulative calls, total and maximum wall per pack conversion, on the spool thread or, for packs with a helper marker, a conversion helper; nested within conversion; excludes core and map tables",
                "frame_encoding": frame_encoding,
                "record_writes": record_writes,
                "table_rows_storage_bytes": std::mem::size_of::<TableRows>(),
                "background_sync": {
                    "scope": "current open segments; periodic durability worker wall, overlapping normal spool work; completed includes failed requests; threads counts the segment's persistent worker, started once and reused by every request; forced barriers remain in record_writes",
                    "segments": owned_sequence(background_sync),
                },
                "native_projection": sequence(self.campaigns.values().map(|campaign| campaign.projection_scratch.status())),
                "comparison": self.comparison.status(),
                "lexical_conversion": lexical_conversion_status(self.campaigns.values().map(|campaign| &campaign.lexical_conversion)),
                "intel_shadow": intel_shadow_status(self.campaigns.values().map(|campaign| &campaign.intel_shadow)),
                "character_shadow": character_shadow_status(self.campaigns.values().map(|campaign| &campaign.character_shadow)),
                "phases": crate::spool_phases::SPOOL.status(self.session.qpc_frequency),
                "helper_phases": crate::spool_phases::HELPER.status(self.session.qpc_frequency),
                "sizing_phases": crate::spool_phases::SIZING.status(self.session.qpc_frequency),
                "conversion_helper": self.helper.is_some(),
                "conversion_helpers": usize::from(self.helper.is_some()) + usize::from(self.second_helper.is_some()),
                "conversion_helper_scope": "conversion_helper: whether a persistent helper converts the map tables and the helper packs beside the spool thread; conversion_helpers: how many started, the second converting lane 1 of the helper packs; with none the spool thread converts every table itself",

        }).join(work);
        let value = status!({
            "order_instance_counters": self.stats.order_counters.status(),
            "slices": status_map(crate::slicing::FAMILIES.iter().enumerate().filter(|(index, _)| stats.slices[*index].frames != 0).map(move |(index, family)| {
                (family.name, stats.slices[index].status(self.session.qpc_frequency).join(status!({"owner": DebugValue(family.owner)})))
            })),
            "campaign": &stats.campaign, "hours": stats.hours, "last_hour": stats.last_hour,
            "player": &stats.player,
            "total_ns": stats.total_ns, "p50_ns": callback.p50_ns, "p95_ns": callback.p95_ns,
            "callback_work": callback_work,
            "callback_tail": stats.callback_tail.status(self.session.qpc_frequency),
            "callback_work_scope": "cumulative per-hour game-thread acquisition body wall inside the hook since attach (the block before the trampoline), excluding the original native tick; equals acquisition_work.collect_wait_ns per hour. Drops, failed frames and unattributed startup callbacks excluded; sampled asynchronously after decode, not a CPU counter or exact partition with native_tick_work",
            "max_ns": callback.max_ns, "table_ns": stats.table_ns.as_slice(),
            "table_total_ns": stats.table_total_ns.as_slice(),
            "table_total_ns_scope": "cumulative reported acquisition costs for successfully processed recorded frames; same cohort as callback_work; skipped tables contribute zero, shared reads retain existing anchor charges, worker costs overlap and are not additive callback wall time",
            "table_reads": sequence(stats.table_reads.iter().copied().map(read_counts_status)),
            "table_total_reads": sequence(stats.table_total_reads.iter().copied().map(read_counts_status)),
            "table_reads_recorded": stats.hours != 0,
            "table_reads_frames": stats.hours,
            "cadence_policy": "hourly map tables; daily slots; static once per branch",
            "static_tables": sequence(TableId::ALL.into_iter().filter(|table| replay_data::cadence::cadence(*table) == replay_data::cadence::Cadence::Static).map(DebugValue)),
            "daily_slots": sequence((0..24).map(|slot| status!({
                "slot": slot,
                "tables": sequence(TableId::ALL.into_iter().filter(move |table| replay_data::cadence::refresh_slot(*table) == Some(slot)).map(DebugValue))
            }))),
            "table_reads_scope": "exact per-table native acquisition read work for the latest successfully processed frame; indexed by TableId like table_ns; no observation until table_reads_recorded is true; skipped tables contribute zero; shared native work is charged once to its capture anchor; table failures can still incur reads",
            "table_read_counts_semantics": "native API calls include empty and faulted calls; spans and bytes count nonempty requested source ranges for the executed prefix plus the first failed or rejected source span; empty spans and unexecuted suffixes excluded; invalid destination layouts and short plan buffers count the call only; a plan ControlChanged stop excludes the next unattempted span; bytes are requested lengths, not claimed successfully transferred partial-fault bytes",
            "table_total_reads_scope": "cumulative read counters over the same successfully processed frames as table_total_ns and callback_work, including failed table attempts and fallbacks within those frames; dropped, failed or unpublished frames and startup callbacks excluded; totals saturate at u64::MAX; requested spans and bytes are read work, not elapsed time or retained memory",
            "staggered_skipped": stats.staggered_skipped.as_slice(),
            "staggered_walked": stats.staggered_walked.as_slice(),
            "hours_per_second": if stats.interval_qpc == 0 { 0.0 } else {
                stats.interval_hours as f64 * self.session.qpc_frequency as f64 / stats.interval_qpc as f64
            },
            "dropped_frames": stats.dropped_frames,
            "truncated_tables": stats.truncated_tables,
            "weather_display_reads": {
                "failures": stats.weather_display_failures,
                "last_failure": stats.weather_display_last_failure,
                "scope": "cumulative over processed frames: retained display boundary refused independently; province simulation and display-byte fields remain available",
            },
            "weather_suspend_reads": {
                "failures": stats.weather_suspend_failures,
                "last_failure": stats.weather_suspend_last_failure,
                "scope": "cumulative over processed frames: independent weather-manager suspension reads failed; other SimulationFeatures operands remain available and weather_suspend is absent",
            },
            "raid_reads": {
                "torn_memo_reads": stats.raid_torn_memo_reads,
                "refused_leader_rows": stats.raid_refused_leaders,
                "scope": "cumulative over processed frames: raid path memo copies rejected because a game writer held its lock or changed its header during the copy (each retried, the table unavailable after three), and raid, cooldown and raid-target rows refused because their leader arm resolved to an object whose captor is not proven",
            },
            "shared_read_groups": sequence(crate::extended::read_groups().map(move |(name, anchor, members)| {
                // Game-thread copy-only for moved families: `table_ns` for
                // moved tables carries copy ns only, while `decode_ns`
                // lives in the decode calibration (`runtime` status). Do not
                // compare `shared_read_groups` totals across moved vs unmoved
                // families (PLAN-appendix apples-to-oranges).
                let total_ns: u64 = members.iter().map(|table| stats.table_ns[table.index()]).sum();
                status!({"name": name, "anchor": DebugValue(anchor),
                    "members": sequence(members.iter().copied().map(DebugValue)),
                    "last_ns": total_ns})
            })),
            "country_authentication": {
                "entries": stats.country_cache_entries,
                "seed_status": DebugValue(stats.country_cache_status),
                "fallbacks": stats.country_cache_fallbacks,
                "cost": country_cache_cost,
                "accounted_in": "Countries",
            },
            "pool_bytes": stats.pool_bytes, "write_errors": stats.errors, "last_error": &stats.last_error,
            "validation_refusals": &stats.validation_refusals,
            "failed_hook_families": crate::hook_failures::status(),
            "input_demand_errors": stats.input_demand_errors,
            "input_demand_last_error": stats.input_demand_last_error,
            "sampled_buffer_allocation": {
                "scope": "last consumed native row buffer, before off-thread growth; not the whole pool",
                "bytes": stats.sampled_buffer_bytes,
                "parts": sequence(stats.sampled_buffer_parts.iter().map(|(name, bytes)| {
                    status!({"name": name, "bytes": bytes})
                })),
            },
            "unknown_losses_pending": self.unknown_pending,
            "spool_work": spool_work,
            "war_owned_snapshots": stats.war_snapshots.status(),
            "game_score": stats.game_score.status(),
            "division_naming": stats.division_naming.status(),
            "counter_tooltip_inputs": stats.counter_tooltip_inputs.status(),
            "country_rosters": country_rosters,
            "fresh_map_inputs": crate::status::status!({
                "states": stats.state_map_inputs.status(),
                "railway": stats.railway_map_inputs.status(),
                "physical_navies": stats.physical_navies.status(),
            }),
            "battle_plan_child_fronts": stats.battle_plan_child_fronts.status(),
            "battle_plan_execution": stats.battle_plan_execution.status(),
            "character_naming": stats.character_naming.status(),
            "supply_vehicles": stats.supply_vehicles.status(),
            "supply_node_walk": crate::extended::supply_trains::node_walk_data(),
            "script_variable_seeds": stats.variable_seeds.status(),
            "scientist_activity": crate::extended::projects::scientist_active_data(),
            "instant_decisions": crate::extended::visibility::instant_decisions_data(),
            "political_runtime_scalars": crate::extended::visibility::runtime_scalars_data(),
            "movement_defines": crate::extended::visibility::movement_defines_data(),
            "selected_ordered_enemies":crate::extended::visibility::ordered_enemy_data(),
            "unit_combat_references":crate::extended::unit_combat_references::data(),
            "air_wing_names": crate::map_capture::air_wing_name_data(),
            "ship_names": crate::map_capture::ship_name_data(),
            "railway_gun_names": crate::extended::railway_guns::name_data(),
            "railway_gun_eta": crate::extended::railway_guns::eta_data(),
            "railway_gun_owner_transport": crate::extended::railway_guns::transport_data(),
            "division_speed_terms": crate::extended::division_presentation::speed_term_data(),
            "division_template_speed_keys": crate::extended::division_templates::speed_key_data(),
            "division_movement_arrows": crate::extended::division_presentation::movement_arrow_data(),
            "railway_gun_movement_arrows": crate::extended::railway_guns::movement_arrow_data(),
            "task_force_movement_arrows": crate::extended::naval_counters::movement_arrow_data(),
            "task_force_fresh_eta": crate::extended::naval_counters::fresh_eta_data(),
            "task_force_counter_phases": crate::extended::naval_counters::phase_data(),
            "naval_combat_member_phases": crate::extended::naval_details::member_phase_data(),
            "transport_subject_access": crate::extended::visibility::transport_subject_data(),
            "ship_model_operands": crate::extended::ship_models::optional_data(),
            "country_original_tags": crate::extended::country_appearance::original_tag_data(),
            "reserved_reverse_visibility": stats.reserved_visibility.status(),
            "reserved_land_theatres": crate::extended::naval_history::reserved_land_data(),
            "orders_group_tooltip_inputs": crate::extended::group_counters::tooltip_data(),
            "research_walk_ns": research_walk_ns,
            "stability_walk_ns": stability_walk_ns,
            "relations_copy_ns": relations_copy_ns,
            "characters_copy_ns": characters_copy_ns,
            "projects_copy_ns": projects_copy_ns,
            "lexical_copy_ns": lexical_copy_ns,
            "variants_walk_ns": variants_walk_ns,
            "buildings_walk_ns": buildings_walk_ns,
            "wars_walk_ns": wars_walk_ns,
            "divisions_copy_ns": divisions_copy_ns,
            "production_rest_copy_ns": production_rest_copy_ns,
            "construction_rest_copy_ns": construction_rest_copy_ns,
            "map_walk_ns": map_walk_ns,
            "air_wing_copy_ns": air_wing_copy_ns,
            "air_stats_copy_ns": air_stats_copy_ns,
            "naval_copy_ns": naval_copy_ns,
            "players_copy_ns": players_copy_ns,
            "session_copy_ns": session_copy_ns,
        });
        Ok(value)
    }

    #[cfg(test)]
    fn json(&self) -> Value {
        serde_json::to_value(self.status().unwrap()).unwrap()
    }
}

fn prepare_campaigns<'a>(
    ids: impl IntoIterator<Item = &'a [u8]>,
    mut open: impl FnMut(&str) -> io::Result<()>,
) -> io::Result<()> {
    let mut seen = Vec::new();
    for bytes in ids {
        // Malformed identities cannot become valid through an I/O retry. Their
        // actual consumption emits an unattributed loss and a named error.
        let id = match campaign_name(bytes) {
            Ok(id) => id,
            Err(error) if error.kind() == io::ErrorKind::OutOfMemory => return Err(error),
            Err(_) => continue,
        };
        if !seen.iter().any(|previous: &String| previous == &id) {
            open(&id)?;
            engine_allocation::reserve(&mut seen, 1)?;
            seen.push(id);
        }
    }
    Ok(())
}

pub(super) fn campaign_name(bytes: &[u8]) -> io::Result<String> {
    if !is_campaign_uuid(bytes) {
        return Err(invalid(
            "campaign id is not a UUID; refusing an unsafe output path",
        ));
    }
    // Preserve the engine's spelling, including case, in both the path and row.
    engine_allocation::string(std::str::from_utf8(bytes).expect("validated UUID is ASCII"))
}

pub(crate) use crate::table_conversion::{table, table_iter, unavailable};

/// The controller-area node a province row holds
/// ([`engine_native::cprovince::Row::area_node`]): the area it labels.
fn area_node(
    nodes: &engine_native::controller_areas::Output,
    province: &engine_native::cprovince::Row,
) -> Option<replay_data::rows::ControllerAreaNode> {
    let node = nodes.nodes.get(province.area_node? as usize)?;
    Some(replay_data::rows::ControllerAreaNode {
        first_land: node.first_land,
        neighbours: nodes.neighbours(node).map(<[u32]>::to_vec),
    })
}

/// [`area_node`] compared with a baseline row without building it.
fn area_node_equal(
    nodes: &engine_native::controller_areas::Output,
    province: &engine_native::cprovince::Row,
    previous: &ProvinceRow,
) -> bool {
    let current = province
        .area_node
        .and_then(|index| nodes.nodes.get(index as usize));
    match (&previous.area_node, current) {
        (replay_data::rows::Recorded::Value(None), None) => true,
        (replay_data::rows::Recorded::Value(Some(previous)), Some(node)) => {
            previous.first_land == node.first_land
                && previous.neighbours.as_deref() == nodes.neighbours(node)
        }
        _ => false,
    }
}

/// States with both display names from the frame's name arena; a span outside
/// the bytes the walk copied denies the table.
fn state_table(
    captured: &Captured,
    projection: Option<&mut crate::projection::Projection<'_>>,
) -> replay_frame::frame::TableInput {
    let copied = captured
        .state_names
        .get(..captured.state_names_len)
        .filter(|_| captured.states_result.len() <= captured.states.len());
    let Some(names) = copied else {
        return unavailable("state name arena length invalid").into();
    };
    let text = |span: engine_native::cstate::NameSpan| {
        span.start
            .checked_add(span.len)
            .and_then(|end| names.get(span.start..end))
    };
    if captured.states[..captured.states_result.len()]
        .iter()
        .any(|row| text(row.custom_name).is_none() || text(row.owner_name).is_none())
    {
        return unavailable("state name lies outside the captured byte arena").into();
    }
    scalar_table::<tables::States, _>(
        projection,
        captured.states_result,
        &captured.states,
        |state| u128::from(state.id),
        |state| StateRow {
            id: state.id,
            owner: state.owner,
            controller: state.controller,
            movement_infrastructure: state.movement_infrastructure,
            continent: replay_data::rows::Recorded::Value(state.continent),
            manpower_available: state.manpower_available,
            manpower_locked: replay_data::rows::Recorded::Value(state.manpower_locked),
            population: replay_data::rows::Recorded::Value(state.population),
            best_vp_index: replay_data::rows::Recorded::Value(state.best_vp_index),
            single_controller: replay_data::rows::Recorded::Value(
                state.single_controller.map(u32::from),
            ),
            category: replay_data::rows::Recorded::Value(state.category),
            railway_infrastructure: replay_data::rows::Recorded::Value(
                captured.state_inputs.fraction(state.id),
            ),
            custom_name: engine_allocation::owned_vec(engine_allocation::copy_vec(
                text(state.custom_name).unwrap_or_default(),
            )),
            owner_name: engine_allocation::owned_vec(engine_allocation::copy_vec(
                text(state.owner_name).unwrap_or_default(),
            )),
        },
    )
}

fn faction_table(captured: &Captured) -> TableView {
    if captured.faction_names_len > captured.faction_names.len()
        || captured.factions_result.len() > captured.factions.len()
        || captured.factions[..captured.factions_result.len()]
            .iter()
            .any(|row| {
                row.name_start
                    .checked_add(row.name_len)
                    .is_none_or(|end| end > captured.faction_names_len)
            })
    {
        return unavailable("faction name lies outside the captured byte arena");
    }
    table::<tables::Factions, _>(captured.factions_result, &captured.factions, |faction| {
        FactionRow {
            id: faction.id,
            name: engine_allocation::owned_vec(engine_allocation::copy_vec(
                &captured.faction_names[faction.name_start..faction.name_start + faction.name_len],
            )),
            template_token: faction.template_token,
            color_red: faction.color[0],
            color_green: faction.color[1],
            color_blue: faction.color[2],
            color_alpha: faction.color[3],
            color_overridden: faction.color_overridden,
        }
    })
}

#[cfg(test)]
fn sampled_frame(source: FrameInput) -> replay_frame::frame::DecodedFrame {
    replay_frame::frame::DecodedFrame {
        branch: source.branch,
        hour: source.hour,
        metrics: source.metrics,
        tables: source
            .tables
            .map(|input| match input {
                TableInput::Sampled(view) => view,
                TableInput::SharedComplete(rows) => TableView::Complete((*rows).clone()),
                TableInput::ComparedEqual(_)
                | TableInput::ComparedChanges { .. }
                | TableInput::SliceUpdate { .. } => {
                    panic!("fixture expected ordinary sampled rows")
                }
            })
            .map(Into::into),
    }
}

#[cfg(test)]
fn decoded(
    captured: &Captured,
    clock: Clock,
    branch: BranchId,
) -> replay_frame::frame::DecodedFrame {
    sampled_frame(
        decoded_with_costs(
            captured,
            clock.borrowed(),
            branch,
            &mut SpoolConversion {
                variables_baseline: None,
                variables_scratch: &mut ScriptVariableComparison::default(),
                characters: None,
                lexical: None,
                intel: None,
                projection: None,
                helper: None,
            },
        )
        .0,
    )
}

fn decoded_with_costs(
    captured: &Captured,
    clock: Clock<&[u8]>,
    branch: BranchId,
    conversion: &mut SpoolConversion<'_, '_>,
) -> (FrameInput, [Option<i64>; crate::extended::PACK_NAMES.len()]) {
    let mut pack_ticks = [None; crate::extended::PACK_NAMES.len()];
    let metrics = Metrics {
        total_ns: captured.total_ns,
        table_ns: captured.table_ns,
        pool_bytes: captured.pool_bytes,
        dropped_frames: captured.dropped_frames,
    };
    let failure = captured
        .clock
        .is_none()
        .then_some("clock unreadable at hook entry");
    let span = crate::spool_phases::SPOOL.span(crate::spool_phases::Phase::ConvertCore);
    let tables = if let Some(reason) = failure {
        unavailable_tables(reason)
    } else {
        let mut tables = unavailable_tables("table was not acquired");
        match conversion.helper {
            None => {
                let map = crate::map_spool::table_inputs(
                    &captured.map,
                    conversion.projection.as_deref_mut(),
                );
                let core = core_tables(captured, &clock, conversion.projection.as_deref_mut());
                for (destination, value) in tables.iter_mut().zip(core.into_iter().chain(map)) {
                    *destination = value;
                }
                tables[TableId::BorderConflicts.index()] = border_conflicts_table(captured).into();
                let _packs = span.switch(crate::spool_phases::Phase::ConvertPacks);
                pack_ticks = captured
                    .extended
                    .insert_tables(clock.hour, &mut tables, conversion, false)
                    .map(Some);
            }
            Some(helpers) => {
                pack_ticks =
                    convert_with_helper(captured, &clock, conversion, helpers, &mut tables, span);
            }
        }
        tables[TableId::StateResistance.index()] = captured.state_inputs.occupation.table().into();
        for (table, value) in crate::faction_companions::TABLES
            .into_iter()
            .zip(captured.faction_companions.inputs())
        {
            tables[table.index()] = value;
        }
        for (table, value) in crate::faction_goals::TABLES
            .into_iter()
            .zip(captured.faction_goals.inputs())
        {
            tables[table.index()] = value;
        }
        captured
            .map
            .naval
            .management
            .insert(collector_naval_management::forces::TABLES, &mut tables);
        tables
    };
    let mut input = FrameInput {
        branch,
        hour: clock.hour,
        tables,
        metrics,
    };
    // Every converted table's stepped fields are rounded to their display
    // steps (replay_data::display) before merges, carries and comparisons read
    // them, so a row that drifted only below the game's display precision
    // equals its baseline. Projected rows arrive rounded (crate::projection);
    // a table still shared with its producer's cache is rounded only while
    // this hour holds its sole reference, and the encoder rounds the rest.
    input.round_to_display();
    (input, pack_ticks)
}

/// Clock through FactionMembers: the core tables before the map tables.
fn core_tables(
    captured: &Captured,
    clock: &Clock<&[u8]>,
    mut projection: Option<&mut crate::projection::Projection<'_>>,
) -> [TableInput; 10] {
    let counters = captured.counters;
    [
        table::<tables::Clock, _>(
            Outcome::Rows { len: 1 },
            std::slice::from_ref(clock),
            |clock| ClockRow {
                hour: clock.hour,
                play_time: clock.play_time,
                play_session_started_at: clock.play_session_started_at,
                seed: clock.seed,
                speed: clock.speed,
                debug_current_ref_id: counters.and_then(|c| c.debug_current_ref_id),
                unit: counters.and_then(|c| c.unit),
                order_index: counters.and_then(|c| c.order_index),
                front_index: counters.and_then(|c| c.front_index),
                theatre_index: counters.and_then(|c| c.theatre_index),
                theater_group_index: counters.and_then(|c| c.theater_group_index),
                military_deployment_line_index: counters
                    .and_then(|c| c.military_deployment_line_index),
                military_deployment_conveyor_index: counters
                    .and_then(|c| c.military_deployment_conveyor_index),
                equipment_variant_index: counters.and_then(|c| c.equipment_variant_index),
                country_leader_index: counters.and_then(|c| c.country_leader_index),
                navy_id: counters.and_then(|c| c.navy_id),
                land_combat_id: counters.and_then(|c| c.land_combat_id),
                multiplayer_random_count: counters.and_then(|c| c.multiplayer_random_count),
            },
        )
        .into(),
        table::<tables::Campaign, _>(
            Outcome::Rows { len: 1 },
            std::slice::from_ref(clock),
            |clock| CampaignRow {
                id: engine_allocation::owned_vec(engine_allocation::copy_vec(clock.campaign_id)),
            },
        )
        .into(),
        project_table::<tables::Countries, _>(
            projection.as_deref_mut(),
            captured.countries_result,
            &captured.countries,
            |country| u128::from(country.id),
            |country, previous| {
                country.id == previous.id
                    && country.canonical_slot == previous.canonical_slot
                    && country.tag.as_bytes() == previous.tag
                    && country.political_power_raw == previous.political_power_raw
                    && country.command_power_raw == previous.command_power_raw
                    && country.stability_raw == previous.stability_raw
                    && country.war_support_raw == previous.war_support_raw
                    && country.convoys_raw == previous.convoys_raw
                    && country.manpower_residual == previous.manpower_residual
                    && replay_data::rows::Recorded::Value(country.border_land_doctrine_level)
                        == previous.border_land_doctrine_level
                    && country.peace_conference_participant == previous.peace_conference_participant
            },
            |country| CountryRow {
                id: country.id,
                canonical_slot: country.canonical_slot,
                tag: engine_allocation::owned_vec(engine_allocation::copy_vec(
                    country.tag.as_bytes(),
                )),
                political_power_raw: country.political_power_raw,
                command_power_raw: country.command_power_raw,
                stability_raw: country.stability_raw,
                war_support_raw: country.war_support_raw,
                convoys_raw: country.convoys_raw,
                peace_conference_participant: country.peace_conference_participant,
                manpower_residual: country.manpower_residual,
                border_land_doctrine_level: replay_data::rows::Recorded::Value(
                    country.border_land_doctrine_level,
                ),
            },
        ),
        scalar_table::<tables::CountryLinks, _>(
            projection.as_deref_mut(),
            captured.links_result,
            &captured.links,
            |link| u128::from(link.id),
            |link| CountryLinkRow {
                id: link.id,
                canonical_slot: link.canonical_slot,
            },
        ),
        project_table::<tables::Provinces, _>(
            projection.as_deref_mut(),
            captured.provinces_result,
            &captured.provinces.rows,
            |province| u128::from(province.id),
            |province, previous| {
                province.id == previous.id
                    && province.state == previous.state
                    && province.owner == previous.owner
                    && province.controller == previous.controller
                    && province.victory_points == previous.victory_points
                    && province.fort_positive == previous.fort_positive
                    && province.move_gate == previous.move_gate
                    && province.port_sea == previous.port_sea
                    && replay_data::rows::Recorded::Value(province.fort_bunker)
                        == previous.fort_bunker
                    && previous.selected_areas
                        == replay_data::rows::Recorded::Value(replay_data::rows::Retired)
                    && previous.attack
                        == replay_data::rows::Recorded::Value(replay_data::rows::Witnessed(None))
                    && previous.controller_area
                        == replay_data::rows::Recorded::Value(province.controller_area)
                    && previous.owner_area
                        == replay_data::rows::Recorded::Value(province.owner_area)
                    && area_node_equal(&captured.provinces.area_nodes, province, previous)
            },
            |province| ProvinceRow {
                id: province.id,
                state: province.state,
                owner: province.owner,
                controller: province.controller,
                victory_points: province.victory_points,
                fort_positive: province.fort_positive,
                move_gate: province.move_gate,
                port_sea: province.port_sea,
                fort_bunker: replay_data::rows::Recorded::Value(province.fort_bunker),
                selected_areas: replay_data::rows::Recorded::Value(replay_data::rows::Retired),
                // No province operand is witnessed; current writers leave it empty.
                attack: replay_data::rows::Recorded::Value(replay_data::rows::Witnessed(None)),
                controller_area: replay_data::rows::Recorded::Value(province.controller_area),
                owner_area: replay_data::rows::Recorded::Value(province.owner_area),
                area_node: replay_data::rows::Recorded::Value(area_node(
                    &captured.provinces.area_nodes,
                    province,
                )),
            },
        ),
        state_table(captured, projection.as_deref_mut()),
        scalar_table::<tables::Divisions, _>(
            projection.as_deref_mut(),
            captured.divisions_result,
            &captured.divisions,
            |division| u128::from(division.id),
            |division| DivisionRow {
                id: division.id,
                roster_owner: division.roster_owner,
                logical_owner: division.logical_owner,
                expeditionary_owner: division.expeditionary_owner,
                province: division.province,
                template: division.template,
                hit_points: division.hit_points,
                organisation: division.organisation,
                orders_group: division.orders_group,
                reveal_skip: division.reveal_skip,
            },
        ),
        scalar_table::<tables::WarRelations, _>(
            projection.as_deref_mut(),
            captured.wars_result,
            &captured.wars,
            |war| (u128::from(war.first_country) << 32) | u128::from(war.second_country),
            |war| WarRelationRow {
                first_country: war.first_country,
                second_country: war.second_country,
                start_hour: war.start_hour,
                end_hour: war.end_hour,
                cancel: war.cancel,
                first_casualties: war.first_casualties,
                second_casualties: war.second_casualties,
                first_unknown_casualties: war.first_unknown_casualties,
                second_unknown_casualties: war.second_unknown_casualties,
                first_was_instigator: war.first_was_instigator,
            },
        ),
        faction_table(captured).into(),
        scalar_table::<tables::FactionMembers, _>(
            projection,
            captured.faction_members_result,
            &captured.faction_members,
            |member| u128::from(member.country),
            |member| FactionMemberRow {
                country: member.country,
                faction: member.faction,
                ordinal: member.ordinal,
            },
        ),
    ]
}

/// One hour's map tables and the helper packs converted by the helpers while
/// this thread converts the core tables and the other packs: the first helper
/// converts the map tables and lane 0, the second lane 1, or the first both
/// lanes when it is alone. Every table lands in the slot the serial order
/// fills, and the ordinary-job refusal is applied after every share merged,
/// so the frame is the serial one.
fn convert_with_helper(
    captured: &Captured,
    clock: &Clock<&[u8]>,
    conversion: &mut SpoolConversion<'_, '_>,
    helpers: crate::spool_helper::Helpers<'_>,
    tables: &mut [TableInput; TABLE_COUNT],
    span: crate::spool_phases::Span<'static>,
) -> [Option<i64>; crate::extended::PACK_NAMES.len()] {
    use crate::spool_phases::{HELPER, Phase};
    struct HelperOutput {
        map: Option<[TableInput; 10]>,
        packs: [Option<TableInput>; TABLE_COUNT],
        costs: [i64; crate::extended::PACK_NAMES.len()],
    }
    const EMPTY_OUTPUT: HelperOutput = HelperOutput {
        map: None,
        packs: [const { None }; TABLE_COUNT],
        costs: [0; crate::extended::PACK_NAMES.len()],
    };
    let projection = conversion
        .projection
        .as_ref()
        .map(|projection| **projection);
    let hour = clock.hour;
    let view = captured.extended.helper_view();
    let map = &captured.map;
    let context = crate::allocation::context();
    // The characters and intel packs convert on a helper lane with the spool
    // caches lent for this join; the spool thread converts the rest with its
    // other caches.
    let SpoolConversion {
        variables_baseline,
        variables_scratch,
        characters,
        lexical,
        intel,
        projection: own_projection,
        helper: _,
    } = conversion;
    let lent = std::sync::Mutex::new(crate::extended::LentCaches {
        characters: characters.take(),
        intel: intel.take(),
        variables_baseline: *variables_baseline,
        variables_scratch: Some(&mut **variables_scratch),
    });
    let mut spare_scratch = ScriptVariableComparison::default();
    let mut conversion = SpoolConversion {
        variables_baseline: None,
        variables_scratch: &mut spare_scratch,
        characters: None,
        lexical: lexical.as_deref_mut(),
        intel: None,
        projection: own_projection.as_deref_mut(),
        helper: None,
    };
    let lane = |lanes: u8, with_map: bool, output: &std::sync::Mutex<HelperOutput>| {
        let _phase = crate::allocation::enter(crate::allocation::Phase::Spool);
        crate::allocation::set_context(context);
        let caches = {
            let mut lent = lent
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let lane_a = lanes & 0b01 != 0;
            crate::extended::LentCaches {
                characters: if lanes & 0b10 != 0 {
                    lent.characters.take()
                } else {
                    None
                },
                intel: if lane_a { lent.intel.take() } else { None },
                variables_baseline: if lane_a {
                    lent.variables_baseline
                } else {
                    None
                },
                variables_scratch: if lane_a {
                    lent.variables_scratch.take()
                } else {
                    None
                },
            }
        };
        let mut projection = projection;
        // Fill the caller's output slot directly, so the complete pack array
        // is never copied through another helper-frame aggregate.
        let mut output = output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let span = HELPER.span(Phase::ConvertCore);
        output.map = with_map.then(|| crate::map_spool::table_inputs(map, projection.as_mut()));
        let span = span.switch(Phase::ConvertPacks);
        let HelperOutput { packs, costs, .. } = &mut *output;
        view.tables(hour, lanes, projection.as_mut(), caches, packs, costs);
        span.finish();
    };
    let first = std::sync::Mutex::new(EMPTY_OUTPUT);
    let second = helpers.second.map(|_| std::sync::Mutex::new(EMPTY_OUTPUT));
    let first_lanes = if helpers.second.is_some() { 0b01 } else { 0b11 };
    let first_job = || lane(first_lanes, true, &first);
    let second_job = || lane(0b10, false, second.as_ref().expect("second helper output"));
    let mut pack_ticks = [None; crate::extended::PACK_NAMES.len()];
    let own = || {
        let conversion = &mut conversion;
        let core = core_tables(captured, clock, conversion.projection.as_deref_mut());
        for (destination, value) in tables.iter_mut().zip(core) {
            *destination = value;
        }
        tables[TableId::BorderConflicts.index()] = border_conflicts_table(captured).into();
        let span = span.switch(Phase::ConvertPacks);
        pack_ticks = captured
            .extended
            .insert_tables(clock.hour, tables, conversion, true)
            .map(Some);
        span.switch(Phase::HelperWait)
    };
    let wait = match helpers.second {
        Some(second_helper) => helpers.first.join(helpers.backlogged, &first_job, || {
            second_helper.join(helpers.backlogged, &second_job, own)
        }),
        None => helpers.first.join(helpers.backlogged, &first_job, own),
    };
    wait.finish();
    for output in [Some(first), second].into_iter().flatten() {
        let HelperOutput { map, packs, costs } = output
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(map) = map {
            for (destination, value) in tables[TableId::Players.index()..].iter_mut().zip(map) {
                *destination = value;
            }
        }
        for (destination, value) in tables.iter_mut().zip(packs) {
            if let Some(value) = value {
                *destination = value;
            }
        }
        for (ticks, cost) in pack_ticks.iter_mut().zip(costs) {
            if cost != 0 {
                *ticks = Some(cost);
            }
        }
    }
    captured.extended.refuse_failed_ordinary(tables);
    pack_ticks
}

/// The BorderConflicts outcome after a Provinces walk: unavailable with it,
/// and truncated with it, since a province the walk did not reach recorded
/// no conflicts; otherwise the conflict walk's own outcome.
fn border_conflicts_outcome(provinces: Outcome, conflicts: Outcome) -> Outcome {
    match (provinces, conflicts) {
        (Outcome::Unavailable { reason }, _) | (_, Outcome::Unavailable { reason }) => {
            Outcome::Unavailable { reason }
        }
        (Outcome::Truncated { unread_rows, .. }, conflicts) => Outcome::Truncated {
            len: conflicts.len(),
            unread_rows: unread_rows.max(1).saturating_add(match conflicts {
                Outcome::Truncated { unread_rows, .. } => unread_rows,
                _ => 0,
            }),
        },
        (Outcome::Rows { .. }, conflicts) => conflicts,
    }
}

/// The border-conflict rows of the Provinces walk. They are complete only
/// when that walk completed, and fail alone after it did.
fn border_conflicts_table(captured: &Captured) -> TableView {
    let province = &captured.provinces;
    let outcome =
        border_conflicts_outcome(captured.provinces_result, province.conflict_walk.outcome);
    let Some(groups) = province.groups.get(..province.conflict_walk.groups_len) else {
        return unavailable("border conflict group arena length invalid");
    };
    let list = |r: &engine_native::cprovince::ConflictRow| {
        r.groups_start
            .checked_add(r.groups_len)
            .and_then(|end| groups.get(r.groups_start..end))
    };
    let rows = &province.conflicts[..outcome.len().min(province.conflicts.len())];
    if rows.iter().any(|r| list(r).is_none()) {
        return unavailable("border conflict groups outside captured arena");
    }
    table::<tables::BorderConflicts, _>(outcome, &province.conflicts, |r| BorderConflictRow {
        province: r.province,
        groups: engine_allocation::owned_vec(engine_allocation::copy_vec(list(r).unwrap())),
    })
}

fn unavailable_tables(reason: &'static str) -> [TableInput; TABLE_COUNT] {
    std::array::from_fn(|index| {
        TableInput::Sampled(match retired_reason(TableId::ALL[index]) {
            Some(retired) => unavailable(retired),
            None => unavailable(reason),
        })
    })
}

/// A retired table's catalogued refusal, which every recording decodes in its
/// place; no pack produces the table, so its frame view is this refusal.
fn retired_reason(table: TableId) -> Option<&'static str> {
    static REASONS: std::sync::OnceLock<[Option<String>; TABLE_COUNT]> = std::sync::OnceLock::new();
    REASONS.get_or_init(|| {
        let current = replay_data::format::FormatVersion::CURRENT.version();
        std::array::from_fn(|index| {
            let table = TableId::ALL[index];
            (!table.last_change().present(current))
                .then(|| replay_data::format::FormatVersion::CURRENT.table_refusal(table))
                .flatten()
        })
    })[table.index()]
    .as_deref()
}

#[cfg(test)]
pub(crate) fn spool_failure_fixture(root: &Path) -> io::Result<Processor> {
    Processor::new(root, PreparedCampaigns::default(), tests::session())
}

#[cfg(test)]
pub(crate) fn allocation_failure_frame(hour: i32) -> Captured {
    let mut frame = tests::captured(hour);
    let clock = frame.clock.as_mut().unwrap();
    let uuid = b"01234567-89ab-cdef-0123-456789abcdef";
    clock.campaign_id.len = uuid.len();
    clock.campaign_id.bytes[..uuid.len()].copy_from_slice(uuid);
    engine_allocation::resize_default(&mut frame.map.land_combats.rows, 1).unwrap();
    engine_allocation::resize_default(&mut frame.map.land_participants, 2).unwrap();
    engine_allocation::resize_default(&mut frame.map.land_classes, 2).unwrap();
    frame.map.land_combats.rows[0] = engine_native::cland_combat::Row {
        id: 1,
        attacker_start: 0,
        attacker_len: 1,
        defender_start: 1,
        defender_len: 1,
        ..Default::default()
    };
    frame.map.land_combats.outcome = Outcome::Rows { len: 1 };
    frame.map.land_participants[..2].copy_from_slice(&[1, 2]);
    frame.map.land_participants_len = 2;
    frame
}

#[cfg(test)]
mod byte_identity_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{container::Replay, container::SeekResult};
    use replay_data::{rows::Recorded, rows::TableId};
    use replay_frame::frame::DecodedFrame;

    #[test]
    fn the_viewed_country_is_published_from_committed_rows_and_copied_only_on_change() {
        use replay_data::country_flags::FlagOperands;
        // No committed rows: nothing to publish.
        assert_eq!(viewed_flag(&FrameEncoder::new()), None);
        let germany = FlagOperands {
            tag: "mod-country-long-tag",
            cosmetic: "",
            original: Some("mod-country-long-tag"),
            group: "synthetic_group",
        };
        let mut slot = None;
        set_viewed(&mut slot, Some(germany)).unwrap();
        assert_eq!(
            serde_json::to_value(&slot).unwrap(),
            json!({"tag":"mod-country-long-tag","cosmetic":"","original":"mod-country-long-tag","group":"synthetic_group"})
        );
        // The same operands an hour later keep the copied text.
        let copied = slot.as_ref().unwrap().tag.as_ptr();
        set_viewed(&mut slot, Some(germany)).unwrap();
        assert_eq!(slot.as_ref().unwrap().tag.as_ptr(), copied);
        // A new cosmetic tag or ruling group replaces it; an unread original stays unknown.
        let changed = FlagOperands {
            cosmetic: "variants/red",
            original: None,
            ..germany
        };
        set_viewed(&mut slot, Some(changed)).unwrap();
        assert_eq!(slot.as_ref().unwrap().cosmetic, "variants/red");
        assert_eq!(
            serde_json::to_value(&slot).unwrap()["original"],
            Value::Null
        );
        // Unavailable rows withdraw it rather than keep a stale country.
        set_viewed(&mut slot, None).unwrap();
        assert_eq!(slot, None);
    }

    #[test]
    fn war_snapshot_work_only_counts_sampled_passes_and_keeps_capacity_scope() {
        use engine_native::{
            cwar_relation::OwnedSnapshotCapacity, cwar_relation::OwnedSnapshotStats,
        };
        let first = OwnedSnapshotStats {
            queries: 8,
            hits: 5,
            installed: 2,
            acquisition_headers: 8,
            acquisition_words: 21,
            required_vectors: 4,
            required_words: 40,
            required_cells: 128,
            ..OwnedSnapshotStats::default()
        };
        let first_capacity = OwnedSnapshotCapacity {
            vectors: 8,
            root_slots: 16,
            root_capacity: 32,
            words: 64,
            initialized_count_cells: 128,
            count_capacity: 256,
            allocated_bytes: 8192,
        };
        let mut work = WarSnapshotWork::default();
        work.observe(false, first, first_capacity);
        assert_eq!(work.passes, 0);
        assert_eq!(work.totals, [0; 6]);
        assert_eq!(work.max_required, [0; 3]);
        assert_eq!(work.peak_frame_bytes, 0);
        assert!(work.json()["latest"].is_null());
        work.observe(true, first, first_capacity);
        let second = OwnedSnapshotStats {
            queries: 3,
            hits: 1,
            fallbacks: 1,
            acquisition_headers: 3,
            acquisition_words: 5,
            required_vectors: 5,
            required_words: 12,
            required_cells: 32,
            ..OwnedSnapshotStats::default()
        };
        let second_capacity = OwnedSnapshotCapacity {
            vectors: 6,
            root_slots: 16,
            root_capacity: 16,
            words: 16,
            initialized_count_cells: 32,
            count_capacity: 64,
            allocated_bytes: 2048,
        };
        work.observe(true, second, second_capacity);
        assert_eq!(work.passes, 2);
        assert_eq!(work.totals, [11, 6, 2, 1, 11, 26]);
        assert_eq!(work.max_required, [5, 40, 128]);
        assert_eq!(work.peak_frame_bytes, 8192);
        assert_eq!(work.last, Some((second, second_capacity)));
        let status = work.json();
        assert_eq!(status["acquisition_consumed_nonnull_words"], 26);
        assert_eq!(status["maximum_required"]["words"], 40);
        assert_eq!(status["latest"]["required_words"], 12);
        assert_eq!(status["latest"]["capacity"]["root_capacity"], 16);
        assert_eq!(status["latest"]["capacity"]["allocated_bytes"], 2048);
        assert_eq!(status["peak_sampled_frame_capacity_bytes"], 8192);
        assert!(
            status["scope"]
                .as_str()
                .unwrap()
                .contains("not replay acceptance")
        );
        assert!(
            status["word_counter_scope"]
                .as_str()
                .unwrap()
                .contains("not physical native reads")
        );
        assert!(
            status["capacity_scope"]
                .as_str()
                .unwrap()
                .contains("not whole-pool ownership")
        );
        work.observe(false, first, first_capacity);
        assert_eq!(work.json(), status);
    }

    #[test]
    fn war_snapshot_work_saturates_cumulative_counters() {
        use engine_native::{
            cwar_relation::OwnedSnapshotCapacity, cwar_relation::OwnedSnapshotStats,
        };
        let mut work = WarSnapshotWork {
            passes: u64::MAX,
            totals: [u64::MAX - 1; 6],
            ..WarSnapshotWork::default()
        };
        let stats = OwnedSnapshotStats {
            queries: 2,
            hits: 2,
            installed: 2,
            fallbacks: 2,
            acquisition_headers: 2,
            acquisition_words: 2,
            ..OwnedSnapshotStats::default()
        };
        work.observe(true, stats, OwnedSnapshotCapacity::default());
        assert_eq!(work.passes, u64::MAX);
        assert_eq!(work.totals, [u64::MAX; 6]);
        assert_eq!(work.json()["queries"], u64::MAX);
    }

    #[test]
    fn war_snapshot_work_includes_rejected_consumed_frame_without_io() {
        let mut processor = Processor {
            root: PathBuf::new(),
            history: PreparedCampaigns::default(),
            session: session(),
            campaigns: Campaigns::new(),
            stats: Statistics::default(),
            unknown_pending: 0,
            suppress_origin: false,
            retry_full: false,
            unpublished_families: 0,
            helper: None,
            second_helper: None,
            backlogged: false,
            comparison: Box::default(),
        };
        let mut source = Captured::new();
        source.wars_snapshot_sampled = true;
        source.table_reads[TableId::Clock.index()] = ReadCounts {
            copy_calls: 1,
            spans: 1,
            bytes: 64,
            ..ReadCounts::ZERO
        };
        assert!(source.clock.is_none());
        assert!(processor.frame(&source).is_err());
        assert_eq!(processor.stats.war_snapshots.passes, 1);
        assert_eq!(processor.stats.hours, 0);
        assert_eq!(
            processor.stats.table_total_reads,
            [ReadCounts::ZERO; TABLE_COUNT]
        );
        assert_eq!(processor.json()["table_reads_recorded"], false);
        assert_eq!(processor.stats.errors, 1);
        assert!(processor.campaigns.is_empty());
        let status = processor.json()["war_owned_snapshots"].clone();
        assert_eq!(status["passes"], 1);
        assert_eq!(
            status["latest"]["capacity"]["allocated_bytes"],
            source.wars_snapshot.allocated_bytes()
        );
        source.wars_snapshot_sampled = false;
        assert!(processor.frame(&source).is_err());
        assert_eq!(processor.json()["war_owned_snapshots"], status);
        assert_eq!(processor.stats.errors, 2);
    }

    #[test]
    fn compared_variables_never_override_a_missing_entry_clock() {
        let mut encoder = FrameEncoder::new();
        encoder
            .encode(staggered_frame(1, 10, false), false)
            .unwrap();
        let baseline = encoder
            .comparison_baseline::<tables::ScriptVariables>(1, 11, false)
            .unwrap();
        let mut capture = captured(11);
        capture.clock = None;
        let (input, _) = decoded_with_costs(
            &capture,
            clock(11).borrowed(),
            1,
            &mut SpoolConversion {
                variables_baseline: Some(baseline),
                variables_scratch: &mut ScriptVariableComparison::default(),
                characters: None,
                lexical: None,
                intel: None,
                projection: None,
                helper: None,
            },
        );
        assert_eq!(
            input.tables,
            unavailable_tables("clock unreadable at hook entry")
        );
    }

    #[test]
    fn pack_costs_keep_fixed_totals_however_many_hours_are_recorded() {
        let mut stats = Statistics::default();
        let before = std::mem::size_of_val(&stats.conversion_packs);
        for hour in 0..10_000_i64 {
            stats.conversion_packs[3].record_ticks(hour % 7, 10_000_000);
        }
        // Totals, not a per-hour sample list: nothing grows with the hours.
        assert_eq!(std::mem::size_of_val(&stats.conversion_packs), before);
        let status = serde_json::to_value(stats.conversion_packs[3].status()).unwrap();
        assert_eq!(status["calls"], 10_000);
        assert_eq!(status["max_ns"], 600);
        assert_eq!(
            status["total_ns"],
            (0..10_000_u64).map(|hour| hour % 7 * 100).sum::<u64>()
        );
        assert_eq!(
            serde_json::to_value(stats.conversion_packs[4].status()).unwrap()["mean_ns"],
            0.0
        );
    }

    #[test]
    fn callback_tail_histogram_and_slowest_hours_share_callback_work_samples() {
        let mut processor = Processor {
            root: PathBuf::new(),
            history: PreparedCampaigns::default(),
            session: session(),
            campaigns: Campaigns::new(),
            stats: Statistics::default(),
            unknown_pending: 0,
            suppress_origin: false,
            retry_full: false,
            unpublished_families: 0,
            helper: None,
            second_helper: None,
            backlogged: false,
            comparison: Box::default(),
        };
        let mut samples = vec![0];
        for floor_ms in [2, 4, 8, 16, 32, 64, 128, 256] {
            samples.extend([floor_ms * 1_000_000 - 1, floor_ms * 1_000_000]);
        }
        samples.extend([512_000_000, 512_000_000, 4_000_000, 4_000_000]);
        let mut frame = captured(-20);
        for (index, &ns) in samples.iter().enumerate() {
            frame.clock = Some(clock(index as i32 - 20));
            frame.qpc = 6_000_000_000_000 + index as i64;
            frame.total_ns = ns;
            frame.hour_cost = crate::sampling::HourCost {
                entry_ns: 100 + index as u64,
                buffer_wait_ns: 200 + index as u64,
                setup_ns: ns / 4,
                parallel_ns: ns / 2,
                residual_ns: ns - ns / 4 - ns / 2,
                segments_ns: [index as u64; crate::sampling::SEGMENTS],
                slowest_join: (index == 17).then_some(crate::sampling::SlowestJoin {
                    role: 1,
                    wait_ns: 300,
                    collect_stall_ns: 250,
                }),
                collect_stalls_over_1ms: index as u64,
                growths: index as u64 + 1,
                first_since_attach: index == 17,
                continuous: index != 17,
                force_full: index == 17,
                recovery: index == 17,
                refresh: index == 17,
            };
            CapturedStatistics::capture(&frame)
                .unwrap()
                .record(&mut processor.stats);
        }
        let status = processor.json();
        let tail = &status["callback_tail"];
        assert_eq!(
            tail["histogram_floor_ms"],
            json!([0, 2, 4, 8, 16, 32, 64, 128, 256])
        );
        assert_eq!(tail["histogram"], json!([2, 2, 4, 2, 2, 2, 2, 2, 3]));
        assert_eq!(tail["capacity"], 16);
        assert_eq!(tail["qpc_frequency"], processor.session.qpc_frequency);
        assert_eq!(status["callback_work"]["calls"], samples.len());
        let slowest = tail["slowest"].as_array().unwrap();
        // The third 4 ms hour arrived at capacity and cannot displace an equal one.
        let indices = [17, 18, 16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 19];
        assert_eq!(
            slowest
                .iter()
                .map(|entry| entry["hour"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            indices.map(|index| index - 20),
        );
        let first = &slowest[0];
        assert_eq!(first["callback_ns"], status["callback_work"]["max_ns"]);
        assert_eq!(
            first,
            &json!({
                "hour": -3, "qpc": 6_000_000_000_017_i64,
                "daily_slot": (-3_i32).rem_euclid(replay_data::cadence::STAGGER_PERIOD),
                "callback_ns": 512_000_000, "entry_ns": 117, "buffer_wait_ns": 217,
                "setup_ns": 128_000_000, "parallel_ns": 256_000_000, "residual_ns": 128_000_000,
                "segments_ns": vec![17; crate::sampling::SEGMENTS],
                "slowest_join": {
                    "role": format!("{:?}", crate::acquisition_jobs::ROLES[1]),
                    "wait_ns": 300, "collect_stall_ns": 250,
                },
                "collect_stalls_over_1ms": 17, "growths": 18,
                "first_since_attach": true, "continuous": false,
                "force_full": true, "recovery": true, "refresh": true,
            }),
        );
    }

    #[test]
    fn validation_refuses_only_bad_table_and_next_hour_rebaselines() {
        use replay_frame::frame::{FrameDecoder, TableUpdate};
        let directory =
            std::env::temp_dir().join(format!("hoi4-contain-table-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let mut processor =
            Processor::new(&directory, PreparedCampaigns::default(), session()).unwrap();
        let retained = processor.validation_refusals();
        processor.frame(&captured(10)).unwrap();
        for hour in [11, 12] {
            let mut prepared = processor.prepare_frame(&captured(hour)).unwrap();
            prepared.source.tables[TableId::States.index()] =
                TableView::Complete(TableRows::States(vec![StateRow {
                    id: 64,
                    continent: Recorded::NotRecorded,
                    ..Default::default()
                }]))
                .into();
            // Also fail a second table: containment must visit every offender.
            prepared.source.tables[TableId::Campaign.index()] =
                TableView::Complete(TableRows::Campaign(Vec::new())).into();
            processor.write_prepared(prepared).unwrap();
        }
        processor.frame(&captured(13)).unwrap();
        processor.detach().unwrap();
        let status = processor.json();
        assert_eq!(status["hours"], 4);
        assert_eq!(status["write_errors"], 0);
        let refusals = status["validation_refusals"].as_array().unwrap();
        let states = refusals.iter().find(|f| f["table"] == "States").unwrap();
        assert_eq!(states["column"], "continent");
        assert_eq!(states["first_hour"], 11);
        assert_eq!(states["last_hour"], 12);
        assert_eq!(states["count"], 2);
        let log = fs::read_to_string(directory.join("recorder.log")).unwrap();
        assert_eq!(log.matches("States: row 64 column continent").count(), 1);
        let campaign = processor
            .campaigns
            .get_mut(std::str::from_utf8(ID).unwrap())
            .unwrap();
        assert!(campaign.pending.is_empty());
        let mut decoder = FrameDecoder::new();
        let mut frames = 0;
        for file in fs::read_dir(&campaign.directory).unwrap().flatten() {
            if file.file_name().to_string_lossy().starts_with("spool.") {
                for record in replay::spool::recover_prefix(&fs::read(file.path()).unwrap()).records
                {
                    if let Record::Frame(frame) = record {
                        let decoded = decoder.apply(&frame).unwrap();
                        frames += 1;
                        if [11, 12].contains(&frame.hour) {
                            assert!(
                                matches!(&*decoded.tables[TableId::States.index()], TableView::Unavailable { reason } if reason.contains("States: row 64 column continent"))
                            );
                            assert!(decoded.complete::<tables::Clock>().is_ok());
                        }
                        if frame.hour == 13 {
                            assert!(matches!(
                                frame.tables[TableId::States.index()],
                                TableUpdate::Snapshot(_)
                            ));
                        }
                    }
                }
            }
        }
        assert_eq!(frames, 4);
        drop(processor);
        // Runtime fallback status keeps the ledger even after spool ownership ends.
        let retained = serde_json::to_value(&retained).unwrap();
        assert_eq!(retained, status["validation_refusals"]);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn spool_validation_detail_survives_ordinary_failure_and_detach() {
        const TEST: &str =
            "spooling::tests::spool_validation_detail_survives_ordinary_failure_and_detach";
        const CHILD: &str = "BATTLE_PLAN_SPOOL_VALIDATION_DETAIL_TEST";
        // Allocation hooks are process-wide. Exercise the worker with the same
        // hooks as capture without changing other tests' diagnostic behaviour.
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST, "--nocapture"])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            return;
        }
        crate::allocation::install_hooks();
        let directory = std::env::temp_dir().join(format!(
            "hoi4-spool-validation-detail-{}",
            std::process::id()
        ));
        let mut processor =
            Processor::new(&directory, PreparedCampaigns::default(), session()).unwrap();
        let mut source = captured(10);
        prepare_order_counter_sample(&mut source, 10, 7);
        processor.prepare(&source).unwrap();
        processor.frame(&source).unwrap();

        // Fail an ordinary hour after the forced sample's successful keyframe.
        // The failed write closes the segment and queues a gap for recovery.
        source = captured(11);
        source.order_counters.forced = false;
        let mut prepared = processor.prepare_frame(&source).unwrap();
        prepared.source.tables[TableId::Clock.index()] =
            TableView::Complete(TableRows::Clock(vec![ClockRow {
                hour: 12,
                ..Default::default()
            }]))
            .into();
        let detail = "Clock: clock row hour differs from frame hour";
        let expected = format!("spool validation failed: {detail}");
        assert_eq!(
            processor.write_prepared(prepared).unwrap_err().to_string(),
            detail
        );
        let campaign = processor
            .campaigns
            .get_mut(std::str::from_utf8(ID).unwrap())
            .unwrap();
        assert!(!campaign.spool.is_open());
        assert!(!campaign.pending.is_empty());
        let detach_error = processor.detach().unwrap_err();

        // Persist the terminal status after both original errors are gone.
        let detach_detail = detach_error.to_string();
        drop(detach_error);
        let mut bytes = Vec::new();
        crate::status::serialize_into(&mut bytes, &processor.status().unwrap()).unwrap();
        let status_path = directory.join("status.json");
        fs::write(&status_path, bytes).unwrap();
        let persisted: Value = serde_json::from_slice(&fs::read(status_path).unwrap()).unwrap();
        assert_eq!(persisted["write_errors"], 2);
        assert_eq!(persisted["last_error"], expected);
        let sample = &persisted["order_instance_counters"]["sample"];
        assert_eq!(sample["clock"]["hour"], 10);
        assert_eq!(sample["frame_write_succeeded"], true);
        assert_eq!(sample["subsequent_io_error"], expected);
        assert_eq!(sample["usable_with_recorded_frame"], false);
        assert_eq!(
            detach_detail,
            "spool unavailable at detach; pending gaps need another observed keyframe"
        );

        // Preflight and loss handling must retain static reasons with capture
        // hooks installed too, and must restore callback behaviour on return.
        let mut preflight =
            Processor::new(&directory, PreparedCampaigns::default(), session()).unwrap();
        assert_eq!(
            preflight.prepare(&source).unwrap_err().to_string(),
            "attach output already contains spool segments; use a fresh attach directory"
        );
        let loss = LostObservation {
            campaign_id: owned_text(ID),
            first_hour: 12,
            last_hour: 12,
            count: 2,
            ..Default::default()
        };
        assert_eq!(
            processor.lost(&loss).unwrap_err().to_string(),
            "queue loss witness has inconsistent hours and count"
        );
        assert_eq!(processor.json()["write_errors"], 3);
        assert_eq!(processor.json()["last_error"], expected);
        assert!(invalid("callback refusal").get_ref().is_none());
        drop(preflight);
        drop(processor);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn spool_validation_detail_survives_owned_status_and_forced_witness() {
        let mut processor = Processor {
            root: PathBuf::new(),
            history: PreparedCampaigns::default(),
            session: session(),
            campaigns: Campaigns::new(),
            stats: Statistics::default(),
            unknown_pending: 0,
            suppress_origin: false,
            retry_full: false,
            unpublished_families: 0,
            helper: None,
            second_helper: None,
            backlogged: false,
            comparison: Box::default(),
        };
        let mut source = captured(10);
        prepare_order_counter_sample(&mut source, 10, 7);
        let mut witness = OrderCounterWitness::default();
        let prepared = PreparedOrderWitness::capture(&source, Some(1));
        let detail = "Clock: clock row hour differs from frame hour";
        let expected = format!("spool validation failed: {detail}");
        {
            let (mut campaign, _) = campaign();
            let branch = campaign.observe(None, 10, Some(1)).unwrap();
            let mut frame = staggered_frame(branch, 10, false);
            *frame.tables[TableId::Clock.index()] =
                TableView::Complete(TableRows::Clock(vec![ClockRow {
                    hour: 11,
                    ..Default::default()
                }]));
            let result = campaign.write(frame, 1000, Some(1), 0, &mut WorkSample::default());
            assert_eq!(result.as_ref().unwrap_err().to_string(), detail);
            processor.error(result.as_ref().unwrap_err());
            witness.observe(&source, Some(1), &result);
            // Preparation failures borrow the original error, rather than
            // reconstructing one from its kind before retaining the witness.
            witness.observe_error(&source, Some(1), result.as_ref().err());
            prepared.bind(&mut processor.stats.order_counters, &result);
            witness.invalidate_write(
                std::str::from_utf8(ID).unwrap(),
                result.as_ref().unwrap_err(),
            );
        }
        let generic = io::Error::from(io::ErrorKind::InvalidData);
        processor.error(&generic);
        witness.invalidate_write(std::str::from_utf8(ID).unwrap(), &generic);
        processor.stats.order_counters.observe_error(
            &source,
            Some(1),
            Some(&io::ErrorKind::OutOfMemory.into()),
        );
        // The original error is gone before the status writer serializes it.
        let mut bytes = Vec::new();
        crate::status::serialize_into(&mut bytes, &processor.status().unwrap()).unwrap();
        let persisted: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(persisted["write_errors"], 2);
        assert_eq!(persisted["last_error"], expected);
        assert_eq!(
            persisted["order_instance_counters"]["sample"]["frame_write_error"],
            expected
        );
        assert_eq!(witness.json()["sample"]["frame_write_error"], expected);
        assert_eq!(witness.json()["sample"]["subsequent_io_error"], expected);
    }

    #[test]
    fn cached_callback_summary_does_not_freeze_status_errors_or_latest_cost() {
        let mut processor = Processor {
            root: PathBuf::new(),
            history: PreparedCampaigns::default(),
            session: session(),
            campaigns: Campaigns::new(),
            stats: Statistics::default(),
            unknown_pending: 0,
            suppress_origin: false,
            retry_full: false,
            unpublished_families: 0,
            helper: None,
            second_helper: None,
            backlogged: false,
            comparison: Box::default(),
        };
        for ns in [100, 200, 300] {
            processor.stats.callback.record_ns(ns);
        }
        processor.stats.hours = 3;
        processor.stats.total_ns = 300;
        let first = processor.json();
        assert_eq!(first["p50_ns"], 200);
        assert_eq!(first["p95_ns"], 300);
        assert_eq!(first["max_ns"], 300);
        assert_eq!(first["total_ns"], 300);
        assert_eq!(first["callback_work"]["calls"], 3);
        assert_eq!(first["callback_work"]["total_ns"], 600);
        // Other tests can update the process-wide acquisition counters between
        // snapshots. Compare this processor's callback summary and gauges only.
        let repeated = processor.json();
        for field in [
            "hours",
            "total_ns",
            "p50_ns",
            "p95_ns",
            "max_ns",
            "callback_work",
        ] {
            assert_eq!(repeated[field], first[field], "{field}");
        }

        processor.observe_input_demand_error("country roster header unreadable or invalid");
        processor.observe_input_demand_error("unit leader identity unreadable");
        let demand_failed = processor.json();
        assert_eq!(demand_failed["input_demand_errors"], 2);
        assert_eq!(
            demand_failed["input_demand_last_error"],
            "unit leader identity unreadable"
        );
        assert_eq!(demand_failed["write_errors"], 0);

        processor.error(&io::Error::other("new disk failure while paused"));
        let failed = processor.json();
        assert_eq!(failed["last_error"], "spool I/O failed");
        assert_eq!(failed["write_errors"], 1);
        assert_eq!(failed["p50_ns"], first["p50_ns"]);

        processor.stats.callback.record_ns(50);
        processor.stats.hours += 1;
        processor.stats.total_ns = 50;
        let resumed = processor.json();
        assert_eq!(resumed["hours"], 4);
        assert_eq!(resumed["total_ns"], 50);
        assert_eq!(resumed["callback_work"]["calls"], 4);
        assert_eq!(resumed["callback_work"]["total_ns"], 650);
        assert_eq!(resumed["p50_ns"], 100);
        assert_eq!(resumed["p95_ns"], 300);
        assert_eq!(resumed["max_ns"], 300);
        assert_eq!(resumed["last_error"], failed["last_error"]);
    }

    #[test]
    fn table_acquisition_totals_keep_zero_skips_distinct_from_latest_gauges() {
        let mut stats = Statistics::default();
        let mut first = [0; TABLE_COUNT];
        first[0] = 100;
        first[1] = 300;
        stats.observe_table_costs(first);
        let mut second = [0; TABLE_COUNT];
        second[0] = 50;
        second[2] = 200;
        stats.observe_table_costs(second);
        assert_eq!(stats.table_ns, second);
        assert_eq!(&stats.table_total_ns[..3], &[150, 300, 200]);
        assert!(stats.table_total_ns[3..].iter().all(|&ns| ns == 0));
        stats.table_total_ns[0] = u64::MAX;
        stats.observe_table_costs(first);
        assert_eq!(stats.table_total_ns[0], u64::MAX);
    }

    #[test]
    fn table_read_totals_preserve_zero_skips_and_saturate_each_counter() {
        let counts = ReadCounts {
            copy_calls: 2,
            scattered_calls: 3,
            record_calls: 5,
            plan_calls: 7,
            spans: 11,
            bytes: 13,
        };
        let mut first = [ReadCounts::ZERO; TABLE_COUNT];
        first[0] = counts;
        first[1] = counts;
        let mut stats = Statistics::default();
        stats.observe_table_reads(first);
        let mut second = [ReadCounts::ZERO; TABLE_COUNT];
        second[1] = counts;
        stats.observe_table_reads(second);
        assert_eq!(stats.table_reads, second);
        assert_eq!(stats.table_total_reads[0], counts);
        assert_eq!(
            stats.table_total_reads[1],
            ReadCounts {
                copy_calls: 4,
                scattered_calls: 6,
                record_calls: 10,
                plan_calls: 14,
                spans: 22,
                bytes: 26,
            }
        );
        assert!(
            stats.table_total_reads[2..]
                .iter()
                .all(|&value| value == ReadCounts::ZERO)
        );
        stats.table_total_reads[0] = ReadCounts {
            copy_calls: u64::MAX - 1,
            scattered_calls: u64::MAX - 1,
            record_calls: u64::MAX - 1,
            plan_calls: u64::MAX - 1,
            spans: u64::MAX - 1,
            bytes: u64::MAX - 1,
        };
        stats.observe_table_reads(first);
        assert_eq!(
            stats.table_total_reads[0],
            ReadCounts {
                copy_calls: u64::MAX,
                scattered_calls: u64::MAX,
                record_calls: u64::MAX,
                plan_calls: u64::MAX,
                spans: u64::MAX,
                bytes: u64::MAX,
            }
        );
    }

    #[test]
    fn status_read_counts_keep_latest_totals_and_full_width_integers() {
        let mut processor = Processor {
            root: PathBuf::new(),
            history: PreparedCampaigns::default(),
            session: session(),
            campaigns: Campaigns::new(),
            stats: Statistics::default(),
            unknown_pending: 0,
            suppress_origin: false,
            retry_full: false,
            unpublished_families: 0,
            helper: None,
            second_helper: None,
            backlogged: false,
            comparison: Box::default(),
        };
        assert_eq!(processor.json()["table_reads_recorded"], false);
        let counts = ReadCounts {
            copy_calls: 1,
            scattered_calls: 2,
            record_calls: 3,
            plan_calls: 4,
            spans: 9_007_199_254_740_993,
            bytes: u64::MAX,
        };
        let mut first = [ReadCounts::ZERO; TABLE_COUNT];
        first[0] = counts;
        processor.stats.observe_table_reads(first);
        processor
            .stats
            .observe_table_reads([ReadCounts::ZERO; TABLE_COUNT]);
        processor.stats.hours = 2;
        let status = processor.json();
        assert_eq!(status["table_reads_recorded"], true);
        assert_eq!(status["table_reads_frames"], 2);
        assert_eq!(status["table_reads"].as_array().unwrap().len(), TABLE_COUNT);
        assert_eq!(
            status["table_total_reads"].as_array().unwrap().len(),
            TABLE_COUNT
        );
        assert_eq!(status["table_reads"][0], read_counts_json(ReadCounts::ZERO));
        let serialized = serde_json::to_vec(&status).unwrap();
        let reparsed: Value = serde_json::from_slice(&serialized).unwrap();
        assert_eq!(reparsed["table_total_reads"][0], read_counts_json(counts));
        assert_eq!(
            reparsed["table_total_reads"][0]["bytes"].as_u64(),
            Some(u64::MAX)
        );
        assert_eq!(
            reparsed["table_total_reads"][1],
            read_counts_json(ReadCounts::ZERO)
        );
    }

    #[test]
    fn status_reports_pilot_copy_ns_additively() {
        let mut processor = Processor {
            root: PathBuf::new(),
            history: PreparedCampaigns::default(),
            session: session(),
            campaigns: Campaigns::new(),
            stats: Statistics::default(),
            unknown_pending: 0,
            suppress_origin: false,
            retry_full: false,
            unpublished_families: 0,
            helper: None,
            second_helper: None,
            backlogged: false,
            comparison: Box::default(),
        };
        processor.stats.table_ns[replay_data::rows::TableId::ResearchTechnologies.index()] = 11;
        processor.stats.table_ns[replay_data::rows::TableId::ResearchSlots.index()] = 22;
        processor.stats.table_ns[replay_data::rows::TableId::TechnologyDefinitions.index()] = 33;
        processor.stats.table_ns[replay_data::rows::TableId::LandCombats.index()] = 101;
        processor.stats.table_ns[replay_data::rows::TableId::WingEquipment.index()] = 201;
        processor.stats.table_ns[replay_data::rows::TableId::LandTheatres.index()] = 301;
        processor.stats.table_ns[replay_data::rows::TableId::Players.index()] = 401;
        processor.stats.table_ns[replay_data::rows::TableId::WorldTension.index()] = 501;
        processor.stats.table_ns[replay_data::rows::TableId::WarRelations.index()] = 601;
        processor.stats.table_ns[replay_data::rows::TableId::StateBuildings.index()] = 701;
        processor.stats.table_ns[replay_data::rows::TableId::RaidTargetStatuses.index()] = 703;
        processor.stats.observe_stability_walk_ticks([1, 2, 3, 4]);
        processor
            .stats
            .observe_stability_walk_ticks([10, 20, 30, 40]);
        let status = processor.json();
        assert_eq!(status["buildings_walk_ns"], json!([701, 0, 703]));
        assert_eq!(status["research_walk_ns"], json!([11, 22, 33]));
        assert_eq!(status["stability_walk_ns"], json!([1100, 2200, 3300, 4400]));
        assert_eq!(status["wars_walk_ns"], json!([601]));
        // New game-thread copy-only mirrors for the coupled unit, air,
        // naval, players and session families.
        assert_eq!(status["map_walk_ns"][0], json!(101));
        assert_eq!(status["map_walk_ns"].as_array().unwrap().len(), 8);
        assert_eq!(status["air_wing_copy_ns"], json!([201, 0]));
        assert_eq!(status["naval_copy_ns"][0], json!(301));
        assert_eq!(status["naval_copy_ns"].as_array().unwrap().len(), 9);
        assert_eq!(status["players_copy_ns"], json!([401, 0]));
        assert_eq!(status["session_copy_ns"], json!([501, 0, 0]));
        // Existing keys survive the additive merge.
        assert!(status["table_ns"].is_array());
        assert_eq!(status["hours"], 0);
        assert_eq!(status["spool_work"]["lexical_conversion"]["hits"], 0);
        assert_eq!(status["spool_work"]["lexical_conversion"]["misses"], 0);
        assert_eq!(
            status["spool_work"]["lexical_conversion"]["additional_native_dictionary_capacity_bytes"],
            0
        );
        assert_eq!(status["spool_work"]["character_shadow"]["campaigns"], 0);
        assert_eq!(
            status["spool_work"]["character_shadow"]["retained_capacity_bytes"],
            0
        );
    }

    use engine_native::msvc_string::Text;
    use replay::{spool::SegmentSink, spool::recover_prefix};
    use replay_data::{identity::Identity, identity::Verification};
    use replay_frame::{branches::BranchReason, branches::Parentage};
    use std::{cell::RefCell, io::Write, rc::Rc};

    const ID: &[u8] = b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6";

    fn text<const N: usize>(value: &[u8]) -> Text<N> {
        let mut result = Text {
            len: value.len(),
            ..Text::default()
        };
        result.bytes[..value.len()].copy_from_slice(value);
        result
    }

    fn owned_text(bytes: &[u8]) -> engine_native::capture_text::Owned {
        engine_native::capture_text::Owned {
            bytes: bytes.to_vec(),
            len: bytes.len(),
        }
    }

    fn clock(hour: i32) -> Clock {
        Clock {
            game_state: 4096,
            hour,
            play_time: u64::MAX,
            play_session_started_at: 1234,
            seed: i32::MIN,
            campaign_id: owned_text(ID),
            speed: 5,
            tick_busy: 0,
        }
    }

    pub(super) fn captured(hour: i32) -> Captured {
        // The conversion fixtures write one core row per family, two division
        // identities/member occurrences and six faction-name bytes.
        let mut captured = Captured::with_capacities(
            &[1, 1, 1, 1, 2, 1, 1, 2, 6, 0, 0, 0],
            &crate::map_capture::MapCapacities::default(),
            &crate::extended::Capacities::default(),
        )
        .unwrap();
        // End-of-hour sampling reads the clock once before the original tick.
        captured.clock = Some(clock(hour));
        captured.qpc = 1000 + i64::from(hour);
        captured.countries_result = Outcome::Rows { len: 0 };
        captured.links_result = Outcome::Rows { len: 0 };
        captured.provinces_result = Outcome::Rows { len: 0 };
        captured.states_result = Outcome::Rows { len: 0 };
        captured.divisions_result = Outcome::Rows { len: 0 };
        captured.wars_result = Outcome::Rows { len: 0 };
        captured.factions_result = Outcome::Rows { len: 0 };
        captured.faction_members_result = Outcome::Rows { len: 0 };
        captured.map.players.outcome = Outcome::Rows { len: 0 };
        captured.map.human_controllers.outcome = Outcome::Rows { len: 0 };
        captured.map.land_combats.outcome = Outcome::Rows { len: 0 };
        captured.map.naval.fleets.outcome = Outcome::Rows { len: 0 };
        captured.map.naval.task_forces.outcome = Outcome::Rows { len: 0 };
        captured.map.naval.ships.outcome = Outcome::Rows { len: 0 };
        captured.map.naval_combats.outcome = Outcome::Rows { len: 0 };
        captured.map.naval_combat_participants.outcome = Outcome::Rows { len: 0 };
        captured.map.air_wings.outcome = Outcome::Rows { len: 0 };
        captured.map.orders.rows.outcome = Outcome::Rows { len: 0 };
        captured
    }

    fn prepare_order_counter_sample(source: &mut Captured, hour: i32, seq: u64) {
        source.capture_seq = seq;
        source.clock = Some(clock(hour));
        source.qpc = 1000;
        source.country_cache_status = engine_native::country_memory::SeedStatus::Ready;
        source.order_counters.forced = true;
        source.order_counters.read = Some(engine_native::ccountry::OrderInstanceCounterRead {
            len: 1,
            required: 1,
            unreadable: 0,
        });
        source.order_counters.rows[0] = engine_native::ccountry::OrderInstanceCounter {
            pointer: 0x2000,
            country: 3,
            canonical_slot: 1,
            value: Some(3806),
        };
        source.order_counters.qpc_begin = 2000;
        source.order_counters.qpc_end = 2100;
        source.order_counters.elapsed_ns = 10_000;
    }

    fn owned_prepared_fixture(captured: &Captured, branch: BranchId) -> PreparedFrame {
        let clock = captured.clock.as_ref().unwrap().copy_owned().unwrap();
        let source: FrameInput = decoded(captured, clock.clone(), branch).into();
        let retry_full = needs_retry(&source.tables);
        PreparedFrame {
            source,
            id: std::str::from_utf8(ID).unwrap().into(),
            clock,
            qpc: captured.qpc,
            unknown: 0,
            retry_full,
            unpublished_families: 0,
            skipped: [false; TABLE_COUNT],
            work: WorkSample::default(),
            pack_ticks: [None; crate::extended::PACK_NAMES.len()],
            sampled: CapturedStatistics::capture(captured).unwrap(),
            witness: PreparedOrderWitness::capture(captured, Some(branch)),
            operation_publication: None,
            map_event_publication: None,
            event_feed_publication: None,
        }
    }

    #[test]
    fn prepared_output_statistics_and_forced_witness_survive_capture_reuse() {
        let mut source = captured(10);
        prepare_order_counter_sample(&mut source, 10, 7);
        source.total_ns = 123;
        source.hour_cost.growths = 43;
        source.table_ns[TableId::Clock.index()] = 45;
        source.table_reads[TableId::Clock.index()].copy_calls = 6;
        source.pool_bytes = 789;
        source.country_cache_entries = 12;
        source.country_cache_ns = 34;
        source.dropped_frames = 2;
        let expected = decoded(&source, clock(10), 1);
        let allocated = source.allocated_bytes();
        let prepared = owned_prepared_fixture(&source, 1);
        // Reuse every field and allocation of the producer before consumption.
        source = captured(11);
        prepare_order_counter_sample(&mut source, 11, 8);
        source.order_counters.rows[0].value = Some(999);
        let PreparedFrame {
            source: owned,
            sampled,
            witness,
            ..
        } = prepared;
        let mut encoder = FrameEncoder::new();
        let encoded = encoder.encode_input(owned, true).unwrap();
        let actual = replay_frame::frame::FrameDecoder::new()
            .apply(&encoded)
            .unwrap();
        assert_eq!(actual.tables, expected.tables);
        assert_eq!(actual.metrics, expected.metrics);
        let mut stats = Statistics::default();
        sampled.record(&mut stats);
        witness.bind(&mut stats.order_counters, &Ok(()));
        assert_eq!(stats.total_ns, 123);
        let tail = serde_json::to_value(stats.callback_tail.status(1)).unwrap();
        assert_eq!(tail["slowest"][0]["hour"], 10);
        assert_eq!(tail["slowest"][0]["growths"], 43);
        assert_eq!(stats.table_ns[TableId::Clock.index()], 45);
        assert_eq!(
            stats.table_total_reads[TableId::Clock.index()].copy_calls,
            6
        );
        assert_eq!(stats.pool_bytes, 789);
        assert_eq!(stats.sampled_buffer_bytes, allocated);
        assert_eq!(stats.country_cache_entries, 12);
        assert_eq!(stats.country_cache_cost.total_ns, 34);
        assert_eq!(stats.dropped_frames, 2);
        let sample = stats.order_counters.json();
        assert_eq!(sample["sample"]["clock"]["hour"], 10);
        assert_eq!(sample["sample"]["sequence"], 7);
        assert_eq!(sample["sample"]["countries"][0]["counter"], 3806);
        assert_eq!(sample["sample"]["usable_with_recorded_frame"], true);
    }

    #[test]
    fn prepared_forced_witness_binds_write_failure_after_capture_is_dropped() {
        let (mut campaign, store) = campaign();
        let branch = campaign.observe(None, 10, Some(1234)).unwrap();
        let mut source = captured(10);
        prepare_order_counter_sample(&mut source, 10, 7);
        let PreparedFrame {
            source: owned,
            witness,
            qpc,
            clock,
            ..
        } = owned_prepared_fixture(&source, branch);
        drop(source);
        store.borrow_mut().fail_frame_flush = true;
        let result = campaign.write(
            owned,
            qpc,
            Some(clock.play_session_started_at),
            0,
            &mut WorkSample::default(),
        );
        assert!(result.is_err());
        let mut retained = OrderCounterWitness::default();
        witness.bind(&mut retained, &result);
        assert_eq!(retained.json()["sample"]["sequence"], 7);
        assert_eq!(retained.json()["sample"]["frame_write_succeeded"], false);
        assert_eq!(
            retained.json()["sample"]["usable_with_recorded_frame"],
            false
        );
        assert!(campaign.disk_branch.is_none());
        assert!(!campaign.pending.is_empty());
        // A later ordinary failure must still invalidate the retained sample.
        let mut later = captured(11);
        later.order_counters.forced = false;
        let ordinary = PreparedOrderWitness::capture(&later, Some(branch));
        drop(later);
        ordinary.bind(&mut retained, &Err(io::Error::other("later failure")));
        assert_eq!(
            retained.json()["sample"]["subsequent_io_error"],
            "spool I/O failed"
        );
    }

    #[test]
    fn prepared_retry_verdict_retains_frame_but_forced_diagnostics_do_not() {
        let mut source = captured(10);
        prepare_order_counter_sample(&mut source, 10, 7);
        let mut prepared = owned_prepared_fixture(&source, 1);
        // Synthetic fully complete tables represent a successful full refresh.
        prepared.source.tables = TableId::ALL
            .map(|table| TableInput::Sampled(TableView::Complete(TableRows::empty(table))));
        prepared.retry_full = needs_retry(&prepared.source.tables);
        assert!(!prepared.needs_retry());
        assert!(prepared.allows_early_recycle());
        assert!(prepared.witness.forced.is_some());
        let static_table = TableId::ALL
            .into_iter()
            .find(|table| {
                replay_data::cadence::cadence(*table) == replay_data::cadence::Cadence::Static
            })
            .unwrap();
        prepared.source.tables[static_table.index()] =
            unavailable("synthetic static failure").into();
        prepared.retry_full = needs_retry(&prepared.source.tables);
        assert!(prepared.needs_retry());
        assert!(!prepared.allows_early_recycle());
        let shared = crate::sampling::Shared::new(0, 0, 1);
        if prepared.needs_retry() {
            shared.request_retry_full();
        }
        assert!(
            shared
                .refresh_next
                .load(std::sync::atomic::Ordering::Acquire)
        );
        assert!(!shared.force_next.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn order_counter_status_keeps_owned_sample_hour_sequence_and_write_failure() {
        let mut source = captured(10);
        let mut witness = OrderCounterWitness::default();
        assert!(witness.json()["sample"].is_null());
        prepare_order_counter_sample(&mut source, 10, 0);
        witness.observe(&source, Some(2), &Ok(()));
        let first = witness.json();
        assert_eq!(first["sample"]["sequence"], 0);
        assert_eq!(first["sample"]["branch"], 2);
        assert_eq!(first["sample"]["usable_with_recorded_frame"], true);
        assert_eq!(
            first["sample"]["source_capacity_bytes"],
            source.order_counters.allocated_bytes()
        );
        assert_eq!(
            first["retained_capacity_bytes"],
            witness.rows.capacity()
                * std::mem::size_of::<engine_native::ccountry::OrderInstanceCounter>()
        );
        prepare_order_counter_sample(&mut source, 11, 1);
        source.order_counters.forced = false;
        source.order_counters.rows[0].value = Some(999);
        witness.observe(&source, Some(3), &Ok(()));
        assert_eq!(witness.json(), first);
        source.order_counters.forced = true;
        witness.observe(
            &source,
            Some(3),
            &Err(io::Error::other("injected frame failure")),
        );
        let failed = witness.json();
        assert_eq!(failed["sample"]["clock"]["hour"], 11);
        assert_eq!(failed["sample"]["sequence"], 1);
        assert_eq!(failed["sample"]["branch"], 3);
        assert_eq!(failed["sample"]["countries"][0]["counter"], 999);
        assert_eq!(failed["sample"]["sample_valid"], true);
        assert_eq!(failed["sample"]["frame_write_succeeded"], false);
        assert_eq!(failed["sample"]["frame_write_error"], "spool I/O failed");
        assert_eq!(failed["sample"]["usable_with_recorded_frame"], false);
        source.order_counters.forced = false;
        witness.observe(&source, Some(3), &Ok(()));
        assert_eq!(witness.json(), failed);
        prepare_order_counter_sample(&mut source, 12, 2);
        source.clock.as_mut().unwrap().play_session_started_at = 5678;
        witness.observe(&source, Some(4), &Ok(()));
        assert_eq!(witness.json()["sample"]["usable_with_recorded_frame"], true);
        assert_eq!(witness.json()["sample"]["branch"], 4);
        assert_eq!(witness.json()["sample"]["clock"]["load_signal"], 5678);
    }

    #[test]
    fn order_counter_status_retains_sampled_allocation_values_and_missing_metadata() {
        let mut source = captured(10);
        prepare_order_counter_sample(&mut source, 10, 7);
        let before = AllocationCounters {
            debug_current_ref_id: Some(u32::MAX),
            unit: Some(11),
            order_index: Some(12),
            front_index: Some(13),
            theatre_index: Some(14),
            theater_group_index: Some(15),
            military_deployment_line_index: Some(16),
            military_deployment_conveyor_index: Some(17),
            equipment_variant_index: Some(18),
            country_leader_index: Some(19),
            navy_id: Some(20),
            land_combat_id: Some(21),
            multiplayer_random_count: Some(-100),
        };
        source.counters = Some(before);
        let mut witness = OrderCounterWitness::default();
        witness.observe(&source, Some(2), &Ok(()));
        let retained = witness.json();
        let sample = &retained["sample"];
        assert_eq!(sample["sequence"], 7);
        assert_eq!(sample["branch"], 2);
        assert_eq!(sample["clock"]["hour"], 10);
        assert_eq!(sample["sample_valid"], true);
        assert_eq!(sample["usable_with_recorded_frame"], true);
        assert_eq!(
            sample["allocation_counters"],
            json!({
                "debug_current_ref_id": u32::MAX,
                "unit": 11,
                "order_index": 12,
                "front_index": 13,
                "theatre_index": 14,
                "theater_group_index": 15,
                "military_deployment_line_index": 16,
                "military_deployment_conveyor_index": 17,
                "equipment_variant_index": 18,
                "country_leader_index": 19,
                "navy_id": 20,
                "land_combat_id": 21,
                "multiplayer_random_count": -100,
            })
        );
        // A reused frame and a later ordinary hour cannot change the bound
        // force_full sample. Missing counter metadata adds no new validity gate.
        prepare_order_counter_sample(&mut source, 11, 8);
        source.order_counters.forced = false;
        source.counters = None;
        witness.observe(&source, Some(3), &Ok(()));
        assert_eq!(witness.json(), retained);
        source.order_counters.forced = true;
        witness.observe(&source, Some(3), &Ok(()));
        let missing = witness.json();
        assert_eq!(missing["sample"]["sequence"], 8);
        assert_eq!(missing["sample"]["branch"], 3);
        assert!(missing["sample"]["allocation_counters"].is_null());
        assert_eq!(missing["sample"]["sample_valid"], true);
        assert_eq!(missing["sample"]["usable_with_recorded_frame"], true);
    }

    #[test]
    fn order_counter_status_rejects_incomplete_or_unbound_samples() {
        let mut source = captured(10);
        let mut witness = OrderCounterWitness::default();
        for case in 0..9 {
            prepare_order_counter_sample(&mut source, 10, case);
            match case {
                0 => source.clock = None,
                1 => {
                    source.country_cache_status =
                        engine_native::country_memory::SeedStatus::Unavailable {
                            reason: "no roster",
                        }
                }
                2 => source.order_counters.read = None,
                3 => source.order_counters.read.as_mut().unwrap().required = 2,
                4 => source.order_counters.read.as_mut().unwrap().unreadable = 1,
                5 => source.order_counters.rows[0].value = None,
                6 => {
                    source.order_counters.read.as_mut().unwrap().len =
                        source.order_counters.rows.len() + 1
                }
                7 => source.order_counters.qpc_end = source.order_counters.qpc_begin - 1,
                8 => source.order_counters.qpc_begin = source.qpc - 1,
                _ => unreachable!(),
            }
            witness.observe(&source, Some(2), &Ok(()));
            assert_eq!(
                witness.json()["sample"]["sample_valid"],
                false,
                "case {case}"
            );
            assert_eq!(
                witness.json()["sample"]["usable_with_recorded_frame"],
                false,
                "case {case}"
            );
        }
        prepare_order_counter_sample(&mut source, 10, 20);
        witness.observe(&source, None, &Err(io::Error::other("branch unavailable")));
        assert!(witness.json()["sample"]["branch"].is_null());
        assert_eq!(
            witness.json()["sample"]["usable_with_recorded_frame"],
            false
        );
    }

    #[test]
    fn order_counter_status_io_failure_cannot_be_healed_by_an_off_hour() {
        let mut source = captured(10);
        prepare_order_counter_sample(&mut source, 10, 1);
        let mut witness = OrderCounterWitness::default();
        witness.observe(&source, Some(2), &Ok(()));
        witness.invalidate_write("different campaign", &io::Error::other("unrelated"));
        assert_eq!(witness.json()["sample"]["usable_with_recorded_frame"], true);
        prepare_order_counter_sample(&mut source, 11, 2);
        source.order_counters.forced = false;
        witness.observe(
            &source,
            Some(2),
            &Err(io::Error::other("later frame failure")),
        );
        assert_eq!(witness.json()["sample"]["clock"]["hour"], 10);
        assert_eq!(witness.json()["sample"]["sequence"], 1);
        assert_eq!(
            witness.json()["sample"]["subsequent_io_error"],
            "spool I/O failed"
        );
        assert_eq!(
            witness.json()["sample"]["usable_with_recorded_frame"],
            false
        );
        witness.observe(&source, Some(2), &Ok(()));
        assert_eq!(
            witness.json()["sample"]["usable_with_recorded_frame"],
            false
        );
        prepare_order_counter_sample(&mut source, 12, 3);
        witness.observe(&source, Some(2), &Ok(()));
        assert_eq!(witness.json()["sample"]["usable_with_recorded_frame"], true);
        witness.invalidate_write(
            std::str::from_utf8(ID).unwrap(),
            &io::Error::other("later sync failure"),
        );
        assert_eq!(witness.json()["sample"]["frame_write_succeeded"], true);
        assert_eq!(
            witness.json()["sample"]["subsequent_io_error"],
            "spool I/O failed"
        );
        assert_eq!(
            witness.json()["sample"]["usable_with_recorded_frame"],
            false
        );
        source.order_counters.forced = false;
        witness.observe(&source, Some(2), &Ok(()));
        assert_eq!(
            witness.json()["sample"]["usable_with_recorded_frame"],
            false
        );
        prepare_order_counter_sample(&mut source, 12, 3);
        witness.observe(&source, Some(2), &Ok(()));
        assert_eq!(witness.json()["sample"]["usable_with_recorded_frame"], true);
        assert!(witness.json()["sample"]["subsequent_io_error"].is_null());
    }

    #[test]
    fn conversion_keeps_native_values_aliases_and_raw_text() {
        let mut captured = captured(10);
        captured.countries[0] = engine_native::ccountry::Row {
            politics_witness: None,
            id: 7,
            canonical_slot: 1,
            tag: text(b"GER"),
            political_power_raw: None,
            command_power_raw: i64::MIN,
            stability_raw: 100_001,
            war_support_raw: -9_007_199_254_740_993,
            convoys_raw: Some(i64::MAX),
            peace_conference_participant: 1,
            manpower_residual: -19,
            border_land_doctrine_level: None,
        };
        captured.countries_result = Outcome::Rows { len: 1 };
        captured.links[0] = engine_native::ccountry::LinkRow {
            id: 2,
            canonical_slot: 1,
        };
        captured.links_result = Outcome::Rows { len: 1 };
        captured.provinces.rows[0] = engine_native::cprovince::Row {
            id: 6,
            state: 4,
            owner: 7,
            controller: 2,
            victory_points: i32::MIN,
            ..Default::default()
        };
        captured.provinces_result = Outcome::Rows { len: 1 };
        captured.states[0] = engine_native::cstate::Row {
            id: 4,
            owner: 7,
            controller: u32::MAX,
            ..Default::default()
        };
        captured.states_result = Outcome::Rows { len: 1 };
        // Equal numeric ids in different native type namespaces remain separate
        // rows. Their roster order is intentionally opposite to wire key order.
        captured.divisions[0] = engine_native::carmy::Row {
            id: (u64::from(u32::MAX) << 32) | 4713,
            roster_owner: 7,
            logical_owner: 2,
            expeditionary_owner: 0,
            province: 6,
            template: u64::MAX,
            hit_points: i64::MIN,
            organisation: 9_007_199_254_740_993,
            orders_group: (57_u64 << 32) | 53,
            reveal_skip: 0,
        };
        captured.divisions[1] = engine_native::carmy::Row {
            id: (u64::from(u32::MAX) << 32) | 51,
            ..captured.divisions[0]
        };
        captured.divisions_result = Outcome::Rows { len: 2 };
        let frame = decoded(&captured, clock(10), 1);
        assert_eq!(
            frame.tables[2],
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                border_land_doctrine_level: replay_data::rows::Recorded::Value(None),
                id: 7,
                canonical_slot: 1,
                tag: b"GER".to_vec(),
                political_power_raw: None,
                command_power_raw: i64::MIN,
                stability_raw: 100_001,
                war_support_raw: -9_007_199_254_740_993,
                convoys_raw: Some(i64::MAX),
                peace_conference_participant: 1,
                manpower_residual: -19,
            }]))
        );
        let TableView::Complete(divisions) = &*frame.tables[TableId::Divisions.index()] else {
            panic!("distinct full division handles must survive conversion");
        };
        assert_eq!(
            divisions
                .rows::<tables::Divisions>()
                .unwrap()
                .iter()
                .map(DivisionRow::key)
                .collect::<Vec<_>>(),
            vec![
                u128::from(captured.divisions[1].id),
                u128::from(captured.divisions[0].id)
            ]
        );
        for division in divisions.rows::<tables::Divisions>().unwrap() {
            // Capture stores organisation at its display step, a tenth
            // truncated toward zero (replay_data::display); the sentinel stays.
            assert_eq!(division.hit_points, i64::MIN);
            assert_eq!(division.organisation, 9_007_199_254_740_000);
            assert_eq!(division.template, u64::MAX);
            assert_eq!(division.expeditionary_owner, 0);
            assert_eq!(division.orders_group, (57_u64 << 32) | 53);
        }
        assert_eq!(
            frame.tables[3],
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 2,
                canonical_slot: 1
            }]))
        );
        assert_eq!(
            frame.tables[4],
            TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
                id: 6,
                state: 4,
                owner: 7,
                controller: 2,
                victory_points: i32::MIN,
                fort_positive: 0,
                move_gate: 0,
                port_sea: 0,
                attack: Recorded::Value(replay_data::rows::Witnessed(None)),
                selected_areas: Default::default(),
                fort_bunker: Recorded::Value(None),
                controller_area: Default::default(),
                owner_area: Default::default(),
                area_node: Default::default(),
            }]))
        );
        let value = &frame.complete::<tables::Clock>().unwrap()[0];
        assert_eq!(value.play_time, u64::MAX);
        assert_eq!(
            frame.tables[TableId::States.index()],
            TableView::Complete(TableRows::States(vec![StateRow {
                id: 4,
                owner: 7,
                controller: u32::MAX,
                movement_infrastructure: 0,
                railway_infrastructure: replay_data::rows::Recorded::Value(None),
                continent: replay_data::rows::Recorded::Value(None),
                ..Default::default()
            }]))
        );
    }

    #[test]
    fn failures_remain_local_and_missing_clock_invalidates_conversion() {
        let mut captured = captured(10);
        captured.countries_result = Outcome::Unavailable {
            reason: "country pointer unreadable",
        };
        captured.links_result = Outcome::Truncated {
            len: 0,
            unread_rows: 9,
        };
        captured.states_result = Outcome::Unavailable {
            reason: "state pointer unreadable",
        };
        captured.divisions_result = Outcome::Unavailable {
            reason: "division pointer unreadable",
        };
        let frame = decoded(&captured, clock(10), 1);
        assert_eq!(frame.tables[2], unavailable("country pointer unreadable"));
        assert_eq!(
            frame.tables[3],
            TableView::Truncated {
                rows: TableRows::CountryLinks(Vec::new()),
                missing: 9
            }
        );
        assert_eq!(
            frame.tables[4],
            TableView::Complete(TableRows::Provinces(Vec::new()))
        );
        captured.provinces_result = Outcome::Unavailable {
            reason: "province pointer unreadable",
        };
        assert_eq!(
            decoded(&captured, clock(10), 1).tables[4],
            unavailable("province pointer unreadable")
        );
        captured.provinces_result = Outcome::Rows { len: 0 };
        assert_eq!(
            decoded(&captured, clock(10), 1).tables[4],
            TableView::Complete(TableRows::Provinces(Vec::new()))
        );
        assert_eq!(
            frame.tables[TableId::States.index()],
            unavailable("state pointer unreadable")
        );
        assert_eq!(
            frame.tables[TableId::Divisions.index()],
            unavailable("division pointer unreadable")
        );
        captured.clock = None;
        assert_eq!(
            decoded(&captured, clock(10), 1).tables,
            unavailable_tables("clock unreadable at hook entry").map(|input| match input {
                TableInput::Sampled(view) => view,
                _ => unreachable!("fixture expected ordinary sampled rows"),
            })
        );
    }

    #[test]
    fn border_conflicts_are_truncated_or_unavailable_with_their_provinces() {
        let rows = |len| Outcome::Rows { len };
        let truncated = |len, unread_rows| Outcome::Truncated { len, unread_rows };
        let failed = Outcome::Unavailable {
            reason: "conflicts",
        };
        let provinces = Outcome::Unavailable {
            reason: "provinces",
        };
        assert_eq!(border_conflicts_outcome(rows(9), rows(3)), rows(3));
        assert_eq!(border_conflicts_outcome(rows(9), failed), failed);
        assert_eq!(border_conflicts_outcome(provinces, rows(3)), provinces);
        // A truncated walk never reached some provinces' conflicts.
        assert_eq!(
            border_conflicts_outcome(truncated(9, 4), rows(3)),
            truncated(3, 4)
        );
        assert_eq!(
            border_conflicts_outcome(truncated(9, 4), truncated(3, 2)),
            truncated(3, 6)
        );
        // Through a decoded frame.
        let mut captured = captured(10);
        captured.provinces_result = truncated(0, 5);
        captured.provinces.conflict_walk.outcome = rows(0);
        let frame = decoded(&captured, clock(10), 1);
        assert!(matches!(
            *frame.tables[TableId::BorderConflicts.index()],
            TableView::Truncated { .. }
        ));
    }

    #[test]
    fn primary_failures_and_truncation_keep_independent_values() {
        let mut source = captured(10);
        source.states_result = Outcome::Unavailable {
            reason: "primary state pointer unreadable",
        };
        source.divisions_result = Outcome::Truncated {
            len: 0,
            unread_rows: 3,
        };
        source.factions[0] = engine_native::cfaction::Row {
            id: 88,
            name_start: 0,
            name_len: 3,
            ..Default::default()
        };
        source.factions_result = Outcome::Rows { len: 1 };
        source.faction_names[..3].copy_from_slice(&[b'N', 0, 255]);
        source.faction_names_len = 3;
        let frame = decoded(&source, clock(10), 1);
        for table in [
            TableId::CountryLinks,
            TableId::WarRelations,
            TableId::FactionMembers,
        ] {
            assert_eq!(
                frame.tables[table.index()],
                TableView::Complete(TableRows::empty(table))
            );
        }
        assert_eq!(
            frame.tables[TableId::States.index()],
            unavailable("primary state pointer unreadable")
        );
        assert_eq!(
            frame.tables[TableId::Divisions.index()],
            TableView::Truncated {
                rows: TableRows::Divisions(Vec::new()),
                missing: 3
            }
        );
        assert_eq!(
            frame.tables[TableId::Factions.index()],
            TableView::Complete(TableRows::Factions(vec![FactionRow {
                id: 88,
                name: vec![b'N', 0, 255],
                ..FactionRow::default()
            }]))
        );
    }

    #[test]
    fn healthy_hours_restore_failed_baselines_and_keep_new_values() {
        use replay_frame::{frame::FrameDecoder, frame::TableUpdate};

        let mut source = captured(10);
        source.states_result = Outcome::Unavailable {
            reason: "state pointer unreadable",
        };
        source.factions_result = Outcome::Unavailable {
            reason: "faction pointer unreadable",
        };
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let failed = encoder
            .encode(decoded(&source, clock(10), 1), true)
            .unwrap();
        decoder.apply(&failed).unwrap();

        source.states_result = Outcome::Rows { len: 1 };
        source.states[0] = engine_native::cstate::Row {
            id: 4,
            owner: 7,
            controller: 7,
            ..Default::default()
        };
        source.factions_result = Outcome::Rows { len: 1 };
        source.factions[0] = engine_native::cfaction::Row {
            id: 88,
            name_start: 0,
            name_len: 3,
            ..Default::default()
        };
        source.faction_names_len = 3;
        for hour in [11, 12] {
            source.clock = Some(clock(hour));
            source.states[0].controller = hour as u32;
            source.faction_names[..3].copy_from_slice(if hour == 11 { b"one" } else { b"two" });
            let expected = decoded(&source, clock(hour), 1);
            let frame = encoder.encode(expected.clone(), false).unwrap();
            for table in [TableId::States, TableId::Factions] {
                if hour == 11 {
                    assert!(matches!(
                        frame.tables[table.index()],
                        TableUpdate::Snapshot(_)
                    ));
                } else {
                    assert!(matches!(
                        frame.tables[table.index()],
                        TableUpdate::Delta { .. }
                    ));
                }
            }
            assert_eq!(decoder.apply(&frame).unwrap().tables, expected.tables);
        }
    }

    #[test]
    fn relation_and_faction_conversion_preserves_native_fields_and_member_order() {
        let mut source = captured(10);
        source.wars[0] = engine_native::cwar_relation::Row {
            first_country: u32::MAX,
            second_country: 1,
            start_hour: i32::MIN,
            end_hour: i32::MAX,
            cancel: 255,
            first_casualties: i64::MAX,
            second_casualties: i64::MIN,
            first_unknown_casualties: -1,
            second_unknown_casualties: 1_i64 << 40,
            first_was_instigator: 254,
        };
        source.wars_result = Outcome::Rows { len: 1 };
        source.factions[0] = engine_native::cfaction::Row {
            id: u64::MAX,
            name_start: 2,
            name_len: 4,
            ..Default::default()
        };
        source.factions_result = Outcome::Rows { len: 1 };
        source.faction_names[..6].copy_from_slice(&[0, 0, b'A', 0, 255, b'B']);
        source.faction_names_len = 6;
        source.faction_members[0] = engine_native::cfaction_system::MemberRow {
            country: 9,
            faction: u64::MAX,
            ordinal: 0,
        };
        source.faction_members[1] = engine_native::cfaction_system::MemberRow {
            country: 1,
            faction: u64::MAX,
            ordinal: 1,
        };
        source.faction_members_result = Outcome::Rows { len: 2 };
        let frame = decoded(&source, clock(10), 1);
        let wars = frame.complete::<tables::WarRelations>().unwrap();
        let war = &wars[0];
        assert_eq!(wars[0].key(), (u128::from(u32::MAX) << 32) | 1);
        assert_eq!((war.cancel, war.first_was_instigator), (255, 254));
        assert_eq!(
            (
                war.first_casualties,
                war.second_casualties,
                war.second_unknown_casualties
            ),
            (i64::MAX, i64::MIN, 1_i64 << 40)
        );
        assert_eq!(
            frame.tables[TableId::Factions.index()],
            TableView::Complete(TableRows::Factions(vec![FactionRow {
                id: u64::MAX,
                name: vec![b'A', 0, 255, b'B'],
                ..FactionRow::default()
            }]))
        );
        assert_eq!(
            frame.tables[TableId::FactionMembers.index()],
            TableView::Complete(TableRows::FactionMembers(vec![
                FactionMemberRow {
                    country: 1,
                    faction: u64::MAX,
                    ordinal: 1
                },
                FactionMemberRow {
                    country: 9,
                    faction: u64::MAX,
                    ordinal: 0
                },
            ]))
        );
        let encoded = FrameEncoder::new().encode(frame.clone(), true).unwrap();
        let decoded = replay_frame::frame::FrameDecoder::new()
            .apply(&encoded)
            .unwrap();
        assert_eq!(decoded.tables, frame.tables);
    }

    #[test]
    fn faction_byte_ranges_and_primary_failures_cannot_publish_partial_values_as_complete() {
        let mut source = captured(10);
        source.factions[0] = engine_native::cfaction::Row {
            id: 88,
            name_start: usize::MAX,
            name_len: 1,
            ..Default::default()
        };
        source.factions_result = Outcome::Rows { len: 1 };
        assert_eq!(
            faction_table(&source),
            unavailable("faction name lies outside the captured byte arena")
        );
        source.faction_members_result = Outcome::Unavailable {
            reason: "faction member pointer unreadable",
        };
        let frame = decoded(&source, clock(10), 1);
        assert_eq!(
            frame.tables[TableId::Factions.index()],
            unavailable("faction name lies outside the captured byte arena")
        );
        assert_eq!(
            frame.tables[TableId::FactionMembers.index()],
            unavailable("faction member pointer unreadable")
        );
    }

    #[test]
    fn table_conversion_rejects_duplicate_keys_and_path_validation_is_strict() {
        let source = [(2, i32::MIN), (1, i32::MAX)];
        let values = table_iter::<tables::Provinces, _>(
            Outcome::Truncated {
                len: 2,
                unread_rows: 3,
            },
            source.into_iter(),
            |(id, victory_points)| ProvinceRow {
                id,
                victory_points,
                ..Default::default()
            },
        );
        assert_eq!(
            values,
            TableView::Truncated {
                rows: TableRows::Provinces(vec![
                    ProvinceRow {
                        id: 1,
                        victory_points: i32::MAX,
                        ..Default::default()
                    },
                    ProvinceRow {
                        id: 2,
                        victory_points: i32::MIN,
                        ..Default::default()
                    },
                ]),
                missing: 3,
            }
        );
        assert_eq!(
            table_iter::<tables::Provinces, _>(
                Outcome::Rows { len: 1 },
                source.into_iter(),
                |(id, victory_points)| ProvinceRow {
                    id,
                    victory_points,
                    ..Default::default()
                }
            ),
            unavailable("walker row count exceeds allocated buffer"),
        );
        assert_eq!(
            table::<tables::Provinces, _>(Outcome::Rows { len: 2 }, &[1, 1], |id| ProvinceRow {
                id: *id,
                ..Default::default()
            }),
            unavailable("walker produced duplicate engine ids")
        );
        assert_eq!(
            table::<tables::Provinces, _>(Outcome::Rows { len: 3 }, &[1], |id| ProvinceRow {
                id: *id,
                ..Default::default()
            }),
            unavailable("walker row count exceeds allocated buffer")
        );
        assert_eq!(campaign_name(ID).unwrap().as_bytes(), ID);
        let mixed_case = b"21b7CC84-3028-4f65-8E0d-aDf2f37Ca1a6";
        assert_eq!(campaign_name(mixed_case).unwrap().as_bytes(), mixed_case);
        for id in [
            b"../outside".as_slice(),
            b"C:\\outside",
            b"21b7cc84/3028-4f65-8e0d-adf2f37ca1a6",
            b"",
        ] {
            assert!(campaign_name(id).is_err());
        }
    }

    #[derive(Default)]
    struct Store {
        segments: BTreeMap<u32, Vec<u8>>,
        fail_frame_flush: bool,
    }

    struct MemoryBackend(Rc<RefCell<Store>>);
    struct MemorySink {
        store: Rc<RefCell<Store>>,
        index: u32,
        last_tag: u8,
    }
    impl SegmentBackend for MemoryBackend {
        type Sink = MemorySink;
        fn create(&mut self, index: u32) -> io::Result<Self::Sink> {
            self.0.borrow_mut().segments.insert(index, Vec::new());
            Ok(MemorySink {
                store: self.0.clone(),
                index,
                last_tag: 255,
            })
        }
    }
    impl Write for MemorySink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.last_tag = bytes[replay::spool::RECORD_HEADER_BYTES];
            self.store
                .borrow_mut()
                .segments
                .get_mut(&self.index)
                .unwrap()
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            let mut store = self.store.borrow_mut();
            if store.fail_frame_flush && self.last_tag == 2 {
                store.fail_frame_flush = false;
                return Err(io::Error::other("injected flush failure"));
            }
            Ok(())
        }
    }
    impl SegmentSink for MemorySink {
        fn sync_all(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    pub(super) fn session() -> SessionStart {
        SessionStart {
            id: 1,
            process_id: 100,
            process_start_time: 1000,
            qpc_frequency: 10_000_000,
            sample_boundary: replay_data::identity::SampleBoundary::EndOfHour,
            identity: Identity {
                game_version: "fixture".into(),
                exe_hash: [1; 32],
                exe_size: 1,
                vanilla_hash: [2; 32],
                dlc: Vec::new(),
                mods: Vec::new(),
                coverage_policy: 1,
                language: "english".into(),
                recorder_version: "fixture".into(),
                collector_hash: [3; 32],
                verification: Verification::Unchecked,
            },
        }
    }

    fn campaign() -> (Campaign<MemoryBackend>, Rc<RefCell<Store>>) {
        let mut tracker = BranchTracker::new();
        tracker.begin_session(session().id).unwrap();
        campaign_with(tracker)
    }

    fn campaign_with(tracker: BranchTracker) -> (Campaign<MemoryBackend>, Rc<RefCell<Store>>) {
        let store = Rc::new(RefCell::new(Store::default()));
        let session = session();
        (
            Campaign {
                validation_refusals: ValidationRefusals::default(),
                world_generation: None,
                directory: PathBuf::new(),
                session,
                resumed: tracker.last_observation(),
                tracker,
                encoder: FrameEncoder::new(),
                spool: SegmentedSpool::new(MemoryBackend(store.clone())),
                disk_branch: None,
                last_written: None,
                pending: Vec::new(),
                staggered_cache: TableCache::new(),
                staggered_branch: None,
                country_intel_sample: None,
                country_intel_hour: None,
                sliced_cache: TableCache::new(),
                sliced_reads: ReadHours::new(),
                sliced_branch: None,
                pending_slices: Vec::new(),
                retired: Vec::new(),
                line_output: Default::default(),
                operation_outcomes: Default::default(),
                map_events: Default::default(),
                politics: Default::default(),
                construction_ic: Default::default(),
                fuel_awards: Default::default(),
                variables_scratch: ScriptVariableComparison::default(),
                character_shadow: CharacterShadow::default(),
                intel_shadow: IntelShadow::default(),
                projection_scratch: ProjectionScratch::default(),
                lexical_conversion: LexicalConversionStats::default(),
                _history_file: None,
            },
            store,
        )
    }

    fn publish(
        campaign: &mut Campaign<MemoryBackend>,
        hour: i32,
        signal: i64,
    ) -> io::Result<BranchId> {
        let branch = campaign.observe(Some(hour - 1), hour, Some(signal))?;
        let source = decoded(&captured(hour), clock(hour), branch);
        campaign.write(
            source,
            1000 + i64::from(hour),
            Some(signal),
            0,
            &mut WorkSample::default(),
        )?;
        Ok(branch)
    }

    #[test]
    fn uncertain_frame_is_not_retried_and_reload_losses_keep_their_branches() {
        let (mut campaign, store) = campaign();
        let first = publish(&mut campaign, 10, 1).unwrap();
        store.borrow_mut().fail_frame_flush = true;
        assert!(publish(&mut campaign, 11, 1).is_err());
        assert_eq!(
            campaign.tracker.recorded_hours(first).unwrap().to_vec(),
            vec![10]
        );
        campaign
            .lost(
                &LostObservation {
                    campaign_id: owned_text(ID),
                    first_hour: 5,
                    last_hour: 7,
                    loaded_hour: Some(4),
                    load_signal: Some(2),
                    count: 3,
                },
                false,
            )
            .unwrap();
        let second = campaign.tracker.current_branch().unwrap();
        let third = publish(&mut campaign, 11, 3).unwrap();
        assert_ne!(second, third);
        assert_eq!(
            campaign.tracker.branch_start(third).unwrap().parent,
            Parentage::Probable {
                branch: first,
                hour: 10
            }
        );
        assert_eq!(
            campaign.tracker.branch_start(second).unwrap().reason,
            BranchReason::LoadSignal
        );
        campaign.detach(0).unwrap();
        let recovered: Vec<_> = store
            .borrow()
            .segments
            .iter()
            .map(|(&index, bytes)| (index, recover_prefix(bytes)))
            .collect();
        let replay = Replay::from_recovered(ID.to_vec(), &recovered).unwrap();
        // Complete bytes may survive a failed flush. They occur only once.
        assert_eq!(replay.recorded_hours(first).unwrap(), vec![10, 11]);
        assert!(matches!(
            replay.seek(first, 11).unwrap(),
            SeekResult::Frame(_)
        ));
        for hour in 5..=7 {
            assert!(
                matches!(replay.seek(second, hour).unwrap(), SeekResult::Gap { reason, .. } if reason == "collector queue overflow")
            );
        }
        assert_eq!(replay.branches.len(), 3);
        assert!(matches!(
            replay.seek(third, 11).unwrap(),
            SeekResult::Frame(_)
        ));
    }

    /// End-of-hour boundary: the first hook call after a load reads the loaded
    /// hour L at the entry of tick L+1, so the new branch starts at L and its
    /// parent is the branch that recorded L. Detach writes no frame for the
    /// hour still open when it happens; the last hour needs one more tick.
    #[test]
    fn end_of_hour_reload_starts_at_the_loaded_hour_and_detach_invents_no_sample() {
        let (mut campaign, store) = campaign();
        let write = |campaign: &mut Campaign<MemoryBackend>, hour: i32, signal: i64| {
            let mut source = captured(hour);
            source.clock.as_mut().unwrap().play_session_started_at = signal;
            let current = source.clock.as_ref().unwrap().copy_owned().unwrap();
            let origin = campaign.tracker.current_branch().map(|_| current.hour);
            assert!(origin.is_none() || origin == Some(hour));
            let branch = campaign.observe(origin, hour, Some(signal)).unwrap();
            let frame = decoded(&source, current, branch);
            campaign
                .write(
                    frame,
                    1000 + i64::from(hour),
                    Some(signal),
                    0,
                    &mut WorkSample::default(),
                )
                .unwrap();
            branch
        };
        let first = write(&mut campaign, 10, 1);
        assert_eq!(write(&mut campaign, 11, 1), first);
        assert_eq!(write(&mut campaign, 12, 1), first);
        // Load the hour-11 save: the first tick after the load samples hour 11.
        let reloaded = write(&mut campaign, 11, 2);
        assert_ne!(reloaded, first);
        let start = campaign.tracker.branch_start(reloaded).unwrap();
        assert_eq!(start.reason, BranchReason::LoadSignal);
        assert_eq!(start.first_hour, 11);
        assert_eq!(start.loaded_hour, Some(11));
        assert_eq!(
            start.parent,
            Parentage::Probable {
                branch: first,
                hour: 11
            }
        );
        campaign.detach(0).unwrap();
        let recovered: Vec<_> = store
            .borrow()
            .segments
            .iter()
            .map(|(&index, bytes)| (index, recover_prefix(bytes)))
            .collect();
        let replay = Replay::from_recovered(ID.to_vec(), &recovered).unwrap();
        assert_eq!(replay.recorded_hours(first).unwrap(), vec![10, 11, 12]);
        assert_eq!(replay.recorded_hours(reloaded).unwrap(), vec![11]);
        assert_eq!(
            replay.sessions[0].sample_boundary,
            replay_data::identity::SampleBoundary::EndOfHour
        );
        let last_detach = replay
            .controls
            .iter()
            .rev()
            .find_map(|control| match control {
                ControlRecord::Detach { last_hour, .. } => Some(*last_hour),
                _ => None,
            });
        assert_eq!(last_detach, Some(Some(11)));
    }

    #[test]
    fn a_failed_final_write_does_not_duplicate_an_hour_to_manufacture_detach() {
        let (mut campaign, store) = campaign();
        publish(&mut campaign, 10, 1).unwrap();
        store.borrow_mut().fail_frame_flush = true;
        assert!(publish(&mut campaign, 11, 1).is_err());
        assert!(campaign.detach(0).is_err());
        assert_eq!(store.borrow().segments.len(), 1);
    }

    #[test]
    fn unknown_tail_loss_is_written_at_detach_without_inventing_a_range() {
        let (mut campaign, store) = campaign();
        let branch = publish(&mut campaign, 10, 1).unwrap();
        campaign.detach(3).unwrap();
        let recovered: Vec<_> = store
            .borrow()
            .segments
            .iter()
            .map(|(&index, bytes)| (index, recover_prefix(bytes)))
            .collect();
        let replay = Replay::from_recovered(ID.to_vec(), &recovered).unwrap();
        assert_eq!(replay.recorded_hours(branch).unwrap(), vec![10]);
        assert!(replay.gaps.is_empty());
        assert!(replay.controls.iter().any(|record| matches!(record,
            ControlRecord::GapSummary { dropped_frames: 3, reason, .. } if reason.contains("hour range are unknown"))));
    }

    fn recovered(store: &Rc<RefCell<Store>>) -> Replay {
        let recovered: Vec<_> = store
            .borrow()
            .segments
            .iter()
            .map(|(&index, bytes)| (index, recover_prefix(bytes)))
            .collect();
        Replay::from_recovered(ID.to_vec(), &recovered).unwrap()
    }

    fn event_losses(replay: &Replay) -> Vec<(BranchId, i32, i32)> {
        replay
            .controls
            .iter()
            .filter_map(|control| match *control {
                ControlRecord::EventLoss {
                    session: owner,
                    branch,
                    after_hour,
                    hour,
                } => {
                    assert_eq!(owner, session().id);
                    Some((branch, after_hour, hour))
                }
                _ => None,
            })
            .collect()
    }

    /// A same-process reattach that continues its branch records an event
    /// loss between the detached session's last read and the reattached
    /// session's first: event completeness was lost for part of that interval.
    /// A fresh session, and a reattach whose first observation starts another
    /// branch, record none.
    #[test]
    fn a_same_process_reattach_records_the_unobserved_event_interval_once() {
        // The frames' Clock rows carry this load signal; the resumed tail reads it.
        let signal = clock(10).play_session_started_at;
        let (mut first, store) = campaign();
        let branch = publish(&mut first, 10, signal).unwrap();
        publish(&mut first, 11, signal).unwrap();
        first.detach(0).unwrap();
        let earlier = recovered(&store);
        assert!(event_losses(&earlier).is_empty());
        let reattach = || {
            let history = recovered(&store).into_history().unwrap();
            campaign_with(history.tracker_for(&session()).unwrap())
        };

        let (mut second, appended) = reattach();
        assert_eq!(publish(&mut second, 12, signal).unwrap(), branch);
        assert_eq!(publish(&mut second, 13, signal).unwrap(), branch);
        second.detach(0).unwrap();
        let resumed = recovered(&appended);
        assert_eq!(event_losses(&resumed), [(branch, 11, 12)]);
        assert_eq!(resumed.event_losses(branch).collect::<Vec<_>>(), [(11, 12)]);
        assert!(resumed.gaps.is_empty());
        assert_eq!(resumed.recorded_hours(branch).unwrap(), vec![12, 13]);

        // Time passed while detached: the first observation is a new branch,
        // whose first interval is its own, so no loss is recorded.
        let (mut moved, later) = reattach();
        assert_ne!(publish(&mut moved, 15, signal).unwrap(), branch);
        moved.detach(0).unwrap();
        assert!(event_losses(&recovered(&later)).is_empty());
    }

    /// The reattached session's first frame can fail after its context was
    /// written, or be lost to the queue: the event loss still ends at that
    /// first observation, is kept once after the retry, and the lost frame
    /// stays its own gap.
    #[test]
    fn a_reattach_whose_first_frame_is_lost_keeps_one_event_loss_and_the_gap() {
        let signal = clock(10).play_session_started_at;
        let (mut first, store) = campaign();
        let branch = publish(&mut first, 10, signal).unwrap();
        publish(&mut first, 11, signal).unwrap();
        first.detach(0).unwrap();
        let reattach = || {
            let history = recovered(&store).into_history().unwrap();
            campaign_with(history.tracker_for(&session()).unwrap())
        };

        let (mut failed, appended) = reattach();
        appended.borrow_mut().fail_frame_flush = true;
        assert!(publish(&mut failed, 12, signal).is_err());
        assert_eq!(publish(&mut failed, 13, signal).unwrap(), branch);
        failed.detach(0).unwrap();
        let retried = recovered(&appended);
        assert_eq!(event_losses(&retried), [(branch, 11, 12)]);
        assert!(
            retried
                .gaps
                .iter()
                .any(|gap| gap.first_hour == 12 && gap.last_hour == 12)
        );

        let (mut dropped, later) = reattach();
        dropped
            .lost(
                &LostObservation {
                    campaign_id: owned_text(ID),
                    first_hour: 12,
                    last_hour: 12,
                    loaded_hour: Some(11),
                    load_signal: Some(signal),
                    count: 1,
                },
                false,
            )
            .unwrap();
        assert_eq!(publish(&mut dropped, 13, signal).unwrap(), branch);
        dropped.detach(0).unwrap();
        let queued = recovered(&later);
        assert_eq!(event_losses(&queued), [(branch, 11, 12)]);
        assert_eq!(queued.recorded_hours(branch).unwrap(), vec![13]);
        assert!(
            queued
                .gaps
                .iter()
                .any(|gap| gap.first_hour == 12 && gap.last_hour == 12)
        );
    }

    #[test]
    fn campaign_preflight_retries_unopened_campaigns_before_consuming_any_witness() {
        let second = b"31b7cc84-3028-4f65-8e0d-adf2f37ca1a6".as_slice();
        let third = b"41b7cc84-3028-4f65-8e0d-adf2f37ca1a6".as_slice();
        let ids = [ID, b"../invalid", second, ID, third];
        let mut prepared = BTreeSet::new();
        let mut attempts = Vec::new();
        let mut fail_once = true;
        let mut open = |id: &str| -> io::Result<()> {
            if prepared.contains(id) {
                return Ok(());
            }
            attempts.push(id.to_owned());
            if id.as_bytes() == second && fail_once {
                fail_once = false;
                return Err(io::Error::other("temporary campaign directory failure"));
            }
            prepared.insert(id.to_owned());
            Ok(())
        };
        assert!(prepare_campaigns(ids, &mut open).is_err());
        prepare_campaigns(ids, &mut open).unwrap();
        assert_eq!(
            attempts.iter().map(String::as_bytes).collect::<Vec<_>>(),
            vec![ID, second, second, third]
        );
        assert_eq!(prepared.len(), 3);
        // A malformed-only batch completes preflight without an endlessly
        // retryable open failure; consumption below still records its loss.
        prepare_campaigns([b"malformed".as_slice()], |_| {
            panic!("invalid path was opened")
        })
        .unwrap();
    }

    #[test]
    fn malformed_campaign_witnesses_become_unknown_losses_without_filesystem_access() {
        let mut processor = Processor {
            root: PathBuf::new(),
            history: PreparedCampaigns::default(),
            session: session(),
            campaigns: Campaigns::new(),
            stats: Statistics::default(),
            unknown_pending: 0,
            suppress_origin: false,
            retry_full: false,
            unpublished_families: 0,
            helper: None,
            second_helper: None,
            backlogged: false,
            comparison: Box::default(),
        };
        let loss = LostObservation {
            campaign_id: owned_text(b"../invalid"),
            count: 3,
            first_hour: 1,
            last_hour: 3,
            loaded_hour: Some(0),
            load_signal: Some(1),
        };
        assert!(processor.lost(&loss).is_err());
        let mut captured = captured(4);
        captured.clock.as_mut().unwrap().campaign_id = owned_text(b"../invalid");
        captured.table_reads[TableId::Countries.index()] = ReadCounts {
            copy_calls: 1,
            spans: 1,
            bytes: u64::MAX,
            ..ReadCounts::ZERO
        };
        processor.prepare(&captured).unwrap();
        assert!(processor.frame(&captured).is_err());
        assert_eq!(processor.unknown_pending, 4);
        assert!(processor.suppress_origin);
        assert!(processor.campaigns.is_empty());
        assert_eq!(
            processor.stats.table_total_reads,
            [ReadCounts::ZERO; TABLE_COUNT]
        );
        assert_eq!(processor.json()["table_reads_frames"], 0);
    }

    fn research_rows(rows: &[(u32, u32, i64, i32)]) -> TableRows {
        TableRows::ResearchInputs(
            rows.iter()
                .map(
                    |&(country, index, progress_raw, _read)| replay_data::rows::ResearchInputRow {
                        country,
                        index,
                        progress_raw,
                        ..Default::default()
                    },
                )
                .collect(),
        )
    }

    fn sliced_input(branch: BranchId, hour: i32, view: TableView) -> FrameInput {
        let mut input: FrameInput = staggered_frame(branch, hour, false).into();
        input.tables[TableId::ResearchInputs.index()] = TableInput::Sampled(view);
        input
    }

    fn merged(input: &FrameInput) -> &TableInput {
        &input.tables[TableId::ResearchInputs.index()]
    }

    /// The (owner, read hour) pairs SliceReads holds for ResearchInputs.
    fn reads(input: &FrameInput) -> Vec<(u32, i32)> {
        let TableInput::SharedComplete(rows) = &input.tables[TableId::SliceReads.index()] else {
            panic!("slice reads assembled")
        };
        let TableRows::SliceReads(rows) = &**rows else {
            panic!("slice read rows")
        };
        rows.iter()
            .filter(|row| row.table == TableId::ResearchInputs.index() as u32)
            .map(|row| (row.owner, row.sample_hour))
            .collect()
    }

    fn merge_with(
        campaign: &mut Campaign<MemoryBackend>,
        input: &mut FrameInput,
        selection: crate::slicing::SliceSelection,
    ) -> bool {
        campaign
            .merge_sliced_inputs(input, |_| Some(&selection), |_| true)
            .0
    }

    #[test]
    fn a_walked_slice_that_fails_conversion_reports_its_family() {
        use crate::slicing::{RESEARCH, SliceSelection};
        let (mut campaign, _) = campaign();
        let selection = SliceSelection::for_test(1, false, &[]);
        let failed = || {
            sliced_input(
                1,
                1,
                TableView::Unavailable {
                    reason: "walker produced duplicate engine ids".into(),
                },
            )
        };
        // The walk published rows, conversion withheld them: forget the family.
        let (retry, unpublished) =
            campaign.merge_sliced_inputs(&mut failed(), |_| Some(&selection), |_| true);
        assert_eq!(unpublished, 1 << RESEARCH);
        assert!(retry);
        // A failed walk is invalidated by the collector itself.
        let (_, unpublished) =
            campaign.merge_sliced_inputs(&mut failed(), |_| Some(&selection), |_| false);
        assert_eq!(unpublished, 0);
        // A skipped table was never read.
        let mut skipped = sliced_input(
            1,
            1,
            TableView::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON.into(),
            },
        );
        let (_, unpublished) =
            campaign.merge_sliced_inputs(&mut skipped, |_| Some(&selection), |_| true);
        assert_eq!(unpublished, 0);
    }

    /// A sliced table's first hour on a branch is a full read: it publishes
    /// its complete rows with no earlier ones and records every owner's read
    /// hour. Only a truncated first read has nothing to merge into, which is
    /// why a full hour's capture must hold every row.
    #[test]
    fn a_first_full_hour_publishes_complete_sliced_rows_without_earlier_ones() {
        use crate::slicing::SliceSelection;
        let (mut first_campaign, _) = campaign();
        let rows = research_rows(&[(1, 0, 5, 0), (2, 0, 6, 0)]);
        let mut first = sliced_input(1, 0, TableView::Complete(rows.clone()));
        assert!(!merge_with(
            &mut first_campaign,
            &mut first,
            SliceSelection::for_test(0, true, &[]).with_census(&[1, 2])
        ));
        assert!(matches!(merged(&first), TableInput::SharedComplete(merged) if **merged == rows));
        assert_eq!(reads(&first), [(1, 0), (2, 0)]);
        let (mut fresh, _) = campaign();
        let mut truncated = sliced_input(
            1,
            0,
            TableView::Truncated {
                rows: research_rows(&[(1, 0, 5, 0)]),
                missing: 1,
            },
        );
        assert!(merge_with(
            &mut fresh,
            &mut truncated,
            SliceSelection::for_test(0, true, &[]).with_census(&[1, 2])
        ));
        assert!(matches!(
            merged(&truncated),
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
    }

    #[test]
    fn no_hour_merges_rows_from_before_an_unpublished_read() {
        use crate::slicing::SliceSelection;
        let (mut campaign, _) = campaign();
        let full = |hour| {
            sliced_input(
                1,
                hour,
                TableView::Complete(research_rows(&[(1, 0, 5, hour), (2, 0, 6, hour)])),
            )
        };
        assert!(!merge_with(
            &mut campaign,
            &mut full(0),
            SliceSelection::for_test(0, true, &[])
        ));
        // Hour 1's read is withheld by conversion.
        let mut failed = sliced_input(
            1,
            1,
            TableView::Unavailable {
                reason: "walker produced duplicate engine ids".into(),
            },
        );
        let selection = SliceSelection::for_test(1, false, &[]);
        let (_, unpublished) =
            campaign.merge_sliced_inputs(&mut failed, |_| Some(&selection), |_| true);
        assert_ne!(unpublished, 0);
        assert_eq!(reads(&failed), [(1, 0), (2, 0)]);
        // Hour 2 was captured before the game thread forgot: its slice is not
        // merged onto hour 0's rows, and a full refresh is requested.
        let mut slice = sliced_input(1, 2, TableView::Complete(research_rows(&[(2, 0, 7, 2)])));
        assert!(merge_with(
            &mut campaign,
            &mut slice,
            SliceSelection::for_test(2, false, &[])
        ));
        assert!(matches!(
            merged(&slice),
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
        assert!(reads(&slice).is_empty());
        // The full read publishes again.
        let mut refreshed = full(3);
        assert!(!merge_with(
            &mut campaign,
            &mut refreshed,
            SliceSelection::for_test(3, true, &[])
        ));
        assert!(matches!(merged(&refreshed), TableInput::SharedComplete(_)));
    }

    fn failed_slice_merge_recovers(table: TableId, truncated: bool) {
        use crate::slicing::{FAMILIES, SliceFingerprints, SliceSelection, family_of};

        let rows = |values: &[(u32, i64)]| match table {
            TableId::ResearchInputs => TableRows::ResearchInputs(
                values
                    .iter()
                    .map(
                        |&(country, progress_raw)| replay_data::rows::ResearchInputRow {
                            country,
                            progress_raw,
                            ..Default::default()
                        },
                    )
                    .collect(),
            ),
            TableId::StateResourceInputs => TableRows::StateResourceInputs(
                values
                    .iter()
                    .map(
                        |&(state, global_factor)| replay_data::rows::StateResourceInputRow {
                            state,
                            global_factor,
                            ..Default::default()
                        },
                    )
                    .collect(),
            ),
            _ => unreachable!(),
        };
        let frame = |hour, view| {
            let mut input: FrameInput = staggered_frame(1, hour, false).into();
            input.tables[table.index()] = TableInput::Sampled(view);
            input
        };
        let scan = |store: &mut SliceFingerprints, hour, full| {
            let mut selection = SliceSelection::for_test(hour, full, &[]);
            for owner in [1, 2, 3] {
                // Research changes its print at hour 1; raid state inputs use
                // a constant print and read the changed owner in its slice.
                let print = u64::from(table == TableId::ResearchInputs && owner == 1 && hour > 0);
                selection.scan(store, owner, Some(print));
            }
            selection.finish_census(store);
            store.finish_scan();
            selection
        };
        let merge = |campaign: &mut Campaign<MemoryBackend>,
                     input: &mut FrameInput,
                     selection: &SliceSelection| {
            campaign.merge_sliced_inputs(input, |_| Some(selection), |_| true)
        };
        let family = family_of(table).unwrap();
        let (mut campaign, _) = campaign();
        let mut store = SliceFingerprints::new();
        let initial = rows(&[(1, 10), (2, 20), (3, 30)]);
        let mut first = frame(0, TableView::Complete(initial));
        assert_eq!(
            merge(&mut campaign, &mut first, &scan(&mut store, 0, true)),
            (false, 0)
        );

        let selection = scan(&mut store, 1, false);
        assert!(selection.contains(1));
        assert!(!selection.contains(2));
        // Sorted, unique rows pass conversion, but owner 2 was not selected:
        // merge_slice itself rejects this read after owner 1's print advanced.
        let bad_rows = rows(&[(1, 11), (2, 20)]);
        let bad_view = if truncated {
            selection.invalidate(&mut store);
            TableView::Truncated {
                rows: bad_rows,
                missing: 1,
            }
        } else {
            TableView::Complete(bad_rows)
        };
        let mut failed = frame(1, bad_view);

        // These frames are already captured when the spool discovers the error.
        let in_flight: Vec<_> = (2..=3)
            .map(|hour| {
                let selection = scan(&mut store, hour, false);
                if !truncated {
                    assert!(
                        !selection.contains(1),
                        "the failed read's print is already held"
                    );
                }
                let values: Vec<_> = [(1, 11), (2, 22), (3, 33)]
                    .into_iter()
                    .filter(|&(owner, _)| selection.contains(owner))
                    .collect();
                (frame(hour, TableView::Complete(rows(&values))), selection)
            })
            .collect();
        let (retry, unpublished) = merge(&mut campaign, &mut failed, &selection);
        assert!(matches!(
            &failed.tables[table.index()],
            TableInput::Sampled(TableView::Unavailable { reason })
                if reason == "sliced table merge failed"
        ));
        for (mut input, selection) in in_flight {
            let (retry, _) = merge(&mut campaign, &mut input, &selection);
            assert!(
                matches!(
                    &input.tables[table.index()],
                    TableInput::Sampled(TableView::Unavailable { .. })
                ),
                "hour {} reused rows predating the failed merge: {:?}",
                input.hour,
                input.tables[table.index()]
            );
            assert!(retry);
        }
        assert!(retry);
        assert_eq!(unpublished, 1 << family);
        for candidate in TableId::ALL
            .into_iter()
            .filter(|&id| replay_data::cadence::is_sliced(id))
        {
            let affected = FAMILIES[family].tables.contains(&candidate);
            assert_eq!(
                campaign.sliced_cache.contains_key(&candidate),
                !affected,
                "{candidate:?}"
            );
            assert_eq!(
                campaign
                    .sliced_reads
                    .keys()
                    .any(|(id, _)| id == candidate.index() as u32),
                !affected && !replay_data::sliced::is_keyed(candidate),
                "{candidate:?} read hours"
            );
        }

        // Deliver the spool's feedback. Forgetting reads every owner even off
        // slice; the requested full refresh restores exact rows and read hours.
        store.forget();
        let selection = scan(&mut store, 4, false);
        assert_eq!(selection.read_owners().collect::<Vec<_>>(), [1, 2, 3]);
        let expected = rows(&[(1, 11), (2, 22), (3, 33)]);
        let mut full = frame(4, TableView::Complete(expected.clone()));
        assert_eq!(
            merge(&mut campaign, &mut full, &scan(&mut store, 4, true)),
            (false, 0)
        );
        // Ordinary hours can carry that recovered baseline indefinitely.
        for hour in 5..=6 {
            let mut later = frame(hour, TableView::Complete(rows(&[])));
            assert_eq!(
                merge(&mut campaign, &mut later, &scan(&mut store, hour, false)),
                (false, 0)
            );
            assert!(
                matches!(&later.tables[table.index()], TableInput::SharedComplete(held) if **held == expected)
            );
            for owner in [1, 2, 3] {
                assert_eq!(
                    campaign.sliced_reads.get(&(table.index() as u32, owner)),
                    Some(&4)
                );
            }
        }
    }

    #[test]
    fn failed_research_slice_merge_withholds_in_flight_frames_until_full_refresh() {
        for truncated in [false, true] {
            failed_slice_merge_recovers(TableId::ResearchInputs, truncated);
        }
    }

    #[test]
    fn failed_state_slice_merge_withholds_in_flight_frames_until_full_refresh() {
        for truncated in [false, true] {
            failed_slice_merge_recovers(TableId::StateResourceInputs, truncated);
        }
    }

    /// The supply trains (owner decision 2026-10-11: daily slices, hourly
    /// before) merge by country: a country's node and vertex rows and its
    /// inventory row take the same read hour, an hour's slice keeps every
    /// other country's rows and hours, and a vanished country leaves at once.
    #[test]
    fn supply_trains_merge_each_countrys_nodes_and_inventory_at_its_read_hour() {
        use crate::slicing::SliceSelection;
        use replay_data::{rows::SupplyNodeRow, rows::TrainInventoryRow};
        const VERTEX: u32 = SupplyNodeRow::VERTEX;
        let nodes = |rows: &[(u32, u32, i64)]| {
            TableRows::SupplyNodes(
                rows.iter()
                    .map(|&(country, index, available)| SupplyNodeRow {
                        country,
                        index,
                        available,
                        ..Default::default()
                    })
                    .collect(),
            )
        };
        let trains = |rows: &[(u32, u32)]| {
            TableRows::TrainInventory(
                rows.iter()
                    .map(|&(country, node_count)| TrainInventoryRow {
                        country,
                        node_count,
                        ..Default::default()
                    })
                    .collect(),
            )
        };
        let input = |hour: i32, read_nodes: TableRows, read_trains: TableRows| {
            let mut input: FrameInput = staggered_frame(1, hour, false).into();
            input.tables[TableId::SupplyNodes.index()] =
                TableInput::Sampled(TableView::Complete(read_nodes));
            input.tables[TableId::TrainInventory.index()] =
                TableInput::Sampled(TableView::Complete(read_trains));
            input
        };
        let held = |input: &FrameInput, table: TableId| {
            let TableInput::SharedComplete(rows) = &input.tables[table.index()] else {
                panic!("{table:?} merged")
            };
            TableRows::clone(rows)
        };
        let read_hours = |input: &FrameInput, table: TableId| {
            let TableInput::SharedComplete(rows) = &input.tables[TableId::SliceReads.index()]
            else {
                panic!("slice reads assembled")
            };
            let TableRows::SliceReads(rows) = &**rows else {
                panic!("slice read rows")
            };
            rows.iter()
                .filter(|row| row.table == table.index() as u32)
                .map(|row| (row.owner, row.sample_hour))
                .collect::<Vec<_>>()
        };
        let (mut campaign, _) = campaign();
        // Hour 0 is a full hour: every country is read.
        let mut full = input(
            0,
            nodes(&[(1, 0, 10), (1, VERTEX | 7, 0), (2, 0, 20), (25, 0, 250)]),
            trains(&[(1, 1), (2, 1), (25, 1)]),
        );
        assert!(!merge_with(
            &mut campaign,
            &mut full,
            SliceSelection::for_test(0, true, &[]).with_census(&[1, 2, 25])
        ));
        // Hour 1 reads slice 1, countries 1 and 25; country 2 keeps hour 0.
        let mut slice = input(
            1,
            nodes(&[(1, 0, 11), (1, VERTEX | 8, 0), (25, 0, 251)]),
            trains(&[(1, 1), (25, 1)]),
        );
        assert!(!merge_with(
            &mut campaign,
            &mut slice,
            SliceSelection::for_test(1, false, &[]).with_census(&[1, 2, 25])
        ));
        assert_eq!(
            held(&slice, TableId::SupplyNodes),
            nodes(&[(1, 0, 11), (1, VERTEX | 8, 0), (2, 0, 20), (25, 0, 251)])
        );
        assert_eq!(
            held(&slice, TableId::TrainInventory),
            trains(&[(1, 1), (2, 1), (25, 1)])
        );
        for table in [TableId::SupplyNodes, TableId::TrainInventory] {
            assert_eq!(
                read_hours(&slice, table),
                [(1, 1), (2, 0), (25, 1)],
                "{table:?}"
            );
        }
        // Hour 3 reads nobody: every country holds its own read and hour.
        let mut quiet = input(3, nodes(&[]), trains(&[]));
        assert!(!merge_with(
            &mut campaign,
            &mut quiet,
            SliceSelection::for_test(3, false, &[]).with_census(&[1, 2, 25])
        ));
        assert_eq!(
            held(&quiet, TableId::SupplyNodes),
            held(&slice, TableId::SupplyNodes)
        );
        for table in [TableId::SupplyNodes, TableId::TrainInventory] {
            assert_eq!(
                read_hours(&quiet, table),
                [(1, 1), (2, 0), (25, 1)],
                "{table:?}"
            );
        }
        // A census without country 25 drops its rows now, not at its slice.
        let mut vanished = input(4, nodes(&[]), trains(&[]));
        assert!(!merge_with(
            &mut campaign,
            &mut vanished,
            SliceSelection::for_test(4, false, &[]).with_census(&[1, 2])
        ));
        assert_eq!(
            held(&vanished, TableId::SupplyNodes),
            nodes(&[(1, 0, 11), (1, VERTEX | 8, 0), (2, 0, 20)])
        );
        assert_eq!(
            held(&vanished, TableId::TrainInventory),
            trains(&[(1, 1), (2, 1)])
        );
    }

    #[test]
    fn slices_replace_only_their_owners_and_keep_other_owners_rows_and_hours() {
        use crate::slicing::SliceSelection;
        let (mut campaign, _) = campaign();
        // Without earlier complete rows a slice cannot stand alone.
        let mut orphan = sliced_input(1, 1, TableView::Complete(research_rows(&[(1, 0, 5, 1)])));
        assert!(merge_with(
            &mut campaign,
            &mut orphan,
            SliceSelection::for_test(1, false, &[])
        ));
        assert!(matches!(
            merged(&orphan),
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
        // A full read is complete by itself.
        let full_rows =
            research_rows(&[(1, 0, 10, 0), (1, 1, 11, 0), (3, 0, 30, 0), (25, 0, 250, 0)]);
        let mut full = sliced_input(1, 0, TableView::Complete(full_rows.clone()));
        assert!(!merge_with(
            &mut campaign,
            &mut full,
            SliceSelection::for_test(0, true, &[])
        ));
        let TableInput::SharedComplete(rows) = merged(&full) else {
            panic!("full read complete")
        };
        assert_eq!(**rows, full_rows);
        // Hour 1 reads slice 1 (owners 1 and 25) plus an early re-read of 3.
        // Owner 1 lost a slot, owner 25 is gone, owner 3 changed; the rest stays.
        let mut slice = sliced_input(
            1,
            1,
            TableView::Complete(research_rows(&[(1, 0, 12, 1), (3, 0, 31, 1)])),
        );
        assert!(!merge_with(
            &mut campaign,
            &mut slice,
            SliceSelection::for_test(1, false, &[3])
        ));
        let TableInput::SharedComplete(rows) = merged(&slice) else {
            panic!("slice merged")
        };
        assert_eq!(**rows, research_rows(&[(1, 0, 12, 1), (3, 0, 31, 1)]));
        assert_eq!(reads(&slice), [(1, 1), (3, 1)]);
        // Hour 2 reads nobody here; owners keep their own read hours.
        let mut quiet = sliced_input(1, 2, TableView::Complete(research_rows(&[])));
        assert!(!merge_with(
            &mut campaign,
            &mut quiet,
            SliceSelection::for_test(2, false, &[])
        ));
        let TableInput::SharedComplete(rows) = merged(&quiet) else {
            panic!("quiet merged")
        };
        assert_eq!(**rows, research_rows(&[(1, 0, 12, 1), (3, 0, 31, 1)]));
        assert_eq!(reads(&quiet), [(1, 1), (3, 1)]);
        // A failed walk publishes its failure and keeps the earlier rows.
        let mut failed = sliced_input(1, 3, unavailable("research acquisition panicked"));
        let selection = SliceSelection::for_test(3, false, &[]);
        assert!(
            !campaign
                .merge_sliced_inputs(&mut failed, |_| Some(&selection), |_| false)
                .0
        );
        assert!(matches!(
            merged(&failed),
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
        // A truncated slice keeps everyone else and publishes what it read.
        let mut partial = sliced_input(
            1,
            27,
            TableView::Truncated {
                rows: research_rows(&[(3, 0, 33, 27)]),
                missing: 1,
            },
        );
        // Owner 1 is selected early but the truncated walk never reached it:
        // it keeps its earlier row and sample hour instead of disappearing.
        assert!(!merge_with(
            &mut campaign,
            &mut partial,
            SliceSelection::for_test(27, false, &[1])
        ));
        let TableInput::Sampled(TableView::Truncated { rows, missing: 1 }) = merged(&partial)
        else {
            panic!("truncated slice merged")
        };
        assert_eq!(*rows, research_rows(&[(1, 0, 12, 1), (3, 0, 33, 27)]));
        // Only the returned owner is read at this hour in the truncated view.
        assert_eq!(reads(&partial), [(1, 1), (3, 27)]);
        // The truncated read did not replace the base for later slices.
        let mut later = sliced_input(1, 28, TableView::Complete(research_rows(&[])));
        assert!(!merge_with(
            &mut campaign,
            &mut later,
            SliceSelection::for_test(28, false, &[])
        ));
        let TableInput::SharedComplete(rows) = merged(&later) else {
            panic!("later merged")
        };
        assert_eq!(**rows, research_rows(&[(1, 0, 12, 1), (3, 0, 31, 1)]));
        // The truncated read did not replace the base hours either.
        assert_eq!(reads(&later), [(1, 1), (3, 1)]);
        // A new branch starts without earlier rows.
        // A complete census that no longer sees owner 3 drops its rows now,
        // not at its next slice; owner 1 was seen and keeps its row.
        let mut vanished = sliced_input(1, 30, TableView::Complete(research_rows(&[])));
        assert!(!merge_with(
            &mut campaign,
            &mut vanished,
            SliceSelection::for_test(30, false, &[]).with_census(&[1])
        ));
        let TableInput::SharedComplete(rows) = merged(&vanished) else {
            panic!("census merged")
        };
        assert_eq!(**rows, research_rows(&[(1, 0, 12, 1)]));
        // A new branch starts without earlier rows.
        let mut other = sliced_input(2, 29, TableView::Complete(research_rows(&[])));
        assert!(merge_with(
            &mut campaign,
            &mut other,
            SliceSelection::for_test(29, false, &[])
        ));
    }

    /// A walker that reads only some census owners (the building overrides
    /// visit controllers only) stamps only the owners it read, including one
    /// read with no rows; a census owner it never walked gets no read hour.
    #[test]
    fn a_tracking_walker_stamps_only_the_owners_it_read() {
        use crate::slicing::SliceSelection;
        let (mut campaign, _) = campaign();
        let mut full = sliced_input(1, 0, TableView::Complete(research_rows(&[(2, 0, 20, 0)])));
        let mut selection = SliceSelection::for_test(0, true, &[]).with_census(&[2, 3, 26]);
        selection.track_reads();
        // Owner 2 returned a row, 26 was read with none; 3 was never walked.
        selection.mark_read(2);
        selection.mark_read(26);
        assert_eq!(selection.read_owners().collect::<Vec<_>>(), [2, 26]);
        merge_with(&mut campaign, &mut full, selection);
        assert_eq!(reads(&full), [(2, 0), (26, 0)]);
        // Without tracking every covered census owner counts as read.
        let untracked = SliceSelection::for_test(0, true, &[]).with_census(&[2, 3, 26]);
        assert_eq!(untracked.read_owners().collect::<Vec<_>>(), [2, 3, 26]);
    }

    /// An owner a complete read covered that returned no rows still has its
    /// read hour, kept while it lives and not read again; without a scan the
    /// spool cannot know it was read, so it has none.
    #[test]
    fn owners_read_without_rows_keep_their_read_hour() {
        use crate::slicing::SliceSelection;
        let (mut campaign, _) = campaign();
        let mut full = sliced_input(1, 0, TableView::Complete(research_rows(&[(2, 0, 20, 0)])));
        merge_with(
            &mut campaign,
            &mut full,
            SliceSelection::for_test(0, true, &[]).with_census(&[2, 26]),
        );
        assert_eq!(reads(&full), [(2, 0), (26, 0)]);
        // Hour 2 reads slice 2 (owners 2 and 26); 26 again has no rows.
        let mut slice = sliced_input(1, 2, TableView::Complete(research_rows(&[(2, 0, 21, 2)])));
        merge_with(
            &mut campaign,
            &mut slice,
            SliceSelection::for_test(2, false, &[]).with_census(&[2, 26]),
        );
        assert_eq!(reads(&slice), [(2, 2), (26, 2)]);
        // Hour 3 reads neither; both keep their hours.
        let mut quiet = sliced_input(1, 3, TableView::Complete(research_rows(&[])));
        merge_with(
            &mut campaign,
            &mut quiet,
            SliceSelection::for_test(3, false, &[]).with_census(&[2, 26]),
        );
        assert_eq!(reads(&quiet), [(2, 2), (26, 2)]);
        // A census without 26 drops its read.
        let mut gone = sliced_input(1, 4, TableView::Complete(research_rows(&[])));
        merge_with(
            &mut campaign,
            &mut gone,
            SliceSelection::for_test(4, false, &[]).with_census(&[2]),
        );
        assert_eq!(reads(&gone), [(2, 2)]);
        // An early read of 26 without a scan does not know it returned no rows.
        let mut unscanned = sliced_input(1, 27, TableView::Complete(research_rows(&[])));
        merge_with(
            &mut campaign,
            &mut unscanned,
            SliceSelection::for_test(27, false, &[26]),
        );
        assert_eq!(reads(&unscanned), [(2, 2)]);
    }

    #[test]
    fn a_complete_definitions_read_drops_removed_occurrences_and_an_empty_one_keeps_all() {
        use crate::slicing::SliceSelection;
        let definitions = |handles: &[u32]| {
            TableRows::TechnologyDefinitions(
                handles
                    .iter()
                    .map(
                        |&technology_handle| replay_data::rows::TechnologyDefinitionRow {
                            technology_handle,
                            database_index: technology_handle as i32,
                            ..Default::default()
                        },
                    )
                    .collect(),
            )
        };
        let (mut campaign, _) = campaign();
        let mut frame = |hour: i32, rows: TableRows, full: bool| {
            let mut input: FrameInput = staggered_frame(1, hour, false).into();
            input.tables[TableId::TechnologyDefinitions.index()] =
                TableInput::Sampled(TableView::Complete(rows));
            assert!(!merge_with(
                &mut campaign,
                &mut input,
                SliceSelection::for_test(hour, full, &[])
            ));
            let TableInput::SharedComplete(rows) =
                &input.tables[TableId::TechnologyDefinitions.index()]
            else {
                panic!("definitions merged")
            };
            (**rows).clone()
        };
        assert_eq!(frame(0, definitions(&[1, 3]), true), definitions(&[1, 3]));
        // A reload removed template 3: the next complete read replaces the table.
        assert_eq!(frame(1, definitions(&[1]), false), definitions(&[1]));
        // An hour whose slice reached no roster keeps every definition.
        assert_eq!(frame(2, definitions(&[]), false), definitions(&[1]));
    }

    #[test]
    fn periodic_keyframes_re_encode_held_sliced_rows_and_merging_continues_across_them() {
        use crate::slicing::SliceSelection;
        use replay_frame::frame::FrameDecoder;
        let (mut campaign, store) = campaign();
        let owners = 1..=48_u32;
        let slice = |hour: i32, full: bool| {
            research_rows(
                &owners
                    .clone()
                    .filter(|&owner| {
                        full || replay_data::cadence::slice_of(owner) == hour.rem_euclid(24) as u32
                    })
                    .map(|owner| (owner, 0, i64::from(hour), hour))
                    .collect::<Vec<_>>(),
            )
        };
        let mut expected = BTreeMap::new();
        let mut held: BTreeMap<u32, (i64, i32)> = BTreeMap::new();
        let mut allocation = None;
        for hour in 0..=745 {
            let branch = campaign.observe(None, hour, Some(1)).unwrap();
            let full = hour == 0;
            let fresh = slice(hour, full);
            for owner in replay_data::sliced::owners(&fresh) {
                held.insert(owner, (i64::from(hour), hour));
            }
            let mut input = sliced_input(branch, hour, TableView::Complete(fresh));
            assert!(!merge_with(
                &mut campaign,
                &mut input,
                SliceSelection::for_test(hour, full, &[])
            ));
            // Between keyframes the read updates the rows the encoder shares
            // with the cache; the first hour and the keyframe carry them whole.
            assert_eq!(
                matches!(merged(&input), TableInput::SliceUpdate { .. }),
                hour != 0 && hour != 720,
                "hour {hour}"
            );
            let want = research_rows(
                &held
                    .iter()
                    .map(|(&owner, &(progress, read))| (owner, 0, progress, read))
                    .collect::<Vec<_>>(),
            );
            expected.insert(hour, want);
            campaign
                .write(
                    input,
                    1000 + i64::from(hour),
                    Some(1),
                    0,
                    &mut WorkSample::default(),
                )
                .unwrap();
            let cached = campaign.sliced_cache.get(&TableId::ResearchInputs).unwrap();
            let shared = campaign
                .encoder
                .shared_baseline(TableId::ResearchInputs)
                .unwrap();
            assert!(Arc::ptr_eq(cached, shared), "hour {hour}");
            // Equal row counts replace the held rows in place: one allocation
            // from each whole table to the next.
            let current = Arc::as_ptr(cached);
            if hour != 0 && hour != 720 {
                assert_eq!(allocation, Some(current), "hour {hour}");
            }
            allocation = Some(current);
            // Each commit hands back what it no longer holds.
            assert_eq!(campaign.retired.len(), hour as usize + 1);
        }
        campaign.spool.finish_segment().unwrap();
        let frames: Vec<_> = store
            .borrow()
            .segments
            .values()
            .flat_map(|bytes| recover_prefix(bytes).records)
            .filter_map(|record| match record {
                Record::Frame(frame) => Some(frame),
                _ => None,
            })
            .collect();
        assert_eq!(frames.len(), 746);
        let mut decoder = FrameDecoder::new();
        let mut keyframes = Vec::new();
        for frame in &frames {
            let decoded = decoder.advance(frame).unwrap();
            let rows = &*decoded.tables[TableId::ResearchInputs.index()];
            assert_eq!(
                rows,
                &TableView::Complete(expected[&decoded.hour].clone()),
                "hour {}",
                decoded.hour
            );
            let TableView::Complete(TableRows::SliceReads(reads)) =
                &*decoded.tables[TableId::SliceReads.index()]
            else {
                panic!("slice reads decoded")
            };
            // Each owner was read at its own slot, or at the full hour 0.
            for row in reads {
                let slot = replay_data::cadence::slice_of(row.owner) as i32;
                let expected_read = (0..=decoded.hour)
                    .rev()
                    .find(|hour| hour % 24 == slot)
                    .unwrap_or(0);
                assert_eq!(row.sample_hour, expected_read, "owner {}", row.owner);
            }
            assert_eq!(reads.len(), 48);
            if frame.keyframe {
                keyframes.push(decoded.hour);
                // A keyframe alone decodes the whole held table.
                let alone = FrameDecoder::new().advance(frame).unwrap().tables
                    [TableId::ResearchInputs.index()]
                .clone();
                assert_eq!(*alone, TableView::Complete(expected[&decoded.hour].clone()));
            }
        }
        assert_eq!(keyframes, [0, 720]);
    }

    fn frames_of(store: &Rc<RefCell<Store>>) -> Vec<replay_frame::frame::Frame> {
        store
            .borrow()
            .segments
            .values()
            .flat_map(|bytes| recover_prefix(bytes).records)
            .filter_map(|record| match record {
                Record::Frame(frame) => Some(frame),
                _ => None,
            })
            .collect()
    }

    /// A slice update whose write fails leaves the cache with the merged rows
    /// of that hour, as a merged copy would have been cached, and the next
    /// hours merge into them.
    #[test]
    fn a_failed_slice_update_write_caches_the_merged_rows_of_its_hour() {
        use crate::slicing::SliceSelection;
        use replay_frame::{frame::FrameDecoder, frame::TableUpdate};
        for fail_encoding in [false, true] {
            let (mut campaign, store) = campaign();
            let write = |campaign: &mut Campaign<MemoryBackend>,
                         hour: i32,
                         rows: &[(u32, u32, i64, i32)],
                         full: bool,
                         fail: bool| {
                let branch = campaign.observe(None, hour, Some(1)).unwrap();
                let mut input =
                    sliced_input(branch, hour, TableView::Complete(research_rows(rows)));
                let early: Vec<u32> = rows.iter().map(|row| row.0).collect();
                assert!(!merge_with(
                    campaign,
                    &mut input,
                    SliceSelection::for_test(hour, full, &early)
                ));
                assert_eq!(
                    matches!(merged(&input), TableInput::SliceUpdate { .. }),
                    !full && campaign.spool.is_open(),
                    "hour {hour}"
                );
                if fail && fail_encoding {
                    input.tables[TableId::Clock.index()] =
                        TableView::Complete(TableRows::Clock(Vec::new())).into();
                }
                store.borrow_mut().fail_frame_flush = fail && !fail_encoding;
                let result = campaign.write(
                    input,
                    1000 + i64::from(hour),
                    Some(1),
                    0,
                    &mut WorkSample::default(),
                );
                store.borrow_mut().fail_frame_flush = false;
                assert_eq!(result.is_err(), fail, "hour {hour}");
            };
            write(
                &mut campaign,
                0,
                &[(1, 0, 10, 0), (2, 0, 20, 0), (3, 0, 30, 0)],
                true,
                false,
            );
            // Hour 1 updates owner 1 and its write fails.
            write(&mut campaign, 1, &[(1, 0, 11, 1)], false, true);
            assert!(campaign.pending_slices.is_empty());
            assert_eq!(
                **campaign.sliced_cache.get(&TableId::ResearchInputs).unwrap(),
                research_rows(&[(1, 0, 11, 1), (2, 0, 20, 0), (3, 0, 30, 0)])
            );
            // Hour 2 restarts the spool with a keyframe of the merged rows.
            write(&mut campaign, 2, &[(2, 0, 22, 2)], false, false);
            let last = frames_of(&store).pop().unwrap();
            assert!(last.keyframe);
            assert_eq!(
                last.tables[TableId::ResearchInputs.index()],
                TableUpdate::Snapshot(research_rows(&[
                    (1, 0, 11, 1),
                    (2, 0, 22, 2),
                    (3, 0, 30, 0)
                ]))
            );
            // Hour 3 updates the rows the keyframe committed.
            write(&mut campaign, 3, &[(3, 0, 33, 3)], false, false);
            let mut decoder = FrameDecoder::new();
            let mut decoded = None;
            for frame in frames_of(&store).iter().filter(|frame| frame.hour >= 2) {
                decoded = Some(decoder.advance(frame).unwrap().clone());
            }
            assert_eq!(
                *decoded.unwrap().tables[TableId::ResearchInputs.index()],
                TableView::Complete(research_rows(&[
                    (1, 0, 11, 1),
                    (2, 0, 22, 2),
                    (3, 0, 33, 3)
                ]))
            );
        }
    }

    fn staggered_frame(branch: BranchId, hour: i32, skip: bool) -> DecodedFrame {
        use replay_data::{
            rows::CampaignRow, rows::ClockRow, rows::SunkShipsRow, rows::TableId, rows::TableRows,
            rows::WorldTensionRow,
        };
        let mut tables =
            std::array::from_fn(|index| TableView::Complete(TableRows::empty(TableId::ALL[index])));
        tables[TableId::Clock.index()] = TableView::Complete(TableRows::Clock(vec![ClockRow {
            hour,
            ..Default::default()
        }]));
        tables[TableId::Campaign.index()] =
            TableView::Complete(TableRows::Campaign(vec![CampaignRow {
                id: b"test".to_vec(),
            }]));
        tables[TableId::SunkShips.index()] =
            TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow::default()]));
        tables[TableId::WorldTension.index()] =
            TableView::Complete(TableRows::WorldTension(vec![WorldTensionRow::default()]));
        tables[TableId::LocalPlayer.index()] =
            TableView::Complete(TableRows::LocalPlayer(vec![Default::default()]));
        tables[TableId::SimulationFeatures.index()] =
            TableView::Complete(TableRows::SimulationFeatures(vec![
                replay_data::rows::SimulationFeatureRow::default(),
            ]));
        tables[TableId::CombatLogState.index()] =
            TableView::Complete(TableRows::CombatLogState(vec![
                replay_data::rows::CombatLogStateRow::default(),
            ]));
        tables[TableId::AirSortieModifierMetadata.index()] =
            TableView::Complete(TableRows::AirSortieModifierMetadata(vec![
                replay_data::rows::AirSortieModifierMetadataRow::default(),
            ]));
        tables[TableId::ReservedCountryNaming.index()] =
            TableView::Complete(TableRows::ReservedCountryNaming(vec![
                replay_data::rows::ReservedCountryNamingRow::default(),
            ]));
        tables[TableId::MapEvents.index()] =
            TableView::Complete(TableRows::MapEvents(vec![Default::default()]));
        tables[TableId::DesignObserverContext.index()] =
            TableView::Complete(TableRows::DesignObserverContext(vec![Default::default()]));
        tables[TableId::GameScoreMultipliers.index()] = TableView::Unavailable {
            reason: "score multipliers outside staggered fixture".into(),
        };
        tables[TableId::EventFeedHistory.index()] = TableView::Unavailable {
            reason: "Event feed outside staggered fixture".into(),
        };
        if skip {
            for table in TableId::ALL {
                if replay_data::cadence::is_staggered(table) {
                    tables[table.index()] = TableView::Unavailable {
                        reason: replay_data::cadence::SKIPPED_REASON.into(),
                    };
                }
            }
        }
        // A retired table decodes as its catalogued refusal.
        for table in TableId::ALL {
            if let Some(reason) = super::retired_reason(table) {
                tables[table.index()] = TableView::Unavailable {
                    reason: reason.into(),
                };
            }
        }
        DecodedFrame {
            branch,
            hour,
            tables: tables.map(Into::into),
            metrics: Metrics::default(),
        }
    }

    /// A staggered (daily) table.
    const DAILY: TableId = TableId::AirAces;

    /// Rows of [`DAILY`].
    fn daily_rows(value: u8) -> TableRows {
        assert!(replay_data::cadence::is_staggered(DAILY));
        TableRows::AirAces(vec![replay_data::rows::AirAceRow {
            id: 7,
            name: vec![0, 0xff, value],
            ..Default::default()
        }])
    }

    fn reserved_intel_rows(army: i64) -> TableRows {
        TableRows::CountryIntel(vec![
            replay_data::rows::CountryIntelRow {
                owner_country: 0,
                target_index: 0,
                army,
                ..Default::default()
            },
            replay_data::rows::CountryIntelRow {
                owner_country: 0,
                target_index: 2,
                ..Default::default()
            },
        ])
    }

    fn reserved_intel_sample(
        source: &FrameInput,
    ) -> Option<replay_data::rows::ReservedCountryIntelSample> {
        let TableInput::Sampled(TableView::Complete(TableRows::ReservedCountryNaming(rows))) =
            &source.tables[TableId::ReservedCountryNaming.index()]
        else {
            panic!("complete RCN fixture")
        };
        let replay_data::rows::Recorded::Value(sample) = rows[0].country_intel_sample else {
            panic!("assembled field")
        };
        sample
    }

    #[test]
    fn reserved_intel_certificate_carries_advances_equal_rows_and_clears_fresh_refusal() {
        use replay_data::rows::ReservedCountryIntelSample;
        let (mut campaign, _) = campaign();
        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let mut first: FrameInput = staggered_frame(branch, 0, false).into();
        first.tables[TableId::CountryIntel.index()] =
            TableView::Complete(reserved_intel_rows(17)).into();
        let initial = ReservedCountryIntelSample {
            sample_hour: 0,
            vector_count: 3,
        };
        campaign
            .try_apply_staggered_inputs(&mut first, Some(Ok(initial)))
            .unwrap();
        campaign.assemble_reserved_intel_sample(&mut first);
        assert_eq!(reserved_intel_sample(&first), Some(initial));

        let mut carried: FrameInput = staggered_frame(branch, 1, true).into();
        campaign
            .try_apply_staggered_inputs(&mut carried, None)
            .unwrap();
        campaign.assemble_reserved_intel_sample(&mut carried);
        assert_eq!(reserved_intel_sample(&carried), Some(initial));

        let mut equal: FrameInput = staggered_frame(branch, 2, false).into();
        equal.tables[TableId::CountryIntel.index()] =
            TableView::Complete(reserved_intel_rows(17)).into();
        let latest = ReservedCountryIntelSample {
            sample_hour: 2,
            vector_count: 3,
        };
        campaign
            .try_apply_staggered_inputs(&mut equal, Some(Ok(latest)))
            .unwrap();
        campaign.assemble_reserved_intel_sample(&mut equal);
        assert_eq!(reserved_intel_sample(&equal), Some(latest));

        let mut refused: FrameInput = staggered_frame(branch, 3, false).into();
        refused.tables[TableId::CountryIntel.index()] =
            TableView::Complete(TableRows::CountryIntel(vec![
                replay_data::rows::CountryIntelRow {
                    owner_country: 1,
                    target_index: 2,
                    ..Default::default()
                },
            ]))
            .into();
        campaign
            .try_apply_staggered_inputs(&mut refused, Some(Err("reserved body fault")))
            .unwrap();
        campaign.assemble_reserved_intel_sample(&mut refused);
        assert_eq!(reserved_intel_sample(&refused), None);
        assert!(matches!(
            refused.tables[TableId::CountryIntel.index()],
            TableInput::SharedComplete(_)
        ));
        let mut after: FrameInput = staggered_frame(branch, 4, true).into();
        campaign
            .try_apply_staggered_inputs(&mut after, None)
            .unwrap();
        campaign.assemble_reserved_intel_sample(&mut after);
        assert_eq!(reserved_intel_sample(&after), None);
    }

    #[test]
    fn reserved_intel_empty_certificate_prefix_refusal_and_branch_reset_are_distinct() {
        use replay_data::rows::ReservedCountryIntelSample;
        let (mut campaign, _) = campaign();
        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let empty = ReservedCountryIntelSample {
            sample_hour: 0,
            vector_count: 0,
        };
        let mut first: FrameInput = staggered_frame(branch, 0, false).into();
        campaign
            .try_apply_staggered_inputs(&mut first, Some(Ok(empty)))
            .unwrap();
        first.tables[TableId::ReservedCountryNaming.index()] = unavailable("naming failed").into();
        campaign.assemble_reserved_intel_sample(&mut first);
        assert!(
            matches!(&first.tables[TableId::ReservedCountryNaming.index()],
            TableInput::Sampled(TableView::Unavailable { reason }) if reason == "naming failed")
        );
        assert_eq!(campaign.country_intel_sample, Some(empty));
        let mut next: FrameInput = staggered_frame(branch, 1, true).into();
        campaign
            .try_apply_staggered_inputs(&mut next, None)
            .unwrap();
        campaign.assemble_reserved_intel_sample(&mut next);
        assert_eq!(reserved_intel_sample(&next), Some(empty));
        let other = branch + 1;
        let mut reset: FrameInput = staggered_frame(other, 2, true).into();
        campaign
            .try_apply_staggered_inputs(&mut reset, None)
            .unwrap();
        campaign.assemble_reserved_intel_sample(&mut reset);
        assert_eq!(reserved_intel_sample(&reset), None);
        assert_eq!(campaign.country_intel_sample, None);
    }

    #[test]
    fn reserved_intel_noncomplete_sample_clears_previous_certificate() {
        for failed in [
            TableView::Truncated {
                rows: TableRows::CountryIntel(Vec::new()),
                missing: 1,
            },
            unavailable("positive intel table failed"),
        ] {
            let (mut campaign, _) = campaign();
            let branch = campaign.observe(None, 0, Some(1)).unwrap();
            let mut first: FrameInput = staggered_frame(branch, 0, false).into();
            let sample = replay_data::rows::ReservedCountryIntelSample {
                sample_hour: 0,
                vector_count: 0,
            };
            campaign
                .try_apply_staggered_inputs(&mut first, Some(Ok(sample)))
                .unwrap();
            let mut refused: FrameInput = staggered_frame(branch, 1, false).into();
            refused.tables[TableId::CountryIntel.index()] = failed.into();
            campaign
                .try_apply_staggered_inputs(&mut refused, Some(Err("positive prerequisite failed")))
                .unwrap();
            campaign.assemble_reserved_intel_sample(&mut refused);
            assert_eq!(reserved_intel_sample(&refused), None);
        }
    }

    #[test]
    fn reserved_intel_latest_pair_survives_encoding_or_write_failure_recovery() {
        use replay_data::rows::ReservedCountryIntelSample;
        use replay_frame::{frame::FrameDecoder, frame::TableUpdate};
        for fail_encoding in [false, true] {
            let (mut campaign, store) = campaign();
            let branch = campaign.observe(None, 0, Some(1)).unwrap();
            let mut first: FrameInput = staggered_frame(branch, 0, false).into();
            first.tables[TableId::CountryIntel.index()] =
                TableView::Complete(reserved_intel_rows(17)).into();
            campaign
                .try_apply_staggered_inputs(
                    &mut first,
                    Some(Ok(ReservedCountryIntelSample {
                        sample_hour: 0,
                        vector_count: 3,
                    })),
                )
                .unwrap();
            campaign.assemble_reserved_intel_sample(&mut first);
            campaign
                .write(first, 1000, Some(1), 0, &mut WorkSample::default())
                .unwrap();
            campaign.observe(None, 1, Some(1)).unwrap();
            let mut failed: FrameInput = staggered_frame(branch, 1, false).into();
            failed.tables[TableId::CountryIntel.index()] =
                TableView::Complete(reserved_intel_rows(-29)).into();
            let latest = ReservedCountryIntelSample {
                sample_hour: 1,
                vector_count: 3,
            };
            campaign
                .try_apply_staggered_inputs(&mut failed, Some(Ok(latest)))
                .unwrap();
            campaign.assemble_reserved_intel_sample(&mut failed);
            if fail_encoding {
                failed.tables[TableId::Clock.index()] =
                    TableView::Complete(TableRows::Clock(Vec::new())).into();
            } else {
                store.borrow_mut().fail_frame_flush = true;
            }
            assert!(
                campaign
                    .write(failed, 1001, Some(1), 0, &mut WorkSample::default())
                    .is_err()
            );
            assert_eq!(campaign.country_intel_sample, Some(latest));
            campaign.observe(None, 2, Some(1)).unwrap();
            let mut restored: FrameInput = staggered_frame(branch, 2, true).into();
            campaign
                .try_apply_staggered_inputs(&mut restored, None)
                .unwrap();
            campaign.assemble_reserved_intel_sample(&mut restored);
            assert_eq!(reserved_intel_sample(&restored), Some(latest));
            campaign
                .write(restored, 1002, Some(1), 0, &mut WorkSample::default())
                .unwrap();
            let recovered = recover_prefix(store.borrow().segments.get(&1).unwrap());
            let frame = recovered
                .records
                .iter()
                .find_map(|record| match record {
                    Record::Frame(frame) => Some(frame),
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                frame.tables[TableId::CountryIntel.index()],
                TableUpdate::Snapshot(reserved_intel_rows(-29))
            );
            let decoded = FrameDecoder::new().apply(frame).unwrap();
            let rcn = decoded
                .complete::<replay_data::rows::tables::ReservedCountryNaming>()
                .unwrap();
            assert_eq!(
                rcn[0].country_intel_sample,
                replay_data::rows::Recorded::Value(Some(latest))
            );
        }
    }

    #[test]
    fn fixed_layout_refusal_survives_daily_carry_and_independent_keyframes() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("modifier-name tests require BATTLE_PLAN_GAME_DIR with the supported installation");
        use replay_frame::frame::FrameDecoder;
        let (mut campaign, _) = campaign();
        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let key = engine_executable::modifier_keys::POWER_BALANCE_DAILY;
        let mut report = engine_fixed_layout::fixed_layout::Report {
            static_token_maximum: 0x6000,
            gap: Ok(()),
            modifiers: [Ok(()); engine_fixed_layout::fixed_layout::GAP as usize],
            tokens: [Ok(()); engine_fixed_layout::fixed_layout::TOKEN_IDS.len()],
        };
        report.modifiers[key as usize] = Err("definition token spells a different script name");
        let mut decoder = FrameDecoder::new();
        for (hour, skip, force_keyframe) in [(0, false, true), (1, true, false), (2, true, true)] {
            let mut source: FrameInput = staggered_frame(branch, hour, skip).into();
            let retry_before = needs_retry(&source.tables);
            campaign.apply_staggered_inputs(&mut source);
            crate::fixed_layout::apply(&mut source.tables, &Ok(report)).unwrap();
            // Permanent refusals run after the recovery decision; a successful
            // static/daily capture does not trigger repeated full acquisition.
            assert!(!retry_before);
            let frame = campaign
                .encoder
                .encode_input(source, force_keyframe)
                .unwrap();
            let decoded = decoder.apply(&frame).unwrap();
            for &table in replay_derive::fixed_layout::POWER_BALANCE.tables {
                assert!(
                    matches!(&*decoded.tables[table.index()], TableView::Unavailable { reason } if reason.contains("power_balance_daily"))
                );
                if force_keyframe {
                    let alone = FrameDecoder::new().apply(&frame).unwrap();
                    assert_eq!(alone.tables[table.index()], decoded.tables[table.index()]);
                }
            }
            assert!(decoded.complete::<tables::Countries>().is_ok());
        }
    }

    /// The hour's drained ticks reach the branch totals whether or not the
    /// lines' slot is read; the table is published with the census, carried
    /// off its slot, and round-trips through the encoder.
    #[test]
    fn line_output_totals_publish_with_the_census_and_carry_off_its_slot() {
        use crate::line_output::{Drained, Entry};
        use replay_data::{rows::ProductionLineRow, rows::TableRows};
        use replay_frame::frame::FrameDecoder;
        const LINE: u64 = 0x10_0000_0021;
        const VARIANT: u64 = 0x5_0000_0046;
        let (mut campaign, _) = campaign();
        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let mut captured = Captured::new();
        let mut decoder = FrameDecoder::new();
        let mut published = Vec::new();
        for (hour, first, ticks, skip) in [(0, 10, 1, false), (1, 11, 2, true), (2, 13, 0, false)] {
            let entries = vec![
                Entry {
                    line: LINE,
                    country: 5001,
                    variant: VARIANT,
                    mio: 0,
                    kind: crate::line_output::OutputKind::Equipment,
                    units: 2,
                    credit: 2,
                    spending: None,
                    exact: true,
                    purpose: replay_data::rows::OutputPurpose::New,
                };
                ticks
            ];
            captured.extended.line_output_mut().set_drained_for_test(
                Ok(Drained {
                    epoch: 1,
                    first,
                    cursor: first + ticks as u64,
                    copied: ticks,
                    pending: ticks,
                }),
                &entries,
            );
            let mut source: FrameInput = staggered_frame(branch, hour, skip).into();
            if !skip {
                source.tables[TableId::ProductionLines.index()] =
                    TableInput::Sampled(TableView::Complete(TableRows::ProductionLines(vec![
                        ProductionLineRow {
                            handle: LINE,
                            country: 5001,
                            variant: VARIANT,
                            ..Default::default()
                        },
                    ])));
            }
            campaign.assemble_line_output(&mut source, &captured);
            campaign.apply_staggered_inputs(&mut source);
            let frame = campaign.encoder.encode_input(source, hour == 0).unwrap();
            let decoded = decoder.apply(&frame).unwrap();
            let rows = decoded.complete::<tables::ProductionLineOutput>().unwrap();
            assert_eq!(rows[0].country, 5001);
            published.push((rows[0].units, rows[0].mio_credit, rows[0].since_hour));
        }
        // The first frame's ticks came before the observation began; hour 1
        // carries hour 0's publication; hour 2 publishes hour 1's two ticks.
        let retired = replay_data::rows::RetiredI64::default();
        assert_eq!(
            published,
            [(0, retired, 0), (0, retired, 0), (4, retired, 0)]
        );
    }

    /// Each hour's drained construction output reaches the branch totals whether
    /// or not the queues' slot is read; the table is published with the queues'
    /// census, carried off its slot, and round-trips through the encoder.
    #[test]
    fn construction_output_publishes_with_the_queues_and_carries_off_their_slot() {
        use crate::construction_ic::{Drained, Entry};
        use replay_data::{
            rows::BuildingConstructionRow, rows::RailwayConstructionRow, rows::TableRows,
        };
        use replay_frame::frame::FrameDecoder;
        const LINE: u64 = 0x10_0000_003b;
        const RAIL: u64 = 0x11_0000_1269;
        let (mut campaign, _) = campaign();
        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let mut captured = Captured::new();
        let mut decoder = FrameDecoder::new();
        let mut published = Vec::new();
        for (hour, first, days, skip) in [(0, 10, 1, false), (1, 11, 2, true), (2, 13, 0, false)] {
            let entries: Vec<Entry> = (0..days)
                .map(|day| Entry {
                    line: if day == 0 { LINE } else { RAIL },
                    amount: 150_000,
                    bank: 20_000,
                    railway: day != 0,
                    ..Entry::default()
                })
                .collect();
            captured
                .extended
                .construction_ic_mut()
                .set_drained_for_test(
                    Ok(Drained {
                        epoch: 1,
                        first,
                        cursor: first + days as u64,
                        copied: days,
                        pending: days,
                    }),
                    &entries,
                );
            let mut source: FrameInput = staggered_frame(branch, hour, skip).into();
            if !skip {
                source.tables[TableId::BuildingConstruction.index()] =
                    TableInput::Sampled(TableView::Complete(TableRows::BuildingConstruction(
                        vec![BuildingConstructionRow {
                            id: LINE,
                            country: 5001,
                            ..Default::default()
                        }],
                    )));
                source.tables[TableId::RailwayConstruction.index()] =
                    TableInput::Sampled(TableView::Complete(TableRows::RailwayConstruction(vec![
                        RailwayConstructionRow {
                            id: RAIL,
                            country: 5001,
                            ..Default::default()
                        },
                    ])));
            }
            campaign.assemble_construction_ic(&mut source, &captured);
            campaign.apply_staggered_inputs(&mut source);
            let frame = campaign.encoder.encode_input(source, hour == 0).unwrap();
            let decoded = decoder.apply(&frame).unwrap();
            let rows = decoded
                .complete::<tables::ConstructionLineOutput>()
                .unwrap();
            published.push(
                rows.iter()
                    .map(|row| (row.line, row.railway, row.construction_raw, row.bank_raw))
                    .collect::<Vec<_>>(),
            );
        }
        // The first frame's output came before the observation began; hour 1
        // carries hour 0's publication; hour 2 publishes hour 1's two days.
        let zero = vec![(LINE, false, 0, 0), (RAIL, true, 0, 0)];
        assert_eq!(
            published,
            [
                zero.clone(),
                zero,
                vec![
                    (LINE, false, 130_000, 20_000),
                    (RAIL, true, 130_000, 20_000)
                ]
            ]
        );
    }

    /// A line that ends in a publication whose write fails keeps its final
    /// totals: the next written frame, a census at the next hour, repeats it.
    #[test]
    fn a_failed_write_keeps_the_final_totals_of_ended_lines() {
        use crate::line_output::{Drained, Entry};
        use replay_data::{rows::ProductionLineRow, rows::TableRows};
        use replay_frame::frame::FrameDecoder;
        const LINE_A: u64 = 0x10_0000_0021;
        const LINE_B: u64 = 0x11_0000_0021;
        const VARIANT: u64 = 0x5_0000_0046;
        let (mut campaign, store) = campaign();
        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let mut captured = Captured::new();
        for (hour, first, lines, ticks, fails) in [
            (0, 10, &[LINE_A, LINE_B][..], 0, false),
            (1, 10, &[LINE_A][..], 1, true),
            (2, 11, &[LINE_A][..], 0, false),
        ] {
            if hour != 0 {
                campaign.observe(None, hour, Some(1)).unwrap();
            }
            let entries = vec![
                Entry {
                    line: LINE_B,
                    country: 5001,
                    variant: VARIANT,
                    mio: 0,
                    kind: crate::line_output::OutputKind::Equipment,
                    units: 1,
                    credit: 1,
                    spending: None,
                    exact: true,
                    purpose: replay_data::rows::OutputPurpose::New,
                };
                ticks
            ];
            captured.extended.line_output_mut().set_drained_for_test(
                Ok(Drained {
                    epoch: 1,
                    first,
                    cursor: first + ticks as u64,
                    copied: ticks,
                    pending: ticks,
                }),
                &entries,
            );
            let mut source: FrameInput = staggered_frame(branch, hour, false).into();
            source.tables[TableId::ProductionLines.index()] =
                TableInput::Sampled(TableView::Complete(TableRows::ProductionLines(
                    lines
                        .iter()
                        .map(|&handle| ProductionLineRow {
                            handle,
                            country: 5001,
                            variant: VARIANT,
                            ..Default::default()
                        })
                        .collect(),
                )));
            campaign.assemble_line_output(&mut source, &captured);
            campaign.apply_staggered_inputs(&mut source);
            store.borrow_mut().fail_frame_flush = fails;
            let written = campaign.write(
                source,
                1000 + i64::from(hour),
                Some(1),
                0,
                &mut WorkSample::default(),
            );
            assert_eq!(written.is_err(), fails, "hour {hour}");
        }
        // The failure closed the first segment; the next one starts at hour 2.
        let recovered = recover_prefix(store.borrow().segments.get(&1).unwrap());
        let frame = recovered
            .records
            .iter()
            .find_map(|record| match record {
                Record::Frame(frame) => Some(frame),
                _ => None,
            })
            .unwrap();
        let decoded = FrameDecoder::new().apply(frame).unwrap();
        let rows = decoded.complete::<tables::ProductionLineOutput>().unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| (row.line, row.units, row.ended))
                .collect::<Vec<_>>(),
            [(LINE_A, 0, false), (LINE_B, 1, true)]
        );
    }

    fn conversion_reuse_frame(
        campaign: &mut Campaign<MemoryBackend>,
        branch: BranchId,
        hour: i32,
        value: u8,
        table: TableId,
    ) -> FrameInput {
        if table != TableId::LexicalTokens {
            return character_frame(campaign, branch, hour, value);
        }
        let native = crate::extended::lexical::lexical_reuse_fixture(value);
        let mut reuse = LexicalConversion::new(
            lexical_cache(&campaign.staggered_cache, campaign.staggered_branch, branch),
            &mut campaign.lexical_conversion,
        );
        let [input] = native.table_inputs(Some(&mut reuse));
        let mut frame: FrameInput = staggered_frame(branch, hour, false).into();
        frame.tables[table.index()] = input;
        frame
    }

    #[test]
    fn lexical_reuse_cache_eligibility_primary_failures_and_costs_stay_explicit() {
        let table = TableId::LexicalTokens;
        let (mut campaign, _) = campaign();
        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let mut first = conversion_reuse_frame(&mut campaign, branch, 0, 9, table);
        campaign.apply_staggered_inputs(&mut first);
        let cached = Arc::clone(cached_complete(&campaign, table));
        for cached_branch in [None, Some(branch + 1)] {
            assert!(lexical_cache(&campaign.staggered_cache, cached_branch, branch).is_none());
        }
        assert!(Arc::ptr_eq(
            lexical_cache(&campaign.staggered_cache, Some(branch), branch).unwrap(),
            &cached
        ));
        for invalid in [
            CachedTable::Unavailable {
                reason: "fresh lexical failure".into(),
            },
            CachedTable::Truncated {
                rows: (*cached).clone(),
                missing: 1,
            },
        ] {
            campaign.staggered_cache.insert(table, invalid);
            assert!(lexical_cache(&campaign.staggered_cache, Some(branch), branch).is_none());
            let current = conversion_reuse_frame(&mut campaign, branch, 1, 9, table);
            assert!(matches!(
                current.tables[table.index()],
                TableInput::Sampled(TableView::Complete(_))
            ));
        }
        campaign
            .staggered_cache
            .insert(table, CachedTable::Complete(Arc::clone(&cached)));
        let mut current = conversion_reuse_frame(&mut campaign, branch, 1, 9, table);
        assert!(matches!(
            current.tables[table.index()],
            TableInput::SharedComplete(_)
        ));
        current.tables[table.index()] =
            TableInput::Sampled(unavailable("native dictionary unreadable"));
        assert_eq!(
            current.tables[table.index()],
            TableInput::Sampled(unavailable("native dictionary unreadable"))
        );
        campaign.apply_staggered_inputs(&mut current);
        assert!(lexical_cache(&campaign.staggered_cache, Some(branch), branch).is_none());

        let mut source = captured(1);
        source.clock = None;
        let before = campaign.lexical_conversion;
        let mut reuse = LexicalConversion::new(Some(&cached), &mut campaign.lexical_conversion);
        let (invalid, packs) = decoded_with_costs(
            &source,
            clock(1).borrowed(),
            branch,
            &mut SpoolConversion {
                variables_baseline: None,
                variables_scratch: &mut ScriptVariableComparison::default(),
                characters: None,
                lexical: Some(&mut reuse),
                intel: None,
                projection: None,
                helper: None,
            },
        );
        assert_eq!(
            invalid.tables,
            unavailable_tables("clock unreadable at hook entry")
        );
        assert!(packs.iter().all(Option::is_none));
        assert_eq!(campaign.lexical_conversion, before);

        let other = LexicalConversionStats {
            hits: 7,
            misses: 11,
        };
        let status = serde_json::to_value(lexical_conversion_status([
            &campaign.lexical_conversion,
            &other,
        ]))
        .unwrap();
        assert_eq!(status["hits"], before.hits + 7);
        assert_eq!(status["misses"], before.misses + 11);
        assert_eq!(status["additional_native_dictionary_capacity_bytes"], 0);
        assert!(
            status["cost_scope"]
                .as_str()
                .unwrap()
                .contains("conversion_packs lexical")
        );
    }

    fn character_frame(
        campaign: &mut Campaign<MemoryBackend>,
        branch: BranchId,
        hour: i32,
        progress: u8,
    ) -> FrameInput {
        let native = crate::extended::characters::character_shadow_fixture(progress);
        let mut reuse = CharacterConversion::new(
            branch,
            &mut campaign.character_shadow,
            character_cache(&campaign.staggered_cache, campaign.staggered_branch, branch),
        );
        let mut frame: FrameInput = staggered_frame(branch, hour, false).into();
        for (index, input) in native
            .table_inputs(Some(&mut reuse))
            .into_iter()
            .enumerate()
        {
            if matches!(index, 0 | 1 | 2 | 4) {
                frame.tables[crate::extended::characters::TABLES[index].index()] = input;
            }
        }
        frame
    }

    #[test]
    fn character_shadow_status_sums_current_capacity_and_decisions_across_campaigns() {
        let (mut first, _) = campaign();
        let branch = first.observe(None, 0, Some(1)).unwrap();
        let mut source = character_frame(&mut first, branch, 0, 10);
        first.apply_staggered_inputs(&mut source);
        let _equal = character_frame(&mut first, branch, 1, 10);
        let (mut second, _) = campaign();
        let branch = second.observe(None, 0, Some(1)).unwrap();
        let _source = character_frame(&mut second, branch, 0, 10);

        let status = serde_json::to_value(character_shadow_status([
            &first.character_shadow,
            &second.character_shadow,
        ]))
        .unwrap();
        assert_eq!(status["campaigns"], 2);
        let mut bytes = 0usize;
        for (index, (first, second)) in first
            .character_shadow
            .statistics()
            .into_iter()
            .zip(second.character_shadow.statistics())
            .enumerate()
        {
            let table = &status["tables"][index];
            assert_eq!(table["hits"], 1);
            assert_eq!(table["misses"], 2);
            assert_eq!(
                table["row_capacity_bytes"],
                first.row_capacity_bytes + second.row_capacity_bytes
            );
            assert_eq!(
                table["arena_capacity_bytes"],
                first.arena_capacity_bytes + second.arena_capacity_bytes
            );
            bytes += first.row_capacity_bytes
                + second.row_capacity_bytes
                + first.arena_capacity_bytes
                + second.arena_capacity_bytes;
        }
        assert_eq!(status["retained_capacity_bytes"], bytes);
        assert!(
            status["allocation_scope"]
                .as_str()
                .unwrap()
                .contains("separate from raw/pool")
        );
        assert!(
            status["counter_scope"]
                .as_str()
                .unwrap()
                .contains("skipped inputs excluded")
        );
    }

    fn intel_frame(
        campaign: &mut Campaign<MemoryBackend>,
        branch: BranchId,
        hour: i32,
        value: i64,
    ) -> FrameInput {
        let native = crate::extended::intel::intel_shadow_fixture(value);
        let inputs = native.table_inputs(
            None,
            &mut ScriptVariableComparison::default(),
            Some(&mut IntelConversion::new(
                branch,
                &mut campaign.intel_shadow,
            )),
        );
        let mut frame: FrameInput = staggered_frame(branch, hour, false).into();
        frame.tables[TableId::ScriptFlags.index()] = inputs[7].clone();
        frame
    }

    fn intel_allocation(frame: &FrameInput, table: TableId) -> &Arc<TableRows> {
        let TableInput::SharedComplete(rows) = &frame.tables[table.index()] else {
            panic!("expected shared Intel rows")
        };
        rows
    }

    #[test]
    fn intel_shadow_reuses_exact_hourly_rows_through_keyframes_failed_writes_and_new_branches() {
        use replay_frame::frame::TableUpdate;
        for fail_encoding in [false, true] {
            let (mut campaign, store) = campaign();
            let branch = campaign.observe(None, 0, Some(1)).unwrap();
            let first = intel_frame(&mut campaign, branch, 0, 10);
            let original = Arc::clone(intel_allocation(&first, TableId::ScriptFlags));
            let mut reference = FrameEncoder::new();
            let expected_first = reference
                .encode(sampled_frame(first.clone()), false)
                .unwrap();
            campaign
                .write(first, 1000, Some(1), 0, &mut WorkSample::default())
                .unwrap();
            campaign.observe(None, 1, Some(1)).unwrap();
            let second = intel_frame(&mut campaign, branch, 1, 10);
            assert!(Arc::ptr_eq(
                &original,
                intel_allocation(&second, TableId::ScriptFlags)
            ));
            let expected_second = reference
                .encode(sampled_frame(second.clone()), false)
                .unwrap();
            campaign
                .write(second, 1001, Some(1), 0, &mut WorkSample::default())
                .unwrap();
            let recovered = recover_prefix(store.borrow().segments.get(&0).unwrap());
            let frames: Vec<_> = recovered
                .records
                .iter()
                .filter_map(|record| match record {
                    Record::Frame(frame) => Some(frame),
                    _ => None,
                })
                .collect();
            assert_eq!(frames, vec![&expected_first, &expected_second]);

            campaign.observe(None, 2, Some(1)).unwrap();
            let mut failed = intel_frame(&mut campaign, branch, 2, 20);
            let latest = Arc::clone(intel_allocation(&failed, TableId::ScriptFlags));
            assert!(!Arc::ptr_eq(&original, &latest));
            if fail_encoding {
                failed.tables[TableId::Clock.index()] =
                    TableView::Complete(TableRows::Clock(Vec::new())).into();
            } else {
                store.borrow_mut().fail_frame_flush = true;
            }
            assert!(
                campaign
                    .write(failed, 1002, Some(1), 0, &mut WorkSample::default())
                    .is_err()
            );
            assert!(!campaign.spool.is_open());
            campaign.observe(None, 3, Some(1)).unwrap();
            // A freshly acquired exact native input is required even though the
            // memo's immutable result survives the failed write/encoder reset.
            let restored = intel_frame(&mut campaign, branch, 3, 20);
            assert!(Arc::ptr_eq(
                &latest,
                intel_allocation(&restored, TableId::ScriptFlags)
            ));
            campaign
                .write(restored, 1003, Some(1), 0, &mut WorkSample::default())
                .unwrap();
            let recovered = recover_prefix(store.borrow().segments.get(&1).unwrap());
            let frame = recovered
                .records
                .iter()
                .find_map(|record| match record {
                    Record::Frame(frame) => Some(frame),
                    _ => None,
                })
                .unwrap();
            assert!(frame.keyframe);
            assert_eq!(
                frame.tables[TableId::ScriptFlags.index()],
                TableUpdate::Snapshot((*latest).clone())
            );

            let next_branch = campaign.observe(None, 4, Some(2)).unwrap();
            assert_ne!(branch, next_branch);
            let next = intel_frame(&mut campaign, next_branch, 4, 20);
            assert!(!Arc::ptr_eq(
                &latest,
                intel_allocation(&next, TableId::ScriptFlags)
            ));
            campaign
                .write(next, 1004, Some(2), 0, &mut WorkSample::default())
                .unwrap();
        }
    }

    #[test]
    fn intel_shadow_missing_clock_skips_conversion_and_preserves_failure() {
        let table = TableId::ScriptFlags;
        let (mut campaign, _) = campaign();
        let mut source = captured(0);
        source.extended.set_intel_shadow_fixture(10);
        let first = intel_frame(&mut campaign, 1, 0, 10);
        let old = Arc::clone(intel_allocation(&first, table));
        let before = campaign.intel_shadow.statistics();
        source.clock = None;
        let mut reuse = IntelConversion::new(1, &mut campaign.intel_shadow);
        let (current, costs) = decoded_with_costs(
            &source,
            clock(0).borrowed(),
            1,
            &mut SpoolConversion {
                variables_baseline: None,
                variables_scratch: &mut ScriptVariableComparison::default(),
                characters: None,
                lexical: None,
                intel: Some(&mut reuse),
                projection: None,
                helper: None,
            },
        );
        assert_eq!(
            current.tables,
            unavailable_tables("clock unreadable at hook entry")
        );
        let after = campaign.intel_shadow.statistics();
        assert_eq!(
            (after.replay_capacity_bytes, after.hits, after.misses),
            (before.replay_capacity_bytes, before.hits, before.misses)
        );
        assert!(costs.iter().all(Option::is_none));
        let next = intel_frame(&mut campaign, 1, 1, 10);
        assert!(Arc::ptr_eq(&old, intel_allocation(&next, table)));
    }

    #[test]
    fn intel_shadow_status_reports_native_and_shared_replay_storage_separately() {
        let (mut first, _) = campaign();
        let (mut second, _) = campaign();
        intel_frame(&mut first, 1, 0, 10);
        intel_frame(&mut first, 1, 1, 10);
        intel_frame(&mut second, 1, 0, 20);
        let report = serde_json::to_value(intel_shadow_status([
            &first.intel_shadow,
            &second.intel_shadow,
        ]))
        .unwrap();
        assert_eq!(report["campaigns"], 2);
        let (a, b) = (
            first.intel_shadow.statistics(),
            second.intel_shadow.statistics(),
        );
        assert_eq!(
            report["retained_native_capacity_bytes"],
            a.row_capacity_bytes + b.row_capacity_bytes
        );
        assert_eq!(
            report["retained_replay_capacity_bytes"],
            a.replay_capacity_bytes + b.replay_capacity_bytes
        );
        assert_eq!(report["hits"], 1);
        assert_eq!(report["misses"], 2);
    }

    #[test]
    fn character_shadow_reuse_never_overrides_primary_failure_or_missing_clock() {
        for table in crate::extended::characters::SHADOW_TABLES {
            for failure in 0..2 {
                let (mut campaign, _) = campaign();
                let branch = campaign.observe(None, 0, Some(1)).unwrap();
                let mut first = character_frame(&mut campaign, branch, 0, 10);
                campaign.apply_staggered_inputs(&mut first);
                campaign
                    .write(first, 1000, Some(1), 0, &mut WorkSample::default())
                    .unwrap();
                let previous = Arc::downgrade(cached_complete(&campaign, table));
                campaign.observe(None, 1, Some(1)).unwrap();
                let mut current = character_frame(&mut campaign, branch, 1, 10);
                let TableInput::SharedComplete(rows) = &current.tables[table.index()] else {
                    panic!()
                };
                assert!(Arc::ptr_eq(rows, &previous.upgrade().unwrap()));
                match failure {
                    0 => {
                        current.tables[table.index()] =
                            TableInput::Sampled(unavailable("native character unreadable"));
                    }
                    1 => {
                        let mut source = captured(1);
                        source.clock = None;
                        let mut reuse = CharacterConversion::new(
                            branch,
                            &mut campaign.character_shadow,
                            character_cache(
                                &campaign.staggered_cache,
                                campaign.staggered_branch,
                                branch,
                            ),
                        );
                        current = decoded_with_costs(
                            &source,
                            clock(1).borrowed(),
                            branch,
                            &mut SpoolConversion {
                                variables_baseline: None,
                                variables_scratch: &mut ScriptVariableComparison::default(),
                                characters: Some(&mut reuse),
                                lexical: None,
                                intel: None,
                                projection: None,
                                helper: None,
                            },
                        )
                        .0;
                        assert!(current.tables.iter().all(|input| matches!(
                            input,
                            TableInput::Sampled(TableView::Unavailable { .. })
                        )));
                    }
                    _ => unreachable!(),
                }
                assert!(matches!(
                    current.tables[table.index()],
                    TableInput::Sampled(TableView::Unavailable { .. })
                ));
                campaign.apply_staggered_inputs(&mut current);
                campaign
                    .write(current, 1001, Some(1), 0, &mut WorkSample::default())
                    .unwrap();
                // The runtime frees what the commit handed back.
                campaign.retired.clear();
                assert!(previous.upgrade().is_none());
                let next = character_frame(&mut campaign, branch, 2, 10);
                assert!(matches!(
                    next.tables[table.index()],
                    TableInput::SharedComplete(_)
                ));
            }
        }
    }

    fn cached_complete(campaign: &Campaign<MemoryBackend>, table: TableId) -> &Arc<TableRows> {
        let CachedTable::Complete(rows) = campaign.staggered_cache.get(&table).unwrap() else {
            panic!("fixture expected complete cache")
        };
        rows
    }

    #[test]
    fn scientist_daily_projection_survives_off_slot_keyframe_slot_and_following_delta() {
        use replay_frame::frame::{FrameDecoder, TableUpdate};
        let (mut campaign, store) = campaign();
        let mut decoder = FrameDecoder::new();
        for (hour, daily, ordinal) in [(3, false, 0), (4, true, 0), (5, false, 1)] {
            let branch = campaign.observe(None, hour, Some(1)).unwrap();
            let mut input: FrameInput = staggered_frame(branch, hour, false).into();
            for (id, table) in collector_projects::projects::TABLES.into_iter().zip(
                collector_projects::projects::scientist_spool_fixture(daily, ordinal),
            ) {
                if matches!(
                    id,
                    TableId::Scientists | TableId::ScientistSkills | TableId::ScientistDaily
                ) {
                    input.tables[id.index()] = table.into();
                }
            }
            let skipped = campaign.apply_staggered_inputs(&mut input);
            assert_eq!(skipped[TableId::ScientistDaily.index()], !daily);
            campaign
                .write(
                    input,
                    1000 + i64::from(hour),
                    Some(1),
                    0,
                    &mut WorkSample::default(),
                )
                .unwrap();
            assert_eq!(campaign.last_written, Some(hour));
            campaign.retired.clear();
            let recovered = recover_prefix(store.borrow().segments.values().last().unwrap());
            let encoded = recovered
                .records
                .iter()
                .rev()
                .find_map(|record| match record {
                    Record::Frame(frame) => Some(frame),
                    _ => None,
                })
                .unwrap();
            if hour == 5 {
                assert!(matches!(
                    &encoded.tables[TableId::Scientists.index()],
                    TableUpdate::Delta { .. }
                ));
            }
            let decoded = decoder.apply(encoded).unwrap();
            assert_eq!(
                decoded.complete::<tables::Scientists>().unwrap()[0].ordinal,
                ordinal
            );
            if hour >= 4 {
                let daily = &decoded.complete::<tables::ScientistDaily>().unwrap()[0];
                assert_eq!(
                    daily.injury_days,
                    replay_data::rows::Recorded::Value(Some(8))
                );
                assert_eq!(
                    daily.template_visibility,
                    replay_data::rows::Recorded::Value(Some(1))
                );
                assert!(
                    matches!(&daily.template_origin, replay_data::rows::Recorded::Value(Some(origin)) if origin.line == 17)
                );
            } else {
                assert!(decoded.complete::<tables::ScientistDaily>().is_err());
            }
        }
    }

    #[test]
    fn staggered_complete_cache_and_encoder_share_refreshes_off_hours_and_keyframes() {
        let (mut campaign, store) = campaign();
        let mut reference = FrameEncoder::new();
        let table = DAILY;
        let mut value = 1;
        let mut previous_allocation = None;
        let mut previous_hour = None;
        for (hour, refresh) in [
            (0, Some(1)),
            (1, None),
            (3, Some(1)),
            (7, Some(2)),
            (8, None),
            (720, None),
        ] {
            if let Some(previous) = previous_hour
                && hour > previous + 1
            {
                campaign
                    .gap(
                        campaign.tracker.current_branch().unwrap(),
                        previous + 1,
                        hour - 1,
                        "fixture queue gap".into(),
                    )
                    .unwrap();
            }
            let branch = campaign.observe(None, hour, Some(1)).unwrap();
            previous_hour = Some(hour);
            let mut expected = staggered_frame(branch, hour, false);
            if let Some(next_value) = refresh {
                value = next_value;
            }
            *expected.tables[table.index()] = TableView::Complete(daily_rows(value));
            let mut input: FrameInput = if refresh.is_some() {
                expected.clone().into()
            } else {
                staggered_frame(branch, hour, true).into()
            };
            let sampled_literal = match &input.tables[table.index()] {
                TableInput::Sampled(TableView::Complete(TableRows::AirAces(rows))) => {
                    Some(rows[0].name.as_ptr())
                }
                _ => None,
            };
            let skipped = campaign.apply_staggered_inputs(&mut input);
            assert_eq!(skipped[table.index()], refresh.is_none());
            if let TableInput::SharedComplete(rows) = &input.tables[table.index()] {
                assert!(Arc::ptr_eq(rows, cached_complete(&campaign, table)));
                if let Some(pointer) = sampled_literal {
                    assert_eq!(
                        rows.rows::<tables::AirAces>().unwrap()[0].name.as_ptr(),
                        pointer
                    );
                }
            } else {
                assert!(
                    matches!(&input.tables[table.index()], TableInput::Sampled(TableView::Unavailable { reason }) if reason == replay_data::cadence::SKIPPED_REASON)
                );
            }
            campaign
                .write(
                    input,
                    1000 + i64::from(hour),
                    Some(1),
                    0,
                    &mut WorkSample::default(),
                )
                .unwrap();
            // The runtime frees what the commit handed back.
            campaign.retired.clear();
            let rows = cached_complete(&campaign, table);
            assert_eq!(
                Arc::strong_count(rows),
                2,
                "only cache and encoder own the allocation"
            );
            if let Some(previous) = previous_allocation.take() {
                if refresh.is_some() {
                    assert!(
                        std::sync::Weak::upgrade(&previous).is_none(),
                        "equal refresh must replace the old allocation too"
                    );
                } else {
                    assert!(Arc::ptr_eq(&previous.upgrade().unwrap(), rows));
                }
            }
            previous_allocation = Some(Arc::downgrade(rows));
            let reference_frame = reference.encode(expected, false).unwrap();
            let recovered = recover_prefix(store.borrow().segments.values().last().unwrap());
            let actual = recovered
                .records
                .iter()
                .rev()
                .find_map(|record| match record {
                    Record::Frame(frame) => Some(frame),
                    _ => None,
                })
                .unwrap();
            assert_eq!(actual, &reference_frame, "hour {hour}");
        }
    }

    #[test]
    fn latest_shared_sample_survives_encoding_or_write_failure_and_recovery_snapshots_it() {
        use replay_frame::frame::TableUpdate;
        let table = DAILY;
        for fail_encoding in [false, true] {
            let (mut campaign, store) = campaign();
            let branch = campaign.observe(None, 0, Some(1)).unwrap();
            let mut first: FrameInput = staggered_frame(branch, 0, false).into();
            first.tables[table.index()] = TableView::Complete(daily_rows(1)).into();
            campaign.apply_staggered_inputs(&mut first);
            campaign
                .write(first, 1000, Some(1), 0, &mut WorkSample::default())
                .unwrap();
            let previous = Arc::downgrade(cached_complete(&campaign, table));
            campaign.observe(None, 1, Some(1)).unwrap();
            let mut failed: FrameInput = staggered_frame(branch, 1, false).into();
            failed.tables[table.index()] = TableView::Complete(daily_rows(2)).into();
            if fail_encoding {
                failed.tables[TableId::Clock.index()] =
                    TableView::Complete(TableRows::Clock(Vec::new())).into();
            } else {
                store.borrow_mut().fail_frame_flush = true;
            }
            campaign.apply_staggered_inputs(&mut failed);
            let latest = Arc::downgrade(cached_complete(&campaign, table));
            assert!(
                campaign
                    .write(failed, 1001, Some(1), 0, &mut WorkSample::default())
                    .is_err()
            );
            assert!(previous.upgrade().is_none());
            assert_eq!(
                Arc::strong_count(cached_complete(&campaign, table)),
                1,
                "failed encoder has been discarded; latest sample remains cached"
            );
            assert!(!campaign.spool.is_open());
            assert!(!campaign.encoder.can_carry(table, branch, 2, false));
            campaign.observe(None, 2, Some(1)).unwrap();
            let mut restored: FrameInput = staggered_frame(branch, 2, true).into();
            campaign.apply_staggered_inputs(&mut restored);
            let TableInput::SharedComplete(rows) = &restored.tables[table.index()] else {
                panic!()
            };
            assert!(Arc::ptr_eq(rows, &latest.upgrade().unwrap()));
            campaign
                .write(restored, 1002, Some(1), 0, &mut WorkSample::default())
                .unwrap();
            assert_eq!(Arc::strong_count(cached_complete(&campaign, table)), 2);
            let recovered = recover_prefix(store.borrow().segments.get(&1).unwrap());
            let frame = recovered
                .records
                .iter()
                .find_map(|record| match record {
                    Record::Frame(frame) => Some(frame),
                    _ => None,
                })
                .unwrap();
            assert!(frame.keyframe);
            assert_eq!(frame.hour, 2);
            assert_eq!(
                frame.tables[table.index()],
                TableUpdate::Snapshot(daily_rows(2))
            );
            assert!(recovered.records.iter().any(|record| matches!(record, Record::Gap(gap) if gap.first_hour == 1 && gap.last_hour == 1)));
        }
    }

    #[test]
    fn open_spool_carries_skip_marker_but_restart_materializes_cached_rows() {
        use replay_frame::{frame::FrameDecoder, frame::TableUpdate};
        let (mut campaign, _) = campaign();
        let branch = campaign.observe(None, 0, Some(1)).unwrap();
        let mut first = staggered_frame(branch, 0, false);
        campaign.apply_staggered_cache(&mut first);
        let mut reference = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        decoder
            .apply(&reference.encode(first.clone(), true).unwrap())
            .unwrap();
        campaign
            .write(first.clone(), 1000, Some(1), 0, &mut WorkSample::default())
            .unwrap();
        assert!(campaign.spool.is_open());
        let mut second = staggered_frame(branch, 1, true);
        campaign.apply_staggered_cache(&mut second);
        assert!(
            matches!(&*second.tables[DAILY.index()], TableView::Unavailable { reason } if reason == replay_data::cadence::SKIPPED_REASON)
        );
        let encoded = campaign.encoder.encode(second, false).unwrap();
        assert_eq!(
            encoded.tables[DAILY.index()],
            TableUpdate::Unchanged { baseline: 0 }
        );
        assert_eq!(decoder.apply(&encoded).unwrap(), first.clone_with_hour(1));
        campaign.spool.finish_segment().unwrap();
        let mut third = staggered_frame(branch, 2, true);
        campaign.apply_staggered_cache(&mut third);
        assert_eq!(third, first.clone_with_hour(2));
        assert!(campaign.encoder.encode(third, true).unwrap().keyframe);
    }

    #[test]
    fn staggered_off_hours_emit_unchanged_and_hold_last_refresh() {
        use replay_data::rows::TableId;
        use replay_frame::{frame::FrameDecoder, frame::TableUpdate};
        let (mut campaign, _) = campaign();
        let branch = 1;
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let mut first = staggered_frame(branch, 0, false);
        let skipped = campaign.apply_staggered_cache(&mut first);
        assert!(!skipped[DAILY.index()]);
        let frame = encoder.encode(first.clone(), false).unwrap();
        assert!(matches!(
            frame.tables[DAILY.index()],
            TableUpdate::Snapshot(_)
        ));
        assert_eq!(decoder.apply(&frame).unwrap(), first);
        let mut second = staggered_frame(branch, 1, true);
        let skipped = campaign.apply_staggered_cache(&mut second);
        assert!(skipped[DAILY.index()]);
        assert!(!skipped[TableId::Clock.index()]);
        assert_eq!(second, first.clone_with_hour(1));
        let frame = encoder.encode(second.clone(), false).unwrap();
        assert_eq!(
            frame.tables[DAILY.index()],
            TableUpdate::Unchanged { baseline: 0 }
        );
        assert_eq!(decoder.apply(&frame).unwrap(), second);
    }

    trait WithHour {
        fn clone_with_hour(&self, hour: i32) -> Self;
    }

    impl WithHour for DecodedFrame {
        fn clone_with_hour(&self, hour: i32) -> Self {
            let mut next = self.clone();
            next.hour = hour;
            if let TableView::Complete(TableRows::Clock(rows)) =
                &mut *next.tables[replay_data::rows::TableId::Clock.index()]
            {
                rows[0].hour = hour;
            }
            next
        }
    }

    #[test]
    fn refresh_hour_decode_equals_hourly_baseline() {
        use replay_data::rows::TableId;
        use replay_frame::frame::FrameDecoder;
        let branch = 7;
        let mut hourly_encoder = FrameEncoder::new();
        let mut hourly_decoder = FrameDecoder::new();
        let mut staggered_encoder = FrameEncoder::new();
        let mut staggered_decoder = FrameDecoder::new();
        let (mut campaign, _) = campaign();
        let mut expected_tables: [Option<TableView>; TABLE_COUNT] = std::array::from_fn(|_| None);
        for hour in 0..48 {
            let mut hourly = staggered_frame(branch, hour, false);
            *hourly.tables[DAILY.index()] = TableView::Complete(daily_rows(hour as u8));
            let hourly_frame = hourly_encoder.encode(hourly.clone(), false).unwrap();
            let hourly_decoded = hourly_decoder.apply(&hourly_frame).unwrap().clone();
            let mut staggered = hourly;
            for table in TableId::ALL {
                let index = table.index();
                // A retired table is never walked; both recordings decode
                // its catalogued refusal every hour.
                let retired = super::retired_reason(table).is_some();
                if retired || replay_data::cadence::should_walk(table, hour, hour == 0) {
                    expected_tables[index] = Some((*hourly_decoded.tables[index]).clone());
                } else {
                    *staggered.tables[index] = TableView::Unavailable {
                        reason: replay_data::cadence::SKIPPED_REASON.into(),
                    };
                }
            }
            campaign.apply_staggered_cache(&mut staggered);
            let staggered_frame = staggered_encoder.encode(staggered.clone(), false).unwrap();
            let staggered_decoded = staggered_decoder.apply(&staggered_frame).unwrap().clone();
            for table in TableId::ALL {
                let index = table.index();
                assert_eq!(
                    &staggered_decoded.tables[index],
                    expected_tables[index].as_ref().unwrap(),
                    "{table:?} at hour {hour} must equal its last complete sample"
                );
            }
        }
    }

    #[test]
    fn branch_start_clears_staggered_cache() {
        let (mut campaign, _) = campaign();
        let mut first = staggered_frame(1, 0, false);
        campaign.apply_staggered_cache(&mut first);
        let mut second = staggered_frame(2, 1, true);
        let skipped = campaign.apply_staggered_cache(&mut second);
        assert!(skipped[DAILY.index()]);
        assert!(
            matches!(
                &*second.tables[DAILY.index()],
                TableView::Unavailable { reason } if reason.contains("full refresh required")
            ),
            "new branch must not carry stale rows"
        );
    }

    #[test]
    fn truncated_decode_forces_snapshot_next_refresh() {
        // B3: a truncated refresh (as produced by a copy→decode payload
        // with pooled caps smaller than the census) clears the encoder
        // baseline, so the next refresh of that table is a snapshot —
        // never unchanged/delta off a cleared baseline. Extends the
        // hand-built-frame pattern with decode-payload shapes: first frame
        // carries Truncated (rows + missing), second carries the full rows.
        use replay_data::rows::TableId;
        use replay_frame::{frame::FrameDecoder, frame::TableUpdate};
        let (mut campaign, _) = campaign();
        let branch = 3;
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        // First refresh: WorldTension truncated (1 row kept, 2 missing),
        // as a decode payload with tiny pooled caps would emit. Precede it
        // with a Complete frame so the truncation lands off-keyframe
        // (branch-start keyframes refuse truncated tables by design).
        let mut base = staggered_frame(branch, 10, false);
        campaign.apply_staggered_cache(&mut base);
        let base_frame = encoder.encode(base.clone(), false).unwrap();
        decoder.apply(&base_frame).unwrap();
        let mut first = staggered_frame(branch, 11, false);
        *first.tables[TableId::WorldTension.index()] = TableView::Truncated {
            rows: TableRows::WorldTension(vec![replay_data::rows::WorldTensionRow::default()]),
            missing: 2,
        };
        campaign.apply_staggered_cache(&mut first);
        let frame = encoder.encode(first.clone(), false).unwrap();
        assert!(
            matches!(
                frame.tables[TableId::WorldTension.index()],
                TableUpdate::Truncated { .. }
            ),
            "truncated refresh must encode as truncated, got {:?}",
            frame.tables[TableId::WorldTension.index()]
        );
        assert_eq!(decoder.apply(&frame).unwrap(), first);
        // Second refresh: full rows. Must be a snapshot (baseline was
        // cleared by the truncation), never unchanged/delta.
        let mut second = staggered_frame(branch, 12, false);
        campaign.apply_staggered_cache(&mut second);
        let frame = encoder.encode(second.clone(), false).unwrap();
        assert!(
            matches!(
                frame.tables[TableId::WorldTension.index()],
                TableUpdate::Snapshot(_)
            ),
            "post-truncation refresh must be a snapshot, not delta/unchanged"
        );
        assert_eq!(decoder.apply(&frame).unwrap(), second);
    }
}
