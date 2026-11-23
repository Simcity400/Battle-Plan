use replay_recording::{capture::Capture, schema::schema};
use replay_journal::{columnar_journal::Witness, native_frame::{self, FrameHour, FrameWriter, RowFrame}};
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
pub(crate) use journal_thread::JournalThread;
use journal_thread::{Ending, Failure, Lane, Op};
#[cfg(test)]
mod tests;

static FINISHED_COUNTERS: Mutex<Vec<Counters>> = Mutex::new(Vec::new());

pub(crate) fn finished_status() -> Vec<Value> {
    FINISHED_COUNTERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .map(|counters| counters.status(&[], &[], 0, true, None, None))
        .collect()
}

pub(crate) enum Columnar {
    Live(Box<Live>),
    Unopened,
    #[cfg(test)]
    Off,
}

pub(crate) struct Lanes<'a> {
    helpers: Option<collector_workers::spool_helper::Helpers<'a>>,
    spool: std::thread::ThreadId,
}

impl<'a> Lanes<'a> {
    pub(crate) fn new(helpers: Option<collector_workers::spool_helper::Helpers<'a>>) -> Self {
        Self {
            helpers,
            spool: std::thread::current().id(),
        }
    }
}

impl native_frame::Lanes for Lanes<'_> {
    fn count(&self) -> usize {
        self.helpers
            .map_or(1, collector_workers::spool_helper::Helpers::encoding_lanes)
    }

    fn run(&self, job: &(dyn Fn() + Sync)) {
        let Some(helpers) = self.helpers else {
            return job();
        };
        let (phase, context) = (
            collector_memory::terminal::phase(),
            collector_memory::terminal::context(),
        );
        let helper = || {
            if collector_workers::tick_gate::ticking() {
                return;
            }
            let _phase = collector_memory::terminal::enter(phase);
            collector_memory::terminal::set_context(context);
            let _span =
            // SAFETY: This conversion worker records its phase on the session's helper counters.
            unsafe { collector_workers::spool_phases::HELPER
                .span_now(collector_workers::spool_phases::Phase::Columnar) };
            engine_allocation::advisory(job);
        };
        // SAFETY: The spool owns this conversion round and uses no helper in a competing round.
        unsafe { helpers.join_each(&helper, job) };
    }

    fn proceeds(&self) -> bool {
        std::thread::current().id() == self.spool || !collector_workers::tick_gate::ticking()
    }
}

pub(crate) struct Live {
    directory: PathBuf,
    identity: Option<envelope::Identity>,
    capture: Option<Capture>,
    template: (envelope::Identity, Capture),
    facts: Vec<Vec<u8>>,
    reopen: Option<Reopen>,
    noted: HashSet<[u8; 16]>,
    notes: VecDeque<Note>,
    staged: Vec<Noted>,
    witness: Option<(i32, Witness)>,
    controls: Vec<Vec<u8>>,
    opened: bool,
    thread: Arc<JournalThread>,
    lane: Arc<Lane>,
    run: Option<Run>,
    pending: VecDeque<Pending>,
    next_epoch: Option<u8>,
    finished: bool,
    command_branch: Option<u64>,
    started: u64,
    counters: Counters,
}

struct Reopen {
    wait: u32,
    failures: u32,
}

struct Note {
    awaits: Option<(u64, i32)>,
    records: Vec<Noted>,
}

struct Noted {
    bytes: Vec<u8>,
    fact: bool,
}

struct Counters {
    stopped: Option<Reason>,
    failures: u64,
    lost: u64,
    hours: u64,
    failed: u64,
    runs: u64,
    journaled: u64,
    hour_ns: u64,
    journal: [u64; 4],
    ended_ns: [u64; TABLE_COUNT],
    ended_retried: [u64; TABLE_COUNT],
    durability: Option<journal_thread::Timing>,
}

#[derive(Clone, Copy)]
struct RunStart {
    branch: u64,
    epoch: u8,
    first_hour: i32,
}

