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

/// Why a span of hours was never admitted to the data lane. Routes the count to
/// the right telemetry axis. These spans are counted, never turned into control
/// ranges -- see `count_unobserved_span`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnobservedCause {
    ClockUnknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureRequest {
    pub stamp: ClockStamp,
    pub force_baseline: bool,
}

/// What the whole-frame capture owner may do with the scheduler's data lane.
///
/// `AwaitingNextHour` and `AdvancePending` both retain the exact request already
/// owned by the reader.  Keeping them distinct is load-bearing: an ordinary
/// idle poll must leave the delayed frame alone, while a newly observed frontier
/// authorizes publication of that delayed frame before the successor is claimed.
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
    /// Admitted hours released by [`ReadOnlyScheduler::reject_request`]. These
    /// never published; the caller accounts each one with a frame-level `H4FR`
    /// record, which is why they raise no per-domain gap.
    pub refused_hours: u64,
    /// Admitted hours whose capture request was replaced by a newer frontier
    /// before any reader could take it. THIS is the counter that answers "how
    /// many hours were never swept": each one leaves a per-domain
    /// `Backpressure` range and no DATA frame.
    ///
    /// Note carefully that "no DATA frame" is not "no `FRAME_END`". Those ranges
    /// are transported by `build_scheduler_gap_frames`, which closes each one
    /// with `FRAME_END(range_end)` -- so the LAST hour of a coalesced run of
    /// evictions does carry a frame, one containing no domain record at all.
    /// That is why the host cannot learn this number by counting FRAME_END
    /// holes, and why it is counted here at the eviction site instead.
    pub evicted_pending_hours: u64,
    /// Admitted hours dropped because the clock went unknown or the campaign
    /// reloaded under an in-flight or pending request.
    pub abandoned_hours: u64,
    /// Admitted hours dropped by the teardown flush of the not-yet-issued
    /// frontier.
    pub flushed_pending_hours: u64,
    /// Hours that elapsed BETWEEN two stable observations and so were never
    /// admitted at all. Each leaves a per-domain `Overrun` range.
    pub unobserved_overrun_hours: u64,
    /// Hours that elapsed while the authenticated clock could not be read, and
    /// so were never admitted. Counted, not published: naming them on the
    /// control lane would close the last one with a `FRAME_END`.
    pub unobserved_clock_unknown_hours: u64,
    /// Forward counter jumps the scheduler refused to believe
    /// (`delta > MAX_FORWARD_DELTA`). Counted as EVENTS, never as hours: the
    /// implied span rests on a value the scheduler has just rejected, so
    /// turning it into an hour count would assert elapsed time on the strength
    /// of a read that is not trusted.
    pub counter_anomaly_events: u64,
    /// Stable observations discarded because the scheduler was already
    /// terminal. Nothing publishes after a terminal, so these hours leave the
    /// capture span rather than opening a hole -- but they are not invisible.
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
    /// Deepest publication custody ever held (0 or 1). One means the capture
    /// ran the pipelined publication path at least once; the scheduler makes a
    /// second concurrent publication a terminal anomaly, so this can never
    /// exceed one.
    pub max_publishing_depth: u32,
    pub max_control_depth: u32,
    pub request_cap: u32,
    pub control_cap: u32,
    pub terminal: Option<TerminalOutcome>,
}

