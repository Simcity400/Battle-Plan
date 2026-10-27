use super::*;
use crate::division_census::{Census, MAX_PARTS};
use std::sync::MutexGuard;

pub(crate) struct PresentationInputs {
    executor: Weak<acquisition::Pool<Job, Report>>,
    calibration: Arc<SharedCalibration>,
    state: Mutex<State>,
    hold_ticks: AtomicU64,
    deferred_polls: AtomicU64,
    trace: Mutex<Option<acquisition::idle_diagnostics::Inputs>>,
    #[cfg(test)]
    before_store: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>>,
    #[cfg(test)]
    before_submit: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>>,
    #[cfg(test)]
    before_failure: std::sync::OnceLock<Box<dyn Fn() + Send + Sync>>,
}
#[derive(Default)]
struct State {
    closed: bool,
    remaining: usize,
    master: Option<(Census, Result<Report, Failure>)>,
    parts: [Option<(Census, Result<Report, Failure>)>; MAX_PARTS - 1],
    count: usize,
    joined: bool,
    failure: Option<&'static str>,
    request: Option<PresentationRequest>,
    pending: Option<RangeTicket>,
    /// The capture mode leaves the status side out: the last publisher joins
    /// the census and submits nothing.
    omit_status: bool,
    movement: Option<PresentationMovementRequest>,
    speed_terms: Option<PresentationSpeedTermsRequest>,
    split: bool,
    bytes: usize,
    store_wait_ticks: u64,
    last_publish: i64,
    store: Option<Arc<division_presentation::held_store::Slot>>,
    trace: Option<acquisition::idle_diagnostics::Inputs>,
}
impl std::fmt::Debug for PresentationInputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PresentationInputs").finish_non_exhaustive()
    }
}
impl PresentationInputs {
    pub(in crate::acquisition_jobs) fn new(
        executor: &Arc<acquisition::Pool<Job, Report>>,
        calibration: &Arc<SharedCalibration>,
    ) -> Self {
        Self {
            executor: Arc::downgrade(executor),
            calibration: Arc::clone(calibration),
            state: Mutex::new(State {
                closed: true,
                ..Default::default()
            }),
            hold_ticks: AtomicU64::new(0),
            deferred_polls: AtomicU64::new(0),
            trace: Mutex::new(None),
            #[cfg(test)]
            before_store: std::sync::OnceLock::new(),
            #[cfg(test)]
            before_submit: std::sync::OnceLock::new(),
            #[cfg(test)]
            before_failure: std::sync::OnceLock::new(),
        }
    }
    pub(crate) fn arm(
        &self,
        request: PresentationRequest,
        count: usize,
        store: Option<Arc<division_presentation::held_store::Slot>>,
        omit_status: bool,
    ) {
        let mut state = lock_callback(&self.state, Site::Inputs);
        assert!(state.closed && state.request.is_none() && state.pending.is_none());
        self.deferred_polls.store(0, Ordering::Relaxed);
        *state = State {
            remaining: count + 1,
            count,
            request: Some(request),
            store,
            omit_status,
            ..Default::default()
        };
    }
    pub(super) fn store_master(&self, census: Census, report: Result<Report, Failure>) {
        lock_callback(&self.state, Site::Inputs).master = Some((census, report));
    }
    pub(super) fn store_part(&self, index: usize, census: Census, report: Result<Report, Failure>) {
        lock_callback(&self.state, Site::Inputs).parts[index] = Some((census, report));
    }
    pub(crate) fn try_close(&self) -> Option<ClosedInputs<'_>> {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                self.deferred_polls.fetch_add(1, Ordering::Relaxed);
                return None;
            }
        };
        // Successful tickets can publish before their notifications run.
        if !state.closed
            && state.remaining != 0
            && state
                .master
                .as_ref()
                .is_some_and(|(_, report)| report.is_ok())
            && state.parts[..state.count]
                .iter()
                .all(|part| part.as_ref().is_some_and(|(_, report)| report.is_ok()))
        {
            self.deferred_polls.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        state.closed = true;
        Some(ClosedInputs(state))
    }

    pub(crate) fn hold_ticks(&self) -> u64 {
        self.hold_ticks.load(Ordering::Relaxed)
    }

    pub(crate) fn close(&self) {
        lock_callback(&self.state, Site::Inputs).closed = true;
    }
    pub(crate) fn drain(&self) {
        let pending = lock_callback(&self.state, Site::Inputs).pending.take();
        super::drain(&self.executor, &self.calibration, pending);
        let mut state = lock_callback(&self.state, Site::Inputs);
        if let Some(mut trace) = self
            .trace
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            trace.deferred_polls = self.deferred_polls.load(Ordering::Relaxed);
            acquisition::idle_diagnostics::update(|hour| hour.inputs = Some(trace));
        }
        *state = State {
            closed: true,
            ..Default::default()
        };
    }
}

