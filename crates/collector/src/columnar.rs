//! A campaign's columnar recording, handed to the recorder live beside its row spool from the
//! frames the row encoder takes: each hour's manifest and sections appended to the capture's
//! journal ([`replay::columnar_journal`]), uncompressed, for the recorder to write as final
//! blocks. It holds every table the rows leave to it, so an hour it cannot prepare or commit
//! fails, rows and all, and the next hour starts a run; a failure of its journal stops it with
//! the reason, and every later hour fails. Its allocations are advisory, so a failed one fails
//! only its hour, and an hour reaches the journal only once its rows' write completed. The
//! journal is written on its own thread ([`journal_thread`]): the spool thread queues, and an
//! hour the journal could not take is a gap of the next capture. Each hour's journal record
//! carries the clock witness the rows were given with it: the recorder reads the journal alone.

use replay::{
    columnar_file::Capture,
    columnar_frames::schema,
    columnar_journal::Witness,
    native_frame::{self, FrameHour, FrameWriter, RowFrame},
};
use replay_columnar::envelope;
use replay_data::{
    identity::{IdentityCodec, SessionStart},
    rows::{TABLE_COUNT, TableId},
};
use replay_frame::frame::FrameInput;
use serde_json::{Value, json};
use std::{
    collections::{HashSet, VecDeque},
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

mod journal_thread;
use journal_thread::{Ending, Failure, Lane, Op};
pub(crate) use journal_thread::JournalThread;
#[cfg(test)]
mod tests;

/// Each finished campaign's counters, which the spool status keeps reporting after detach.
/// Finishing only moves them here; the status thread formats them.
static FINISHED_COUNTERS: Mutex<Vec<Counters>> = Mutex::new(Vec::new());

/// The statuses of the campaigns whose recordings finished.
pub(crate) fn finished_status() -> Vec<Value> {
    FINISHED_COUNTERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .map(|counters| counters.status(&[], &[], 0, true, None, None))
        .collect()
}

/// A campaign's columnar recording.
pub(crate) enum Columnar {
    Live(Box<Live>),
    /// It could not be opened, so no hour can be recorded.
    Unopened,
    /// Spool tests' campaigns, whose rows are recorded alone.
    #[cfg(test)]
    Off,
}

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
        self.helpers.map_or(1, crate::spool_helper::Helpers::encoding_lanes)
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

pub(crate) struct Live {
    directory: PathBuf,
    /// The file's identity until the first hour or command opens the journal.
    identity: Option<envelope::Identity>,
    /// The capture the journal begins with: this session alone, every run its own.
    capture: Option<Capture>,
    /// What every capture of the session begins with, for one opened after a failure.
    template: (envelope::Identity, Capture),
    /// The branch starts, gaps and event losses the session's captures hold, which a capture
    /// opened after a failure holds again.
    facts: Vec<Vec<u8>>,
    /// After a failure: the hours to wait before a new capture opens, and the failures in a
    /// row.
    reopen: Option<Reopen>,
    /// The digests of the branch starts, gaps and event losses the capture holds.
    noted: HashSet<[u8; 16]>,
    /// The records the rows were given and the capture does not hold yet, in order: a queued
    /// frame's wait for its write.
    notes: VecDeque<Note>,
    /// The frame being given to the rows: its records, until it is written or queued.
    staged: Vec<Noted>,
    /// That frame's hour and clock witness, until its hour commits.
    witness: Option<(i32, Witness)>,
    /// Control records released and not yet in the journal, which opens at the first hour or
    /// command.
    controls: Vec<Vec<u8>>,
    /// The capture's journal was queued to open on the journal thread.
    opened: bool,
    /// The session's journal thread, shared by its recordings.
    thread: Arc<JournalThread>,
    /// This recording's journal on that thread.
    lane: Arc<Lane>,
    run: Option<Run>,
    /// Committed hours whose rows' write has not completed, in order.
    pending: VecDeque<Pending>,
    /// Why the next run starts, when the rows said.
    next_epoch: Option<u8>,
    finished: bool,
    command_branch: Option<u64>,
    /// Runs started, which a run's first hour carries to the journal.
    started: u64,
    counters: Counters,
}

/// A failed capture's wait for the next: hours left, and failures in a row, each doubling the
/// wait to at most a block.
struct Reopen {
    wait: u32,
    failures: u32,
}

/// Records the rows were given, which the capture takes once `awaits`, the branch and hour of
/// the queued frame they came with, is written; none once it was, or when they came alone.
struct Note {
    awaits: Option<(u64, i32)>,
    records: Vec<Noted>,
}

/// One record as the rows encode it.
struct Noted {
    bytes: Vec<u8>,
    /// A branch start, gap or event loss: one fact, which a context given again repeats.
    fact: bool,
}

/// What a recording measured, and why it last failed.
struct Counters {
    /// The last failure, where it happened and why.
    stopped: Option<Reason>,
    /// Captures that failed, each followed by a new one.
    failures: u64,
    /// Hours the rows wrote that a failed capture lost, each a gap of the next capture.
    lost: u64,
    hours: u64,
    /// Hours it could not take, which the rows record as gaps.
    failed: u64,
    /// Runs the journal holds.
    runs: u64,
    /// Bytes of manifests and sections given to the journal.
    journaled: u64,
    /// Spool-thread wall in preparing and committing hours.
    hour_ns: u64,
    /// The journal thread's operations queued and not yet done, the most ever queued, and its
    /// appends' total and longest wall, kept once the recording finished.
    journal: [u64; 4],
    /// Each table's frame-writer wall in runs that ended.
    ended_ns: [u64; TABLE_COUNT],
    /// Each table's hours prepared again in table order after a lane, in runs that ended.
    ended_retried: [u64; TABLE_COUNT],
    /// The journal's segment syncs, kept once the recording finished.
    durability: Option<journal_thread::Timing>,
}

/// A run's start as the journal records it.
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
    /// The run's start, until its first hour carries it to the journal.
    start: Option<RunStart>,
}

