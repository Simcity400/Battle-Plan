use collector_memory::terminal::Context;
use replay_capture_spool::spool::{FileBackend, Record, SealedRecord, SegmentBackend, SegmentedSpool};
use replay_frame::frame::EncodedFrame;
use std::{
    collections::VecDeque,
    io,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, Weak,
        atomic::{AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

type CampaignId = [u8; 36];
const MIN_BUDGET: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy)]
struct FrameWitness {
    ticket: u64,
    context: Context,
}

enum Command {
    StartSegment {
        records: Vec<SealedRecord>,
        frame: bool,
    },
    Records {
        records: Vec<SealedRecord>,
        frame: bool,
    },
    FinishSegment(CloseReceipt),
    CloseContext {
        records: Vec<SealedRecord>,
        restarting: bool,
        receipt: CloseReceipt,
    },
}

#[derive(Clone, Default)]
pub struct CloseReceipt {
    result: Arc<Mutex<Option<Result<(), io::ErrorKind>>>>,
    pub ticket: u64,
}
impl CloseReceipt {
    pub fn result(&self) -> Option<Result<(), io::ErrorKind>> {
        *self.result.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// # Safety
    /// Only the writer completing this receipt, or an isolated fixture holding its own receipt, may
    /// publish the result. It must be the actual result for this receipt's close.
    pub unsafe fn complete(&self, result: Result<(), io::ErrorKind>) {
        let mut outcome = self.result.lock().unwrap_or_else(PoisonError::into_inner);
        if outcome.is_none() {
            *outcome = Some(result);
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn pending() -> Self {
        Self::default()
    }
}

struct Job {
    ticket: u64,
    campaign: CampaignId,
    generation: u64,
    context: Context,
    payload_bytes: usize,
    gaps: Vec<(u64, i32, i32)>,
    command: Command,
}

pub struct Completion {
    pub ticket: u64,
    pub context: Context,
    pub frame: bool,
    pub committed: bool,
    pub result: io::Result<()>,
}

#[derive(Clone, Copy, Default)]
pub struct Diagnostics {
    bytes: usize,
    deferred_bytes: usize,
    owned_bytes: usize,
    waiting: usize,
    budget: usize,
}

#[derive(Default)]
struct PoolCharge {
    counter: Option<engine_allocation::SharedBox<AtomicU64>>,
    bytes: usize,
}
impl PoolCharge {
    fn set(&mut self, bytes: usize) {
        if let Some(counter) = &self.counter {
            if bytes >= self.bytes {
                counter.fetch_add((bytes - self.bytes) as u64, Ordering::Relaxed);
            } else {
                counter.fetch_sub((self.bytes - bytes) as u64, Ordering::Relaxed);
            }
        }
        self.bytes = bytes;
    }
}
impl Drop for PoolCharge {
    fn drop(&mut self) {
        self.set(0);
    }
}

pub(crate) struct ReceiptStorage {
    shared: Arc<Shared>,
    bytes: usize,
    owned_bytes: usize,
}
impl Drop for ReceiptStorage {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.diagnostics.deferred_bytes -= self.bytes;
        state.deferred_owned_bytes -= self.owned_bytes;
        Shared::reconcile(&mut state);
        self.shared.changed.notify_all();
    }
}

pub(crate) struct StagedFrame {
    records: Vec<SealedRecord>,
    restarting: bool,
}
impl StagedFrame {
    pub(crate) fn admission_bytes(&self) -> io::Result<usize> {
        self.records.iter().try_fold(
            self.records.capacity() * size_of::<SealedRecord>()
                + self
                    .records
                    .iter()
                    .filter(|record| record.gap_range().is_some())
                    .count()
                    * size_of::<(u64, i32, i32)>()
                + size_of::<Job>()
                + size_of::<Completion>()
                + size_of::<FrameWitness>()
                + size_of::<Context>(),
            |bytes, record| {
                bytes
                    .checked_add(record.storage_bytes())
                    .ok_or_else(engine_allocation::oom)
            },
        )
    }
}

struct State {
    jobs: VecDeque<Job>,
    completions: VecDeque<Completion>,
    frames: Vec<FrameWitness>,
    contexts: Vec<Context>,
    watermarks: Vec<(CampaignId, Option<(u32, u64)>)>,
    diagnostics: Diagnostics,
    next_ticket: u64,
    completed: u64,
    failed: Vec<(CampaignId, Option<u64>)>,
    stop: bool,
    stopped: bool,
    drainers: usize,
    blocked: bool,
    failure_epoch: u64,
    payload_bytes: usize,
    deferred_owned_bytes: usize,
    unfinished_jobs: usize,
    ledger_bytes: usize,
    backend_bytes: usize,
    factory_bytes: usize,
    close_receipts: Vec<Weak<Mutex<Option<Result<(), io::ErrorKind>>>>>,
    pool: PoolCharge,
}
impl Default for State {
    fn default() -> Self {
        Self {
            jobs: VecDeque::new(),
            completions: VecDeque::new(),
            frames: Vec::new(),
            contexts: Vec::new(),
            watermarks: Vec::new(),
            diagnostics: Diagnostics {
                budget: MIN_BUDGET,
                ..Diagnostics::default()
            },
            next_ticket: 1,
            completed: 0,
            failed: Vec::new(),
            stop: false,
            stopped: false,
            drainers: 0,
            blocked: false,
            failure_epoch: 0,
            payload_bytes: 0,
            deferred_owned_bytes: 0,
            unfinished_jobs: 0,
            ledger_bytes: 0,
            backend_bytes: 0,
            factory_bytes: 0,
            close_receipts: Vec::new(),
            pool: PoolCharge::default(),
        }
    }
}
impl State {
    fn retire_contexts(&mut self) {
        self.contexts.retain(|context| {
            self.frames
                .iter()
                .any(|frame| same_context(*context, frame.context))
        });
    }
    fn failed(&self, campaign: CampaignId, generation: u64) -> bool {
        self.failed
            .iter()
            .any(|&(id, failed)| id == campaign && failed == Some(generation))
    }
    fn fail(&mut self, campaign: CampaignId, generation: u64) {
        self.failed
            .iter_mut()
            .find(|(id, _)| *id == campaign)
            .unwrap()
            .1 = Some(generation);
    }
}

pub struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    pipeline: OnceLock<engine_allocation::SharedBox<collector_workers::frame_priority::Pipeline>>,
}
impl Shared {
    fn reconcile(state: &mut State) {
        state.diagnostics.bytes = state.payload_bytes
            + state.diagnostics.deferred_bytes
            + state.unfinished_jobs * size_of::<Job>()
            + state.completions.len() * size_of::<Completion>()
            + state.frames.len() * size_of::<FrameWitness>()
            + state.contexts.len() * size_of::<Context>();
        state
            .close_receipts
            .retain(|receipt| receipt.strong_count() != 0);
        let bytes = size_of::<Shared>()
            + 2 * size_of::<usize>()
            + state.jobs.capacity() * size_of::<Job>()
            + state.completions.capacity() * size_of::<Completion>()
            + state.frames.capacity() * size_of::<FrameWitness>()
            + state.contexts.capacity() * size_of::<Context>()
            + state.watermarks.capacity() * size_of::<(CampaignId, Option<(u32, u64)>)>()
            + state.failed.capacity() * size_of::<(CampaignId, Option<u64>)>()
            + state.close_receipts.capacity()
                * size_of::<Weak<Mutex<Option<Result<(), io::ErrorKind>>>>>()
            + state.close_receipts.len()
                * (2 * size_of::<usize>() + size_of::<Mutex<Option<Result<(), io::ErrorKind>>>>())
            + state.payload_bytes
            + state.deferred_owned_bytes
            + state.ledger_bytes
            + state.backend_bytes
            + state.factory_bytes
            + if engine_allocation::capture_active() {
                collector_memory::terminal::uncommitted_storage_bytes()
            } else {
                0
            };
        state.diagnostics.owned_bytes = bytes;
        state.pool.set(bytes);
    }
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn hand_turn(&self, state: &State, schedule: &impl Schedule) {
        if schedule.grant(|| !state.jobs.is_empty() && state.unfinished_jobs == state.jobs.len()) {
            self.changed.notify_all();
        }
    }

