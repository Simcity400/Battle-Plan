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

use crate::custody_shadow::{CustodyShadow, CustodyShadowSummary, ShadowClock, ShadowMode};

/// The largest forward clock motion this scheduler believes.
///
/// Aliased from the contract rather than restated: the `H4CJ` record carries
/// this value and its reader REQUIRES the pinned one, so a scheduler that
/// drifted would emit a record its own consumer refuses. One constant, two
/// users, no drift.
const MAX_FORWARD_DELTA: i64 =
    compact_contract::untrusted_clock_recovery::UNTRUSTED_CLOCK_RECOVERY_TRUST_CEILING_HOURS;

/// How many unpublished clock-unreadable intervals the scheduler will hold.
///
/// One interval is minted per OUTAGE, and the observer mints them on its own
/// poll while the capture loop drains them only when it returns to the top of a
/// sweep -- so the queue depth is bounded by outages per sweep, and nothing in
/// the tree bounds that. A small cap was therefore reachable by an honest
/// capture: 257 short outages during one long publication silently lost the
/// last one, and a lost interval becomes an unexplained hole that fails the
/// host's gap gate. The number of outages is in turn bounded by the hours of the
/// capture, so this matches the control lane's own ceiling and cannot be reached
/// by any bounded run -- while still refusing to grow without limit under a
/// wedged transport.
///
/// A drop is never silent: it is counted in
/// [`SchedulerTelemetry::clock_unreadable_intervals_dropped`], which reaches the
/// wire as the `rs_clock_unreadable_intervals_dropped` probe unit.
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

/// Why a span of hours was never admitted to the data lane. Routes the count to
/// the right telemetry axis. These spans are counted, never turned into control
/// ranges -- see `count_unobserved_span`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnobservedCause {
    ClockUnknown,
}

/// What the scheduler makes of the value the clock returned when it read again
/// after an outage.
///
/// The three arms are exhaustive and disjoint, and each one owns exactly one
/// wire outcome. This type exists so the ceiling is applied ONCE, at the single
/// site that decides, rather than being re-derived by each consumer of the
/// recovery -- an earlier shape applied it inside the interval minter, where a
/// refusal could not say which of two very different reasons had refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClockRecoveryTrust {
    /// The outage resumed in the hour it started, or the next one. No hour was
    /// lost, so there is nothing to account for and NOTHING is minted -- the
    /// wire grammar refuses an empty range, and a producer that emitted one
    /// would be writing a record its own reader rejects.
    NoHoursLost,
    /// A believable forward motion. The hours between are vouched for and reach
    /// the wire as an `H4CU` interval.
    Trusted,
    /// A forward motion larger than `MAX_FORWARD_DELTA`, which the ordinary
    /// path refuses outright as a counter anomaly. The recovery path ACCEPTS it
    /// -- one failed clock read is far too weak a reason to destroy a capture,
    /// and the owner ruled on 2026-08-05 that it is recorded rather than made
    /// terminal -- but the span it implies is never converted into vouched
    /// hours, and the jump reaches the wire as its own `H4CJ` record carrying
    /// the OBSERVED delta.
    Untrusted { observed_delta_hours: i64 },
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
    /// so were never admitted. Never named on the CONTROL lane -- a range there
    /// would close its last hour with a `FRAME_END` -- but published as their
    /// own frame-level `H4CU` interval record, which closes nothing.
    pub unobserved_clock_unknown_hours: u64,
    /// Clock-unreadable intervals discarded because the publication queue was
    /// full. Nonzero means some outage reached no wire record and its hours will
    /// read as unaccounted; a capture is not allowed to lose that silently.
    pub clock_unreadable_intervals_dropped: u64,
    /// Forward counter jumps the scheduler refused to believe
    /// (`delta > MAX_FORWARD_DELTA`). Counted as EVENTS, never as hours: the
    /// implied span rests on a value the scheduler has just rejected, so
    /// turning it into an hour count would assert elapsed time on the strength
    /// of a read that is not trusted.
    pub counter_anomaly_events: u64,
    /// Clock recoveries whose resumed value was a forward jump the scheduler
    /// does not trust (`delta > MAX_FORWARD_DELTA`). Counted as EVENTS for the
    /// same reason `counter_anomaly_events` is: the implied span rests on the
    /// rejected value, so turning it into an hour count would assert elapsed
    /// time on the strength of a read that is not trusted.
    ///
    /// The ordinary path makes such a jump terminal. This path does not -- the
    /// owner ruled that one failed clock read must not destroy a capture -- so
    /// this counter, the delta beside it, and the `H4CJ` record the recovery
    /// publishes are the whole of the evidence that the hours scheduled from
    /// here on descend from an untrusted read.
    pub untrusted_clock_recovery_events: u64,
    /// The largest jump any of those recoveries observed, in hours. THE ACTUAL
    /// SIZE, not a boolean: a fold deciding what to do with the capture, and a
    /// human reading the probe, both need to know whether the clock moved a day
    /// past the ceiling or a decade. Per-event deltas are on the wire in `H4CJ`;
    /// this is the running maximum, so it never resets.
    pub max_untrusted_clock_recovery_delta_hours: u64,
    /// Untrusted recoveries discarded because the publication queue was full.
    /// Nonzero means an untrusted jump reached no wire record, so the hours it
    /// scheduled cannot be told apart from hours reached through a trusted read
    /// -- exactly the silence this record exists to end.
    pub untrusted_clock_recoveries_dropped: u64,
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
    /// Capture-floor stage S3 SHADOW accounting. Describes what the end-state
    /// custody pipeline WOULD have done with the same hours; it never describes
    /// what this capture did, and nothing in this struct above it is derived
    /// from it. See [`crate::custody_shadow`].
    pub custody_shadow: CustodyShadowSummary,
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
    /// Clock recoveries that resumed on a forward jump the scheduler does not
    /// trust. An event count, not an hour count, for the same reason: the span
    /// rests on the rejected value. These recoveries DID schedule their resumed
    /// hour -- see [`SchedulerTelemetry::untrusted_clock_recovery_events`].
    pub untrusted_clock_recovery_events: u64,
    /// The largest jump those recoveries observed, in hours. The observed size,
    /// never a boolean.
    pub max_untrusted_clock_recovery_delta_hours: u64,
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
    /// Failed clock reads since the current outage began. Reset when the clock
    /// reads again, and carried into the interval the recovery publishes -- an
    /// outage that claims hours must be able to say how many reads failed.
    clock_unknown_events: u32,
    /// Clock-unreadable intervals observed and not yet published. Drained by the
    /// capture loop, which emits one `H4CU` record per entry.
    ///
    /// These are NOT control gap ranges and must never be put on that lane: see
    /// `observe_stable`. They are held here so the interval reaches the wire as
    /// its own frame-level record, which is what makes those hours ACCOUNTED for
    /// the host's gap gate instead of an unexplained hole.
    clock_unreadable_intervals: VecDeque<ClockUnreadableInterval>,
    /// Untrusted clock recoveries observed and not yet published. Drained by the
    /// capture loop, which emits one `H4CJ` record per entry.
    ///
    /// A separate queue from `clock_unreadable_intervals` because the two carry
    /// opposite claims -- an interval VOUCHES for its hours, a recovery
    /// explicitly refuses to -- and one queue holding both would have let a
    /// publication path treat them alike. Bounded by the same ceiling, and a
    /// drop is counted in
    /// [`SchedulerTelemetry::untrusted_clock_recoveries_dropped`].
    untrusted_clock_recoveries: VecDeque<UntrustedClockRecovery>,
    telemetry: SchedulerTelemetry,
    /// THE SHADOW. Read the module comment on [`crate::custody_shadow`] before
    /// touching this field.
    ///
    /// It is written to at the END of the methods below and read by NOTHING in
    /// this file except [`Self::telemetry`] and the test accessors. Every call
    /// hands it `Copy` scalars; it is never given `&self`, so it cannot reach
    /// one field of this scheduler on its own. That, plus the call placement,
    /// is why a shadow that malfunctioned could not change one dispatch
    /// DECISION -- which
    /// `the_shadow_cannot_change_a_dispatch_decision_even_when_it_is_hostile`
    /// then demonstrates against a shadow built to malfunction.
    ///
    /// A decision is not the same as a schedule. The notifications are
    /// synchronous, so they do occupy the calling thread -- and in production
    /// the scheduler mutex -- for the 316-588 ns per frame measured in
    /// `custody_shadow::tests::shadow_cost_per_frame_is_negligible`. That is
    /// the whole of the physical cost, it is five orders of magnitude under the
    /// 95 ms hour every scheduler decision turns on, and `BATTLE_PLAN_CUSTODY_SHADOW=0`
    /// removes it entirely.
    shadow: CustodyShadow,
}

