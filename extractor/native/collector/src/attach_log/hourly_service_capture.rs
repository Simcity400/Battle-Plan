use super::hourly_service_timing::{
    elapsed_ns, ClosedServiceMeasurement, HourlyServiceTiming, SERVICE_LIMIT_NS,
};
pub(crate) use super::hourly_service_timing::{
    RequestOutcome, ServiceOwner, ServiceStage, TimingFault,
};
use compact_contract::hourly_service::{
    HourlyServiceCensusV1, HourlyServiceOutcomeV1, HourlyServiceOwnerV1, HourlyServiceSampleV1,
};
use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::rc::Rc;
use std::time::Instant;

const SAMPLE_CAPACITY: usize = 4096;
const SETUP_CAPACITY: usize = 64;
const OWNER_CAPACITY: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Setup,
    Owned(ServiceOwner, ServiceStage),
    Unattributed,
    Idle,
}
#[derive(Clone, Copy)]
struct OwnerState {
    owner: ServiceOwner,
    edge: Option<Instant>,
    publication: Option<Instant>,
}

pub(crate) struct Report {
    pub samples: Vec<HourlyServiceSampleV1>,
    pub census: HourlyServiceCensusV1,
}

struct Runtime {
    generation: u64,
    origin: Instant,
    mark: Instant,
    phase: Phase,
    timing: HourlyServiceTiming,
    owners: [Option<OwnerState>; OWNER_CAPACITY],
    setup: [(u64, u64); SETUP_CAPACITY],
    setup_len: usize,
    samples: Vec<HourlyServiceSampleV1>,
    unattributed_ns: u64,
    retired_scheduled: u64,
    retired_boundary: u64,
    retired_overrun: u64,
    retired_unknown: u64,
    /// Nanoseconds taken out of the owned phase by [`exclude_wait`]: the
    /// sweep thread's blocked time on a publisher that outlived its readers.
    excluded_wait_ns: u64,
}

impl Runtime {
    fn new(now: Instant) -> Self {
        let mut samples = Vec::new();
        let mut timing = HourlyServiceTiming::default();
        if samples.try_reserve_exact(SAMPLE_CAPACITY).is_err() {
            timing.note_fault(TimingFault::CarrierLoss);
        }
        Self {
            generation: 0,
            origin: now,
            mark: now,
            phase: Phase::Setup,
            timing,
            owners: [None; OWNER_CAPACITY],
            setup: [(0, 0); SETUP_CAPACITY],
            setup_len: 0,
            samples,
            unattributed_ns: 0,
            retired_scheduled: 0,
            retired_boundary: 0,
            retired_overrun: 0,
            retired_unknown: 0,
            excluded_wait_ns: 0,
        }
    }
    fn stamp(&mut self, mark: Instant) -> Option<u64> {
        match elapsed_ns(self.origin, mark) {
            Ok(ns) => Some(ns),
            Err(fault) => {
                self.clock_fault(fault);
                None
            }
        }
    }
    fn clock_fault(&mut self, fault: TimingFault) {
        self.timing.note_fault(fault);
        if let Phase::Owned(owner, _) = self.phase {
            let _ = self.timing.record_fault(owner, fault);
        }
    }
    fn claim_setup(&mut self, owner: ServiceOwner, stage: ServiceStage) {
        for index in 0..self.setup_len {
            let (start, end) = self.setup[index];
            self.record_ns(owner, stage, start, end);
        }
        self.setup_len = 0;
    }
    fn checked_add(&mut self, a: u64, b: u64) -> u64 {
        match a.checked_add(b) {
            Some(value) => value,
            None => {
                self.timing.note_fault(TimingFault::CounterOverflow);
                a
            }
        }
    }
    fn unowned_ns(&mut self, start: u64, end: u64) {
        match end.checked_sub(start) {
            Some(ns) => {
                self.unattributed_ns = self.checked_add(self.unattributed_ns, ns);
            }
            None => self.timing.note_fault(TimingFault::ClockReversed),
        }
    }
    fn record_ns(&mut self, owner: ServiceOwner, stage: ServiceStage, start: u64, end: u64) {
        let _ = self.timing.record_interval(owner, stage, start, end);
    }
    fn flush(&mut self, now: Instant) {
        let Some(start) = self.stamp(self.mark) else {
            return;
        };
        let Some(end) = self.stamp(now) else {
            return;
        };
        if end < start {
            self.clock_fault(TimingFault::ClockReversed);
            return;
        }
        match self.phase {
            Phase::Owned(owner, stage) => self.record_ns(owner, stage, start, end),
            Phase::Unattributed => self.unowned_ns(start, end),
            Phase::Setup if start != end => {
                if self.setup_len != 0 && self.setup[self.setup_len - 1].1 == start {
                    self.setup[self.setup_len - 1].1 = end;
                } else if self.setup_len < SETUP_CAPACITY {
                    self.setup[self.setup_len] = (start, end);
                    self.setup_len += 1;
                } else {
                    self.timing.note_fault(TimingFault::IntervalCapacity);
                    self.unowned_ns(start, end);
                }
            }
            Phase::Setup | Phase::Idle => {}
        }
        self.mark = now;
    }
    fn switch(&mut self, phase: Phase, now: Instant) -> Phase {
        self.flush(now);
        std::mem::replace(&mut self.phase, phase)
    }
    fn begin_at(&mut self, owner: ServiceOwner, edge: Option<Instant>, now: Instant) {
        self.flush(now);
        if self.timing.begin(owner, 0).is_err() {
            self.phase = Phase::Unattributed;
            return;
        }
        if let Some(slot) = self.owners.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(OwnerState {
                owner,
                edge,
                publication: None,
            });
        } else {
            self.timing.note_fault(TimingFault::RequestCapacity);
        }
        let claimed = self.stamp(now).is_some_and(|completed| {
            self.timing
                .record_preselection(owner, &self.setup[..self.setup_len], completed)
                .is_ok()
        });
        if claimed {
            self.setup_len = 0;
        } else {
            self.claim_setup(owner, ServiceStage::Capture);
        }

