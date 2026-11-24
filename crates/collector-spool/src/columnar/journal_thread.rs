use replay_recording::capture::{Capture, Stop, capture_of};
use replay_journal::columnar_journal::{JournalWriter, Witness, segment_of};
use std::{
    collections::VecDeque,
    fs, io,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::Instant,
};

const STACK_BYTES: usize = 256 * 1024;
const STEP_NS: u64 = 100_000;
const STEPS: usize = 2048;

pub(crate) const OPENING: &str = "opening the journal";
pub(crate) const NEXT_SEGMENT: &str = "starting the journal's next segment";
pub(crate) const RUN: &str = "starting a run in the journal";
pub(crate) const APPEND: &str = "appending an hour to the journal";
pub(crate) const DURABLE: &str = "making an ended journal segment durable";
pub(crate) const CONTROLS: &str = "writing control records to the journal";
pub(crate) const FINISHING: &str = "finishing the journal";

pub(crate) struct JournalThread {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    #[cfg(test)]
    refuse: AtomicUsize,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    pipeline: OnceLock<engine_allocation::SharedBox<collector_workers::frame_priority::Pipeline>>,
}

struct State {
    work: VecDeque<(Arc<Lane>, Op)>,
    stop: bool,
    timing: Timing,
}

pub(super) struct Opening {
    pub directory: PathBuf,
    pub identity: Vec<u8>,
    pub capture: Capture,
    pub(crate) controls: Vec<Vec<u8>>,
}

pub(super) struct Hour {
    pub(crate) starts: Option<(u64, u8, i32)>,
    pub branch: u64,
    pub hour: i32,
    pub(crate) ends_segment: bool,
    pub witness: Option<Witness>,
    pub manifest: Vec<u8>,
    pub section_bytes: Vec<u8>,
    pub section_ends: Vec<u32>,
}

pub(super) enum Op {
    Open(Box<Opening>),
    Controls(Vec<Vec<u8>>),
    Hour(Box<Hour>),
    End(Ending),
}

pub(super) enum Ending {
    Finished,
    Stopped {
        reason: String,
        command_branch: Option<u64>,
    },
    Abandoned,
}

#[derive(Default)]
pub(super) struct Failure {
    pub failed: Option<(&'static str, io::Error)>,
    pub lost: Vec<(u64, i32)>,
    pub(crate) controls: Vec<Vec<u8>>,
}

pub(super) struct Lane {
    writer: Mutex<Option<JournalWriter>>,
    failure: Mutex<Failure>,
    refusing: AtomicBool,
    last: Mutex<Option<(u64, i32)>>,
    #[cfg(feature = "columnar-fault-injection")]
    directory: Mutex<Option<PathBuf>>,
    queued: AtomicUsize,
    queued_max: AtomicUsize,
    append_ns: AtomicU64,
    append_max_ns: AtomicU64,
}

#[derive(Clone, Copy)]
pub(super) struct Timing {
    steps: [u32; STEPS],
    count: u64,
    max_ns: u64,
}

impl Timing {
    const fn new() -> Self {
        Self {
            steps: [0; STEPS],
            count: 0,
            max_ns: 0,
        }
    }

    fn record(&mut self, ns: u64) {
        let step = ((ns / STEP_NS) as usize).min(STEPS - 1);
        self.steps[step] = self.steps[step].saturating_add(1);
        self.count += 1;
        self.max_ns = self.max_ns.max(ns);
    }

    fn quantile_ns(&self, quantile: f64) -> Option<u64> {
        if self.count == 0 {
            return None;
        }
        let rank = ((self.count - 1) as f64 * quantile).round() as u64;
        let mut seen = 0;
        for (step, &count) in self.steps.iter().enumerate() {
            seen += u64::from(count);
            if seen > rank {
                return Some(if step == STEPS - 1 {
                    self.max_ns
                } else {
                    ((step as u64 + 1) * STEP_NS).min(self.max_ns)
                });
            }
        }
        Some(self.max_ns)
    }

