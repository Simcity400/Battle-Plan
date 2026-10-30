//! A campaign's columnar recording, written live beside its row spool from the frames the row
//! encoder takes. It is best effort while the rows remain the recording: its first failure
//! stops it with the reason, and the rows go on. Its allocations are advisory, so a failed one
//! stops only it, and an hour reaches the file only once its rows' write completed.

use replay::{
    columnar_file::{
        CAMPAIGN_FILE as FINISHED, Capture, Closed, FileWriter, Hour, Position, RestartState,
        Settings, Stop,
    },
    columnar_frames::schema,
    native_frame::{self, FrameHour, FrameWriter, RowFrame},
};
use replay_columnar::{envelope, manifest::Status};
use replay_data::{
    identity::{IdentityCodec, SessionStart},
    rows::{TABLE_COUNT, TableId},
};
use replay_frame::frame::FrameInput;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Instant,
};

#[cfg(test)]
mod tests;

/// The file while it is written; finishing renames it to [`FINISHED`], which the recorder
/// moves beside the row recording it finalizes.
const PARTIAL: &str = "columnar.h4col.partial";

/// Each finished campaign's counters, which the spool status keeps reporting after detach.
/// Finishing only moves them here; the status thread formats them.
static FINISHED_COUNTERS: Mutex<Vec<Counters>> = Mutex::new(Vec::new());

/// The statuses of the campaigns whose recordings finished.
pub(crate) fn finished_status() -> Vec<Value> {
    FINISHED_COUNTERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .map(|counters| counters.status(&[], &[], 0, true))
        .collect()
}

/// A campaign's columnar recording; nothing is held while it is off.
pub(crate) struct Columnar(Option<Box<Live>>);

/// The spool thread and its conversion helpers, which prepare and commit a columnar hour's
/// tables. A helper takes tables only between the game's ticks, and the spool thread
/// finishes the hour, so it never waits out a tick for a helper.
pub(crate) struct Lanes<'a> {
    helpers: Option<crate::spool_helper::Helpers<'a>>,
    spool: std::thread::ThreadId,
}

impl<'a> Lanes<'a> {
    /// The calling spool thread's lanes.
    pub(crate) fn new(helpers: Option<crate::spool_helper::Helpers<'a>>) -> Self {
        Self {
            helpers,
            spool: std::thread::current().id(),
        }
    }
}

impl native_frame::Lanes for Lanes<'_> {
    fn count(&self) -> usize {
        self.helpers
            .map_or(1, |helpers| 2 + usize::from(helpers.second.is_some()))
    }

    fn run(&self, job: &(dyn Fn() + Sync)) {
        let Some(helpers) = self.helpers else {
            return job();
        };
        let (phase, context) = (crate::allocation::phase(), crate::allocation::context());
        let helper = || {
            if crate::tick_gate::ticking() {
                return;
            }
            let _phase = crate::allocation::enter(phase);
            crate::allocation::set_context(context);
            let _span =
                crate::spool_phases::HELPER.span_now(crate::spool_phases::Phase::Columnar);
            engine_allocation::advisory(job);
        };
        helpers.join_each(&helper, job);
    }

    fn proceeds(&self) -> bool {
        std::thread::current().id() == self.spool || !crate::tick_gate::ticking()
    }
}

struct Live {
    directory: PathBuf,
    /// The file while it is written, and its finished name.
    partial: PathBuf,
    finished_path: PathBuf,
    /// The file's identity until the first hour opens it.
    identity: Option<envelope::Identity>,
    /// The capture the root names: this session alone, every run its own.
    capture: Option<Capture>,
    file: Option<FileWriter<File>>,
    run: Option<Run>,
    /// Committed hours whose rows' write has not completed, in order.
    pending: VecDeque<Pending>,
    /// The branch and hour of the last hour the file holds.
    last: Option<(u64, i32)>,
    /// Why the next run starts, when the rows said.
    next_epoch: Option<u8>,
    finished: bool,
    /// Runs started, which a run's first hour carries to the file.
    started: u64,
    counters: Counters,
}