/// A committed hour as the journal takes it once its rows' write completed: the run it starts,
/// when it starts one, and its ordinal in its run.
struct Pending {
    starts: Option<RunStart>,
    branch: u64,
    hour: i32,
    ordinal: u32,
    /// The clock witness the rows were given with the hour.
    witness: Option<Witness>,
    manifest: Vec<u8>,
    sections: Vec<Vec<u8>>,
}

impl Columnar {
    /// The campaign's columnar recording in `directory`, opened at its first hour; off when
    /// it cannot be held.
    pub(crate) fn open(
        directory: &Path,
        campaign: &str,
        session: &SessionStart,
        thread: &Arc<JournalThread>,
    ) -> Self {
        let live = engine_allocation::advisory(|| -> io::Result<Box<Live>> {
            let (identity, capture) = identity(campaign, session)?;
            engine_allocation::boxed(Live {
                directory: owned_path(directory, None)?,
                template: (identity.clone(), capture.clone()),
                identity: Some(identity),
                capture: Some(capture),
                facts: Vec::new(),
                reopen: None,
                noted: HashSet::new(),
                notes: VecDeque::new(),
                staged: Vec::new(),
                witness: None,
                controls: Vec::new(),
                opened: false,
                thread: Arc::clone(thread),
                lane: Lane::new(),
                run: None,
                pending: VecDeque::new(),
                next_epoch: None,
                finished: false,
                command_branch: None,
                started: 0,
                counters: Counters::new(),
            })
        });
        match live {
            Ok(live) => Self::Live(live),
            Err(_) => Self::Unopened,
        }
    }

    /// A campaign that writes no columnar recording, as spool tests' campaigns.
    #[cfg(test)]
    pub(crate) const fn off() -> Self {
        Self::Off
    }