    pub(super) fn status(&self) -> serde_json::Value {
        serde_json::json!({
            "count": self.count,
            "p50_ns": self.quantile_ns(0.5),
            "p99_ns": self.quantile_ns(0.99),
            "max_ns": self.max_ns,
        })
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait<'a>(&self, state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        self.changed
            .wait(state)
            .unwrap_or_else(PoisonError::into_inner)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Lane {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            writer: Mutex::new(None),
            failure: Mutex::new(Failure::default()),
            refusing: AtomicBool::new(false),
            last: Mutex::new(None),
            #[cfg(feature = "columnar-fault-injection")]
            directory: Mutex::new(None),
            queued: AtomicUsize::new(0),
            queued_max: AtomicUsize::new(0),
            append_ns: AtomicU64::new(0),
            append_max_ns: AtomicU64::new(0),
        })
    }

    pub(super) fn take_failure(&self) -> Option<Failure> {
        let mut failure = lock(&self.failure);
        (failure.failed.is_some() || !failure.lost.is_empty() || !failure.controls.is_empty())
            .then(|| std::mem::take(&mut *failure))
    }

    pub(super) fn counters(&self) -> [u64; 4] {
        [
            self.queued.load(Ordering::Acquire) as u64,
            self.queued_max.load(Ordering::Relaxed) as u64,
            self.append_ns.load(Ordering::Relaxed),
            self.append_max_ns.load(Ordering::Relaxed),
        ]
    }

    #[cfg(test)]
    pub(super) fn hold_writer(&self) -> MutexGuard<'_, Option<JournalWriter>> {
        lock(&self.writer)
    }

    fn fail(&self, site: &'static str, error: io::Error) {
        self.refusing.store(true, Ordering::Release);
        let mut failure = lock(&self.failure);
        failure.failed.get_or_insert((site, error));
    }

    fn refused(&self, op: Op) {
        let mut failure = lock(&self.failure);
        match op {
            Op::Open(opening) => failure.controls.extend(opening.controls),
            Op::Controls(controls) => failure.controls.extend(controls),
            Op::Hour(hour) => failure.lost.push((hour.branch, hour.hour)),
            Op::End(_) => {}
        }
    }

    fn run(&self, op: Op, timing: &Mutex<State>) {
        let mut writer = lock(&self.writer);
        if let Op::End(ending) = op {
            let journal = writer.take();
            let stop = match ending {
                Ending::Finished => None,
                Ending::Stopped {
                    reason,
                    command_branch,
                } => {
                    let last = *lock(&self.last);
                    Some(Stop {
                        branch: last
                            .map(|(branch, _)| branch)
                            .or(command_branch)
                            .unwrap_or(0),
                        last_hour: last.map(|(_, hour)| hour),
                        reason,
                    })
                }
                Ending::Abandoned => {
                    self.refusing.store(false, Ordering::Release);
                    return;
                }
            };
            if let Some(journal) = journal
                && let Err(error) = journal.finish(stop.as_ref())
                && !self.refusing.load(Ordering::Acquire)
            {
                self.fail(FINISHING, error);
            }
            self.refusing.store(false, Ordering::Release);
            return;
        }
        if self.refusing.load(Ordering::Acquire) {
            self.refused(op);
            return;
        }
        match op {
            Op::Open(opening) => {
                let controls = match open(&opening) {
                    Ok(journal) => {
                        *writer = Some(journal);
                        #[cfg(feature = "columnar-fault-injection")]
                        {
                            *lock(&self.directory) = Some(opening.directory.clone());
                        }
                        opening.controls
                    }
                    Err(error) => {
                        self.fail(OPENING, error);
                        drop(writer);
                        self.refused(Op::Open(opening));
                        return;
                    }
                };
                if let Some(journal) = writer.as_mut()
                    && let Err(error) = journal.controls(&controls)
                {
                    self.fail(CONTROLS, error);
                    lock(&self.failure).controls.extend(controls);
                }
            }
            Op::Controls(controls) => match writer.as_mut() {
                Some(journal) => {
                    if let Err(error) = journal.controls(&controls) {
                        self.fail(CONTROLS, error);
                        lock(&self.failure).controls.extend(controls);
                    }
                }
                None => {
                    self.fail(CONTROLS, io::Error::from(io::ErrorKind::NotConnected));
                    lock(&self.failure).controls.extend(controls);
                }
            },
            Op::Hour(hour) => {
                let started = Instant::now();
                let (branch, at) = (hour.branch, hour.hour);
                #[cfg(feature = "columnar-fault-injection")]
                let injected = lock(&self.directory).as_ref().is_some_and(|directory| {
                    fs::remove_file(directory.join("columnar.fail")).is_ok()
                });
                #[cfg(not(feature = "columnar-fault-injection"))]
                let injected = false;
                let written = match writer.as_mut() {
                    _ if injected => Err((
                        APPEND,
                        io::Error::other("a fault injected by columnar.fail"),
                    )),
                    Some(journal) => write(journal, &hour, timing),
                    None => Err((APPEND, io::Error::from(io::ErrorKind::NotConnected))),
                };
                let elapsed = started.elapsed().as_nanos() as u64;
                self.append_ns.fetch_add(elapsed, Ordering::Relaxed);
                self.append_max_ns.fetch_max(elapsed, Ordering::Relaxed);
                match written {
                    Ok(()) => *lock(&self.last) = Some((branch, at)),
                    Err((site, error)) => {
                        self.fail(site, error);
                        lock(&self.failure).lost.push((branch, at));
                    }
                }
            }
            Op::End(_) => unreachable!("ended above"),
        }
    }
}