/// What a recording measured, and why it stopped.
struct Counters {
    stopped: Option<Reason>,
    hours: u64,
    /// Runs the file holds.
    runs: u64,
    restarts: u64,
    /// Stored bytes of the units, restarts and pages written.
    stored: u64,
    /// Spool-thread wall in preparing and committing hours, and in closing positions.
    hour_ns: u64,
    close_ns: u64,
    close_max_ns: u64,
    /// Each table's frame-writer wall in runs that ended.
    ended_ns: [u64; TABLE_COUNT],
    /// Each table's hours prepared again in table order after a lane, in runs that ended.
    ended_retried: [u64; TABLE_COUNT],
}

/// A run's start as the file records it.
#[derive(Clone, Copy)]
struct RunStart {
    branch: u64,
    epoch: u8,
    first_hour: i32,
}

/// The run under way: one branch's continuous hours.
struct Run {
    frames: FrameWriter,
    branch: u64,
    next_hour: i32,
    /// Hours committed in the run.
    ordinal: u32,
    /// The run's start, until its first hour carries it to the file.
    start: Option<RunStart>,
}

/// A committed hour as the file takes it once its rows' write completed: the run it starts,
/// when it starts one, and the restart a position it fills carries.
struct Pending {
    starts: Option<RunStart>,
    branch: u64,
    hour: i32,
    manifest: Vec<u8>,
    sections: Vec<(usize, Vec<u8>)>,
    statuses: Vec<Status>,
    read: Vec<u64>,
    restart: Option<(Vec<Box<[u8]>>, Vec<RestartState>)>,
}

impl Columnar {
    /// The campaign's columnar recording in `directory`, opened at its first hour; off when
    /// it cannot be held.
    pub(crate) fn open(directory: &Path, campaign: &str, session: &SessionStart) -> Self {
        let live = engine_allocation::advisory(|| -> io::Result<Box<Live>> {
            let (identity, capture) = identity(campaign, session)?;
            engine_allocation::boxed(Live {
                directory: owned_path(directory, None)?,
                partial: owned_path(directory, Some(PARTIAL))?,
                finished_path: owned_path(directory, Some(FINISHED))?,
                identity: Some(identity),
                capture: Some(capture),
                file: None,
                run: None,
                pending: VecDeque::new(),
                last: None,
                next_epoch: None,
                finished: false,
                started: 0,
                counters: Counters {
                    stopped: None,
                    hours: 0,
                    runs: 0,
                    restarts: 0,
                    stored: 0,
                    hour_ns: 0,
                    close_ns: 0,
                    close_max_ns: 0,
                    ended_ns: [0; TABLE_COUNT],
                    ended_retried: [0; TABLE_COUNT],
                },
            })
        });
        Self(live.ok())
    }

    /// A campaign that writes no columnar recording, as spool tests' campaigns.
    #[cfg(test)]
    pub(crate) const fn off() -> Self {
        Self(None)
    }

    /// Prepares `input`'s hour on `lanes`, starting a run at a branch change, a gap or a write
    /// failure, beside what the row encoder makes of it.
    pub(crate) fn prepare(
        &mut self,
        input: &FrameInput,
        row: RowFrame<'_>,
        lanes: &dyn native_frame::Lanes,
    ) -> Option<FrameHour> {
        let live = self.0.as_deref_mut()?;
        engine_allocation::advisory(|| live.prepare(input, row, lanes))
    }

    /// Commits a prepared hour on `lanes` once its rows' baseline committed. The file takes it
    /// now when the rows are written as they are encoded; a queued hour waits for
    /// [`Self::complete`].
    pub(crate) fn commit(
        &mut self,
        hour: FrameHour,
        queued: bool,
        lanes: &dyn native_frame::Lanes,
    ) {
        if let Some(live) = self.0.as_deref_mut() {
            engine_allocation::advisory(|| live.commit(hour, queued, lanes));
        }
    }

    /// The rows' write of `hour` of `branch` completed: the file takes the hours through it.
    pub(crate) fn complete(&mut self, branch: u64, hour: i32) {
        if let Some(live) = self.0.as_deref_mut() {
            engine_allocation::advisory(|| live.complete(branch, hour));
        }
    }

