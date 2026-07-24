//! Zero-silent-gap hour latch — pure accounting state machine (lane/hour-latch).
//!
//! # The problem this solves
//!
//! The collector samples a **free-running** game (read-only; extraction does NOT
//! gate the sim). The heavy per-domain decode sweep costs ~100-150 ms on deep-war
//! worlds, but the sim's *local* inter-hour interval is bursty: measured p50
//! 129.5 ms, **min 28.2 ms** (~35 game-hours/sec bursts), p1 33 ms. A synchronous
//! sample loop that runs the heavy sweep inline cannot notice hours that elapse
//! *during* a sweep, so any game-hour that begins and ends inside one sweep is
//! silently skipped. The 2026-07-09 deep-war run skipped **370 of 1790 advanced
//! game-hours (20.7 %)** this way with no record that they ever happened.
//!
//! # What this module is
//!
//! `HourLatch` is the **pure, single-threaded accounting core** of a decoupled
//! architecture:
//!
//! * a tiny **latch thread** polls ONLY the hour counter (`gs+0x468`) at a period
//!   well under the 28.2 ms min burst interval (default 2 ms) and calls
//!   [`HourLatch::observe`] on every advance — this is the SOLE authority for the
//!   observed-hour frontier; it reads the counter VALUE and folds in the full
//!   delta (`last+1..=current`), not edges, so a multi-hour jump between two polls
//!   cannot lose an hour's *existence*;
//! * the **heavy sweep** (the existing per-domain decode) calls
//!   [`HourLatch::take_capture`] whenever it is ready to read the world; the latch
//!   hands it the current frontier hour to attach and marks any older still-pending
//!   observed hours as coalesced. The sweep NEVER feeds its own counter read into
//!   the epoch/frontier logic (that would let a stale sweep read fabricate a
//!   reload — see the `stale_sweep_read` test).
//!
//! # Losslessness — precise, honestly scoped (load-bearing for the whole program)
//!
//! **This latch provides ZERO-SILENT-GAP ACCOUNTING, not zero data loss.** An
//! async sampler of a sim it may not stop cannot record the exact instantaneous
//! world-state at each hour boundary: the sim advances while we read, so every
//! observation carries a bounded read-skew, and intra-hour transients (a value
//! that changes and reverts inside one game-hour — e.g. an order issued and
//! cancelled) are unobservable to an *hourly* sampler at all (a finer or
//! event-driven sampler could catch them; that is out of scope). What the latch
//! guarantees, within an explicitly delimited capture interval and assuming a
//! valid monotonic hour counter, is:
//!
//! 1. **COVERAGE (zero silent gaps).** Every game-hour the sim passes through is
//!    *observed and accounted* as exactly one of: (a) a capture ATTEMPT (a heavy
//!    sweep was run for it), which the runtime then records as a status-complete
//!    frame emitted to the sink path or leaves as a recorded failed attempt; or
//!    (b) a COALESCED hour with a specific reason
//!    (superseded by a later capture / ring overflow / end-of-session flush /
//!    reload-epoch close / anomalous-jump cap). Formally
//!    `observed == attempts + coalesced + in_flight` holds after every operation,
//!    and `in_flight == 0` after [`HourLatch::flush`]. No hour is ever *silently*
//!    unaccounted.
//! 2. **BOUNDED, NON-REGRESSED SKEW.** The emitted snapshot is labeled with the
//!    exact frontier hour consumed by `take_capture`; the sweep then reads the
//!    current world. The latch removes sample-loop dead-time (sweeps run
//!    back-to-back), which usually *tightens* skew, but it also adds a small
//!    mutex/decision latency before the sweep starts, so the skew is **different
//!    — usually better, occasionally marginally worse — not provably
//!    monotonically better.** It is bounded by the sweep duration plus that
//!    latency, the same order as today.
//!
//! **A COALESCED hour IS data loss of that hour's per-hour world state** — the
//! latch does not pretend otherwise; it makes the loss explicit, reason-tagged,
//! and bounded instead of silent. Under a burst that outruns the sweep throughput
//! the intermediate hours are physically unrecoverable (we cannot rewind the sim),
//! so they are coalesced. Giving every hour its own faithful snapshot is the job
//! of a *faster sweep* (Phase 2/4), not of this latch. The latch's contribution
//! is: (a) convert the 20.7 % *silently-skipped* hours into exact, reason-tagged
//! accounting; (b) remove dead-time so the sweep captures as many hours as its
//! throughput allows; and (c) be the architectural guarantee that, once the sweep
//! is fast enough, *every* hour boundary triggers a capture.

