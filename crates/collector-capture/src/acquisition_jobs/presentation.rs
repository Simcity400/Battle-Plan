use super::*;
use collector_workers::acquisition::lock_unpoisoned;
use std::sync::Weak;
mod final_inputs;
mod inputs;
mod stitch;
pub(crate) use final_inputs::FinalInputsDispatch;
pub(crate) use inputs::PresentationInputs;
pub(crate) use stitch::PresentationFanIn;

pub(crate) struct RangeTicket {
    pub ticket: Ticket,
    pub(crate) parked: bool,
}
pub(crate) struct PresentationDispatch {
    executor: Weak<collector_workers::acquisition::Pool<Job, Report>>,
    calibration: Option<Arc<SharedCalibration>>,
    parts: [Option<RangeTicket>; division_presentation::MAX_STATUS_PARTS],
    dispatched: bool,
    released: usize,
    preparation: Option<Arc<PresentationFanIn>>,
    pub(super) stitch: Option<Arc<PresentationFanIn>>,
    pub count: usize,
    final_ranges: bool,
}
impl Default for PresentationDispatch {
    fn default() -> Self {
        Self {
            executor: Weak::new(),
            calibration: None,
            parts: std::array::from_fn(|_| None),
            dispatched: false,
            released: 0,
            preparation: None,
            stitch: None,
            count: 0,
            final_ranges: false,
        }
    }
}
impl PresentationDispatch {
    pub(super) fn arm(&mut self, pool: &Pool, stitch: Arc<PresentationFanIn>) {
        assert!(self.parts.iter().all(Option::is_none));
        self.dispatched = false;
        self.released = 0;
        self.executor = Arc::downgrade(&pool.executor);
        self.calibration = Some(Arc::clone(&pool.calibration));
        self.preparation = Some(Arc::clone(&pool.presentation_preparation));
        self.stitch = Some(stitch);
        self.count = 0;
        self.final_ranges = false;
    }
    pub(crate) fn dispatched(&self) -> bool {
        self.dispatched
    }
    pub(crate) fn continuation(&self) -> Option<Arc<PresentationFanIn>> {
        self.dispatched.then(|| {
            if self.final_ranges {
                self.stitch.as_ref().unwrap().clone()
            } else {
                self.preparation.as_ref().unwrap().clone()
            }
        })
    }
    pub(crate) fn final_ranges(&self) -> bool {
        self.final_ranges
    }
    fn successor(&self) -> Self {
        Self {
            executor: self.executor.clone(),
            calibration: self.calibration.clone(),
            parts: std::array::from_fn(|_| None),
            dispatched: false,
            released: 0,
            preparation: None,
            stitch: self.stitch.clone(),
            count: 0,
            final_ranges: false,
        }
    }
    pub(super) fn take(&mut self, index: usize) -> Option<RangeTicket> {
        self.parts[index].take()
    }
    #[inline(never)]
    pub(super) fn dispatch(
        &mut self,
        captured: &mut division_presentation::Captured,
        countries: &engine_allocation::SharedBox<engine_native::country_memory::Cache>,
        context: division_presentation::CaptureContext,
        inputs: &Option<
            engine_allocation::SharedBox<collector_divisions::division_census::UnitInputs>,
        >,
    ) {
        let Some(executor) = self.executor.upgrade() else {
            return;
        };
        if collector_memory::terminal::failed() {
            return;
        }
        self.count = captured.status_part_count();
        self.final_ranges = !captured.preparation_pending();
        let continuation = if self.final_ranges {
            self.stitch.as_ref().unwrap()
        } else {
            self.preparation.as_ref().unwrap()
        };
        if !continuation.arm(self.count, self.final_ranges) {
            return;
        }
        self.dispatched = true;
        for index in 0..captured.status_part_count() {
            let mut job = Some(Job::PresentationStatus(PresentationStatusRequest {
                countries: engine_allocation::SharedBox::clone(countries),
                context,
                inputs: inputs.clone(),
                part: Some(captured.take_status_part(index)),
                preparation: !self.final_ranges,
                stitch: Some((Arc::clone(continuation), index)),
            }));
            let role = job.as_ref().unwrap().role();
            // SAFETY: the parent belongs to the callback's Batch. The ledger
            // drains every child on unwind until that Batch adopts its ticket.
            let result =
                unsafe { executor.submit_with_priority(&mut job, role.priority(), true, false) };
            note_dispatch(
                &self.calibration.as_ref().unwrap().roles[role as usize],
                result.is_ok(),
            );
            self.parts[index] = Some(match result {
                Ok(ticket) => {
                    self.released += 1;
                    RangeTicket {
                        ticket,
                        parked: false,
                    }
                }
                Err(_) => RangeTicket {
                    ticket: executor
                        .park(&mut job, true)
                        .expect("presentation range retention slot"),
                    parked: true,
                },
            });
        }
        executor.wake_workers(self.released);
    }
}