        self.phase = Phase::Owned(owner, ServiceStage::Capture);
    }
    fn interval(&mut self, owner: ServiceOwner, stage: ServiceStage, start: Instant, end: Instant) {
        let valid = match (elapsed_ns(self.origin, start), elapsed_ns(self.origin, end)) {
            (Ok(start), Ok(end)) => self
                .timing
                .record_interval(owner, stage, start, end)
                .is_ok(),
            (Err(fault), _) | (_, Err(fault)) => {
                self.timing.note_fault(fault);
                let _ = self.timing.record_fault(owner, fault);
                false
            }
        };
        if valid && stage == ServiceStage::Sink {
            if let Some(state) = self
                .owners
                .iter_mut()
                .flatten()
                .find(|state| state.owner == owner)
            {
                state.publication = Some(state.publication.map_or(end, |old| old.max(end)));
            }
        }
    }
    fn owner_state(&self, owner: ServiceOwner) -> Option<OwnerState> {
        self.owners
            .iter()
            .flatten()
            .find(|state| state.owner == owner)
            .copied()
    }
    fn queue(
        &mut self,
        closed: ClosedServiceMeasurement,
        data: Option<(u64, u64)>,
        now: Instant,
    ) -> Option<usize> {
        if self.samples.len() >= self.samples.capacity().min(SAMPLE_CAPACITY) {
            let _ = self.timing.discard_undelivered(closed.owner);
            return None;
        }
        let state = self.owner_state(closed.owner);
        let edge_to_completion_ns = state
            .and_then(|state| state.edge)
            .and_then(|edge| elapsed_ns(edge, now).ok());
        let edge_to_publication_ns = state
            .filter(|_| {
                data.is_some()
                    && matches!(
                        closed.outcome,
                        RequestOutcome::Complete | RequestOutcome::Partial
                    )
            })
            .and_then(|state| Some((state.edge?, state.publication?)))
            .and_then(|(edge, publication)| elapsed_ns(edge, publication).ok());
        let sample = HourlyServiceSampleV1 {
            owner: HourlyServiceOwnerV1 {
                logical_stream_id: closed.owner.logical_stream_id,
                epoch: closed.owner.epoch,
                game_hour_raw: closed.owner.game_hour_raw,
                observation_ordinal: closed.owner.observation_ordinal,
            },
            outcome: match closed.outcome {
                RequestOutcome::Complete => HourlyServiceOutcomeV1::Complete,
                RequestOutcome::Partial => HourlyServiceOutcomeV1::Partial,
                RequestOutcome::Refused => HourlyServiceOutcomeV1::Refused,
                RequestOutcome::Aborted => HourlyServiceOutcomeV1::Aborted,
                RequestOutcome::TeardownCut => HourlyServiceOutcomeV1::TeardownCut,
            },
            data_sequence: data,
            observed_service_ns: closed.observed_service_ns,
            stage_ns: closed.stage_ns,
            observed_stages: closed.observed_stages,
            closed_stages: closed.closed_stages,
            not_applicable_stages: closed.not_applicable_stages,
            edge_to_publication_ns,
            edge_to_completion_ns,
            fault_bits: closed.faults.bits(),
        };
        let index = self.samples.len();
        self.samples.push(sample);
        let _ = self.timing.release_to_carrier(closed.owner);
        Some(index)
    }
    fn close_at(
        &mut self,
        owner: ServiceOwner,
        outcome: RequestOutcome,
        data: Option<(u64, u64)>,
        now: Instant,
    ) -> Option<usize> {
        self.flush(now);
        let completed = self.stamp(now)?;
        self.timing.finish_observed_stages(owner, outcome);
        let closed = self.timing.close(owner, outcome, completed).ok()?;
        let index = self.queue(closed, data, now);
        if matches!(self.phase, Phase::Owned(current, _) if current == owner) {
            self.phase = Phase::Setup;
        }
        for slot in &mut self.owners {
            if slot.is_some_and(|state| state.owner == owner) {
                *slot = None;
            }
        }
        index
    }
    fn add_close_cost(&mut self, index: usize, start: Instant, end: Instant) {
        let ns = match elapsed_ns(start, end) {
            Ok(ns) => ns,
            Err(fault) => {
                self.timing.note_fault(fault);
                self.samples[index].fault_bits |= 1 << fault as u8;
                return;
            }
        };
        let sample = &mut self.samples[index];
        let finish = ServiceStage::Finish as usize;
        let total = sample
            .observed_service_ns
            .and_then(|value| value.checked_add(ns));
        let stage = sample.stage_ns[finish].unwrap_or(0).checked_add(ns);
        if total.is_none() || stage.is_none() {
            sample.fault_bits |= 1 << TimingFault::CounterOverflow as u8;
        }
        sample.observed_service_ns = total;
        sample.stage_ns[finish] = stage;
        sample.observed_stages |= 1 << finish;
        sample.closed_stages |= 1 << finish;
        sample.not_applicable_stages &= !(1 << finish);
        sample.edge_to_completion_ns = sample
            .edge_to_completion_ns
            .and_then(|value| value.checked_add(ns));
        self.mark = end;
    }
    fn retire_scheduler(
        &mut self,
        scheduled: u64,
        boundary: u64,
        overrun: u64,
        unknown: u64,
    ) {
        self.retired_scheduled = self.checked_add(self.retired_scheduled, scheduled);
        self.retired_boundary = self.checked_add(self.retired_boundary, boundary);
        self.retired_overrun = self.checked_add(self.retired_overrun, overrun);
        self.retired_unknown = self.checked_add(self.retired_unknown, unknown);
    }
    fn report(
        mut self,
        scheduled: u64,
        boundary: u64,
        overrun: u64,
        unknown: u64,
        now: Instant,
    ) -> Report {
        let report_started = Instant::now();
        self.flush(now);
        self.setup_len = 0;
        self.retire_scheduler(scheduled, boundary, overrun, unknown);
        let admitted = self.timing.census().admitted_requests;
        match self
            .retired_scheduled
            .checked_sub(self.retired_boundary)
            .and_then(|scheduled| scheduled.checked_sub(admitted))
        {
            Some(missed) => self.timing.note_unmeasured_hours(missed),
            None => self.timing.note_fault(TimingFault::CounterOverflow),
        }
        self.timing.note_unmeasured_hours(self.retired_overrun);
        self.timing.note_unmeasured_hours(self.retired_unknown);
        if self.unattributed_ns != 0 {
            self.timing.note_fault(TimingFault::UnmeasuredHours);
        }
        let completed = self.stamp(now).unwrap_or(0);
        self.timing.seal(completed);
        while let Some(closed) = self.timing.next_closed() {
            self.queue(closed, None, now);
        }
        let local = self.timing.census();
        // The TEARDOWN CUT is the request the terminal close itself closed: it
        // never completed and its terminal stage is teardown work, not
        // service. It stays in the samples for the record and counts as a
        // boundary hour, never as a measurement. An ABORTED request is not
        // that -- the loop lost it -- and stays a measured hour.
        let cut_samples = self
            .samples
            .iter()
            .filter(|sample| sample.outcome == HourlyServiceOutcomeV1::TeardownCut)
            .count() as u64;
        let mut census = HourlyServiceCensusV1 {
            offered_requests: local.offered_requests,
            admitted_requests: local.admitted_requests,
            closed_requests: local.closed_requests,
            queued_measurements: local.queued_measurements,
            lost_measurements: local.lost_measurements,
            partial_requests: local.partial_requests,
            refused_requests: local.refused_requests,
            aborted_requests: local.aborted_requests,
            unmeasured_hours: local.unmeasured_hours,
            boundary_excluded_hours: self.retired_boundary.saturating_add(cut_samples),
            active_requests: local.active_requests as u64,
            pending_delivery: local.pending_delivery as u64,
            unattributed_service_ns: self.unattributed_ns,
            fault_bits: local.faults.bits(),
            sealed: local.sealed,
            ..HourlyServiceCensusV1::default()
        };
        for sample in &self.samples {
            census.fault_bits |= sample.fault_bits;
            if sample.fault_bits == 0 && sample.outcome != HourlyServiceOutcomeV1::TeardownCut {
                if let Some(ns) = sample.observed_service_ns {
                    census.complete_measurements += 1;
                    match census.total_service_ns.checked_add(ns) {
                        Some(total) => census.total_service_ns = total,
                        None => census.fault_bits |= 1 << TimingFault::CounterOverflow as u8,
                    }
                    census.maximum_service_ns =
                        Some(census.maximum_service_ns.map_or(ns, |old| old.max(ns)));
                    census.over_limit_measurements += u64::from(ns > SERVICE_LIMIT_NS);
                    continue;
                }
            }
            census.incomplete_measurements += 1;
        }
        match elapsed_ns(report_started, Instant::now()) {
            Ok(ns) => census.diagnostic_transport_ns = ns,
            Err(fault) => census.fault_bits |= 1 << fault as u8,
        }
        Report {
            samples: self.samples,
            census,
        }
    }
}

