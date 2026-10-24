//! Presentation workers release prepared ranges without waiting for the
//! callback's next unrelated result installation.
use super::*;
use acquisition::callback_parks::{Site, lock_callback};
use std::sync::Weak;
mod final_inputs;
mod inputs;
mod stitch;
pub(crate) use final_inputs::FinalInputsDispatch;
pub(crate) use inputs::PresentationInputs;
pub(crate) use stitch::PresentationFanIn;

#[derive(Default)]
pub(super) struct ContinuationCounters {
    dispatched: AtomicU64,
    rejected: AtomicU64,
    ticks: AtomicU64,
    max_ticks: AtomicU64,
}
#[derive(Serialize)]
pub(crate) struct ContinuationCalibration {
    edge: &'static str,
    dispatched: u64,
    rejected: u64,
    publish_to_queue_mean_ns: u64,
    publish_to_queue_max_ns: u64,
}
impl ContinuationCounters {
    pub(super) fn snapshot(&self, edge: &'static str, frequency: u64) -> ContinuationCalibration {
        let dispatched = self.dispatched.load(Ordering::Relaxed);
        let ns = |ticks: u64| {
            (u128::from(ticks) * 1_000_000_000 / u128::from(frequency)).min(u128::from(u64::MAX))
                as u64
        };
        ContinuationCalibration {
            edge,
            dispatched,
            rejected: self.rejected.load(Ordering::Relaxed),
            publish_to_queue_mean_ns: ns(self.ticks.load(Ordering::Relaxed) / dispatched.max(1)),
            publish_to_queue_max_ns: ns(self.max_ticks.load(Ordering::Relaxed)),
        }
    }
}

pub(crate) struct RangeTicket {
    pub ticket: Ticket,
    pub parked: bool,
}
pub(crate) struct PresentationDispatch {
    executor: Weak<acquisition::Pool<Job, Report>>,
    calibration: Option<Arc<SharedCalibration>>,
    parts: [Option<RangeTicket>; division_presentation::MAX_STATUS_PARTS],
    dispatched: bool,
    pub(crate) ticks: u64,
    released: usize,
    preparation: Option<Arc<PresentationFanIn>>,
    pub(super) stitch: Option<Arc<PresentationFanIn>>,
    pub(crate) count: usize,
    final_ranges: bool,
}
impl Default for PresentationDispatch {
    fn default() -> Self {
        Self {
            executor: Weak::new(),
            calibration: None,
            parts: std::array::from_fn(|_| None),
            dispatched: false,
            ticks: 0,
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
        self.ticks = 0;
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
            ticks: 0,
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
        countries: &Arc<engine_native::country_memory::Cache>,
        context: division_presentation::CaptureContext,
        inputs: &Option<Arc<crate::division_census::UnitInputs>>,
    ) {
        let Some(executor) = self.executor.upgrade() else {
            return;
        };
        if crate::allocation::failed() {
            return;
        }
        let begin = sampling::qpc();
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
                countries: Arc::clone(countries),
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
        self.ticks = sampling::qpc().saturating_sub(begin).max(0) as u64;
    }
}

/// Only ownership transfers run before publication: no native reads, allocation
/// or successor admission. Cancelled queued requests retain their original data.
pub(super) fn completed(
    job: &mut Job,
    report: &Result<Report, Failure>,
) -> Option<Arc<dyn acquisition::Publication>> {
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
            // Drop every immutable preparation lease before its successor.
            request.inputs = None;
            Some(dispatch.clone())
        }
        _ => None,
    }
}

/// The same worker-only admission and retained rejection used by range release.
/// Retain the ticket and release its ledger before waking accepted work.
fn submit(
    executor: &acquisition::Pool<Job, Report>,
    calibration: &SharedCalibration,
    job: Job,
    last_publish: i64,
    edge: usize,
) -> RangeTicket {
    let role = job.role();
    let mut job = Some(job);
    // SAFETY: Batch owns the ledger until every child is drained. Each phase
    // has its own reservation because preceding tickets may be uncollected.
    let result =
        unsafe { executor.submit_with_priority_timed(&mut job, role.priority(), true, false) };
    note_dispatch(&calibration.roles[role as usize], result.is_ok());
    match result {
        Ok((ticket, queued_qpc)) => {
            let ticks = queued_qpc.saturating_sub(last_publish).max(0) as u64;
            let counters = &calibration.presentation_continuations[edge];
            counters.ticks.fetch_add(ticks, Ordering::Relaxed);
            counters.max_ticks.fetch_max(ticks, Ordering::Relaxed);
            counters.dispatched.fetch_add(1, Ordering::Relaxed);
            RangeTicket {
                ticket,
                parked: false,
            }
        }
        Err(_) => {
            calibration.presentation_continuations[edge]
                .rejected
                .fetch_add(1, Ordering::Relaxed);
            RangeTicket {
                ticket: executor
                    .park(&mut job, true)
                    .expect("presentation successor slot"),
                parked: true,
            }
        }
    }
}

pub(super) fn drain(
    executor: &Weak<acquisition::Pool<Job, Report>>,
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
        // Normally the callback already adopted every ticket. A callback
        // unwind or failed parent must still end every shared native lease.
        for pending in self.parts.iter_mut().filter_map(Option::take) {
            drain(&self.executor, calibration, Some(pending));
        }
    }
}