use std::collections::VecDeque;

/// Hard cap on how many hours a single `observe` may enqueue, so a torn/garbage
/// counter read (which would otherwise look like a giant forward jump) cannot
/// blow up memory. Excess hours beyond this are accounted as coalesced
/// immediately and flagged via `coalesced_anomalous`. 24*32 = 32 game-days, far
/// above any real single-poll advance (min real interval 28.2 ms >> 2 ms poll).
const MAX_ENQUEUE_PER_OBSERVE: i64 = 24 * 32;

/// What the heavy sweep should do, returned by [`HourLatch::take_capture`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureDecision {
    /// No new hour since the last capture — the sweep should idle/wait for the
    /// latch to signal a new hour rather than burning a redundant sweep.
    Idle,
    /// A heavy sweep was authorized for `hour` (the current observed frontier).
    /// `coalesced` older observed hours were superseded by it. The runtime MUST
    /// call [`HourLatch::record_complete_frame_emitted_to_sink`] iff the sweep
    /// produced a complete frame and handed it to the collector sink path;
    /// otherwise the attempt stands as recorded-failed
    /// (`attempts - emitted_to_sink`). The sink API does not report durable
    /// write success, so this deliberately makes no sink-commit claim.
    Capture { hour: i32, coalesced: u32 },
}

/// Immutable snapshot of the latch's accounting counters for telemetry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HourLatchTelemetry {
    /// Total game-hours the sim passed through that the latch observed.
    pub observed: u64,
    /// Capture ATTEMPTS: heavy sweeps authorized (one per fresh frontier hour).
    /// NOT a completeness claim — see `emitted_to_sink`.
    pub attempts: u64,
    /// Attempts that produced a status-complete frame and handed it to the
    /// collector sink path. The collector-wide sink API swallows write errors,
    /// so this is NOT a durable-write confirmation.
    /// `attempts - emitted_to_sink` = recorded-failed captures.
    pub emitted_to_sink: u64,
    /// Observed hours that did NOT get their own capture attempt, by reason.
    /// `coalesced == superseded + overflow + flush + backward + anomalous`.
    pub coalesced: u64,
    /// Coalesced because a later capture superseded them (the common case).
    pub coalesced_superseded: u64,
    /// Coalesced because the pending ring overflowed (sustained overrun).
    pub coalesced_overflow: u64,
    /// Coalesced at end-of-session flush (observed but never swept).
    pub coalesced_flush: u64,
    /// Coalesced because a backward counter jump (reload) closed the epoch.
    pub coalesced_backward: u64,
    /// Coalesced because an implausible forward jump was capped (suspected torn read).
    pub coalesced_anomalous: u64,
    /// Observed hours currently queued, neither attempted nor coalesced yet.
    pub in_flight: u64,
    /// High-water mark of the pending ring depth.
    pub max_queue_depth: u32,
    /// Configured ring capacity (in hours).
    pub queue_cap: u32,
    /// Times the counter went backwards (save reload / new game): a coverage
    /// epoch boundary, never a silent event.
    pub backward_jumps: u64,
    /// Times a single `observe` saw an implausibly large forward jump (capped by
    /// `MAX_ENQUEUE_PER_OBSERVE`) — suspected torn/garbage read.
    pub anomalous_jumps: u64,
    /// Monotonic generation, bumped whenever the observed frontier advances. The
    /// thread wrapper uses it as a condvar predicate (lost-wakeup free).
    pub generation: u64,
    /// Latch-thread poll count (set by the thread wrapper, not the core).
    pub polls: u64,
    /// Latch-thread failed counter reads (set by the thread wrapper).
    pub read_failures: u64,
    /// Whether the most recent observation attempt failed. A true terminal
    /// value means the run can only claim accounting through the last good
    /// observation; never fabricate hours that may have advanced afterwards.
    pub last_observe_failed: bool,
    /// The latch was requested but no runtime became active (for example, the
    /// latch thread failed to spawn). The collector falls back to the legacy
    /// loop, but the accounting claim must be INACTIVE rather than COMPLETE.
    pub latch_requested_but_inactive: bool,
    /// Hour frontiers for which the in-process capture barrier was acquired.
    /// The barrier temporarily freezes the simulation while the corresponding
    /// world frame is read; it does not change the recorded player pause state.
    pub capture_barrier_acquisitions: u64,
    /// Capture-barrier acquisitions that could not authenticate or freeze the
    /// exact-build pause clock. Any non-zero value invalidates lossless capture.
    pub capture_barrier_failures: u64,
    /// Capture barriers released after their world frame was fully assembled.
    pub capture_barrier_releases: u64,
    /// Barrier releases whose exact prior pause state could not be restored.
    pub capture_barrier_restore_failures: u64,
    /// Whether the exact-build pause clock was authenticated for this latch.
    pub capture_barrier_supported: bool,
    /// Whether a capture barrier remained owned at the telemetry boundary.
    pub capture_barrier_active: bool,
}