pub(super) fn completed(
    job: &mut Job,
    report: &Result<Report, Failure>,
) -> Option<Arc<dyn collector_workers::acquisition::Publication>> {
    match job {
        Job::ShipModels(request) => {
            let dispatch = request.details.as_ref()?;
            dispatch.store(&mut request.captured, report);
            Some(dispatch.clone())
        }
        Job::Characters(request) => {
            let dispatch = request.final_inputs.as_ref()?;
            dispatch.store(request.payload.part, report);
            Some(dispatch.clone())
        }
        Job::DivisionInputs(request) => {
            let dispatch = request.payload.presentation.as_ref()?.clone();
            dispatch.store_master(std::mem::take(&mut request.payload.census), *report);
            Some(dispatch)
        }
        Job::DivisionInputParts(request) => {
            let (dispatch, index) = request.presentation.as_ref()?;
            dispatch.store_part(*index, std::mem::take(&mut *request.census), *report);
            Some(dispatch.clone())
        }
        Job::PresentationPrepare(request) | Job::PresentationSettle(request)
            if request.dispatch.dispatched =>
        {
            let dispatch = request.dispatch.continuation()?;
            dispatch.store_parent(request, *report);
            Some(dispatch)
        }
        Job::PresentationStatus(request) => {
            let (dispatch, index) = request.stitch.as_ref()?;
            dispatch.store_part(*index, request.part.take(), *report);
            request.inputs = None;
            Some(dispatch.clone())
        }
        _ => None,
    }
}

fn submit(
    executor: &collector_workers::acquisition::Pool<Job, Report>,
    calibration: &SharedCalibration,
    job: Job,
) -> RangeTicket {
    submit_retained(executor, calibration, &mut Some(job))
}

fn submit_retained(
    executor: &collector_workers::acquisition::Pool<Job, Report>,
    calibration: &SharedCalibration,
    job: &mut Option<Job>,
) -> RangeTicket {
    let role = job.as_ref().expect("retained successor").role();
    // SAFETY: Batch owns the ledger until every child is drained. Each phase
    // has its own reservation because preceding tickets may be uncollected.
    let result = unsafe { executor.submit_with_priority(job, role.priority(), true, false) };
    note_dispatch(&calibration.roles[role as usize], result.is_ok());
    match result {
        Ok(ticket) => RangeTicket {
            ticket,
            parked: false,
        },
        Err(_) => RangeTicket {
            ticket: executor
                .park(job, true)
                .expect("presentation successor slot"),
            parked: true,
        },
    }
}

pub(super) fn drain(
    executor: &Weak<collector_workers::acquisition::Pool<Job, Report>>,
    calibration: &SharedCalibration,
    pending: Option<RangeTicket>,
) {
    let (Some(executor), Some(pending)) = (executor.upgrade(), pending) else {
        return;
    };
    contain(|| {
        if pending.parked {
            drop(executor.unpark(pending.ticket));
        } else {
            let mut completed = None;
            if executor.finish_into(pending.ticket, &mut completed) {
                account_completion(
                    calibration,
                    completed.as_ref().expect("collected completion"),
                );
            }
            drop(completed);
        }
    });
}
impl Drop for PresentationDispatch {
    fn drop(&mut self) {
        let Some(calibration) = self.calibration.as_deref() else {
            return;
        };
        for pending in self.parts.iter_mut().filter_map(Option::take) {
            drain(&self.executor, calibration, Some(pending));
        }
    }
}
