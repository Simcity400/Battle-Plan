use super::*;

pub(crate) struct PresentationFanIn {
    executor: Weak<collector_workers::acquisition::Pool<Job, Report>>,
    calibration: Arc<SharedCalibration>,
    state: Mutex<State>,
}
struct State {
    admission_open: bool,
    closed: bool,
    remaining: usize,
    count: usize,
    parent: Option<PresentationRequest>,
    parts: [Option<(division_presentation::StatusWork, Result<Report, Failure>)>;
        division_presentation::MAX_STATUS_PARTS],
    pending: Option<RangeTicket>,
    failure: Option<Failure>,
    final_ranges: bool,
}
impl PresentationFanIn {
    pub(in crate::acquisition_jobs) fn new(
        executor: &Arc<collector_workers::acquisition::Pool<Job, Report>>,
        calibration: &Arc<SharedCalibration>,
    ) -> Self {
        Self {
            executor: Arc::downgrade(executor),
            calibration: Arc::clone(calibration),
            state: Mutex::new(State {
                admission_open: false,
                closed: true,
                remaining: 0,
                count: 0,
                parent: None,
                parts: std::array::from_fn(|_| None),
                pending: None,
                failure: None,
                final_ranges: false,
            }),
        }
    }
    pub(in crate::acquisition_jobs) fn open(&self) {
        let mut state = lock_unpoisoned(&self.state);
        assert!(state.closed && state.parent.is_none() && state.pending.is_none());
        state.admission_open = true;
    }
    pub(super) fn arm(&self, count: usize, final_ranges: bool) -> bool {
        let mut state = lock_unpoisoned(&self.state);
        if !state.admission_open {
            return false;
        }
        assert!(
            state.closed
                && state.parent.is_none()
                && state.pending.is_none()
                && state.parts.iter().all(Option::is_none)
        );
        state.closed = false;
        state.failure = None;
        state.count = count;
        state.remaining = count + 1;
        state.final_ranges = final_ranges;
        true
    }
    pub(super) fn store_parent(
        &self,
        request: &mut PresentationRequest,
        report: Result<Report, Failure>,
    ) {
        let mut state = lock_unpoisoned(&self.state);
        state.failure = report.err();
        state.parent = Some(PresentationRequest {
            countries: engine_allocation::SharedBox::clone(&request.countries),
            context: request.context,
            captured: std::mem::replace(
                &mut request.captured,
                division_presentation::Captured::vacant(),
            ),
            dispatch: if state.final_ranges {
                Default::default()
            } else {
                request.dispatch.successor()
            },
            inputs: request.inputs.take(),
            army_stats: request.army_stats.take(),
        });
    }
    pub(crate) fn store_part(
        &self,
        index: usize,
        part: Option<division_presentation::StatusWork>,
        report: Result<Report, Failure>,
    ) {
        lock_unpoisoned(&self.state).parts[index] = part.map(|part| (part, report));
    }
    pub fn take(
        &self,
    ) -> (
        Option<RangeTicket>,
        Option<PresentationRequest>,
        Option<Failure>,
    ) {
        let mut state = lock_unpoisoned(&self.state);
        if state.pending.is_none() {
            Self::restore(&mut state);
        }
        state.closed = true;
        (state.pending.take(), state.parent.take(), state.failure)
    }
    fn restore(state: &mut State) {
        let Some(parent) = state.parent.as_mut() else {
            return;
        };
        for (index, part) in state.parts[..state.count].iter_mut().enumerate() {
            let Some((mut part, report)) = part.take() else {
                continue;
            };
            if let Err(failure) = report {
                state.failure = state.failure.or(Some(failure));
                part.fail("division presentation part acquisition failed");
            }
            let ticks = match report {
                Ok(Report::PresentationStatus(ticks)) => ticks,
                _ => 0,
            };
            parent.captured.restore_status_part(index, part, ticks);
        }
        parent
            .captured
            .restore_preparation_store(parent.army_stats.as_deref_mut());
    }
    pub fn close(&self) {
        let mut state = lock_unpoisoned(&self.state);
        state.admission_open = false;
        state.closed = true;
    }
    pub fn drain(&self) {
        let pending = lock_unpoisoned(&self.state).pending.take();
        super::drain(&self.executor, &self.calibration, pending);
        let mut state = lock_unpoisoned(&self.state);
        for part in &mut state.parts {
            *part = None;
        }
        state.parent = None;
    }
}
impl collector_workers::acquisition::Publication for PresentationFanIn {
    fn failed(&self) {
        let mut state = lock_unpoisoned(&self.state);
        state.admission_open = false;
        state.closed = true;
        state.failure = Some(Failure::Panicked);
    }
    fn published(&self) {
        let mut state = lock_unpoisoned(&self.state);
        if state.closed {
            return;
        }
        state.remaining -= 1;
        if state.remaining != 0 {
            return;
        }
        if state.parent.is_none() || state.parts[..state.count].iter().any(Option::is_none) {
            return;
        }
        Self::restore(&mut state);
        if state.failure.is_some() || collector_memory::terminal::failed() {
            return;
        }
        let Some(executor) = self.executor.upgrade() else {
            return;
        };
        let request = state.parent.take().unwrap();
        let job = if state.final_ranges {
            Job::Presentation(request)
        } else {
            Job::PresentationSettle(request)
        };
        let pending = submit(&executor, &self.calibration, job);
        let wake = !pending.parked;
        state.pending = Some(pending);
        drop(state);
        if wake {
            executor.wake_workers(1);
        }
    }
}