/// Closed accounting for every game hour the scheduler has seen.
///
/// The point of this type is that no consumer may compute "hours we never
/// looked at" by subtracting published frames from an advanced-hour span. Each
/// disposition below is counted at the exact site where it happens, and
/// [`HourLedger::balanced`] states the identity that makes the set exhaustive.
///
/// Production consumes the individual counters through the `rs_*` probe units,
/// not this struct; `balanced` exists so that a drop site added later without a
/// counter fails a test rather than quietly widening the unexplained set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HourLedger {
    /// Hours admitted to the data lane (one per distinct observed frontier).
    pub scheduled: u64,
    /// Admitted hours that published a frame (complete or degraded).
    pub accepted: u64,
    /// Admitted hours released without publication, accounted by `H4FR`.
    pub refused: u64,
    /// Admitted hours evicted by a newer frontier before a reader took them.
    pub evicted_pending: u64,
    /// Admitted hours dropped at a clock-unknown or reload boundary.
    pub abandoned: u64,
    /// Admitted hours dropped by the teardown flush.
    pub flushed_pending: u64,
    /// Admitted hours still held right now: in read custody, in publication
    /// custody, and/or pending.
    pub outstanding: u64,
    /// The SUBSET of `outstanding` whose reader has finished and whose frame
    /// is awaiting its sink outcome. Broken out because it is the only
    /// disposition an hour can sit in while a LATER hour is already being
    /// read, and a reader of the ledger must not mistake that for a queued
    /// read.
    pub publishing: u64,
    /// Hours never admitted: elapsed between two stable observations.
    pub unobserved_overrun: u64,
    /// Hours never admitted: elapsed while the clock was unreadable.
    pub unobserved_clock_unknown: u64,
    /// Rejected forward counter jumps. An event count, not an hour count.
    pub counter_anomaly_events: u64,
    /// Stable observations discarded because the scheduler was already
    /// terminal. Nothing publishes after a terminal, so these hours leave the
    /// capture span rather than opening a hole -- but they are not invisible.
    pub post_terminal_observations: u64,
    /// False when the per-domain slots have diverged; `balanced` is only
    /// defined for the whole-frame lane and says so rather than guessing.
    pub slots_uniform: bool,
}

impl HourLedger {
    /// Every admitted hour has exactly one disposition, so the dispositions
    /// must sum back to `scheduled`. A new drop site that forgets to count
    /// itself breaks this, which is the whole point.
    pub fn balanced(&self) -> bool {
        self.slots_uniform
            && self.accepted
            .checked_add(self.refused)
            .and_then(|n| n.checked_add(self.evicted_pending))
            .and_then(|n| n.checked_add(self.abandoned))
            .and_then(|n| n.checked_add(self.flushed_pending))
            .and_then(|n| n.checked_add(self.outstanding))
                == Some(self.scheduled)
    }

