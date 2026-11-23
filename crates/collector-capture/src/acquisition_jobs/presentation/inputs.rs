use super::*;
use collector_divisions::division_census::{Census, MAX_PARTS};
use std::sync::MutexGuard;

pub(crate) struct PresentationInputs {
    executor: Weak<collector_workers::acquisition::Pool<Job, Report>>,
    calibration: Arc<SharedCalibration>,
    state: Mutex<State>,
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
    omit_status: bool,
    movement: Option<PresentationMovementRequest>,
    speed_terms: Option<PresentationSpeedTermsRequest>,
    split: bool,
    store: Option<division_presentation::held_store::Slot>,
}
impl std::fmt::Debug for PresentationInputs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PresentationInputs").finish_non_exhaustive()
    }
}
impl PresentationInputs {
    pub(in crate::acquisition_jobs) fn new(
        executor: &Arc<collector_workers::acquisition::Pool<Job, Report>>,
        calibration: &Arc<SharedCalibration>,
    ) -> Self {
        Self {
            executor: Arc::downgrade(executor),
            calibration: Arc::clone(calibration),
            state: Mutex::new(State {
                closed: true,
                ..Default::default()
            }),
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
        store: Option<division_presentation::held_store::Slot>,
        omit_status: bool,
    ) {
        let mut state = lock_unpoisoned(&self.state);
        assert!(state.closed && state.request.is_none() && state.pending.is_none());
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
        lock_unpoisoned(&self.state).master = Some((census, report));
    }
    pub(super) fn store_part(&self, index: usize, census: Census, report: Result<Report, Failure>) {
        lock_unpoisoned(&self.state).parts[index] = Some((census, report));
    }
    pub(crate) fn try_close(&self) -> Option<ClosedInputs<'_>> {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return None,
        };
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
            return None;
        }
        state.closed = true;
        Some(ClosedInputs(state))
    }

    pub fn close(&self) {
        lock_unpoisoned(&self.state).closed = true;
    }
    pub fn drain(&self) {
        let pending = lock_unpoisoned(&self.state).pending.take();
        super::drain(&self.executor, &self.calibration, pending);
        let mut state = lock_unpoisoned(&self.state);
        *state = State {
            closed: true,
            ..Default::default()
        };
    }
}

struct ReadyOnDrop<'a>(&'a Weak<collector_workers::acquisition::Pool<Job, Report>>);
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
impl collector_workers::acquisition::Publication for PresentationInputs {
    fn failed(&self) {
        let _ready = ReadyOnDrop(&self.executor);
        let _failure = FailureOnDrop(&self.state);
        #[cfg(test)]
        if let Some(hook) = self.before_failure.get() {
            hook();
        }
    }
    fn published(&self) {
        let _ready = ReadyOnDrop(&self.executor);
        let mut state = lock_unpoisoned(&self.state);
        let wake = self.publish_locked(&mut state);
        if !state.closed {
            state.remaining -= 1;
        }
        drop(state);
        if wake && let Some(executor) = self.executor.upgrade() {
            executor.wake_workers(1);
        }
    }
}
impl PresentationInputs {
    fn publish_locked(&self, state: &mut State) -> bool {
        if state.closed {
            return false;
        }
        if state.remaining != 1 {
            return false;
        }
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
        if state.omit_status {
            return false;
        }
        let request = state.request.as_mut().unwrap();
        request.inputs = inputs;
        if collector_memory::terminal::failed() {
            return false;
        }
        if let Some(store) = &state.store {
            #[cfg(test)]
            if let Some(hook) = self.before_store.get() {
                hook();
            }
            request.army_stats = store.take();
            if let Some(store) = request.army_stats.as_deref_mut() {
                store
                    .observations
                    .take(&collector_division_parts::army_stat_changes::Observer);
            }
            if request.army_stats.is_some() {
                request.captured.enable_owned_settlement();
            }
        }
        state.movement = Some(request.take_movement());
        state.speed_terms = Some(request.take_speed_terms());
        let divisions = request
            .inputs
            .as_deref()
            .map_or(0, |inputs| inputs.outcome().len().min(inputs.seeds().len()));
        state.split =
            division_presentation::status_parts_for(divisions, request.captured.status_workers) > 1;
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
        let pending = submit_retained(&executor, &self.calibration, &mut submission.job);
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
    pub(crate) fn take_dispatched(
        &mut self,
    ) -> Option<(
        RangeTicket,
        PresentationMovementRequest,
        PresentationSpeedTermsRequest,
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
            state.split,
        ))
    }
}