    /// Prepares `input`'s hour on `lanes`, starting a run at a branch change, a gap or a write
    /// failure, beside what the row encoder makes of it. An error fails the hour, rows and all.
    pub(crate) fn prepare(
        &mut self,
        input: &FrameInput,
        row: RowFrame<'_>,
        lanes: &dyn native_frame::Lanes,
    ) -> io::Result<Option<FrameHour>> {
        match self {
            Self::Live(live) => engine_allocation::advisory(|| live.prepare(input, row, lanes))
                .map(Some)
                .map_err(hour_failed),
            Self::Unopened => Err(invalid("The columnar recording could not be opened")),
            #[cfg(test)]
            Self::Off => Ok(None),
        }
    }

    /// Why this hour cannot be recorded, when it cannot: a failed capture's wait for the next,
    /// which this hour counts down, or a recording that could not be opened.
    pub(crate) fn halted(&mut self) -> Option<&str> {
        match self {
            Self::Live(live) => {
                engine_allocation::advisory(|| live.tick_reopen());
                live.halt_reason()
            }
            Self::Unopened => Some("the columnar recording could not be opened"),
            #[cfg(test)]
            Self::Off => None,
        }
    }

    /// Commits a prepared hour on `lanes` once its rows' baseline committed. The file takes it
    /// now when the rows are written as they are encoded; a queued hour waits for
    /// [`Self::complete`]. An error fails the hour, rows and all.
    pub(crate) fn commit(
        &mut self,
        hour: FrameHour,
        queued: bool,
        lanes: &dyn native_frame::Lanes,
    ) -> io::Result<()> {
        match self {
            Self::Live(live) => {
                engine_allocation::advisory(|| live.commit(hour, queued, lanes)).map_err(hour_failed)
            }
            _ => Err(invalid("A columnar hour commits within its recording")),
        }
    }

    /// The rows' write of `hour` of `branch` completed: the file takes the hours through it.
    pub(crate) fn complete(&mut self, branch: u64, hour: i32) {
        if let Self::Live(live) = self {
            engine_allocation::advisory(|| live.complete(branch, hour));
        }
    }

    /// The rows' spool takes `records`: the file's capture keeps each branch start, gap and
    /// control record but clock witnesses, in the order the rows were given them.
    pub(crate) fn note(&mut self, records: &[replay::spool::Record]) {
        if let Self::Live(live) = self {
            engine_allocation::advisory(|| live.note(records));
        }
    }

    /// [`Self::note`] of a frame's records, which the capture takes once the frame is written
    /// ([`Self::written`]) or its queued write completes ([`Self::queued`]).
    pub(crate) fn stage(&mut self, records: &[replay::spool::Record]) {
        if let Self::Live(live) = self {
            engine_allocation::advisory(|| live.stage(records));
        }
    }

    /// The frame staged was written.
    pub(crate) fn written(&mut self) {
        if let Self::Live(live) = self {
            engine_allocation::advisory(|| live.written());
        }
    }

    /// The frame staged, of `hour` of `branch`, was queued for the rows' writer.
    pub(crate) fn queued(&mut self, branch: u64, hour: i32) {
        if let Self::Live(live) = self {
            engine_allocation::advisory(|| live.queued(branch, hour));
        }
    }

    /// The rows' write failed: hours not yet written are dropped, and the next starts a run.
    pub(crate) fn interrupt(&mut self) {
        if let Self::Live(live) = self {
            engine_allocation::advisory(|| live.interrupt());
        }
    }

    /// Ends the recording on its healthy path: the journal's last record, naming where writing
    /// stopped when it did, durable. Records no journal took open one for them; a capture that
    /// holds no hour, command or other record has no journal.
    pub(crate) fn finish(&mut self) {
        if let Self::Live(live) = self {
            engine_allocation::advisory(|| live.finish());
        }
    }

    /// Waits until the journal thread wrote or refused everything queued for this recording.
    #[cfg(test)]
    pub(crate) fn settle(&self) {
        if let Self::Live(live) = self {
            live.thread.settle(&live.lane);
        }
    }