thread_local! {
    static RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) };
    static BORROW_FAULT: Cell<bool> = const { Cell::new(false) };
    static NEXT_GENERATION: Cell<u64> = const { Cell::new(0) };
}

fn with_runtime<T>(f: impl FnOnce(&mut Runtime) -> T) -> Option<T> {
    RUNTIME.with(|cell| {
        let Ok(mut slot) = cell.try_borrow_mut() else {
            BORROW_FAULT.with(|fault| fault.set(true));
            return None;
        };
        let runtime = slot.as_mut()?;
        if BORROW_FAULT.with(|fault| fault.replace(false)) {
            runtime.timing.note_fault(TimingFault::UnknownOwner);
        }
        Some(f(runtime))
    })
}

pub(crate) struct SessionGuard {
    generation: Option<u64>,
    _thread: PhantomData<Rc<()>>,
}
impl Drop for SessionGuard {
    fn drop(&mut self) {
        if let Some(generation) = self.generation {
            RUNTIME.with(|cell| {
                if let Ok(mut slot) = cell.try_borrow_mut() {
                    if slot
                        .as_ref()
                        .is_some_and(|runtime| runtime.generation == generation)
                    {
                        *slot = None;
                    }
                } else {
                    BORROW_FAULT.with(|fault| fault.set(true));
                }
            });
        }
    }
}

pub(crate) fn start() -> SessionGuard {
    let generation = RUNTIME.with(|cell| {
        let Ok(mut slot) = cell.try_borrow_mut() else {
            BORROW_FAULT.with(|fault| fault.set(true));
            return None;
        };
        if let Some(runtime) = slot.as_mut() {
            runtime.timing.note_fault(TimingFault::AlreadyClosed);
            return None;
        }
        let generation = NEXT_GENERATION.with(|next| {
            let value = next.get().checked_add(1)?;
            next.set(value);
            Some(value)
        });
        let Some(generation) = generation else {
            BORROW_FAULT.with(|fault| fault.set(true));
            return None;
        };
        let mut runtime = Runtime::new(Instant::now());
        runtime.generation = generation;
        *slot = Some(runtime);
        Some(generation)
    });
    SessionGuard {
        generation,
        _thread: PhantomData,
    }
}

pub(crate) struct PhaseGuard {
    previous: Option<(Phase, u64)>,
    _thread: PhantomData<Rc<()>>,
}
impl Drop for PhaseGuard {
    fn drop(&mut self) {
        if let Some((previous, generation)) = self.previous {
            with_runtime(|runtime| {
                if runtime.generation != generation {
                    return;
                }
                let previous = match previous {
                    Phase::Owned(owner, _) if runtime.owner_state(owner).is_none() => Phase::Setup,
                    other => other,
                };
                runtime.switch(previous, Instant::now());
            });
        }
    }
}
impl PhaseGuard {
    pub(crate) fn finish_service(
        mut self,
        close: Option<(ServiceOwner, RequestOutcome, Option<(u64, u64)>)>,
        suspend: bool,
    ) {
        let Some((previous, generation)) = self.previous.take() else {
            return;
        };
        with_runtime(|runtime| {
            if runtime.generation != generation {
                return;
            }
            let mut closed = false;
            if let Some((owner, outcome, bounds)) = close {
                if !matches!(runtime.phase, Phase::Owned(current, _) if current == owner) {
                    let _ = runtime
                        .timing
                        .record_fault(owner, TimingFault::UnknownOwner);
                }
                let start = Instant::now();
                if let Some(index) = runtime.close_at(owner, outcome, bounds, start) {
                    runtime.add_close_cost(index, start, Instant::now());
                    closed = true;
                }
            }
            let next = if suspend {
                Phase::Setup
            } else {
                match previous {
                    Phase::Owned(owner, _) if runtime.owner_state(owner).is_none() => Phase::Setup,
                    other => other,
                }
            };
            if closed {
                runtime.phase = next;
            } else {
                runtime.switch(next, Instant::now());
            }
        });
    }
}