struct Run {
    frames: FrameWriter,
    branch: u64,
    next_hour: i32,
    ordinal: u32,
    start: Option<RunStart>,
}

struct Pending {
    starts: Option<RunStart>,
    branch: u64,
    hour: i32,
    ordinal: u32,
    witness: Option<Witness>,
    manifest: Vec<u8>,
    sections: Vec<Vec<u8>>,
}

impl Columnar {
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

    #[cfg(test)]
    pub(crate) const fn off() -> Self {
        Self::Off
    }

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

    pub(crate) fn commit(
        &mut self,
        hour: FrameHour,
        queued: bool,
        lanes: &dyn native_frame::Lanes,
    ) -> io::Result<()> {
        match self {
            Self::Live(live) => engine_allocation::advisory(|| live.commit(hour, queued, lanes))
                .map_err(hour_failed),
            _ => Err(invalid("A columnar hour commits within its recording")),
        }
    }

    pub(crate) fn complete(&mut self, branch: u64, hour: i32) {
        if let Self::Live(live) = self {
            engine_allocation::advisory(|| live.complete(branch, hour));
        }
    }

    pub(crate) fn note(&mut self, records: &[replay_capture_spool::spool::Record]) {
        if let Self::Live(live) = self {
            engine_allocation::advisory(|| live.note(records));
        }
    }

    pub(crate) fn stage(&mut self, records: &[replay_capture_spool::spool::Record]) {
        if let Self::Live(live) = self {
            engine_allocation::advisory(|| live.stage(records));
        }
    }

    pub(crate) fn written(&mut self) {
        if let Self::Live(live) = self {
            engine_allocation::advisory(|| live.written());
        }
    }

    pub(crate) fn queued(&mut self, branch: u64, hour: i32) {
        if let Self::Live(live) = self {
            engine_allocation::advisory(|| live.queued(branch, hour));
        }
    }

    pub(crate) fn interrupt(&mut self) {
        if let Self::Live(live) = self {
            engine_allocation::advisory(|| live.interrupt());
        }
    }

    pub(crate) fn finish(&mut self) {
        if let Self::Live(live) = self {
            engine_allocation::advisory(|| live.finish());
        }
    }

    #[cfg(test)]
    pub(crate) fn settle(&self) {
        if let Self::Live(live) = self {
            live.thread.settle(&live.lane);
        }
    }

    pub(crate) fn status(&self) -> Option<Value> {
        match self {
            Self::Live(live) => Some(live.status()),
            Self::Unopened => {
                Some(json!({"stopped": "the columnar recording could not be opened"}))
            }
            #[cfg(test)]
            Self::Off => None,
        }
    }
}

