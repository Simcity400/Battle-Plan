use super::*;

/// The source packs return on the callback; the remaining character jobs
/// contribute only raw-owner verdicts, not native operands to FinalInputs.
pub(crate) struct FinalInputsDispatch {
    executor: Weak<acquisition::Pool<Job, Report>>,
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
    last_publish: i64,
    request: Option<crate::extended::FinalInputsRequest>,
    pending: Option<RangeTicket>,
    bytes: u64,
}
impl FinalInputsDispatch {
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
        }
    }
    pub(in crate::acquisition_jobs) fn open(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        assert!(state.closed && state.request.is_none() && state.pending.is_none());
        *state = State::default();
    }
    pub(super) fn store(&self, part: usize, report: &Result<Report, Failure>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return;
        }
        state.seen |= 1 << part;
        state.failed |= !matches!(report, Ok(Report::Characters(report)) if report.raw_ok && !report.plan_failed);
    }
    /// The complete owned source packs and every Late/history admission are
    /// installed before this call. No merely-ready prefix lends those packs.
    pub(crate) fn arm(
        &self,
        request: crate::extended::FinalInputsRequest,
        required: usize,
        last_publish: i64,
    ) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        assert!(!state.closed && state.request.is_none() && state.pending.is_none());
        state.bytes = request.allocated_bytes();
        state.request = Some(request);
        state.required = required;
        state.last_publish = state.last_publish.max(last_publish);
        let wake = self.dispatch(&mut state);
        drop(state);
        if let Some(executor) = wake {
            executor.wake_workers(1);
        }
    }
    fn dispatch(&self, state: &mut State) -> Option<Arc<acquisition::Pool<Job, Report>>> {
        if state.closed
            || state.failed
            || state.request.is_none()
            || state.seen != state.required
            || state.published != state.required.count_ones() as usize
            || crate::allocation::failed()
        {
            return None;
        }
        let executor = self.executor.upgrade()?;
        let request = state.request.take().unwrap();
        let pending = submit(
            &executor,
            &self.calibration,
            Job::FinalInputs(request),
            state.last_publish,
            3,
        );
        let wake = !pending.parked;
        state.pending = Some(pending);
        wake.then_some(executor)
    }
    pub(crate) fn take(
        &self,
    ) -> (
        Option<RangeTicket>,
        Option<crate::extended::FinalInputsRequest>,
        u64,
    ) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closed = true;
        (state.pending.take(), state.request.take(), state.bytes)
    }
    pub(crate) fn close(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
    }
    pub(crate) fn drain(&self) {
        let pending = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending
            .take();
        super::drain(&self.executor, &self.calibration, pending);
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = State {
            closed: true,
            ..Default::default()
        };
    }
}
impl acquisition::Publication for FinalInputsDispatch {
    fn failed(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.failed = true;
    }
    fn published(&self, at: i64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return;
        }
        state.published += 1;
        state.last_publish = state.last_publish.max(at);
        let wake = self.dispatch(&mut state);
        drop(state);
        if let Some(executor) = wake {
            executor.wake_workers(1);
        }
    }
}