    /// The rows' write failed: hours not yet written are dropped, and the next starts a run.
    pub(crate) fn interrupt(&mut self) {
        if let Some(live) = self.0.as_deref_mut() {
            live.interrupt();
        }
    }

    /// Ends the recording on its healthy path: the root naming the capture (and where writing
    /// stopped, when it did) and the footer, durable under its finished name. A file that
    /// holds no hour, or cannot finish, is removed.
    pub(crate) fn finish(&mut self) {
        if let Some(live) = self.0.as_deref_mut() {
            engine_allocation::advisory(|| live.finish());
        }
    }

    /// None while columnar recording is off.
    pub(crate) fn status(&self) -> Option<Value> {
        self.0.as_deref().map(Live::status)
    }
}

impl Live {
    fn writing(&self) -> bool {
        self.counters.stopped.is_none() && !self.finished
    }

    /// Ends the run under way, keeping its tables' wall.
    fn end_run(&mut self) {
        if let Some(run) = self.run.take() {
            for (total, ns) in self.counters.ended_ns.iter_mut().zip(run.frames.table_ns()) {
                *total += ns;
            }
            for (total, hours) in self.counters.ended_retried.iter_mut().zip(run.frames.retried()) {
                *total += hours;
            }
        }
    }

    fn prepare(
        &mut self,
        input: &FrameInput,
        row: RowFrame<'_>,
        lanes: &dyn native_frame::Lanes,
    ) -> Option<FrameHour> {
        if !self.writing() {
            return None;
        }
        let started = Instant::now();
        let prepared = self.try_prepare(input, row, lanes);
        self.counters.hour_ns += started.elapsed().as_nanos() as u64;
        prepared.map_err(|error| self.stop(&error)).ok()
    }

    fn try_prepare(
        &mut self,
        input: &FrameInput,
        row: RowFrame<'_>,
        lanes: &dyn native_frame::Lanes,
    ) -> io::Result<FrameHour> {
        let continues = self
            .run
            .as_ref()
            .is_some_and(|run| run.branch == input.branch && run.next_hour == input.hour);
        if !continues {
            let epoch = match &self.run {
                _ if self.started == 0 => envelope::epoch::START,
                Some(run) if run.branch != input.branch => envelope::epoch::BRANCH,
                _ => self.next_epoch.take().unwrap_or(envelope::epoch::GAP),
            };
            self.end_run();
            self.run = Some(Run {
                frames: FrameWriter::new()?,
                branch: input.branch,
                next_hour: input.hour,
                ordinal: 0,
                start: Some(RunStart {
                    branch: input.branch,
                    epoch,
                    first_hour: input.hour,
                }),
            });
            self.started += 1;
        }
        self.run
            .as_mut()
            .expect("a run under way")
            .frames
            .hour_on(input, &[], Some(row), lanes)
    }

    fn commit(&mut self, hour: FrameHour, queued: bool, lanes: &dyn native_frame::Lanes) {
        if !self.writing() {
            return;
        }
        let started = Instant::now();
        let committed = self.try_commit(hour, queued, lanes);
        self.counters.hour_ns += started.elapsed().as_nanos() as u64;
        if let Err(error) = committed {
            self.stop(&error);
        }
    }

    fn try_commit(
        &mut self,
        hour: FrameHour,
        queued: bool,
        lanes: &dyn native_frame::Lanes,
    ) -> io::Result<()> {
        let run = self
            .run
            .as_mut()
            .ok_or_else(|| invalid("A columnar hour commits within its run"))?;
        let next_hour = hour
            .hour()
            .checked_add(1)
            .ok_or_else(|| invalid("Hours pass 2^31"))?;
        let mut sections = engine_allocation::vec(TABLE_COUNT)?;
        for (table, section) in hour.sections() {
            sections.push((table, engine_allocation::copy_vec(section)?));
        }
        let mut pending = Pending {
            starts: run.start,
            branch: run.branch,
            hour: hour.hour(),
            manifest: engine_allocation::copy_vec(&hour.manifest)?,
            sections,
            statuses: engine_allocation::copy_vec(hour.statuses())?,
            read: engine_allocation::copy_vec(hour.read())?,
            restart: None,
        };
        run.frames.commit_on(hour, lanes)?;
        run.start = None;
        run.next_hour = next_hour;
        run.ordinal += 1;
        self.counters.hours += 1;
        let units = run.ordinal / envelope::UNIT_ORDINALS;
        if run.ordinal % envelope::UNIT_ORDINALS == 0
            && units % Settings::RECORDING.restart_units == 0
        {
            pending.restart = Some(run.frames.restart()?);
        }
        if queued {
            self.pending
                .try_reserve(1)
                .map_err(|_| engine_allocation::oom())?;
            self.pending.push_back(pending);
            Ok(())
        } else {
            self.write(pending)
        }
    }

