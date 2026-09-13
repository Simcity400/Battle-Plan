//! Conversion, differencing and durable output. Nothing here runs on the game thread.

use crate::decode;
use crate::history::PreparedCampaigns;
use crate::sampling::{
    CONSISTENCY_TABLES, CONSISTENCY_TARGET, Captured, LossBatch, LostObservation,
    qpc as current_qpc, stable_clock,
};
use engine::{cgame_state::Clock, walk::Outcome};
use replay::{
    BranchId,
    branches::{BranchStart, BranchTracker, Gap},
    codec::invalid,
    frame::{DecodedFrame, FrameEncoder, Metrics, TABLE_COUNT, TableView},
    identity::SessionStart,
    rows::{
        CampaignRow, ClockRow, CountryLinkRow, CountryRow, DivisionRow, FactionMemberRow,
        FactionRow, ProvinceRow, RowCodec, StateRow, TableRows, TableSchema, WarRelationRow,
        tables,
    },
    spool::{ControlRecord, FileBackend, Record, SegmentBackend, SegmentedSpool},
};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub(super) struct Distribution {
    samples_ns: Vec<u64>,
    total_ns: u64,
    cached: Cell<Option<Percentiles>>,
    scratch: RefCell<Vec<u64>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Percentiles {
    p50_ns: u64,
    p95_ns: u64,
    p99_ns: u64,
    max_ns: u64,
}

fn exact_percentiles(samples: &mut [u64]) -> Percentiles {
    if samples.is_empty() {
        return Percentiles::default();
    }
    let max_ns = samples.iter().copied().max().unwrap();
    let ranks = [50, 95, 99].map(|percent| (samples.len() * percent).div_ceil(100) - 1);
    let mut values = [0; 3];
    let mut remaining = samples;
    let mut offset = 0;
    let mut selected = 0;
    for (rank, value) in ranks.into_iter().zip(&mut values) {
        if rank >= offset {
            let (_, at_rank, suffix) = remaining.select_nth_unstable(rank - offset);
            selected = *at_rank;
            remaining = suffix;
            offset = rank + 1;
        }
        // Small samples can put several requested percentiles at the same rank.
        *value = selected;
    }
    Percentiles {
        p50_ns: values[0],
        p95_ns: values[1],
        p99_ns: values[2],
        max_ns,
    }
}

impl Distribution {
    pub fn record_ticks(&mut self, ticks: i64, frequency: u64) {
        let ns = ((ticks.max(0) as u128 * 1_000_000_000) / u128::from(frequency)) as u64;
        self.record_ns(ns);
    }

    fn record_ns(&mut self, ns: u64) {
        self.total_ns = self.total_ns.saturating_add(ns);
        self.samples_ns.push(ns);
        self.cached.set(None);
    }

    fn percentiles(&self) -> Percentiles {
        // Repeated status reads reuse exact results until another sample arrives.
        if let Some(cached) = self.cached.get() {
            return cached;
        }
        let mut scratch = self.scratch.borrow_mut();
        scratch.clone_from(&self.samples_ns);
        let result = exact_percentiles(&mut scratch);
        self.cached.set(Some(result));
        result
    }

    pub fn json(&self) -> Value {
        let percentiles = self.percentiles();
        let count = self.samples_ns.len();
        json!({
            "calls": count, "total_ns": self.total_ns,
            "mean_ns": if count == 0 { 0.0 } else { self.total_ns as f64 / count as f64 },
            "p50_ns": percentiles.p50_ns, "p95_ns": percentiles.p95_ns,
            "p99_ns": percentiles.p99_ns, "max_ns": percentiles.max_ns,
        })
    }
}

struct PendingBranch {
    start: BranchStart,
    gaps: Vec<Gap>,
}

#[derive(Default)]
struct WorkSample {
    conversion: i64,
    frame_encoding: Option<i64>,
    record_writes: Option<i64>,
}

struct Campaign<B: SegmentBackend = FileBackend> {
    directory: PathBuf,
    session: SessionStart,
    tracker: BranchTracker,
    encoder: FrameEncoder,
    spool: SegmentedSpool<B>,
    disk_branch: Option<BranchId>,
    last_written: Option<i32>,
    pending: Vec<PendingBranch>,
    staggered_cache: BTreeMap<replay::rows::TableId, TableView>,
    staggered_branch: Option<BranchId>,
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
            && fs::read_dir(&directory)?.any(|entry| {
                entry.is_ok_and(|entry| entry.file_name().to_string_lossy().starts_with("spool."))
            })
        {
            return Err(invalid(
                "attach output already contains spool segments; use a fresh attach directory",
            ));
        }
        let spool = SegmentedSpool::new(FileBackend::new(&directory)?);
        let (tracker, history_file) = history.take(id, session)?;
        Ok(Self {
            directory,
            session: session.clone(),
            tracker,
            encoder: FrameEncoder::new(),
            spool,
            disk_branch: None,
            last_written: None,
            pending: Vec::new(),
            staggered_cache: BTreeMap::new(),
            staggered_branch: None,
            _history_file: history_file,
        })
    }
}