struct ReadyOnDrop<'a>(&'a Weak<acquisition::Pool<Job, Report>>);
impl Drop for ReadyOnDrop<'_> {
    fn drop(&mut self) {
        if let Some(executor) = self.0.upgrade() {
            executor.notify_ready();
        }
    }
}
struct FailureOnDrop<'a>(&'a Mutex<State>);
impl Drop for FailureOnDrop<'_> {
    fn drop(&mut self) {
        let mut state = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.failure = Some("division input continuation panicked");
        state.joined = true;
        state.closed = true;
    }
}
impl acquisition::Publication for PresentationInputs {
    fn failed(&self) {
        let _ready = ReadyOnDrop(&self.executor);
        // Install the failure on return or unwind, before notifying readiness.
        let _failure = FailureOnDrop(&self.state);
        #[cfg(test)]
        if let Some(hook) = self.before_failure.get() {
            hook();
        }
    }
    fn published(&self, at: i64) {
        let _ready = ReadyOnDrop(&self.executor);
        let lock_begin = sampling::qpc();
        let mut state = lock_callback(&self.state, Site::Inputs);
        let acquired = sampling::qpc();
        struct Hold<'a>(&'a AtomicU64, i64);
        impl Drop for Hold<'_> {
            fn drop(&mut self) {
                self.0.fetch_add(
                    sampling::qpc().saturating_sub(self.1).max(0) as u64,
                    Ordering::Relaxed,
                );
            }
        }
        let hold = Hold(&self.hold_ticks, acquired);
        #[cfg(feature = "callback_profile")]
        let cycles = crate::callback_profile::thread_cycles();
        let wake = self.publish_locked(&mut state, at, lock_begin, acquired);
        if !state.closed {
            // A published ticket is not a settled continuation, especially on unwind.
            state.remaining -= 1;
        }
        let mut trace = state.trace.take();
        if let Some(trace) = &mut trace {
            trace.unlock_begin_qpc = sampling::qpc();
        }
        drop(hold);
        drop(state);
        if let Some(mut trace) = trace {
            trace.unlock_qpc = sampling::qpc();
            #[cfg(feature = "callback_profile")]
            {
                trace.thread_cycles = cycles
                    .zip(crate::callback_profile::thread_cycles())
                    .and_then(|(begin, end)| end.checked_sub(begin));
            }
            *self
                .trace
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(trace);
        }
        if wake && let Some(executor) = self.executor.upgrade() {
            executor.wake_workers(1);
        }
    }
}
impl PresentationInputs {
    fn publish_locked(&self, state: &mut State, at: i64, lock_begin: i64, acquired: i64) -> bool {
        if state.closed {
            return false;
        }
        state.last_publish = state.last_publish.max(at);
        if state.remaining != 1 {
            return false;
        }
        // Failed inputs retain the unchanged slot-ordered callback settlement.
        if !state
            .master
            .as_ref()
            .is_some_and(|(_, report)| report.is_ok())
            || state.parts[..state.count]
                .iter()
                .any(|part| !part.as_ref().is_some_and(|(_, report)| report.is_ok()))
        {
            return false;
        }
        let Some(executor) = self.executor.upgrade() else {
            return false;
        };
        state.trace = Some(acquisition::idle_diagnostics::Inputs {
            publisher_thread_id: unsafe {
                windows_sys::Win32::System::Threading::GetCurrentThreadId()
            },
            lock_begin_qpc: lock_begin,
            lock_acquired_qpc: acquired,
            ..Default::default()
        });
        let State {
            master,
            parts,
            count,
            failure,
            ..
        } = &mut *state;
        let (master, report) = master.as_mut().unwrap();
        let mut joining = true;
        for (part, _) in parts[..*count].iter_mut().flatten() {
            if joining {
                if let Err(reason) = master.absorb(part) {
                    *failure = Some(reason);
                }
                joining = failure.is_none() && master.joinable();
            }
        }
        let inputs_ok = matches!(report, Ok(Report::DivisionInputs(true))) && !master.overflowed;
        let inputs = if inputs_ok && failure.is_none() {
            master.unit_inputs().cloned()
        } else {
            None
        };
        state.joined = true;
        state.trace.as_mut().unwrap().merge_end_qpc = sampling::qpc();
        if state.omit_status {
            return false;
        }
        let request = state.request.as_mut().unwrap();
        request.inputs = inputs;
        if crate::allocation::failed() {
            return false;
        }
        // Do not delay the held first burst on the preceding decode's loan.
        // The last input publisher waits here, beside the independent jobs.
        if let Some(store) = &state.store {
            let begin = sampling::qpc();
            state.trace.as_mut().unwrap().store_begin_qpc = begin;
            #[cfg(test)]
            if let Some(hook) = self.before_store.get() {
                hook();
            }
            request.army_stats = store.take();
            let end = sampling::qpc();
            state.trace.as_mut().unwrap().store_end_qpc = end;
            state.store_wait_ticks = end.saturating_sub(begin).max(0) as u64;
            if request.army_stats.is_some() {
                request.captured.enable_owned_settlement();
            }
        }
        state.movement = Some(request.take_movement());
        state.speed_terms = Some(request.take_speed_terms());
        state.bytes = request.allocated_bytes()
            + state.movement.as_ref().unwrap().allocated_bytes()
            + state.speed_terms.as_ref().unwrap().allocated_bytes();
        let divisions = request
            .inputs
            .as_deref()
            .map_or(0, |inputs| inputs.outcome().len().min(inputs.seeds().len()));
        state.split =
            division_presentation::status_parts_for(divisions, request.captured.status_workers) > 1;
        state.trace.as_mut().unwrap().submit_begin_qpc = sampling::qpc();
        let request = state.request.take().unwrap();
        let job = if state.split {
            Job::PresentationPrepare(request)
        } else {
            Job::Presentation(request)
        };
        let mut submission = RetainedSubmission {
            request: &mut state.request,
            job: Some(job),
        };
        #[cfg(test)]
        if let Some(hook) = self.before_submit.get() {
            hook();
        }
        let pending = submit_retained(
            &executor,
            &self.calibration,
            &mut submission.job,
            state.last_publish,
            0,
        );
        let wake = !pending.parked;
        state.pending = Some(pending);
        wake
    }
}