    /// None while columnar recording is off.
    pub(crate) fn status(&self) -> Option<Value> {
        match self {
            Self::Live(live) => Some(live.status()),
            Self::Unopened => Some(json!({"stopped": "the columnar recording could not be opened"})),
            #[cfg(test)]
            Self::Off => None,
        }
    }
}

impl Live {
    fn note(&mut self, records: &[replay::spool::Record]) {
        let command_branch = records.iter().find_map(|record| match record {
            replay::spool::Record::Control(replay::spool::ControlRecord::PlayerCommands {
                packet,
                ..
            }) => Some(packet.branch),
            _ => None,
        });
        let noted = Self::encoded(records).and_then(|records| {
            self.notes.try_reserve(1).map_err(|_| engine_allocation::oom())?;
            self.notes.push_back(Note {
                awaits: None,
                records,
            });
            if let Some(branch) = command_branch.filter(|_| self.writing()) {
                self.open_journal()?;
                self.command_branch = Some(branch);
            }
            self.release()
        });
        if let Err(error) = noted {
            self.stop("noting control records", &error);
        }
    }

    fn stage(&mut self, records: &[replay::spool::Record]) {
        self.witness = records.iter().find_map(|record| match record {
            replay::spool::Record::Control(replay::spool::ControlRecord::ClockWitness {
                qpc,
                hour,
                load_signal,
                ..
            }) => Some((
                *hour,
                Witness {
                    qpc: *qpc,
                    load_signal: *load_signal,
                },
            )),
            _ => None,
        });
        match Self::encoded(records) {
            Ok(records) => self.staged = records,
            Err(error) => {
                self.staged.clear();
                self.stop("encoding a frame's control records", &error);
            }
        }
    }

    fn written(&mut self) {
        self.give(None);
    }

    fn queued(&mut self, branch: u64, hour: i32) {
        self.give(Some((branch, hour)));
    }

    fn give(&mut self, awaits: Option<(u64, i32)>) {
        let records = std::mem::take(&mut self.staged);
        let given = (|| {
            self.notes.try_reserve(1).map_err(|_| engine_allocation::oom())?;
            self.notes.push_back(Note { awaits, records });
            self.release()
        })();
        if let Err(error) = given {
            self.stop("releasing a frame's control records", &error);
        }
    }

    /// The records of `records` the capture keeps, as the rows encode them: all but frames,
    /// session starts, which it holds already, and clock witnesses, which each hour's own
    /// record carries.
    fn encoded(records: &[replay::spool::Record]) -> io::Result<Vec<Noted>> {
        use replay::spool::{ControlRecord, Record};
        let mut kept = Vec::new();
        for record in records {
            if matches!(
                record,
                Record::Frame(_)
                    | Record::SessionStart(_)
                    | Record::Control(ControlRecord::ClockWitness { .. })
            ) {
                continue;
            }
            let bytes = replay::spool::encode_record(record)?;
            let fact = matches!(
                record,
                Record::BranchStart(_)
                    | Record::Gap(_)
                    | Record::Control(ControlRecord::EventLoss { .. })
            );
            engine_allocation::reserve(&mut kept, 1)?;
            kept.push(Noted { bytes, fact });
        }
        Ok(kept)
    }

    /// The capture takes the notes no queued write holds back, in order, into its journal once
    /// it is open. A fact it holds already is the same fact given again with a refused or
    /// failed write's context.
    fn release(&mut self) -> io::Result<()> {
        if self.finished {
            self.notes.clear();
            return Ok(());
        }
        while self.notes.front().is_some_and(|note| note.awaits.is_none()) {
            let note = self.notes.pop_front().expect("a note");
            for noted in note.records {
                if noted.fact {
                    engine_allocation::reserve_set(&mut self.noted, 1)?;
                    if !self.noted.insert(envelope::digest(&noted.bytes)) {
                        continue;
                    }
                    engine_allocation::reserve(&mut self.facts, 1)?;
                    self.facts.push(engine_allocation::copy_vec(&noted.bytes)?);
                }
                engine_allocation::reserve(&mut self.controls, 1)?;
                self.controls.push(noted.bytes);
            }
        }
        if self.opened && !self.controls.is_empty() {
            let controls = std::mem::take(&mut self.controls);
            self.thread.queue(&self.lane, Op::Controls(controls));
        }
        Ok(())
    }