impl HourLatchTelemetry {
    /// The coverage accounting invariant. Must ALWAYS hold. Uses checked math so a
    /// (physically impossible) counter saturation surfaces as `false`, never a
    /// silent wrong `true`.
    pub fn invariant_holds(&self) -> bool {
        let rhs = self
            .attempts
            .checked_add(self.coalesced)
            .and_then(|v| v.checked_add(self.in_flight));
        let reasons = [
            self.coalesced_superseded,
            self.coalesced_overflow,
            self.coalesced_flush,
            self.coalesced_backward,
            self.coalesced_anomalous,
        ]
        .into_iter()
        .try_fold(0u64, |acc, v| acc.checked_add(v));
        rhs == Some(self.observed)
            && reasons == Some(self.coalesced)
            && self.emitted_to_sink <= self.attempts
    }

    /// Capture attempts that did not produce a status-complete frame handed to
    /// the sink path — a distinct, explicitly-counted loss class.
    pub fn capture_failed(&self) -> u64 {
        self.attempts.saturating_sub(self.emitted_to_sink)
    }

    /// Honest accounting claim carried into the manifest. The arithmetic
    /// invariant remains meaningful for the observations that did succeed,
    /// including a terminally truncated run.
    pub fn accounting_claim(&self) -> &'static str {
        if self.latch_requested_but_inactive {
            "INACTIVE"
        } else if self.last_observe_failed {
            "TRUNCATED-AT-LAST-GOOD-OBSERVE"
        } else {
            "COMPLETE"
        }
    }

    /// Telemetry used when latch activation was requested but failed. Zero
    /// counters are not presented as a successful zero-hour accounting run.
    pub fn requested_but_inactive(queue_cap: usize) -> Self {
        Self {
            queue_cap: queue_cap.clamp(1, 65_535) as u32,
            latch_requested_but_inactive: true,
            ..Self::default()
        }
    }
}

/// Pure hour-boundary latch accounting core. Single-threaded; the thread wrapper
/// guards it with a mutex and drives `observe` (latch thread) and `take_capture`
/// / `record_complete_frame_emitted_to_sink` / `flush` (sweep thread).
#[derive(Debug)]
pub struct HourLatch {
    /// Last hour-counter value folded into the accounting, or `None` before the
    /// first observation / after a reload reset. Written ONLY by `observe` (the
    /// latch thread) — never by the sweep.
    last_raw: Option<i32>,
    /// Observed-but-unresolved hours, oldest at the front. Bounded by `queue_cap`.
    pending: VecDeque<i32>,
    queue_cap: usize,
    observed: u64,
    attempts: u64,
    emitted_to_sink: u64,
    coalesced_superseded: u64,
    coalesced_overflow: u64,
    coalesced_flush: u64,
    coalesced_backward: u64,
    coalesced_anomalous: u64,
    max_queue_depth: u32,
    backward_jumps: u64,
    anomalous_jumps: u64,
    generation: u64,
    polls: u64,
    read_failures: u64,
    last_observe_failed: bool,
}

impl HourLatch {
    /// `queue_cap` = maximum pending-hour backlog before the oldest pending hours
    /// are coalesced to make room (bounds memory AND the capture lag). Clamped to
    /// `[1, 65_535]`.
    pub fn new(queue_cap: usize) -> Self {
        let queue_cap = queue_cap.clamp(1, 65_535);
        Self {
            last_raw: None,
            pending: VecDeque::with_capacity(queue_cap.min(1024)),
            queue_cap,
            observed: 0,
            attempts: 0,
            emitted_to_sink: 0,
            coalesced_superseded: 0,
            coalesced_overflow: 0,
            coalesced_flush: 0,
            coalesced_backward: 0,
            coalesced_anomalous: 0,
            max_queue_depth: 0,
            backward_jumps: 0,
            anomalous_jumps: 0,
            generation: 0,
            polls: 0,
            read_failures: 0,
            last_observe_failed: false,
        }
    }

