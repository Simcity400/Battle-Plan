//! The ordered presentation worker can release its already-prepared ranges
//! without waiting for the callback's next unrelated result installation.
use super::*;
use std::sync::Weak;

pub(super) struct RangeTicket {
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
        }
    }
}
impl PresentationDispatch {
    pub(super) fn arm(&mut self, pool: &Pool) {
        assert!(!self.dispatched && self.parts.iter().all(Option::is_none));
        self.executor = Arc::downgrade(&pool.executor);
        self.calibration = Some(Arc::clone(&pool.calibration));
    }
    pub(crate) fn dispatched(&self) -> bool {
        self.dispatched
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
        self.dispatched = true;
        for index in 0..captured.status_part_count() {
            let mut job = Some(Job::PresentationStatus(PresentationStatusRequest {
                countries: Arc::clone(countries),
                context,
                inputs: inputs.clone(),
                part: captured.take_status_part(index),
            }));
            // SAFETY: the parent belongs to the callback's Batch. The ledger
            // drains every child on unwind until that Batch adopts its ticket.
            let result = unsafe {
                executor.submit_with_priority(
                    &mut job,
                    Role::PresentationStatus.priority(),
                    true,
                    false,
                )
            };
            note_dispatch(
                &self.calibration.as_ref().unwrap().roles[Role::PresentationStatus as usize],
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
impl Drop for PresentationDispatch {
    fn drop(&mut self) {
        let Some(executor) = self.executor.upgrade() else {
            return;
        };
        // Normally the callback already adopted every ticket. A callback
        // unwind or failed parent must still end every shared native lease.
        for pending in self.parts.iter_mut().filter_map(Option::take) {
            contain(|| {
                if pending.parked {
                    drop(executor.unpark(pending.ticket));
                } else {
                    let mut completed = None;
                    executor.finish_into(pending.ticket, &mut completed);
                    drop(completed);
                }
            });
        }
    }
}
