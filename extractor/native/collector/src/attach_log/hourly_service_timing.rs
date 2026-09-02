use std::time::{Duration, Instant};

pub(crate) const SERVICE_LIMIT_NS: u64 = 16_000_000;
const REQUEST_CAPACITY: usize = 4;
const INTERVAL_CAPACITY: usize = 64;
const STAGE_COUNT: usize = 7;
const ALL_STAGES: u8 = (1 << STAGE_COUNT) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ServiceOwner {
    pub logical_stream_id: u64,
    pub epoch: u64,
    pub game_hour_raw: i32,
    pub observation_ordinal: u64,
}

impl ServiceOwner {
    fn follows(self, previous: Self) -> bool {
        if self.logical_stream_id != previous.logical_stream_id {
            return self.logical_stream_id > previous.logical_stream_id;
        }
        if self.epoch != previous.epoch {
            return self.epoch > previous.epoch;
        }
        self.game_hour_raw > previous.game_hour_raw
            && self.observation_ordinal > previous.observation_ordinal
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum ServiceStage {
    Capture,
    Finish,
    BuildTail,
    PublicationPrepare,
    Sink,
    Commit,
    Terminal,
}

const STAGES: [ServiceStage; STAGE_COUNT] = [
    ServiceStage::Capture,
    ServiceStage::Finish,
    ServiceStage::BuildTail,
    ServiceStage::PublicationPrepare,
    ServiceStage::Sink,
    ServiceStage::Commit,
    ServiceStage::Terminal,
];

impl ServiceStage {
    fn bit(self) -> u8 {
        1 << self as u8
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum TimingFault {
    OwnerOrder,
    UnknownOwner,
    RequestCapacity,
    IntervalCapacity,
    ClockReversed,
    ClockOverflow,
    BeforeObservedEdge,
    StageAlreadyClosed,
    StageUnobserved,
    MissingStages,
    CompletionBeforeServiceEnd,
    AlreadyClosed,
    CounterOverflow,
    UnclosedRequest,
    CarrierLoss,
    UnmeasuredHours,
    Sealed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TimingFaults(u32);

impl TimingFaults {
    fn insert(&mut self, fault: TimingFault) {
        self.0 |= 1 << fault as u8;
    }
    pub(crate) fn contains(self, fault: TimingFault) -> bool {
        self.0 & (1 << fault as u8) != 0
    }
    pub(crate) fn bits(self) -> u32 {
        self.0
    }
    fn is_empty(self) -> bool {
        self.0 == 0
    }
}

fn duration_ns(duration: Duration) -> Result<u64, TimingFault> {
    u64::try_from(duration.as_nanos()).map_err(|_| TimingFault::ClockOverflow)
}

pub(crate) fn elapsed_ns(origin: Instant, mark: Instant) -> Result<u64, TimingFault> {
    duration_ns(
        mark.checked_duration_since(origin)
            .ok_or(TimingFault::ClockReversed)?,
    )
}

pub(crate) fn micros_ceil(nanoseconds: u64) -> u64 {
    nanoseconds / 1_000 + u64::from(nanoseconds % 1_000 != 0)
}

#[derive(Clone, Copy)]
struct ServiceInterval {
    start_ns: u64,
    end_ns: u64,
    stage: ServiceStage,
}

impl ServiceInterval {
    const EMPTY: Self = Self {
        start_ns: 0,
        end_ns: 0,
        stage: ServiceStage::Capture,
    };
}

fn interval_union_ns(intervals: &mut [ServiceInterval]) -> Result<u64, TimingFault> {
    intervals.sort_unstable_by_key(|interval| (interval.start_ns, interval.end_ns));
    let mut total = 0u64;
    let mut span: Option<(u64, u64)> = None;
    for interval in intervals {
        if interval.end_ns < interval.start_ns {
            return Err(TimingFault::ClockReversed);
        }
        span = Some(match span {
            Some((start, end)) if interval.start_ns <= end => (start, end.max(interval.end_ns)),
            Some((start, end)) => {
                total = total
                    .checked_add(end - start)
                    .ok_or(TimingFault::CounterOverflow)?;
                (interval.start_ns, interval.end_ns)
            }
            None => (interval.start_ns, interval.end_ns),
        });
    }
    if let Some((start, end)) = span {
        total = total
            .checked_add(end - start)
            .ok_or(TimingFault::CounterOverflow)?;
    }
    Ok(total)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RequestOutcome {
    Complete,
    Partial,
    Refused,
    Aborted,
    /// The request the capture teardown cut: closed by the terminal close
    /// itself, never by the loop. A boundary hour, not a lost one.
    TeardownCut,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ClosedServiceMeasurement {
    pub owner: ServiceOwner,
    pub outcome: RequestOutcome,
    pub observed_service_ns: Option<u64>,
    pub stage_ns: [Option<u64>; STAGE_COUNT],
    pub observed_stages: u8,
    pub closed_stages: u8,
    pub not_applicable_stages: u8,
    pub edge_to_publication_ns: Option<u64>,
    pub edge_to_completion_ns: Option<u64>,
    pub faults: TimingFaults,
}

impl ClosedServiceMeasurement {
    pub(crate) fn complete_service_ns(self) -> Option<u64> {
        if self.faults.is_empty() {
            self.observed_service_ns
        } else {
            None
        }
    }
    pub(crate) fn exceeds_limit(self) -> Option<bool> {
        self.complete_service_ns().map(|ns| ns > SERVICE_LIMIT_NS)
    }
}

struct ActiveMeasurement {
    owner: ServiceOwner,
    observed_edge_ns: u64,
    intervals: [ServiceInterval; INTERVAL_CAPACITY],
    interval_count: usize,
    completed_prefix: Option<(u64, u64)>,
    observed_stages: u8,
    closed_stages: u8,
    not_applicable_stages: u8,
    stage_last_end_ns: [Option<u64>; STAGE_COUNT],
    faults: TimingFaults,
}

impl ActiveMeasurement {
    fn new(owner: ServiceOwner, observed_edge_ns: u64) -> Self {
        Self {
            owner,
            observed_edge_ns,
            intervals: [ServiceInterval::EMPTY; INTERVAL_CAPACITY],
            interval_count: 0,
            completed_prefix: None,
            observed_stages: 0,
            closed_stages: 0,
            not_applicable_stages: 0,
            stage_last_end_ns: [None; STAGE_COUNT],
            faults: TimingFaults::default(),
        }
    }
    fn refuse(&mut self, fault: TimingFault) -> Result<(), TimingFault> {
        self.faults.insert(fault);
        Err(fault)
    }
    fn record_preselection(
        &mut self,
        intervals: &[(u64, u64)],
        completed_ns: u64,
    ) -> Result<(), TimingFault> {
        if self.completed_prefix.is_some() || self.observed_stages != 0 || self.closed_stages != 0 {
            return self.refuse(TimingFault::AlreadyClosed);
        }
        if completed_ns < self.observed_edge_ns {
            return self.refuse(TimingFault::BeforeObservedEdge);
        }
        let mut total = 0u64;
        let mut previous_end = self.observed_edge_ns;
        for &(start, end) in intervals {
            if start < previous_end || end < start || end > completed_ns {
                return self.refuse(TimingFault::ClockReversed);
            }
            let Some(next) = total.checked_add(end - start) else {
                return self.refuse(TimingFault::CounterOverflow);
            };
            total = next;
            previous_end = end;
        }
        // Admission closes this prefix; later owned intervals may not overlap it.
        self.completed_prefix = Some((total, completed_ns));
        if !intervals.is_empty() {
            self.observed_stages |= ServiceStage::Capture.bit();
            self.stage_last_end_ns[ServiceStage::Capture as usize] = Some(previous_end);
        }
        Ok(())
    }
    fn record(
        &mut self,
        stage: ServiceStage,
        start_ns: u64,
        end_ns: u64,
    ) -> Result<(), TimingFault> {
        if self.closed_stages & stage.bit() != 0 {
            return self.refuse(TimingFault::StageAlreadyClosed);
        }
        if end_ns < start_ns {
            return self.refuse(TimingFault::ClockReversed);
        }
        if start_ns < self.observed_edge_ns
            || self
                .completed_prefix
                .is_some_and(|(_, completed)| start_ns < completed)
        {
            return self.refuse(TimingFault::BeforeObservedEdge);
        }
        if start_ns != end_ns {
            if self.interval_count == INTERVAL_CAPACITY {
                return self.refuse(TimingFault::IntervalCapacity);
            }
            self.intervals[self.interval_count] = ServiceInterval {
                start_ns,
                end_ns,
                stage,
            };
            self.interval_count += 1;
        }
        self.observed_stages |= stage.bit();
        let last = &mut self.stage_last_end_ns[stage as usize];
        *last = Some(last.map_or(end_ns, |previous| previous.max(end_ns)));
        Ok(())
    }
    fn finish_stage(&mut self, stage: ServiceStage) -> Result<(), TimingFault> {
        if self.closed_stages & stage.bit() != 0 {
            return self.refuse(TimingFault::StageAlreadyClosed);
        }
        if self.observed_stages & stage.bit() == 0 {
            return self.refuse(TimingFault::StageUnobserved);
        }
        self.closed_stages |= stage.bit();
        Ok(())
    }
    fn stage_not_applicable(&mut self, stage: ServiceStage) -> Result<(), TimingFault> {
        let bit = stage.bit();
        if (self.observed_stages | self.closed_stages) & bit != 0 {
            return self.refuse(TimingFault::StageAlreadyClosed);
        }
        self.not_applicable_stages |= bit;
        self.closed_stages |= bit;
        Ok(())
    }
    fn close(mut self, outcome: RequestOutcome, completed_ns: u64) -> ClosedServiceMeasurement {
        if self.closed_stages != ALL_STAGES {
            self.faults.insert(TimingFault::MissingStages);
        }
        if matches!(outcome, RequestOutcome::Complete | RequestOutcome::Partial)
            && self.not_applicable_stages & !ServiceStage::Terminal.bit() != 0
        {
            self.faults.insert(TimingFault::MissingStages);
        }
        let edge_to_completion_ns = completed_ns.checked_sub(self.observed_edge_ns);
        if edge_to_completion_ns.is_none() {
            self.faults.insert(TimingFault::ClockReversed);
        }
        if self
            .stage_last_end_ns
            .iter()
            .flatten()
            .any(|end| *end > completed_ns)
        {
            self.faults.insert(TimingFault::CompletionBeforeServiceEnd);
        }
        if self
            .completed_prefix
            .is_some_and(|(_, end)| end > completed_ns)
        {
            self.faults.insert(TimingFault::CompletionBeforeServiceEnd);
        }
        let prefix_ns = self.completed_prefix.map_or(0, |(ns, _)| ns);
        let mut stage_ns = [None; STAGE_COUNT];
        for stage in STAGES {
            if self.observed_stages & stage.bit() == 0 {
                continue;
            }
            let mut selected = [ServiceInterval::EMPTY; INTERVAL_CAPACITY];
            let mut count = 0;
            for interval in &self.intervals[..self.interval_count] {
                if interval.stage == stage {
                    selected[count] = *interval;
                    count += 1;
                }
            }
            let duration = interval_union_ns(&mut selected[..count]).and_then(|ns| {
                ns.checked_add(if stage == ServiceStage::Capture {
                    prefix_ns
                } else {
                    0
                })
                .ok_or(TimingFault::CounterOverflow)
            });
            match duration {
                Ok(ns) => stage_ns[stage as usize] = Some(ns),
                Err(fault) => self.faults.insert(fault),
            }
        }
        let observed_service_ns =
            match interval_union_ns(&mut self.intervals[..self.interval_count]).and_then(|ns| {
                ns.checked_add(prefix_ns)
                    .ok_or(TimingFault::CounterOverflow)
            }) {
                Ok(ns) => Some(ns),
                Err(fault) => {
                    self.faults.insert(fault);
                    None
                }
            };
        let edge_to_publication_ns = match outcome {
            RequestOutcome::Complete | RequestOutcome::Partial
                if self.closed_stages & ServiceStage::Sink.bit() != 0 =>
            {
                self.stage_last_end_ns[ServiceStage::Sink as usize]
                    .and_then(|end| end.checked_sub(self.observed_edge_ns))
            }
            _ => None,
        };
        ClosedServiceMeasurement {
            owner: self.owner,
            outcome,
            observed_service_ns,
            stage_ns,
            observed_stages: self.observed_stages,
            closed_stages: self.closed_stages,
            not_applicable_stages: self.not_applicable_stages,
            edge_to_publication_ns,
            edge_to_completion_ns,
            faults: self.faults,
        }
    }
}

enum MeasurementSlot {
    Active(ActiveMeasurement),
    Closed(ClosedServiceMeasurement),
}

impl MeasurementSlot {
    fn owner(&self) -> ServiceOwner {
        match self {
            Self::Active(value) => value.owner,
            Self::Closed(value) => value.owner,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CompletionCensus {
    pub offered_requests: u64,
    pub admitted_requests: u64,
    pub closed_requests: u64,
    pub queued_measurements: u64,
    pub lost_measurements: u64,
    pub complete_measurements: u64,
    pub incomplete_measurements: u64,
    pub partial_requests: u64,
    pub refused_requests: u64,
    pub aborted_requests: u64,
    pub unmeasured_hours: u64,
    pub over_limit_measurements: u64,
    pub total_service_ns: u64,
    pub maximum_service_ns: Option<u64>,
    pub active_requests: usize,
    pub pending_delivery: usize,
    pub sealed: bool,
    pub faults: TimingFaults,
}

impl CompletionCensus {
    pub(crate) fn timing_complete(self) -> bool {
        self.sealed
            && self.faults.is_empty()
            && self.admitted_requests > 0
            && self.offered_requests == self.admitted_requests
            && self.admitted_requests == self.closed_requests
            && self.closed_requests == self.queued_measurements
            && self.closed_requests == self.complete_measurements
            && self.incomplete_measurements == 0
            && self.active_requests == 0
            && self.pending_delivery == 0
    }
    // Integration must also prove that every scheduled hour reached this census.
    pub(crate) fn measured_population_within_limit(self) -> Option<bool> {
        if !self.timing_complete() {
            return None;
        }
        Some(self.over_limit_measurements == 0)
    }
}

fn add_count(value: &mut u64, increment: u64, faults: &mut TimingFaults) -> bool {
    match value.checked_add(increment) {
        Some(next) => {
            *value = next;
            true
        }
        None => {
            faults.insert(TimingFault::CounterOverflow);
            false
        }
    }
}

// One monotonic origin; intervals contain owned service, never boundary/queue waits.
pub(crate) struct HourlyServiceTiming {
    slots: [Option<MeasurementSlot>; REQUEST_CAPACITY],
    last_offered_owner: Option<ServiceOwner>,
    last_observed_edge_ns: Option<u64>,
    last_completed_ns: Option<u64>,
    census: CompletionCensus,
}

impl Default for HourlyServiceTiming {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
            last_offered_owner: None,
            last_observed_edge_ns: None,
            last_completed_ns: None,
            census: CompletionCensus::default(),
        }
    }
}

impl HourlyServiceTiming {
    fn refuse<T>(&mut self, fault: TimingFault) -> Result<T, TimingFault> {
        self.census.faults.insert(fault);
        Err(fault)
    }
    fn index(&self, owner: ServiceOwner) -> Option<usize> {
        self.slots
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|slot| slot.owner() == owner))
    }
    pub(crate) fn begin(
        &mut self,
        owner: ServiceOwner,
        observed_edge_ns: u64,
    ) -> Result<(), TimingFault> {
        if self.census.sealed {
            return self.refuse(TimingFault::Sealed);
        }
        if !add_count(
            &mut self.census.offered_requests,
            1,
            &mut self.census.faults,
        ) {
            return Err(TimingFault::CounterOverflow);
        }
        if self
            .last_offered_owner
            .is_some_and(|previous| !owner.follows(previous))
        {
            return self.refuse(TimingFault::OwnerOrder);
        }
        if self
            .last_observed_edge_ns
            .is_some_and(|previous| observed_edge_ns < previous)
        {
            return self.refuse(TimingFault::ClockReversed);
        }
        self.last_offered_owner = Some(owner);
        self.last_observed_edge_ns = Some(observed_edge_ns);
        let Some(index) = self.slots.iter().position(Option::is_none) else {
            return self.refuse(TimingFault::RequestCapacity);
        };
        if !add_count(
            &mut self.census.admitted_requests,
            1,
            &mut self.census.faults,
        ) {
            return Err(TimingFault::CounterOverflow);
        }
        self.slots[index] = Some(MeasurementSlot::Active(ActiveMeasurement::new(
            owner,
            observed_edge_ns,
        )));
        Ok(())
    }
    fn active(&mut self, owner: ServiceOwner) -> Result<&mut ActiveMeasurement, TimingFault> {
        let found = self
            .slots
            .iter_mut()
            .find(|slot| slot.as_ref().is_some_and(|slot| slot.owner() == owner));
        let fault = match found {
            Some(Some(MeasurementSlot::Active(active))) => return Ok(active),
            Some(_) => TimingFault::AlreadyClosed,
            None => TimingFault::UnknownOwner,
        };
        self.census.faults.insert(fault);
        Err(fault)
    }
    pub(crate) fn record_preselection(
        &mut self,
        owner: ServiceOwner,
        intervals: &[(u64, u64)],
        completed_ns: u64,
    ) -> Result<(), TimingFault> {
        self.active(owner)?
            .record_preselection(intervals, completed_ns)
    }
    pub(crate) fn record_interval(
        &mut self,
        owner: ServiceOwner,
        stage: ServiceStage,
        start_ns: u64,
        end_ns: u64,
    ) -> Result<(), TimingFault> {
        self.active(owner)?.record(stage, start_ns, end_ns)
    }
    pub(crate) fn finish_stage(
        &mut self,
        owner: ServiceOwner,
        stage: ServiceStage,
    ) -> Result<(), TimingFault> {
        self.active(owner)?.finish_stage(stage)
    }
    pub(crate) fn terminal_not_applicable(
        &mut self,
        owner: ServiceOwner,
    ) -> Result<(), TimingFault> {
        self.active(owner)?
            .stage_not_applicable(ServiceStage::Terminal)
    }
    pub(crate) fn finish_observed_stages(&mut self, owner: ServiceOwner, outcome: RequestOutcome) {
        let Ok(active) = self.active(owner) else {
            return;
        };
        for stage in STAGES {
            if active.closed_stages & stage.bit() != 0 {
                continue;
            }
            if active.observed_stages & stage.bit() != 0 {
                let _ = active.finish_stage(stage);
            } else if stage == ServiceStage::Terminal
                || matches!(
                    outcome,
                    RequestOutcome::Refused
                        | RequestOutcome::Aborted
                        | RequestOutcome::TeardownCut
                )
            {
                let _ = active.stage_not_applicable(stage);
            }
        }
    }
    pub(crate) fn note_fault(&mut self, fault: TimingFault) {
        self.census.faults.insert(fault);
    }
    pub(crate) fn record_fault(
        &mut self,
        owner: ServiceOwner,
        fault: TimingFault,
    ) -> Result<(), TimingFault> {
        self.active(owner)?.refuse(fault)
    }
    pub(crate) fn close(
        &mut self,
        owner: ServiceOwner,
        outcome: RequestOutcome,
        completed_ns: u64,
    ) -> Result<ClosedServiceMeasurement, TimingFault> {
        let Some(index) = self.index(owner) else {
            return self.refuse(TimingFault::UnknownOwner);
        };
        let active = match self.slots[index].take() {
            Some(MeasurementSlot::Active(active)) => active,
            other => {
                self.slots[index] = other;
                return self.refuse(TimingFault::AlreadyClosed);
            }
        };
        let closed = active.close(outcome, completed_ns);
        self.last_completed_ns = Some(
            self.last_completed_ns
                .map_or(completed_ns, |last| last.max(completed_ns)),
        );
        self.census.faults.0 |= closed.faults.0;
        add_count(&mut self.census.closed_requests, 1, &mut self.census.faults);
        if let Some(ns) = closed.complete_service_ns() {
            add_count(
                &mut self.census.complete_measurements,
                1,
                &mut self.census.faults,
            );
            add_count(
                &mut self.census.total_service_ns,
                ns,
                &mut self.census.faults,
            );
            self.census.maximum_service_ns =
                Some(self.census.maximum_service_ns.map_or(ns, |old| old.max(ns)));
            if ns > SERVICE_LIMIT_NS {
                add_count(
                    &mut self.census.over_limit_measurements,
                    1,
                    &mut self.census.faults,
                );
            }
        } else {
            add_count(
                &mut self.census.incomplete_measurements,
                1,
                &mut self.census.faults,
            );
        }
        match outcome {
            RequestOutcome::Complete => {}
            RequestOutcome::Partial => {
                add_count(
                    &mut self.census.partial_requests,
                    1,
                    &mut self.census.faults,
                );
            }
            RequestOutcome::Refused => {
                add_count(
                    &mut self.census.refused_requests,
                    1,
                    &mut self.census.faults,
                );
            }
            RequestOutcome::Aborted | RequestOutcome::TeardownCut => {
                add_count(
                    &mut self.census.aborted_requests,
                    1,
                    &mut self.census.faults,
                );
            }
        }
        self.slots[index] = Some(MeasurementSlot::Closed(closed));
        Ok(closed)
    }
    pub(crate) fn next_closed(&self) -> Option<ClosedServiceMeasurement> {
        self.slots.iter().find_map(|slot| match slot {
            Some(MeasurementSlot::Closed(closed)) => Some(*closed),
            _ => None,
        })
    }
    fn retire(&mut self, owner: ServiceOwner, delivered: bool) -> Result<(), TimingFault> {
        let Some(index) = self.index(owner) else {
            return self.refuse(TimingFault::UnknownOwner);
        };
        if !matches!(self.slots[index], Some(MeasurementSlot::Closed(_))) {
            return self.refuse(TimingFault::UnclosedRequest);
        }
        self.slots[index] = None;
        if delivered {
            add_count(
                &mut self.census.queued_measurements,
                1,
                &mut self.census.faults,
            );
        } else {
            add_count(
                &mut self.census.lost_measurements,
                1,
                &mut self.census.faults,
            );
            self.census.faults.insert(TimingFault::CarrierLoss);
        }
        Ok(())
    }
    pub(crate) fn release_to_carrier(&mut self, owner: ServiceOwner) -> Result<(), TimingFault> {
        self.retire(owner, true)
    }
    pub(crate) fn discard_undelivered(&mut self, owner: ServiceOwner) -> Result<(), TimingFault> {
        self.retire(owner, false)
    }
    pub(crate) fn note_unmeasured_hours(&mut self, count: u64) {
        if count != 0 {
            add_count(
                &mut self.census.unmeasured_hours,
                count,
                &mut self.census.faults,
            );
            self.census.faults.insert(TimingFault::UnmeasuredHours);
        }
    }
    pub(crate) fn seal(&mut self, completed_ns: u64) {
        if self.census.sealed {
            self.census.faults.insert(TimingFault::Sealed);
            return;
        }
        self.census.sealed = true;
        if self
            .last_completed_ns
            .is_some_and(|last| completed_ns < last)
            || self
                .last_observed_edge_ns
                .is_some_and(|last| completed_ns < last)
        {
            self.census.faults.insert(TimingFault::ClockReversed);
        }
        for index in 0..REQUEST_CAPACITY {
            let owner = match self.slots[index].as_mut() {
                Some(MeasurementSlot::Active(active)) => {
                    active.faults.insert(TimingFault::UnclosedRequest);
                    Some(active.owner)
                }
                _ => None,
            };
            if let Some(owner) = owner {
                let _ = self.close(owner, RequestOutcome::Aborted, completed_ns);
            }
        }
    }
    pub(crate) fn census(&self) -> CompletionCensus {
        let mut census = self.census;
        for slot in &self.slots {
            match slot {
                Some(MeasurementSlot::Active(_)) => census.active_requests += 1,
                Some(MeasurementSlot::Closed(_)) => census.pending_delivery += 1,
                None => {}
            }
        }
        census
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(hour: i32) -> ServiceOwner {
        ServiceOwner {
            logical_stream_id: 0,
            epoch: 0,
            game_hour_raw: hour,
            observation_ordinal: hour as u64,
        }
    }
    fn finish_remaining(timing: &mut HourlyServiceTiming, who: ServiceOwner, at: u64) {
        for stage in STAGES {
            if stage == ServiceStage::Terminal {
                timing.terminal_not_applicable(who).unwrap();
            } else if timing.active(who).unwrap().closed_stages & stage.bit() == 0 {
                timing.record_interval(who, stage, at, at).unwrap();
                timing.finish_stage(who, stage).unwrap();
            }
        }
    }
    fn closed(
        timing: &mut HourlyServiceTiming,
        who: ServiceOwner,
        ns: u64,
        outcome: RequestOutcome,
    ) -> ClosedServiceMeasurement {
        timing.begin(who, 0).unwrap();
        timing
            .record_interval(who, ServiceStage::Capture, 0, ns)
            .unwrap();
        finish_remaining(timing, who, ns);
        timing.close(who, outcome, ns).unwrap()
    }

    #[test]
    fn owned_union_counts_overlap_once_and_leaves_waits_in_latency() {
        let mut timing = HourlyServiceTiming::default();
        let who = owner(1);
        timing.begin(who, 100).unwrap();
        timing
            .record_interval(who, ServiceStage::Capture, 200, 500)
            .unwrap();
        timing
            .record_interval(who, ServiceStage::Capture, 100, 400)
            .unwrap();
        timing.finish_stage(who, ServiceStage::Capture).unwrap();
        timing
            .record_interval(who, ServiceStage::Finish, 450, 600)
            .unwrap();
        timing.finish_stage(who, ServiceStage::Finish).unwrap();
        timing
            .record_interval(who, ServiceStage::Sink, 10_000, 10_100)
            .unwrap();
        timing.finish_stage(who, ServiceStage::Sink).unwrap();
        finish_remaining(&mut timing, who, 10_200);
        let sample = timing.close(who, RequestOutcome::Complete, 10_200).unwrap();
        assert_eq!(sample.complete_service_ns(), Some(600));
        assert_eq!(sample.stage_ns[ServiceStage::Capture as usize], Some(400));
        assert_eq!(sample.stage_ns[ServiceStage::Finish as usize], Some(150));
        assert_eq!(sample.edge_to_publication_ns, Some(10_000));
        assert_eq!(sample.edge_to_completion_ns, Some(10_100));
    }

    #[test]
    fn nanosecond_overrun_cannot_round_down_to_sixteen_milliseconds() {
        for (ns, expected) in [(SERVICE_LIMIT_NS, false), (SERVICE_LIMIT_NS + 1, true)] {
            let mut timing = HourlyServiceTiming::default();
            let sample = closed(&mut timing, owner(1), ns, RequestOutcome::Complete);
            assert_eq!(sample.exceeds_limit(), Some(expected));
            assert_eq!(micros_ceil(ns), if expected { 16_001 } else { 16_000 });
            timing.release_to_carrier(owner(1)).unwrap();
            timing.seal(ns);
            assert_eq!(
                timing.census().measured_population_within_limit(),
                Some(!expected)
            );
        }
        assert_eq!(micros_ceil(u64::MAX), u64::MAX / 1_000 + 1);
    }

    #[test]
    fn final_request_needs_no_next_data_but_does_need_delivery() {
        let mut timing = HourlyServiceTiming::default();
        let sample = closed(&mut timing, owner(1), 20_000, RequestOutcome::Complete);
        timing.seal(50_000);
        assert_eq!(timing.next_closed().unwrap().owner, sample.owner);
        assert!(!timing.census().timing_complete());
        timing.release_to_carrier(sample.owner).unwrap();
        assert!(timing.census().timing_complete());
        assert_eq!(timing.census().maximum_service_ns, Some(20_000));
    }

    #[test]
    fn measured_zero_missing_and_not_applicable_are_distinct() {
        let mut timing = HourlyServiceTiming::default();
        let sample = closed(&mut timing, owner(1), 0, RequestOutcome::Complete);
        assert_eq!(sample.stage_ns[ServiceStage::Capture as usize], Some(0));
        assert_eq!(sample.stage_ns[ServiceStage::Terminal as usize], None);
        assert_eq!(sample.not_applicable_stages, ServiceStage::Terminal.bit());
        assert_ne!(sample.observed_stages & ServiceStage::Capture.bit(), 0);
        assert_eq!(sample.closed_stages, ALL_STAGES);
        timing.begin(owner(2), 0).unwrap();
        let missing = timing.close(owner(2), RequestOutcome::Complete, 0).unwrap();
        assert_eq!(missing.observed_service_ns, Some(0));
        assert_eq!(missing.complete_service_ns(), None);
        assert!(missing.faults.contains(TimingFault::MissingStages));
    }

    #[test]
    fn early_close_and_unclosed_terminal_requests_keep_their_cause() {
        let mut timing = HourlyServiceTiming::default();
        timing.begin(owner(1), 100).unwrap();
        timing
            .record_interval(owner(1), ServiceStage::Capture, 100, 200)
            .unwrap();
        timing.seal(150);
        let sample = timing.next_closed().unwrap();
        assert_eq!(sample.outcome, RequestOutcome::Aborted);
        assert_eq!(sample.observed_service_ns, Some(100));
        assert!(sample.faults.contains(TimingFault::UnclosedRequest));
        assert!(sample.faults.contains(TimingFault::MissingStages));
        assert!(sample
            .faults
            .contains(TimingFault::CompletionBeforeServiceEnd));
        timing.release_to_carrier(owner(1)).unwrap();
        assert_eq!(timing.census().measured_population_within_limit(), None);
    }

    #[test]
    fn delayed_publication_keeps_owner_while_next_hour_finishes_first() {
        let mut timing = HourlyServiceTiming::default();
        timing.begin(owner(1), 0).unwrap();
        timing
            .record_interval(owner(1), ServiceStage::Capture, 0, 100)
            .unwrap();
        timing.begin(owner(2), 100).unwrap();
        timing
            .record_interval(owner(2), ServiceStage::Capture, 100, 300)
            .unwrap();
        finish_remaining(&mut timing, owner(2), 300);
        let second = timing
            .close(owner(2), RequestOutcome::Complete, 300)
            .unwrap();
        timing
            .record_interval(owner(1), ServiceStage::Sink, 200, 400)
            .unwrap();
        finish_remaining(&mut timing, owner(1), 400);
        let first = timing
            .close(owner(1), RequestOutcome::Complete, 400)
            .unwrap();
        assert_eq!(first.complete_service_ns(), Some(300));
        assert_eq!(second.complete_service_ns(), Some(200));
        assert_eq!(first.owner.game_hour_raw, 1);
        assert_eq!(second.owner.game_hour_raw, 2);
    }

    #[test]
    fn invalid_clocks_remain_invalid_after_later_valid_intervals() {
        for (start, end, fault) in [
            (200, 199, TimingFault::ClockReversed),
            (99, 200, TimingFault::BeforeObservedEdge),
        ] {
            let mut timing = HourlyServiceTiming::default();
            timing.begin(owner(1), 100).unwrap();
            assert_eq!(
                timing.record_interval(owner(1), ServiceStage::Capture, start, end),
                Err(fault)
            );
            finish_remaining(&mut timing, owner(1), 200);
            let sample = timing
                .close(owner(1), RequestOutcome::Complete, 200)
                .unwrap();
            assert!(sample.faults.contains(fault));
            assert_eq!(sample.complete_service_ns(), None);
        }
        let origin = Instant::now();
        assert_eq!(elapsed_ns(origin, origin), Ok(0));
        assert_eq!(
            elapsed_ns(origin + Duration::from_millis(1), origin),
            Err(TimingFault::ClockReversed)
        );
        assert_eq!(duration_ns(Duration::from_nanos(u64::MAX)), Ok(u64::MAX));
        assert_eq!(
            duration_ns(Duration::from_secs(u64::MAX)),
            Err(TimingFault::ClockOverflow)
        );
    }

    #[test]
    fn interval_capacity_is_sticky_loss_and_never_silently_truncates() {
        let mut timing = HourlyServiceTiming::default();
        timing.begin(owner(1), 0).unwrap();
        for index in 0..INTERVAL_CAPACITY {
            let start = index as u64 * 2;
            timing
                .record_interval(owner(1), ServiceStage::Capture, start, start + 1)
                .unwrap();
        }
        assert_eq!(
            timing.record_interval(owner(1), ServiceStage::Capture, 500, 501),
            Err(TimingFault::IntervalCapacity)
        );
        finish_remaining(&mut timing, owner(1), 600);
        let sample = timing
            .close(owner(1), RequestOutcome::Complete, 600)
            .unwrap();
        assert_eq!(sample.observed_service_ns, Some(INTERVAL_CAPACITY as u64));
        assert_eq!(sample.complete_service_ns(), None);
    }

    #[test]
    fn undelivered_samples_hold_capacity_and_explicit_loss_releases_it() {
        let mut timing = HourlyServiceTiming::default();
        for hour in 1..=REQUEST_CAPACITY {
            closed(&mut timing, owner(hour as i32), 1, RequestOutcome::Complete);
        }
        assert_eq!(timing.begin(owner(5), 0), Err(TimingFault::RequestCapacity));
        timing.discard_undelivered(owner(1)).unwrap();
        timing.begin(owner(6), 0).unwrap();
        assert_eq!(timing.census().lost_measurements, 1);
        assert!(timing.census().faults.contains(TimingFault::CarrierLoss));
        assert!(timing
            .census()
            .faults
            .contains(TimingFault::RequestCapacity));
    }

    #[test]
    fn owner_history_is_bounded_and_prevents_reusing_retired_hours() {
        let mut timing = HourlyServiceTiming::default();
        for hour in 1..=1_000 {
            closed(&mut timing, owner(hour), 1, RequestOutcome::Complete);
            timing.release_to_carrier(owner(hour)).unwrap();
        }
        assert_eq!(timing.census().pending_delivery, 0);
        assert_eq!(timing.begin(owner(1), 0), Err(TimingFault::OwnerOrder));
        let mut rollover = owner(1);
        rollover.epoch = 1;
        timing.begin(rollover, 0).unwrap();
        let mut new_stream = owner(1);
        new_stream.logical_stream_id = 1;
        timing.begin(new_stream, 0).unwrap();
    }

    #[test]
    fn wrong_owner_duplicate_delivery_and_stage_reopen_are_refused() {
        let mut timing = HourlyServiceTiming::default();
        timing.begin(owner(1), 0).unwrap();
        assert_eq!(
            timing.finish_stage(owner(1), ServiceStage::Capture),
            Err(TimingFault::StageUnobserved)
        );
        timing
            .record_interval(owner(1), ServiceStage::Capture, 0, 1)
            .unwrap();
        timing
            .finish_stage(owner(1), ServiceStage::Capture)
            .unwrap();
        assert_eq!(
            timing.record_interval(owner(1), ServiceStage::Capture, 1, 2),
            Err(TimingFault::StageAlreadyClosed)
        );
        assert_eq!(
            timing.record_interval(owner(2), ServiceStage::Capture, 0, 1),
            Err(TimingFault::UnknownOwner)
        );
        finish_remaining(&mut timing, owner(1), 1);
        timing.close(owner(1), RequestOutcome::Complete, 1).unwrap();
        assert!(matches!(
            timing.close(owner(1), RequestOutcome::Complete, 1),
            Err(TimingFault::AlreadyClosed)
        ));
        timing.release_to_carrier(owner(1)).unwrap();
        assert_eq!(
            timing.release_to_carrier(owner(1)),
            Err(TimingFault::UnknownOwner)
        );
    }

    #[test]
    fn partial_and_refused_samples_remain_in_the_timing_population() {
        for outcome in [
            RequestOutcome::Partial,
            RequestOutcome::Refused,
            RequestOutcome::Aborted,
        ] {
            let mut timing = HourlyServiceTiming::default();
            let sample = closed(&mut timing, owner(1), SERVICE_LIMIT_NS + 1, outcome);
            assert_eq!(sample.exceeds_limit(), Some(true));
            timing.release_to_carrier(owner(1)).unwrap();
            timing.seal(SERVICE_LIMIT_NS + 1);
            let census = timing.census();
            assert!(census.timing_complete());
            assert_eq!(census.over_limit_measurements, 1);
            assert_eq!(census.maximum_service_ns, Some(SERVICE_LIMIT_NS + 1));
            assert_eq!(census.measured_population_within_limit(), Some(false));
        }
    }

    #[test]
    fn empty_run_gaps_and_counter_overflow_never_produce_acceptance() {
        let mut empty = HourlyServiceTiming::default();
        empty.seal(0);
        assert_eq!(empty.census().measured_population_within_limit(), None);
        let mut gap = HourlyServiceTiming::default();
        closed(&mut gap, owner(1), 0, RequestOutcome::Complete);
        gap.release_to_carrier(owner(1)).unwrap();
        gap.note_unmeasured_hours(2);
        gap.seal(0);
        assert_eq!(gap.census().unmeasured_hours, 2);
        assert_eq!(gap.census().measured_population_within_limit(), None);
        let mut overflow = HourlyServiceTiming::default();
        overflow.census.offered_requests = u64::MAX;
        assert_eq!(
            overflow.begin(owner(1), 0),
            Err(TimingFault::CounterOverflow)
        );
        assert!(overflow
            .census()
            .faults
            .contains(TimingFault::CounterOverflow));
        let mut total_overflow = HourlyServiceTiming::default();
        total_overflow.census.total_service_ns = u64::MAX;
        closed(&mut total_overflow, owner(1), 1, RequestOutcome::Complete);
        total_overflow.release_to_carrier(owner(1)).unwrap();
        total_overflow.seal(1);
        assert_eq!(
            total_overflow.census().measured_population_within_limit(),
            None
        );
    }

    #[test]
    fn terminal_work_after_capture_window_is_owned_and_cannot_be_waived() {
        let mut timing = HourlyServiceTiming::default();
        timing.begin(owner(1), 0).unwrap();
        for stage in STAGES {
            let (start, end) = if stage == ServiceStage::Terminal {
                (100_000_000, 116_000_001)
            } else {
                (0, 0)
            };
            timing.record_interval(owner(1), stage, start, end).unwrap();
            timing.finish_stage(owner(1), stage).unwrap();
        }
        let sample = timing
            .close(owner(1), RequestOutcome::Complete, 116_000_001)
            .unwrap();
        assert_eq!(sample.complete_service_ns(), Some(16_000_001));
        assert_eq!(sample.edge_to_completion_ns, Some(116_000_001));
        assert_eq!(sample.exceeds_limit(), Some(true));
        timing.release_to_carrier(owner(1)).unwrap();
        timing.seal(116_000_001);
        assert_eq!(
            timing.begin(owner(2), 120_000_000),
            Err(TimingFault::Sealed)
        );
    }
    #[test]
    fn seal_clock_cannot_precede_already_delivered_service() {
        let mut timing = HourlyServiceTiming::default();
        closed(&mut timing, owner(1), 100, RequestOutcome::Complete);
        timing.release_to_carrier(owner(1)).unwrap();
        timing.seal(99);
        assert!(timing.census().faults.contains(TimingFault::ClockReversed));
        assert_eq!(timing.census().measured_population_within_limit(), None);
    }

    #[test]
    fn external_clock_failure_is_sticky_and_cannot_be_reported_as_zero() {
        let mut timing = HourlyServiceTiming::default();
        timing.begin(owner(1), 0).unwrap();
        assert_eq!(
            timing.record_fault(owner(1), TimingFault::ClockOverflow),
            Err(TimingFault::ClockOverflow)
        );
        finish_remaining(&mut timing, owner(1), 0);
        let sample = timing.close(owner(1), RequestOutcome::Complete, 0).unwrap();
        assert_eq!(sample.complete_service_ns(), None);
        assert!(sample.faults.contains(TimingFault::ClockOverflow));
    }

    #[test]
    fn completed_preselection_counts_only_work_and_keeps_late_sink_overlap_exact() {
        let who = owner(1);
        let mut timing = HourlyServiceTiming::default();
        timing.begin(who, 0).unwrap();
        timing
            .record_preselection(who, &[(0, 10), (100, 110)], 200)
            .unwrap();
        timing
            .record_interval(who, ServiceStage::Capture, 200, 220)
            .unwrap();
        timing
            .record_interval(who, ServiceStage::Finish, 210, 240)
            .unwrap();
        timing
            .record_interval(who, ServiceStage::Sink, 215, 245)
            .unwrap();
        finish_remaining(&mut timing, who, 245);
        let sample = timing.close(who, RequestOutcome::Complete, 245).unwrap();
        assert_eq!(sample.complete_service_ns(), Some(65));
        assert_eq!(sample.stage_ns[ServiceStage::Capture as usize], Some(40));
        assert_eq!(sample.stage_ns[ServiceStage::Finish as usize], Some(30));
        assert_eq!(sample.stage_ns[ServiceStage::Sink as usize], Some(30));
        assert_eq!(sample.edge_to_completion_ns, Some(245));
    }

    #[test]
    fn completed_preselection_rejects_overlapping_or_reopened_history() {
        for spans in [vec![(10, 5)], vec![(0, 10), (5, 15)], vec![(0, 21)]] {
            let mut active = ActiveMeasurement::new(owner(1), 0);
            assert_eq!(
                active.record_preselection(&spans, 20),
                Err(TimingFault::ClockReversed)
            );
            assert!(active.completed_prefix.is_none());
            active.record_preselection(&[(0, 10)], 20).unwrap();
            assert!(active
                .close(RequestOutcome::Aborted, 20)
                .faults
                .contains(TimingFault::ClockReversed));
        }
        for spans in [Vec::new(), vec![(0, 10)]] {
            let mut active = ActiveMeasurement::new(owner(1), 0);
            active.record_preselection(&spans, 20).unwrap();
            assert_eq!(
                active.record_preselection(&[], 20),
                Err(TimingFault::AlreadyClosed)
            );
            assert_eq!(
                active.record(ServiceStage::Sink, 19, 21),
                Err(TimingFault::BeforeObservedEdge)
            );
            active.record(ServiceStage::Sink, 20, 21).unwrap();
            let sample = active.close(RequestOutcome::Aborted, 19);
            assert!(sample
                .faults
                .contains(TimingFault::CompletionBeforeServiceEnd));
            assert_eq!(sample.complete_service_ns(), None);
        }
        let mut active = ActiveMeasurement::new(owner(1), 0);
        active.record(ServiceStage::Capture, 0, 1).unwrap();
        assert_eq!(
            active.record_preselection(&[], 1),
            Err(TimingFault::AlreadyClosed)
        );
    }
}