    fn complete(&mut self, branch: u64, hour: i32) {
        // The rows complete queued hours in order; an hour the writer never queued is none.
        while self
            .pending
            .front()
            .is_some_and(|front| front.branch == branch && front.hour <= hour)
        {
            let pending = self.pending.pop_front().expect("a pending hour");
            if self.finished {
                continue;
            }
            if let Err(error) = self.write(pending) {
                self.stop(&error);
            }
        }
    }

    /// The file takes a committed hour, after the run it starts, closing a full unit position.
    fn write(&mut self, pending: Pending) -> io::Result<()> {
        if let Some(start) = pending.starts {
            self.file()?
                .start_run(start.branch, start.epoch, start.first_hour)?;
            self.counters.runs += 1;
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| invalid("A columnar hour is written to its file"))?;
        let sections = engine_allocation::collect_vec(
            pending
                .sections
                .iter()
                .map(|(table, section)| (*table, section.as_slice())),
        )?;
        let position = file.hour(Hour {
            hour: pending.hour,
            manifest: &pending.manifest,
            sections: &sections,
            statuses: &pending.statuses,
            read: &pending.read,
        })?;
        self.last = Some((pending.branch, pending.hour));
        let Position::Full { restart } = position else {
            return Ok(());
        };
        // Compressing a unit is the heaviest columnar work; it starts outside the game's tick.
        crate::tick_gate::wait();
        let started = Instant::now();
        let closed = match (restart, &pending.restart) {
            (true, Some((strings, tables))) => file.close_with_restart(strings.as_slice(), tables)?,
            (true, None) => return Err(invalid("A due restart was not taken at its hour")),
            (false, _) => file.close_position()?,
        };
        let elapsed = started.elapsed().as_nanos() as u64;
        self.counters.close_ns += elapsed;
        self.counters.close_max_ns = self.counters.close_max_ns.max(elapsed);
        self.account(&closed);
        Ok(())
    }

    fn interrupt(&mut self) {
        self.pending.clear();
        self.end_run();
        self.next_epoch = Some(envelope::epoch::WRITE_FAILURE);
    }

    fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        // Detach drains the rows' writes first; an hour still waiting was never written.
        self.pending.clear();
        self.end_run();
        let Some(file) = self.file.take() else {
            return;
        };
        let (Some(mut capture), Some((branch, last_hour))) = (self.capture.take(), self.last)
        else {
            drop(file);
            // A file that holds no hour is removed only on the healthy path; after a failure it
            // stays partial, which nothing publishes, since removing it would allocate.
            if self.counters.stopped.is_none() {
                let _ = fs::remove_file(&self.partial);
            }
            return;
        };
        let finished = (|| -> io::Result<()> {
            let runs =
                usize::try_from(self.counters.runs).map_err(|_| invalid("Runs pass the address space"))?;
            capture.run_sessions = engine_allocation::filled(runs, 0)?;
            capture.stop = match &self.counters.stopped {
                Some(reason) => Some(Stop {
                    branch,
                    last_hour: Some(last_hour),
                    reason: engine_allocation::string(reason.text())?,
                }),
                None => None,
            };
            let finished = file.finish_with_capture(&capture)?;
            self.counters.stored += finished.index_bytes;
            finished.file.sync_all()?;
            drop(finished.file);
            fs::rename(&self.partial, &self.finished_path)
        })();
        // A file that could not finish stays partial, which nothing publishes: removing it
        // would allocate while memory may be exhausted.
        if let Err(error) = finished {
            self.stop(&error);
        }
        // Diagnostics only: kept when a slot can be reserved, without formatting here.
        let mut kept = FINISHED_COUNTERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if kept.try_reserve(1).is_ok() {
            kept.push(std::mem::replace(&mut self.counters, Counters::new()));
        }
    }

    fn status(&self) -> Value {
        let (current, retried) = self
            .run
            .as_ref()
            .map_or((&[][..], &[][..]), |run| (run.frames.table_ns(), run.frames.retried()));
        self.counters.status(current, retried, self.pending.len(), self.finished)
    }

    fn file(&mut self) -> io::Result<&mut FileWriter<File>> {
        if self.file.is_none() {
            let identity = self
                .identity
                .take()
                .ok_or_else(|| invalid("A columnar file opens once"))?;
            let columns = engine_allocation::collect_vec(schema()?.iter().map(Vec::len))?;
            // The row spool creates the campaign's directory at its first segment, after this.
            fs::create_dir_all(&self.directory)?;
            let file = File::create(&self.partial)?;
            self.file = Some(FileWriter::create(
                file,
                identity,
                columns,
                Settings::RECORDING,
            )?);
        }
        Ok(self.file.as_mut().expect("opened"))
    }

    fn account(&mut self, closed: &Closed) {
        self.counters.stored += closed.units.iter().map(|unit| unit.stored).sum::<u64>();
        if let Some(restart) = &closed.restart {
            self.counters.restarts += 1;
            self.counters.stored += restart.chunk.stored
                + restart
                    .tables
                    .iter()
                    .map(|table| table.extent.stored)
                    .sum::<u64>();
        }
    }

    fn stop(&mut self, error: &io::Error) {
        self.end_run();
        if self.counters.stopped.is_none() {
            self.counters.stopped = Some(Reason::of(error));
        }
    }
}

/// The file's identity (this build's schema, the campaign, its content identity and the
/// session's capture mode) and the capture its root names: this session, as the row
/// recording encodes it.
fn identity(campaign: &str, session: &SessionStart) -> io::Result<(envelope::Identity, Capture)> {
    let mut content = Vec::new();
    session.identity.encode(&mut content)?;
    let mut encoded = Vec::new();
    session.encode(&mut encoded)?;
    let mut sessions = engine_allocation::vec(1)?;
    sessions.push(encoded);
    let capture = Capture {
        campaign_id: engine_allocation::copy_vec(campaign.as_bytes())?,
        content: engine_allocation::copy_vec(&content)?,
        capture_mode: session.capture_mode.byte(),
        sessions,
        run_sessions: Vec::new(),
        stop: None,
    };
    let identity = envelope::Identity {
        schema: envelope::schema_digest(&schema()?),
        campaign: engine_allocation::copy_vec(campaign.as_bytes())?,
        content,
        capture_mode: session.capture_mode.byte(),
    };
    Ok((identity, capture))
}

impl Counters {
    const fn new() -> Self {
        Self {
            stopped: None,
            hours: 0,
            runs: 0,
            restarts: 0,
            stored: 0,
            hour_ns: 0,
            close_ns: 0,
            close_max_ns: 0,
            ended_ns: [0; TABLE_COUNT],
            ended_retried: [0; TABLE_COUNT],
        }
    }