    /// Takes in what the journal thread could not write: a failure stops the capture, its lost
    /// hours become the next capture's gap, and its control records go to the next capture
    /// ahead of those held now.
    fn poll(&mut self) {
        if let Some(failure) = self.lane.take_failure() {
            self.absorb(failure);
        }
    }

    fn absorb(&mut self, failure: Failure) {
        let Failure {
            failed,
            lost,
            mut controls,
        } = failure;
        if !controls.is_empty() {
            controls.append(&mut self.controls);
            self.controls = controls;
        }
        if let Some((site, error)) = failed {
            self.stop(site, &error);
        }
        let mut lost = lost.into_iter().peekable();
        while let Some((branch, first)) = lost.next() {
            let mut last = first;
            while let Some(&(next_branch, next)) = lost.peek() {
                if next_branch != branch || Some(next) != last.checked_add(1) {
                    break;
                }
                last = next;
                lost.next();
            }
            self.lose(branch, first, last);
        }
    }

    fn writing(&self) -> bool {
        self.reopen.is_none() && !self.finished
    }

    /// Why hours are not recorded now: a failed capture waits for the next, or the recording
    /// finished.
    fn halt_reason(&self) -> Option<&str> {
        if self.finished {
            return Some("the columnar recording finished");
        }
        self.reopen
            .as_ref()
            .map(|_| self.counters.stopped.as_ref().map_or("the columnar recording failed", Reason::text))
    }

    /// Counts down a failed capture's wait; once it ends, this hour opens a new capture,
    /// holding the session's facts again, and starts a run.
    fn tick_reopen(&mut self) {
        let Some(reopen) = self.reopen.as_mut() else {
            return;
        };
        if reopen.wait > 0 {
            reopen.wait -= 1;
            return;
        }
        if let Err(error) = self.reopened() {
            self.stop("opening a capture after a failure", &error);
        }
    }

    fn reopened(&mut self) -> io::Result<()> {
        let (identity, capture) = (self.template.0.clone(), self.template.1.clone());
        let mut controls = engine_allocation::vec(self.facts.len() + self.controls.len())?;
        for fact in &self.facts {
            controls.push(engine_allocation::copy_vec(fact)?);
        }
        controls.append(&mut self.controls);
        self.identity = Some(identity);
        self.capture = Some(capture);
        self.controls = controls;
        self.reopen = None;
        self.next_epoch = Some(envelope::epoch::WRITE_FAILURE);
        Ok(())
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
    ) -> io::Result<FrameHour> {
        self.poll();
        self.writable()?;
        let started = Instant::now();
        let prepared = self.try_prepare(input, row, lanes);
        self.counters.hour_ns += started.elapsed().as_nanos() as u64;
        if prepared.is_err() {
            self.fail_hour();
        }
        prepared
    }

    /// Ok while the file takes hours.
    fn writable(&self) -> io::Result<()> {
        if self.writing() {
            Ok(())
        } else {
            Err(invalid("The columnar recording stopped"))
        }
    }