#[cfg(test)]
pub(super) fn closed_requests_for_tests() -> u64 {
    with_runtime(|runtime| runtime.timing.census().closed_requests).unwrap_or(0)
}

fn guarded_phase(next: Phase) -> PhaseGuard {
    PhaseGuard {
        previous: with_runtime(|runtime| {
            (runtime.switch(next, Instant::now()), runtime.generation)
        }),
        _thread: PhantomData,
    }
}
pub(crate) fn begin(owner: ServiceOwner, observed_edge: Option<Instant>) {
    with_runtime(|runtime| runtime.begin_at(owner, observed_edge, Instant::now()));
}
pub(crate) fn claim_setup(owner: ServiceOwner, stage: ServiceStage) {
    with_runtime(|runtime| {
        runtime.flush(Instant::now());
        runtime.claim_setup(owner, stage);
    });
}
pub(crate) fn current_owner() -> Option<ServiceOwner> {
    with_runtime(|runtime| match runtime.phase {
        Phase::Owned(owner, _) => Some(owner),
        _ => None,
    })
    .flatten()
}
pub(crate) fn set_stage(stage: ServiceStage) {
    with_runtime(|runtime| {
        if let Phase::Owned(owner, _) = runtime.phase {
            runtime.switch(Phase::Owned(owner, stage), Instant::now());
        } else {
            runtime.timing.note_fault(TimingFault::UnknownOwner);
        }
    });
}
pub(crate) fn phase(owner: ServiceOwner, stage: ServiceStage) -> PhaseGuard {
    guarded_phase(Phase::Owned(owner, stage))
}
pub(crate) fn pause() -> PhaseGuard {
    guarded_phase(Phase::Idle)
}
pub(crate) fn unowned() -> PhaseGuard {
    guarded_phase(Phase::Unattributed)
}
pub(crate) fn clear_current() {
    with_runtime(|runtime| {
        runtime.switch(Phase::Setup, Instant::now());
    });
}
pub(crate) fn suspend() {
    clear_current();
}
pub(crate) fn fault(fault: TimingFault) {
    with_runtime(|runtime| runtime.timing.note_fault(fault));
}
pub(crate) fn record_fault(owner: ServiceOwner, fault: TimingFault) {
    with_runtime(|runtime| {
        let _ = runtime.timing.record_fault(owner, fault);
    });
}
pub(crate) fn admitted_requests() -> u64 {
    with_runtime(|runtime| runtime.timing.census().admitted_requests).unwrap_or(0)
}
pub(crate) fn record_interval(
    owner: ServiceOwner,
    stage: ServiceStage,
    start: Instant,
    end: Instant,
) {
    with_runtime(|runtime| runtime.interval(owner, stage, start, end));
}
pub(crate) fn record_unowned_interval(start: Instant, end: Instant) {
    with_runtime(|runtime| {
        if let (Some(start), Some(end)) = (runtime.stamp(start), runtime.stamp(end)) {
            runtime.unowned_ns(start, end);
        }
    });
}
pub(crate) fn exclude_wait(start: Instant, end: Instant) {
    with_runtime(|runtime| {
        runtime.flush(start);
        let excluded = match elapsed_ns(start, end) {
            Ok(ns) => ns,
            Err(_) => {
                runtime.clock_fault(TimingFault::ClockReversed);
                return;
            }
        };
        runtime.excluded_wait_ns = runtime.excluded_wait_ns.saturating_add(excluded);
        runtime.mark = end;
    });
}
/// What [`exclude_wait`] has taken out of the session so far, for a driven
/// frame to check the wait it excluded against the marks it published.
#[cfg(test)]
pub(super) fn excluded_wait_ns_for_tests() -> u64 {
    with_runtime(|runtime| runtime.excluded_wait_ns).unwrap_or(0)
}
/// The session's own origin -- the instant every interval it records is
/// measured from -- so a driven frame can begin its owner with an edge the
/// timer's accounting starts at exactly, with no slack for a test's clock.
#[cfg(test)]
pub(super) fn session_origin_for_tests() -> Option<Instant> {
    with_runtime(|runtime| runtime.origin)
}
pub(crate) fn close(
    owner: ServiceOwner,
    outcome: RequestOutcome,
    data_sequence: Option<(u64, u64)>,
) {
    with_runtime(|runtime| {
        let start = Instant::now();
        if let Some(index) = runtime.close_at(owner, outcome, data_sequence, start) {
            // Final bookkeeping stores the last timestamp; transport is outside owned service.
            runtime.add_close_cost(index, start, Instant::now());
        }
    });
}
pub(crate) fn note_scheduler_retired(
    scheduled: u64,
    boundary: u64,
    overrun: u64,
    unknown: u64,
) {
    with_runtime(|runtime| runtime.retire_scheduler(scheduled, boundary, overrun, unknown));
}
pub(crate) fn finish_with_scheduler(
    scheduled: u64,
    boundary: u64,
    overrun: u64,
    unknown: u64,
) -> Option<Report> {
    finish_with_scheduler_and_close(scheduled, boundary, overrun, unknown, None)
}
pub(crate) fn finish_with_scheduler_and_close(
    scheduled: u64,
    boundary: u64,
    overrun: u64,
    unknown: u64,
    final_close: Option<(ServiceOwner, RequestOutcome, Option<(u64, u64)>)>,
) -> Option<Report> {
    RUNTIME.with(|cell| {
        let Ok(mut slot) = cell.try_borrow_mut() else {
            BORROW_FAULT.with(|fault| fault.set(true));
            return None;
        };
        let mut runtime = slot.take()?;
        if BORROW_FAULT.with(|fault| fault.replace(false)) {
            runtime.timing.note_fault(TimingFault::UnknownOwner);
        }
        let mut cutoff = Instant::now();
        if let Some((owner, outcome, sequence)) = final_close {
            if let Some(index) = runtime.close_at(owner, outcome, sequence, cutoff) {
                let end = Instant::now();
                runtime.add_close_cost(index, cutoff, end);
                cutoff = end;
            }
        }
        Some(runtime.report(scheduled, boundary, overrun, unknown, cutoff))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    fn owner(hour: i32) -> ServiceOwner {
        ServiceOwner {
            logical_stream_id: 1,
            epoch: 1,
            game_hour_raw: hour,
            observation_ordinal: hour as u64,
        }
    }
    fn at(origin: Instant, ns: u64) -> Instant {
        origin + Duration::from_nanos(ns)
    }
    fn published(runtime: &mut Runtime, owner: ServiceOwner, start: u64, end: u64) {
        runtime.begin_at(owner, None, at(runtime.origin, start));
        for stage in [
            ServiceStage::Finish,
            ServiceStage::BuildTail,
            ServiceStage::PublicationPrepare,
            ServiceStage::Sink,
            ServiceStage::Commit,
        ] {
            runtime.switch(Phase::Owned(owner, stage), at(runtime.origin, start));
        }
        runtime.close_at(
            owner,
            RequestOutcome::Complete,
            Some((
                owner.observation_ordinal * 10,
                owner.observation_ordinal * 10 + 9,
            )),
            at(runtime.origin, end),
        );
    }

    #[test]
    fn capture_runtime_hands_setup_to_selected_owner_and_excludes_idle() {
        let now = Instant::now();
        let mut runtime = Runtime::new(now);
        runtime.switch(Phase::Idle, at(now, 10));
        runtime.switch(Phase::Setup, at(now, 1_010));
        runtime.begin_at(owner(1), None, at(now, 1_020));
        runtime.close_at(owner(1), RequestOutcome::Refused, None, at(now, 1_030));
        let report = runtime.report(1, 0, 0, 0, at(now, 1_030));
        assert_eq!(report.samples[0].observed_service_ns, Some(30));
        assert_eq!(report.samples[0].stage_ns[0], Some(30));
        assert_eq!(report.samples[0].edge_to_completion_ns, None);
        assert_eq!(report.census.fault_bits, 0);
        assert_eq!(report.census.refused_requests, 1);
        assert_eq!(report.census.complete_measurements, 1);
    }

    #[test]
    fn capture_runtime_keeps_previous_sink_union_out_of_successor() {
        let now = Instant::now();
        let mut runtime = Runtime::new(now);
        runtime.begin_at(owner(1), None, now);
        runtime.switch(Phase::Setup, at(now, 10));
        runtime.begin_at(owner(2), None, at(now, 20));
        runtime.interval(owner(1), ServiceStage::Sink, at(now, 5), at(now, 50));
        runtime.switch(Phase::Owned(owner(1), ServiceStage::Commit), at(now, 40));
        runtime.close_at(owner(1), RequestOutcome::Aborted, None, at(now, 60));
        runtime.switch(Phase::Owned(owner(2), ServiceStage::Finish), at(now, 60));
        runtime.close_at(owner(2), RequestOutcome::Refused, None, at(now, 70));
        let report = runtime.report(2, 0, 0, 0, at(now, 70));
        assert_eq!(report.samples[0].owner.game_hour_raw, 1);
        assert_eq!(report.samples[0].observed_service_ns, Some(60));
        assert_eq!(report.samples[1].owner.game_hour_raw, 2);
        assert_eq!(report.samples[1].observed_service_ns, Some(40));
    }

    #[test]
    fn capture_runtime_close_enqueue_cost_crosses_exact_limit() {
        let now = Instant::now();
        let mut runtime = Runtime::new(now);
        published(&mut runtime, owner(1), 0, SERVICE_LIMIT_NS);
        runtime.add_close_cost(0, at(now, SERVICE_LIMIT_NS), at(now, SERVICE_LIMIT_NS + 1));
        let report = runtime.report(1, 0, 0, 0, at(now, SERVICE_LIMIT_NS + 1));
        assert_eq!(
            report.samples[0].observed_service_ns,
            Some(SERVICE_LIMIT_NS + 1)
        );
        assert_eq!(
            report.samples[0].stage_ns[ServiceStage::Finish as usize],
            Some(1)
        );
        assert_eq!(report.census.total_service_ns, SERVICE_LIMIT_NS + 1);
        assert_eq!(report.census.over_limit_measurements, 1);
        assert_eq!(report.census.fault_bits, 0);
    }

    #[test]
    fn capture_runtime_scheduler_rollovers_and_unobserved_hours_close_census() {
        let now = Instant::now();
        let mut runtime = Runtime::new(now);
        published(&mut runtime, owner(1), 0, 10);
        runtime.retire_scheduler(2, 0, 3, 4);
        let report = runtime.report(5, 0, 6, 7, at(now, 10));
        assert_eq!(report.census.unmeasured_hours, 26);
        assert_ne!(
            report.census.fault_bits & (1 << TimingFault::UnmeasuredHours as u8),
            0
        );
        let mut invalid = Runtime::new(now);
        published(&mut invalid, owner(1), 0, 10);
        let report = invalid.report(0, 0, 0, 0, at(now, 10));
        assert_ne!(
            report.census.fault_bits & (1 << TimingFault::CounterOverflow as u8),
            0
        );
    }

    #[test]
    fn capture_runtime_excludes_the_unstarted_teardown_boundary_from_loss() {
        let now = Instant::now();
        let mut runtime = Runtime::new(now);
        published(&mut runtime, owner(1), 0, 10);
        let report = runtime.report(2, 1, 0, 0, at(now, 10));
        assert_eq!(report.census.complete_measurements, 1);
        assert_eq!(report.census.unmeasured_hours, 0);
        assert_eq!(report.census.boundary_excluded_hours, 1);
        assert_eq!(report.census.fault_bits, 0);
    }

    #[test]
    fn capture_runtime_terminal_carrier_overflow_preserves_capture_and_names_loss() {
        let now = Instant::now();
        let mut runtime = Runtime::new(now);
        let capacity = runtime.samples.capacity();
        for hour in 1..=(SAMPLE_CAPACITY as i32 + 1) {
            let t = hour as u64;
            runtime.begin_at(owner(hour), None, at(now, t));
            runtime.close_at(owner(hour), RequestOutcome::Refused, None, at(now, t));
        }
        assert_eq!(runtime.samples.capacity(), capacity);
        let report = runtime.report(
            SAMPLE_CAPACITY as u64 + 1,
            0,
            0,
            0,
            at(now, SAMPLE_CAPACITY as u64 + 1),
        );
        assert_eq!(report.samples.len(), SAMPLE_CAPACITY);
        assert_eq!(report.census.closed_requests, SAMPLE_CAPACITY as u64 + 1);
        assert_eq!(report.census.lost_measurements, 1);
        assert_eq!(report.census.active_requests, 0);
        assert_ne!(
            report.census.fault_bits & (1 << TimingFault::CarrierLoss as u8),
            0
        );
    }

    #[test]
    fn capture_runtime_last_unclosed_hour_is_retained_without_next_data() {
        let now = Instant::now();
        let mut runtime = Runtime::new(now);
        runtime.begin_at(owner(1), None, now);
        let report = runtime.report(1, 0, 0, 0, at(now, 50));
        assert_eq!(report.samples.len(), 1);
        assert_eq!(report.samples[0].outcome, HourlyServiceOutcomeV1::Aborted);
        assert_eq!(report.samples[0].observed_service_ns, Some(50));
        assert_ne!(
            report.samples[0].fault_bits & (1 << TimingFault::UnclosedRequest as u8),
            0
        );
    }

    #[test]
    fn capture_runtime_shared_work_cannot_disappear_into_idle_or_gap_zero() {
        let now = Instant::now();
        let mut runtime = Runtime::new(now);
        published(&mut runtime, owner(1), 0, 10);
        runtime.switch(Phase::Unattributed, at(now, 10));
        let report = runtime.report(1, 0, 0, 0, at(now, 15));
        assert_eq!(report.census.unattributed_service_ns, 5);
        assert_ne!(report.census.fault_bits, 0);
    }

    #[test]
    fn capture_runtime_tls_guard_does_not_reopen_closed_owner() {
        let session = start();
        begin(owner(1), None);
        {
            let outer = phase(owner(1), ServiceStage::Commit);
            close(owner(1), RequestOutcome::Refused, None);
            drop(outer);
        }
        assert_eq!(current_owner(), None);
        let report = finish_with_scheduler(1, 0, 0, 0).unwrap();
        assert_eq!(report.samples.len(), 1);
        assert_eq!(report.samples[0].fault_bits, 0);
        drop(session);
        assert!(finish_with_scheduler(0, 0, 0, 0).is_none());
    }
    #[test]
    fn capture_runtime_clock_failure_taints_owner_despite_later_valid_stages() {
        let now = Instant::now();
        let mut runtime = Runtime::new(now);
        runtime.begin_at(owner(1), None, now);
        runtime.interval(
            owner(1),
            ServiceStage::Sink,
            now - Duration::from_millis(1),
            now,
        );
        for stage in [
            ServiceStage::Finish,
            ServiceStage::BuildTail,
            ServiceStage::PublicationPrepare,
            ServiceStage::Sink,
            ServiceStage::Commit,
        ] {
            runtime.switch(Phase::Owned(owner(1), stage), now);
        }
        runtime.close_at(owner(1), RequestOutcome::Complete, Some((1, 2)), now);
        let report = runtime.report(1, 0, 0, 0, now);
        assert_ne!(
            report.samples[0].fault_bits & (1 << TimingFault::ClockReversed as u8),
            0
        );
        assert_eq!(report.census.complete_measurements, 0);
        assert_eq!(report.census.incomplete_measurements, 1);
        let mut runtime = Runtime::new(now);
        published(&mut runtime, owner(1), 0, 2_000_000);
        runtime.add_close_cost(0, at(now, 2_000_000), at(now, 1_000_000));
        assert_ne!(
            runtime.samples[0].fault_bits & (1 << TimingFault::ClockReversed as u8),
            0
        );
    }

    #[test]
    fn capture_runtime_old_guards_cannot_delete_or_change_new_session() {
        let old_session = start();
        begin(owner(1), None);
        let old_phase = pause();
        finish_with_scheduler(1, 0, 0, 0).unwrap();
        let new_session = start();
        begin(owner(2), None);
        drop(old_phase);
        drop(old_session);
        assert_eq!(current_owner(), Some(owner(2)));
        let report = finish_with_scheduler_and_close(
            1,
            0,
            0,
            0,
            Some((owner(2), RequestOutcome::Refused, None)),
        )
        .unwrap();
        assert_eq!(report.samples.len(), 1);
        assert_eq!(report.census.unattributed_service_ns, 0);
        assert_eq!(report.census.fault_bits, 0);
        drop(new_session);
    }

    #[test]
    fn capture_runtime_refused_sink_never_claims_publication_latency() {
        let now = Instant::now();
        let mut runtime = Runtime::new(now);
        runtime.begin_at(owner(1), Some(now), now);
        runtime.interval(owner(1), ServiceStage::Sink, now, at(now, 10));
        runtime.close_at(owner(1), RequestOutcome::Refused, None, at(now, 10));
        let report = runtime.report(1, 0, 0, 0, at(now, 10));
        assert_eq!(report.samples[0].edge_to_publication_ns, None);
        assert_eq!(report.samples[0].edge_to_completion_ns, Some(10));
    }

    #[test]
    fn capture_runtime_final_setup_is_explicitly_owned_by_terminal_request() {
        let now = Instant::now();
        let mut runtime = Runtime::new(now);
        runtime.begin_at(owner(1), None, now);
        runtime.switch(Phase::Setup, at(now, 10));
        runtime.switch(Phase::Owned(owner(1), ServiceStage::Terminal), at(now, 40));
        runtime.claim_setup(owner(1), ServiceStage::Terminal);
        runtime.close_at(owner(1), RequestOutcome::Refused, None, at(now, 50));
        let report = runtime.report(1, 0, 0, 0, at(now, 50));
        assert_eq!(report.samples[0].observed_service_ns, Some(50));
        assert_eq!(
            report.samples[0].stage_ns[ServiceStage::Terminal as usize],
            Some(40)
        );
        assert_eq!(report.census.unattributed_service_ns, 0);
        assert_eq!(report.census.fault_bits, 0);
    }
    #[test]
    fn capture_runtime_unavailable_carrier_allocation_never_allocates_on_close() {
        let now = Instant::now();
        let mut runtime = Runtime::new(now);
        runtime.samples = Vec::new();
        runtime.begin_at(owner(1), None, now);
        runtime.close_at(owner(1), RequestOutcome::Refused, None, now);
        assert_eq!(runtime.samples.capacity(), 0);
        let report = runtime.report(1, 0, 0, 0, now);
        assert_eq!(report.census.closed_requests, 1);
        assert_eq!(report.census.lost_measurements, 1);
        assert_eq!(report.census.active_requests, 0);
        assert!(report.samples.is_empty());
    }
    #[test]
    fn capture_runtime_consumed_close_guard_cannot_touch_a_new_session() {
        let first = start();
        begin(owner(1), None);
        let old = phase(owner(1), ServiceStage::Commit);
        drop(first);
        let second = start();
        begin(owner(2), None);
        old.finish_service(Some((owner(1), RequestOutcome::Aborted, None)), false);
        assert_eq!(current_owner(), Some(owner(2)));
        assert_eq!(closed_requests_for_tests(), 0);
        let report = finish_with_scheduler_and_close(
            1,
            0,
            0,
            0,
            Some((owner(2), RequestOutcome::Aborted, None)),
        )
        .unwrap();
        assert_eq!(report.samples.len(), 1);
        assert_eq!(report.samples[0].owner.game_hour_raw, 2);
        drop(second);
    }
    #[test]
    fn capture_runtime_boundary_setup_pushes_only_original_owner_over_limit() {
        let now = Instant::now();
        let mut runtime = Runtime::new(now);
        runtime.begin_at(owner(1), None, now);
        for stage in [
            ServiceStage::Finish,
            ServiceStage::BuildTail,
            ServiceStage::PublicationPrepare,
            ServiceStage::Sink,
            ServiceStage::Commit,
        ] {
            runtime.switch(Phase::Owned(owner(1), stage), now);
        }
        runtime.switch(Phase::Setup, at(now, 1_000_000));
        runtime.switch(Phase::Unattributed, at(now, SERVICE_LIMIT_NS));
        runtime.switch(
            Phase::Owned(owner(1), ServiceStage::Terminal),
            at(now, SERVICE_LIMIT_NS),
        );
        runtime.claim_setup(owner(1), ServiceStage::Terminal);
        runtime.close_at(
            owner(1),
            RequestOutcome::Complete,
            Some((1, 2)),
            at(now, SERVICE_LIMIT_NS + 1_000_000),
        );
        published(
            &mut runtime,
            owner(2),
            SERVICE_LIMIT_NS + 1_000_000,
            SERVICE_LIMIT_NS + 2_000_000,
        );
        runtime.switch(Phase::Unattributed, at(now, SERVICE_LIMIT_NS + 2_000_000));
        let report = runtime.report(2, 0, 0, 0, at(now, SERVICE_LIMIT_NS + 3_000_000));
        assert_eq!(
            report.samples[0].observed_service_ns,
            Some(SERVICE_LIMIT_NS + 1_000_000)
        );
        assert_eq!(
            report.samples[0].stage_ns[ServiceStage::Terminal as usize],
            Some(SERVICE_LIMIT_NS)
        );
        assert_eq!(report.samples[0].data_sequence, Some((1, 2)));
        assert_eq!(report.samples[0].fault_bits, 0);
        assert_eq!(report.samples[1].observed_service_ns, Some(1_000_000));
        assert_eq!(report.census.over_limit_measurements, 1);
        assert_eq!(report.census.unattributed_service_ns, 1_000_000);
        assert_ne!(
            report.census.fault_bits & (1 << TimingFault::UnmeasuredHours as u8),
            0
        );
    }

    #[test]
    fn capture_runtime_preselection_fragments_leave_capacity_for_owned_stages() {
        for fragments in [SETUP_CAPACITY - 1, SETUP_CAPACITY] {
            let now = Instant::now();
            let mut runtime = Runtime::new(now);
            for index in 0..fragments {
                let start = index as u64 * 20;
                runtime.switch(Phase::Idle, at(now, start + 5));
                runtime.switch(Phase::Setup, at(now, start + 20));
            }
            let admitted = fragments as u64 * 20;
            runtime.begin_at(owner(1), None, at(now, admitted));
            for (index, stage) in [
                ServiceStage::Finish,
                ServiceStage::BuildTail,
                ServiceStage::PublicationPrepare,
                ServiceStage::Sink,
                ServiceStage::Commit,
            ]
            .into_iter()
            .enumerate()
            {
                runtime.switch(
                    Phase::Owned(owner(1), stage),
                    at(now, admitted + (index as u64 + 1) * 10),
                );
            }
            runtime.interval(
                owner(1),
                ServiceStage::Sink,
                at(now, admitted + 45),
                at(now, admitted + 55),
            );
            runtime.close_at(
                owner(1),
                RequestOutcome::Complete,
                Some((1, 2)),
                at(now, admitted + 60),
            );
            let report = runtime.report(1, 0, 0, 0, at(now, admitted + 60));
            let sample = report.samples[0];
            assert_eq!(sample.observed_service_ns, Some(fragments as u64 * 5 + 60));
            assert_eq!(
                sample.stage_ns[ServiceStage::Capture as usize],
                Some(fragments as u64 * 5 + 10)
            );
            assert_eq!(sample.stage_ns[ServiceStage::Sink as usize], Some(15));
            assert_eq!(sample.fault_bits, 0);
            assert_eq!(report.census.fault_bits, 0);
            assert_eq!(report.census.complete_measurements, 1);
        }
    }

    #[test]
    fn capture_runtime_preselection_overflow_still_names_unowned_loss() {
        let now = Instant::now();
        let mut runtime = Runtime::new(now);
        for index in 0..=SETUP_CAPACITY {
            let start = index as u64 * 20;
            runtime.switch(Phase::Idle, at(now, start + 5));
            runtime.switch(Phase::Setup, at(now, start + 20));
        }
        let admitted = (SETUP_CAPACITY as u64 + 1) * 20;
        runtime.begin_at(owner(1), None, at(now, admitted));
        runtime.close_at(
            owner(1),
            RequestOutcome::Refused,
            None,
            at(now, admitted + 10),
        );
        let report = runtime.report(1, 0, 0, 0, at(now, admitted + 10));
        assert_eq!(
            report.samples[0].observed_service_ns,
            Some(SETUP_CAPACITY as u64 * 5 + 10)
        );
        assert_eq!(report.census.unattributed_service_ns, 5);
        assert_ne!(
            report.census.fault_bits & (1 << TimingFault::IntervalCapacity as u8),
            0
        );
        assert_ne!(
            report.census.fault_bits & (1 << TimingFault::UnmeasuredHours as u8),
            0
        );
    }

    #[test]
    fn capture_runtime_owns_every_hour_outcome_class_and_names_unmeasured_retirements() {
        let now = Instant::now();
        let mut runtime = Runtime::new(now);
        let outcomes = [
            RequestOutcome::Complete,
            RequestOutcome::Partial,
            RequestOutcome::Refused,
            RequestOutcome::Aborted,
            RequestOutcome::TeardownCut,
        ];
        for (index, outcome) in outcomes.into_iter().enumerate() {
            let who = owner(index as i32 + 1);
            let base = index as u64 * 20;
            runtime.begin_at(who, None, at(now, base));
            for (offset, stage) in [
                ServiceStage::Finish,
                ServiceStage::BuildTail,
                ServiceStage::PublicationPrepare,
                ServiceStage::Sink,
                ServiceStage::Commit,
            ]
            .into_iter()
            .enumerate()
            {
                runtime.switch(Phase::Owned(who, stage), at(now, base + offset as u64 + 1));
            }
            let data = matches!(outcome, RequestOutcome::Complete | RequestOutcome::Partial)
                .then_some((index as u64 * 10 + 1, index as u64 * 10 + 3));
            runtime.close_at(who, outcome, data, at(now, base + 7));
        }
        let report = runtime.report(6, 0, 1, 1, at(now, 100));
        assert_eq!(report.samples.len(), 5);
        assert_eq!(
            report
                .samples
                .iter()
                .map(|sample| sample.outcome)
                .collect::<Vec<_>>(),
            vec![
                HourlyServiceOutcomeV1::Complete,
                HourlyServiceOutcomeV1::Partial,
                HourlyServiceOutcomeV1::Refused,
                HourlyServiceOutcomeV1::Aborted,
                HourlyServiceOutcomeV1::TeardownCut,
            ],
        );
        assert_eq!(report.census.admitted_requests, 5);
        assert_eq!(report.census.closed_requests, 5);
        assert_eq!(
            report.census.complete_measurements, 4,
            "the teardown cut is a boundary, not a measurement; the mid-run abort \
             is still a measured hour"
        );
        assert_eq!(report.census.incomplete_measurements, 1);
        assert_eq!(report.census.boundary_excluded_hours, 1);
        assert_eq!(report.census.partial_requests, 1);
        assert_eq!(report.census.refused_requests, 1);
        assert_eq!(report.census.aborted_requests, 2, "the cut is counted with the aborts");
        assert_eq!(
            report.census.total_service_ns,
            report.samples[..4]
                .iter()
                .map(|sample| sample.observed_service_ns.unwrap())
                .sum::<u64>(),
            "the cut sample's service does not enter the population totals"
        );
        assert_eq!(report.census.unmeasured_hours, 3);
        assert_ne!(
            report.census.fault_bits & (1 << TimingFault::UnmeasuredHours as u8),
            0,
        );
    }

    #[test]
    fn capture_timer_hot_path_has_zero_locks_zero_growth_and_three_clock_reads_max() {
        // include_str! carries the checkout's line endings verbatim, so the
        // split anchor must survive a CRLF working copy.
        let source = include_str!("hourly_service_capture.rs").replace("\r\n", "\n");
        let production = source.split_once("\n#[cfg(test)]\nmod tests {").unwrap().0;
        for forbidden in [
            "Mutex",
            "RwLock",
            "Condvar",
            "Box::",
            "String::",
            "format!(",
            ".collect::<",
        ] {
            assert!(
                !production.contains(forbidden),
                "forbidden hot-path token {forbidden}"
            );
        }
        assert_eq!(production.matches("Vec::new()").count(), 1);
        assert_eq!(production.matches("try_reserve_exact").count(), 1);
        assert_eq!(production.matches("self.samples.push(sample)").count(), 1);
        let queue = production
            .split_once("fn queue(")
            .unwrap()
            .1
            .split_once("fn close_at(")
            .unwrap()
            .0;
        assert!(
            queue
                .find("self.samples.len() >= self.samples.capacity()")
                .unwrap()
                < queue.find("self.samples.push(sample)").unwrap()
        );

        let close_source = production
            .split_once("pub(crate) fn close(")
            .unwrap()
            .1
            .split_once("pub(crate) fn note_scheduler_retired")
            .unwrap()
            .0;
        let finish = production
            .split_once("pub(crate) fn finish_with_scheduler_and_close(")
            .unwrap()
            .1;
        let guarded_phase = production
            .split_once("fn guarded_phase(")
            .unwrap()
            .1
            .split_once("pub(crate) fn begin(")
            .unwrap()
            .0;
        assert_eq!(close_source.matches("Instant::now()").count(), 2);
        assert!(finish.matches("Instant::now()").count() <= 3);
        assert_eq!(guarded_phase.matches("Instant::now()").count(), 1);

        let now = Instant::now();
        let session = start();
        let (report, allocations) = crate::test_allocation_counter::measure(|| {
            begin(owner(1), Some(now));
            set_stage(ServiceStage::Finish);
            {
                let _sink = phase(owner(1), ServiceStage::Sink);
            }
            close(owner(1), RequestOutcome::Refused, None);
            finish_with_scheduler(1, 0, 0, 0).unwrap()
        });
        assert_eq!(allocations, 0);
        assert_eq!(report.samples.len(), 1);
        assert_eq!(report.census.complete_measurements, 1);
        drop(session);
    }
}