    /// The status, with the run under way's tables' wall added; formatted on the status
    /// thread.
    fn status(
        &self,
        current: &[u64],
        current_retried: &[u64],
        pending: usize,
        finished: bool,
    ) -> Value {
        let mut tables = self.ended_ns;
        for (total, ns) in tables.iter_mut().zip(current) {
            *total += ns;
        }
        let mut retried = self.ended_retried;
        for (total, hours) in retried.iter_mut().zip(current_retried) {
            *total += hours;
        }
        let mut most: Vec<usize> = (0..TABLE_COUNT).filter(|&table| retried[table] > 0).collect();
        most.sort_unstable_by_key(|&table| std::cmp::Reverse(retried[table]));
        let retried_tables: Vec<Value> = most
            .iter()
            .take(16)
            .map(|&table| {
                json!({"table": format!("{:?}", TableId::ALL[table]), "hours": retried[table]})
            })
            .collect();
        let mut order: Vec<usize> = (0..TABLE_COUNT).collect();
        order.sort_unstable_by_key(|&table| std::cmp::Reverse(tables[table]));
        let costliest: Vec<Value> = order
            .iter()
            .take(16)
            .filter(|&&table| tables[table] > 0)
            .map(|&table| json!({"table": format!("{:?}", TableId::ALL[table]), "ns": tables[table]}))
            .collect();
        json!({
            "costliest_tables": costliest,
            "retried_tables": retried_tables,
            "hours": self.hours,
            "pending": pending,
            "runs": self.runs,
            "restarts": self.restarts,
            "stored_bytes": self.stored,
            "hour_ns": self.hour_ns,
            "close_ns": self.close_ns,
            "close_max_ns": self.close_max_ns,
            "finished": finished,
            "stopped": self.stopped.as_ref().map(Reason::text),
            "scope": "cumulative over the campaign: hours committed, runs and restarts written, hours waiting for their rows' write; the sixteen tables with the most frame-writer wall, summed over the lanes; the sixteen tables whose lane hours were most often prepared again on the spool thread, for a string the run had not used or a failure; stored bytes of units, restarts, pages and, once finished, directories and the root (finish adds index raw bytes); spool-thread wall preparing and committing hours, and closing unit positions",
        })
    }
}

/// Why writing stopped: the error's text within a fixed reservation, or a fixed reason when
/// even that cannot be held.
enum Reason {
    Text(String),
    Unrecorded,
}

impl Reason {
    const BYTES: usize = 256;

    fn of(error: &io::Error) -> Self {
        let mut text = String::new();
        if text.try_reserve_exact(Self::BYTES).is_err() {
            return Self::Unrecorded;
        }
        // An OS error's own text is built by allocating; its code is not. The writer's own
        // errors carry a message that formats in place.
        let mut out = Bounded(&mut text);
        let kind = error.kind();
        let _ = match (error.raw_os_error(), error.get_ref()) {
            (Some(code), _) => std::fmt::Write::write_fmt(&mut out, format_args!("{kind:?} (OS error {code})")),
            (None, Some(inner)) => std::fmt::Write::write_fmt(&mut out, format_args!("{kind:?}: {inner}")),
            (None, None) => std::fmt::Write::write_fmt(&mut out, format_args!("{kind:?}")),
        };
        Self::Text(text)
    }

    fn text(&self) -> &str {
        match self {
            Self::Text(text) => text,
            Self::Unrecorded => "the columnar writer failed; its reason could not be held",
        }
    }
}

/// Writes into a string's reserved capacity and drops what would not fit, at a character
/// boundary, so formatting never allocates.
struct Bounded<'a>(&'a mut String);

impl std::fmt::Write for Bounded<'_> {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        let room = self.0.capacity() - self.0.len();
        let mut end = text.len().min(room);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.0.push_str(&text[..end]);
        Ok(())
    }
}

/// `directory`, or `name` in it, owned through a fallible reservation. The separator and
/// name are appended to the reserved string itself: `PathBuf::push` allocates for verbatim
/// paths, which canonical recorder output is.
fn owned_path(directory: &Path, name: Option<&str>) -> io::Result<PathBuf> {
    let base = directory.as_os_str();
    let separated = base
        .as_encoded_bytes()
        .last()
        .is_some_and(|&last| last == b'\\' || last == b'/');
    let extra = name.map_or(0, |name| name.len() + usize::from(!separated));
    let mut owned = std::ffi::OsString::new();
    owned
        .try_reserve_exact(base.len() + extra)
        .map_err(|_| engine_allocation::oom())?;
    owned.push(base);
    if let Some(name) = name {
        if !separated {
            owned.push(std::path::MAIN_SEPARATOR_STR);
        }
        owned.push(name);
    }
    Ok(PathBuf::from(owned))
}

/// Kind-only while a capture runs, whatever diagnostic scope encloses it: the live writer
/// must not allocate to fail.
fn invalid(message: &'static str) -> io::Error {
    if engine_allocation::capture_active() {
        io::Error::from(io::ErrorKind::InvalidData)
    } else {
        io::Error::new(io::ErrorKind::InvalidData, message)
    }
}