/// One untrusted clock recovery, exactly as the wire record spells it.
///
/// The counterpart to [`ClockUnreadableInterval`], and deliberately a different
/// type: an interval names a range of hours the producer VOUCHES elapsed, this
/// names two endpoints and vouches for nothing between them. Sharing one type
/// would have let a publication path account for hours on the strength of a read
/// the scheduler had just rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UntrustedClockRecovery {
    pub last_observed_hour_raw: i32,
    pub resumed_hour_raw: i32,
    pub unreadable_events: u32,
    /// The ACTUAL jump size in hours, not a boolean "too far".
    pub observed_delta_hours: i64,
    /// The ceiling that judged it, carried so the record substantiates its own
    /// claim rather than asking a consumer to know the producer's constant.
    pub trust_ceiling_hours: i64,
}

/// One elapsed-but-unobserved interval, exactly as the wire record spells it.
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
        Self::with_custody_shadow(domains, request_cap, control_cap, CustodyShadow::default())
    }

    /// The production constructor, with the shadow's environment kill switch
    /// applied once. `BATTLE_PLAN_CUSTODY_SHADOW=0` builds a scheduler whose shadow
    /// never runs; everything else about the scheduler is identical, which is
    /// the property `the_shadow_cannot_change_a_dispatch_decision_even_when_it_is_hostile`
    /// pins.
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

    /// Build with an explicit shadow. Tests use this to pin a manual clock or
    /// to turn the shadow off entirely.
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
        // Read here rather than mirrored on every notification: the shadow's
        // summary is a pure function of the shadow, and copying it at each
        // event would put shadow bookkeeping on the live path for no gain.
        telemetry.custody_shadow = self.shadow.summary();
        telemetry
    }

    /// The shadow's own view, for tests and for a host that wants the detail
    /// behind the `rs_shadow_*` counters. Read-only; there is no accessor that
    /// lets a caller step the shadow.
    pub fn custody_shadow(&self) -> &CustodyShadow {
        &self.shadow
    }

    /// Test-only: make the shadow destroy its own state inside every
    /// notification. The notifications happen inside the same critical section
    /// (and, in production, under the same mutex) as the live decisions, so a
    /// scheduler that read shadow state anywhere would produce different
    /// decisions here -- and
    /// `the_shadow_cannot_change_a_dispatch_decision_even_when_it_is_hostile` requires
    /// that it does not.
    #[cfg(test)]
    pub fn set_custody_shadow_hostile(&mut self, hostile: bool) {
        self.shadow.set_hostile(hostile);
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
            untrusted_clock_recovery_events: t.untrusted_clock_recovery_events,
            max_untrusted_clock_recovery_delta_hours: t
                .max_untrusted_clock_recovery_delta_hours,
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
            // What they DO get is a record of their own. The interval is queued
            // here and published as one frame-level `H4CU`
            // (`COMPACT_RECORD_TYPE_CLOCK_UNREADABLE_INTERVAL`), a record that
            // names a RANGE, closes no hour and publishes no FRAME_END -- so the
            // host can count these hours as ACCOUNTED without any of them being
            // claimed as observed. Before that record existed the outage was
            // counted here and nowhere else, and the host's gap gate, which
            // demands a typed producer statement for every hole hour, failed the
            // whole capture over an interval the producer could describe exactly.
            //
            // UNLESS the value the clock came back with is one this scheduler
            // does not believe. The ordinary path refuses a forward delta above
            // `MAX_FORWARD_DELTA` outright; the recovery path accepts it, on the
            // owner's 2026-08-05 ruling that one failed clock read is too weak a
            // reason to destroy a capture. Accepting is not the same as
            // believing: the span is NOT counted as elapsed hours and NOT minted
            // as a vouched interval, because both would assert elapsed time on
            // the strength of the rejected value. The jump gets its own record
            // instead -- see `ClockRecoveryTrust`.
            match self.clock_recovery_trust(self.last, hour_raw) {
                ClockRecoveryTrust::NoHoursLost => {}
                ClockRecoveryTrust::Trusted => {
                    self.count_unobserved_span(
                        self.last,
                        hour_raw,
                        UnobservedCause::ClockUnknown,
                    );
                    self.queue_clock_unreadable_interval(self.last, hour_raw);
                }
                ClockRecoveryTrust::Untrusted {
                    observed_delta_hours,
                } => {
                    self.queue_untrusted_clock_recovery(
                        self.last,
                        hour_raw,
                        observed_delta_hours,
                    );
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
        // Counted per OUTAGE as well as per capture, because the interval record
        // has to substantiate its own claim: a range of unobserved hours that
        // names no failed read is indistinguishable from an ordinary hour jump,
        // and the wire grammar refuses one.
        self.clock_unknown_events = self.clock_unknown_events.saturating_add(1);
        if !self.clock_unknown {
            self.gap_all_pending(GapKind::ClockUnreadable);
            self.clock_unknown = true;
            self.force_baseline = true;
        }
    }

    /// Queue the interval a recovering observation just closed, if it contains
    /// any hours at all.
    ///
    /// An outage that resumes in the SAME hour it started, or in the next one,
    /// lost no hours: there is nothing to account for and no record is minted.
    /// The wire grammar refuses an empty range for exactly that reason, so
    /// emitting one here would be a producer writing a record its own reader
    /// rejects.
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
        // THE CEILING IS NOT APPLIED HERE.
        //
        // It used to be, and the refusal it produced was indistinguishable on
        // the wire from a queue-full drop: both incremented
        // `clock_unreadable_intervals_dropped`, so a nonzero counter could not
        // say whether the producer had run out of room or had refused to vouch
        // for a read it did not trust. The decision now happens ONCE, in
        // `clock_recovery_trust`, and an untrusted recovery never reaches this
        // function at all -- it mints an `H4CJ` instead. This counter therefore
        // means exactly one thing again: an outage that reached no wire record.
        //
        // The events counter is what substantiates the claim. It cannot be zero
        // here -- `clock_unknown` is only set by `note_clock_unreadable`, which
        // increments it first -- but a saturating counter that somehow read zero
        // would mint a record the reader refuses, so state the floor.
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

    /// What this scheduler makes of the value the clock returned on recovery.
    ///
    /// THE SINGLE SITE THAT APPLIES THE CEILING to a recovery, and the reason
    /// the two outcomes can never both fire for one outage.
    ///
    /// Compared as the DELTA between the two observed values, which is what
    /// `observe_stable` compares on the ordinary path -- not as the width of the
    /// missing span, which is one smaller. Comparing the span would admit
    /// exactly the one value the ordinary path refuses: a delta of
    /// `MAX_FORWARD_DELTA + 1` leaves a span of `MAX_FORWARD_DELTA`, so the
    /// untrusted read would have minted a vouched interval covering every hour
    /// it implied.
    fn clock_recovery_trust(&self, from: Option<ClockStamp>, resumed_hour: i32) -> ClockRecoveryTrust {
        let Some(from) = from else {
            return ClockRecoveryTrust::NoHoursLost;
        };
        let delta = i64::from(resumed_hour) - i64::from(from.hour_raw);
        if delta > MAX_FORWARD_DELTA {
            return ClockRecoveryTrust::Untrusted {
                observed_delta_hours: delta,
            };
        }
        // An outage that resumes in the SAME hour it started, or in the next
        // one, lost no hours. Non-forward motion lands here too: a resumed value
        // at or below the last observed one is a reload boundary's business, not
        // an interval's, and minting a range for it would be a producer writing
        // a record its own reader refuses.
        if delta <= 1 {
            return ClockRecoveryTrust::NoHoursLost;
        }
        ClockRecoveryTrust::Trusted
    }

    /// Queue the untrusted jump a recovering observation just made, so the hours
    /// it goes on to schedule can be told apart from hours reached through a
    /// clock read this scheduler believed.
    ///
    /// Counts the event and the OBSERVED DELTA -- never hours. Turning the span
    /// into an hour count is precisely what the ordinary path refuses to do with
    /// the same value, and doing it here would let one failed clock read buy a
    /// claim the scheduler would otherwise reject outright.
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
        // The events counter is what substantiates the outage claim, exactly as
        // it does for an interval: a jump that names no failed read is an
        // ordinary forward jump, and the wire grammar refuses one wearing this
        // record's name.
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

    /// Take every queued untrusted recovery, in the order the jumps happened.
    pub fn drain_untrusted_clock_recoveries(
        &mut self,
    ) -> impl Iterator<Item = UntrustedClockRecovery> + '_ {
        self.untrusted_clock_recoveries.drain(..)
    }

    /// Whether any untrusted recovery is waiting to be published.
    pub fn has_untrusted_clock_recoveries(&self) -> bool {
        !self.untrusted_clock_recoveries.is_empty()
    }

    /// Take every queued interval, in the order the outages happened.
    pub fn drain_clock_unreadable_intervals(
        &mut self,
    ) -> impl Iterator<Item = ClockUnreadableInterval> + '_ {
        self.clock_unreadable_intervals.drain(..)
    }

    /// Whether any interval is waiting to be published. Cheap enough to ask on
    /// every loop iteration.
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
            // This can occur only after a domain-specific consumer. A
            // whole-frame consumer cannot join unlike frontiers and therefore
            // waits for the domain consumer to resolve them.
            return None;
        }
        for slot in &mut self.domain_requests {
            slot.latest_pending = None;
            slot.in_flight = Some(request);
        }
        // SHADOW: the live bracket starts here, and the distance from this
        // instant back to the hour edge is the lateness the whole S3 stage is
        // about.
        self.shadow
            .note_dispatch((request.stamp.epoch, request.stamp.hour_raw));
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

    /// The per-domain lane. DELIBERATELY NOT SHADOWED, along with
    /// [`Self::reject_domain`]: it has no production caller (the collector
    /// drives the whole-frame lane only), and the end-state pipeline holds one
    /// hour per phase for ALL domains at once, so a single moved slot is not a
    /// state it can represent. A capture that somehow used this lane would show
    /// the mismatch as `rs_shadow_illegal_transitions`, which is the honest
    /// answer rather than a fabricated per-domain custody.
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
        // SHADOW: the reader is done and the bytes are frozen, which is the end
        // state's OBSERVING -> PROCESSING handoff.
        self.shadow
            .note_read_custody_released((request.stamp.epoch, request.stamp.hour_raw));
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
        // SHADOW: the sink outcome landed, so the end state's SEALED ->
        // PUBLISHING -> COMMITTED tail runs here. Placed after `force_baseline`
        // so no live effect of this acceptance is still pending.
        self.shadow
            .note_publication_resolved((request.stamp.epoch, request.stamp.hour_raw), true);
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
        // SHADOW: a refusal still RESOLVES the hour, so custody ends here too.
        // Modelling it as a drop would double-report an hour the H4FR record
        // already names.
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
        // SHADOW: teardown. The unissued frontier is dropped on both sides.
        self.shadow.note_invalidated();
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
        // The hour the whole-frame lane evicted, sampled at slot zero for the
        // same reason `evicted_pending_hours` is: once per HOUR, not once per
        // domain. Handed to the shadow after the loop, never during it.
        let mut evicted: Option<(u64, i32)> = None;
        for index in 0..self.domains.len() {
            let domain = self.domains[index];
            if let Some(replaced) = self.domain_requests[index].latest_pending.replace(request) {
                if index == 0 {
                    // UNIT: HOURS. Counted once per hour, not once per domain
                    // (that is what `index == 0` is for), and ONLY here. This
                    // hour is never swept. It may still end up inside a
                    // FRAME_END: the Backpressure range accounting it is closed
                    // by `FRAME_END(range_end)`, so the last hour of a coalesced
                    // run carries a frame with no domain record in it.
                    //
                    // SCOPE, and the reconciliation trap it sets. This is the
                    // eviction site ONLY. Two other sites also raise
                    // `Backpressure` ranges and count themselves elsewhere:
                    // `flush_latest_pending` (-> `flushed_pending_hours`) and
                    // `gap_all_pending` (-> `abandoned_hours`). So
                    // `evicted_pending_hours` is a STRICT SUBSET of the hours
                    // the wire's Backpressure ranges cover, and comparing the
                    // two directly under-reads the wire by the other two sites'
                    // hours. See
                    // `backpressure_counters_reconcile_with_the_ranges_the_wire_carries`
                    // for the identity that relates all of them.
                    self.telemetry.evicted_pending_hours =
                        self.telemetry.evicted_pending_hours.saturating_add(1);
                    evicted = Some((replaced.stamp.epoch, replaced.stamp.hour_raw));
                }
                self.push_gap(domain, replaced.stamp, GapKind::Backpressure);
            }
            self.telemetry.max_request_depth = self.telemetry.max_request_depth.max(u32::from(
                self.domain_requests[index].latest_pending.is_some(),
            ));
        }
        // SHADOW, after every live effect of this admission is complete. This
        // is the hour edge as far as the end state is concerned: the frontier
        // exists and an edge-aligned dispatch would start the bracket here.
        self.shadow.note_frontier_admitted(
            (request.stamp.epoch, request.stamp.hour_raw),
            evicted,
        );
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
        // SHADOW: a clock-unknown, reload or teardown boundary. The end state
        // drops the same hours -- everything unsealed -- and spares the ones
        // whose bytes are already immutable, which is exactly what this
        // function does with `publishing`.
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
            // UNIT: DOMAIN-HOURS, i.e. hours x domains, and it is incremented
            // by `represented` (the range's covered hour count) on EVERY
            // Backpressure push regardless of which site raised it and
            // regardless of whether the push extends an existing range or
            // starts a new one.
            //
            // THIS IS NOT A RECORD COUNT, and the difference is not small. The
            // wire carries ONE `H4DG` record per domain per COALESCED RANGE
            // (`build_scheduler_gap_frames`), and a range covers
            // `range_end - range_start + 1` hours. In run t32 that is 73 ranges
            // x 68 domains = 4_964 records standing for 83 x 68 = 5_644
            // domain-hours. Comparing this counter with a record count is a
            // unit error, not a discrepancy; the hour count each record stands
            // for is on the wire in `range_start_hour_raw`/`range_end_hour_raw`
            // and is independently witnessed by `queue_dropped`.
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

    /// One scheduler with an explicit shadow mode and a MANUAL clock, so a
    /// shadow assertion is an exact number and never a wall-clock race.
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

    /// THE t32 BACKPRESSURE RECONCILIATION, as an identity rather than a note.
    ///
    /// # The question this settles
    ///
    /// Run t32's final scheduler probe published `rs_evicted_pending_hours =
    /// 81` and `rs_backpressure_domain_hours = 5_508`, while its stream carried
    /// 4_964 `Backpressure` `H4DG` records. Subtracting those gives 544, and
    /// 544 is not a quantity: the first two are HOURS and DOMAIN-HOURS and the
    /// third is a RECORD COUNT. Reduced to comparable units, t32 is
    ///
    /// ```text
    ///   4_964 records      = 73 coalesced ranges x 68 domains
    ///   5_644 domain-hours = 83 distinct hours   x 68 domains   (on the wire)
    ///   5_508 domain-hours = 81 distinct hours   x 68 domains   (in the probe)
    ///     136 domain-hours =  2 hours            x 68 domains   (the residual)
    /// ```
    ///
    /// and those 2 hours are IDENTIFIED on the wire: the last probe rode a
    /// frame at hour 60_832_837, exactly 81 of t32's 83 backpressure hours fall
    /// at or before it, and the remaining two — 60_832_841 and 60_832_845, 68
    /// records each — fall after it. The probe is a rider on a published frame,
    /// so it can only ever report the losses that had happened when it was
    /// written. No hour reached the stream without a cause; the WIRE is ahead of
    /// the probe, never behind it, and `game_hours_without_data.unaccounted = 0`
    /// in the same manifest agrees.
    ///
    /// NOT CLAIMED: that those two hours are the two the ledger showed as
    /// `outstanding` when the probe was taken (`scheduled 660 - accepted 576 -
    /// refused 1 - evicted 81 = 2`). Both counts are 2 and both are consistent
    /// with the residual, but nothing on the wire preserves a request's
    /// identity from the probe to its disposition, so a hole closed by a
    /// coincidence of arithmetic is not a hole closed (Codex review
    /// 019fd9b9-6318-7bd2-8882-a1c3927a19c5, finding 8). What IS established is
    /// the weaker, sufficient statement: the residual is the backpressure hours
    /// that resolved after the last probe.
    ///
    /// # What this test pins
    ///
    /// The three-way arithmetic on a scheduler driven through a comparable
    /// shape: a long coalesced eviction run, a teardown flush that EXTENDS that
    /// run rather than starting a new range, and a probe taken before that
    /// resolution. Teardown is ONE way an hour resolves after a probe; it is
    /// used here because it is the one a unit test can force, not because t32's
    /// two hours are known to have taken it.
    ///
    /// Production changes that fail it: counting `evicted_pending_hours` per
    /// domain instead of per hour; dropping the `record_gap_range` increment on
    /// the extend path so a coalesced range stops accruing domain-hours;
    /// removing `Backpressure` from `can_extend` so ranges stop coalescing and
    /// records silently become hours; or making `flush_latest_pending` raise
    /// something other than a `Backpressure` range, which would leave the
    /// teardown hour un-named on the wire.
    #[test]
    fn backpressure_counters_reconcile_with_the_ranges_the_wire_carries() {
        const DOMAINS: u64 = 3;
        let mut s = scheduler(&[10, 11, 12], 1, 64);

        // Hour 1 is admitted and taken: a reader now holds it.
        s.observe_stable(1, 1);
        let in_flight = s.take_current_request().expect("hour 1 is takeable");
        // Hours 2..=12 arrive while that reader is busy. Each new frontier
        // evicts the previous pending one, so hours 2..=11 are evicted (hour 12
        // survives as the pending request) into ONE coalesced range.
        for hour in 2..=12 {
            s.observe_stable(hour, hour as u64);
        }
        s.accept_request(in_flight);

        // --- The probe, taken here, with hour 12 still outstanding. ---
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

        // --- Teardown, which is what happens after the last probe. ---
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

        // THE RESIDUAL IDENTITY, which is the whole point: the domain-hours the
        // wire ends up carrying exceed the last probe's by exactly the hours
        // that resolved into Backpressure after that probe, times the roster.
        // In this scenario that set is the one hour left outstanding; in t32 it
        // was two hours the stream places after the probe's frame. The SHAPE of
        // the residual is the durable claim, not which requests filled it.
        let hours_resolved_after_probe = after_ledger.flushed_pending - probe_ledger.flushed_pending;
        assert_eq!(hours_resolved_after_probe, 1);
        assert_eq!(
            after.backpressure_domain_hours,
            probe.backpressure_domain_hours + hours_resolved_after_probe * DOMAINS,
            "a probe can only report the losses that had happened when it was \
             written; the wire keeps accruing after it"
        );

        // --- What the wire actually carries. ---
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

        // `build_scheduler_gap_frames` emits exactly one record per domain per
        // range, so this is the record count the stream will hold.
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

    /// TASK #38: the interval is PUBLISHED, on a lane that closes no hour.
    ///
    /// Counting it was never enough. The host's gap accounting demands a typed
    /// producer statement for every hour that put no data on the wire, and an
    /// outage the producer knew about, counted, and could describe exactly still
    /// arrived as silence -- so a capture that hit one clock outage was demoted
    /// over hours nobody was confused about.
    ///
    /// The queued interval carries the whole claim and its own substantiation:
    /// the range, the hours either side of it, and how many clock reads failed.
    /// It still does NOT become a control range -- the assertion above this one
    /// is unchanged and remains the reason this is a separate lane at all.
    ///
    /// MUTATION RUN: delete the `queue_clock_unreadable_interval` call from the
    /// `clock_unknown` branch of `observe_stable`. Observed failure:
    ///   `an outage must reach the wire as its own interval record: []`.
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
        // Draining is a handover, not a peek.
        assert!(!s.has_clock_unreadable_intervals());
        assert!(s.drain_clock_unreadable_intervals().next().is_none());

        // The control lane is still empty: this interval is deliberately NOT a
        // gap range, because a range would close its last hour with a FRAME_END.
        let gaps: Vec<_> = s.drain_control().collect();
        assert!(gaps.is_empty(), "{gaps:?}");
    }

    /// An outage that loses no hours mints no record.
    ///
    /// The wire grammar refuses an empty range, so emitting one here would be a
    /// producer writing a record its own reader rejects -- and it would also
    /// account for zero hours while looking like evidence.
    ///
    /// MUTATION RUN: drop the `range_start > range_end` early return from
    /// `queue_clock_unreadable_interval`. Observed failure:
    ///   `an outage that lost no hours must mint no record: [ClockUnreadable
    ///    Interval { range_start_hour_raw: 101, range_end_hour_raw: 100, .. }]`.
    #[test]
    fn an_outage_that_loses_no_hour_mints_no_interval() {
        let mut s = scheduler(&[10], 1, 64);
        s.observe_stable(100, 1);
        let baseline = s.take_current_request().expect("baseline hour is takeable");
        s.accept_request(baseline);

        s.note_clock_unreadable();
        // The very next hour reads fine: nothing elapsed unobserved.
        s.observe_stable(101, 2);
        let intervals: Vec<_> = s.drain_clock_unreadable_intervals().collect();
        assert!(
            intervals.is_empty(),
            "an outage that lost no hours must mint no record: {intervals:?}"
        );
        assert_eq!(s.hour_ledger().unobserved_clock_unknown, 0);
    }

    /// A FAILED CLOCK READ IS NOT A LICENCE. An outage whose implied span
    /// exceeds the trusted forward delta mints no VOUCHED interval.
    ///
    /// On the ordinary path a jump larger than `MAX_FORWARD_DELTA` is a
    /// terminal counter anomaly and is deliberately never converted into
    /// elapsed hours: the span rests on a value the scheduler has just
    /// rejected. The recovery branch runs BEFORE that check, so one failed read
    /// would otherwise turn the identical untrusted value into hundreds of hours
    /// the host then counts as accounted.
    ///
    /// What it mints INSTEAD is the subject of
    /// `an_untrusted_recovery_is_recorded_on_the_wire_and_is_not_terminal`.
    ///
    /// MUTATION RUN: change the `delta > MAX_FORWARD_DELTA` arm of
    /// `clock_recovery_trust` to return `ClockRecoveryTrust::Trusted`. Observed
    /// failure:
    ///   `an untrusted forward jump must not become accounted hours because one
    ///    clock read failed: [ClockUnreadableInterval { range_start_hour_raw:
    ///    101, range_end_hour_raw: 868, last_observed_hour_raw: 100,
    ///    resumed_hour_raw: 869, unreadable_events: 1 }]`.
    #[test]
    fn an_outage_wider_than_the_trusted_forward_delta_mints_no_interval() {
        let mut s = scheduler(&[10], 1, 64);
        s.observe_stable(100, 1);
        let baseline = s.take_current_request().expect("baseline hour is takeable");
        s.accept_request(baseline);

        s.note_clock_unreadable();
        // EXACTLY the first delta the ordinary path refuses. Comparing the
        // missing SPAN instead of the DELTA admitted this one value, because the
        // span is one smaller than the delta that produced it.
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
        // The interval-drop counter means ONE thing: an outage that reached no
        // wire record. This outage reaches one -- a different one -- so counting
        // it here would restate a refusal that is no longer a refusal.
        assert_eq!(
            s.telemetry().clock_unreadable_intervals_dropped,
            0,
            "an untrusted recovery is not an interval the producer failed to publish"
        );

        // The largest delta the ordinary path DOES trust still publishes, so the
        // refusal above is the ceiling and not the fixture.
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

    /// AN UNTRUSTED CLOCK JUMP IS ACCEPTED AND RECORDED.
    ///
    /// The owner ruled on 2026-08-05 that a recovery on a jump the ordinary
    /// path would refuse must NOT be terminal -- one failed clock read is far
    /// too weak a reason to destroy a capture -- but must not be silent either.
    /// Before this the resumed hour was scheduled with `terminal: None`, every
    /// anomaly counter at zero and (after the interval was refused) nothing on
    /// the wire at all, so the hours that followed were indistinguishable from
    /// hours reached through a clock read the scheduler believed.
    ///
    /// Three things are pinned here, and they are the three the ruling asked
    /// for: the capture SURVIVES, the anomaly is COUNTED, and the record
    /// carries the OBSERVED DELTA rather than a boolean.
    ///
    /// MUTATION RUN 1: delete the `queue_untrusted_clock_recovery` call from
    /// the `Untrusted` arm of `observe_stable`. Observed failure:
    ///   `assertion `left == right` failed: the untrusted jump must be counted
    ///     left: 0
    ///    right: 1`.
    ///
    /// MUTATION RUN 2: replace `observed_delta_hours` in
    /// `queue_untrusted_clock_recovery` with `1` (a boolean-grade "some jump").
    /// Observed failure:
    ///   `assertion `left == right` failed: the record must carry the ACTUAL
    ///    jump size, not a flag
    ///     left: 1
    ///    right: 769`.
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

        // ACCEPTED: the capture keeps running and the resumed hour is scheduled.
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

        // RECORDED: counted, and on the wire.
        assert_eq!(
            s.hour_ledger().untrusted_clock_recovery_events,
            1,
            "the untrusted jump must be counted"
        );
        let recoveries: Vec<_> = s.drain_untrusted_clock_recoveries().collect();
        assert_eq!(recoveries.len(), 1, "the untrusted jump must reach the wire");
        let recovery = recoveries[0];
        assert_eq!(recovery.last_observed_hour_raw, 100);
        assert_eq!(recovery.resumed_hour_raw, far);
        assert_eq!(
            recovery.unreadable_events, 2,
            "and it substantiates itself with its own failed reads"
        );
        assert_eq!(recovery.trust_ceiling_hours, MAX_FORWARD_DELTA);

        // THE OBSERVED DELTA, not a boolean. A jump a day past the ceiling and a
        // jump a decade past it are different facts about a capture.
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

    /// THE CEILING THE SCHEDULER APPLIES IS THE ONE THE RECORD'S READER
    /// REQUIRES.
    ///
    /// `H4CJ` carries its ceiling so the record substantiates its own
    /// classification, and its reader pins that value -- otherwise a producer
    /// could declare a ceiling of 1, call a two-hour outage untrusted, and
    /// escape the `H4CU` branch that ACCOUNTS for the hours. That pin only
    /// helps if the scheduler's own constant is the same one, so the two are
    /// literally the same constant and this says so.
    ///
    /// MUTATION RUN: restate `MAX_FORWARD_DELTA` as its own `24 * 32` literal
    /// and change one of them. Observed failure:
    ///   `assertion `left == right` failed: the scheduler's ceiling and the
    ///    record's pinned ceiling are one constant`.
    #[test]
    fn the_scheduler_ceiling_is_the_contract_ceiling() {
        assert_eq!(
            MAX_FORWARD_DELTA,
            compact_contract::untrusted_clock_recovery::
                UNTRUSTED_CLOCK_RECOVERY_TRUST_CEILING_HOURS,
            "the scheduler's ceiling and the record's pinned ceiling are one constant"
        );

        // And the scheduler's own recovery encodes under that pin, so a drift
        // would be caught by the producer refusing its own record rather than
        // by a consumer refusing it later. Real calendar hours here: the record
        // validates them, and a fixture the record refuses would prove nothing
        // about the ceiling.
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

    /// THE TWO RECOVERY RECORDS ARE DISJOINT, AND NEITHER FIRES ON A LOSSLESS
    /// OUTAGE.
    ///
    /// One decision, three arms, and each arm owns exactly one wire outcome.
    /// An outage that loses no hour mints NEITHER -- the invariant BC
    /// established and which this change must not break, because the wire
    /// grammar refuses an empty range and a disclaimer of nothing is not a fact.
    ///
    /// MUTATION RUN: change the `delta > MAX_FORWARD_DELTA` arm of
    /// `clock_recovery_trust` to return `ClockRecoveryTrust::Trusted`, so both
    /// branches claim the same outage. Observed failure, on the untrusted leg:
    ///   `assertion `left == right` failed
    ///     left: 1
    ///    right: 0`
    /// -- an interval was minted for a jump only the recovery may describe.
    ///
    /// NOT proven by weakening the `delta <= 1` arm to `delta <= 0`: that arm is
    /// an EARLY-OUT, not the guarantee. A lossless outage that reaches the
    /// `Trusted` branch still mints nothing, because `count_unobserved_span` and
    /// `queue_clock_unreadable_interval` each refuse an empty range on their own
    /// -- which is what `an_outage_that_loses_no_hour_mints_no_interval` pins.
    /// The battery ran that mutation and the suite stayed green; it is recorded
    /// here rather than dressed up as a kill.
    #[test]
    fn the_two_recovery_records_are_disjoint_and_neither_fires_on_a_lossless_outage() {
        // Lossless: the same hour, and the very next one.
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

        // Trusted: the interval, and only the interval.
        let mut s = scheduler(&[10], 1, 64);
        s.observe_stable(100, 1);
        let baseline = s.take_current_request().expect("baseline is takeable");
        s.accept_request(baseline);
        s.note_clock_unreadable();
        s.observe_stable(105, 2);
        assert_eq!(s.drain_clock_unreadable_intervals().count(), 1);
        assert_eq!(s.drain_untrusted_clock_recoveries().count(), 0);

        // Untrusted: the recovery, and only the recovery.
        let mut s = scheduler(&[10], 1, 64);
        s.observe_stable(100, 1);
        let baseline = s.take_current_request().expect("baseline is takeable");
        s.accept_request(baseline);
        s.note_clock_unreadable();
        s.observe_stable(100 + MAX_FORWARD_DELTA as i32 + 1, 2);
        assert_eq!(s.drain_clock_unreadable_intervals().count(), 0);
        assert_eq!(s.drain_untrusted_clock_recoveries().count(), 1);
    }

    /// THE QUEUE MUST NOT REFUSE AN HONEST CAPTURE.
    ///
    /// The observer mints an interval on its own poll; the capture loop drains
    /// them only when it returns to the top of a sweep. Nothing in the tree
    /// bounds outages-per-sweep, so a small ceiling was reachable by an ordinary
    /// run -- and a dropped interval is not a lost diagnostic, it is hours with
    /// no producer statement at all, which fails the host's gap gate.
    ///
    /// The ceiling now matches the control lane's, which no bounded capture can
    /// reach: the number of outages is bounded by the hours of the run.
    ///
    /// MUTATION RUN: set `CLOCK_UNREADABLE_INTERVAL_QUEUE_CAP` back to 256.
    /// Observed failure:
    ///   `assertion `left == right` failed: an ordinary run's outages must all
    ///    reach the wire
    ///     left: 256
    ///    right: 300`.
    #[test]
    fn many_undrained_outages_all_survive_to_be_published() {
        const OUTAGES: usize = 300;
        let mut s = scheduler(&[10], 1, 64);
        s.observe_stable(100, 1);
        let baseline = s.take_current_request().expect("baseline hour is takeable");
        s.accept_request(baseline);

        // Each recovery skips exactly one hour, so every interval is a valid,
        // disjoint, single-hour outage -- and NOTHING drains between them.
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

    /// Each outage counts its OWN failed reads. A per-capture total would make
    /// the second interval claim the first one's evidence.
    ///
    /// MUTATION RUN: delete `self.clock_unknown_events = 0;` from the recovery
    /// branch of `observe_stable`. Observed failure:
    ///   `assertion `left == right` failed: the second outage must not inherit
    ///    the first one's failed reads
    ///     left: 3
    ///    right: 1`.
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

    // --- capture-floor stage S3: the custody shadow -----------------------

    /// One step of the script the non-interference test replays.
    ///
    /// Deliberately INPUT ONLY: no step depends on what an earlier step
    /// returned, so the three schedulers below are driven identically even if
    /// one of them were perturbed. If the script branched on outputs, a
    /// perturbed scheduler would diverge into a different script and the
    /// comparison would be of two different experiments.
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

    /// Everything a step can be observed to produce. This is the comparison
    /// surface; the shadow's own summary is deliberately NOT in it.
    #[derive(Clone, Debug, PartialEq, Eq)]
    enum StepOutput {
        Decision(SchedulerDecision),
        Request(Option<CaptureRequest>),
        Count(usize),
        None,
    }

    /// A deterministic script of WELL-FORMED frames. The LCG is a generator of
    /// INPUTS, nothing more.
    ///
    /// Well-formed matters. The live protocol is `observe -> take -> [release]
    /// -> accept|reject`, and violating it is TERMINAL by design: resolving a
    /// request the scheduler does not hold, or crossing a clock boundary
    /// between the take and the resolution, sets `CounterAnomaly` and stops the
    /// scheduler admitting anything ever again. A script that wandered into
    /// that on step nine would compare three schedulers that had all been dead
    /// since step nine, and would pass while proving nothing -- which is what
    /// the vacuity assertions at the end of the test exist to catch.
    ///
    /// So the boundary decorations (an extra frontier that will be evicted, a
    /// clock outage, a teardown flush, an idle decide) are emitted BETWEEN
    /// frames, and the hour motion -- forward, multi-hour, and backwards into a
    /// reload -- happens there too. Every loss path the scheduler has is still
    /// reachable; none of them is reachable in the middle of a frame's custody.
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
                    // A frontier no reader will ever take: the next observation
                    // evicts it, which is the backpressure path.
                    hour += 3;
                    script.push(Step::Observe(hour));
                }
                2 => script.push(Step::FlushPending),
                3 => script.push(Step::Decide),
                _ => {}
            }
            // Mostly forward, sometimes a multi-hour overrun, sometimes a
            // reload. Never more than the trust ceiling, which is terminal.
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

    /// Replay a script and return every observable output, in order.
    ///
    /// `Accept`/`Reject` need a request to resolve, and the only honest source
    /// of one is the scheduler itself -- so the replay resolves the request
    /// that scheduler last handed it, and resolves it once. That IS an output
    /// dependency, and it is safe for exactly one reason: every take and every
    /// release is RECORDED, so a scheduler that handed out a different request
    /// (or none) fails the output comparison directly. The dependency can hide
    /// nothing that the comparison would otherwise have seen.
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

    /// The shadow's summary is the ONE field of telemetry that is allowed to
    /// differ between a shadow-on and a shadow-off scheduler. Everything else
    /// must be identical, so it is compared with that field cleared.
    fn live_half(mut telemetry: SchedulerTelemetry) -> SchedulerTelemetry {
        telemetry.custody_shadow = CustodyShadowSummary::default();
        telemetry
    }

    /// NON-INTERFERENCE OF DECISIONS, as a forced comparison rather than an
    /// argument.
    ///
    /// SCOPE, stated first because the test name would otherwise overclaim:
    /// this is a LOGICAL comparison. It proves that the same inputs produce the
    /// same decisions, telemetry, ledgers and control ranges whatever the
    /// shadow does. It says nothing about wall-clock -- the notifications are
    /// synchronous and do occupy the thread, and the measured 316-588 ns per
    /// frame is the whole of that cost (`custody_shadow`'s
    /// `shadow_cost_per_frame_is_negligible`). No test here has a concurrency
    /// or elapsed-time dimension, and none claims one.
    ///
    /// The same script is replayed against three schedulers:
    ///
    /// * one with the shadow OFF,
    /// * one with the shadow ON,
    /// * one with the shadow ON and HOSTILE -- every notification wipes the
    ///   machine, multiplies its own counters and clears its rings, inside the
    ///   same critical section the live decisions are taken in.
    ///
    /// All three must produce the identical sequence of decisions and requests,
    /// the identical live telemetry, the identical hour ledger and the identical
    /// control ranges. The hostile arm is what makes this a proof about the
    /// SCHEDULER rather than about this particular shadow implementation: any
    /// live read of shadow state at all would show up as a difference here.
    ///
    /// MUTATION: make `take_current_request` return `None` when
    /// `self.shadow.summary().illegal_transitions > 0`, and the hostile arm
    /// diverges on the first refusal.
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

        // VACUITY GUARDS. Three schedulers that all went terminal on step nine
        // would agree perfectly and prove nothing, so the run has to be shown
        // to have done the work: hundreds of dispatches, hours that reached the
        // sink, hours that were lost, and a scheduler still alive at the end.
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

    /// The shadow follows a clean pipelined capture and says the end state
    /// would have committed every hour.
    ///
    /// MUTATION: notify the shadow of the dispatch BEFORE the live slots are
    /// filled and nothing here changes -- which is why the non-interference
    /// test above, not this one, is what pins the ordering. What this pins is
    /// TRUTHFULNESS: the counters describe the hours that actually happened.
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
        // The live ledger is untouched by any of it.
        assert_eq!(s.hour_ledger().accepted, 10);
        assert!(s.hour_ledger().balanced());
    }

    /// A live eviction run -- the shape t32 measured -- reads on the shadow as
    /// the end state dropping the same hours, agreeing with the live eviction
    /// every time.
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
        // The live capture published hour 1 anyway. That disagreement is the
        // measurement, and it is named on both sides.
        assert_eq!(summary.live_kept_hours_shadow_dropped, 1);
        assert_eq!(summary.committed_hours, 0);
        assert_eq!(summary.illegal_transitions, 0);
        let ledger = s.custody_shadow().ledger();
        assert!(ledger.balanced(), "{ledger:?}");
    }

    /// THE ONE LIVE ORDERING THAT REACHES `ProcessingBackpressure`, and the
    /// reason it is rare.
    ///
    /// A snapshot is dropped when a NEW one needs the processing slot, i.e.
    /// when a second frame's read finishes before the first frame's sink
    /// outcome lands. The live scheduler makes two unresolved publications
    /// terminal, so the only way to reach it is the mixed path: hour 500 is
    /// released into publication custody, hour 501 is taken and published
    /// SYNCHRONOUSLY (which closes 501's observation while 500 is still
    /// unsealed), and only then does 500 resolve.
    ///
    /// This is also why `rs_shadow_processing_backpressure_hours` is expected
    /// to be ~0 on ordinary captures: today's sweep serialises read and publish,
    /// so the compute custody the end state introduces is not the constraint --
    /// observation lateness is. That expectation is a RESULT of the stage, and
    /// this test is what makes the counter's zero mean something.
    #[test]
    fn the_mixed_live_publication_order_reaches_processing_backpressure() {
        let mut s = scheduler_with_shadow(&[10], 1, 64, ShadowMode::Enabled);
        s.observe_stable(500, 1);
        let first = s.take_current_request().expect("hour 500 is takeable");
        s.release_read_custody(first);
        s.observe_stable(501, 2);
        let second = s.take_current_request().expect("the successor is claimable");
        // Hour 501 publishes synchronously while 500 is still in publication
        // custody. The live scheduler allows exactly this.
        s.accept_request(second);
        let summary = s.telemetry().custody_shadow;
        assert_eq!(
            summary.processing_backpressure_hours, 1,
            "closing 501 found 500 still unsealed"
        );
        assert_eq!(summary.collapsed_publications, 1, "501 never released");
        assert_eq!(summary.committed_hours, 1, "501");
        assert_eq!(summary.illegal_transitions, 0);

        // And when 500 finally resolves, the shadow says it would have been
        // gone -- once, not as a second backpressure.
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

    /// The ORDINARY slow-frame shape, for contrast: three hours pass while one
    /// frame is unresolved. Every unopened hour misses its deadline, and the
    /// held snapshot is NOT dropped -- nothing needed its slot.
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
        assert_eq!(summary.committed_hours, 1, "hour 600 commits in the shadow too");
        assert_eq!(summary.live_kept_hours_shadow_dropped, 0);
        assert_eq!(summary.illegal_transitions, 0);
        let ledger = s.custody_shadow().ledger();
        assert!(ledger.balanced(), "{ledger:?}");
    }

    /// A clock outage and a reload both invalidate shadow custody, and the
    /// shadow accepts the lower hour a reload brings instead of refusing it as
    /// backwards motion.
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
        // A reload: the hour goes backwards.
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

    /// The synchronous publication path -- accept without releasing read
    /// custody -- is named as a collapse rather than modelled as a pipeline the
    /// recorder does not have.
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