struct RetainedSubmission<'a> {
    request: &'a mut Option<PresentationRequest>,
    job: Option<Job>,
}
impl Drop for RetainedSubmission<'_> {
    fn drop(&mut self) {
        if let Some(Job::Presentation(request) | Job::PresentationPrepare(request)) =
            self.job.take()
        {
            *self.request = Some(request);
        }
    }
}

#[cfg(test)]
mod tests;

pub(crate) struct ClosedInputs<'a>(MutexGuard<'a, State>);
impl ClosedInputs<'_> {
    // Late notifications and rejected inputs keep the callback fallback.
    pub(crate) fn restore_master(&mut self, census: &mut Census) -> (bool, Option<&'static str>) {
        let state = &mut self.0;
        state.closed = true;
        if let Some((captured, _)) = state.master.take() {
            *census = captured;
        }
        (state.joined, state.failure)
    }
    pub(crate) fn restore_part(&mut self, index: usize, census: &mut Census) {
        let state = &mut self.0;
        if let Some((captured, _)) = state.parts[index].take() {
            *census = captured;
        }
    }
    pub(crate) fn take_request(&mut self) -> Option<PresentationRequest> {
        let state = &mut self.0;
        let mut request = state.request.take()?;
        if let Some(movement) = state.movement.take() {
            request.captured.restore_movement(movement.pass);
        }
        if let Some(speed_terms) = state.speed_terms.take() {
            request.captured.restore_speed_terms(speed_terms.pass);
        }
        Some(request)
    }
    pub(crate) fn store_wait_ticks(&mut self) -> u64 {
        self.0.store_wait_ticks
    }
    pub(crate) fn take_dispatched(
        &mut self,
    ) -> Option<(
        RangeTicket,
        PresentationMovementRequest,
        PresentationSpeedTermsRequest,
        usize,
        bool,
    )> {
        let state = &mut self.0;
        Some((
            state.pending.take()?,
            state
                .movement
                .take()
                .expect("presentation movement retained"),
            state
                .speed_terms
                .take()
                .expect("presentation speed terms retained"),
            state.bytes,
            state.split,
        ))
    }
}