    fn outstanding(
        state: &State,
    ) -> impl Iterator<Item = collector_memory::terminal::Outstanding> + '_ {
        state.contexts.iter().filter_map(|&context| {
            let first = state
                .frames
                .iter()
                .find(|frame| same_context(context, frame.context))?;
            let last = state
                .frames
                .iter()
                .rfind(|frame| same_context(context, frame.context))
                .unwrap();
            let campaign_id = context.campaign_id.unwrap();
            let watermark = state
                .watermarks
                .iter()
                .find(|(id, _)| *id == campaign_id)
                .and_then(|(_, watermark)| *watermark);
            Some(collector_memory::terminal::Outstanding {
                range: replay_data::witness::UncommittedRange {
                    campaign_id,
                    branch: context.branch.unwrap(),
                    load_signal: context.load_signal,
                    first_hour: first.context.failed_hour.unwrap(),
                    last_hour: last.context.failed_hour.unwrap(),
                },
                committed_segment: watermark.map(|(segment, _)| segment),
                committed_bytes: watermark.map_or(0, |(_, bytes)| bytes),
            })
        })
    }

    fn publish_range(state: &mut State) {
        state.retire_contexts();
        if engine_allocation::capture_active() {
            collector_memory::terminal::publish_uncommitted(Self::outstanding(state));
        }
    }

    fn submit(
        &self,
        campaign: CampaignId,
        generation: u64,
        command: Command,
        receipt_bytes: usize,
        schedule: &impl Schedule,
    ) -> io::Result<u64> {
        self.submit_with_wait(campaign, generation, command, receipt_bytes, true, schedule)
    }