    /// Record that the latch thread performed one poll of the counter (whether or
    /// not it advanced). Pure telemetry.
    pub fn note_poll(&mut self) {
        self.polls = self.polls.saturating_add(1);
    }

    /// Record that a counter read failed (unreadable / source unavailable).
    pub fn note_read_failure(&mut self) {
        self.read_failures = self.read_failures.saturating_add(1);
        self.last_observe_failed = true;
    }

    /// The observed-frontier generation. Bumps on every advancing `observe`; used
    /// by the thread wrapper as a condvar predicate.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Fold a freshly-read STABLE counter value into the accounting. This is the
    /// SOLE authority for the observed frontier and epoch transitions — call it
    /// ONLY from the latch thread with a torn-read-guarded (`read_i32_stable`)
    /// value, never from the sweep with an ordinary read.
    pub fn observe(&mut self, raw: i32) {
        self.last_observe_failed = false;
        let Some(last) = self.last_raw else {
            // First observation establishes the baseline hour, which itself needs
            // a capture.
            self.last_raw = Some(raw);
            self.observed = self.observed.saturating_add(1);
            self.generation = self.generation.saturating_add(1);
            self.enqueue(raw);
            return;
        };
        if raw == last {
            return;
        }
        let delta = raw as i64 - last as i64;
        if delta < 0 {
            // Counter went backwards: save reload / new game. Close the current
            // coverage epoch (any unresolved pending hours are from the old game
            // and will never be captured -> coalesce them as `backward`), then
            // reset and treat `raw` as a fresh baseline hour.
            self.backward_jumps = self.backward_jumps.saturating_add(1);
            self.coalesce_all_pending(CoalesceReason::Backward);
            self.last_raw = Some(raw);
            self.observed = self.observed.saturating_add(1);
            self.generation = self.generation.saturating_add(1);
            self.enqueue(raw);
            return;
        }
        // Forward advance. Fold in EVERY intervening hour so a multi-hour jump
        // (or a slow poll) can never miss an hour's existence.
        let hours = delta; // number of new hours (last+1 ..= raw)
        if hours > MAX_ENQUEUE_PER_OBSERVE {
            // Implausible jump — suspect a torn/garbage read. Count the hours as
            // observed (advance coverage honestly) but do not enqueue the whole
            // range: enqueue only the newest and coalesce the rest (`anomalous`)
            // so a bad read cannot OOM the ring.
            self.anomalous_jumps = self.anomalous_jumps.saturating_add(1);
            let coalesced = (hours - 1) as u64;
            self.observed = self.observed.saturating_add(hours as u64);
            self.coalesced_anomalous = self.coalesced_anomalous.saturating_add(coalesced);
            self.last_raw = Some(raw);
            self.generation = self.generation.saturating_add(1);
            self.enqueue(raw);
            return;
        }
        self.observed = self.observed.saturating_add(hours as u64);
        self.generation = self.generation.saturating_add(1);
        let mut hour = last;
        let mut remaining = hours;
        while remaining > 0 {
            hour = hour.wrapping_add(1);
            self.enqueue(hour);
            remaining -= 1;
        }
        self.last_raw = Some(raw);
    }

    /// Push one observed hour onto the pending ring, coalescing the oldest if the
    /// ring is full (bounds memory and capture lag; the drop is accounted).
    fn enqueue(&mut self, hour: i32) {
        if self.pending.len() >= self.queue_cap {
            // Overflow: drop the OLDEST pending hour (most stale, least useful)
            // and account it as coalesced-by-overflow. Never silent.
            if self.pending.pop_front().is_some() {
                self.coalesced_overflow = self.coalesced_overflow.saturating_add(1);
            }
        }
        self.pending.push_back(hour);
        let depth = self.pending.len() as u32;
        if depth > self.max_queue_depth {
            self.max_queue_depth = depth;
        }
    }

    fn coalesce_all_pending(&mut self, reason: CoalesceReason) {
        let n = self.pending.len() as u64;
        self.pending.clear();
        self.add_coalesced(reason, n);
    }

    fn add_coalesced(&mut self, reason: CoalesceReason, n: u64) {
        let slot = match reason {
            CoalesceReason::Superseded => &mut self.coalesced_superseded,
            CoalesceReason::Flush => &mut self.coalesced_flush,
            CoalesceReason::Backward => &mut self.coalesced_backward,
        };
        *slot = slot.saturating_add(n);
    }

