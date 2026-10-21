use super::*;
use crate::division_census::{Census, MAX_PARTS};

pub(crate) struct PresentationInputs {
    executor: Weak<acquisition::Pool<Job, Report>>,
    calibration: Arc<SharedCalibration>,
    state: Mutex<State>,
    hold_ticks: AtomicU64,
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
    movement: Option<PresentationMovementRequest>,
    speed_terms: Option<PresentationSpeedTermsRequest>,
    split: bool,
    bytes: usize,
    store_wait_ticks: u64,
    last_publish: i64,
    store: Option<Arc<division_presentation::held_store::Slot>>,
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
        }
    }
    pub(crate) fn arm(
        &self,
        request: PresentationRequest,
        count: usize,
        store: Option<Arc<division_presentation::held_store::Slot>>,
    ) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        assert!(state.closed && state.request.is_none() && state.pending.is_none());
        *state = State {
            remaining: count + 1,
            count,
            request: Some(request),
            store,
            ..Default::default()
        };
    }
    pub(super) fn store_master(&self, census: Census, report: Result<Report, Failure>) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).master = Some((census, report));
    }
    pub(super) fn store_part(&self, index: usize, census: Census, report: Result<Report, Failure>) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).parts[index] = Some((census, report));
    }
    /// Called only after every input ticket has published. An unnotified
    /// publication or rejected input keeps the original callback admission path.
    pub(crate) fn restore_master(&self, census: &mut Census) -> (bool, Option<&'static str>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closed = true;
        if let Some((captured, _)) = state.master.take() {
            *census = captured;
        }
        (state.joined, state.failure)
    }
    pub(crate) fn restore_part(&self, index: usize, census: &mut Census) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((captured, _)) = state.parts[index].take() {
            *census = captured;
        }
    }
    pub(crate) fn take_request(&self) -> Option<PresentationRequest> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .request
            .take()
    }
    pub(crate) fn store_wait_ticks(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .store_wait_ticks
    }
    pub(crate) fn hold_ticks(&self) -> u64 {
        self.hold_ticks.load(Ordering::Relaxed)
    }
    pub(crate) fn take_dispatched(
        &self,
    ) -> Option<(
        RangeTicket,
        PresentationMovementRequest,
        PresentationSpeedTermsRequest,
        usize,
        bool,
    )> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
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
impl acquisition::Publication for PresentationInputs {
    fn failed(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closed = true;
        state.joined = true;
        state.failure = Some("division input continuation panicked");
    }
    fn published(&self, at: i64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // Measure every publisher's hold, including early returns and unwind,
        // but neither its lock acquisition nor the successor wake.
        struct Hold<'a>(&'a AtomicU64, i64);
        impl Drop for Hold<'_> {
            fn drop(&mut self) {
                self.0.fetch_add(
                    sampling::qpc().saturating_sub(self.1).max(0) as u64,
                    Ordering::Relaxed,
                );
            }
        }
        let hold = Hold(&self.hold_ticks, sampling::qpc());
        if state.closed {
            return;
        }
        state.remaining -= 1;
        state.last_publish = state.last_publish.max(at);
        if state.remaining != 0 {
            return;
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
            return;
        }
        let Some(executor) = self.executor.upgrade() else {
            return;
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
        let mut request = state.request.take().unwrap();
        request.inputs = inputs;
        if crate::allocation::failed() {
            state.request = Some(request);
            return;
        }
        // Do not delay the held first burst on the preceding decode's loan.
        // The last input publisher waits here, beside the independent jobs.
        if let Some(store) = &state.store {
            let begin = sampling::qpc();
            request.army_stats = store.take();
            state.store_wait_ticks = sampling::qpc().saturating_sub(begin).max(0) as u64;
            if request.army_stats.is_some() {
                request.captured.enable_owned_settlement();
            }
        }
        let movement = request.take_movement();
        let speed_terms = request.take_speed_terms();
        state.bytes =
            request.allocated_bytes() + movement.allocated_bytes() + speed_terms.allocated_bytes();
        let divisions = request
            .inputs
            .as_deref()
            .map_or(0, |inputs| inputs.outcome().len().min(inputs.seeds().len()));
        state.split =
            division_presentation::status_parts_for(divisions, request.captured.status_workers) > 1;
        state.movement = Some(movement);
        state.speed_terms = Some(speed_terms);
        let job = if state.split {
            Job::PresentationPrepare(request)
        } else {
            Job::Presentation(request)
        };
        let pending = submit(&executor, &self.calibration, job, state.last_publish, 0);
        let wake = !pending.parked;
        state.pending = Some(pending);
        // Release the input handoff before waking the admitted successor.
        drop(hold);
        drop(state);
        if wake {
            executor.wake_workers(1);
        }
    }
}
