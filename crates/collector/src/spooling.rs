//! Conversion, differencing and durable output. Nothing here runs on the game thread.

use crate::sampling::{Captured, LossBatch, LostObservation, qpc as current_qpc};
use engine::{cgame_state::Clock, walk::Outcome};
use replay::{
    BranchId,
    branches::{BranchStart, BranchTracker, Gap},
    codec::invalid,
    container::{Replay, SeekResult},
    frame::{DecodedFrame, FrameEncoder, Metrics, TABLE_COUNT, TableView},
    identity::SessionStart,
    rows::{
        CampaignRow, ClockRow, CountryLinkRow, CountryRow, DivisionRow, ProvinceRow, Row, StateRow,
        TableId,
    },
    spool::{FileBackend, Record, SegmentBackend, SegmentedSpool},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub(super) struct Distribution {
    samples_ns: Vec<u64>,
    total_ns: u64,
}

impl Distribution {
    pub fn record_ticks(&mut self, ticks: i64, frequency: u64) {
        let ns = ((ticks.max(0) as u128 * 1_000_000_000) / u128::from(frequency)) as u64;
        self.total_ns = self.total_ns.saturating_add(ns);
        self.samples_ns.push(ns);
    }

    pub fn json(&self) -> Value {
        let mut ordered = self.samples_ns.clone();
        ordered.sort_unstable();
        let percentile = |percent: usize| {
            if ordered.is_empty() {
                0
            } else {
                ordered[(ordered.len() * percent).div_ceil(100) - 1]
            }
        };
        json!({
            "calls": ordered.len(), "total_ns": self.total_ns,
            "mean_ns": if ordered.is_empty() { 0.0 } else { self.total_ns as f64 / ordered.len() as f64 },
            "p50_ns": percentile(50), "p95_ns": percentile(95), "p99_ns": percentile(99),
            "max_ns": ordered.last().copied().unwrap_or(0),
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
}

impl Campaign {
    fn open(root: &Path, replay_dir: &Path, id: &str, session: &SessionStart) -> io::Result<Self> {
        let mut tracker = BranchTracker::new();
        let mut session = session.clone();
        let replay_path = replay_dir.join(format!("{id}.battleplan"));
        let previous = match fs::read(&replay_path) {
            Ok(bytes) => Some(Replay::open(bytes)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        if let Some(previous) = &previous {
            if previous.campaign_id != id.as_bytes() {
                return Err(invalid("existing replay has a different campaign id"));
            }
            if !previous.identity.same_content(&session.identity) {
                return Err(invalid("existing campaign replay has different content"));
            }
            for branch in &previous.branches {
                tracker
                    .seed_recorded_branch(branch.clone(), previous.recorded_hours(branch.id)?)?;
            }
            if let Some(old_session) = previous.sessions.iter().find(|old| old.id == session.id) {
                if old_session != &session {
                    return Err(invalid(
                        "same process session has conflicting collector or content metadata",
                    ));
                }
                session = old_session.clone();
                let branch = previous
                    .index
                    .iter()
                    .rev()
                    .find_map(|block| {
                        previous
                            .branches
                            .iter()
                            .find(|branch| {
                                branch.id == block.branch && branch.session == session.id
                            })
                            .map(|branch| (branch.id, block.last_hour))
                    })
                    .ok_or_else(|| {
                        invalid("existing process session has no recorded hour to resume")
                    })?;
                let signal = match previous.seek(branch.0, branch.1)? {
                    SeekResult::Frame(frame) => match &frame.tables[TableId::Clock.index()] {
                        TableView::Complete(rows) => rows.iter().find_map(|row| match row {
                            Row::Clock(clock) => Some(clock.play_session_started_at),
                            _ => None,
                        }),
                        _ => None,
                    },
                    SeekResult::Gap { .. } => None,
                }
                .or_else(|| {
                    previous
                        .controls
                        .iter()
                        .rev()
                        .find_map(|record| match record {
                            Record::ClockWitness {
                                session: witness_session,
                                hour,
                                load_signal,
                                ..
                            } if *witness_session == session.id && *hour == branch.1 => {
                                *load_signal
                            }
                            _ => None,
                        })
                });
                tracker.resume_session(session.id, branch.0, branch.1, signal)?;
            } else {
                tracker.begin_session(session.id)?;
            }
        } else {
            tracker.begin_session(session.id)?;
        }
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
        Ok(Self {
            directory,
            session,
            tracker,
            encoder: FrameEncoder::new(),
            spool,
            disk_branch: None,
            last_written: None,
            pending: Vec::new(),
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
        source: &DecodedFrame,
        qpc: i64,
        signal: Option<i64>,
        unknown: u64,
        work: &mut WorkSample,
    ) -> io::Result<()> {
        let result = self.write_inner(source, qpc, signal, unknown, work);
        if let Err(error) = &result {
            // Codec/validation errors are failures too. Closing also prevents a
            // partially published context from being reused after such an error.
            let close_started = current_qpc();
            let _ = self.spool.finish_segment();
            *work.record_writes.get_or_insert(0) += current_qpc() - close_started;
            self.disk_branch = None;
            self.encoder = FrameEncoder::new();
            self.gap(
                source.branch,
                source.hour,
                source.hour,
                format!("spool write failed: {error}"),
            );
        }
        result
    }

    fn write_inner(
        &mut self,
        source: &DecodedFrame,
        qpc: i64,
        signal: Option<i64>,
        unknown: u64,
        work: &mut WorkSample,
    ) -> io::Result<()> {
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
                context.push(Record::GapSummary {
                    session: self.session.id,
                    dropped_frames: unknown,
                    reason: "collector lost clock metadata; campaign and hour range are unknown"
                        .into(),
                });
            }
            context.push(Record::ClockWitness {
                session: self.session.id,
                qpc: u64::try_from(qpc).map_err(|_| invalid("negative callback QPC"))?,
                hour: source.hour,
                load_signal: signal,
            });
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
                self.tracker.record_hour(source.branch, source.hour)?;
                self.disk_branch = Some(source.branch);
                self.last_written = Some(source.hour);
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
            self.spool.append(&Record::GapSummary {
                session: self.session.id,
                dropped_frames: unknown,
                reason: "collector lost clock metadata; campaign and hour range are unknown".into(),
            })?;
        }
        self.spool.append(&Record::Detach {
            session: self.session.id,
            last_hour: self.last_written,
        })?;
        self.pending.clear();
        self.spool.finish_segment()
    }
}

#[derive(Default)]
struct Statistics {
    hours: u64,
    campaign: Option<String>,
    last_hour: Option<i32>,
    total_ns: u64,
    samples: Vec<u64>,
    table_ns: [u64; TABLE_COUNT],
    dropped_frames: u64,
    truncated_tables: usize,
    links_mismatches: u64,
    states_mismatches: u64,
    divisions_mismatches: u64,
    pool_bytes: u64,
    previous: Option<(String, BranchId, i32, i64)>,
    interval_hours: u64,
    interval_qpc: u64,
    errors: u64,
    last_error: Option<String>,
    conversion: Distribution,
    frame_encoding: Distribution,
    record_writes: Distribution,
}

pub struct Processor {
    root: PathBuf,
    replay_dir: PathBuf,
    session: SessionStart,
    campaigns: BTreeMap<String, Campaign>,
    stats: Statistics,
    unknown_pending: u64,
    suppress_origin: bool,
}

impl Processor {
    pub fn new(root: &Path, replay_dir: &Path, session: SessionStart) -> io::Result<Self> {
        session.validate()?;
        fs::create_dir_all(root)?;
        Ok(Self {
            root: root.into(),
            replay_dir: replay_dir.into(),
            session,
            campaigns: BTreeMap::new(),
            stats: Statistics::default(),
            unknown_pending: 0,
            suppress_origin: false,
        })
    }

    fn campaign(&mut self, id: &str) -> io::Result<&mut Campaign> {
        if !self.campaigns.contains_key(id) {
            let campaign = Campaign::open(&self.root, &self.replay_dir, id, &self.session)?;
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
        let source = decoded(captured, clock, branch);
        let mut work = WorkSample {
            conversion: current_qpc() - conversion_started,
            ..WorkSample::default()
        };
        let result = campaign.write(
            &source,
            captured.qpc,
            Some(clock.play_session_started_at),
            unknown,
            &mut work,
        );
        let frequency = self.session.qpc_frequency;
        self.stats
            .conversion
            .record_ticks(work.conversion, frequency);
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
        stats.samples.push(captured.total_ns);
        stats.table_ns = captured.table_ns;
        stats.pool_bytes = captured.pool_bytes;
        stats.dropped_frames = captured.dropped_frames;
        stats.links_mismatches += captured.links_mismatches as u64;
        stats.states_mismatches += captured.states_mismatches as u64;
        stats.divisions_mismatches += captured.divisions_mismatches as u64;
        stats.truncated_tables = [
            captured.countries_result,
            captured.links_result,
            captured.provinces_result,
            captured.states_result,
            captured.divisions_result,
        ]
        .into_iter()
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
        let mut samples = stats.samples.clone();
        samples.sort_unstable();
        let percentile = |percent: usize| {
            if samples.is_empty() {
                0
            } else {
                samples[(samples.len() * percent).div_ceil(100) - 1]
            }
        };
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
        json!({
            "campaign": stats.campaign, "hours": stats.hours, "last_hour": stats.last_hour,
            "total_ns": stats.total_ns, "p50_ns": percentile(50), "p95_ns": percentile(95),
            "max_ns": samples.last().copied().unwrap_or(0), "table_ns": stats.table_ns,
            "hours_per_second": if stats.interval_qpc == 0 { 0.0 } else {
                stats.interval_hours as f64 * self.session.qpc_frequency as f64 / stats.interval_qpc as f64
            },
            "spool_bytes": spool_bytes, "dropped_frames": stats.dropped_frames,
            "truncated_tables": stats.truncated_tables, "links_mismatches": stats.links_mismatches,
            "states_mismatches": stats.states_mismatches,
            "divisions_mismatches": stats.divisions_mismatches,
            "pool_bytes": stats.pool_bytes, "write_errors": stats.errors, "last_error": stats.last_error,
            "unknown_losses_pending": self.unknown_pending,
            "spool_work": {
                "conversion": stats.conversion.json(),
                "frame_encoding": stats.frame_encoding.json(),
                "record_writes": stats.record_writes.json(),
                "row_enum_bytes": std::mem::size_of::<Row>(),
            },
        })
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

fn campaign_name(bytes: &[u8]) -> io::Result<String> {
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

fn unavailable(reason: &str) -> TableView {
    TableView::Unavailable {
        reason: reason.into(),
    }
}

fn table<T>(outcome: Outcome, source: &[T], convert: impl Fn(&T) -> Row) -> TableView {
    match outcome {
        Outcome::Unavailable { reason } => unavailable(reason),
        Outcome::Rows { len } | Outcome::Truncated { len, .. } => {
            let Some(source) = source.get(..len) else {
                return unavailable("walker row count exceeds allocated buffer");
            };
            let mut rows: Vec<_> = source.iter().map(convert).collect();
            rows.sort_unstable_by_key(Row::key);
            if rows.windows(2).any(|rows| rows[0].key() == rows[1].key()) {
                return unavailable("walker produced duplicate engine ids");
            }
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

fn decoded(captured: &Captured, clock: Clock, branch: BranchId) -> DecodedFrame {
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
    let tables = if let Some(reason) = failure {
        std::array::from_fn(|_| unavailable(reason))
    } else {
        [
            TableView::Complete(vec![Row::Clock(ClockRow {
                hour: clock.hour,
                play_time: clock.play_time,
                play_session_started_at: clock.play_session_started_at,
                seed: clock.seed,
            })]),
            TableView::Complete(vec![Row::Campaign(CampaignRow {
                id: clock.campaign_id.as_bytes().to_vec(),
            })]),
            table(captured.countries_result, &captured.countries, |country| {
                Row::Country(CountryRow {
                    id: country.id,
                    canonical_slot: country.canonical_slot,
                    tag: country.tag.as_bytes().to_vec(),
                    name: country.name.as_bytes().to_vec(),
                    political_power_raw: country.political_power_raw,
                    command_power_raw: country.command_power_raw,
                    stability_raw: country.stability_raw,
                    war_support_raw: country.war_support_raw,
                    convoys_raw: country.convoys_raw,
                })
            }),
            if captured.links_mismatches != 0 {
                unavailable("country links changed during double read")
            } else {
                table(captured.links_result, &captured.links, |link| {
                    Row::CountryLink(CountryLinkRow {
                        id: link.id,
                        canonical_slot: link.canonical_slot,
                    })
                })
            },
            table(captured.provinces_result, &captured.provinces, |province| {
                Row::Province(ProvinceRow {
                    id: province.id,
                    state: province.state,
                    owner: province.owner,
                    controller: province.controller,
                })
            }),
            if captured.states_mismatches != 0 {
                unavailable("states changed during double read")
            } else {
                table(captured.states_result, &captured.states, |state| {
                    Row::State(StateRow {
                        id: state.id,
                        owner: state.owner,
                        controller: state.controller,
                    })
                })
            },
            if captured.divisions_mismatches != 0 {
                unavailable("divisions changed during double read")
            } else {
                table(captured.divisions_result, &captured.divisions, |division| {
                    Row::Division(DivisionRow {
                        id: division.id,
                        roster_owner: division.roster_owner,
                        logical_owner: division.logical_owner,
                        expeditionary_owner: division.expeditionary_owner,
                        province: division.province,
                        template: division.template,
                        hit_points: division.hit_points,
                        organisation: division.organisation,
                    })
                })
            },
        ]
    };
    DecodedFrame {
        branch,
        hour: clock.hour,
        tables,
        metrics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        captured.divisions[1] = engine::carmy::Row {
            id: (u64::from(u32::MAX) << 32) | 51,
            ..captured.divisions[0]
        };
        captured.divisions_result = Outcome::Rows { len: 2 };
        let frame = decoded(&captured, clock(10), 1);
        assert_eq!(
            frame.tables[2],
            TableView::Complete(vec![Row::Country(CountryRow {
                id: 7,
                canonical_slot: 1,
                tag: b"GER".to_vec(),
                name: vec![0xff, b'A'],
                political_power_raw: None,
                command_power_raw: i64::MIN,
                stability_raw: 100_001,
                war_support_raw: -9_007_199_254_740_993,
                convoys_raw: Some(i64::MAX),
            })])
        );
        let TableView::Complete(divisions) = &frame.tables[TableId::Divisions.index()] else {
            panic!("distinct full division handles must survive conversion");
        };
        assert_eq!(
            divisions.iter().map(Row::key).collect::<Vec<_>>(),
            vec![captured.divisions[1].id, captured.divisions[0].id]
        );
        for row in divisions {
            let Row::Division(division) = row else {
                panic!()
            };
            assert_eq!(division.hit_points, i64::MIN);
            assert_eq!(division.organisation, 9_007_199_254_740_993);
            assert_eq!(division.template, u64::MAX);
            assert_eq!(division.expeditionary_owner, 0);
        }
        assert_eq!(
            frame.tables[3],
            TableView::Complete(vec![Row::CountryLink(CountryLinkRow {
                id: 2,
                canonical_slot: 1
            })])
        );
        assert_eq!(
            frame.tables[4],
            TableView::Complete(vec![Row::Province(ProvinceRow {
                id: 6,
                state: 4,
                owner: 7,
                controller: 2
            })])
        );
        let Row::Clock(value) = &match &frame.tables[0] {
            TableView::Complete(rows) => rows,
            _ => panic!(),
        }[0] else {
            panic!()
        };
        assert_eq!(value.play_time, u64::MAX);
        assert_eq!(
            frame.tables[TableId::States.index()],
            TableView::Complete(vec![Row::State(StateRow {
                id: 4,
                owner: 7,
                controller: u32::MAX,
            })])
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
                rows: Vec::new(),
                missing: 9
            }
        );
        assert_eq!(frame.tables[4], TableView::Complete(Vec::new()));
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
    fn table_conversion_rejects_duplicate_keys_and_path_validation_is_strict() {
        assert_eq!(
            table(Outcome::Rows { len: 2 }, &[1, 1], |id| Row::Province(
                ProvinceRow {
                    id: *id,
                    ..Default::default()
                }
            )),
            unavailable("walker produced duplicate engine ids")
        );
        assert_eq!(
            table(Outcome::Rows { len: 3 }, &[1], |id| Row::Province(
                ProvinceRow {
                    id: *id,
                    ..Default::default()
                }
            )),
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
            &source,
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
            Record::GapSummary { dropped_frames: 3, reason, .. } if reason.contains("hour range are unknown"))));
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
            replay_dir: PathBuf::new(),
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
}