    /// Called by the heavy sweep when it is ready to read the world. Uses ONLY the
    /// latch's own observed frontier (never a sweep-side counter read), so a stale
    /// sweep read can neither fabricate a reload nor duplicate a captured hour.
    ///
    /// Captures the current frontier hour (the newest pending) and coalesces any
    /// older pending hours as `superseded`. Returns `Idle` when no fresh hour is
    /// available (nothing pending) so the sweep does not burn a redundant pass.
    ///
    /// This only records a capture ATTEMPT; the runtime must call
    /// [`HourLatch::record_complete_frame_emitted_to_sink`] iff the sweep
    /// produced a complete frame and handed it to the sink path.
    pub fn take_capture(&mut self) -> CaptureDecision {
        let Some(&frontier) = self.pending.back() else {
            return CaptureDecision::Idle;
        };
        // Coalesce every pending hour older than the frontier (all but the last).
        let mut superseded: u32 = 0;
        while self.pending.len() > 1 {
            self.pending.pop_front();
            superseded = superseded.saturating_add(1);
        }
        // Consume the frontier itself as the capture attempt.
        self.pending.pop_front();
        self.add_coalesced(CoalesceReason::Superseded, superseded as u64);
        self.attempts = self.attempts.saturating_add(1);
        CaptureDecision::Capture {
            hour: frontier,
            coalesced: superseded,
        }
    }

    /// Record that the most recent [`take_capture`] attempt produced a
    /// status-complete frame and was handed to the collector sink path. The sink
    /// API does not return write success, so this is intentionally not named or
    /// documented as durable confirmation.
    pub fn record_complete_frame_emitted_to_sink(&mut self) {
        // Never let emitted exceed attempts (would break the invariant).
        if self.emitted_to_sink < self.attempts {
            self.emitted_to_sink = self.emitted_to_sink.saturating_add(1);
        }
    }

    /// End-of-session flush: any pending hours never got a sweep and are now
    /// coalesced (`flush`). After this, `in_flight == 0` and
    /// `observed == attempts + coalesced`.
    pub fn flush(&mut self) {
        self.coalesce_all_pending(CoalesceReason::Flush);
    }

