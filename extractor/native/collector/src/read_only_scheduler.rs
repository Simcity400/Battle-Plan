use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::custody_shadow::{CustodyShadow, CustodyShadowSummary, ShadowClock, ShadowMode};

const MAX_FORWARD_DELTA: i64 =
    compact_contract::untrusted_clock_recovery::UNTRUSTED_CLOCK_RECOVERY_TRUST_CEILING_HOURS;

const CLOCK_UNREADABLE_INTERVAL_QUEUE_CAP: usize = 65_535;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockStamp {
    pub epoch: u64,
    pub hour_raw: i32,
    pub observation_ordinal: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GapKind {
    Torn,
    Overrun,
    Backpressure,
    ClockUnreadable,
    WorkerPanic,
    ReloadBaseline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnobservedCause {
    ClockUnknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClockRecoveryTrust {
    NoHoursLost,
    Trusted,
    Untrusted { observed_delta_hours: i64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureRequest {
    pub stamp: ClockStamp,
    pub force_baseline: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulerDecision {
    Idle,
    Capture(CaptureRequest),
    AwaitingNextHour(CaptureRequest),
    AdvancePending(CaptureRequest),
    DomainFrontiersDiverged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DomainGapRange {
    pub domain_id: u16,
    pub epoch: u64,
    pub first_hour_raw: i32,
    pub last_hour_raw: i32,
    pub kind: GapKind,
    pub force_next_baseline: bool,
    pub tick_before: u64,
    pub tick_after: u64,
    pub queue_capacity: u32,
    pub queue_depth: u32,
    pub queue_dropped: u64,
}

impl DomainGapRange {
    fn can_extend(self, domain_id: u16, stamp: ClockStamp, kind: GapKind) -> bool {
        matches!(kind, GapKind::Overrun | GapKind::Backpressure)
            && self.domain_id == domain_id
            && self.epoch == stamp.epoch
            && self.kind == kind
            && self.last_hour_raw.checked_add(1) == Some(stamp.hour_raw)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalOutcome {
    CounterAnomaly,
    ActivationFailed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SchedulerTelemetry {
    pub observations: u64,
    pub scheduled_hours: u64,
    pub accepted_hours: u64,
    pub degraded_hours: u64,
    /// Admitted hours released by [`ReadOnlyScheduler::reject_request`]. These
    pub refused_hours: u64,
    pub evicted_pending_hours: u64,
    /// The subset of `evicted_pending_hours` evicted before ANY request was
    /// taken: hours the clock scheduled while the capture loop was still
    /// setting up. A start-boundary loss, named apart from a mid-run eviction
    /// (which is the loop failing to keep pace).
    pub evicted_before_first_take_hours: u64,
    pub abandoned_hours: u64,
    pub flushed_pending_hours: u64,
    pub unobserved_overrun_hours: u64,
    pub unobserved_clock_unknown_hours: u64,
    pub clock_unreadable_intervals_dropped: u64,
    pub counter_anomaly_events: u64,
    pub untrusted_clock_recovery_events: u64,
    pub max_untrusted_clock_recovery_delta_hours: u64,
    pub untrusted_clock_recoveries_dropped: u64,
    pub post_terminal_observations: u64,
    pub gapped_domain_hours: u64,
    pub overrun_domain_hours: u64,
    pub backpressure_domain_hours: u64,
    pub torn_domain_hours: u64,
    pub worker_panic_domain_hours: u64,
    pub clock_unreadable_events: u64,
    pub control_lane_compactions: u64,
    pub reloads: u64,
    pub max_request_depth: u32,
    pub max_publishing_depth: u32,
    pub max_control_depth: u32,
    pub request_cap: u32,
    pub control_cap: u32,
    pub terminal: Option<TerminalOutcome>,
    pub custody_shadow: CustodyShadowSummary,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HourLedger {
    pub scheduled: u64,
    pub accepted: u64,
    /// Admitted hours released without publication, accounted by `H4FR`.
    pub refused: u64,
    pub evicted_pending: u64,
    pub abandoned: u64,
    pub flushed_pending: u64,
    pub outstanding: u64,
    pub publishing: u64,
    pub unobserved_overrun: u64,
    pub unobserved_clock_unknown: u64,
    pub counter_anomaly_events: u64,
    pub untrusted_clock_recovery_events: u64,
    pub max_untrusted_clock_recovery_delta_hours: u64,
    pub post_terminal_observations: u64,
    pub slots_uniform: bool,
}

impl HourLedger {
    pub fn balanced(&self) -> bool {
        self.slots_uniform
            && self
                .accepted
                .checked_add(self.refused)
                .and_then(|n| n.checked_add(self.evicted_pending))
                .and_then(|n| n.checked_add(self.abandoned))
                .and_then(|n| n.checked_add(self.flushed_pending))
                .and_then(|n| n.checked_add(self.outstanding))
                == Some(self.scheduled)
    }

    pub fn hours_with_no_data_frame(&self) -> u64 {
        self.refused
            .saturating_add(self.evicted_pending)
            .saturating_add(self.abandoned)
            .saturating_add(self.flushed_pending)
            .saturating_add(self.unobserved_overrun)
            .saturating_add(self.unobserved_clock_unknown)
    }
}

#[derive(Debug)]
pub struct ReadOnlyScheduler {
    domains: Vec<u16>,
    domain_requests: Vec<DomainRequestSlot>,
    control: VecDeque<DomainGapRange>,
    reserved_control: Option<Vec<DomainGapRange>>,
    control_cap: usize,
    last: Option<ClockStamp>,
    next_epoch: u64,
    force_baseline: bool,
    clock_unknown: bool,
    clock_unknown_events: u32,
    clock_unreadable_intervals: VecDeque<ClockUnreadableInterval>,
    untrusted_clock_recoveries: VecDeque<UntrustedClockRecovery>,
    telemetry: SchedulerTelemetry,
    shadow: CustodyShadow,
    /// Set by the first take; evictions before it are start-boundary losses.
    any_request_taken: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UntrustedClockRecovery {
    pub last_observed_hour_raw: i32,
    pub resumed_hour_raw: i32,
    pub unreadable_events: u32,
    pub observed_delta_hours: i64,
    pub trust_ceiling_hours: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClockUnreadableInterval {
    pub range_start_hour_raw: i32,
    pub range_end_hour_raw: i32,
    pub last_observed_hour_raw: i32,
    pub resumed_hour_raw: i32,
    pub unreadable_events: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DomainRequestSlot {
    in_flight: Option<CaptureRequest>,
    /// [`ReadOnlyScheduler::release_read_custody`] for why this is a separate
    publishing: Option<CaptureRequest>,
    latest_pending: Option<CaptureRequest>,
}

impl ReadOnlyScheduler {
    pub fn new(
        mut domains: Vec<u16>,
        request_cap: usize,
        control_cap: usize,
    ) -> Result<Self, &'static str> {
        Self::with_custody_shadow(domains, request_cap, control_cap, CustodyShadow::default())
    }

    pub fn new_with_environment_shadow(
        domains: Vec<u16>,
        request_cap: usize,
        control_cap: usize,
    ) -> Result<Self, &'static str> {
        Self::with_custody_shadow(
            domains,
            request_cap,
            control_cap,
            CustodyShadow::from_environment(),
        )
    }

    pub fn with_custody_shadow(
        mut domains: Vec<u16>,
        request_cap: usize,
        control_cap: usize,
        shadow: CustodyShadow,
    ) -> Result<Self, &'static str> {
        domains.sort_unstable();
        domains.dedup();
        if domains.is_empty() {
            return Err("read-only scheduler requires at least one domain");
        }
        if domains.iter().any(|domain| *domain == 0) {
            return Err("read-only scheduler domain id zero is reserved");
        }
        let _requested_queue_cap = request_cap.clamp(1, 65_535);
        let control_cap = control_cap.clamp(domains.len(), 65_535);
        let domain_count = domains.len();
        Ok(Self {
            domains,
            domain_requests: vec![DomainRequestSlot::default(); domain_count],
            control: VecDeque::with_capacity(control_cap.min(1024)),
            reserved_control: None,
            control_cap,
            last: None,
            next_epoch: 0,
            force_baseline: true,
            clock_unknown: false,
            any_request_taken: false,
            clock_unknown_events: 0,
            clock_unreadable_intervals: VecDeque::new(),
            untrusted_clock_recoveries: VecDeque::new(),
            telemetry: SchedulerTelemetry {
                request_cap: 1,
                control_cap: control_cap as u32,
                custody_shadow: shadow.summary(),
                ..SchedulerTelemetry::default()
            },
            shadow,
        })
    }

    pub fn domains(&self) -> &[u16] {
        &self.domains
    }

    pub fn generation(&self) -> u64 {
        self.telemetry.observations
    }

    pub fn telemetry(&self) -> SchedulerTelemetry {
        let mut telemetry = self.telemetry.clone();
        telemetry.custody_shadow = self.shadow.summary();
        telemetry
    }

    pub fn evicted_pending_hours(&self) -> u64 {
        self.telemetry.evicted_pending_hours
    }

    pub fn custody_shadow(&self) -> &CustodyShadow {
        &self.shadow
    }

    #[cfg(test)]
    pub fn set_custody_shadow_hostile(&mut self, hostile: bool) {
        self.shadow.set_hostile(hostile);
    }

    pub fn terminal(&self) -> Option<TerminalOutcome> {
        self.telemetry.terminal
    }

    pub fn hour_ledger(&self) -> HourLedger {
        let t = &self.telemetry;
        HourLedger {
            scheduled: t.scheduled_hours,
            accepted: t.accepted_hours,
            refused: t.refused_hours,
            evicted_pending: t.evicted_pending_hours,
            abandoned: t.abandoned_hours,
            flushed_pending: t.flushed_pending_hours,
            outstanding: self.outstanding_hours(),
            publishing: self.publishing_hours(),
            slots_uniform: self.slots_uniform(),
            unobserved_overrun: t.unobserved_overrun_hours,
            unobserved_clock_unknown: t.unobserved_clock_unknown_hours,
            counter_anomaly_events: t.counter_anomaly_events,
            untrusted_clock_recovery_events: t.untrusted_clock_recovery_events,
            max_untrusted_clock_recovery_delta_hours: t.max_untrusted_clock_recovery_delta_hours,
            post_terminal_observations: t.post_terminal_observations,
        }
    }

    fn outstanding_hours(&self) -> u64 {
        self.domain_requests.first().map_or(0, |slot| {
            u64::from(slot.in_flight.is_some())
                + u64::from(slot.publishing.is_some())
                + u64::from(slot.latest_pending.is_some())
        })
    }

    fn publishing_hours(&self) -> u64 {
        self.domain_requests
            .first()
            .map_or(0, |slot| u64::from(slot.publishing.is_some()))
    }

    fn slots_uniform(&self) -> bool {
        let Some(first) = self.domain_requests.first() else {
            return true;
        };
        self.domain_requests.iter().all(|slot| {
            slot.in_flight == first.in_flight
                && slot.publishing == first.publishing
                && slot.latest_pending == first.latest_pending
        })
    }

    pub fn observe_stable(&mut self, hour_raw: i32, observation_ordinal: u64) {
        if self.telemetry.terminal.is_some() {
            self.telemetry.post_terminal_observations =
                self.telemetry.post_terminal_observations.saturating_add(1);
            return;
        }
        self.telemetry.observations = self.telemetry.observations.saturating_add(1);
        if self.clock_unknown {
            self.gap_all_pending(GapKind::ClockUnreadable);
            match self.clock_recovery_trust(self.last, hour_raw) {
                ClockRecoveryTrust::NoHoursLost => {}
                ClockRecoveryTrust::Trusted => {
                    self.count_unobserved_span(self.last, hour_raw, UnobservedCause::ClockUnknown);
                    self.queue_clock_unreadable_interval(self.last, hour_raw);
                }
                ClockRecoveryTrust::Untrusted {
                    observed_delta_hours,
                } => {
                    self.queue_untrusted_clock_recovery(self.last, hour_raw, observed_delta_hours);
                }
            }
            self.clock_unknown_events = 0;
            self.next_epoch = self.next_epoch.saturating_add(1);
            self.force_baseline = true;
            self.clock_unknown = false;
            let stamp = ClockStamp {
                epoch: self.next_epoch,
                hour_raw,
                observation_ordinal,
            };
            self.last = Some(stamp);
            self.enqueue_request(CaptureRequest {
                stamp,
                force_baseline: true,
            });
            return;
        }
        let Some(last) = self.last else {
            let stamp = ClockStamp {
                epoch: self.next_epoch,
                hour_raw,
                observation_ordinal,
            };
            self.last = Some(stamp);
            self.enqueue_request(CaptureRequest {
                stamp,
                force_baseline: true,
            });
            return;
        };
        if hour_raw == last.hour_raw {
            self.last = Some(ClockStamp {
                observation_ordinal,
                ..last
            });
            return;
        }
        if hour_raw < last.hour_raw {
            self.gap_all_pending(GapKind::ReloadBaseline);
            self.next_epoch = self.next_epoch.saturating_add(1);
            self.telemetry.reloads = self.telemetry.reloads.saturating_add(1);
            self.force_baseline = true;
            let stamp = ClockStamp {
                epoch: self.next_epoch,
                hour_raw,
                observation_ordinal,
            };
            self.last = Some(stamp);
            self.enqueue_request(CaptureRequest {
                stamp,
                force_baseline: true,
            });
            return;
        }
        let delta = i64::from(hour_raw) - i64::from(last.hour_raw);
        if delta > MAX_FORWARD_DELTA {
            self.telemetry.counter_anomaly_events =
                self.telemetry.counter_anomaly_events.saturating_add(1);
            self.telemetry.terminal = Some(TerminalOutcome::CounterAnomaly);
            return;
        }

        if delta > 1 {
            self.push_all_domain_gap_range(
                last.epoch,
                last.hour_raw.saturating_add(1),
                hour_raw.saturating_sub(1),
                GapKind::Overrun,
                clock_witness(last.hour_raw),
                clock_witness(hour_raw.saturating_sub(1)),
            );
            self.telemetry.unobserved_overrun_hours = self
                .telemetry
                .unobserved_overrun_hours
                .saturating_add(u64::try_from(delta - 1).unwrap_or(0));
        }
        let stamp = ClockStamp {
            epoch: last.epoch,
            hour_raw,
            observation_ordinal,
        };
        self.last = Some(stamp);
        self.enqueue_request(CaptureRequest {
            stamp,
            force_baseline: self.force_baseline,
        });
    }

    pub fn note_clock_unreadable(&mut self) {
        self.telemetry.clock_unreadable_events =
            self.telemetry.clock_unreadable_events.saturating_add(1);
        self.clock_unknown_events = self.clock_unknown_events.saturating_add(1);
        if !self.clock_unknown {
            self.gap_all_pending(GapKind::ClockUnreadable);
            self.clock_unknown = true;
            self.force_baseline = true;
        }
    }

    fn queue_clock_unreadable_interval(&mut self, from: Option<ClockStamp>, resumed_hour: i32) {
        let Some(from) = from else {
            return;
        };
        let (Some(range_start), Some(range_end)) =
            (from.hour_raw.checked_add(1), resumed_hour.checked_sub(1))
        else {
            return;
        };
        if range_start > range_end {
            return;
        }
        let unreadable_events = self.clock_unknown_events.max(1);
        if self.clock_unreadable_intervals.len() >= CLOCK_UNREADABLE_INTERVAL_QUEUE_CAP {
            self.telemetry.clock_unreadable_intervals_dropped = self
                .telemetry
                .clock_unreadable_intervals_dropped
                .saturating_add(1);
            return;
        }
        self.clock_unreadable_intervals
            .push_back(ClockUnreadableInterval {
                range_start_hour_raw: range_start,
                range_end_hour_raw: range_end,
                last_observed_hour_raw: from.hour_raw,
                resumed_hour_raw: resumed_hour,
                unreadable_events,
            });
    }

    fn clock_recovery_trust(
        &self,
        from: Option<ClockStamp>,
        resumed_hour: i32,
    ) -> ClockRecoveryTrust {
        let Some(from) = from else {
            return ClockRecoveryTrust::NoHoursLost;
        };
        let delta = i64::from(resumed_hour) - i64::from(from.hour_raw);
        if delta > MAX_FORWARD_DELTA {
            return ClockRecoveryTrust::Untrusted {
                observed_delta_hours: delta,
            };
        }
        if delta <= 1 {
            return ClockRecoveryTrust::NoHoursLost;
        }
        ClockRecoveryTrust::Trusted
    }

    fn queue_untrusted_clock_recovery(
        &mut self,
        from: Option<ClockStamp>,
        resumed_hour: i32,
        observed_delta_hours: i64,
    ) {
        let Some(from) = from else {
            return;
        };
        self.telemetry.untrusted_clock_recovery_events = self
            .telemetry
            .untrusted_clock_recovery_events
            .saturating_add(1);
        let delta_magnitude = u64::try_from(observed_delta_hours).unwrap_or(0);
        self.telemetry.max_untrusted_clock_recovery_delta_hours = self
            .telemetry
            .max_untrusted_clock_recovery_delta_hours
            .max(delta_magnitude);
        let unreadable_events = self.clock_unknown_events.max(1);
        if self.untrusted_clock_recoveries.len() >= CLOCK_UNREADABLE_INTERVAL_QUEUE_CAP {
            self.telemetry.untrusted_clock_recoveries_dropped = self
                .telemetry
                .untrusted_clock_recoveries_dropped
                .saturating_add(1);
            return;
        }
        self.untrusted_clock_recoveries
            .push_back(UntrustedClockRecovery {
                last_observed_hour_raw: from.hour_raw,
                resumed_hour_raw: resumed_hour,
                unreadable_events,
                observed_delta_hours,
                trust_ceiling_hours: MAX_FORWARD_DELTA,
            });
    }

    pub fn drain_untrusted_clock_recoveries(
        &mut self,
    ) -> impl Iterator<Item = UntrustedClockRecovery> + '_ {
        self.untrusted_clock_recoveries.drain(..)
    }

    pub fn has_untrusted_clock_recoveries(&self) -> bool {
        !self.untrusted_clock_recoveries.is_empty()
    }

    pub fn drain_clock_unreadable_intervals(
        &mut self,
    ) -> impl Iterator<Item = ClockUnreadableInterval> + '_ {
        self.clock_unreadable_intervals.drain(..)
    }

    pub fn has_clock_unreadable_intervals(&self) -> bool {
        !self.clock_unreadable_intervals.is_empty()
    }

    pub fn take_current_request(&mut self) -> Option<CaptureRequest> {
        if self
            .domain_requests
            .iter()
            .any(|slot| slot.in_flight.is_some())
        {
            return None;
        }
        let request = self
            .domain_requests
            .first()
            .and_then(|slot| slot.latest_pending)?;
        if self
            .domain_requests
            .iter()
            .any(|slot| slot.latest_pending != Some(request))
        {
            return None;
        }
        for slot in &mut self.domain_requests {
            slot.latest_pending = None;
            slot.in_flight = Some(request);
        }
        self.any_request_taken = true;
        self.shadow
            .note_dispatch((request.stamp.epoch, request.stamp.hour_raw));
        Some(request)
    }

    pub fn decide(&mut self) -> SchedulerDecision {
        let first_in_flight = self.domain_requests.first().and_then(|slot| slot.in_flight);
        if self
            .domain_requests
            .iter()
            .any(|slot| slot.in_flight != first_in_flight)
        {
            return SchedulerDecision::DomainFrontiersDiverged;
        }

        let first_pending = self
            .domain_requests
            .first()
            .and_then(|slot| slot.latest_pending);
        if self
            .domain_requests
            .iter()
            .any(|slot| slot.latest_pending != first_pending)
        {
            return SchedulerDecision::DomainFrontiersDiverged;
        }

        match (first_in_flight, first_pending) {
            (None, Some(_)) => self
                .take_current_request()
                .map(SchedulerDecision::Capture)
                .unwrap_or(SchedulerDecision::DomainFrontiersDiverged),
            (None, None) => SchedulerDecision::Idle,
            (Some(in_flight), None) => SchedulerDecision::AwaitingNextHour(in_flight),
            (Some(in_flight), Some(_)) => SchedulerDecision::AdvancePending(in_flight),
        }
    }

    pub fn take_domain_request(&mut self, domain_id: u16) -> Option<CaptureRequest> {
        let index = self.domains.binary_search(&domain_id).ok()?;
        let slot = &mut self.domain_requests[index];
        if slot.in_flight.is_some() {
            return None;
        }
        let request = slot.latest_pending.take()?;
        slot.in_flight = Some(request);
        self.any_request_taken = true;
        Some(request)
    }

    /// hour released here stays accounted as `outstanding` until its
    /// release without an intervening accept/reject would mean two unresolved
    /// real release from a no-op without inspecting private state.
    pub fn release_read_custody(&mut self, request: CaptureRequest) -> usize {
        if self
            .domain_requests
            .iter()
            .any(|slot| slot.publishing.is_some())
        {
            self.telemetry.terminal = Some(TerminalOutcome::CounterAnomaly);
            return 0;
        }
        let mut moved = 0usize;
        for slot in &mut self.domain_requests {
            if slot.in_flight == Some(request) {
                slot.in_flight = None;
                slot.publishing = Some(request);
                moved += 1;
            }
        }
        if moved == 0 {
            self.telemetry.terminal = Some(TerminalOutcome::CounterAnomaly);
            return 0;
        }
        self.telemetry.max_publishing_depth = self.telemetry.max_publishing_depth.max(1);
        self.shadow
            .note_read_custody_released((request.stamp.epoch, request.stamp.hour_raw));
        moved
    }

    pub fn accept_degraded_request(&mut self, request: CaptureRequest) {
        let before = self.telemetry.accepted_hours;
        self.accept_request(request);
        if self.telemetry.accepted_hours != before {
            self.telemetry.degraded_hours = self.telemetry.degraded_hours.saturating_add(1);
        }
    }

    pub fn accept_request(&mut self, request: CaptureRequest) {
        let mut matched = false;
        for slot in &mut self.domain_requests {
            // Publication custody first: a pipelined caller released read
            if slot.publishing == Some(request) {
                slot.publishing = None;
                matched = true;
            } else if slot.in_flight == Some(request) {
                slot.in_flight = None;
                matched = true;
            }
        }
        if !matched {
            self.telemetry.terminal = Some(TerminalOutcome::CounterAnomaly);
            return;
        }
        self.telemetry.accepted_hours = self.telemetry.accepted_hours.saturating_add(1);
        if self.last == Some(request.stamp) {
            self.force_baseline = false;
        }
        self.shadow
            .note_publication_resolved((request.stamp.epoch, request.stamp.hour_raw), true);
    }

    /// Returns the number of domain slots released, so a caller can distinguish
    /// a real release from a no-op without inspecting private state.
    pub fn reject_request(&mut self, request: CaptureRequest) -> usize {
        let rejected: Vec<u16> = self
            .domain_requests
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.publishing == Some(request) || slot.in_flight == Some(request))
            .map(|(index, _)| self.domains[index])
            .collect();
        if rejected.is_empty() {
            self.telemetry.terminal = Some(TerminalOutcome::CounterAnomaly);
            return 0;
        }
        for slot in &mut self.domain_requests {
            if slot.publishing == Some(request) {
                slot.publishing = None;
            } else if slot.in_flight == Some(request) {
                slot.in_flight = None;
            }
        }
        self.telemetry.refused_hours = self.telemetry.refused_hours.saturating_add(1);
        self.force_baseline = true;
        self.shadow
            .note_publication_resolved((request.stamp.epoch, request.stamp.hour_raw), false);
        rejected.len()
    }

    pub fn reject_domain(&mut self, domain_id: u16, request: CaptureRequest, kind: GapKind) {
        if let Ok(index) = self.domains.binary_search(&domain_id) {
            if self.domain_requests[index].in_flight == Some(request) {
                self.domain_requests[index].in_flight = None;
                self.push_gap(domain_id, request.stamp, kind);
                self.force_baseline = true;
                return;
            }
        }
        self.telemetry.terminal = Some(TerminalOutcome::CounterAnomaly);
    }

    #[cfg(test)]
    pub fn drain_control(&mut self) -> impl Iterator<Item = DomainGapRange> {
        let mut drained: Vec<DomainGapRange> = self.control.drain(..).collect();
        drained.sort_by_key(|gap| {
            (
                gap.epoch,
                gap.first_hour_raw,
                gap.last_hour_raw,
                gap.kind as u8,
                gap.domain_id,
            )
        });
        drained.into_iter()
    }

    pub fn reserve_control(&mut self) -> Result<Vec<DomainGapRange>, &'static str> {
        if self.reserved_control.is_some() {
            return Err("scheduler control-gap reservation already active");
        }
        let mut reserved = self.control.drain(..).collect::<Vec<_>>();
        reserved.sort_by_key(|gap| {
            (
                gap.epoch,
                gap.first_hour_raw,
                gap.last_hour_raw,
                gap.kind as u8,
                gap.domain_id,
            )
        });
        self.reserved_control = Some(reserved.clone());
        Ok(reserved)
    }

    pub fn commit_empty_reserved_control(&mut self) -> Result<(), &'static str> {
        match self.reserved_control.as_ref() {
            Some(reserved) if reserved.is_empty() => {
                self.reserved_control = None;
                Ok(())
            }
            Some(_) => Err("nonempty scheduler control-gap custody must commit a durable prefix"),
            None => Err("scheduler control-gap reservation is not active"),
        }
    }

    pub fn commit_reserved_control_prefix(&mut self, count: usize) -> Result<(), &'static str> {
        let reserved = self
            .reserved_control
            .as_mut()
            .ok_or("scheduler control-gap reservation is not active")?;
        let leading_group_len = reserved.first().map_or(0, |first| {
            reserved
                .iter()
                .take_while(|gap| {
                    gap.epoch == first.epoch
                        && gap.first_hour_raw == first.first_hour_raw
                        && gap.last_hour_raw == first.last_hour_raw
                        && gap.kind == first.kind
                })
                .count()
        });
        if count == 0 || count != leading_group_len {
            return Err("scheduler control-gap committed prefix is invalid");
        }
        reserved.drain(..count);
        if reserved.is_empty() {
            self.reserved_control = None;
        }
        Ok(())
    }

    pub fn rollback_reserved_control(&mut self) -> Result<(), &'static str> {
        let reserved = self
            .reserved_control
            .take()
            .ok_or("scheduler control-gap reservation is not active")?;
        if self.control.len().saturating_add(reserved.len()) > self.control_cap {
            self.telemetry.terminal = Some(TerminalOutcome::CounterAnomaly);
            return Err("scheduler control-gap rollback exceeds capacity");
        }
        self.control.extend(reserved);
        Ok(())
    }

    pub fn flush(&mut self) {
        self.gap_all_pending(GapKind::Backpressure);
    }

    pub fn flush_latest_pending(&mut self) {
        for index in 0..self.domains.len() {
            let domain = self.domains[index];
            if let Some(request) = self.domain_requests[index].latest_pending.take() {
                if index == 0 {
                    self.telemetry.flushed_pending_hours =
                        self.telemetry.flushed_pending_hours.saturating_add(1);
                }
                self.push_gap(domain, request.stamp, GapKind::Backpressure);
            }
        }
        self.shadow.note_invalidated();
    }

    fn count_unobserved_span(&mut self, from: Option<ClockStamp>, to: i32, cause: UnobservedCause) {
        let Some(from) = from else {
            return;
        };
        let (Some(first_missing), Some(last_missing)) =
            (from.hour_raw.checked_add(1), to.checked_sub(1))
        else {
            return;
        };
        if first_missing > last_missing {
            return;
        }
        let span = i64::from(last_missing) - i64::from(first_missing) + 1;
        let span = u64::try_from(span).unwrap_or(0);
        match cause {
            UnobservedCause::ClockUnknown => {
                self.telemetry.unobserved_clock_unknown_hours = self
                    .telemetry
                    .unobserved_clock_unknown_hours
                    .saturating_add(span)
            }
        }
    }

    fn enqueue_request(&mut self, request: CaptureRequest) {
        self.telemetry.scheduled_hours = self.telemetry.scheduled_hours.saturating_add(1);
        let mut evicted: Option<(u64, i32)> = None;
        for index in 0..self.domains.len() {
            let domain = self.domains[index];
            if let Some(replaced) = self.domain_requests[index].latest_pending.replace(request) {
                if index == 0 {
                    self.telemetry.evicted_pending_hours =
                        self.telemetry.evicted_pending_hours.saturating_add(1);
                    if !self.any_request_taken {
                        self.telemetry.evicted_before_first_take_hours =
                            self.telemetry.evicted_before_first_take_hours.saturating_add(1);
                    }
                    evicted = Some((replaced.stamp.epoch, replaced.stamp.hour_raw));
                }
                self.push_gap(domain, replaced.stamp, GapKind::Backpressure);
            }
            self.telemetry.max_request_depth = self.telemetry.max_request_depth.max(u32::from(
                self.domain_requests[index].latest_pending.is_some(),
            ));
        }
        self.shadow
            .note_frontier_admitted((request.stamp.epoch, request.stamp.hour_raw), evicted);
    }

    fn gap_all_pending(&mut self, kind: GapKind) {
        for index in 0..self.domains.len() {
            let domain = self.domains[index];
            if let Some(request) = self.domain_requests[index].in_flight.take() {
                if index == 0 {
                    self.telemetry.abandoned_hours =
                        self.telemetry.abandoned_hours.saturating_add(1);
                }
                self.push_gap(domain, request.stamp, kind);
            }
            if let Some(request) = self.domain_requests[index].latest_pending.take() {
                if index == 0 {
                    self.telemetry.abandoned_hours =
                        self.telemetry.abandoned_hours.saturating_add(1);
                }
                self.push_gap(domain, request.stamp, kind);
            }
        }
        self.shadow.note_invalidated();
    }

    fn push_all_domain_gap_range(
        &mut self,
        epoch: u64,
        first_hour_raw: i32,
        last_hour_raw: i32,
        kind: GapKind,
        tick_before: u64,
        tick_after: u64,
    ) {
        let domains = self.domains.clone();
        for domain in domains {
            self.push_gap_range(
                domain,
                epoch,
                first_hour_raw,
                last_hour_raw,
                kind,
                tick_before,
                tick_after,
            );
        }
        self.force_baseline = true;
    }

    fn push_gap(&mut self, domain_id: u16, stamp: ClockStamp, kind: GapKind) {
        self.push_gap_range(
            domain_id,
            stamp.epoch,
            stamp.hour_raw,
            stamp.hour_raw,
            kind,
            gap_tick_before(stamp, kind),
            gap_tick_after(stamp, kind),
        );
    }

    fn push_gap_range(
        &mut self,
        domain_id: u16,
        epoch: u64,
        first_hour_raw: i32,
        last_hour_raw: i32,
        kind: GapKind,
        tick_before: u64,
        tick_after: u64,
    ) {
        if self.domains.binary_search(&domain_id).is_err() {
            return;
        }
        let represented = i64::from(last_hour_raw)
            .checked_sub(i64::from(first_hour_raw))
            .and_then(|delta| delta.checked_add(1))
            .and_then(|count| u64::try_from(count).ok());
        let Some(represented) = represented else {
            self.telemetry.terminal = Some(TerminalOutcome::CounterAnomaly);
            return;
        };
        self.telemetry.gapped_domain_hours = self
            .telemetry
            .gapped_domain_hours
            .saturating_add(represented);
        match kind {
            GapKind::Torn => {
                self.telemetry.torn_domain_hours =
                    self.telemetry.torn_domain_hours.saturating_add(represented)
            }
            GapKind::Overrun => {
                self.telemetry.overrun_domain_hours = self
                    .telemetry
                    .overrun_domain_hours
                    .saturating_add(represented)
            }
            GapKind::Backpressure => {
                self.telemetry.backpressure_domain_hours = self
                    .telemetry
                    .backpressure_domain_hours
                    .saturating_add(represented)
            }
            GapKind::WorkerPanic => {
                self.telemetry.worker_panic_domain_hours = self
                    .telemetry
                    .worker_panic_domain_hours
                    .saturating_add(represented)
            }
            GapKind::ClockUnreadable | GapKind::ReloadBaseline => {}
        }
        if let Some(last) = self
            .control
            .iter_mut()
            .rev()
            .find(|gap| gap.domain_id == domain_id)
        {
            let first_stamp = ClockStamp {
                epoch,
                hour_raw: first_hour_raw,
                observation_ordinal: tick_after,
            };
            if last.can_extend(domain_id, first_stamp, kind) {
                last.last_hour_raw = last_hour_raw;
                last.force_next_baseline = true;
                match kind {
                    GapKind::Overrun => last.tick_after = tick_after,
                    GapKind::Backpressure => {
                        last.tick_before = tick_after;
                        last.tick_after = tick_after;
                        last.queue_dropped = last.queue_dropped.saturating_add(represented);
                    }
                    _ => unreachable!("only range-capable gap kinds can extend"),
                }
                return;
            }
        }
        let reserved_control_len = self.reserved_control.as_ref().map_or(0, Vec::len);
        if self.control.len().saturating_add(reserved_control_len) == self.control_cap {
            self.telemetry.terminal = Some(TerminalOutcome::ActivationFailed);
            return;
        }
        self.control.push_back(DomainGapRange {
            domain_id,
            epoch,
            first_hour_raw,
            last_hour_raw,
            kind,
            force_next_baseline: true,
            tick_before,
            tick_after,
            queue_capacity: if kind == GapKind::Backpressure { 1 } else { 0 },
            queue_depth: if kind == GapKind::Backpressure { 1 } else { 0 },
            queue_dropped: if kind == GapKind::Backpressure {
                represented
            } else {
                0
            },
        });
        self.telemetry.max_control_depth = self
            .telemetry
            .max_control_depth
            .max(self.control.len().saturating_add(reserved_control_len) as u32);
    }
}

fn clock_witness(hour_raw: i32) -> u64 {
    u64::try_from(hour_raw).unwrap_or(0)
}

fn gap_tick_before(stamp: ClockStamp, kind: GapKind) -> u64 {
    match kind {
        GapKind::ClockUnreadable => 0,
        GapKind::Torn | GapKind::Overrun => clock_witness(stamp.hour_raw.saturating_sub(1)),
        GapKind::Backpressure | GapKind::WorkerPanic | GapKind::ReloadBaseline => {
            clock_witness(stamp.hour_raw)
        }
    }
}

fn gap_tick_after(stamp: ClockStamp, kind: GapKind) -> u64 {
    match kind {
        GapKind::ClockUnreadable => 0,
        GapKind::Torn | GapKind::Overrun => clock_witness(stamp.hour_raw),
        GapKind::Backpressure | GapKind::WorkerPanic | GapKind::ReloadBaseline => {
            clock_witness(stamp.hour_raw)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DomainOutcome<T> {
    Accepted { value: T, closing: ClockStamp },
    Gap { kind: GapKind },
}

pub fn run_domain_transaction<T, C, Read, Close>(
    opening: ClockStamp,
    cursor: &mut C,
    reader: Read,
    closing_clock: Close,
) -> DomainOutcome<T>
where
    C: Clone,
    Read: FnOnce(&mut C) -> T,
    Close: FnOnce() -> Option<ClockStamp>,
{
    let mut staged = cursor.clone();
    let value = match catch_unwind(AssertUnwindSafe(|| reader(&mut staged))) {
        Ok(value) => value,
        Err(_) => {
            return DomainOutcome::Gap {
                kind: GapKind::WorkerPanic,
            }
        }
    };
    let Some(closing) = closing_clock() else {
        return DomainOutcome::Gap {
            kind: GapKind::ClockUnreadable,
        };
    };
    if closing.epoch != opening.epoch || closing.hour_raw != opening.hour_raw {
        return DomainOutcome::Gap {
            kind: GapKind::Torn,
        };
    }
    *cursor = staged;
    DomainOutcome::Accepted { value, closing }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scheduler(domains: &[u16], request_cap: usize, control_cap: usize) -> ReadOnlyScheduler {
        ReadOnlyScheduler::new(domains.to_vec(), request_cap, control_cap).unwrap()
    }

    fn scheduler_with_shadow(
        domains: &[u16],
        request_cap: usize,
        control_cap: usize,
        mode: ShadowMode,
    ) -> ReadOnlyScheduler {
        ReadOnlyScheduler::with_custody_shadow(
            domains.to_vec(),
            request_cap,
            control_cap,
            CustodyShadow::new(mode, ShadowClock::Manual { now_us: 0 }),
        )
        .unwrap()
    }

    /// `reject_request` existed there was no way to release it, and
    #[test]
    fn a_refused_frame_releases_its_request_instead_of_wedging_the_scheduler() {
        let mut s = scheduler(&[10, 11, 12], 1, 64);

        s.observe_stable(100, 1);
        let first = s.take_current_request().expect("hour 100 must be takeable");

        s.observe_stable(101, 2);

        // THE WEDGE: without a release, no later hour can ever be taken.
        assert_eq!(
            s.take_current_request(),
            None,
            "an unaccepted in-flight request must block later takes -- if this \
             ever passes, the wedge premise is wrong and this test is worthless"
        );

        // Release it the way a refusal now does.
        let released = s.reject_request(first);
        assert_eq!(
            released, 3,
            "every domain slot holding the request is released"
        );

        let second = s
            .take_current_request()
            .expect("after a reject the scheduler must service the next hour");
        assert_ne!(second, first, "the next hour is a distinct request");

        let telemetry = s.telemetry();
        assert_eq!(telemetry.accepted_hours, 0, "nothing was published");
        assert_eq!(
            telemetry.gapped_domain_hours, 0,
            "a refusal is not a throughput gap"
        );

        assert_eq!(s.reject_request(first), 0);
    }

    #[test]
    fn zero_mutation_surface_has_no_address_or_write_input() {
        let mut s = scheduler(&[10], 2, 4);
        s.observe_stable(100, 1);
        assert_eq!(
            s.take_current_request(),
            Some(CaptureRequest {
                stamp: ClockStamp {
                    epoch: 0,
                    hour_raw: 100,
                    observation_ordinal: 1,
                },
                force_baseline: true,
            })
        );
    }

    #[test]
    fn torn_transaction_rolls_back_tracker() {
        let opening = ClockStamp {
            epoch: 3,
            hour_raw: 50,
            observation_ordinal: 10,
        };
        let mut cursor = vec![1u64, 2];
        let outcome = run_domain_transaction(
            opening,
            &mut cursor,
            |staged| {
                staged.push(3);
                99
            },
            || {
                Some(ClockStamp {
                    epoch: 3,
                    hour_raw: 51,
                    observation_ordinal: 0,
                })
            },
        );
        assert_eq!(
            outcome,
            DomainOutcome::Gap {
                kind: GapKind::Torn
            }
        );
        assert_eq!(cursor, vec![1, 2]);
    }

    #[test]
    fn hook_cursor_advances_only_on_accepted_sample() {
        let stamp = ClockStamp {
            epoch: 0,
            hour_raw: 10,
            observation_ordinal: 4,
        };
        let mut cursor = 7u64;
        let accepted = run_domain_transaction(
            stamp,
            &mut cursor,
            |staged| {
                *staged += 1;
                *staged
            },
            || Some(stamp),
        );
        assert_eq!(
            accepted,
            DomainOutcome::Accepted {
                value: 8,
                closing: stamp
            }
        );
        assert_eq!(cursor, 8);
    }

    #[test]
    fn overrun_never_relabels_current_snapshot_as_old_hour() {
        let mut s = scheduler(&[10, 11], 8, 32);
        s.observe_stable(20, 1);
        let first = s.take_current_request().unwrap();
        s.accept_request(first);
        s.observe_stable(24, 2);
        let current = s.take_current_request().unwrap();
        assert_eq!(current.stamp.hour_raw, 24);
        let gaps: Vec<_> = s.drain_control().collect();
        assert!(gaps.iter().all(|gap| gap.last_hour_raw < 24));
        assert!(gaps.iter().all(|gap| {
            gap.tick_after.checked_sub(gap.tick_before)
                == Some((i64::from(gap.last_hour_raw) - i64::from(gap.first_hour_raw) + 1) as u64)
        }));
        assert_eq!(
            gaps.iter()
                .map(|gap| i64::from(gap.last_hour_raw) - i64::from(gap.first_hour_raw) + 1)
                .sum::<i64>(),
            6
        );
    }

    /// Lane PF: hours the clock schedules before the capture loop takes its
    /// first request are a start boundary, named apart from a mid-run
    /// eviction. P4 lost 3 of its 4 unmeasured hours this way.
    #[test]
    fn evictions_before_the_first_take_are_a_start_boundary() {
        let mut s = scheduler(&[10], 1, 64);
        s.observe_stable(1, 1);
        s.observe_stable(2, 2);
        s.observe_stable(3, 3);
        let before = s.telemetry();
        assert_eq!(before.evicted_pending_hours, 2, "hours 1 and 2");
        assert_eq!(before.evicted_before_first_take_hours, 2);

        let taken = s.take_current_request().expect("hour 3 is takeable");
        assert_eq!(taken.stamp.hour_raw, 3);
        s.observe_stable(4, 4);
        s.observe_stable(5, 5);
        let after = s.telemetry();
        assert_eq!(after.evicted_pending_hours, 3, "hour 4 was a mid-run eviction");
        assert_eq!(
            after.evicted_before_first_take_hours, 2,
            "an eviction after the first take is the loop failing to keep pace, not a boundary"
        );
    }

    #[test]
    fn full_request_ring_reports_backpressure_without_invoking_reader() {
        let mut s = scheduler(&[10], 1, 8);
        s.observe_stable(1, 1);
        s.observe_stable(2, 2);
        let mut invoked = false;
        let gaps: Vec<_> = s.drain_control().collect();
        assert!(!invoked);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].kind, GapKind::Backpressure);
        invoked = s.take_current_request().is_some();
        assert!(invoked);
    }

    #[test]
    fn backpressure_counters_reconcile_with_the_ranges_the_wire_carries() {
        const DOMAINS: u64 = 3;
        let mut s = scheduler(&[10, 11, 12], 1, 64);

        s.observe_stable(1, 1);
        let in_flight = s.take_current_request().expect("hour 1 is takeable");
        for hour in 2..=12 {
            s.observe_stable(hour, hour as u64);
        }
        s.accept_request(in_flight);

        let probe = s.telemetry();
        let probe_ledger = s.hour_ledger();
        assert!(probe_ledger.balanced(), "{probe_ledger:?}");
        assert_eq!(probe_ledger.scheduled, 12);
        assert_eq!(probe_ledger.accepted, 1);
        assert_eq!(
            probe_ledger.evicted_pending, 10,
            "hours 2..=11 were each replaced before a reader took them"
        );
        assert_eq!(
            probe_ledger.outstanding, 1,
            "hour 12 is pending and has no disposition yet"
        );
        assert_eq!(
            probe.evicted_pending_hours * DOMAINS,
            probe.backpressure_domain_hours,
            "with only the eviction site active, the domain-hour counter is \
             exactly the hour counter times the roster"
        );
        assert_eq!(probe.backpressure_domain_hours, 30);
        assert_eq!(probe.flushed_pending_hours, 0);
        assert_eq!(probe.abandoned_hours, 0);

        s.flush_latest_pending();
        let after = s.telemetry();
        let after_ledger = s.hour_ledger();
        assert!(after_ledger.balanced(), "{after_ledger:?}");
        assert_eq!(after_ledger.outstanding, 0);
        assert_eq!(after_ledger.flushed_pending, 1, "hour 12");
        assert_eq!(
            after.evicted_pending_hours, probe.evicted_pending_hours,
            "the teardown flush is NOT an eviction and must not be counted as one"
        );

        let hours_resolved_after_probe =
            after_ledger.flushed_pending - probe_ledger.flushed_pending;
        assert_eq!(hours_resolved_after_probe, 1);
        assert_eq!(
            after.backpressure_domain_hours,
            probe.backpressure_domain_hours + hours_resolved_after_probe * DOMAINS,
            "a probe can only report the losses that had happened when it was \
             written; the wire keeps accruing after it"
        );

        let gaps: Vec<_> = s.drain_control().collect();
        assert!(
            gaps.iter().all(|gap| gap.kind == GapKind::Backpressure),
            "every range raised here is Backpressure"
        );
        assert_eq!(
            gaps.len() as u64,
            DOMAINS,
            "the eviction run and the teardown hour COALESCE into one range per \
             domain, because hour 12 is adjacent to the 2..=11 run"
        );
        for gap in &gaps {
            assert_eq!((gap.first_hour_raw, gap.last_hour_raw), (2, 12));
            assert_eq!(
                gap.queue_dropped, 11,
                "the wire's own witness of how many hours this range stands for"
            );
        }

        let records = gaps.len() as u64;
        let covered_domain_hours: u64 = gaps
            .iter()
            .map(|gap| (gap.last_hour_raw - gap.first_hour_raw + 1) as u64)
            .sum();

        assert_eq!(records, 3, "3 ranges x 1 record per domain per range");
        assert_eq!(
            covered_domain_hours, after.backpressure_domain_hours,
            "summing each record's OWN hour span reproduces the domain-hour \
             counter exactly; this is the comparison a reconciliation must make"
        );
        assert_ne!(
            records, covered_domain_hours,
            "records and domain-hours are different units whenever any range \
             coalesces, which is precisely why t32's 5_508 - 4_964 = 544 was \
             not a discrepancy"
        );
        assert_eq!(
            covered_domain_hours - records,
            33 - 3,
            "the gap between the two counts is the coalescing, and nothing else"
        );
    }

    #[test]
    fn sustained_overload_coalesces_one_exact_range_per_domain() {
        let mut s = scheduler(&[10, 11, 12], 1, 3);
        for hour in 1..=10_000 {
            s.observe_stable(hour, hour as u64);
        }
        assert_eq!(s.terminal(), None);
        let gaps: Vec<_> = s.drain_control().collect();
        assert_eq!(gaps.len(), 3);
        for gap in gaps {
            assert_eq!(gap.kind, GapKind::Backpressure);
            assert_eq!(gap.first_hour_raw, 1);
            assert_eq!(gap.last_hour_raw, 9_999);
            assert_eq!(gap.queue_dropped, 9_999);
            assert_eq!(gap.tick_before, gap.tick_after);
        }
        assert_eq!(s.take_current_request().unwrap().stamp.hour_raw, 10_000);
    }

    #[test]
    fn control_lane_is_bounded_and_exhaustion_fails_without_fabricating_ranges() {
        let mut s = scheduler(&[10, 11], 1, 2);
        s.observe_stable(1, 1);
        s.observe_stable(3, 2);
        assert_eq!(s.telemetry().max_control_depth, 2);
        assert!(s.telemetry().max_control_depth <= s.telemetry().control_cap);
        assert_eq!(s.terminal(), Some(TerminalOutcome::ActivationFailed));
        assert_eq!(s.telemetry().control_lane_compactions, 0);
        assert!(!s.drain_control().collect::<Vec<_>>().is_empty());
    }

    #[test]
    fn worker_panic_is_terminal_domain_outcome_and_rolls_back() {
        let stamp = ClockStamp {
            epoch: 0,
            hour_raw: 5,
            observation_ordinal: 1,
        };
        let mut cursor = 9u64;
        let outcome = run_domain_transaction(
            stamp,
            &mut cursor,
            |staged| {
                *staged = 10;
                panic!("synthetic worker panic");
            },
            || Some(stamp),
        );
        assert_eq!(
            outcome,
            DomainOutcome::<()>::Gap {
                kind: GapKind::WorkerPanic
            }
        );
        assert_eq!(cursor, 9);
    }

    #[test]
    fn reload_closes_pending_epoch_and_forces_baseline() {
        let mut s = scheduler(&[10], 4, 8);
        s.observe_stable(500, 1);
        let request = s.take_current_request().unwrap();
        s.accept_request(request);
        s.observe_stable(501, 2);
        s.observe_stable(200, 3);
        let request = s.take_current_request().unwrap();
        assert_eq!(request.stamp.epoch, 1);
        assert_eq!(request.stamp.hour_raw, 200);
        assert!(request.force_baseline);
        assert_eq!(s.telemetry().reloads, 1);
    }

    #[test]
    fn every_domain_has_exactly_one_outcome_for_each_skipped_hour() {
        let mut s = scheduler(&[10, 11, 12], 8, 64);
        s.observe_stable(7, 1);
        s.take_current_request();
        s.observe_stable(10, 2);
        let gaps: Vec<_> = s.drain_control().collect();
        let represented: i64 = gaps
            .iter()
            .map(|gap| i64::from(gap.last_hour_raw) - i64::from(gap.first_hour_raw) + 1)
            .sum();
        assert_eq!(represented, 2 * 3);
    }

    #[test]
    fn control_gap_reservation_commits_or_rolls_back_exactly_once() {
        let mut s = scheduler(&[10], 1, 8);
        s.observe_stable(1, 1);
        s.observe_stable(2, 2);
        let reserved = s.reserve_control().unwrap();
        assert_eq!(reserved.len(), 1);
        assert!(s.drain_control().collect::<Vec<_>>().is_empty());
        s.rollback_reserved_control().unwrap();
        assert_eq!(s.reserve_control().unwrap(), reserved);
        s.commit_reserved_control_prefix(1).unwrap();
        assert!(s.drain_control().collect::<Vec<_>>().is_empty());
        assert!(s.commit_empty_reserved_control().is_err());

        let mut empty = scheduler(&[10], 1, 8);
        assert!(empty.reserve_control().unwrap().is_empty());
        empty.commit_empty_reserved_control().unwrap();
    }

    #[test]
    fn durable_control_gap_prefix_is_never_rolled_back_with_unpublished_suffix() {
        let mut s = scheduler(&[10, 11], 1, 8);
        s.observe_stable(1, 1);
        let first = s.take_current_request().unwrap();
        s.reject_domain(10, first, GapKind::Torn);
        s.reject_domain(11, first, GapKind::Torn);
        s.observe_stable(2, 2);
        let second = s.take_current_request().unwrap();
        s.reject_domain(10, second, GapKind::WorkerPanic);
        s.reject_domain(11, second, GapKind::WorkerPanic);
        let reserved = s.reserve_control().unwrap();
        assert_eq!(reserved.len(), 4);
        assert!(s.commit_reserved_control_prefix(1).is_err());
        s.commit_reserved_control_prefix(2).unwrap();
        s.rollback_reserved_control().unwrap();
        let remaining = s.reserve_control().unwrap();
        assert_eq!(remaining, reserved[2..]);
        s.commit_reserved_control_prefix(2).unwrap();
        assert!(s.commit_empty_reserved_control().is_err());
        assert!(s.rollback_reserved_control().is_err());
    }

    #[test]
    fn hours_elapsed_while_the_clock_was_unreadable_are_counted_but_never_published() {
        let mut s = scheduler(&[10, 11], 1, 64);
        s.observe_stable(100, 1);
        let baseline = s.take_current_request().expect("baseline hour is takeable");
        s.accept_request(baseline);
        let _ = s.drain_control().count();

        s.note_clock_unreadable();
        s.note_clock_unreadable();
        s.observe_stable(105, 2);

        let ledger = s.hour_ledger();
        assert_eq!(
            ledger.unobserved_clock_unknown, 4,
            "the interval is counted positively, not derived by subtraction"
        );
        assert!(ledger.balanced(), "{ledger:?}");

        let gaps: Vec<_> = s.drain_control().collect();
        assert!(
            gaps.is_empty(),
            "an unobserved interval must not become a control range: every range \
             is closed by FRAME_END(range_end), which would publish a frame for \
             an hour nobody looked at -- {gaps:?}"
        );
        assert!(s.take_current_request().is_some());
    }

    #[test]
    fn a_clock_outage_is_published_as_an_interval_that_closes_no_hour() {
        let mut s = scheduler(&[10, 11], 1, 64);
        s.observe_stable(100, 1);
        let baseline = s.take_current_request().expect("baseline hour is takeable");
        s.accept_request(baseline);
        let _ = s.drain_control().count();

        s.note_clock_unreadable();
        s.note_clock_unreadable();
        s.note_clock_unreadable();
        s.observe_stable(105, 2);

        assert!(s.has_clock_unreadable_intervals());
        let intervals: Vec<_> = s.drain_clock_unreadable_intervals().collect();
        assert_eq!(
            intervals.len(),
            1,
            "an outage must reach the wire as its own interval record: {intervals:?}"
        );
        let interval = intervals[0];
        assert_eq!(interval.range_start_hour_raw, 101);
        assert_eq!(interval.range_end_hour_raw, 104);
        assert_eq!(interval.last_observed_hour_raw, 100);
        assert_eq!(interval.resumed_hour_raw, 105);
        assert_eq!(
            interval.unreadable_events, 3,
            "the record must substantiate its own claim with the failed reads it saw"
        );
        assert!(!s.has_clock_unreadable_intervals());
        assert!(s.drain_clock_unreadable_intervals().next().is_none());

        let gaps: Vec<_> = s.drain_control().collect();
        assert!(gaps.is_empty(), "{gaps:?}");
    }

    #[test]
    fn an_outage_that_loses_no_hour_mints_no_interval() {
        let mut s = scheduler(&[10], 1, 64);
        s.observe_stable(100, 1);
        let baseline = s.take_current_request().expect("baseline hour is takeable");
        s.accept_request(baseline);

        s.note_clock_unreadable();
        s.observe_stable(101, 2);
        let intervals: Vec<_> = s.drain_clock_unreadable_intervals().collect();
        assert!(
            intervals.is_empty(),
            "an outage that lost no hours must mint no record: {intervals:?}"
        );
        assert_eq!(s.hour_ledger().unobserved_clock_unknown, 0);
    }

    #[test]
    fn an_outage_wider_than_the_trusted_forward_delta_mints_no_interval() {
        let mut s = scheduler(&[10], 1, 64);
        s.observe_stable(100, 1);
        let baseline = s.take_current_request().expect("baseline hour is takeable");
        s.accept_request(baseline);

        s.note_clock_unreadable();
        let far = 100 + MAX_FORWARD_DELTA as i32 + 1;
        s.observe_stable(far, 2);

        let intervals: Vec<_> = s.drain_clock_unreadable_intervals().collect();
        assert!(
            intervals.is_empty(),
            "an untrusted forward jump must not become accounted hours because one clock \
             read failed: {intervals:?}"
        );
        assert_eq!(
            s.hour_ledger().unobserved_clock_unknown,
            0,
            "and the span refused as a record is not counted as hours either"
        );
        assert_eq!(
            s.telemetry().clock_unreadable_intervals_dropped,
            0,
            "an untrusted recovery is not an interval the producer failed to publish"
        );

        let mut s = scheduler(&[10], 1, 64);
        s.observe_stable(100, 1);
        let baseline = s.take_current_request().expect("baseline hour is takeable");
        s.accept_request(baseline);
        s.note_clock_unreadable();
        s.observe_stable(100 + MAX_FORWARD_DELTA as i32, 2);
        assert_eq!(
            s.drain_clock_unreadable_intervals().count(),
            1,
            "an outage at the trusted ceiling still reaches the wire"
        );
        assert_eq!(
            s.telemetry().untrusted_clock_recovery_events,
            0,
            "and it is not ALSO disclaimed -- the two records are disjoint"
        );
    }

    #[test]
    fn an_untrusted_recovery_is_recorded_on_the_wire_and_is_not_terminal() {
        let mut s = scheduler(&[10], 1, 64);
        s.observe_stable(100, 1);
        let baseline = s.take_current_request().expect("baseline hour is takeable");
        s.accept_request(baseline);

        s.note_clock_unreadable();
        s.note_clock_unreadable();
        let delta = MAX_FORWARD_DELTA + 1;
        let far = 100 + delta as i32;
        s.observe_stable(far, 2);

        assert_eq!(
            s.terminal(),
            None,
            "one failed clock read must not destroy the capture"
        );
        let resumed = s
            .take_current_request()
            .expect("the resumed hour is still scheduled");
        assert_eq!(resumed.stamp.hour_raw, far);
        assert!(
            resumed.force_baseline,
            "and it re-baselines, as any epoch boundary does"
        );

        assert_eq!(
            s.hour_ledger().untrusted_clock_recovery_events,
            1,
            "the untrusted jump must be counted"
        );
        let recoveries: Vec<_> = s.drain_untrusted_clock_recoveries().collect();
        assert_eq!(
            recoveries.len(),
            1,
            "the untrusted jump must reach the wire"
        );
        let recovery = recoveries[0];
        assert_eq!(recovery.last_observed_hour_raw, 100);
        assert_eq!(recovery.resumed_hour_raw, far);
        assert_eq!(
            recovery.unreadable_events, 2,
            "and it substantiates itself with its own failed reads"
        );
        assert_eq!(recovery.trust_ceiling_hours, MAX_FORWARD_DELTA);

        assert_eq!(
            recovery.observed_delta_hours, delta,
            "the record must carry the ACTUAL jump size, not a flag"
        );
        assert_eq!(
            s.hour_ledger().max_untrusted_clock_recovery_delta_hours,
            delta as u64,
            "and the telemetry carries the size too, not just the fact"
        );
    }

    #[test]
    fn the_scheduler_ceiling_is_the_contract_ceiling() {
        assert_eq!(
            MAX_FORWARD_DELTA,
            compact_contract::untrusted_clock_recovery::
                UNTRUSTED_CLOCK_RECOVERY_TRUST_CEILING_HOURS,
            "the scheduler's ceiling and the record's pinned ceiling are one constant"
        );

        let base = compact_contract::calendar::DATE_EPOCH_CONST + 1936 * 365 * 24;
        let mut s = scheduler(&[10], 1, 64);
        s.observe_stable(base, 1);
        let baseline = s.take_current_request().expect("baseline is takeable");
        s.accept_request(baseline);
        s.note_clock_unreadable();
        s.observe_stable(base + MAX_FORWARD_DELTA as i32 + 1, 2);
        let recovery = s
            .drain_untrusted_clock_recoveries()
            .next()
            .expect("the untrusted recovery is queued");
        compact_contract::untrusted_clock_recovery::UntrustedClockRecoveryRecordV1 {
            last_observed_hour_raw: recovery.last_observed_hour_raw,
            resumed_hour_raw: recovery.resumed_hour_raw,
            unreadable_events: recovery.unreadable_events,
            observed_delta_hours: recovery.observed_delta_hours,
            trust_ceiling_hours: recovery.trust_ceiling_hours,
        }
        .encode()
        .expect("the scheduler's own recovery satisfies the record's pinned ceiling");
    }

    #[test]
    fn the_two_recovery_records_are_disjoint_and_neither_fires_on_a_lossless_outage() {
        for resumed in [100, 101] {
            let mut s = scheduler(&[10], 1, 64);
            s.observe_stable(100, 1);
            let baseline = s.take_current_request().expect("baseline is takeable");
            s.accept_request(baseline);
            s.note_clock_unreadable();
            s.observe_stable(resumed, 2);
            let intervals: Vec<_> = s.drain_clock_unreadable_intervals().collect();
            let recoveries: Vec<_> = s.drain_untrusted_clock_recoveries().collect();
            assert!(
                intervals.is_empty() && recoveries.is_empty(),
                "an outage that lost no hour must mint neither record: \
                 intervals={intervals:?} recoveries={recoveries:?}"
            );
        }

        let mut s = scheduler(&[10], 1, 64);
        s.observe_stable(100, 1);
        let baseline = s.take_current_request().expect("baseline is takeable");
        s.accept_request(baseline);
        s.note_clock_unreadable();
        s.observe_stable(105, 2);
        assert_eq!(s.drain_clock_unreadable_intervals().count(), 1);
        assert_eq!(s.drain_untrusted_clock_recoveries().count(), 0);

        let mut s = scheduler(&[10], 1, 64);
        s.observe_stable(100, 1);
        let baseline = s.take_current_request().expect("baseline is takeable");
        s.accept_request(baseline);
        s.note_clock_unreadable();
        s.observe_stable(100 + MAX_FORWARD_DELTA as i32 + 1, 2);
        assert_eq!(s.drain_clock_unreadable_intervals().count(), 0);
        assert_eq!(s.drain_untrusted_clock_recoveries().count(), 1);
    }

    #[test]
    fn many_undrained_outages_all_survive_to_be_published() {
        const OUTAGES: usize = 300;
        let mut s = scheduler(&[10], 1, 64);
        s.observe_stable(100, 1);
        let baseline = s.take_current_request().expect("baseline hour is takeable");
        s.accept_request(baseline);

        for step in 0..OUTAGES {
            s.note_clock_unreadable();
            let resumed = 100 + 2 * (step as i32 + 1);
            s.observe_stable(resumed, step as u64 + 2);
            let taken = s.take_current_request();
            if let Some(request) = taken {
                s.accept_request(request);
            }
        }

        assert_eq!(
            s.drain_clock_unreadable_intervals().count(),
            OUTAGES,
            "an ordinary run's outages must all reach the wire"
        );
        assert_eq!(
            s.telemetry().clock_unreadable_intervals_dropped,
            0,
            "and none of them may be dropped"
        );
    }

    #[test]
    fn each_outage_substantiates_itself_with_its_own_failed_reads() {
        let mut s = scheduler(&[10], 1, 64);
        s.observe_stable(100, 1);
        let baseline = s.take_current_request().expect("baseline hour is takeable");
        s.accept_request(baseline);

        s.note_clock_unreadable();
        s.note_clock_unreadable();
        s.observe_stable(105, 2);
        let first = s.take_current_request().expect("recovery hour is takeable");
        s.accept_request(first);

        s.note_clock_unreadable();
        s.observe_stable(110, 3);

        let intervals: Vec<_> = s.drain_clock_unreadable_intervals().collect();
        assert_eq!(intervals.len(), 2, "{intervals:?}");
        assert_eq!(intervals[0].unreadable_events, 2);
        assert_eq!(
            intervals[1].unreadable_events, 1,
            "the second outage must not inherit the first one's failed reads"
        );
        assert_eq!(intervals[0].range_start_hour_raw, 101);
        assert_eq!(intervals[0].range_end_hour_raw, 104);
        assert_eq!(intervals[1].range_start_hour_raw, 106);
        assert_eq!(intervals[1].range_end_hour_raw, 109);
    }

    #[test]
    fn a_rejected_forward_counter_jump_is_an_event_not_a_claimed_span() {
        let mut s = scheduler(&[10], 1, 64);
        s.observe_stable(100, 1);
        let baseline = s.take_current_request().expect("baseline hour is takeable");
        s.accept_request(baseline);
        let _ = s.drain_control().count();

        let far = 100 + MAX_FORWARD_DELTA as i32 + 10;
        s.observe_stable(far, 2);
        assert_eq!(
            s.terminal(),
            Some(TerminalOutcome::CounterAnomaly),
            "the jump is still rejected"
        );
        assert_eq!(s.hour_ledger().counter_anomaly_events, 1);
        assert_eq!(
            s.hour_ledger().unobserved_clock_unknown,
            0,
            "a rejected read must not be converted into elapsed hours"
        );
        let gaps: Vec<_> = s.drain_control().collect();
        assert!(
            gaps.is_empty(),
            "no range may be claimed from a rejected read: {gaps:?}"
        );

        s.observe_stable(far + 1, 3);
        assert_eq!(s.telemetry().post_terminal_observations, 1);
    }

    #[test]
    fn an_hour_evicted_before_any_reader_took_it_is_counted_where_it_happens() {
        let mut s = scheduler(&[10, 11], 1, 64);
        s.observe_stable(100, 1);
        let in_flight = s.take_current_request().expect("hour 100 is takeable");

        s.observe_stable(101, 2);
        s.observe_stable(102, 3);

        let ledger = s.hour_ledger();
        assert_eq!(ledger.scheduled, 3);
        assert_eq!(ledger.accepted, 0);
        assert_eq!(
            ledger.evicted_pending, 1,
            "hour 101 is never swept and nothing else in the ledger claims it"
        );
        assert_eq!(ledger.outstanding, 2, "100 in flight, 102 pending");
        assert!(ledger.balanced(), "{ledger:?}");
        assert_eq!(ledger.hours_with_no_data_frame(), 1);

        s.accept_request(in_flight);
        let ledger = s.hour_ledger();
        assert_eq!(ledger.accepted, 1);
        assert_eq!(ledger.outstanding, 1);
        assert!(ledger.balanced(), "{ledger:?}");
    }

    #[test]
    fn every_admitted_hour_has_exactly_one_counted_disposition() {
        let mut s = scheduler(&[10], 1, 64);

        s.observe_stable(1, 1);
        let r = s.take_current_request().unwrap();
        s.accept_request(r);
        s.observe_stable(2, 2);
        let r = s.take_current_request().unwrap();
        s.reject_request(r);
        s.observe_stable(3, 3);
        let r = s.take_current_request().unwrap();
        s.observe_stable(4, 4);
        s.observe_stable(5, 5);
        s.note_clock_unreadable();
        s.observe_stable(6, 6);
        s.flush_latest_pending();
        let _ = r;

        let ledger = s.hour_ledger();
        assert_eq!(ledger.scheduled, 6);
        assert_eq!(ledger.accepted, 1);
        assert_eq!(ledger.refused, 1);
        assert_eq!(ledger.evicted_pending, 1, "hour 4 was replaced by hour 5");
        assert_eq!(ledger.abandoned, 2, "hour 3 in flight and hour 5 pending");
        assert_eq!(ledger.flushed_pending, 1, "hour 6");
        assert_eq!(ledger.outstanding, 0);
        assert!(ledger.balanced(), "{ledger:?}");
        assert_eq!(
            ledger.hours_with_no_data_frame(),
            5,
            "hours 2 (refused), 3, 4, 5 and 6 put no domain data on the wire"
        );
    }

    /// So: hours 100 (read, released, publishing), then 101 and 102 arrive
    /// `release_read_custody` and the `publishing == 1` / `balanced`
    #[test]
    fn releasing_read_custody_claims_the_current_frontier_not_a_queued_hour() {
        let mut s = scheduler(&[10, 11, 12], 1, 64);
        s.observe_stable(100, 1);
        let first = s.take_current_request().expect("hour 100 is takeable");

        assert_eq!(s.release_read_custody(first), 3);
        let ledger = s.hour_ledger();
        assert_eq!(
            ledger.publishing, 1,
            "hour 100 is awaiting its sink outcome"
        );
        assert_eq!(ledger.outstanding, 1);
        assert_eq!(ledger.accepted, 0, "nothing has published yet");
        assert!(ledger.balanced(), "{ledger:?}");

        s.observe_stable(101, 2);
        s.observe_stable(102, 3);

        let successor = s
            .take_current_request()
            .expect("read custody was released, so the successor is claimable");
        assert_eq!(
            successor.stamp.hour_raw, 102,
            "the successor must be the CURRENT frontier; claiming 101 here would \
             label a read of the 102 world as hour 101"
        );

        let gaps: Vec<_> = s.drain_control().collect();
        assert_eq!(gaps.len(), 3, "one Backpressure range per domain: {gaps:?}");
        for gap in &gaps {
            assert_eq!(gap.kind, GapKind::Backpressure);
            assert_eq!(gap.first_hour_raw, 101);
            assert_eq!(gap.last_hour_raw, 101);
        }

        s.accept_request(first);
        let ledger = s.hour_ledger();
        assert_eq!(ledger.accepted, 1);
        assert_eq!(ledger.publishing, 0);
        assert_eq!(ledger.evicted_pending, 1, "hour 101");
        assert_eq!(ledger.outstanding, 1, "hour 102 is still being read");
        assert!(ledger.balanced(), "{ledger:?}");
    }

    /// guard from `release_read_custody` and the terminal assertion fails.
    #[test]
    fn a_second_unresolved_publication_is_terminal_not_a_deeper_queue() {
        let mut s = scheduler(&[10, 11], 1, 64);
        s.observe_stable(100, 1);
        let first = s.take_current_request().unwrap();
        assert_eq!(s.release_read_custody(first), 2);

        s.observe_stable(101, 2);
        let second = s.take_current_request().expect("successor is claimable");
        assert_eq!(s.terminal(), None);

        assert_eq!(
            s.release_read_custody(second),
            0,
            "the second release must be refused, not silently queued"
        );
        assert_eq!(s.terminal(), Some(TerminalOutcome::CounterAnomaly));

        let mut s = scheduler(&[10], 1, 64);
        s.observe_stable(1, 1);
        let held = s.take_current_request().unwrap();
        let stranger = CaptureRequest {
            stamp: ClockStamp {
                epoch: 9,
                hour_raw: 9,
                observation_ordinal: 9,
            },
            force_baseline: false,
        };
        assert_eq!(s.release_read_custody(stranger), 0);
        assert_eq!(s.terminal(), Some(TerminalOutcome::CounterAnomaly));
        let _ = held;
    }

    /// A released hour is still resolvable both ways, and a boundary that
    #[test]
    fn a_publishing_hour_survives_a_clock_boundary_and_still_resolves() {
        let mut s = scheduler(&[10, 11], 1, 64);
        s.observe_stable(100, 1);
        let published = s.take_current_request().unwrap();
        s.release_read_custody(published);
        let _ = s.drain_control().count();

        s.note_clock_unreadable();
        let ledger = s.hour_ledger();
        assert_eq!(
            ledger.publishing, 1,
            "the boundary must not touch publication custody"
        );
        assert_eq!(
            ledger.abandoned, 0,
            "nothing was in read custody to abandon"
        );
        assert!(ledger.balanced(), "{ledger:?}");
        assert_eq!(
            s.telemetry().gapped_domain_hours,
            0,
            "a frame already handed to the sink is not a throughput gap"
        );

        s.accept_request(published);
        let ledger = s.hour_ledger();
        assert_eq!(ledger.accepted, 1);
        assert_eq!(ledger.publishing, 0);
        assert!(ledger.balanced(), "{ledger:?}");

        let mut s = scheduler(&[10], 1, 64);
        s.observe_stable(5, 1);
        let refused = s.take_current_request().unwrap();
        s.release_read_custody(refused);
        assert_eq!(s.reject_request(refused), 1);
        let ledger = s.hour_ledger();
        assert_eq!(ledger.refused, 1);
        assert_eq!(ledger.publishing, 0);
        assert!(ledger.balanced(), "{ledger:?}");
        assert_eq!(s.terminal(), None, "a normal refusal is not an anomaly");
    }

    #[test]
    fn a_diverged_per_domain_slot_is_reported_not_silently_balanced() {
        let mut s = scheduler(&[10, 11], 1, 64);
        s.observe_stable(1, 1);
        assert!(s.hour_ledger().slots_uniform);

        let request = s
            .take_domain_request(10)
            .expect("domain 10 can be taken alone");
        assert!(!s.hour_ledger().slots_uniform, "slot 10 moved on its own");
        s.accept_request(request);

        let ledger = s.hour_ledger();
        assert_eq!(ledger.scheduled, 1);
        assert_eq!(ledger.accepted, 1);
        assert_eq!(ledger.outstanding, 0, "slot zero looks empty");
        assert!(
            !ledger.balanced(),
            "domain 11 still holds the hour, so this arithmetic must not be \
             reported as a balanced ledger: {ledger:?}"
        );
    }

    #[derive(Clone, Copy, Debug)]
    enum Step {
        Observe(i32),
        Take,
        Release,
        Accept,
        AcceptDegraded,
        Reject,
        ClockUnreadable,
        FlushPending,
        Decide,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum StepOutput {
        Decision(SchedulerDecision),
        Request(Option<CaptureRequest>),
        Count(usize),
        None,
    }

    /// Well-formed matters. The live protocol is `observe -> take -> [release]
    fn non_interference_script(frames: usize) -> Vec<Step> {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move |state: &mut u64| -> u64 {
            *state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            *state >> 17
        };
        let mut hour = 1_000i32;
        let mut script = Vec::with_capacity(frames * 5);
        for _ in 0..frames {
            match next(&mut state) % 8 {
                0 => script.push(Step::ClockUnreadable),
                1 => {
                    hour += 3;
                    script.push(Step::Observe(hour));
                }
                2 => script.push(Step::FlushPending),
                3 => script.push(Step::Decide),
                _ => {}
            }
            hour += match next(&mut state) % 8 {
                0 => 4,
                1 => -3,
                _ => 1,
            };
            script.push(Step::Observe(hour));
            script.push(Step::Take);
            if next(&mut state) % 2 == 0 {
                script.push(Step::Release);
            }
            script.push(match next(&mut state) % 4 {
                0 => Step::Reject,
                1 => Step::AcceptDegraded,
                _ => Step::Accept,
            });
        }
        script
    }

    /// release is RECORDED, so a scheduler that handed out a different request
    fn replay(scheduler: &mut ReadOnlyScheduler, script: &[Step]) -> Vec<StepOutput> {
        let mut outputs = Vec::with_capacity(script.len());
        let mut last: Option<CaptureRequest> = None;
        let mut ordinal = 0u64;
        for step in script {
            let output = match *step {
                Step::Observe(hour) => {
                    ordinal += 1;
                    scheduler.observe_stable(hour, ordinal);
                    StepOutput::None
                }
                Step::Take => {
                    let request = scheduler.take_current_request();
                    if request.is_some() {
                        last = request;
                    }
                    StepOutput::Request(request)
                }
                Step::Release => match last {
                    Some(request) => StepOutput::Count(scheduler.release_read_custody(request)),
                    None => StepOutput::None,
                },
                Step::Accept => match last.take() {
                    Some(request) => {
                        scheduler.accept_request(request);
                        StepOutput::None
                    }
                    None => StepOutput::None,
                },
                Step::AcceptDegraded => match last.take() {
                    Some(request) => {
                        scheduler.accept_degraded_request(request);
                        StepOutput::None
                    }
                    None => StepOutput::None,
                },
                Step::Reject => match last.take() {
                    Some(request) => StepOutput::Count(scheduler.reject_request(request)),
                    None => StepOutput::None,
                },
                Step::ClockUnreadable => {
                    scheduler.note_clock_unreadable();
                    StepOutput::None
                }
                Step::FlushPending => {
                    scheduler.flush_latest_pending();
                    StepOutput::None
                }
                Step::Decide => StepOutput::Decision(scheduler.decide()),
            };
            outputs.push(output);
        }
        outputs
    }

    fn live_half(mut telemetry: SchedulerTelemetry) -> SchedulerTelemetry {
        telemetry.custody_shadow = CustodyShadowSummary::default();
        telemetry
    }

    #[test]
    fn the_shadow_cannot_change_a_dispatch_decision_even_when_it_is_hostile() {
        let script = non_interference_script(1_000);
        let domains = [10u16, 11, 12];

        let mut off = scheduler_with_shadow(&domains, 1, 65_535, ShadowMode::Disabled);
        let mut on = scheduler_with_shadow(&domains, 1, 65_535, ShadowMode::Enabled);
        let mut hostile = scheduler_with_shadow(&domains, 1, 65_535, ShadowMode::Enabled);
        hostile.set_custody_shadow_hostile(true);

        let off_outputs = replay(&mut off, &script);
        let on_outputs = replay(&mut on, &script);
        let hostile_outputs = replay(&mut hostile, &script);

        assert_eq!(
            off_outputs, on_outputs,
            "the shadow changed a live decision"
        );
        assert_eq!(
            off_outputs, hostile_outputs,
            "a malfunctioning shadow changed a live decision"
        );
        assert_eq!(live_half(off.telemetry()), live_half(on.telemetry()));
        assert_eq!(live_half(off.telemetry()), live_half(hostile.telemetry()));
        assert_eq!(off.hour_ledger(), on.hour_ledger());
        assert_eq!(off.hour_ledger(), hostile.hour_ledger());
        assert_eq!(off.terminal(), on.terminal());
        assert_eq!(off.terminal(), hostile.terminal());

        let off_gaps: Vec<_> = off.drain_control().collect();
        let on_gaps: Vec<_> = on.drain_control().collect();
        let hostile_gaps: Vec<_> = hostile.drain_control().collect();
        assert_eq!(off_gaps, on_gaps);
        assert_eq!(off_gaps, hostile_gaps);

        let dispatches = off_outputs
            .iter()
            .filter(|output| matches!(output, StepOutput::Request(Some(_))))
            .count();
        assert!(
            dispatches > 500,
            "only {dispatches} dispatches; the comparison is vacuous"
        );
        assert_eq!(off.terminal(), None, "the scheduler died mid-script");
        let ledger = off.hour_ledger();
        assert!(ledger.accepted > 400, "{ledger:?}");
        assert!(ledger.refused > 100, "{ledger:?}");
        assert!(ledger.evicted_pending > 50, "{ledger:?}");
        assert!(off.telemetry().reloads > 50);
        assert!(off.telemetry().clock_unreadable_events > 50);
        assert!(!off_gaps.is_empty(), "the script raised no loss at all");
        assert!(
            on.telemetry().custody_shadow.events > 100,
            "the enabled shadow never ran; the comparison is vacuous"
        );
        assert_eq!(
            off.telemetry().custody_shadow,
            CustodyShadowSummary::default(),
            "a disabled shadow must publish nothing but its own inactivity"
        );
    }

    #[test]
    fn the_shadow_follows_a_clean_capture_and_commits_every_hour() {
        let mut s = scheduler_with_shadow(&[10, 11], 1, 64, ShadowMode::Enabled);
        for hour in 2_000..2_010 {
            s.observe_stable(hour, hour as u64);
            let request = s.take_current_request().expect("a clean hour is takeable");
            s.release_read_custody(request);
            s.accept_request(request);
        }
        let summary = s.telemetry().custody_shadow;
        assert!(summary.active);
        assert_eq!(summary.hour_edges, 10);
        assert_eq!(summary.dispatches, 10);
        assert_eq!(summary.committed_hours, 10);
        assert_eq!(summary.processing_backpressure_hours, 0);
        assert_eq!(summary.observation_deadline_miss_hours, 0);
        assert_eq!(summary.illegal_transitions, 0, "the live order is modelled");
        assert_eq!(summary.hour_mismatches, 0);
        assert_eq!(summary.collapsed_publications, 0);
        assert_eq!(summary.live_kept_hours_shadow_dropped, 0);
        assert_eq!(summary.late_dispatch_samples, 10);
        let ledger = s.custody_shadow().ledger();
        assert!(ledger.balanced(), "{ledger:?}");
        assert_eq!(ledger.committed, 10);
        assert_eq!(s.hour_ledger().accepted, 10);
        assert!(s.hour_ledger().balanced());
    }

    #[test]
    fn a_live_eviction_run_reads_as_deadline_misses_the_end_state_agrees_with() {
        let mut s = scheduler_with_shadow(&[10, 11, 12], 1, 64, ShadowMode::Enabled);
        s.observe_stable(1, 1);
        let in_flight = s.take_current_request().expect("hour 1 is takeable");
        for hour in 2..=12 {
            s.observe_stable(hour, hour as u64);
        }
        s.accept_request(in_flight);

        let summary = s.telemetry().custody_shadow;
        assert_eq!(summary.hour_edges, 12);
        assert_eq!(
            summary.agreed_evictions, 10,
            "hours 2..=11 were evicted live and missed their deadline in the \
             shadow: the same ten hours"
        );
        assert_eq!(summary.disagreed_evictions, 0);
        assert_eq!(
            summary.observation_deadline_miss_hours, 11,
            "the ten evicted hours, plus hour 1 -- whose observation was still \
             open when hour 2 began, so its bracket would have spanned the \
             boundary"
        );
        assert_eq!(summary.live_kept_hours_shadow_dropped, 1);
        assert_eq!(summary.committed_hours, 0);
        assert_eq!(summary.illegal_transitions, 0);
        let ledger = s.custody_shadow().ledger();
        assert!(ledger.balanced(), "{ledger:?}");
    }

    /// released into publication custody, hour 501 is taken and published
    #[test]
    fn the_mixed_live_publication_order_reaches_processing_backpressure() {
        let mut s = scheduler_with_shadow(&[10], 1, 64, ShadowMode::Enabled);
        s.observe_stable(500, 1);
        let first = s.take_current_request().expect("hour 500 is takeable");
        s.release_read_custody(first);
        s.observe_stable(501, 2);
        let second = s
            .take_current_request()
            .expect("the successor is claimable");
        s.accept_request(second);
        let summary = s.telemetry().custody_shadow;
        assert_eq!(
            summary.processing_backpressure_hours, 1,
            "closing 501 found 500 still unsealed"
        );
        assert_eq!(summary.collapsed_publications, 1, "501 never released");
        assert_eq!(summary.committed_hours, 1, "501");
        assert_eq!(summary.illegal_transitions, 0);

        s.accept_request(first);
        let summary = s.telemetry().custody_shadow;
        assert_eq!(summary.processing_backpressure_hours, 1);
        assert_eq!(summary.live_kept_hours_shadow_dropped, 1, "hour 500");
        assert_eq!(summary.illegal_transitions, 0);
        let occupancy = s.custody_shadow().machine().occupancy();
        assert_eq!(
            occupancy.iter().filter(|slot| slot.is_some()).count(),
            0,
            "nothing is left in custody: {occupancy:?}"
        );
        let ledger = s.custody_shadow().ledger();
        assert!(ledger.balanced(), "{ledger:?}");
    }

    #[test]
    fn a_slow_live_frame_costs_deadline_misses_and_no_backpressure() {
        let mut s = scheduler_with_shadow(&[10], 1, 64, ShadowMode::Enabled);
        s.observe_stable(600, 1);
        let first = s.take_current_request().expect("hour 600 is takeable");
        s.release_read_custody(first);
        for hour in 601..=603 {
            s.observe_stable(hour, hour as u64);
        }
        s.accept_request(first);

        let summary = s.telemetry().custody_shadow;
        assert_eq!(
            summary.processing_backpressure_hours, 0,
            "hour 600 held the processing slot for three hours and no snapshot \
             ever needed it: the end state loses nothing here"
        );
        assert_eq!(summary.observation_deadline_miss_hours, 2, "hours 601, 602");
        assert_eq!(
            summary.committed_hours, 1,
            "hour 600 commits in the shadow too"
        );
        assert_eq!(summary.live_kept_hours_shadow_dropped, 0);
        assert_eq!(summary.illegal_transitions, 0);
        let ledger = s.custody_shadow().ledger();
        assert!(ledger.balanced(), "{ledger:?}");
    }

    #[test]
    fn a_boundary_invalidates_shadow_custody_and_admits_the_new_epoch() {
        let mut s = scheduler_with_shadow(&[10], 1, 64, ShadowMode::Enabled);
        s.observe_stable(900, 1);
        let request = s.take_current_request().unwrap();
        s.release_read_custody(request);
        s.accept_request(request);
        s.observe_stable(901, 2);
        s.note_clock_unreadable();
        s.observe_stable(905, 3);
        s.observe_stable(100, 4);
        let after_reload = s.take_current_request().expect("the new epoch is takeable");
        assert_eq!(after_reload.stamp.hour_raw, 100);

        let summary = s.telemetry().custody_shadow;
        assert_eq!(
            summary.illegal_transitions, 0,
            "a reload must not read as the shadow's model breaking"
        );
        assert!(summary.invalidated_hours >= 1);
        let ledger = s.custody_shadow().ledger();
        assert!(ledger.balanced(), "{ledger:?}");
    }

    #[test]
    fn a_synchronous_live_publication_is_named_as_a_collapse() {
        let mut s = scheduler_with_shadow(&[10], 1, 64, ShadowMode::Enabled);
        s.observe_stable(700, 1);
        let request = s.take_current_request().unwrap();
        s.accept_request(request); // no release: the synchronous path
        let summary = s.telemetry().custody_shadow;
        assert_eq!(summary.collapsed_publications, 1);
        assert_eq!(summary.committed_hours, 1);
        assert_eq!(summary.illegal_transitions, 0);
    }
}
