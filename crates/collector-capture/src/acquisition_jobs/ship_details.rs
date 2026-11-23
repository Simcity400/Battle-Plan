use super::*;
use presentation::RangeTicket;
use std::sync::{MutexGuard, Weak};

type Parts = [Option<RangeTicket>; ship_models::MAX_PARTS];

pub(crate) struct ShipDetailsDispatch {
    executor: Weak<collector_workers::acquisition::Pool<Job, Report>>,
    calibration: Arc<SharedCalibration>,
    state: Mutex<State>,
}
#[derive(Default)]
struct State {
    closed: bool,
    countries: Option<engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>>,
    context: Option<ship_models::CaptureContext>,
    captured: Option<ship_models::Captured>,
    dispatched: bool,
    pending: Parts,
    failure: Option<Failure>,
}
impl ShipDetailsDispatch {
    pub(super) fn new(
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
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
    pub(super) fn arm(&self, request: &ShipModelsRequest) {
        let mut state = self.lock();
        assert!(
            state.closed && state.captured.is_none() && state.pending.iter().all(Option::is_none)
        );
        *state = State {
            countries: Some(engine_allocation::SharedBox::clone(&request.countries)),
            context: Some(request.context),
            ..Default::default()
        };
    }
    pub(super) fn store(
        &self,
        captured: &mut ship_models::Captured,
        report: &Result<Report, Failure>,
    ) {
        let mut state = self.lock();
        if !state.closed && matches!(report, Ok(Report::ShipModels)) {
            state.captured = Some(std::mem::replace(captured, ship_models::Captured::vacant()));
        }
    }
    pub fn take(&self, captured: &mut ship_models::Captured) -> Result<Option<Parts>, Failure> {
        let mut state = self.lock();
        state.closed = true;
        if let Some(stored) = state.captured.take() {
            *captured = stored;
        }
        let pending = std::mem::take(&mut state.pending);
        if let Some(failure) = state.failure {
            drop(state);
            for pending in pending.into_iter().flatten() {
                presentation::drain(&self.executor, &self.calibration, Some(pending));
            }
            return Err(failure);
        }
        Ok(state.dispatched.then_some(pending))
    }
    pub(super) fn close(&self) {
        self.lock().closed = true;
    }
    pub(super) fn drain(&self) {
        let pending = std::mem::take(&mut self.lock().pending);
        for pending in pending.into_iter().flatten() {
            presentation::drain(&self.executor, &self.calibration, Some(pending));
        }
        *self.lock() = State {
            closed: true,
            ..Default::default()
        };
    }
}
impl collector_workers::acquisition::Publication for ShipDetailsDispatch {
    fn failed(&self) {
        self.lock().failure = Some(Failure::Panicked);
    }
    fn published(&self) {
        let mut state = self.lock();
        if state.closed || state.captured.is_none() || collector_memory::terminal::failed() {
            return;
        }
        let Some(executor) = self.executor.upgrade() else {
            return;
        };
        let countries = engine_allocation::SharedBox::clone(state.countries.as_ref().unwrap());
        let context = state.context.unwrap();
        let captured = state.captured.as_mut().unwrap();
        captured.set_context(context);
        if captured.has_terminal_failure() {
            return;
        }
        let requests = take_parts(captured, &countries, context.base);
        state.dispatched = true;
        let mut wake = 0;
        for (index, request) in requests.into_iter().enumerate() {
            let Some(request) = request else { continue };
            let mut job = Some(Job::ShipModelDetails(request));
            let role = Role::ShipModelDetails;
            // SAFETY: Batch drains this ledger after its census and all publishers settle.
            let result =
                unsafe { executor.submit_with_priority(&mut job, role.priority(), true, false) };
            note_dispatch(&self.calibration.roles[role as usize], result.is_ok());
            state.pending[index] = Some(match result {
                Ok(ticket) => {
                    wake += 1;
                    RangeTicket {
                        ticket,
                        parked: false,
                    }
                }
                Err(_) => RangeTicket {
                    ticket: executor
                        .park(&mut job, true)
                        .expect("ship detail retention slot"),
                    parked: true,
                },
            });
        }
        drop(state);
        executor.wake_workers(wake);
    }
}

pub(crate) fn take_parts(
    captured: &mut ship_models::Captured,
    countries: &engine_allocation::SharedBox<engine_native_intelligence::country_memory::Cache>,
    base: usize,
) -> [Option<ShipModelDetailsRequest>; ship_models::MAX_PARTS] {
    let mut requests = [const { None }; ship_models::MAX_PARTS];
    let parts = captured.split_details(ship_models::parts_for(captured.census_len()));
    let context = captured.capture_context(base);
    for (part, request) in requests.iter_mut().enumerate().take(parts).skip(1) {
        *request = Some(ShipModelDetailsRequest {
            countries: engine_allocation::SharedBox::clone(countries),
            context,
            part,
            captured: captured.take_part(part - 1),
        });
    }
    requests[0] = Some(ShipModelDetailsRequest {
        countries: engine_allocation::SharedBox::clone(countries),
        context,
        part: 0,
        captured: std::mem::replace(captured, ship_models::Captured::vacant()),
    });
    requests
}

#[cfg(test)]
mod tests;