    /// Current accounting snapshot for telemetry.
    pub fn telemetry(&self) -> HourLatchTelemetry {
        let coalesced = self
            .coalesced_superseded
            .saturating_add(self.coalesced_overflow)
            .saturating_add(self.coalesced_flush)
            .saturating_add(self.coalesced_backward)
            .saturating_add(self.coalesced_anomalous);
        HourLatchTelemetry {
            observed: self.observed,
            attempts: self.attempts,
            emitted_to_sink: self.emitted_to_sink,
            coalesced,
            coalesced_superseded: self.coalesced_superseded,
            coalesced_overflow: self.coalesced_overflow,
            coalesced_flush: self.coalesced_flush,
            coalesced_backward: self.coalesced_backward,
            coalesced_anomalous: self.coalesced_anomalous,
            in_flight: self.pending.len() as u64,
            max_queue_depth: self.max_queue_depth,
            queue_cap: self.queue_cap as u32,
            backward_jumps: self.backward_jumps,
            anomalous_jumps: self.anomalous_jumps,
            generation: self.generation,
            polls: self.polls,
            read_failures: self.read_failures,
            last_observe_failed: self.last_observe_failed,
            latch_requested_but_inactive: false,
            capture_barrier_acquisitions: 0,
            capture_barrier_failures: 0,
            capture_barrier_releases: 0,
            capture_barrier_restore_failures: 0,
            capture_barrier_supported: false,
            capture_barrier_active: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum CoalesceReason {
    Superseded,
    Flush,
    Backward,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_invariant(latch: &HourLatch) {
        let t = latch.telemetry();
        assert!(t.invariant_holds(), "accounting invariant violated: {t:?}",);
    }

    /// Drive a capture the way the runtime does: take + complete-frame emit.
    fn capture_ok(latch: &mut HourLatch) -> CaptureDecision {
        let d = latch.take_capture();
        if matches!(d, CaptureDecision::Capture { .. }) {
            latch.record_complete_frame_emitted_to_sink();
        }
        d
    }

    #[test]
    fn first_hour_is_observed_and_capturable() {
        let mut latch = HourLatch::new(64);
        latch.observe(100);
        assert_invariant(&latch);
        assert_eq!(
            capture_ok(&mut latch),
            CaptureDecision::Capture {
                hour: 100,
                coalesced: 0
            }
        );
        let t = latch.telemetry();
        assert_eq!(t.observed, 1);
        assert_eq!(t.attempts, 1);
        assert_eq!(t.emitted_to_sink, 1);
        assert_eq!(t.coalesced, 0);
        assert_invariant(&latch);
    }

    #[test]
    fn steady_state_every_hour_captured_no_coalesce() {
        let mut latch = HourLatch::new(64);
        for h in 0..1000i32 {
            latch.observe(h);
            let d = capture_ok(&mut latch);
            assert_eq!(
                d,
                CaptureDecision::Capture {
                    hour: h,
                    coalesced: 0
                }
            );
            assert_invariant(&latch);
        }
        let t = latch.telemetry();
        assert_eq!(t.observed, 1000);
        assert_eq!(t.attempts, 1000);
        assert_eq!(t.emitted_to_sink, 1000);
        assert_eq!(t.coalesced, 0);
        assert_eq!(t.in_flight, 0);
    }

    #[test]
    fn redundant_capture_without_new_hour_is_idle() {
        let mut latch = HourLatch::new(64);
        latch.observe(5);
        assert!(matches!(
            capture_ok(&mut latch),
            CaptureDecision::Capture { .. }
        ));
        assert_eq!(latch.take_capture(), CaptureDecision::Idle);
        let t = latch.telemetry();
        assert_eq!(t.attempts, 1);
        assert_eq!(t.observed, 1);
        assert_invariant(&latch);
    }

    #[test]
    fn burst_during_sweep_coalesces_intervening_hours_as_superseded() {
        let mut latch = HourLatch::new(64);
        latch.observe(10);
        assert!(matches!(
            capture_ok(&mut latch),
            CaptureDecision::Capture { .. }
        ));
        // Burst: latch thread sees 11,12,13 during the next (slow) sweep.
        latch.observe(11);
        latch.observe(12);
        latch.observe(13);
        assert_invariant(&latch);
        let d = capture_ok(&mut latch);
        assert_eq!(
            d,
            CaptureDecision::Capture {
                hour: 13,
                coalesced: 2
            }
        );
        let t = latch.telemetry();
        assert_eq!(t.observed, 4);
        assert_eq!(t.attempts, 2);
        assert_eq!(t.coalesced, 2);
        assert_eq!(t.coalesced_superseded, 2);
        assert_eq!(t.in_flight, 0);
        assert_invariant(&latch);
    }

    #[test]
    fn failed_capture_is_a_distinct_non_silent_loss() {
        // take_capture without a complete-frame emit = a failed sweep. It is an
        // attempt, not an emitted snapshot, and not coalesced.
        let mut latch = HourLatch::new(64);
        latch.observe(1);
        let d = latch.take_capture(); // sweep runs but produces nothing -> no emit
        assert!(matches!(d, CaptureDecision::Capture { .. }));
        let t = latch.telemetry();
        assert_eq!(t.attempts, 1);
        assert_eq!(t.emitted_to_sink, 0);
        assert_eq!(t.capture_failed(), 1);
        assert_eq!(t.coalesced, 0);
        assert_invariant(&latch);
    }

    #[test]
    fn stale_sweep_read_cannot_fabricate_reload_or_duplicate() {
        // Codex CODE D: with the old API the sweep passed its own (single-read)
        // counter value; a stale read of an already-captured hour triggered a
        // false reload + duplicate capture. The frontier-only API makes that
        // impossible: the sweep never authors epoch transitions.
        let mut latch = HourLatch::new(64);
        latch.observe(100);
        assert!(matches!(
            capture_ok(&mut latch),
            CaptureDecision::Capture { hour: 100, .. }
        ));
        // Game advances; latch observes 101.
        latch.observe(101);
        // The sweep's own clock read is momentarily stale (still 100) — but it no
        // longer feeds the state machine. take_capture uses the frontier (101).
        let d = capture_ok(&mut latch);
        assert_eq!(
            d,
            CaptureDecision::Capture {
                hour: 101,
                coalesced: 0
            }
        );
        let t = latch.telemetry();
        assert_eq!(t.backward_jumps, 0, "no fabricated reload");
        assert_eq!(t.observed, 2);
        assert_eq!(t.attempts, 2);
        assert_invariant(&latch);
    }

    #[test]
    fn multi_hour_jump_in_one_poll_never_misses_an_hour() {
        let mut latch = HourLatch::new(64);
        latch.observe(0);
        capture_ok(&mut latch);
        latch.observe(5); // jumped 1..=5 in one poll
        let t = latch.telemetry();
        assert_eq!(t.observed, 6); // hour 0 + 1,2,3,4,5
        assert_invariant(&latch);
    }

    /// GATE (a): synthetic 28 ms burst interval with a slow 150 ms sweep. Assert
    /// EVERY hour is accounted (attempt or reason-tagged coalesce), the invariant
    /// holds throughout, and nothing is silently dropped.
    #[test]
    fn gate_synthetic_burst_28ms_hours_150ms_sweep_zero_missed() {
        const HOUR_INTERVAL_MS: u64 = 28;
        const SWEEP_MS: u64 = 150;
        const RUN_MS: u64 = 300_000;
        let mut latch = HourLatch::new(64);
        let mut wall: u64 = 0;
        let hour_at = |ms: u64| (ms / HOUR_INTERVAL_MS) as i32;
        let poll_ms: u64 = 2;
        let mut next_poll: u64 = 0;
        let mut sweep_busy_until: u64 = 0;
        let mut expected_observed_max = 0i32;

        while wall <= RUN_MS {
            let next_event = next_poll.min(sweep_busy_until.max(wall));
            wall = next_event;
            if wall >= next_poll {
                latch.note_poll();
                latch.observe(hour_at(wall)); // STABLE frontier authority
                expected_observed_max = hour_at(wall);
                next_poll = wall + poll_ms;
            }
            if wall >= sweep_busy_until {
                match latch.take_capture() {
                    CaptureDecision::Capture { .. } => {
                        latch.record_complete_frame_emitted_to_sink(); // sweep completed
                        sweep_busy_until = wall + SWEEP_MS;
                    }
                    CaptureDecision::Idle => {
                        sweep_busy_until = wall + poll_ms;
                    }
                }
            }
            assert_invariant(&latch);
            if wall == RUN_MS {
                break;
            }
        }
        latch.flush();
        let t = latch.telemetry();

        assert!(t.invariant_holds(), "invariant: {t:?}");
        assert_eq!(t.in_flight, 0, "flush must resolve all pending: {t:?}");
        assert_eq!(
            t.observed,
            t.attempts + t.coalesced,
            "observed must equal attempts+coalesced after flush: {t:?}"
        );
        assert_eq!(
            t.observed,
            expected_observed_max as u64 + 1,
            "latch must observe every advanced hour: {t:?}, max hour {expected_observed_max}"
        );
        // Sustained overrun: most hours coalesce (physically forced), but every
        // one is COUNTED with a reason, and the reasons sum to coalesced.
        assert!(t.attempts > 0);
        assert!(t.coalesced > 0);
        assert_eq!(
            t.coalesced_superseded
                + t.coalesced_overflow
                + t.coalesced_flush
                + t.coalesced_backward
                + t.coalesced_anomalous,
            t.coalesced
        );
        // Attempts are throughput-limited (~ RUN_MS / SWEEP_MS).
        let approx_max = RUN_MS / SWEEP_MS + 2;
        assert!(
            t.attempts <= approx_max,
            "attempts {} > ceiling {}",
            t.attempts,
            approx_max
        );
    }

    #[test]
    fn momentary_burst_is_absorbed_when_sweep_matches_average() {
        let mut latch = HourLatch::new(64);
        let mut sweep_free_at: u64 = 0;
        let mut wall: u64 = 0;
        let mut hour: i32 = 0;
        let intervals = (0..200).map(|i| if i % 5 == 0 { 30 } else { 165 });
        for interval in intervals {
            wall += interval;
            latch.observe(hour);
            if wall >= sweep_free_at {
                if let CaptureDecision::Capture { .. } = latch.take_capture() {
                    latch.record_complete_frame_emitted_to_sink();
                    sweep_free_at = wall + 150;
                }
            }
            assert_invariant(&latch);
            hour += 1;
        }
        latch.flush();
        let t = latch.telemetry();
        assert!(t.invariant_holds());
        assert_eq!(t.observed, t.attempts + t.coalesced);
        assert!(
            t.coalesced * 3 < t.observed,
            "expected <1/3 coalesced with matched throughput: {t:?}"
        );
    }

    #[test]
    fn overflow_is_accounted_not_silent() {
        let mut latch = HourLatch::new(4);
        latch.observe(0);
        for h in 1..100i32 {
            latch.observe(h);
            assert_invariant(&latch);
        }
        let t = latch.telemetry();
        assert_eq!(t.observed, 100);
        assert_eq!(t.in_flight, 4);
        assert_eq!(t.coalesced, 96);
        assert_eq!(t.coalesced_overflow, 96);
        assert!(t.invariant_holds());
        assert_eq!(t.max_queue_depth, 4);
    }

    #[test]
    fn backward_jump_opens_new_epoch_without_losing_accounting() {
        let mut latch = HourLatch::new(64);
        latch.observe(500);
        latch.observe(501);
        latch.observe(502);
        latch.observe(200); // reload
        let t = latch.telemetry();
        assert_eq!(t.backward_jumps, 1);
        assert_eq!(t.observed, 4);
        assert_eq!(t.coalesced, 3);
        assert_eq!(t.coalesced_backward, 3);
        assert_eq!(t.in_flight, 1);
        assert!(t.invariant_holds());
        assert!(matches!(
            capture_ok(&mut latch),
            CaptureDecision::Capture { hour: 200, .. }
        ));
    }

    #[test]
    fn anomalous_jump_is_capped_and_flagged() {
        let mut latch = HourLatch::new(64);
        latch.observe(0);
        latch.observe(1_000_000);
        let t = latch.telemetry();
        assert_eq!(t.anomalous_jumps, 1);
        assert_eq!(t.coalesced_anomalous, 999_999);
        assert!(t.invariant_holds());
        assert!(t.in_flight <= t.queue_cap as u64);
    }

    /// Codex CODE F: measure the pure state-machine cost. The latch thread does
    /// one `observe` per poll (~40-500/s) plus the sweep's `take_capture`; if the
    /// core op is sub-microsecond, the thread's CPU is dominated by the memory read
    /// and sleep, not this logic. Loose upper bound so it is not timing-flaky.
    #[test]
    fn cost_state_machine_op_is_sub_microsecond() {
        use std::time::Instant;
        const N: i32 = 2_000_000;
        let mut latch = HourLatch::new(64);
        let start = Instant::now();
        let mut h = 0i32;
        for _ in 0..N {
            latch.observe(h);
            let _ = latch.take_capture();
            latch.record_complete_frame_emitted_to_sink();
            h = h.wrapping_add(1);
        }
        let elapsed = start.elapsed();
        let ns_per_cycle = elapsed.as_nanos() as f64 / N as f64;
        // Each cycle is observe + take_capture + record-emitted. Even in
        // unoptimized test
        // builds this is well under 1us; assert a generous 2us ceiling.
        assert!(
            ns_per_cycle < 2_000.0,
            "state-machine cycle too slow: {ns_per_cycle:.1} ns/cycle ({elapsed:?} for {N})"
        );
        eprintln!(
            "hour_latch cost: {ns_per_cycle:.1} ns per observe+take_capture+record-emitted cycle \
             ({N} cycles in {elapsed:?}); at a 2 ms poll the thread runs ~500 observe/s"
        );
    }

    #[test]
    fn generation_bumps_only_on_advance() {
        let mut latch = HourLatch::new(64);
        latch.observe(10);
        let g1 = latch.generation();
        latch.observe(10); // no advance
        assert_eq!(latch.generation(), g1);
        latch.observe(11);
        assert!(latch.generation() > g1);
    }

    #[test]
    fn failing_final_polls_mark_claim_truncated_without_breaking_invariant() {
        let mut latch = HourLatch::new(64);
        latch.observe(100);
        let _ = capture_ok(&mut latch);
        latch.note_poll();
        latch.note_read_failure();
        latch.note_poll();
        latch.note_read_failure();
        latch.flush();

        let t = latch.telemetry();
        assert_eq!(t.read_failures, 2);
        assert!(t.last_observe_failed);
        assert!(t.invariant_holds(), "terminal read failure: {t:?}");
        assert_eq!(t.accounting_claim(), "TRUNCATED-AT-LAST-GOOD-OBSERVE");
    }

    #[test]
    fn later_good_observe_clears_terminal_failure_marker() {
        let mut latch = HourLatch::new(64);
        latch.observe(100);
        latch.note_read_failure();
        assert!(latch.telemetry().last_observe_failed);
        latch.observe(101);

        let t = latch.telemetry();
        assert!(!t.last_observe_failed);
        assert_eq!(t.accounting_claim(), "COMPLETE");
        assert!(t.invariant_holds());
    }

    #[test]
    fn requested_but_inactive_never_claims_complete_accounting() {
        let t = HourLatchTelemetry::requested_but_inactive(64);
        assert!(t.latch_requested_but_inactive);
        assert!(t.invariant_holds());
        assert_eq!(t.accounting_claim(), "INACTIVE");
    }
}