fn open(opening: &Opening) -> io::Result<JournalWriter> {
    fs::create_dir_all(&opening.directory)?;
    let mut n = 0_u32;
    for entry in fs::read_dir(&opening.directory)? {
        let name = entry?.file_name();
        let taken = capture_of(&name)
            .map(|(taken, _)| taken)
            .or_else(|| segment_of(&name).map(|(taken, _)| taken));
        if let Some(taken) = taken {
            let next = taken.checked_add(1);
            n = n.max(next.ok_or_else(|| io::Error::from(io::ErrorKind::StorageFull))?);
        }
    }
    JournalWriter::create(&opening.directory, n, &opening.identity, &opening.capture)
}

fn write(
    journal: &mut JournalWriter,
    hour: &Hour,
    timing: &Mutex<State>,
) -> Result<(), (&'static str, io::Error)> {
    if let Some((branch, epoch, first_hour)) = hour.starts {
        let ended = journal
            .next_segment()
            .map_err(|error| (NEXT_SEGMENT, error))?;
        sync(ended, timing)?;
        journal
            .run(branch, epoch, first_hour)
            .map_err(|error| (RUN, error))?;
    }
    let mut sections: [&[u8]; replay_data::rows::TABLE_COUNT] =
        [&[][..]; replay_data::rows::TABLE_COUNT];
    let mut start = 0usize;
    for (at, &end) in hour.section_ends.iter().enumerate() {
        let end = end as usize;
        sections[at] = &hour.section_bytes[start..end];
        start = end;
    }
    journal
        .hour(hour.witness, &hour.manifest, &sections[..hour.section_ends.len()])
        .map_err(|error| (APPEND, error))?;
    if hour.ends_segment {
        let ended = journal
            .next_segment()
            .map_err(|error| (NEXT_SEGMENT, error))?;
        sync(ended, timing)?;
    }
    Ok(())
}

fn sync(ended: Option<fs::File>, timing: &Mutex<State>) -> Result<(), (&'static str, io::Error)> {
    let Some(file) = ended else {
        return Ok(());
    };
    let started = Instant::now();
    let synced = file.sync_data();
    lock(timing)
        .timing
        .record(started.elapsed().as_nanos() as u64);
    synced.map_err(|error| (DURABLE, error))
}