    fn submit_with_wait(
        &self,
        campaign: CampaignId,
        generation: u64,
        mut command: Command,
        receipt_bytes: usize,
        wait: bool,
        schedule: &impl Schedule,
    ) -> io::Result<u64> {
        let context = collector_memory::terminal::context();
        let (mut bytes, frame) = match &command {
            Command::StartSegment { records, frame } | Command::Records { records, frame } => {
                let bytes = records
                    .iter()
                    .try_fold(
                        records
                            .capacity()
                            .saturating_mul(std::mem::size_of::<SealedRecord>()),
                        |bytes, record| bytes.checked_add(record.storage_bytes()),
                    )
                    .ok_or_else(engine_allocation::oom)?;
                (bytes, *frame)
            }
            Command::CloseContext { records, .. } => (
                records
                    .iter()
                    .try_fold(
                        records.capacity() * size_of::<SealedRecord>(),
                        |bytes, record| bytes.checked_add(record.storage_bytes()),
                    )
                    .ok_or_else(engine_allocation::oom)?,
                false,
            ),
            Command::FinishSegment(_) => (0, false),
        };
        let mut gaps = Vec::new();
        if let Command::StartSegment { records, .. }
        | Command::Records { records, .. }
        | Command::CloseContext { records, .. } = &command
        {
            gaps = engine_allocation::vec(
                records
                    .iter()
                    .filter(|record| record.gap_range().is_some())
                    .count(),
            )?;
            gaps.extend(records.iter().filter_map(SealedRecord::gap_range));
        }
        bytes = bytes
            .checked_add(
                gaps.capacity()
                    .saturating_mul(std::mem::size_of::<(u64, i32, i32)>()),
            )
            .ok_or_else(engine_allocation::oom)?;
        let payload_bytes = bytes;
        bytes = bytes
            .checked_add(
                size_of::<Job>()
                    + size_of::<Completion>()
                    + if frame {
                        size_of::<FrameWitness>() + size_of::<Context>()
                    } else {
                        0
                    },
            )
            .ok_or_else(engine_allocation::oom)?;
        let mut state = if wait {
            self.lock()
        } else {
            self.state
                .try_lock()
                .map_err(|_| io::ErrorKind::WouldBlock)?
        };
        let budget = budget_for(
            state.diagnostics.budget,
            bytes
                .checked_add(receipt_bytes)
                .ok_or_else(engine_allocation::oom)?,
        );
        let mut waited = false;
        while state.diagnostics.bytes > budget.saturating_sub(bytes) {
            if !wait {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            if state.stopped || collector_memory::terminal::failed() {
                return Err(io::ErrorKind::OutOfMemory.into());
            }
            if state.blocked {
                return Err(io::Error::other("spool writer failure awaiting receipts"));
            }
            if !waited {
                waited = true;
                state.diagnostics.waiting += 1;
                self.changed.notify_all();
            }
            self.hand_turn(&state, schedule);
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        if waited {
            state.diagnostics.waiting -= 1;
        }
        if collector_memory::terminal::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        if state.stopped {
            return Err(io::Error::other("spool writer stopped"));
        }
        if matches!(command, Command::Records { .. }) && state.failed(campaign, generation) {
            return Err(io::Error::other("spool segment generation failed"));
        }
        if state.blocked {
            return Err(io::Error::other("spool writer failure awaiting receipts"));
        }
        let completion_room = state.jobs.len() + state.watermarks.len() + 3;
        state
            .jobs
            .try_reserve(1)
            .map_err(|_| engine_allocation::oom())?;
        state
            .completions
            .try_reserve(completion_room)
            .map_err(|_| engine_allocation::oom())?;
        let new_context = frame
            && !state
                .contexts
                .iter()
                .any(|saved| same_context(*saved, context));
        if frame {
            engine_allocation::reserve(&mut state.frames, 1)?;
            if new_context {
                engine_allocation::reserve(&mut state.contexts, 1)?;
            }
        }
        if !state.watermarks.iter().any(|(id, _)| id == &campaign) {
            engine_allocation::reserve(&mut state.watermarks, 1)?;
            engine_allocation::reserve(&mut state.failed, 1)?;
            state.watermarks.push((campaign, None));
            state.failed.push((campaign, None));
        }
        let ticket = state.next_ticket;
        let next_ticket = ticket
            .checked_add(1)
            .ok_or_else(|| io::Error::other("spool writer sequence exhausted"))?;
        if let Command::FinishSegment(receipt) | Command::CloseContext { receipt, .. } = &command {
            engine_allocation::reserve(&mut state.close_receipts, 1)?;
            state.close_receipts.push(Arc::downgrade(&receipt.result));
        }
        if frame {
            if engine_allocation::capture_active() {
                collector_memory::terminal::prepare_uncommitted_context(context)?;
            }
            if new_context {
                state.contexts.push(context);
            }
            state.frames.push(FrameWitness { ticket, context });
            Self::publish_range(&mut state);
        }
        if collector_memory::terminal::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        if let Command::FinishSegment(receipt) | Command::CloseContext { receipt, .. } =
            &mut command
        {
            receipt.ticket = ticket;
        }
        state.next_ticket = next_ticket;
        state.diagnostics.budget = budget;
        state.payload_bytes += payload_bytes;
        state.unfinished_jobs += 1;
        state.jobs.push_back(Job {
            ticket,
            campaign,
            generation,
            context,
            payload_bytes,
            gaps,
            command,
        });
        Self::reconcile(&mut state);
        self.changed.notify_all();
        Ok(ticket)
    }

    fn wait(&self, ticket: u64, schedule: &impl Schedule) -> io::Result<()> {
        let mut state = self.lock();
        if state.completed >= ticket {
            return Ok(());
        }
        state.drainers += 1;
        self.changed.notify_all();
        while state.completed < ticket {
            if state.stopped {
                state.drainers -= 1;
                return Err(io::Error::other("spool writer stopped before drain"));
            }
            self.hand_turn(&state, schedule);
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        state.drainers -= 1;
        Ok(())
    }
}

pub trait Schedule: Clone + Send + 'static {
    fn take(&self, _needs_turn: impl FnOnce() -> bool) -> bool {
        true
    }

    fn grant(&self, _idle_with_jobs: impl FnOnce() -> bool) -> bool {
        false
    }
}

#[derive(Clone, Copy)]
pub struct Unrestricted;

impl Schedule for Unrestricted {}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone)]
pub struct FixtureSchedule(Option<Arc<std::sync::atomic::AtomicBool>>);

