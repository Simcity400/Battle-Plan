//! Read-only capture scheduling and transactional domain admission.
//!
//! This module is deliberately incapable of controlling the game. Its only
//! clock input is an authenticated, stable observation supplied by the
//! injected collector. It never accepts an engine address and has no unsafe
//! code, write primitive, pause concept, or UI dependency.
//!
//! The scheduler accounts every observed hour without pretending that a
//! current snapshot represents an earlier hour. If several hours advance
//! before the reader is ready, only the current frontier may be sampled; each
//! older hour becomes an explicit `Overrun` gap. A bounded request ring and a
//! bounded, range-coalescing control lane make sustained speed-five overload
//! finite and observable.

use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};

const MAX_FORWARD_DELTA: i64 = 24 * 32;

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
pub struct CaptureRequest {
    pub stamp: ClockStamp,
    pub force_baseline: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DomainGapRange {
    pub domain_id: u16,
    pub epoch: u64,
    pub first_hour_raw: i32,
    pub last_hour_raw: i32,
    pub kind: GapKind,
    pub force_next_baseline: bool,
    /// Exact read-only clock witnesses associated with this loss.
    pub tick_before: u64,
    pub tick_after: u64,
    /// Nonzero only for bounded-queue backpressure.
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
    /// Hours whose frame reached the production sink. Complete AND degraded
    /// hours both count here, because both published; the question this counter
    /// exists to answer is "did anything ever publish".
    pub accepted_hours: u64,
    /// The SUBSET of `accepted_hours` that published with at least one domain
    /// unknown (an `H4FD` notice rode in the frame). Kept separate so a reader
    /// can never mistake a degraded hour for a clean one from telemetry alone,
    /// which is the same distinction the wire keeps.
    pub degraded_hours: u64,
    pub gapped_domain_hours: u64,
    pub overrun_domain_hours: u64,
    pub backpressure_domain_hours: u64,
    pub torn_domain_hours: u64,
    pub worker_panic_domain_hours: u64,
    pub clock_unreadable_events: u64,
    pub control_lane_compactions: u64,
    pub reloads: u64,
    pub max_request_depth: u32,
    pub max_control_depth: u32,
    pub request_cap: u32,
    pub control_cap: u32,
    pub terminal: Option<TerminalOutcome>,
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
    telemetry: SchedulerTelemetry,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DomainRequestSlot {
    /// Exactly one capture may be owned by the reader at a time.
    in_flight: Option<CaptureRequest>,
    /// While a reader is in flight, only the newest observed frontier is kept.
    /// Replaced frontiers become typed ranges on the independent control lane.
    latest_pending: Option<CaptureRequest>,
}

impl ReadOnlyScheduler {
    pub fn new(
        mut domains: Vec<u16>,
        request_cap: usize,
        control_cap: usize,
    ) -> Result<Self, &'static str> {
        domains.sort_unstable();
        domains.dedup();
        if domains.is_empty() {
            return Err("read-only scheduler requires at least one domain");
        }
        if domains.iter().any(|domain| *domain == 0) {
            return Err("read-only scheduler domain id zero is reserved");
        }
        // The external setting is retained as a compatibility input, but the
        // production scheduler has one semantic capacity: one in-flight plus
        // one latest-pending request per domain. A larger configured number
        // must never silently turn this into a historical snapshot queue.
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
            telemetry: SchedulerTelemetry {
                request_cap: 1,
                control_cap: control_cap as u32,
                ..SchedulerTelemetry::default()
            },
        })
    }

    pub fn domains(&self) -> &[u16] {
        &self.domains
    }

    pub fn generation(&self) -> u64 {
        self.telemetry.observations
    }

    pub fn telemetry(&self) -> SchedulerTelemetry {
        self.telemetry.clone()
    }

    pub fn terminal(&self) -> Option<TerminalOutcome> {
        self.telemetry.terminal
    }

    /// Admit one stable `CGameState+0x468` observation. The caller is
    /// responsible for authenticating the CGameState root and proving the
    /// opening/closing root+hour pair equal.
    pub fn observe_stable(&mut self, hour_raw: i32, observation_ordinal: u64) {
        if self.telemetry.terminal.is_some() {
            return;
        }
        self.telemetry.observations = self.telemetry.observations.saturating_add(1);
        if self.clock_unknown {
            self.gap_all_pending(GapKind::ClockUnreadable);
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
            self.telemetry.terminal = Some(TerminalOutcome::CounterAnomaly);
            return;
        }

        // A free-running game may advance by more than one hour between stable
        // observations. The elapsed intermediate states no longer exist and
        // must never be reconstructed by attaching the latest world to an old
        // label.
        if delta > 1 {
            self.push_all_domain_gap_range(
                last.epoch,
                last.hour_raw.saturating_add(1),
                hour_raw.saturating_sub(1),
                GapKind::Overrun,
                clock_witness(last.hour_raw),
                clock_witness(hour_raw.saturating_sub(1)),
            );
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

    /// A failed authenticated clock read opens an explicit unknown interval.
    /// The next stable observation starts a new epoch and forces a baseline;
    /// elapsed hours are never guessed or silently bridged.
    pub fn note_clock_unreadable(&mut self) {
        self.telemetry.clock_unreadable_events =
            self.telemetry.clock_unreadable_events.saturating_add(1);
        if !self.clock_unknown {
            self.gap_all_pending(GapKind::ClockUnreadable);
            self.clock_unknown = true;
            self.force_baseline = true;
        }
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
            // This can occur only after a domain-specific consumer. A
            // whole-frame consumer cannot join unlike frontiers and therefore
            // waits for the domain consumer to resolve them.
            return None;
        }
        for slot in &mut self.domain_requests {
            slot.latest_pending = None;
            slot.in_flight = Some(request);
        }
        Some(request)
    }

    pub fn take_domain_request(&mut self, domain_id: u16) -> Option<CaptureRequest> {
        let index = self.domains.binary_search(&domain_id).ok()?;
        let slot = &mut self.domain_requests[index];
        if slot.in_flight.is_some() {
            return None;
        }
        let request = slot.latest_pending.take()?;
        slot.in_flight = Some(request);
        Some(request)
    }

    /// Resolve an in-flight request whose frame published with at least one
    /// domain unknown.
    ///
    /// The frame reached the sink, so the request is ACCEPTED exactly as a
    /// complete one is -- leaving it in flight would wedge `take_current_request`
    /// for the rest of the capture, which is the defect `reject_request` exists
    /// to prevent. What differs is only the accounting: the degraded hour is
    /// also counted on its own axis, so `accepted_hours` never implies clean.
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
            if slot.in_flight == Some(request) {
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
    }

    /// Resolve a whole-frame in-flight request whose frame will NOT be published.
    ///
    /// The missing other half of [`Self::accept_request`].
    /// [`Self::take_current_request`] refuses while ANY slot still holds
    /// `in_flight`, and only `accept_request` cleared it -- so a frame that was
    /// taken and then refused left every slot wedged for the rest of the
    /// capture, and every later hour was evicted into a backpressure gap.
    /// Measured on four consecutive live runs: `accepted_hours=0` with
    /// `gapped_domain_hours` exactly one hour short of `scheduled_hours`.
    ///
    /// The capture loop's own comment already required this -- "the in-flight
    /// request still owns the final assembled DATA frame and must be accepted or
    /// rejected after its sink outcome is known" -- but there was no reject to
    /// call.
    ///
    /// `force_baseline` is set so the next observation re-establishes a baseline
    /// instead of assuming continuity across the hole. The refused hour itself is
    /// accounted for by the frame-level H4FR record, not by per-domain gaps.
    ///
    /// Returns the number of domain slots released, so a caller can distinguish
    /// a real release from a no-op without inspecting private state.
    pub fn reject_request(&mut self, request: CaptureRequest) -> usize {
        let rejected: Vec<u16> = self
            .domain_requests
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.in_flight == Some(request))
            .map(|(index, _)| self.domains[index])
            .collect();
        if rejected.is_empty() {
            self.telemetry.terminal = Some(TerminalOutcome::CounterAnomaly);
            return 0;
        }
        for slot in &mut self.domain_requests {
            if slot.in_flight == Some(request) {
                slot.in_flight = None;
            }
        }
        // No per-domain gaps here. The refused hour is accounted for by the
        // frame-level H4FR record the caller publishes, which names the hour and
        // the domains that forced it. Emitting gaps as well would double-report
        // the hour and would tell a consumer the queue lost throughput, which is
        // not what happened.
        self.force_baseline = true;
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
            Some(_) => Err(
                "nonempty scheduler control-gap custody must commit a durable prefix",
            ),
            None => Err("scheduler control-gap reservation is not active"),
        }
    }

    /// Commit an already-published prefix of the active control reservation.
    ///
    /// H4DG ranges can span multiple physical H4CR frames because each range
    /// retains its own game-hour label. Advancing custody one accepted frame
    /// at a time prevents a later sink failure from rolling back (and then
    /// republishing) an earlier range that is already durable.
    pub fn commit_reserved_control_prefix(
        &mut self,
        count: usize,
    ) -> Result<(), &'static str> {
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

    /// Close only requests that have not been handed to a reader. The caller
    /// still owns every in-flight request and must publish or reject it before
    /// teardown; converting it here would overlap a later accepted frame.
    pub fn flush_latest_pending(&mut self) {
        for index in 0..self.domains.len() {
            let domain = self.domains[index];
            if let Some(request) = self.domain_requests[index].latest_pending.take() {
                self.push_gap(domain, request.stamp, GapKind::Backpressure);
            }
        }
    }

    fn enqueue_request(&mut self, request: CaptureRequest) {
        self.telemetry.scheduled_hours = self.telemetry.scheduled_hours.saturating_add(1);
        for index in 0..self.domains.len() {
            let domain = self.domains[index];
            if let Some(evicted) = self.domain_requests[index].latest_pending.replace(request) {
                self.push_gap(domain, evicted.stamp, GapKind::Backpressure);
            }
            self.telemetry.max_request_depth = self.telemetry.max_request_depth.max(u32::from(
                self.domain_requests[index].latest_pending.is_some(),
            ));
        }
    }

    fn gap_all_pending(&mut self, kind: GapKind) {
        for index in 0..self.domains.len() {
            let domain = self.domains[index];
            if let Some(request) = self.domain_requests[index].in_flight.take() {
                self.push_gap(domain, request.stamp, kind);
            }
            if let Some(request) = self.domain_requests[index].latest_pending.take() {
                self.push_gap(domain, request.stamp, kind);
            }
        }
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
        // Control entries are interleaved by domain when a frontier advances.
        // Extend the newest range for this domain, not merely the global tail;
        // otherwise every missed speed-five hour would consume one control
        // slot per domain instead of remaining one exact contiguous range.
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
                        // Queue pressure is observed at one current frontier;
                        // a compacted range retains the newest equal witness.
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
            // Never merge unlike reasons or non-contiguous hours into a false
            // envelope. The control lane is sized independently and drained
            // before data capture; exhausting it is an activation failure, not
            // permission to fabricate loss coverage.
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

/// Run a domain reader transactionally. `cursor` is cloned into staging and is
/// committed only if the opening and closing authenticated clock stamps match.
/// A torn or panicking reader cannot advance a hook cursor or any other
/// cross-frame tracker.
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

    /// The wedge, and that rejecting clears it.
    ///
    /// This is the live failure reproduced in miniature. A frame is taken and
    /// then refused (incomplete), so it is never accepted. Before
    /// `reject_request` existed there was no way to release it, and
    /// `take_current_request` refuses while ANY slot holds `in_flight` -- so
    /// every later hour was evicted into a backpressure gap and the capture
    /// produced nothing for the rest of its window. Measured live four times:
    /// `accepted_hours=0`, `gapped_domain_hours` exactly one hour short of
    /// `scheduled_hours`.
    ///
    /// Mutation proof: delete the `slot.in_flight = None` loop from
    /// `reject_request` and the "must service the next hour" assertion fails --
    /// the scheduler stays wedged exactly as it did in production.
    #[test]
    fn a_refused_frame_releases_its_request_instead_of_wedging_the_scheduler() {
        let mut s = scheduler(&[10, 11, 12], 1, 64);

        s.observe_stable(100, 1);
        let first = s.take_current_request().expect("hour 100 must be takeable");

        // The frame built from `first` is judged incomplete, so it is never
        // accepted. A later hour arrives.
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
        assert_eq!(released, 3, "every domain slot holding the request is released");

        // THE FIX: the next hour is takeable again.
        let second = s
            .take_current_request()
            .expect("after a reject the scheduler must service the next hour");
        assert_ne!(second, first, "the next hour is a distinct request");

        let telemetry = s.telemetry();
        assert_eq!(telemetry.accepted_hours, 0, "nothing was published");
        // Deliberately NO per-domain gaps: the refused hour is accounted for by
        // the frame-level H4FR record the caller publishes. Emitting gaps here
        // as well would double-report the hour and would tell a consumer the
        // queue lost throughput, which is not what happened.
        assert_eq!(
            telemetry.gapped_domain_hours, 0,
            "a refusal is not a throughput gap"
        );

        // And a reject of something not in flight is a no-op that says so,
        // rather than silently releasing an unrelated request.
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
}