impl Live {
    fn note(&mut self, records: &[replay_capture_spool::spool::Record]) {
        let command_branch = records.iter().find_map(|record| match record {
            replay_capture_spool::spool::Record::Control(replay_recording::control::ControlRecord::PlayerCommands {
                packet,
                ..
            }) => Some(packet.branch),
            _ => None,
        });
        let noted = Self::encoded(records).and_then(|records| {
            self.notes
                .try_reserve(1)
                .map_err(|_| engine_allocation::oom())?;
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

    fn stage(&mut self, records: &[replay_capture_spool::spool::Record]) {
        self.witness = records.iter().find_map(|record| match record {
            replay_capture_spool::spool::Record::Control(replay_recording::control::ControlRecord::ClockWitness {
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
            self.notes
                .try_reserve(1)
                .map_err(|_| engine_allocation::oom())?;
            self.notes.push_back(Note { awaits, records });
            self.release()
        })();
        if let Err(error) = given {
            self.stop("releasing a frame's control records", &error);
        }
    }

    fn encoded(records: &[replay_capture_spool::spool::Record]) -> io::Result<Vec<Noted>> {
        use replay_recording::control::ControlRecord;
        use replay_capture_spool::spool::Record;
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
            let bytes = replay_capture_spool::spool::encode_record(record)?;
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

    fn halt_reason(&self) -> Option<&str> {
        if self.finished {
            return Some("the columnar recording finished");
        }
        self.reopen.as_ref().map(|_| {
            self.counters
                .stopped
                .as_ref()
                .map_or("the columnar recording failed", Reason::text)
        })
    }

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

    fn end_run(&mut self) {
        if let Some(run) = self.run.take() {
            for (total, ns) in self.counters.ended_ns.iter_mut().zip(run.frames.table_ns()) {
                *total += ns;
            }
            for (total, hours) in self
                .counters
                .ended_retried
                .iter_mut()
                .zip(run.frames.retried())
            {
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

    fn writable(&self) -> io::Result<()> {
        if self.writing() {
            Ok(())
        } else {
            Err(invalid("The columnar recording stopped"))
        }
    }

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
        let bytes = pending.manifest.len() + pending.sections.iter().map(Vec::len).sum::<usize>();
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
        self.pending.clear();
        self.drop_unwritten();
        self.end_run();
        self.thread.settle(&self.lane);
        self.poll();
        self.finished = true;
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
            let ending = match self
                .counters
                .stopped
                .as_ref()
                .filter(|_| self.reopen.is_some())
            {
                Some(reason) => {
                    engine_allocation::string(reason.text()).map_or(Ending::Abandoned, |reason| {
                        Ending::Stopped {
                            reason,
                            command_branch: self.command_branch,
                        }
                    })
                }
                None => Ending::Finished,
            };
            self.thread.queue(&self.lane, Op::End(ending));
            self.thread.settle(&self.lane);
            if let Some(Failure {
                failed: Some((site, error)),
                ..
            }) = self.lane.take_failure()
            {
                self.stop(site, &error);
            }
        }
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
        let (current, retried) = self.run.as_ref().map_or((&[][..], &[][..]), |run| {
            (run.frames.table_ns(), run.frames.retried())
        });
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
            let ending = engine_allocation::string(reason).map_or(Ending::Abandoned, |reason| {
                Ending::Stopped {
                    reason,
                    command_branch: self.command_branch,
                }
            });
            self.thread.queue(&self.lane, Op::End(ending));
        }
        self.thread.settle(&self.lane);
        if let Some(Failure { lost, controls, .. }) = self.lane.take_failure() {
            self.absorb(Failure {
                failed: None,
                lost,
                controls,
            });
        }
    }

    fn lose(&mut self, branch: u64, first: i32, last: i32) {
        self.counters.lost += (i64::from(last) - i64::from(first) + 1) as u64;
        let reason = self.counters.stopped.as_ref().map_or("", Reason::text);
        let gap = engine_allocation::string(reason).and_then(|reason| {
            replay_capture_spool::spool::encode_record(&replay_capture_spool::spool::Record::Gap(replay_frame::branches::Gap {
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
        let _ = kept;
    }
}

fn hour_failed(_: io::Error) -> io::Error {
    invalid("The columnar recording could not take the hour")
}

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
        let mut most: Vec<usize> = (0..TABLE_COUNT)
            .filter(|&table| retried[table] > 0)
            .collect();
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
        let mut out = Bounded(&mut text);
        let kind = error.kind();
        let _ = match (error.raw_os_error(), error.get_ref()) {
            (Some(code), _) => std::fmt::Write::write_fmt(
                &mut out,
                format_args!("{site}: {kind:?} (OS error {code})"),
            ),
            (None, Some(inner)) => {
                std::fmt::Write::write_fmt(&mut out, format_args!("{site}: {kind:?}: {inner}"))
            }
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

fn invalid(message: &'static str) -> io::Error {
    if engine_allocation::capture_active() {
        io::Error::from(io::ErrorKind::InvalidData)
    } else {
        io::Error::new(io::ErrorKind::InvalidData, message)
    }
}