impl JournalThread {
    pub(crate) fn start() -> io::Result<Arc<Self>> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                work: VecDeque::new(),
                stop: false,
                timing: Timing::new(),
            }),
            changed: Condvar::new(),
            pipeline: OnceLock::new(),
        });
        let worker = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("hoi4-journal-writer".into())
            .stack_size(STACK_BYTES)
            .spawn(move || {
                collector_workers::private_heap::off_hold_path();
                // SAFETY: This dedicated journal writer thread registers and clears its own role.
                unsafe {
                    collector_workers::thread_cpu::register(
                        collector_workers::thread_cpu::Role::JournalWriter,
                    );
                }
                serve(&worker);
                // SAFETY: This dedicated journal writer thread clears its own role before exit.
                unsafe {
                    collector_workers::thread_cpu::register_id(
                        collector_workers::thread_cpu::Role::JournalWriter,
                        0,
                    );
                }
            })?;
        Ok(Arc::new(Self {
            shared,
            thread: Some(thread),
            #[cfg(test)]
            refuse: AtomicUsize::new(0),
        }))
    }

    pub(crate) fn attach_priority(
        &self,
        pipeline: engine_allocation::SharedBox<collector_workers::frame_priority::Pipeline>,
    ) {
        let _ = self.shared.pipeline.set(pipeline);
        self.shared.changed.notify_all();
    }

    pub(crate) fn thread_id(&self) -> Option<u32> {
        use std::os::windows::io::AsRawHandle;
        let handle = self.thread.as_ref()?.as_raw_handle();
        // SAFETY: `self.thread` stays alive for the duration of this query.
        let id = unsafe { windows_sys::Win32::System::Threading::GetThreadId(handle.cast()) };
        (id != 0).then_some(id)
    }

    pub(super) fn queue(&self, lane: &Arc<Lane>, op: Op) {
        let mut state = self.shared.lock();
        #[cfg(test)]
        let refused = self
            .refuse
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |left| {
                left.checked_sub(1)
            })
            .is_ok();
        #[cfg(not(test))]
        let refused = false;
        if refused || state.work.try_reserve(1).is_err() {
            drop(state);
            let site = match &op {
                Op::Open(_) => OPENING,
                Op::Controls(_) => CONTROLS,
                Op::Hour(_) => APPEND,
                Op::End(_) => {
                    self.settle(lane);
                    let _phase = collector_memory::terminal::enter(
                        collector_memory::terminal::Phase::Journal,
                    );
                    engine_allocation::finishing(|| lane.run(op, &self.shared.state));
                    return;
                }
            };
            lane.fail(site, engine_allocation::oom());
            lane.refused(op);
            return;
        }
        let queued = lane.queued.fetch_add(1, Ordering::AcqRel) + 1;
        lane.queued_max.fetch_max(queued, Ordering::Relaxed);
        state.work.push_back((Arc::clone(lane), op));
        drop(state);
        self.shared.changed.notify_all();
    }

    pub(super) fn settle(&self, lane: &Lane) {
        let mut state = self.shared.lock();
        while lane.queued.load(Ordering::Acquire) != 0 {
            state = self.shared.wait(state);
        }
    }

    #[cfg(test)]
    pub(super) fn refuse_queues(&self, count: usize) {
        self.refuse.store(count, Ordering::Release);
    }

    pub(super) fn timing(&self) -> Timing {
        self.shared.lock().timing
    }
}

impl Drop for JournalThread {
    fn drop(&mut self) {
        self.shared.lock().stop = true;
        self.shared.changed.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(shared: &Shared) {
    let _phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Journal);
    let mut own = collector_workers::priority::OwnPriority::new();
    // SAFETY: The owning worker adjusts only its own thread's priority.
    unsafe { own.follow(false) };
    let mut managed = None;
    let mut state = shared.lock();
    loop {
        if let Some(pipeline) = shared.pipeline.get() {
            // SAFETY: This owned worker updates its priority from its session's own pipeline.
            unsafe {
                managed
                    .get_or_insert_with(|| collector_workers::priority::OwnPriority::frame_pipeline())
                    .follow_pipeline(
                        pipeline,
                        collector_workers::frame_priority::Role::JournalWriter,
                    );
            }
        }
        let Some((lane, op)) = state.work.pop_front() else {
            if state.stop {
                return;
            }
            state = shared.wait(state);
            continue;
        };
        drop(state);
        engine_allocation::finishing(|| lane.run(op, &shared.state));
        lane.queued.fetch_sub(1, Ordering::AcqRel);
        state = shared.lock();
        shared.changed.notify_all();
    }
}