    /// Hours that put no domain data on the wire, whether or not they were ever
    /// admitted. `refused` is included: an H4FR hour was swept, but published
    /// nothing. Outstanding hours are excluded -- they are not yet resolved
    /// either way.
    ///
    /// Deliberately NOT "hours with no `FRAME_END`". A coalesced gap range is
    /// closed by `FRAME_END(range_end)`, so some of these hours do carry a
    /// frame; what none of them carries is a domain record.
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
    telemetry: SchedulerTelemetry,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DomainRequestSlot {
    /// Exactly one capture may be owned by the reader at a time.
    in_flight: Option<CaptureRequest>,
    /// An hour whose reader has FINISHED and whose bytes are immutable, but
    /// which has not yet been resolved by its publication outcome. See
    /// [`ReadOnlyScheduler::release_read_custody`] for why this is a separate
    /// slot and not a second `in_flight`.
    publishing: Option<CaptureRequest>,
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

    /// The closed hour accounting. See [`HourLedger`].
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
            post_terminal_observations: t.post_terminal_observations,
        }
    }

    /// Admitted hours the scheduler still holds. The whole-frame lane keeps at
    /// most one in flight plus one pending across ALL domains at once, so slot
    /// zero is the canonical view -- but only while the slots agree.
    fn outstanding_hours(&self) -> u64 {
        self.domain_requests.first().map_or(0, |slot| {
            u64::from(slot.in_flight.is_some())
                + u64::from(slot.publishing.is_some())
                + u64::from(slot.latest_pending.is_some())
        })
    }

    /// The subset of [`Self::outstanding_hours`] awaiting a sink outcome.
    fn publishing_hours(&self) -> u64 {
        self.domain_requests
            .first()
            .map_or(0, |slot| u64::from(slot.publishing.is_some()))
    }

    /// Whether every domain slot holds the same pair of requests.
    ///
    /// The per-domain API (`take_domain_request` / `reject_domain`) can move one
    /// slot without the others. It has no production caller -- the collector
    /// drives the whole-frame lane only -- but a ledger that silently reported
    /// `balanced` under divergence would be exactly the kind of accounting this
    /// change exists to remove, so divergence is stated instead of hidden.
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

    /// Admit one stable `CGameState+0x468` observation. The caller is
    /// responsible for authenticating the CGameState root and proving the
    /// opening/closing root+hour pair equal.
    pub fn observe_stable(&mut self, hour_raw: i32, observation_ordinal: u64) {
        if self.telemetry.terminal.is_some() {
            self.telemetry.post_terminal_observations =
                self.telemetry.post_terminal_observations.saturating_add(1);
            return;
        }
        self.telemetry.observations = self.telemetry.observations.saturating_add(1);
        if self.clock_unknown {
            self.gap_all_pending(GapKind::ClockUnreadable);
            // The hours that elapsed while the clock was unreadable were never
            // admitted, so no eviction site sees them. They are COUNTED here
            // and nowhere else.
            //
            // They are deliberately NOT pushed onto the control lane. A control
            // gap range is transported by `build_scheduler_gap_frames`, which
            // closes each range with `FRAME_END(range_end)` -- so emitting one
            // would publish an empty-but-clean frame for the last hour of an
            // interval the recorder never looked at, and the host would then
            // count that hour in `unique_game_hour_count` as observed. Claiming
            // an unobserved hour is worse than the silence it would replace.
            //
            // CONSEQUENCE, accepted deliberately: these hours therefore carry no
            // PER-HOUR producer record, so the host counts them `unaccounted`
            // and the capture fails its gap-acceptance gate. That is the honest
            // outcome -- the recorder cannot vouch for the interval -- and it is
            // strictly better than the tautological pass this replaced. Giving
            // them per-hour evidence needs a transport that can carry a gap
            // range WITHOUT closing a frame, which is a wire change and not this
            // lane's to make.
            self.count_unobserved_span(self.last, hour_raw, UnobservedCause::ClockUnknown);
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
            // The jump is not believed, so the span it implies is not counted
            // as elapsed hours and certainly not named on the control lane:
            // doing either would assert that hours the recorder has no reason
            // to think existed did exist. Only the fact of the anomaly is
            // recorded, and `terminal` already ends scheduling -- no later hour
            // publishes, so the manifest span simply stops here rather than
            // acquiring a hole that needs explaining.
            self.telemetry.counter_anomaly_events =
                self.telemetry.counter_anomaly_events.saturating_add(1);
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

    /// Decide one whole-frame action without collapsing "scheduler idle" into
    /// "scheduler absent".
    ///
    /// The delayed-frame owner cannot claim a successor while the prior request
    /// is in flight.  It must first learn whether a successor frontier exists,
    /// publish the prior frame, accept that request, and only then call again to
    /// claim the successor.  `take_current_request` alone deliberately returns
    /// `None` for both halves of that distinction.
    pub fn decide(&mut self) -> SchedulerDecision {
        let first_in_flight = self
            .domain_requests
            .first()
            .and_then(|slot| slot.in_flight);
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
        Some(request)
    }

    /// Hand an in-flight request from READ custody to PUBLICATION custody.
    ///
    /// `in_flight` used to mean two unrelated things at once:
    ///
    /// 1. "a reader owns this hour" -- the fact that makes the queue capacity
    ///    of one meaningful. A second concurrent read would have to be QUEUED,
    ///    and a queued read observes a later world than its label, which is
    ///    silent misattribution. This meaning must never be relaxed.
    /// 2. "this hour's frame has not been resolved by a sink outcome yet" --
    ///    a fact about immutable bytes that have already been read, already
    ///    been labelled, and can no longer observe anything.
    ///
    /// Only (1) is a reason to refuse the next read. Conflating them made the
    /// synchronous sink write sit on the critical path of the NEXT hour's
    /// sweep: hour N+1 could not be claimed until hour N's bytes had reached
    /// the pipe. That is 29.55 ms of a 149.36 ms frame against a 77.73 ms
    /// per-hour budget, and every hour a frame overruns is evicted into a
    /// `Backpressure` gap by `enqueue_request`.
    ///
    /// After this call the slot holds no read custody, so
    /// [`Self::take_current_request`] may claim the successor -- which is
    /// still, and only, `latest_pending`, i.e. the frontier the observer
    /// thread saw most recently. The successor is therefore read AT its own
    /// hour, exactly as before; nothing is queued and no label moves. The
    /// hour released here stays accounted as `outstanding` until its
    /// publication outcome lands.
    ///
    /// At most ONE hour may be in publication custody at a time. A second
    /// release without an intervening accept/reject would mean two unresolved
    /// frames, which the depth-one handoff cannot produce; it is a terminal
    /// counter anomaly rather than a silently deeper queue.
    ///
    /// Returns the number of domain slots moved, so a caller can distinguish a
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
        moved
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
            // Publication custody first: a pipelined caller released read
            // custody before the write, a synchronous one did not, and both
            // resolve the same hour exactly once.
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
            .filter(|(_, slot)| {
                slot.publishing == Some(request) || slot.in_flight == Some(request)
            })
            .map(|(index, _)| self.domains[index])
            .collect();
        if rejected.is_empty() {
            self.telemetry.terminal = Some(TerminalOutcome::CounterAnomaly);
            return 0;
        }
        for slot in &mut self.domain_requests {
            // Same two-custody resolution as `accept_request`.
            if slot.publishing == Some(request) {
                slot.publishing = None;
            } else if slot.in_flight == Some(request) {
                slot.in_flight = None;
            }
        }
        // No per-domain gaps here. The refused hour is accounted for by the
        // frame-level H4FR record the caller publishes, which names the hour and
        // the domains that forced it. Emitting gaps as well would double-report
        // the hour and would tell a consumer the queue lost throughput, which is
        // not what happened.
        self.telemetry.refused_hours = self.telemetry.refused_hours.saturating_add(1);
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
                if index == 0 {
                    self.telemetry.flushed_pending_hours =
                        self.telemetry.flushed_pending_hours.saturating_add(1);
                }
                self.push_gap(domain, request.stamp, GapKind::Backpressure);
            }
        }
    }

    /// Count the hours strictly between `from`'s frontier and `to`, which the
    /// scheduler never admitted and which therefore reach no eviction site.
    ///
    /// Counting only. See the call site for why these hours must not become
    /// control-lane ranges.
    fn count_unobserved_span(
        &mut self,
        from: Option<ClockStamp>,
        to: i32,
        cause: UnobservedCause,
    ) {
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
        for index in 0..self.domains.len() {
            let domain = self.domains[index];
            if let Some(evicted) = self.domain_requests[index].latest_pending.replace(request) {
                if index == 0 {
                    // Counted once per HOUR, not once per domain. This hour is
                    // never swept. It may still end up inside a FRAME_END: the
                    // Backpressure range accounting it is closed by
                    // `FRAME_END(range_end)`, so the last hour of a coalesced
                    // run carries a frame with no domain record in it.
                    self.telemetry.evicted_pending_hours =
                        self.telemetry.evicted_pending_hours.saturating_add(1);
                }
                self.push_gap(domain, evicted.stamp, GapKind::Backpressure);
            }
            self.telemetry.max_request_depth = self.telemetry.max_request_depth.max(u32::from(
                self.domain_requests[index].latest_pending.is_some(),
            ));
        }
    }

    /// Close the requests a clock-unknown or reload boundary invalidates.
    ///
    /// Deliberately does NOT touch `publishing`. An hour in publication
    /// custody has already been read, at its own hour, inside its own clock
    /// bracket; its bytes are immutable and are on their way to the sink. A
    /// later boundary says nothing about it, and gapping it would either
    /// double-report an hour that then publishes, or claim a throughput loss
    /// that did not happen. It stays `outstanding` until its own sink outcome
    /// resolves it, and the ledger keeps balancing because `outstanding`
    /// counts it.
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

    /// THE SILENCE HOLE, and the trap in closing it.
    ///
    /// While the authenticated clock cannot be read the game keeps advancing,
    /// and those hours used to leave NO record of any kind: `note_clock_unreadable`
    /// gapped only the requests it happened to be holding, and the recovering
    /// `observe_stable` opened a fresh epoch at the new frontier without ever
    /// mentioning the interval it had jumped.
    ///
    /// The obvious fix -- push a control-lane gap range naming the interval --
    /// is WRONG here, and this test pins that. `build_scheduler_gap_frames`
    /// closes every control range with `FRAME_END(range_end)`, so a range for
    /// hours 101..=104 would publish an empty-but-clean frame for hour 104. The
    /// host derives `unique_hours` from FRAME_END records, so hour 104 would
    /// then be counted as observed -- the recorder claiming an hour it never
    /// looked at, which is worse than the silence it replaced.
    ///
    /// So the interval is COUNTED and not published.
    ///
    /// Mutation proof: delete the `count_unobserved_span` call from the
    /// `clock_unknown` branch of `observe_stable` and the count assertion
    /// fails. Restore the earlier `push_all_domain_gap_range` form and the
    /// "no control range" assertion fails.
    #[test]
    fn hours_elapsed_while_the_clock_was_unreadable_are_counted_but_never_published() {
        let mut s = scheduler(&[10, 11], 1, 64);
        s.observe_stable(100, 1);
        // Drain the baseline request so the outage has nothing pending to gap;
        // any evidence after this point has to come from the interval itself.
        let baseline = s.take_current_request().expect("baseline hour is takeable");
        s.accept_request(baseline);
        let _ = s.drain_control().count();

        s.note_clock_unreadable();
        s.note_clock_unreadable();
        // Four game hours pass unobserved, then the clock reads again.
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
        // And the recovery hour is still claimed normally.
        assert!(s.take_current_request().is_some());
    }

    /// A forward counter jump the scheduler refuses to believe is recorded as an
    /// EVENT, never as a span of hours.
    ///
    /// Turning the rejected value into an hour range would assert that those
    /// hours elapsed on the strength of a read the scheduler has just declared
    /// untrustworthy, and -- because control ranges publish
    /// `FRAME_END(range_end)` -- would close a frame at `untrusted_value - 1`.
    /// `terminal` already stops scheduling, so no later hour publishes and the
    /// capture span simply ends here without acquiring a hole.
    ///
    /// Mutation proof: emit a control range for the span (the earlier form) and
    /// the "no control range" assertion fails; delete the
    /// `counter_anomaly_events` increment and the event assertion fails.
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
            s.hour_ledger().unobserved_clock_unknown, 0,
            "a rejected read must not be converted into elapsed hours"
        );
        let gaps: Vec<_> = s.drain_control().collect();
        assert!(gaps.is_empty(), "no range may be claimed from a rejected read: {gaps:?}");

        // Observations after a terminal are discarded, but not invisibly.
        s.observe_stable(far + 1, 3);
        assert_eq!(s.telemetry().post_terminal_observations, 1);
    }

    /// The never-swept-hour shape, in miniature. A frame that outlives its game
    /// hour causes the NEXT hour's request to be replaced before any reader can
    /// take it, so that hour is never swept. (Measured live as 146 of 762 hours
    /// with no FRAME_END in run t15, plus a further 221 that carry only the
    /// gap-only FRAME_END closing a coalesced range.)
    ///
    /// The counter matters because it is the only way to learn the number
    /// without subtracting published frames from an advanced-hour span --
    /// a subtraction that cannot tell an evidenced hole from a silent one.
    ///
    /// Mutation proof: delete the `evicted_pending_hours` increment in
    /// `enqueue_request` and the ledger stops balancing, failing this test.
    #[test]
    fn an_hour_evicted_before_any_reader_took_it_is_counted_where_it_happens() {
        let mut s = scheduler(&[10, 11], 1, 64);
        s.observe_stable(100, 1);
        let in_flight = s.take_current_request().expect("hour 100 is takeable");

        // The reader is still busy. Hours 101 and 102 arrive; 101 is evicted by
        // 102 and will never be seen by anyone.
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

        // Resolving the in-flight frame moves it to `accepted`, and the ledger
        // still balances -- no disposition is double-counted.
        s.accept_request(in_flight);
        let ledger = s.hour_ledger();
        assert_eq!(ledger.accepted, 1);
        assert_eq!(ledger.outstanding, 1);
        assert!(ledger.balanced(), "{ledger:?}");
    }

    /// Every disposition of an admitted hour is counted at its own site, so a
    /// capture that exercises all of them still balances. A new drop path that
    /// forgets to say so breaks this.
    ///
    /// Mutation proof: delete the `refused_hours` increment in `reject_request`,
    /// or the `flushed_pending_hours` increment in `flush_latest_pending`, or
    /// either `abandoned_hours` increment in `gap_all_pending`, and the
    /// corresponding assertion below fails.
    #[test]
    fn every_admitted_hour_has_exactly_one_counted_disposition() {
        let mut s = scheduler(&[10], 1, 64);

        // accepted
        s.observe_stable(1, 1);
        let r = s.take_current_request().unwrap();
        s.accept_request(r);
        // refused
        s.observe_stable(2, 2);
        let r = s.take_current_request().unwrap();
        s.reject_request(r);
        // evicted pending
        s.observe_stable(3, 3);
        let r = s.take_current_request().unwrap();
        s.observe_stable(4, 4);
        s.observe_stable(5, 5);
        // abandoned (in flight + pending) at a clock-unknown boundary
        s.note_clock_unreadable();
        // flushed pending at teardown
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

    /// THE PIPELINE, and the exact property that makes it legal.
    ///
    /// Releasing READ custody lets hour N+1 be claimed while hour N's frame is
    /// still on its way to the sink. The thing that must survive is that the
    /// claimed successor is the frontier the observer saw MOST RECENTLY -- not
    /// a queued older hour -- because a read labelled with an hour it did not
    /// observe is exactly the misattribution the capacity-of-one rule exists
    /// to prevent.
    ///
    /// So: hours 100 (read, released, publishing), then 101 and 102 arrive
    /// while the write is outstanding. The next claim must be 102, the current
    /// frontier -- never 101, which is evicted with a `Backpressure` gap just
    /// as it is on the synchronous path.
    ///
    /// Mutation proof: change `take_current_request` to prefer an evicted
    /// older frontier (e.g. return the request `enqueue_request` displaced
    /// instead of `latest_pending`) and the `hour_raw == 102` assertion fails.
    /// Delete the `slot.publishing = Some(request)` line from
    /// `release_read_custody` and the `publishing == 1` / `balanced`
    /// assertions fail -- hour 100 would vanish from the ledger entirely while
    /// its bytes were still in flight.
    #[test]
    fn releasing_read_custody_claims_the_current_frontier_not_a_queued_hour() {
        let mut s = scheduler(&[10, 11, 12], 1, 64);
        s.observe_stable(100, 1);
        let first = s.take_current_request().expect("hour 100 is takeable");

        // The reader for 100 has finished; its bytes are immutable and the
        // sink write has not happened yet.
        assert_eq!(s.release_read_custody(first), 3);
        let ledger = s.hour_ledger();
        assert_eq!(ledger.publishing, 1, "hour 100 is awaiting its sink outcome");
        assert_eq!(ledger.outstanding, 1);
        assert_eq!(ledger.accepted, 0, "nothing has published yet");
        assert!(ledger.balanced(), "{ledger:?}");

        // Two hours arrive while the write is outstanding.
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

        // 101 is lost exactly as it is on the synchronous path -- evidenced,
        // not silently relabelled.
        let gaps: Vec<_> = s.drain_control().collect();
        assert_eq!(gaps.len(), 3, "one Backpressure range per domain: {gaps:?}");
        for gap in &gaps {
            assert_eq!(gap.kind, GapKind::Backpressure);
            assert_eq!(gap.first_hour_raw, 101);
            assert_eq!(gap.last_hour_raw, 101);
        }

        // And the two hours resolve independently: the write of 100 lands
        // while 102 is still being read.
        s.accept_request(first);
        let ledger = s.hour_ledger();
        assert_eq!(ledger.accepted, 1);
        assert_eq!(ledger.publishing, 0);
        assert_eq!(ledger.evicted_pending, 1, "hour 101");
        assert_eq!(ledger.outstanding, 1, "hour 102 is still being read");
        assert!(ledger.balanced(), "{ledger:?}");
    }

    /// The pipeline is exactly one frame deep, and a second unresolved
    /// publication is a stated anomaly rather than a deeper queue.
    ///
    /// A queue of publications would be harmless for LABELS, but it would let
    /// an unbounded number of hours sit in a state no gap range accounts for
    /// while the sink fell behind. The bound is what keeps the ledger's
    /// `outstanding` term small and knowable.
    ///
    /// Mutation proof: delete the leading `any(|slot| slot.publishing.is_some())`
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

        // Hour 100's write has still not resolved. Releasing 101 too would
        // make two frames unresolved at once.
        assert_eq!(
            s.release_read_custody(second),
            0,
            "the second release must be refused, not silently queued"
        );
        assert_eq!(s.terminal(), Some(TerminalOutcome::CounterAnomaly));

        // Releasing a request nobody holds is the same stated anomaly.
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
    /// invalidates the READ lane leaves it alone.
    ///
    /// The bytes of a publishing hour were read inside their own clock
    /// bracket and are immutable. A later clock-unknown interval says nothing
    /// about them, so `gap_all_pending` must not convert them into a gap --
    /// that would either double-report the hour when the write lands, or
    /// claim a throughput loss that never happened.
    ///
    /// Mutation proof: add a `publishing` arm to `gap_all_pending` that gaps
    /// and counts it `abandoned`, and the `accepted == 1` /
    /// `gapped_domain_hours` assertions below fail.
    #[test]
    fn a_publishing_hour_survives_a_clock_boundary_and_still_resolves() {
        let mut s = scheduler(&[10, 11], 1, 64);
        s.observe_stable(100, 1);
        let published = s.take_current_request().unwrap();
        s.release_read_custody(published);
        let _ = s.drain_control().count();

        // The clock goes unreadable while the write is outstanding.
        s.note_clock_unreadable();
        let ledger = s.hour_ledger();
        assert_eq!(
            ledger.publishing, 1,
            "the boundary must not touch publication custody"
        );
        assert_eq!(ledger.abandoned, 0, "nothing was in read custody to abandon");
        assert!(ledger.balanced(), "{ledger:?}");
        assert_eq!(
            s.telemetry().gapped_domain_hours,
            0,
            "a frame already handed to the sink is not a throughput gap"
        );

        // The write lands afterwards and resolves the hour normally.
        s.accept_request(published);
        let ledger = s.hour_ledger();
        assert_eq!(ledger.accepted, 1);
        assert_eq!(ledger.publishing, 0);
        assert!(ledger.balanced(), "{ledger:?}");

        // And the refusal half works from publication custody too.
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

    /// `balanced` is a claim about the whole-frame lane. The per-domain API can
    /// move one slot on its own; when it does, the ledger says the accounting is
    /// undefined rather than reporting a balance it cannot support.
    ///
    /// Mutation proof: drop the `self.slots_uniform &&` conjunct from
    /// `HourLedger::balanced` and this test fails -- one hour then reads as
    /// fully accepted while another domain still holds it.
    #[test]
    fn a_diverged_per_domain_slot_is_reported_not_silently_balanced() {
        let mut s = scheduler(&[10, 11], 1, 64);
        s.observe_stable(1, 1);
        assert!(s.hour_ledger().slots_uniform);

        let request = s.take_domain_request(10).expect("domain 10 can be taken alone");
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
}