    /// An hour it could not take: the rows record a gap, and the next hour starts a run.
    fn fail_hour(&mut self) {
        self.counters.failed += 1;
        self.end_run();
        self.next_epoch = Some(envelope::epoch::WRITE_FAILURE);
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

    fn commit(
        &mut self,
        hour: FrameHour,
        queued: bool,
        lanes: &dyn native_frame::Lanes,
    ) -> io::Result<()> {
        self.poll();
        self.writable()?;
        let started = Instant::now();
        let committed = self.try_commit(hour, queued, lanes);
        self.counters.hour_ns += started.elapsed().as_nanos() as u64;
        if committed.is_err() {
            self.fail_hour();
        }
        committed
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
        for (_, section) in hour.sections() {
            sections.push(engine_allocation::copy_vec(section)?);
        }
        let pending = Pending {
            starts: run.start,
            branch: run.branch,
            hour: hour.hour(),
            ordinal: run.ordinal,
            witness: self
                .witness
                .take()
                .filter(|(witnessed, _)| *witnessed == hour.hour())
                .map(|(_, witness)| witness),
            manifest: engine_allocation::copy_vec(&hour.manifest)?,
            sections,
        };
        run.frames.commit_on(hour, lanes)?;
        run.start = None;
        run.next_hour = next_hour;
        run.ordinal += 1;
        self.counters.hours += 1;
        if queued {
            self.pending
                .try_reserve(1)
                .map_err(|_| engine_allocation::oom())?;
            self.pending.push_back(pending);
            Ok(())
        } else {
            self.write(pending).map_err(|(site, error)| {
                self.stop(site, &error);
                error
            })
        }
    }

    fn complete(&mut self, branch: u64, hour: i32) {
        self.poll();
        for note in &mut self.notes {
            if note
                .awaits
                .is_some_and(|(held, at)| held == branch && at <= hour)
            {
                note.awaits = None;
            }
        }
        if let Err(error) = self.release() {
            self.stop("releasing control records", &error);
        }
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
            if self.reopen.is_some() {
                // Its capture failed: the rows hold the hour, the next capture its gap.
                self.lose(pending.branch, pending.hour, pending.hour);
                continue;
            }
            let (branch, hour) = (pending.branch, pending.hour);
            if let Err((site, error)) = self.write(pending) {
                self.stop(site, &error);
                self.lose(branch, hour, hour);
            }
        }
    }

    /// Queues a committed hour for the journal, after the run it starts; a block's last hour
    /// ends its segment. What the journal thread cannot write comes back through [`Self::poll`].
    fn write(&mut self, pending: Pending) -> Result<(), (&'static str, io::Error)> {
        if pending.starts.is_some() {
            self.open_journal()
                .map_err(|error| (journal_thread::OPENING, error))?;
            self.counters.runs += 1;
        } else if !self.opened {
            return Err((
                journal_thread::APPEND,
                invalid("A columnar hour is written to its journal"),
            ));
        }
        let bytes = pending.manifest.len()
            + pending.sections.iter().map(Vec::len).sum::<usize>();
        let hour = engine_allocation::boxed(journal_thread::Hour {
            starts: pending
                .starts
                .map(|start| (start.branch, start.epoch, start.first_hour)),
            branch: pending.branch,
            hour: pending.hour,
            ends_segment: (pending.ordinal + 1) % envelope::UNIT_ORDINALS == 0,
            witness: pending.witness,
            manifest: pending.manifest,
            sections: pending.sections,
        })
        .map_err(|error| (journal_thread::APPEND, error))?;
        self.thread.queue(&self.lane, Op::Hour(hour));
        self.counters.journaled += bytes as u64;
        Ok(())
    }

    /// Drops the records of frames the rows never wrote: a failed write's context is given
    /// again with a later frame.
    fn drop_unwritten(&mut self) {
        self.staged.clear();
        self.witness = None;
        self.notes.retain(|note| note.awaits.is_none());
        if let Err(error) = self.release() {
            self.stop("releasing control records", &error);
        }
    }

    fn interrupt(&mut self) {
        self.pending.clear();
        self.drop_unwritten();
        self.end_run();
        self.next_epoch = Some(envelope::epoch::WRITE_FAILURE);
    }