#[cfg(any(test, feature = "test-support"))]
impl FixtureSchedule {
    pub(crate) fn new(lockstep: bool) -> Self {
        Self(lockstep.then(|| Arc::new(std::sync::atomic::AtomicBool::new(false))))
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Schedule for FixtureSchedule {
    fn take(&self, needs_turn: impl FnOnce() -> bool) -> bool {
        self.0.as_ref().is_none_or(|turn| {
            !needs_turn() || turn.swap(false, Ordering::Relaxed)
        })
    }

    fn grant(&self, idle_with_jobs: impl FnOnce() -> bool) -> bool {
        self.0.as_ref().is_some_and(|turn| {
            idle_with_jobs() && !turn.swap(true, Ordering::Relaxed)
        })
    }
}

pub struct Writer<S: Schedule = Unrestricted> {
    schedule: S,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}
impl Writer {
    pub(crate) fn start(root: PathBuf) -> io::Result<Self> {
        Self::start_scheduled(root, Unrestricted)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn start_with<B: SegmentBackend + 'static>(
        create: impl FnMut(&CampaignId) -> io::Result<B> + Send + 'static,
    ) -> io::Result<Self> {
        Self::start_backend(create, Unrestricted)
    }
}

impl<S: Schedule> Writer<S> {
    pub(crate) fn start_scheduled(root: PathBuf, schedule: S) -> io::Result<Self> {
        let factory_bytes = root.capacity();
        let writer = Self::start_backend(move |campaign| {
            let id = std::str::from_utf8(campaign).map_err(|_| io::ErrorKind::InvalidData)?;
            FileBackend::ordered(root.join(id))
        }, schedule)?;
        {
            let mut state = writer.shared.lock();
            state.factory_bytes = factory_bytes;
            Shared::reconcile(&mut state);
        }
        Ok(writer)
    }

    fn start_backend<B: SegmentBackend + 'static>(
        create: impl FnMut(&CampaignId) -> io::Result<B> + Send + 'static,
        schedule: S,
    ) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::default(),
            changed: Condvar::new(),
            pipeline: OnceLock::new(),
        });
        let worker = Arc::clone(&shared);
        let worker_schedule = schedule.clone();
        let thread = std::thread::Builder::new()
            .name("replay-spool-writer".into())
            .stack_size(collector_workers::priority::BACKGROUND_STACK_BYTES)
            .spawn(move || {
                collector_workers::private_heap::off_hold_path();
                // SAFETY: This dedicated spool writer thread registers its own role.
                unsafe {
                    collector_workers::thread_cpu::register(
                        collector_workers::thread_cpu::Role::SpoolWriter,
                    );
                }
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    serve(&worker, create, &worker_schedule)
                }));
                // SAFETY: This dedicated spool writer thread clears its own role before exit.
                unsafe {
                    collector_workers::thread_cpu::register_id(
                        collector_workers::thread_cpu::Role::SpoolWriter,
                        0,
                    );
                }
                if result.is_err() {
                    collector_memory::terminal::fail(
                        collector_memory::terminal::Phase::Spool,
                        collector_memory::terminal::Code::Thread,
                    );
                }
                {
                    let mut state = worker.lock();
                    state.stopped = true;
                    state.backend_bytes = 0;
                    state.factory_bytes = 0;
                    Shared::reconcile(&mut state);
                }
                worker.changed.notify_all();
            })
            .map_err(engine_allocation::allocation_error)?;
        Ok(Self {
            schedule,
            shared,
            thread: Some(thread),
        })
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
        let id = unsafe { windows_sys::Win32::System::Threading::GetThreadId(handle as _) };
        (id != 0).then_some(id)
    }
    pub(crate) fn account_to(&self, counter: engine_allocation::SharedBox<AtomicU64>) {
        let mut state = self.shared.lock();
        assert!(state.pool.counter.is_none());
        state.pool.bytes = 0;
        state.pool.counter = Some(counter);
        Shared::reconcile(&mut state);
    }
    pub(crate) fn ledger_storage(&self, bytes: usize) {
        let mut state = self.shared.lock();
        state.ledger_bytes = bytes;
        Shared::reconcile(&mut state);
    }
    pub(crate) fn ready_epoch(&self) -> io::Result<Option<u64>> {
        let state = self.shared.lock();
        if state.stopped {
            return Err(io::Error::other("spool writer stopped"));
        }
        Ok((!state.blocked).then_some(state.failure_epoch))
    }
    pub(crate) fn failure_epoch(&self) -> u64 {
        self.shared.lock().failure_epoch
    }
    pub(crate) fn try_receipt_storage(
        &self,
        owned_bytes: usize,
        ledger_bytes: usize,
        write_bytes: usize,
    ) -> io::Result<Option<ReceiptStorage>> {
        let bytes = owned_bytes
            .checked_add(ledger_bytes)
            .ok_or_else(engine_allocation::oom)?;
        let mut state = self.shared.lock();
        if state.stopped || collector_memory::terminal::failed() {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        let admission = bytes
            .checked_add(write_bytes)
            .ok_or_else(engine_allocation::oom)?;
        let budget = budget_for(state.diagnostics.budget, admission);
        if bytes > budget / 2 {
            return Err(engine_allocation::oom());
        }
        if state.diagnostics.deferred_bytes > budget / 2 - bytes
            || state.diagnostics.bytes > budget - admission
        {
            return Ok(None);
        }
        state.diagnostics.budget = budget;
        state.diagnostics.deferred_bytes += bytes;
        state.deferred_owned_bytes += owned_bytes;
        Shared::reconcile(&mut state);
        Ok(Some(ReceiptStorage {
            shared: Arc::clone(&self.shared),
            bytes,
            owned_bytes,
        }))
    }
    pub(crate) fn wait_receipt_progress(&self) -> io::Result<()> {
        let mut state = self.shared.lock();
        state.diagnostics.waiting += 1;
        self.shared.changed.notify_all();
        while state.completions.is_empty() && !state.stopped {
            self.shared.hand_turn(&state, &self.schedule);
            state = self
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        state.diagnostics.waiting -= 1;
        if state.stopped {
            Err(io::Error::other("spool writer stopped before receipt"))
        } else {
            Ok(())
        }
    }

    pub fn output(&self, campaign: CampaignId) -> Output<FileBackend, S> {
        Output::Queued {
            schedule: self.schedule.clone(),
            shared: Arc::clone(&self.shared),
            campaign,
            generation: 0,
            open: false,
            last_ticket: None,
            last_close: None,
        }
    }
    pub fn drain(&self) -> io::Result<()> {
        let last = self.shared.lock().next_ticket - 1;
        self.shared.wait(last, &self.schedule)
    }
    pub(crate) fn resume_after_failure(&self) {
        self.shared.lock().blocked = false;
    }
    pub fn completion(&self) -> Option<Completion> {
        let mut state = self.shared.lock();
        let completion = state.completions.pop_front();
        Shared::reconcile(&mut state);
        self.shared.changed.notify_all();
        completion
    }
    #[cfg(test)]
    pub(crate) fn test_budget(&self, budget: usize) {
        let mut state = self.shared.lock();
        state.diagnostics.budget = budget;
    }
    #[cfg(test)]
    pub(crate) fn test_uncommitted_hours(&self) -> Vec<i32> {
        self.shared
            .lock()
            .frames
            .iter()
            .map(|frame| frame.context.failed_hour.unwrap())
            .collect()
    }
    #[cfg(test)]
    pub(crate) fn test_bytes(&self) -> usize {
        self.shared.lock().diagnostics.bytes
    }
    #[cfg(test)]
    pub(crate) fn test_backpressure_probe(&self) -> impl FnOnce() + Send + use<S> {
        let shared = Arc::clone(&self.shared);
        move || {
            let mut state = shared.lock();
            while state.diagnostics.waiting == 0 {
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            assert!(state.diagnostics.bytes <= state.diagnostics.budget);
        }
    }
    #[cfg(test)]
    pub(crate) fn test_storage_probe(
        &self,
        baseline: u64,
    ) -> impl FnOnce() -> (usize, usize) + Send + use<S> {
        let shared = Arc::clone(&self.shared);
        move || {
            let mut state = shared.lock();
            while state.diagnostics.waiting == 0 {
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
            assert!(state.diagnostics.bytes <= state.diagnostics.budget);
            assert!(state.diagnostics.deferred_bytes <= state.diagnostics.budget / 2);
            assert_eq!(
                state.pool.counter.as_ref().unwrap().load(Ordering::Relaxed),
                baseline + state.diagnostics.owned_bytes as u64
            );
            (
                state.diagnostics.deferred_bytes,
                state.diagnostics.owned_bytes,
            )
        }
    }
    #[cfg(test)]
    pub(crate) fn test_storage(&self) -> (usize, usize) {
        let state = self.shared.lock();
        (
            state.diagnostics.deferred_bytes,
            state.diagnostics.owned_bytes,
        )
    }
    #[cfg(test)]
    pub(crate) fn test_drain_probe(&self) -> impl FnOnce() + Send + use<S> {
        let shared = Arc::clone(&self.shared);
        move || {
            let mut state = shared.lock();
            while state.drainers == 0 {
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        }
    }

    pub(crate) fn shutdown(&mut self) {
        self.shared.lock().stop = true;
        self.shared.changed.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl<S: Schedule> Drop for Writer<S> {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub enum Output<B: SegmentBackend = FileBackend, S: Schedule = Unrestricted> {
    #[cfg_attr(not(test), allow(dead_code))]
    Direct(SegmentedSpool<B>),
    Queued {
        schedule: S,
        shared: Arc<Shared>,
        campaign: CampaignId,
        generation: u64,
        open: bool,
        last_ticket: Option<u64>,
        last_close: Option<CloseReceipt>,
    },
}
impl<B: SegmentBackend, S: Schedule> Output<B, S> {
    pub(crate) fn is_queued(&self) -> bool {
        matches!(self, Self::Queued { .. })
    }
    pub(crate) fn is_open(&self) -> bool {
        match self {
            Self::Direct(spool) => spool.is_open(),
            Self::Queued { open, .. } => *open,
        }
    }
    pub(crate) fn last_ticket(&self) -> Option<u64> {
        match self {
            Self::Direct(_) => None,
            Self::Queued { last_ticket, .. } => *last_ticket,
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn committed_watermark(&self) -> Option<(u32, u64)> {
        match self {
            Self::Direct(spool) => spool.committed_watermark(),
            Self::Queued {
                shared, campaign, ..
            } => shared
                .lock()
                .watermarks
                .iter()
                .find(|(id, _)| id == campaign)
                .and_then(|(_, watermark)| *watermark),
        }
    }
    pub(crate) fn stage_frame(
        context: Vec<Record>,
        frame: EncodedFrame,
        restarting: bool,
    ) -> io::Result<StagedFrame> {
        let mut records = engine_allocation::vec(context.len() + 1)?;
        for record in context {
            records.push(SealedRecord::context(record)?);
        }
        records.push(SealedRecord::frame(frame)?);
        Ok(StagedFrame {
            records,
            restarting,
        })
    }
    pub(crate) fn write_frame(
        &mut self,
        staged: StagedFrame,
        receipt_bytes: usize,
    ) -> io::Result<()> {
        let StagedFrame {
            records,
            restarting,
        } = staged;
        match self {
            Self::Direct(spool) => {
                if restarting {
                    spool.finish_segment()?;
                    spool.start_sealed_segment(records)
                } else {
                    records
                        .into_iter()
                        .try_for_each(|record| spool.append_sealed(record))
                }
            }
            Self::Queued {
                schedule,
                shared,
                campaign,
                generation,
                open,
                last_ticket,
                last_close,
            } => {
                let next_generation = if restarting {
                    generation
                        .checked_add(1)
                        .ok_or_else(|| io::Error::other("spool generation exhausted"))?
                } else {
                    *generation
                };
                let ticket = shared.submit(
                    *campaign,
                    next_generation,
                    if restarting {
                        Command::StartSegment {
                            records,
                            frame: true,
                        }
                    } else {
                        Command::Records {
                            records,
                            frame: true,
                        }
                    },
                    receipt_bytes,
                    schedule,
                )?;
                *generation = next_generation;
                *last_ticket = Some(ticket);
                *last_close = None;
                *open = true;
                Ok(())
            }
        }
    }
    pub(crate) fn start_context(&mut self, context: Vec<Record>) -> io::Result<()> {
        let mut sealed = engine_allocation::vec(context.len())?;
        for record in context {
            sealed.push(SealedRecord::context(record)?);
        }
        match self {
            Self::Direct(spool) => spool.start_sealed_context(sealed),
            Self::Queued {
                schedule,
                shared,
                campaign,
                generation,
                open,
                last_close,
                ..
            } => {
                let next = generation
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("spool generation exhausted"))?;
                shared.submit(
                    *campaign,
                    next,
                    Command::StartSegment {
                        records: sealed,
                        frame: false,
                    },
                    0,
                    schedule,
                )?;
                *generation = next;
                *last_close = None;
                *open = true;
                Ok(())
            }
        }
    }
    pub fn append(&mut self, record: Record) -> io::Result<()> {
        if let Self::Direct(spool) = self {
            return spool.append(&record);
        }
        let mut context = engine_allocation::vec(1)?;
        context.push(record);
        self.append_context(context)
    }
    pub(crate) fn append_tracked(
        &mut self,
        record: Record,
        receipt_bytes: usize,
    ) -> io::Result<Option<u64>> {
        match self {
            Self::Direct(spool) => spool.append(&record).map(|()| None),
            Self::Queued {
                schedule,
                shared,
                campaign,
                generation,
                ..
            } => {
                let mut records = engine_allocation::vec(1)?;
                records.push(SealedRecord::context(record)?);
                shared
                    .submit(
                        *campaign,
                        *generation,
                        Command::Records {
                            records,
                            frame: false,
                        },
                        receipt_bytes,
                        schedule,
                    )
                    .map(Some)
            }
        }
    }
    pub(crate) fn append_context(&mut self, context: Vec<Record>) -> io::Result<()> {
        match self {
            Self::Direct(spool) => context.iter().try_for_each(|record| spool.append(record)),
            Self::Queued {
                schedule,
                shared,
                campaign,
                generation,
                ..
            } => {
                let mut records = engine_allocation::vec(context.len())?;
                for record in context {
                    records.push(SealedRecord::context(record)?);
                }
                shared
                    .submit(
                        *campaign,
                        *generation,
                        Command::Records {
                            records,
                            frame: false,
                        },
                        0,
                        schedule,
                    )
                    .map(|_| ())
            }
        }
    }
    pub(crate) fn try_close_context(
        &mut self,
        context: Vec<Record>,
        restarting: bool,
    ) -> io::Result<CloseReceipt> {
        let Self::Queued {
            schedule,
            shared,
            campaign,
            generation,
            open,
            last_close,
            ..
        } = self
        else {
            return Err(io::Error::other("history requests require queued output"));
        };
        if !*open
            && !restarting
            && let Some(receipt) = last_close
        {
            return Ok(receipt.clone());
        }
        let next = generation
            .checked_add(u64::from(restarting))
            .ok_or_else(|| io::Error::other("spool generation exhausted"))?;
        let mut records = engine_allocation::vec(context.len())?;
        for record in context {
            records.push(SealedRecord::context(record)?);
        }
        let mut receipt = CloseReceipt::default();
        receipt.ticket = shared.submit_with_wait(
            *campaign,
            next,
            Command::CloseContext {
                records,
                restarting,
                receipt: receipt.clone(),
            },
            0,
            false,
            schedule,
        )?;
        *generation = next;
        *last_close = Some(receipt.clone());
        *open = false;
        Ok(receipt)
    }

    pub(crate) fn finish_segment(&mut self) -> io::Result<()> {
        self.close_segment().map(|_| ())
    }
    pub(crate) fn finish_stopped_segment(&mut self) -> io::Result<()> {
        let receipt = self.close_segment()?;
        if let Self::Queued { shared, schedule, .. } = self {
            shared.wait(receipt.ticket, schedule)?;
        }
        receipt
            .result()
            .unwrap_or(Err(io::ErrorKind::Other))
            .map_err(io::Error::from)
    }
    pub(crate) fn close_segment(&mut self) -> io::Result<CloseReceipt> {
        match self {
            Self::Direct(spool) => {
                let receipt = CloseReceipt::default();
                let result = spool.finish_segment();
                // SAFETY: This is the close result from its originating writer, or an isolated
                // receipt owned by the fixture.
                unsafe { receipt.complete(result.as_ref().map(|_| ()).map_err(io::Error::kind)) };
                result.map(|_| receipt)
            }
            Self::Queued {
                schedule,
                shared,
                campaign,
                generation,
                open,
                last_close,
                ..
            } => {
                if !*open && let Some(receipt) = last_close {
                    return Ok(receipt.clone());
                }
                let mut receipt = CloseReceipt::default();
                receipt.ticket = shared.submit(
                    *campaign,
                    *generation,
                    Command::FinishSegment(receipt.clone()),
                    0,
                    schedule,
                )?;
                *last_close = Some(receipt.clone());
                *open = false;
                Ok(receipt)
            }
        }
    }
    pub fn invalidate(&mut self) {
        if let Self::Queued { open, .. } = self {
            *open = false;
        }
    }
}

fn serve<B: SegmentBackend>(
    shared: &Shared,
    mut create: impl FnMut(&CampaignId) -> io::Result<B>,
    schedule: &impl Schedule,
) {
    let _phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Spool);
    // SAFETY: This owned worker applies its session's frame-pipeline priority policy to itself.
    let mut priority = unsafe { collector_workers::priority::OwnPriority::frame_pipeline() };
    let mut spools: Vec<(CampaignId, u64, SegmentedSpool<B>)> = Vec::new();
    loop {
        if let Some(pipeline) = shared.pipeline.get() {
            // SAFETY: This owned worker updates its priority from its session's own pipeline.
            unsafe {
                priority.follow_pipeline(
                    pipeline,
                    collector_workers::frame_priority::Role::SpoolWriter,
                );
            }
        }
        let job = {
            let mut state = shared.lock();
            loop {
                if schedule.take(|| !state.stop && !state.jobs.is_empty())
                    && let Some(job) = state.jobs.pop_front()
                {
                    break Some(job);
                }
                if state.stop {
                    break None;
                }
                let (next, timeout) = shared
                    .changed
                    .wait_timeout(state, Duration::from_millis(100))
                    .unwrap_or_else(PoisonError::into_inner);
                state = next;
                if timeout.timed_out() {
                    break None;
                }
            }
        };
        let Some(job) = job else {
            collector_workers::private_heap::PRIVATE_HEAP.maybe_flush_thread_cache();
            if shared.lock().stop {
                break;
            }
            for (campaign, generation, spool) in &mut spools {
                collector_memory::terminal::set_context(Context {
                    campaign_id: Some(*campaign),
                    ..Context::default()
                });
                if let Err(error) = spool.sync_if_due() {
                    let mut state = shared.lock();
                    state.fail(*campaign, *generation);
                    state.blocked = true;
                    state.failure_epoch += 1;
                    state.completions.push_back(Completion {
                        ticket: 0,
                        context: collector_memory::terminal::context(),
                        frame: false,
                        committed: false,
                        result: Err(error),
                    });
                    cancel_queued(&mut state, io::ErrorKind::Other);
                    shared.changed.notify_all();
                }
            }
            continue;
        };
        collector_memory::terminal::set_context(job.context);
        let is_frame = matches!(
            &job.command,
            Command::StartSegment { frame: true, .. } | Command::Records { frame: true, .. }
        );
        let close = match &job.command {
            Command::FinishSegment(receipt) | Command::CloseContext { receipt, .. } => {
                Some(receipt.clone())
            }
            _ => None,
        };
        let mut committed_context = false;
        let mut committed = false;
        let result = (|| {
            if shared.lock().failed(job.campaign, job.generation) {
                return Err(io::Error::other("spool segment generation failed"));
            }
            if !spools.iter().any(|(id, _, _)| *id == job.campaign) {
                let mut spool = SegmentedSpool::new(create(&job.campaign)?);
                if engine_allocation::capture_active() {
                    spool.set_commit_hook(crate::spooling::committed_spool_record);
                    spool.set_write_start_hook(collector_memory::terminal::record_start);
                }
                engine_allocation::reserve(&mut spools, 1)?;
                spools.push((job.campaign, job.generation, spool));
                let mut state = shared.lock();
                state.backend_bytes = spools.capacity()
                    * size_of::<(CampaignId, u64, SegmentedSpool<B>)>()
                    + spools
                        .iter()
                        .map(|(_, _, spool)| spool.allocated_bytes())
                        .sum::<usize>();
                Shared::reconcile(&mut state);
            }
            let (_, generation, spool) = spools
                .iter_mut()
                .find(|(id, _, _)| *id == job.campaign)
                .unwrap();
            let before_commit = spool.committed_watermark();
            let before_frames = spool.committed_frames();
            let result = match job.command {
                Command::StartSegment { records, frame } => {
                    *generation = job.generation;
                    spool.finish_segment().and_then(|_| {
                        if frame {
                            spool.start_sealed_segment(records)
                        } else {
                            spool.start_sealed_context(records)
                        }
                    })
                }
                Command::Records { records, .. } => {
                    if *generation != job.generation {
                        Err(io::Error::other("stale spool segment generation"))
                    } else {
                        records
                            .into_iter()
                            .try_for_each(|record| spool.append_sealed(record))
                    }
                }
                Command::CloseContext {
                    records,
                    restarting,
                    ..
                } => {
                    if restarting {
                        *generation = job.generation;
                        spool
                            .finish_segment()
                            .and_then(|()| spool.start_sealed_context(records))
                            .and_then(|()| spool.finish_segment())
                    } else if *generation != job.generation {
                        Err(io::Error::other("stale spool close generation"))
                    } else {
                        records
                            .into_iter()
                            .try_for_each(|record| spool.append_sealed(record))
                            .and_then(|()| spool.finish_segment())
                    }
                }
                Command::FinishSegment(_) => {
                    if *generation != job.generation {
                        Err(io::Error::other("stale spool close generation"))
                    } else {
                        spool.finish_segment()
                    }
                }
            };
            committed = is_frame && spool.committed_frames() > before_frames;
            if spool.committed_watermark() != before_commit {
                committed_context = result.is_ok();
            }
            if result.is_err() {
                let _ = spool.finish_segment();
            }
            result
        })();
        let mut state = shared.lock();
        state.backend_bytes = spools.capacity() * size_of::<(CampaignId, u64, SegmentedSpool<B>)>()
            + spools
                .iter()
                .map(|(_, _, spool)| spool.allocated_bytes())
                .sum::<usize>();
        if let Some((_, _, spool)) = spools.iter().find(|(id, _, _)| *id == job.campaign)
            && let Some((_, watermark)) = state
                .watermarks
                .iter_mut()
                .find(|(id, _)| id == &job.campaign)
        {
            *watermark = spool.committed_watermark();
        }
        if let Some(receipt) = close {
            // SAFETY: This is the close result from its originating writer, or an isolated receipt
            // owned by the fixture.
            unsafe { receipt.complete(result.as_ref().map(|_| ()).map_err(io::Error::kind)) };
        }
        let failed = result.as_ref().err().map(io::Error::kind);
        if failed.is_some() {
            state.fail(job.campaign, job.generation);
            state.blocked = true;
            state.failure_epoch += 1;
        }
        if committed || committed_context {
            state.frames.retain(|frame| {
                if frame.context.campaign_id != Some(job.campaign) || frame.ticket > job.ticket {
                    return true;
                }
                if committed {
                    return false;
                }
                !job.gaps.iter().any(|&(branch, first, last)| {
                    frame.context.branch == Some(branch)
                        && frame
                            .context
                            .failed_hour
                            .is_some_and(|hour| (first..=last).contains(&hour))
                })
            });
            Shared::publish_range(&mut state);
        }
        state.payload_bytes -= job.payload_bytes;
        state.unfinished_jobs -= 1;
        state.completed = job.ticket;
        state.completions.push_back(Completion {
            ticket: job.ticket,
            context: job.context,
            frame: is_frame,
            committed,
            result,
        });
        if let Some(kind) = failed {
            cancel_queued(&mut state, kind);
        }
        Shared::reconcile(&mut state);
        shared.changed.notify_all();
    }
    for (campaign, _, mut spool) in spools {
        collector_memory::terminal::set_context(Context {
            campaign_id: Some(campaign),
            ..Context::default()
        });
        let _ = spool.finish_segment();
    }
}

fn cancel_queued(state: &mut State, kind: io::ErrorKind) {
    while let Some(job) = state.jobs.pop_front() {
        state.fail(job.campaign, job.generation);
        let frame = matches!(
            &job.command,
            Command::StartSegment { frame: true, .. } | Command::Records { frame: true, .. }
        );
        if let Command::FinishSegment(receipt) | Command::CloseContext { receipt, .. } =
            &job.command
        {
            // SAFETY: This is the close result from its originating writer, or an isolated receipt
            // owned by the fixture.
            unsafe { receipt.complete(Err(kind)) };
        }
        state.payload_bytes -= job.payload_bytes;
        state.unfinished_jobs -= 1;
        state.completed = job.ticket;
        state.completions.push_back(Completion {
            ticket: job.ticket,
            context: job.context,
            frame,
            committed: false,
            result: Err(io::Error::from(kind)),
        });
    }
    Shared::reconcile(state);
}

fn same_context(left: Context, right: Context) -> bool {
    left.campaign_id == right.campaign_id
        && left.branch == right.branch
        && left.load_signal == right.load_signal
}

fn budget_for(current: usize, record: usize) -> usize {
    current.max(record.saturating_mul(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_close_defers_when_writer_is_busy_without_partially_submitting_context() {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            pipeline: OnceLock::new(),
        });
        let mut output: Output = Output::Queued {
            schedule: Unrestricted,
            shared: Arc::clone(&shared),
            campaign: *b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6",
            generation: 1,
            open: true,
            last_ticket: None,
            last_close: None,
        };
        let guard = shared.lock();
        assert_eq!(
            output
                .try_close_context(Vec::new(), false)
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        drop(guard);
        shared.lock().diagnostics.bytes = MIN_BUDGET;
        assert_eq!(
            output
                .try_close_context(Vec::new(), false)
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(output.is_open());
        assert!(shared.lock().jobs.is_empty());
        shared.lock().diagnostics.bytes = 0;
        let receipt = output.try_close_context(Vec::new(), false).unwrap();
        assert!(!output.is_open());
        assert!(receipt.result().is_none());
        let mut state = shared.lock();
        assert_eq!(state.jobs.len(), 1);
        cancel_queued(&mut state, io::ErrorKind::Interrupted);
        assert_eq!(receipt.result(), Some(Err(io::ErrorKind::Interrupted)));
    }

    #[test]
    fn terminal_ranges_keep_each_context_when_a_campaign_returns_before_its_writes_finish() {
        let a = *b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
        let b = *b"31b7cc84-3028-4f65-8e0d-adf2f37ca1a6";
        let mut state = State::default();
        state.watermarks = vec![(a, Some((2, 1024))), (b, None)];
        for (index, (campaign, branch, load, hour)) in
            [(a, 1, 7, 11), (b, 1, 8, 20), (a, 1, 7, 12), (a, 2, 9, 30)]
                .into_iter()
                .enumerate()
        {
            let context = Context {
                campaign_id: Some(campaign),
                branch: Some(branch),
                load_signal: Some(load),
                failed_hour: Some(hour),
            };
            if !state
                .contexts
                .iter()
                .any(|saved| same_context(*saved, context))
            {
                state.contexts.push(context);
            }
            state.frames.push(FrameWitness {
                ticket: index as u64 + 1,
                context,
            });
        }
        let ranges = Shared::outstanding(&state).collect::<Vec<_>>();
        assert_eq!(ranges.len(), 3);
        assert_eq!(ranges[0].range.campaign_id, a);
        assert_eq!(
            (ranges[0].range.first_hour, ranges[0].range.last_hour),
            (11, 12)
        );
        assert_eq!(ranges[0].committed_segment, Some(2));
        assert_eq!(ranges[0].committed_bytes, 1024);
        assert_eq!(ranges[1].range.campaign_id, b);
        assert_eq!(
            (ranges[1].committed_segment, ranges[1].committed_bytes),
            (None, 0)
        );
        assert_eq!(ranges[2].range.branch, 2);
        assert_eq!(
            (ranges[2].range.first_hour, ranges[2].range.last_hour),
            (30, 30)
        );
        state.frames.retain(|frame| frame.ticket != 1);
        state.retire_contexts();
        let ranges = Shared::outstanding(&state).collect::<Vec<_>>();
        assert_eq!(ranges[0].range.campaign_id, a);
        assert_eq!(
            (ranges[0].range.first_hour, ranges[0].range.last_hour),
            (12, 12)
        );
        assert_eq!(ranges[1].range.campaign_id, b);
        state.frames.retain(|frame| frame.ticket != 3);
        state.retire_contexts();
        let ranges = Shared::outstanding(&state).collect::<Vec<_>>();
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].range.campaign_id, b);
        assert_eq!(ranges[1].range.branch, 2);
    }

    #[test]
    fn a_lockstep_writer_runs_one_job_for_each_turn_a_waiting_thread_hands_it() {
        let root = std::env::temp_dir().join(format!("hoi4-spool-lockstep-{}", std::process::id()));
        let writer = Writer::start_scheduled(root.clone(), FixtureSchedule::new(true)).unwrap();
        let first = writer
            .output(*b"21b7cc84-3028-4f65-8e0d-adf2f37ca1a6")
            .close_segment()
            .unwrap();
        let second = writer
            .output(*b"31b7cc84-3028-4f65-8e0d-adf2f37ca1a6")
            .close_segment()
            .unwrap();
        assert_eq!(writer.shared.lock().completed, 0, "nothing waited");
        writer.shared.wait(first.ticket, &writer.schedule).unwrap();
        {
            let state = writer.shared.lock();
            assert_eq!(state.completed, first.ticket);
            assert_eq!(
                state.jobs.len(),
                1,
                "the writer ran a job nothing handed it"
            );
        }
        assert_eq!(first.result(), Some(Ok(())));
        assert_eq!(second.result(), None);
        writer.drain().unwrap();
        assert_eq!(second.result(), Some(Ok(())));
        drop(writer);
        let _ = std::fs::remove_dir_all(root);
    }

}
