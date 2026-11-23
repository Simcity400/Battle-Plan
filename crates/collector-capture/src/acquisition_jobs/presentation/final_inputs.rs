use super::*;

pub(crate) struct FinalInputsDispatch {
    executor: Weak<collector_workers::acquisition::Pool<Job, Report>>,
    calibration: Arc<SharedCalibration>,
    state: Mutex<State>,
}
#[derive(Default)]
struct State {
    closed: bool,
    seen: usize,
    published: usize,
    required: usize,
    failed: bool,
    request: Option<crate::extended::FinalInputsRequest>,
    pending: Option<RangeTicket>,
}
impl FinalInputsDispatch {
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
        }
    }
    pub(in crate::acquisition_jobs) fn open(&self) {
        let mut state = lock_unpoisoned(&self.state);
        assert!(state.closed && state.request.is_none() && state.pending.is_none());
        *state = State::default();
    }
    pub(super) fn store(&self, part: usize, report: &Result<Report, Failure>) {
        let mut state = lock_unpoisoned(&self.state);
        if state.closed {
            return;
        }
        state.seen |= 1 << part;
        state.failed |= !matches!(report, Ok(Report::Characters(report)) if report.raw_ok && !report.plan_failed);
    }
    pub(crate) fn arm(&self, request: crate::extended::FinalInputsRequest, required: usize) {
        let mut state = lock_unpoisoned(&self.state);
        assert!(!state.closed && state.request.is_none() && state.pending.is_none());
        state.request = Some(request);
        state.required = required;
        let wake = self.dispatch(&mut state);
        drop(state);
        if let Some(executor) = wake {
            executor.wake_workers(1);
        }
    }
    fn dispatch(
        &self,
        state: &mut State,
    ) -> Option<Arc<collector_workers::acquisition::Pool<Job, Report>>> {
        if state.closed
            || state.failed
            || state.request.is_none()
            || state.seen != state.required
            || state.published != state.required.count_ones() as usize
            || collector_memory::terminal::failed()
        {
            return None;
        }
        let executor = self.executor.upgrade()?;
        let request = state.request.take().unwrap();
        let pending = submit(&executor, &self.calibration, Job::FinalInputs(request));
        let wake = !pending.parked;
        state.pending = Some(pending);
        wake.then_some(executor)
    }
    pub fn take(
        &self,
    ) -> (
        Option<RangeTicket>,
        Option<crate::extended::FinalInputsRequest>,
    ) {
        let mut state = lock_unpoisoned(&self.state);
        state.closed = true;
        (state.pending.take(), state.request.take())
    }
    pub fn close(&self) {
        lock_unpoisoned(&self.state).closed = true;
    }
    pub fn drain(&self) {
        let pending = lock_unpoisoned(&self.state).pending.take();
        super::drain(&self.executor, &self.calibration, pending);
        *lock_unpoisoned(&self.state) = State {
            closed: true,
            ..Default::default()
        };
    }
}
impl collector_workers::acquisition::Publication for FinalInputsDispatch {
    fn failed(&self) {
        let mut state = lock_unpoisoned(&self.state);
        state.failed = true;
    }
    fn published(&self) {
        let mut state = lock_unpoisoned(&self.state);
        if state.closed {
            return;
        }
        state.published += 1;
        let wake = self.dispatch(&mut state);
        drop(state);
        if let Some(executor) = wake {
            executor.wake_workers(1);
        }
    }
}