    fn finish(&mut self) {
        if self.finished {
            return;
        }
        // Detach drains the rows' writes first; an hour still waiting was never written.
        self.pending.clear();
        self.drop_unwritten();
        self.end_run();
        self.thread.settle(&self.lane);
        self.poll();
        self.finished = true;
        // Records no journal took, as a failed capture's lost hours or a capture of gaps alone,
        // end in a journal of their own: the recorder reads nothing else.
        if !self.opened && !self.controls.is_empty() {
            let reopened = if self.reopen.is_some() {
                self.reopened()
            } else {
                Ok(())
            };
            if let Err(error) = reopened.and_then(|()| self.open_journal()) {
                self.stop("opening a journal for the records no hour followed", &error);
            }
        }
        if self.opened {
            self.opened = false;
            let ending = match self.counters.stopped.as_ref().filter(|_| self.reopen.is_some()) {
                Some(reason) => engine_allocation::string(reason.text())
                    .map_or(Ending::Abandoned, |reason| Ending::Stopped {
                        reason,
                        command_branch: self.command_branch,
                    }),
                None => Ending::Finished,
            };
            self.thread.queue(&self.lane, Op::End(ending));
            self.thread.settle(&self.lane);
            // A journal that could not finish is finished by the recorder as its collector
            // left it.
            if let Some(Failure {
                failed: Some((site, error)),
                ..
            }) = self.lane.take_failure()
            {
                self.stop(site, &error);
            }
        }
        // Diagnostics only: kept when a slot can be reserved, without formatting here.
        let mut kept = FINISHED_COUNTERS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if kept.try_reserve(1).is_ok() {
            self.counters.durability = Some(self.thread.timing());
            self.counters.journal = self.lane.counters();
            kept.push(std::mem::replace(&mut self.counters, Counters::new()));
        }
    }

    fn status(&self) -> Value {
        let (current, retried) = self
            .run
            .as_ref()
            .map_or((&[][..], &[][..]), |run| (run.frames.table_ns(), run.frames.retried()));
        let durability = self.thread.timing();
        self.counters.status(
            current,
            retried,
            self.pending.len(),
            self.finished,
            Some(&durability),
            Some(self.lane.counters()),
        )
    }

    /// Queues the capture's journal to open, holding the control records released so far; the
    /// journal thread numbers it after every capture the directory holds.
    fn open_journal(&mut self) -> io::Result<()> {
        if self.opened {
            return Ok(());
        }
        let identity = self
            .identity
            .take()
            .ok_or_else(|| invalid("A columnar journal opens once"))?;
        let capture = self
            .capture
            .take()
            .ok_or_else(|| invalid("A columnar journal opens once"))?;
        let opening = engine_allocation::boxed(journal_thread::Opening {
            directory: owned_path(&self.directory, None)?,
            identity: identity.chain_bytes()?,
            capture,
            controls: std::mem::take(&mut self.controls),
        })?;
        self.thread.queue(&self.lane, Op::Open(opening));
        self.opened = true;
        Ok(())
    }

    /// A failure at `site`: the capture ends where its journal stands, its reason kept for the
    /// status, its journal's stop and the rows' gaps, and a new capture opens after a wait.
    fn stop(&mut self, site: &'static str, error: &io::Error) {
        self.end_run();
        self.counters.stopped = Some(Reason::of(site, error));
        if self.finished {
            return;
        }
        let failures = self.reopen.as_ref().map_or(0, |reopen| reopen.failures) + 1;
        self.counters.failures += 1;
        self.reopen = Some(Reopen {
            wait: (1_u32 << (failures.min(10) - 1)).min(envelope::UNIT_ORDINALS),
            failures,
        });
        if self.opened {
            self.opened = false;
            let reason = self.counters.stopped.as_ref().map_or("", Reason::text);
            // A journal that cannot take its stop is finished by the recorder, which then names
            // no reason of its own over the one the next capture's gap holds.
            let ending = engine_allocation::string(reason).map_or(Ending::Abandoned, |reason| {
                Ending::Stopped {
                    reason,
                    command_branch: self.command_branch,
                }
            });
            self.thread.queue(&self.lane, Op::End(ending));
        }
        // The stopped capture ends here: what its journal refused since is lost with it, so the
        // next capture starts with nothing of it outstanding.
        self.thread.settle(&self.lane);
        if let Some(Failure { lost, controls, .. }) = self.lane.take_failure() {
            self.absorb(Failure {
                failed: None,
                lost,
                controls,
            });
        }
    }