impl<B: SegmentBackend> Campaign<B> {
    fn pending_branch(&mut self, branch: BranchId) -> &mut PendingBranch {
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
            self.pending.push(PendingBranch {
                start,
                gaps: Vec::new(),
            });
        }
        self.pending.last_mut().unwrap()
    }

    fn gap(&mut self, branch: BranchId, first_hour: i32, last_hour: i32, reason: String) {
        let gaps = &mut self.pending_branch(branch).gaps;
        if let Some(previous) = gaps.last_mut()
            && previous.reason == reason
            && previous.last_hour.checked_add(1) == Some(first_hour)
        {
            previous.last_hour = last_hour;
        } else {
            gaps.push(Gap {
                branch,
                first_hour,
                last_hour,
                reason,
            });
        }
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
        self.pending_branch(observation.branch);
        Ok(observation.branch)
    }

    // Spool-internal carry-forward keeps format-neutral byte-identical output.
    fn apply_staggered_cache(&mut self, source: &mut DecodedFrame) -> [bool; TABLE_COUNT] {
        if self.staggered_branch != Some(source.branch) {
            self.staggered_cache.clear();
            self.staggered_branch = Some(source.branch);
        }
        let mut skipped = [false; TABLE_COUNT];
        for table in replay::rows::TableId::ALL {
            let index = table.index();
            if replay::cadence::refresh_slot(table).is_none() {
                continue;
            }
            let is_skip = matches!(
                &source.tables[index],
                TableView::Unavailable { reason }
                if reason == replay::cadence::SKIPPED_REASON
            );
            if is_skip {
                skipped[index] = true;
                if let Some(cached) = self.staggered_cache.get(&table) {
                    source.tables[index] = cached.clone();
                } else {
                    source.tables[index] = TableView::Unavailable {
                        reason: "staggered cache empty on new branch; full refresh required".into(),
                    };
                }
            } else {
                self.staggered_cache
                    .insert(table, source.tables[index].clone());
            }
        }
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
        let branch = self.observe(origin, loss.first_hour, loss.load_signal)?;
        if loss.first_hour != loss.last_hour {
            self.gap(
                branch,
                loss.first_hour,
                loss.last_hour - 1,
                "collector queue overflow".into(),
            );
            self.observe(None, loss.last_hour, loss.load_signal)?;
            self.gap(
                branch,
                loss.last_hour,
                loss.last_hour,
                "collector queue overflow".into(),
            );
        } else {
            self.gap(
                branch,
                loss.first_hour,
                loss.last_hour,
                "collector queue overflow".into(),
            );
        }
        Ok(())
    }

    fn context(&self, restarting: bool) -> Vec<Record> {
        let mut records = Vec::new();
        let mut active = self.disk_branch;
        if restarting {
            records.push(Record::SessionStart(self.session.clone()));
            active = None;
        }
        for pending in &self.pending {
            if active != Some(pending.start.id) {
                records.push(Record::BranchStart(pending.start.clone()));
                active = Some(pending.start.id);
            }
            records.extend(pending.gaps.iter().cloned().map(Record::Gap));
        }
        records
    }

    fn write(
        &mut self,
        source: DecodedFrame,
        qpc: i64,
        signal: Option<i64>,
        unknown: u64,
        work: &mut WorkSample,
    ) -> io::Result<()> {
        let (branch, hour) = (source.branch, source.hour);
        let result = self.write_inner(source, qpc, signal, unknown, work);
        if let Err(error) = &result {
            // Codec/validation errors are failures too. Closing also prevents a
            // partially published context from being reused after such an error.
            let close_started = current_qpc();
            let _ = self.spool.finish_segment();
            *work.record_writes.get_or_insert(0) += current_qpc() - close_started;
            self.disk_branch = None;
            self.encoder = FrameEncoder::new();
            self.gap(branch, hour, hour, format!("spool write failed: {error}"));
        }
        result
    }

    fn write_inner(
        &mut self,
        source: DecodedFrame,
        qpc: i64,
        signal: Option<i64>,
        unknown: u64,
        work: &mut WorkSample,
    ) -> io::Result<()> {
        let (branch, hour) = (source.branch, source.hour);
        let restarting = !self.spool.is_open();
        if restarting {
            self.encoder = FrameEncoder::new();
        }
        let encoding_started = current_qpc();
        let frame = self.encoder.encode(source, restarting);
        work.frame_encoding = Some(current_qpc() - encoding_started);
        let frame = frame?;
        let write_started = current_qpc();
        let result = (|| {
            let mut context = self.context(restarting);
            if unknown != 0 {
                context.push(Record::Control(ControlRecord::GapSummary {
                    session: self.session.id,
                    dropped_frames: unknown,
                    reason: "collector lost clock metadata; campaign and hour range are unknown"
                        .into(),
                }));
            }
            context.push(Record::Control(ControlRecord::ClockWitness {
                session: self.session.id,
                qpc: u64::try_from(qpc).map_err(|_| invalid("negative callback QPC"))?,
                hour,
                load_signal: signal,
            }));
            if restarting {
                self.spool.start_segment(&context, &frame)
            } else {
                context
                    .iter()
                    .try_for_each(|record| self.spool.append(record))
                    .and_then(|()| self.spool.append(&Record::Frame(frame)))
            }
        })();
        work.record_writes = Some(current_qpc() - write_started);
        match result {
            Ok(()) => {
                self.tracker.record_hour(branch, hour)?;
                self.disk_branch = Some(branch);
                self.last_written = Some(hour);
                self.pending.clear();
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
        for record in self.context(false) {
            self.spool.append(&record)?;
        }
        if unknown != 0 {
            self.spool
                .append(&Record::Control(ControlRecord::GapSummary {
                    session: self.session.id,
                    dropped_frames: unknown,
                    reason: "collector lost clock metadata; campaign and hour range are unknown"
                        .into(),
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

struct Statistics {
    hours: u64,
    campaign: Option<String>,
    last_hour: Option<i32>,
    total_ns: u64,
    callback: Distribution,
    table_ns: [u64; TABLE_COUNT],
    war_matrix_ns: u64,
    dropped_frames: u64,
    truncated_tables: usize,
    links_mismatches: u64,
    states_mismatches: u64,
    divisions_mismatches: u64,
    wars_mismatches: u64,
    factions_mismatches: u64,
    faction_members_mismatches: u64,
    provinces_mismatches: u64,
    map_mismatches: [u64; crate::map_capture::MAP_TABLES],
    consistency_successful: [u16; CONSISTENCY_TABLES.len()],
    extended_successful: [u16; TABLE_COUNT],
    extended_mismatches: [u64; TABLE_COUNT],
    country_cache_failures: u64,
    country_cache_fallbacks: u64,
    country_cache_entries: usize,
    country_cache_status: engine::country_memory::SeedStatus,
    country_cache_error: Option<&'static str>,
    country_cache_cost: Distribution,
    war_matrix_checks: u64,
    war_matrix_failures: u64,
    war_matrix_cost: Distribution,
    pool_bytes: u64,
    sampled_buffer_bytes: usize,
    sampled_buffer_parts: Vec<(&'static str, usize)>,
    staggered_skipped: [u64; TABLE_COUNT],
    staggered_walked: [u64; TABLE_COUNT],
    previous: Option<(String, BranchId, i32, i64)>,
    interval_hours: u64,
    interval_qpc: u64,
    errors: u64,
    last_error: Option<String>,
    conversion: Distribution,
    conversion_packs: [Distribution; crate::extended::PACK_NAMES.len()],
    frame_encoding: Distribution,
    record_writes: Distribution,
}

impl Default for Statistics {
    fn default() -> Self {
        Self {
            hours: 0,
            campaign: None,
            last_hour: None,
            total_ns: 0,
            callback: Distribution::default(),
            table_ns: [0; TABLE_COUNT],
            war_matrix_ns: 0,
            dropped_frames: 0,
            truncated_tables: 0,
            links_mismatches: 0,
            states_mismatches: 0,
            divisions_mismatches: 0,
            wars_mismatches: 0,
            factions_mismatches: 0,
            faction_members_mismatches: 0,
            provinces_mismatches: 0,
            map_mismatches: [0; crate::map_capture::MAP_TABLES],
            consistency_successful: [0; CONSISTENCY_TABLES.len()],
            extended_successful: [0; TABLE_COUNT],
            extended_mismatches: [0; TABLE_COUNT],
            country_cache_failures: 0,
            country_cache_fallbacks: 0,
            country_cache_entries: 0,
            country_cache_error: None,
            country_cache_status: engine::country_memory::SeedStatus::Unavailable {
                reason: "no observation",
            },
            country_cache_cost: Distribution::default(),
            war_matrix_checks: 0,
            war_matrix_failures: 0,
            war_matrix_cost: Distribution::default(),
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
            conversion: Distribution::default(),
            conversion_packs: std::array::from_fn(|_| Distribution::default()),
            frame_encoding: Distribution::default(),
            record_writes: Distribution::default(),
        }
    }
}

impl Statistics {
    fn observe_consistency(&mut self, captured: &Captured) {
        if captured.consistency_checked[crate::sampling::PROVINCE_CHECK] {
            self.provinces_mismatches += captured.provinces_mismatches as u64;
        }
        for (index, total) in self.consistency_successful.iter_mut().enumerate() {
            *total = (*total).max(captured.consistency_successful[index]);
        }
        let extended_mismatches = captured.extended.checked_mismatches();
        for table in crate::extended::TABLES {
            let index = table.index();
            self.extended_successful[index] =
                self.extended_successful[index].max(captured.extended_successful[index]);
            if captured.extended_checked[index] {
                self.extended_mismatches[index] += extended_mismatches[index]
                    .max(usize::from(captured.extended_raw_mismatches[index]))
                    as u64;
            }
        }
        self.country_cache_failures += u64::from(captured.country_cache_error.is_some());
        self.country_cache_fallbacks +=
            u64::from(captured.country_cache_status != engine::country_memory::SeedStatus::Ready);
        self.country_cache_entries = captured.country_cache_entries;
        self.country_cache_status = captured.country_cache_status;
        self.country_cache_error = captured.country_cache_error;
        self.country_cache_cost.record_ns(captured.country_cache_ns);
        for (total, count) in self
            .map_mismatches
            .iter_mut()
            .zip(captured.map.checked_mismatches())
        {
            *total += count;
        }
        for (index, (total, count)) in [
            (&mut self.links_mismatches, captured.links_mismatches),
            (&mut self.states_mismatches, captured.states_mismatches),
            (
                &mut self.divisions_mismatches,
                captured.divisions_mismatches,
            ),
            (&mut self.wars_mismatches, captured.wars_mismatches),
            (&mut self.factions_mismatches, captured.factions_mismatches),
            (
                &mut self.faction_members_mismatches,
                captured.faction_members_mismatches,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            if captured.consistency_checked[index] {
                *total += count as u64;
            }
        }
        if captured.war_matrix_result.is_some() {
            self.war_matrix_cost
                .record_ticks(captured.war_matrix_ns as i64, 1_000_000_000);
            self.war_matrix_checks += u64::from(
                matches!(
                    (captured.wars_result, captured.war_matrix_result),
                    (Outcome::Rows { .. }, Some(Outcome::Rows { .. }))
                ) && stable_clock(captured.clock_before, captured.clock_after)
                    && (!captured.consistency_checked[3] || captured.wars_mismatches == 0),
            );
            self.war_matrix_failures += u64::from(war_matrix_issue(captured).is_some());
        }
    }

    fn consistency_status(&self) -> Value {
        let mut successful: serde_json::Map<String, Value> = CONSISTENCY_TABLES
            .into_iter()
            .zip(self.consistency_successful)
            .map(|(table, count)| (table.to_owned(), json!(count)))
            .collect();
        let mut mismatches: serde_json::Map<String, Value> = CONSISTENCY_TABLES
            .into_iter()
            .zip(
                [
                    self.links_mismatches,
                    self.states_mismatches,
                    self.divisions_mismatches,
                    self.wars_mismatches,
                    self.factions_mismatches,
                    self.faction_members_mismatches,
                ]
                .into_iter()
                .chain(self.map_mismatches)
                .chain([self.provinces_mismatches]),
            )
            .map(|(table, count)| (table.to_owned(), json!(count)))
            .collect();
        for table in crate::extended::TABLES {
            successful.insert(
                format!("{table:?}"),
                json!(self.extended_successful[table.index()]),
            );
            mismatches.insert(
                format!("{table:?}"),
                json!(self.extended_mismatches[table.index()]),
            );
        }
        json!({
            "scope": "this collector attach",
            "target_per_table": CONSISTENCY_TARGET,
            "successful_checks": successful,
            "mismatches": mismatches,
        })
    }
}

pub struct Processor {
    root: PathBuf,
    history: PreparedCampaigns,
    session: SessionStart,
    campaigns: BTreeMap<String, Campaign>,
    stats: Statistics,
    unknown_pending: u64,
    suppress_origin: bool,
}

impl Processor {
    pub fn new(root: &Path, history: PreparedCampaigns, session: SessionStart) -> io::Result<Self> {
        session.validate()?;
        fs::create_dir_all(root)?;
        Ok(Self {
            root: root.into(),
            history,
            session,
            campaigns: BTreeMap::new(),
            stats: Statistics::default(),
            unknown_pending: 0,
            suppress_origin: false,
        })
    }

    pub fn set_verified_session(&mut self, session: SessionStart) {
        // The spool worker calls this exactly once, before its first observation.
        assert!(self.campaigns.is_empty());
        self.session = session;
    }

    fn campaign(&mut self, id: &str) -> io::Result<&mut Campaign> {
        if !self.campaigns.contains_key(id) {
            let campaign = Campaign::open(&self.root, &mut self.history, id, &self.session)?;
            self.campaigns.insert(id.into(), campaign);
        }
        Ok(self.campaigns.get_mut(id).unwrap())
    }

    fn error(&mut self, error: &io::Error) {
        self.stats.errors += 1;
        self.stats.last_error = Some(error.to_string());
    }

    /// The caller retains the buffer until this succeeds. Opening a campaign may
    /// do I/O, but this preflight never observes or acknowledges a callback.
    /// Once prepared, consume its loss witnesses and then call `frame` once.
    pub fn prepare(&mut self, captured: &Captured) -> io::Result<()> {
        let clock = captured
            .clock_before
            .as_ref()
            .or(captured.clock_after.as_ref());
        let ids = captured
            .dropped_before
            .iter()
            .map(|loss| loss.campaign_id.as_bytes())
            .chain(clock.into_iter().map(|clock| clock.campaign_id.as_bytes()));
        let result = prepare_campaigns(ids, |id| self.campaign(id).map(|_| ()));
        if let Err(error) = &result {
            self.error(error);
        }
        result
    }

    /// Quiescent detach uses the same preflight before consuming its final batch.
    pub fn prepare_losses(&mut self, losses: &LossBatch) -> io::Result<()> {
        let result = prepare_campaigns(
            losses.iter().map(|loss| loss.campaign_id.as_bytes()),
            |id| self.campaign(id).map(|_| ()),
        );
        if let Err(error) = &result {
            self.error(error);
        }
        result
    }

    pub fn lost(&mut self, loss: &LostObservation) -> io::Result<()> {
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

    pub fn frame(&mut self, captured: &Captured) -> io::Result<()> {
        let result = self.frame_inner(captured);
        if let Err(error) = &result {
            self.error(error);
        }
        result
    }

    fn frame_inner(&mut self, captured: &Captured) -> io::Result<()> {
        let Some(clock) = captured.clock_before.or(captured.clock_after) else {
            self.unknown_loss(1);
            return Err(invalid(
                "callback clocks unreadable; campaign and hour cannot be recorded",
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
        let campaign = self.campaign(&id)?;
        let origin = if suppress || campaign.tracker.current_branch().is_none() {
            None
        } else {
            captured
                .clock_entry
                .filter(|entry| entry.campaign_id == clock.campaign_id)
                .map(|entry| entry.hour)
        };
        let branch = campaign.observe(origin, clock.hour, Some(clock.play_session_started_at))?;
        let conversion_started = current_qpc();
        let (mut source, pack_ticks) = decoded_with_costs(captured, clock, branch);
        let skipped = campaign.apply_staggered_cache(&mut source);
        let mut work = WorkSample {
            conversion: current_qpc() - conversion_started,
            ..WorkSample::default()
        };
        let result = campaign.write(
            source,
            captured.qpc,
            Some(clock.play_session_started_at),
            unknown,
            &mut work,
        );
        for table in replay::rows::TableId::ALL {
            let index = table.index();
            if replay::cadence::is_staggered(table) {
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
        self.unknown_pending = 0;
        self.suppress_origin = false;
        let stats = &mut self.stats;
        stats.hours += 1;
        stats.campaign = Some(id.clone());
        stats.last_hour = Some(clock.hour);
        stats.total_ns = captured.total_ns;
        stats.callback.record_ns(captured.total_ns);
        stats.table_ns = captured.table_ns;
        stats.war_matrix_ns = captured.war_matrix_ns;
        stats.pool_bytes = captured.pool_bytes;
        stats.sampled_buffer_bytes = captured.allocated_bytes();
        stats.sampled_buffer_parts = captured.extended.allocated_by_pack();
        let extended_bytes = captured.extended.allocated_bytes();
        let map_bytes = captured.map.allocated_bytes();
        stats.sampled_buffer_parts.push(("map", map_bytes));
        stats.sampled_buffer_parts.push((
            "core",
            stats.sampled_buffer_bytes - extended_bytes - map_bytes,
        ));
        stats.dropped_frames = captured.dropped_frames;
        stats.observe_consistency(captured);
        let extended_outcomes = captured.extended.outcomes();
        stats.truncated_tables = [
            captured.countries_result,
            captured.links_result,
            captured.provinces_result,
            captured.states_result,
            captured.divisions_result,
            captured.wars_result,
            captured.factions_result,
            captured.faction_members_result,
        ]
        .into_iter()
        .chain(captured.map.outcomes())
        .chain(crate::extended::TABLES.map(|table| extended_outcomes[table.index()]))
        .filter(|outcome| matches!(outcome, Outcome::Truncated { .. }))
        .count();
        if let Some((old_id, old_branch, hour, qpc)) = &stats.previous
            && old_id == &id
            && *old_branch == branch
            && hour.checked_add(1) == Some(clock.hour)
            && captured.qpc > *qpc
        {
            stats.interval_hours += 1;
            stats.interval_qpc += (captured.qpc - qpc) as u64;
        }
        stats.previous = Some((id, branch, clock.hour, captured.qpc));
        Ok(())
    }

    pub fn flush(&mut self) -> io::Result<()> {
        let mut first_error = None;
        for campaign in self.campaigns.values_mut() {
            if let Err(error) = campaign.spool.sync_if_due() {
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
        let mut first_error = None;
        let summary_campaign = self
            .stats
            .campaign
            .clone()
            .or_else(|| self.campaigns.keys().next_back().cloned());
        for (id, campaign) in &mut self.campaigns {
            let unknown = if Some(id) == summary_campaign.as_ref() {
                self.unknown_pending
            } else {
                0
            };
            match campaign.detach(unknown) {
                Ok(()) if unknown != 0 => self.unknown_pending = 0,
                Ok(()) => {}
                Err(error) => {
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

    pub fn status(&self) -> Value {
        let stats = &self.stats;
        let callback = stats.callback.percentiles();
        let spool_bytes: u64 = self
            .campaigns
            .values()
            .filter_map(|campaign| fs::read_dir(&campaign.directory).ok())
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("spool."))
            .filter_map(|entry| entry.metadata().ok())
            .map(|metadata| metadata.len())
            .sum();
        // Per-family copy-only mirrors, built outside the status `json!` so
        // the macro stays under its recursion limit as families move over.
        let research_copy_ns = [
            stats.table_ns[replay::rows::TableId::ResearchTechnologies.index()],
            stats.table_ns[replay::rows::TableId::ResearchSlots.index()],
            stats.table_ns[replay::rows::TableId::TechnologyDefinitions.index()],
        ];
        let relations_copy_ns = [
            stats.table_ns[replay::rows::TableId::RelationOpinions.index()],
            stats.table_ns[replay::rows::TableId::RelationTuples.index()],
        ];
        let characters_copy_ns = [
            stats.table_ns[replay::rows::TableId::Characters.index()],
            stats.table_ns[replay::rows::TableId::CountryLeaderRoles.index()],
            stats.table_ns[replay::rows::TableId::AdvisorRoles.index()],
            stats.table_ns[replay::rows::TableId::UnitLeaders.index()],
            stats.table_ns[replay::rows::TableId::UnitLeaderTraits.index()],
            stats.table_ns[replay::rows::TableId::UnitLeaderTraitExperience.index()],
        ];
        let projects_copy_ns = [
            stats.table_ns[replay::rows::TableId::ProjectPrograms.index()],
            stats.table_ns[replay::rows::TableId::Projects.index()],
            stats.table_ns[replay::rows::TableId::ProjectHistory.index()],
            stats.table_ns[replay::rows::TableId::ProjectBreakthroughs.index()],
        ];
        let lexical_copy_ns = [stats.table_ns[replay::rows::TableId::LexicalTokens.index()]];
        let variants_copy_ns = [
            stats.table_ns[replay::rows::TableId::VariantDefinitions.index()],
            stats.table_ns[replay::rows::TableId::VariantModules.index()],
            stats.table_ns[replay::rows::TableId::VariantStats.index()],
            stats.table_ns[replay::rows::TableId::VariantUpgrades.index()],
        ];
        let buildings_copy_ns = [
            stats.table_ns[replay::rows::TableId::StateBuildings.index()],
            stats.table_ns[replay::rows::TableId::ProvinceBuildings.index()],
        ];
        // Wars hourly copy mirror: the owned walk shares the WarRelations
        // slot; the matrix census has no game-thread table slot of its own.
        let wars_copy_ns = [stats.table_ns[decode::WARS[0].index()], stats.war_matrix_ns];
        let divisions_copy_ns = [
            stats.table_ns[decode::DIVISIONS[0].index()],
            stats.table_ns[decode::DIVISIONS[1].index()],
            stats.table_ns[decode::DIVISIONS[2].index()],
            stats.table_ns[decode::DIVISIONS[3].index()],
            stats.table_ns[decode::DIVISIONS[4].index()],
            stats.table_ns[decode::DIVISIONS[5].index()],
        ];
        let focus_copy_ns = [
            stats.table_ns[decode::FOCUS[0].index()],
            stats.table_ns[decode::FOCUS[1].index()],
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
        let map_copy_ns = [
            stats.table_ns[decode::MAP_PACK[0].index()],
            stats.table_ns[decode::MAP_PACK[1].index()],
            stats.table_ns[decode::MAP_PACK[2].index()],
            stats.table_ns[decode::MAP_PACK[3].index()],
            stats.table_ns[decode::MAP_PACK[4].index()],
            stats.table_ns[decode::MAP_PACK[5].index()],
            stats.table_ns[decode::MAP_PACK[6].index()],
            stats.table_ns[decode::MAP_PACK[7].index()],
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
        let value = json!({
            "campaign": stats.campaign, "hours": stats.hours, "last_hour": stats.last_hour,
            "total_ns": stats.total_ns, "p50_ns": callback.p50_ns, "p95_ns": callback.p95_ns,
            "max_ns": callback.max_ns, "table_ns": stats.table_ns.as_slice(),
            "staggered_skipped": stats.staggered_skipped.as_slice(),
            "staggered_walked": stats.staggered_walked.as_slice(),
            "hours_per_second": if stats.interval_qpc == 0 { 0.0 } else {
                stats.interval_hours as f64 * self.session.qpc_frequency as f64 / stats.interval_qpc as f64
            },
            "spool_bytes": spool_bytes, "dropped_frames": stats.dropped_frames,
            "truncated_tables": stats.truncated_tables, "links_mismatches": stats.links_mismatches,
            "states_mismatches": stats.states_mismatches,
            "divisions_mismatches": stats.divisions_mismatches,
            "wars_mismatches": stats.wars_mismatches,
            "factions_mismatches": stats.factions_mismatches,
            "faction_members_mismatches": stats.faction_members_mismatches,
            "consistency": stats.consistency_status(),
            "war_matrix_checks": stats.war_matrix_checks,
            "war_matrix_failures": stats.war_matrix_failures,
            "war_matrix_cost": stats.war_matrix_cost.json(),
            "shared_read_groups": crate::extended::read_groups().into_iter().map(|(name, anchor, members)| {
                // Game-thread copy-only for moved families: `table_ns` for
                // moved tables carries copy ns only, while `decode_ns`
                // lives in the decode calibration (`runtime` status). Do not
                // compare `shared_read_groups` totals across moved vs unmoved
                // families (PLAN-appendix apples-to-oranges).
                let total_ns: u64 = members.iter().map(|table| stats.table_ns[table.index()]).sum();
                json!({"name": name, "anchor": format!("{anchor:?}"),
                    "members": members.iter().map(|table| format!("{table:?}")).collect::<Vec<_>>(),
                    "last_ns": total_ns})
            }).collect::<Vec<_>>(),
            "country_authentication": {
                "entries": stats.country_cache_entries,
                "seed_status": format!("{:?}", stats.country_cache_status),
                "fallbacks": stats.country_cache_fallbacks,
                "verification_failures": stats.country_cache_failures,
                "last_error": stats.country_cache_error,
                "cost": stats.country_cache_cost.json(),
                "accounted_in": "Countries",
            },
            "pool_bytes": stats.pool_bytes, "write_errors": stats.errors, "last_error": stats.last_error,
            "sampled_buffer_allocation": {
                "scope": "last consumed native row buffer, before off-thread growth; not the whole pool",
                "bytes": stats.sampled_buffer_bytes,
                "parts": stats.sampled_buffer_parts.iter().map(|(name, bytes)| {
                    json!({"name": name, "bytes": bytes})
                }).collect::<Vec<_>>(),
            },
            "unknown_losses_pending": self.unknown_pending,
            "spool_work": {
                "conversion": stats.conversion.json(),
                "conversion_packs": crate::extended::PACK_NAMES.iter().zip(&stats.conversion_packs)
                    .map(|(name, cost)| json!({"name": name, "cost": cost.json()})).collect::<Vec<_>>(),
                "conversion_packs_scope": "nested within conversion; excludes core and map tables",
                "frame_encoding": stats.frame_encoding.json(),
                "record_writes": stats.record_writes.json(),
                "table_rows_storage_bytes": std::mem::size_of::<TableRows>(),
            },
        });
        // Copy/decode splits: per-frame copy-only nanoseconds per moved
        // family, mirroring the game-thread `table_ns` slots. Monotonic
        // decode totals live in the runtime status instead. `table_ns` for
        // moved families is game-thread copy-only (decode lives in the
        // calibration); do not compare copy ns against decode ns across
        // threads (PLAN-appendix apples-to-oranges). Inserted outside
        // the status `json!` so the macro stays under its recursion limit as
        // families move over.
        let mut value = value;
        value["research_copy_ns"] = json!(research_copy_ns.as_slice());
        value["relations_copy_ns"] = json!(relations_copy_ns.as_slice());
        value["characters_copy_ns"] = json!(characters_copy_ns.as_slice());
        value["projects_copy_ns"] = json!(projects_copy_ns.as_slice());
        value["lexical_copy_ns"] = json!(lexical_copy_ns.as_slice());
        value["variants_copy_ns"] = json!(variants_copy_ns.as_slice());
        value["buildings_copy_ns"] = json!(buildings_copy_ns.as_slice());
        value["wars_copy_ns"] = json!(wars_copy_ns.as_slice());
        value["divisions_copy_ns"] = json!(divisions_copy_ns.as_slice());
        value["focus_copy_ns"] = json!(focus_copy_ns.as_slice());
        value["production_rest_copy_ns"] = json!(production_rest_copy_ns.as_slice());
        value["construction_rest_copy_ns"] = json!(construction_rest_copy_ns.as_slice());
        value["map_copy_ns"] = json!(map_copy_ns.as_slice());
        value["air_wing_copy_ns"] = json!(air_wing_copy_ns.as_slice());
        value["air_stats_copy_ns"] = json!(air_stats_copy_ns.as_slice());
        value["naval_copy_ns"] = json!(naval_copy_ns.as_slice());
        value["players_copy_ns"] = json!(players_copy_ns.as_slice());
        value["session_copy_ns"] = json!(session_copy_ns.as_slice());
        value
    }
}

fn prepare_campaigns<'a>(
    ids: impl IntoIterator<Item = &'a [u8]>,
    mut open: impl FnMut(&str) -> io::Result<()>,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    for bytes in ids {
        // Malformed identities cannot become valid through an I/O retry. Their
        // actual consumption emits an unattributed loss and a named error.
        if let Ok(id) = campaign_name(bytes)
            && seen.insert(id.clone())
        {
            open(&id)?;
        }
    }
    Ok(())
}

pub(super) fn campaign_name(bytes: &[u8]) -> io::Result<String> {
    if bytes.len() != 36
        || bytes.iter().enumerate().any(|(i, byte)| {
            if [8, 13, 18, 23].contains(&i) {
                *byte != b'-'
            } else {
                !byte.is_ascii_hexdigit()
            }
        })
    {
        return Err(invalid(
            "campaign id is not a UUID; refusing an unsafe output path",
        ));
    }
    // Preserve the engine's spelling, including case, in both the path and row.
    Ok(String::from_utf8(bytes.to_vec()).expect("validated UUID is ASCII"))
}

pub(crate) fn unavailable(reason: &str) -> TableView {
    TableView::Unavailable {
        reason: reason.into(),
    }
}

pub(crate) fn table<S: TableSchema, T>(
    outcome: Outcome,
    source: &[T],
    convert: impl Fn(&T) -> S::Row,
) -> TableView {
    let Some(source) = source.get(..outcome.len()) else {
        return unavailable("walker row count exceeds allocated buffer");
    };
    table_iter::<S, _>(outcome, source.iter(), convert)
}

/// Convert borrowed rows without first materializing a parallel reference list.
pub(crate) fn table_iter<S: TableSchema, T>(
    outcome: Outcome,
    source: impl ExactSizeIterator<Item = T>,
    convert: impl Fn(T) -> S::Row,
) -> TableView {
    match outcome {
        Outcome::Unavailable { reason } => unavailable(reason),
        Outcome::Rows { len } | Outcome::Truncated { len, .. } => {
            if source.len() != len {
                return unavailable("walker row count exceeds allocated buffer");
            }
            let mut rows: Vec<_> = source.map(convert).collect();
            rows.sort_unstable_by_key(RowCodec::key);
            if rows.windows(2).any(|rows| rows[0].key() == rows[1].key()) {
                return unavailable("walker produced duplicate engine ids");
            }
            let rows = TableRows::from_rows::<S>(rows);
            match outcome {
                Outcome::Truncated { unread_rows, .. } if unread_rows != 0 => {
                    TableView::Truncated {
                        rows,
                        missing: unread_rows,
                    }
                }
                Outcome::Truncated { .. } => {
                    unavailable("walker marked truncation without a missing count")
                }
                _ => TableView::Complete(rows),
            }
        }
    }
}

/// The two native censuses have different iteration orders. Compare their full
/// rows here, off the game thread, while leaving encoding to the common path.
fn war_matrix_issue(captured: &Captured) -> Option<&'static str> {
    let Outcome::Rows { len } = captured.wars_result else {
        return None;
    };
    match captured.war_matrix_result? {
        Outcome::Rows { len: other_len } => (len != other_len
            || captured.wars[..len]
                .iter()
                .any(|row| !captured.wars_matrix[..other_len].contains(row)))
        .then_some("war owned-vector and matrix censuses disagree"),
        Outcome::Truncated { .. } => Some("independent war matrix census truncated"),
        Outcome::Unavailable { .. } => Some("independent war matrix census unavailable"),
    }
}

fn faction_table(captured: &Captured) -> TableView {
    if captured.consistency_checked[4] && captured.factions_mismatches != 0 {
        return unavailable("factions changed during double read");
    }
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
            name: captured.faction_names[faction.name_start..faction.name_start + faction.name_len]
                .to_vec(),
        }
    })
}

#[cfg(test)]
fn decoded(captured: &Captured, clock: Clock, branch: BranchId) -> DecodedFrame {
    decoded_with_costs(captured, clock, branch).0
}

fn decoded_with_costs(
    captured: &Captured,
    clock: Clock,
    branch: BranchId,
) -> (
    DecodedFrame,
    [Option<i64>; crate::extended::PACK_NAMES.len()],
) {
    let mut pack_ticks = [None; crate::extended::PACK_NAMES.len()];
    let metrics = Metrics {
        total_ns: captured.total_ns,
        table_ns: captured.table_ns,
        pool_bytes: captured.pool_bytes,
        dropped_frames: captured.dropped_frames,
    };
    let failure = match (captured.clock_before, captured.clock_after) {
        (Some(before), Some(after)) if before.hour != after.hour => {
            Some("hour changed during read")
        }
        (Some(before), Some(after))
            if before.game_state != after.game_state
                || before.campaign_id != after.campaign_id
                || before.play_session_started_at != after.play_session_started_at =>
        {
            Some("campaign identity changed during read")
        }
        (None, _) | (_, None) => Some("clock unreadable around table walk"),
        _ => None,
    };
    let counters = captured.counters;
    let tables = if let Some(reason) = failure {
        std::array::from_fn(|_| unavailable(reason))
    } else {
        let [
            players,
            controllers,
            land,
            fleets,
            task_forces,
            ships,
            naval,
            naval_participants,
            air,
            orders,
        ] = crate::map_spool::tables(&captured.map);
        let core = [
            TableView::Complete(TableRows::Clock(vec![ClockRow {
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
            }])),
            TableView::Complete(TableRows::Campaign(vec![CampaignRow {
                id: clock.campaign_id.as_bytes().to_vec(),
            }])),
            table::<tables::Countries, _>(
                captured.countries_result,
                &captured.countries,
                |country| CountryRow {
                    id: country.id,
                    canonical_slot: country.canonical_slot,
                    tag: country.tag.as_bytes().to_vec(),
                    name: country.name.as_bytes().to_vec(),
                    political_power_raw: country.political_power_raw,
                    command_power_raw: country.command_power_raw,
                    stability_raw: country.stability_raw,
                    war_support_raw: country.war_support_raw,
                    convoys_raw: country.convoys_raw,
                },
            ),
            if captured.consistency_checked[0] && captured.links_mismatches != 0 {
                unavailable("country links changed during double read")
            } else {
                table::<tables::CountryLinks, _>(captured.links_result, &captured.links, |link| {
                    CountryLinkRow {
                        id: link.id,
                        canonical_slot: link.canonical_slot,
                    }
                })
            },
            if captured.consistency_checked[crate::sampling::PROVINCE_CHECK]
                && captured.provinces_mismatches != 0
            {
                unavailable("provinces changed during double read")
            } else {
                table::<tables::Provinces, _>(
                    captured.provinces_result,
                    &captured.provinces,
                    |province| ProvinceRow {
                        id: province.id,
                        state: province.state,
                        owner: province.owner,
                        controller: province.controller,
                        victory_points: province.victory_points,
                    },
                )
            },
            if captured.consistency_checked[1] && captured.states_mismatches != 0 {
                unavailable("states changed during double read")
            } else {
                table::<tables::States, _>(captured.states_result, &captured.states, |state| {
                    StateRow {
                        id: state.id,
                        owner: state.owner,
                        controller: state.controller,
                    }
                })
            },
            if captured.consistency_checked[2] && captured.divisions_mismatches != 0 {
                unavailable("divisions changed during double read")
            } else {
                table::<tables::Divisions, _>(
                    captured.divisions_result,
                    &captured.divisions,
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
                    },
                )
            },
            if captured.consistency_checked[3] && captured.wars_mismatches != 0 {
                unavailable("war relations changed during double read")
            } else if let Some(reason) = war_matrix_issue(captured) {
                unavailable(reason)
            } else {
                table::<tables::WarRelations, _>(captured.wars_result, &captured.wars, |war| {
                    WarRelationRow {
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
                    }
                })
            },
            faction_table(captured),
            if captured.consistency_checked[5] && captured.faction_members_mismatches != 0 {
                unavailable("faction members changed during double read")
            } else {
                table::<tables::FactionMembers, _>(
                    captured.faction_members_result,
                    &captured.faction_members,
                    |member| FactionMemberRow {
                        country: member.country,
                        faction: member.faction,
                        ordinal: member.ordinal,
                    },
                )
            },
            players,
            controllers,
            land,
            fleets,
            task_forces,
            ships,
            naval,
            naval_participants,
            air,
            orders,
        ];
        let mut tables = std::array::from_fn(|_| unavailable("table was not acquired"));
        for (destination, value) in tables.iter_mut().zip(core) {
            *destination = value;
        }
        pack_ticks = captured.extended.insert_tables(&mut tables).map(Some);
        apply_raw_verdicts(&mut tables, &captured.extended_raw_mismatches);
        if let Some(reason) = captured.country_cache_error {
            for (index, value) in tables.iter_mut().enumerate() {
                if !matches!(index, 0 | 1 | 4 | 5)
                    && index != replay::rows::TableId::LexicalTokens.index()
                {
                    *value = unavailable(reason);
                }
            }
        }
        tables
    };
    (
        DecodedFrame {
            branch,
            hour: clock.hour,
            tables,
            metrics,
        },
        pack_ticks,
    )
}

fn apply_raw_verdicts(tables: &mut [TableView; TABLE_COUNT], mismatches: &[bool; TABLE_COUNT]) {
    for (index, view) in tables.iter_mut().enumerate() {
        if mismatches[index] && !matches!(view, TableView::Unavailable { .. }) {
            *view = unavailable("raw capture changed during double read");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::container::{Replay, SeekResult};
    use replay::rows::TableId;

    #[test]
    fn raw_mismatch_survives_pack_reset_and_rejects_decoded_rows() {
        let mut source = captured(12);
        let index = TableId::Characters.index();
        source.extended_checked[index] = true;
        source.extended_raw_mismatches[index] = true;
        source.extended.reset();
        assert_eq!(source.extended.checked_mismatches()[index], 0);
        let mut statistics = Statistics::default();
        statistics.observe_consistency(&source);
        assert_eq!(statistics.extended_mismatches[index], 1);
        let mut tables = std::array::from_fn(|_| unavailable("unreadable"));
        tables[index] = TableView::Complete(TableRows::Characters(Vec::new()));
        apply_raw_verdicts(&mut tables, &source.extended_raw_mismatches);
        assert_eq!(
            tables[index],
            unavailable("raw capture changed during double read")
        );
        assert_eq!(tables[TableId::Clock.index()], unavailable("unreadable"));
    }

    #[test]
    fn work_ticks_use_nanoseconds_and_nearest_rank_percentiles() {
        let mut values = Distribution::default();
        assert_eq!(values.json()["calls"], 0);
        for ticks in [4, 1, 3, 2] {
            values.record_ticks(ticks, 10_000_000);
        }
        let report = values.json();
        assert_eq!(report["calls"], 4);
        assert_eq!(report["total_ns"], 1_000);
        assert_eq!(report["mean_ns"], 250.0);
        assert_eq!(report["p50_ns"], 200);
        assert_eq!(report["p95_ns"], 400);
    }

    #[test]
    fn selected_percentiles_match_sorted_ranks_for_small_tied_and_extreme_samples() {
        for len in 0..=257 {
            let mut input: Vec<_> = (0..len)
                .map(|index| match index % 5 {
                    0 => 0,
                    1 => u64::MAX,
                    2 => (index % 7) as u64,
                    3 => (len - index) as u64,
                    _ => 17,
                })
                .collect();
            let mut sorted = input.clone();
            sorted.sort_unstable();
            let at = |percent: usize| {
                if sorted.is_empty() {
                    0
                } else {
                    sorted[(sorted.len() * percent).div_ceil(100) - 1]
                }
            };
            let expected = Percentiles {
                p50_ns: at(50),
                p95_ns: at(95),
                p99_ns: at(99),
                max_ns: sorted.last().copied().unwrap_or(0),
            };
            assert_eq!(exact_percentiles(&mut input), expected, "length {len}");
            sorted.reverse();
            assert_eq!(
                exact_percentiles(&mut sorted),
                expected,
                "reverse length {len}"
            );
        }
    }

    #[test]
    fn cached_percentiles_refresh_after_samples_below_and_above_the_old_range() {
        let mut values = Distribution::default();
        assert_eq!(values.percentiles(), Percentiles::default());
        assert_eq!(values.cached.get(), Some(Percentiles::default()));
        for ns in [10, 20, 30] {
            values.record_ns(ns);
        }
        assert_eq!(values.cached.get(), None);
        let first = values.json();
        assert_eq!(
            first,
            json!({
                "calls": 3, "total_ns": 60, "mean_ns": 20.0,
                "p50_ns": 20, "p95_ns": 30, "p99_ns": 30, "max_ns": 30,
            })
        );
        let cached = values.cached.get();
        assert!(cached.is_some());
        assert_eq!(values.json(), first);
        assert_eq!(values.cached.get(), cached);

        values.record_ns(5);
        assert_eq!(values.cached.get(), None);
        assert_eq!(
            values.json(),
            json!({
                "calls": 4, "total_ns": 65, "mean_ns": 16.25,
                "p50_ns": 10, "p95_ns": 30, "p99_ns": 30, "max_ns": 30,
            })
        );
        values.record_ticks(100, 1_000_000_000);
        assert_eq!(values.cached.get(), None);
        let last = values.json();
        assert_eq!(
            last,
            json!({
                "calls": 5, "total_ns": 165, "mean_ns": 33.0,
                "p50_ns": 20, "p95_ns": 100, "p99_ns": 100, "max_ns": 100,
            })
        );
        assert_eq!(values.json(), last);
        assert_eq!(values.samples_ns, [10, 20, 30, 5, 100]);
    }

    #[test]
    fn cached_callback_summary_does_not_freeze_status_errors_or_latest_cost() {
        let mut processor = Processor {
            root: PathBuf::new(),
            history: PreparedCampaigns::default(),
            session: session(),
            campaigns: BTreeMap::new(),
            stats: Statistics::default(),
            unknown_pending: 0,
            suppress_origin: false,
        };
        for ns in [100, 200, 300] {
            processor.stats.callback.record_ns(ns);
        }
        processor.stats.hours = 3;
        processor.stats.total_ns = 300;
        let first = processor.status();
        assert_eq!(first["p50_ns"], 200);
        assert_eq!(first["p95_ns"], 300);
        assert_eq!(first["max_ns"], 300);
        assert_eq!(first["total_ns"], 300);
        assert_eq!(processor.status(), first);

        processor.error(&io::Error::other("new disk failure while paused"));
        let failed = processor.status();
        assert_eq!(failed["last_error"], "new disk failure while paused");
        assert_eq!(failed["write_errors"], 1);
        assert_eq!(failed["p50_ns"], first["p50_ns"]);

        processor.stats.callback.record_ns(50);
        processor.stats.hours += 1;
        processor.stats.total_ns = 50;
        let resumed = processor.status();
        assert_eq!(resumed["hours"], 4);
        assert_eq!(resumed["total_ns"], 50);
        assert_eq!(resumed["p50_ns"], 100);
        assert_eq!(resumed["p95_ns"], 300);
        assert_eq!(resumed["max_ns"], 300);
        assert_eq!(resumed["last_error"], failed["last_error"]);
    }

    #[test]
    fn status_reports_pilot_copy_ns_additively() {
        let mut processor = Processor {
            root: PathBuf::new(),
            history: PreparedCampaigns::default(),
            session: session(),
            campaigns: BTreeMap::new(),
            stats: Statistics::default(),
            unknown_pending: 0,
            suppress_origin: false,
        };
        processor.stats.table_ns[replay::rows::TableId::ResearchTechnologies.index()] = 11;
        processor.stats.table_ns[replay::rows::TableId::ResearchSlots.index()] = 22;
        processor.stats.table_ns[replay::rows::TableId::TechnologyDefinitions.index()] = 33;
        processor.stats.table_ns[replay::rows::TableId::LandCombats.index()] = 101;
        processor.stats.table_ns[replay::rows::TableId::WingEquipment.index()] = 201;
        processor.stats.table_ns[replay::rows::TableId::LandTheatres.index()] = 301;
        processor.stats.table_ns[replay::rows::TableId::Players.index()] = 401;
        processor.stats.table_ns[replay::rows::TableId::WorldTension.index()] = 501;
        // war_matrix dual-homing: owned walk in table_ns, matrix census in
        // the dedicated stat (sampling.rs copy ticks, spooling mirror,
        // stats.war_matrix_ns). Both advance together.
        processor.stats.table_ns[replay::rows::TableId::WarRelations.index()] = 601;
        processor.stats.war_matrix_ns = 602;
        let status = processor.status();
        assert_eq!(status["research_copy_ns"], json!([11, 22, 33]));
        assert_eq!(status["wars_copy_ns"], json!([601, 602]));
        // New game-thread copy-only mirrors for the coupled unit, air,
        // naval, players and session families.
        assert_eq!(status["map_copy_ns"][0], json!(101));
        assert_eq!(status["map_copy_ns"].as_array().unwrap().len(), 8);
        assert_eq!(status["air_wing_copy_ns"], json!([201, 0]));
        assert_eq!(status["naval_copy_ns"][0], json!(301));
        assert_eq!(status["naval_copy_ns"].as_array().unwrap().len(), 9);
        assert_eq!(status["players_copy_ns"], json!([401, 0]));
        assert_eq!(status["session_copy_ns"], json!([501, 0, 0]));
        // Existing keys survive the additive merge.
        assert!(status["table_ns"].is_array());
        assert_eq!(status["hours"], 0);
    }

    use engine::msvc_string::Text;
    use replay::{
        branches::{BranchReason, Parentage},
        identity::{Identity, Verification},
        spool::{SegmentSink, recover_prefix},
    };
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

    fn clock(hour: i32) -> Clock {
        Clock {
            game_state: 4096,
            hour,
            play_time: u64::MAX,
            play_session_started_at: 1234,
            seed: i32::MIN,
            campaign_id: text(ID),
            speed: 5,
            tick_busy: 0,
        }
    }

    fn captured(hour: i32) -> Captured {
        let mut captured = Captured::new();
        captured.clock_entry = Some(clock(hour - 1));
        captured.clock_before = Some(clock(hour));
        captured.clock_after = Some(clock(hour));
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
        captured.map.fleets.outcome = Outcome::Rows { len: 0 };
        captured.map.task_forces.outcome = Outcome::Rows { len: 0 };
        captured.map.ships.outcome = Outcome::Rows { len: 0 };
        captured.map.naval_combats.outcome = Outcome::Rows { len: 0 };
        captured.map.naval_combat_participants.outcome = Outcome::Rows { len: 0 };
        captured.map.air_wings.outcome = Outcome::Rows { len: 0 };
        captured.map.army_orders.outcome = Outcome::Rows { len: 0 };
        captured.consistency_checked.fill(true);
        captured
    }

    #[test]
    fn conversion_keeps_native_values_aliases_and_raw_text() {
        let mut captured = captured(10);
        captured.countries[0] = engine::ccountry::Row {
            id: 7,
            canonical_slot: 1,
            tag: text(b"GER"),
            name: text(&[0xff, b'A']),
            political_power_raw: None,
            command_power_raw: i64::MIN,
            stability_raw: 100_001,
            war_support_raw: -9_007_199_254_740_993,
            convoys_raw: Some(i64::MAX),
        };
        captured.countries_result = Outcome::Rows { len: 1 };
        captured.links[0] = engine::ccountry::LinkRow {
            id: 2,
            canonical_slot: 1,
        };
        captured.links_result = Outcome::Rows { len: 1 };
        captured.provinces[0] = engine::cprovince::Row {
            id: 6,
            state: 4,
            owner: 7,
            controller: 2,
            victory_points: i32::MIN,
        };
        captured.provinces_result = Outcome::Rows { len: 1 };
        captured.states[0] = engine::cstate::Row {
            id: 4,
            owner: 7,
            controller: u32::MAX,
        };
        captured.states_result = Outcome::Rows { len: 1 };
        // Equal numeric ids in different native type namespaces remain separate
        // rows. Their roster order is intentionally opposite to wire key order.
        captured.divisions[0] = engine::carmy::Row {
            id: (u64::from(u32::MAX) << 32) | 4713,
            roster_owner: 7,
            logical_owner: 2,
            expeditionary_owner: 0,
            province: 6,
            template: u64::MAX,
            hit_points: i64::MIN,
            organisation: 9_007_199_254_740_993,
            orders_group: (57_u64 << 32) | 53,
        };
        captured.divisions[1] = engine::carmy::Row {
            id: (u64::from(u32::MAX) << 32) | 51,
            ..captured.divisions[0]
        };
        captured.divisions_result = Outcome::Rows { len: 2 };
        let frame = decoded(&captured, clock(10), 1);
        assert_eq!(
            frame.tables[2],
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 7,
                canonical_slot: 1,
                tag: b"GER".to_vec(),
                name: vec![0xff, b'A'],
                political_power_raw: None,
                command_power_raw: i64::MIN,
                stability_raw: 100_001,
                war_support_raw: -9_007_199_254_740_993,
                convoys_raw: Some(i64::MAX),
            }]))
        );
        let TableView::Complete(divisions) = &frame.tables[TableId::Divisions.index()] else {
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
            assert_eq!(division.hit_points, i64::MIN);
            assert_eq!(division.organisation, 9_007_199_254_740_993);
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
            }]))
        );
    }

    #[test]
    fn failures_remain_local_and_clock_change_invalidates_the_whole_walk() {
        let mut captured = captured(10);
        captured.countries_result = Outcome::Unavailable {
            reason: "country pointer unreadable",
        };
        captured.links_result = Outcome::Truncated {
            len: 0,
            unread_rows: 9,
        };
        captured.states_mismatches = 1;
        captured.divisions_mismatches = 1;
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
        captured.provinces_mismatches = 1;
        assert_eq!(
            decoded(&captured, clock(10), 1).tables[4],
            unavailable("provinces changed during double read")
        );
        captured.consistency_checked[crate::sampling::PROVINCE_CHECK] = false;
        assert_eq!(
            decoded(&captured, clock(10), 1).tables[4],
            TableView::Complete(TableRows::Provinces(Vec::new()))
        );
        assert_eq!(
            frame.tables[TableId::States.index()],
            unavailable("states changed during double read")
        );
        assert_eq!(
            frame.tables[TableId::Divisions.index()],
            unavailable("divisions changed during double read")
        );
        captured.clock_after = Some(clock(11));
        assert!(
            decoded(&captured, clock(10), 1)
                .tables
                .iter()
                .all(|table| *table == unavailable("hour changed during read"))
        );
        captured.clock_after = None;
        assert!(
            decoded(&captured, clock(10), 1)
                .tables
                .iter()
                .all(|table| *table == unavailable("clock unreadable around table walk"))
        );
    }

    #[test]
    fn skipped_comparisons_ignore_stale_mismatches_but_keep_primary_failures() {
        let mut source = captured(10);
        source.links_mismatches = 1;
        source.states_mismatches = 1;
        source.divisions_mismatches = 1;
        source.wars_mismatches = 1;
        source.factions_mismatches = 1;
        source.faction_members_mismatches = 1;
        let checked = decoded(&source, clock(10), 1);
        for table in [
            TableId::CountryLinks,
            TableId::States,
            TableId::Divisions,
            TableId::WarRelations,
            TableId::Factions,
            TableId::FactionMembers,
        ] {
            assert!(matches!(
                checked.tables[table.index()],
                TableView::Unavailable { .. }
            ));
        }

        source.consistency_checked.fill(false);
        source.states_result = Outcome::Unavailable {
            reason: "primary state pointer unreadable",
        };
        source.divisions_result = Outcome::Truncated {
            len: 0,
            unread_rows: 3,
        };
        source.factions[0] = engine::cfaction::Row {
            id: 88,
            name_start: 0,
            name_len: 3,
        };
        source.factions_result = Outcome::Rows { len: 1 };
        source.faction_names[..3].copy_from_slice(&[b'N', 0, 255]);
        source.faction_names_len = 3;
        let skipped = decoded(&source, clock(10), 1);
        for table in [
            TableId::CountryLinks,
            TableId::WarRelations,
            TableId::FactionMembers,
        ] {
            assert_eq!(
                skipped.tables[table.index()],
                TableView::Complete(TableRows::empty(table))
            );
        }
        assert_eq!(
            skipped.tables[TableId::States.index()],
            unavailable("primary state pointer unreadable")
        );
        assert_eq!(
            skipped.tables[TableId::Divisions.index()],
            TableView::Truncated {
                rows: TableRows::Divisions(Vec::new()),
                missing: 3
            }
        );
        assert_eq!(
            skipped.tables[TableId::Factions.index()],
            TableView::Complete(TableRows::Factions(vec![FactionRow {
                id: 88,
                name: vec![b'N', 0, 255],
            }]))
        );
    }

    #[test]
    fn consistency_status_keeps_attach_successes_without_counting_skipped_mismatches() {
        let mut source = captured(10);
        source.consistency_successful[..6].copy_from_slice(&[199, 200, 197, 196, 195, 194]);
        source.links_mismatches = 1;
        source.states_mismatches = 2;
        source.divisions_mismatches = 3;
        source.wars_mismatches = 4;
        source.factions_mismatches = 5;
        source.faction_members_mismatches = 6;
        source.provinces_mismatches = 7;
        source.consistency_successful[crate::sampling::PROVINCE_CHECK] = 198;
        let mut stats = Statistics::default();
        stats.observe_consistency(&source);
        source.consistency_checked.fill(false);
        stats.observe_consistency(&source);
        assert_eq!(
            [
                stats.links_mismatches,
                stats.states_mismatches,
                stats.divisions_mismatches,
                stats.wars_mismatches,
                stats.factions_mismatches,
                stats.faction_members_mismatches
            ],
            [1, 2, 3, 4, 5, 6]
        );
        let status = stats.consistency_status();
        assert_eq!(status["mismatches"]["Provinces"], 7);
        assert_eq!(status["successful_checks"]["Provinces"], 198);
        assert_eq!(status["scope"], "this collector attach");
        assert_eq!(status["target_per_table"], CONSISTENCY_TARGET);
        for (table, count) in CONSISTENCY_TABLES
            .into_iter()
            .zip(source.consistency_successful)
        {
            assert_eq!(status["successful_checks"][table], count);
        }
        assert_eq!(
            status["successful_checks"].as_object().unwrap().len(),
            CONSISTENCY_TABLES.len() + crate::extended::TABLES.len()
        );
        for table in crate::extended::TABLES {
            assert_eq!(status["successful_checks"][format!("{table:?}")], 0);
        }
    }

    #[test]
    fn matrix_status_counts_full_stable_attempts_and_keeps_failed_attempt_costs() {
        let mut source = captured(10);
        source.war_matrix_result = Some(Outcome::Rows { len: 0 });
        source.war_matrix_ns = 100;
        let mut stats = Statistics::default();
        stats.observe_consistency(&source);
        assert_eq!(stats.war_matrix_checks, 1);

        source.clock_after = Some(clock(11));
        stats.observe_consistency(&source);
        source.clock_after = None;
        stats.observe_consistency(&source);
        source.clock_after = Some(clock(10));
        source.wars_mismatches = 1;
        stats.observe_consistency(&source);
        source.wars_mismatches = 0;
        source.wars_result = Outcome::Truncated {
            len: 0,
            unread_rows: 1,
        };
        stats.observe_consistency(&source);
        source.wars_result = Outcome::Rows { len: 0 };
        source.war_matrix_result = Some(Outcome::Truncated {
            len: 0,
            unread_rows: 1,
        });
        stats.observe_consistency(&source);
        source.war_matrix_result = Some(Outcome::Unavailable {
            reason: "matrix unreadable",
        });
        stats.observe_consistency(&source);
        source.war_matrix_result = None;
        stats.observe_consistency(&source);
        assert_eq!(stats.war_matrix_checks, 1);
        assert_eq!(stats.war_matrix_failures, 2);
        assert_eq!(stats.war_matrix_cost.json()["calls"], 7);
        assert_eq!(stats.war_matrix_cost.json()["total_ns"], 700);
    }

    #[test]
    fn changed_country_authentication_preserves_clock_map_and_independent_dictionary() {
        let mut source = captured(10);
        let before = decoded(&source, clock(10), 1);
        source.country_cache_error = Some("country authentication changed during capture");
        let failed = decoded(&source, clock(10), 1);
        for table in replay::rows::TableId::ALL {
            let index = table.index();
            if matches!(index, 0 | 1 | 4 | 5) || table == TableId::LexicalTokens {
                assert_eq!(failed.tables[index], before.tables[index]);
            } else {
                assert_eq!(
                    failed.tables[index],
                    unavailable("country authentication changed during capture")
                );
            }
        }
    }

    #[test]
    fn primary_only_hours_restore_failed_baselines_and_keep_new_values() {
        use replay::frame::{FrameDecoder, TableUpdate};

        let mut source = captured(10);
        source.states_mismatches = 1;
        source.factions_mismatches = 1;
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let failed = encoder
            .encode(decoded(&source, clock(10), 1), true)
            .unwrap();
        decoder.apply(&failed).unwrap();

        source.consistency_checked.fill(false);
        source.states_result = Outcome::Rows { len: 1 };
        source.states[0] = engine::cstate::Row {
            id: 4,
            owner: 7,
            controller: 7,
        };
        source.factions_result = Outcome::Rows { len: 1 };
        source.factions[0] = engine::cfaction::Row {
            id: 88,
            name_start: 0,
            name_len: 3,
        };
        source.faction_names_len = 3;
        for hour in [11, 12] {
            source.clock_before = Some(clock(hour));
            source.clock_after = Some(clock(hour));
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
        source.wars[0] = engine::cwar_relation::Row {
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
        source.factions[0] = engine::cfaction::Row {
            id: u64::MAX,
            name_start: 2,
            name_len: 4,
        };
        source.factions_result = Outcome::Rows { len: 1 };
        source.faction_names[..6].copy_from_slice(&[0, 0, b'A', 0, 255, b'B']);
        source.faction_names_len = 6;
        source.faction_members[0] = engine::cfaction_system::MemberRow {
            country: 9,
            faction: u64::MAX,
            ordinal: 0,
        };
        source.faction_members[1] = engine::cfaction_system::MemberRow {
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
                name: vec![b'A', 0, 255, b'B']
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
        let decoded = replay::frame::FrameDecoder::new().apply(&encoded).unwrap();
        assert_eq!(decoded.tables, frame.tables);
    }

    #[test]
    fn independent_census_compares_rows_without_assuming_native_iteration_order() {
        let mut source = captured(10);
        source.wars[0] = engine::cwar_relation::Row {
            first_country: 1,
            second_country: 2,
            ..Default::default()
        };
        source.wars[1] = engine::cwar_relation::Row {
            first_country: 1,
            second_country: 3,
            ..Default::default()
        };
        source.wars_result = Outcome::Rows { len: 2 };
        source.wars_matrix[0] = source.wars[1];
        source.wars_matrix[1] = source.wars[0];
        source.war_matrix_result = Some(Outcome::Rows { len: 2 });
        assert_eq!(war_matrix_issue(&source), None);
        source.wars_matrix[0].first_casualties = 1;
        let frame = decoded(&source, clock(10), 1);
        assert_eq!(
            frame.tables[TableId::WarRelations.index()],
            unavailable("war owned-vector and matrix censuses disagree")
        );
        assert_eq!(
            frame.tables[TableId::Factions.index()],
            TableView::Complete(TableRows::Factions(Vec::new()))
        );
        source.war_matrix_result = Some(Outcome::Unavailable {
            reason: "bad matrix pointer",
        });
        assert_eq!(
            war_matrix_issue(&source),
            Some("independent war matrix census unavailable")
        );
        source.war_matrix_result = None;
        assert_eq!(war_matrix_issue(&source), None);
    }

    #[test]
    fn faction_byte_ranges_and_double_reads_cannot_publish_partial_values_as_complete() {
        let mut source = captured(10);
        source.factions[0] = engine::cfaction::Row {
            id: 88,
            name_start: usize::MAX,
            name_len: 1,
        };
        source.factions_result = Outcome::Rows { len: 1 };
        assert_eq!(
            faction_table(&source),
            unavailable("faction name lies outside the captured byte arena")
        );
        source.factions_mismatches = 1;
        source.faction_members_mismatches = 1;
        let frame = decoded(&source, clock(10), 1);
        assert_eq!(
            frame.tables[TableId::Factions.index()],
            unavailable("factions changed during double read")
        );
        assert_eq!(
            frame.tables[TableId::FactionMembers.index()],
            unavailable("faction members changed during double read")
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
            self.last_tag = bytes[12];
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

    fn session() -> SessionStart {
        SessionStart {
            id: 1,
            process_id: 100,
            process_start_time: 1000,
            qpc_frequency: 10_000_000,
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
        let store = Rc::new(RefCell::new(Store::default()));
        let session = session();
        let mut tracker = BranchTracker::new();
        tracker.begin_session(session.id).unwrap();
        (
            Campaign {
                directory: PathBuf::new(),
                session,
                tracker,
                encoder: FrameEncoder::new(),
                spool: SegmentedSpool::new(MemoryBackend(store.clone())),
                disk_branch: None,
                last_written: None,
                pending: Vec::new(),
                staggered_cache: BTreeMap::new(),
                staggered_branch: None,
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
            campaign
                .tracker
                .recorded_hours(first)
                .unwrap()
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            vec![10]
        );
        campaign
            .lost(
                &LostObservation {
                    campaign_id: text(ID),
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
            campaigns: BTreeMap::new(),
            stats: Statistics::default(),
            unknown_pending: 0,
            suppress_origin: false,
        };
        let loss = LostObservation {
            campaign_id: text(b"../invalid"),
            count: 3,
            first_hour: 1,
            last_hour: 3,
            loaded_hour: Some(0),
            load_signal: Some(1),
        };
        assert!(processor.lost(&loss).is_err());
        let mut captured = captured(4);
        captured.clock_before.as_mut().unwrap().campaign_id = text(b"../invalid");
        captured.clock_after.as_mut().unwrap().campaign_id = text(b"../invalid");
        processor.prepare(&captured).unwrap();
        assert!(processor.frame(&captured).is_err());
        assert_eq!(processor.unknown_pending, 4);
        assert!(processor.suppress_origin);
        assert!(processor.campaigns.is_empty());
    }

    fn staggered_frame(branch: BranchId, hour: i32, skip: bool) -> DecodedFrame {
        use replay::rows::{
            CampaignRow, ClockRow, SunkShipsRow, TableId, TableRows, WeatherLimitsRow,
            WorldTensionRow,
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
        tables[TableId::WeatherLimits.index()] =
            TableView::Complete(TableRows::WeatherLimits(vec![WeatherLimitsRow::default()]));
        tables[TableId::WorldTension.index()] =
            TableView::Complete(TableRows::WorldTension(vec![WorldTensionRow::default()]));
        if skip {
            for table in TableId::ALL {
                if replay::cadence::is_staggered(table) {
                    tables[table.index()] = TableView::Unavailable {
                        reason: replay::cadence::SKIPPED_REASON.into(),
                    };
                }
            }
        }
        DecodedFrame {
            branch,
            hour,
            tables,
            metrics: Metrics::default(),
        }
    }

    #[test]
    fn staggered_off_hours_emit_unchanged_and_hold_last_refresh() {
        use replay::frame::{FrameDecoder, TableUpdate};
        use replay::rows::TableId;
        let (mut campaign, _) = campaign();
        let branch = 1;
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let mut first = staggered_frame(branch, 0, false);
        let skipped = campaign.apply_staggered_cache(&mut first);
        assert!(!skipped[TableId::Characters.index()]);
        let frame = encoder.encode(first.clone(), false).unwrap();
        assert!(matches!(
            frame.tables[TableId::Characters.index()],
            TableUpdate::Snapshot(_)
        ));
        assert_eq!(decoder.apply(&frame).unwrap(), first);
        let mut second = staggered_frame(branch, 1, true);
        let skipped = campaign.apply_staggered_cache(&mut second);
        assert!(skipped[TableId::Characters.index()]);
        assert!(!skipped[TableId::Clock.index()]);
        assert_eq!(second, first.clone_with_hour(1));
        let frame = encoder.encode(second.clone(), false).unwrap();
        assert_eq!(
            frame.tables[TableId::Characters.index()],
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
                &mut next.tables[replay::rows::TableId::Clock.index()]
            {
                rows[0].hour = hour;
            }
            next
        }
    }

    #[test]
    fn refresh_hour_decode_equals_hourly_baseline() {
        use replay::frame::FrameDecoder;
        use replay::rows::TableId;
        let branch = 7;
        let mut hourly_encoder = FrameEncoder::new();
        let mut hourly_decoder = FrameDecoder::new();
        let mut staggered_encoder = FrameEncoder::new();
        let mut staggered_decoder = FrameDecoder::new();
        let (mut campaign, _) = campaign();
        for hour in 0..8 {
            let hourly = staggered_frame(branch, hour, false);
            let hourly_frame = hourly_encoder.encode(hourly.clone(), false).unwrap();
            let hourly_decoded = hourly_decoder.apply(&hourly_frame).unwrap().clone();
            let mut staggered = staggered_frame(
                branch,
                hour,
                !replay::cadence::is_refresh(TableId::Characters, hour),
            );
            campaign.apply_staggered_cache(&mut staggered);
            let staggered_frame = staggered_encoder.encode(staggered.clone(), false).unwrap();
            let staggered_decoded = staggered_decoder.apply(&staggered_frame).unwrap().clone();
            if replay::cadence::is_refresh(TableId::Characters, hour) {
                assert_eq!(
                    hourly_decoded.tables, staggered_decoded.tables,
                    "hour {hour}"
                );
            } else {
                assert_eq!(
                    staggered_decoded.tables[TableId::Characters.index()],
                    hourly_decoded.tables[TableId::Characters.index()],
                    "static data holds at hour {hour}"
                );
            }
        }
    }

    #[test]
    fn branch_start_clears_staggered_cache() {
        use replay::rows::TableId;
        let (mut campaign, _) = campaign();
        let mut first = staggered_frame(1, 0, false);
        campaign.apply_staggered_cache(&mut first);
        let mut second = staggered_frame(2, 1, true);
        let skipped = campaign.apply_staggered_cache(&mut second);
        assert!(skipped[TableId::Characters.index()]);
        assert!(
            matches!(
                &second.tables[TableId::Characters.index()],
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
        use replay::frame::{FrameDecoder, TableUpdate};
        use replay::rows::TableId;
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
        first.tables[TableId::WorldTension.index()] = TableView::Truncated {
            rows: TableRows::WorldTension(vec![replay::rows::WorldTensionRow::default()]),
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