    /// Hours `first..=last` of `branch` the rows wrote and a failed capture lost: a gap the next
    /// capture holds, with why.
    fn lose(&mut self, branch: u64, first: i32, last: i32) {
        self.counters.lost += (i64::from(last) - i64::from(first) + 1) as u64;
        let reason = self.counters.stopped.as_ref().map_or("", Reason::text);
        let gap = engine_allocation::string(reason).and_then(|reason| {
            replay::spool::encode_record(&replay::spool::Record::Gap(replay_frame::branches::Gap {
                branch,
                first_hour: first,
                last_hour: last,
                reason,
            }))
        });
        let kept = gap.and_then(|gap| {
            engine_allocation::reserve(&mut self.controls, 1)?;
            self.controls.push(gap);
            Ok(())
        });
        // A gap that cannot be held leaves the hours to the rows alone; the status counts them.
        let _ = kept;
    }
}

/// An hour the recording could not take. Its own allocations are advisory, so its failure is
/// never the collector's allocation failure.
fn hour_failed(_: io::Error) -> io::Error {
    invalid("The columnar recording could not take the hour")
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
        format: replay_data::format::FormatVersion::CURRENT.version(),
        journaled: 0,
        controls: Vec::new(),
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
            failures: 0,
            lost: 0,
            hours: 0,
            failed: 0,
            runs: 0,
            journaled: 0,
            hour_ns: 0,
            journal: [0; 4],
            ended_ns: [0; TABLE_COUNT],
            ended_retried: [0; TABLE_COUNT],
            durability: None,
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
        durability: Option<&journal_thread::Timing>,
        journal: Option<[u64; 4]>,
    ) -> Value {
        let [queued, queued_max, append_ns, append_max_ns] = journal.unwrap_or(self.journal);
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
            "failed_hours": self.failed,
            "pending": pending,
            "runs": self.runs,
            "journaled_bytes": self.journaled,
            "hour_ns": self.hour_ns,
            "journal_queued": queued,
            "journal_queued_max": queued_max,
            "append_ns": append_ns,
            "append_max_ns": append_max_ns,
            "finished": finished,
            "stopped": self.stopped.as_ref().map(Reason::text),
            "failures": self.failures,
            "lost_hours": self.lost,
            "segment_syncs": durability.or(self.durability.as_ref()).map(journal_thread::Timing::status),
            "scope": "cumulative over the campaign: hours committed and failed, runs written, hours waiting for their rows' write; the sixteen tables with the most frame-writer wall, summed over the lanes; the sixteen tables whose lane hours were most often prepared again on the spool thread, for a string the run had not used or a failure; bytes of manifests and sections given to the journal; spool-thread wall preparing and committing hours; the journal thread's operations queued now and at most, and its wall appending hours (segment opens and syncs included)",
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

    fn of(site: &'static str, error: &io::Error) -> Self {
        let mut text = String::new();
        if text.try_reserve_exact(Self::BYTES).is_err() {
            return Self::Unrecorded;
        }
        // An OS error's own text is built by allocating; its code is not. The writer's own
        // errors carry a message that formats in place.
        let mut out = Bounded(&mut text);
        let kind = error.kind();
        let _ = match (error.raw_os_error(), error.get_ref()) {
            (Some(code), _) => std::fmt::Write::write_fmt(&mut out, format_args!("{site}: {kind:?} (OS error {code})")),
            (None, Some(inner)) => std::fmt::Write::write_fmt(&mut out, format_args!("{site}: {kind:?}: {inner}")),
            (None, None) => std::fmt::Write::write_fmt(&mut out, format_args!("{site}: {kind:?}")),
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
