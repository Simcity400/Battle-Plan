//! WHERE IN ITS HOUR THE SWEEP IS ALLOWED TO START.
//!
//! The frame loop used to have exactly one rendezvous — "a new hour is
//! pending" — which a boundary satisfies the instant it passes. A build that
//! began late therefore stayed late: the next one began wherever the previous
//! ended, and nothing pulled the phase back. t45
//! (`runs/gf_clock_loss/phase_reconciliation.txt`) measured a mean build-start
//! lateness of 53.5 ms in a 101.0 ms hour — the hour's own midpoint, which is
//! the signature of a phase that is not locked to anything — with the median
//! frame having ZERO clock brackets closed by the time its boundary arrived.
//! 21,991 of 22,352 clock-lost domain-hours came from that, 54.7% of every
//! domain-hour in the capture.
//!
//! These tests cover the rule that replaces it, the two estimators it reads,
//! and the rendezvous primitive it parks on. The properties each one exists to
//! keep are named in its own doc comment; collectively they are: it cannot
//! wedge on a paused clock, it cannot drift across a speed change, it cannot
//! skip a boundary or wait into the one after it, and it degrades to the old
//! behaviour rather than to a worse one whenever it cannot measure.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use super::*;

/// t45's measured shape, in micros. Every number here is from
/// `runs/gf_clock_loss/phase_reconciliation.txt`, so a test that passes with
/// these is a test about the capture that funded the change.
const T45_HOUR_US: u64 = 101_010;
const T45_FANOUT_ORIGIN_US: u64 = 44_400;
const T45_CRIT_US: u64 = 49_900;
/// `fanout_origin + crit/2` — the offset by which half the frame's brackets
/// have closed.
const T45_HALF_YIELD_US: u64 = T45_FANOUT_ORIGIN_US + T45_CRIT_US / 2;
/// t45's mean build-start lateness: the defect, in one number.
const T45_LATENESS_US: u64 = 53_500;

/// The process-global `hour_phase` state is shared by every test in the binary,
/// so the ones that touch it take this first. Tests of the PURE rule need no
/// lock and take none.
fn phase_global() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn period_estimator_at(period_us: u64) -> HourPhaseState {
    let mut state = HourPhaseState::new();
    for hour in 0..=(HOUR_PERIOD_SAMPLES as i32) {
        state.note_boundary(hour, u64::try_from(hour).unwrap() * period_us);
    }
    state
}

// ---------------------------------------------------------------------------
// The rule
// ---------------------------------------------------------------------------

/// THE DEFECT AND THE FIX, on t45's own numbers.
///
/// At the measured mean lateness of 53.5 ms only 47.5 ms of the 101.0 ms hour
/// is left, which does not reach the 69.35 ms by which half the brackets close
/// — so the rule holds for the boundary rather than building a frame whose
/// fan-out would run in H+1. At a phase-locked start the whole hour is
/// available and it builds immediately.
///
/// A production change that removes the hold (returning `StartNow` from the
/// remaining-time arm) fails the first assertion; one that holds at the
/// boundary too — the degenerate "always hold" that would skip every hour —
/// fails the second.
#[test]
fn the_t45_phase_holds_for_the_boundary_and_a_locked_phase_builds_at_once() {
    assert_eq!(
        phase_lock_decision(
            Some(T45_LATENESS_US),
            Some(T45_HOUR_US),
            Some(T45_HALF_YIELD_US),
        ),
        PhaseLockDecision::HoldForBoundary {
            wait_us: (T45_HOUR_US - T45_LATENESS_US) + PHASE_LOCK_HOLD_SLACK_US,
        },
        "53.5 ms into a 101.0 ms hour leaves 47.5 ms, which does not reach the \
         69.35 ms by which half the brackets close; the whole fan-out would be \
         refused hour_label_mismatch"
    );
    assert_eq!(
        phase_lock_decision(Some(0), Some(T45_HOUR_US), Some(T45_HALF_YIELD_US)),
        PhaseLockDecision::StartNow(PhaseLockStart::HourHasRoom),
        "at the boundary the whole hour is available and holding could only \
         give it away"
    );
}

/// A MARGINAL SHORTFALL MUST NOT COST A WHOLE HOUR.
///
/// The naive rule — "hold unless the WHOLE sweep fits" — skips an hour to
/// recover the last millisecond of one. At 8 ms lateness a t45-shaped build
/// delivers all but a sliver, and the phase is close enough that the loop's own
/// slack pulls it back for free. The half-yield threshold is what keeps that
/// from being a hold.
#[test]
fn a_build_that_still_delivers_most_of_its_hour_is_not_traded_for_the_next_one() {
    let barely_late = phase_lock_decision(
        Some(8_000),
        Some(T45_HOUR_US),
        Some(T45_HALF_YIELD_US),
    );
    assert_eq!(
        barely_late,
        PhaseLockDecision::StartNow(PhaseLockStart::HourHasRoom),
        "93 ms of hour left against a 69.35 ms half-yield point: this frame \
         delivers, and holding would give up the hour to buy a sliver"
    );
    // The exact hinge, from both sides. `remaining == half_yield` builds;
    // one micro less holds.
    assert_eq!(
        phase_lock_decision(
            Some(T45_HOUR_US - T45_HALF_YIELD_US),
            Some(T45_HOUR_US),
            Some(T45_HALF_YIELD_US),
        ),
        PhaseLockDecision::StartNow(PhaseLockStart::HourHasRoom),
    );
    assert!(matches!(
        phase_lock_decision(
            Some(T45_HOUR_US - T45_HALF_YIELD_US + 1),
            Some(T45_HOUR_US),
            Some(T45_HALF_YIELD_US),
        ),
        PhaseLockDecision::HoldForBoundary { .. }
    ));
}

/// PAUSE IS NORMAL, AND A PAUSED CLOCK MUST NOT PRODUCE A HOLD.
///
/// the game auto-pauses constantly. While it is paused no boundary arrives, so any
/// hold would run to its timeout and the capture would spend its life waiting
/// for an edge that is not coming. The lateness grows past a whole period the
/// moment the clock stops, and that is what this arm keys on: past a period
/// there is no imminent boundary to rendezvous with, so the sweep starts now.
///
/// It is also the correct answer on the merits. A frame built while the clock
/// is stopped cannot straddle a boundary, so it loses nothing by starting
/// wherever it likes.
#[test]
fn a_paused_clock_never_holds_however_long_it_stays_paused() {
    for paused_for_us in [
        T45_HOUR_US,
        T45_HOUR_US + 1,
        T45_HOUR_US * 3,
        T45_HOUR_US * 1_000,
        u64::MAX / 2,
    ] {
        assert_eq!(
            phase_lock_decision(
                Some(paused_for_us),
                Some(T45_HOUR_US),
                Some(T45_HALF_YIELD_US),
            ),
            PhaseLockDecision::StartNow(PhaseLockStart::BoundaryOverdue),
            "a boundary {paused_for_us} us overdue is a stopped clock, not an \
             imminent edge"
        );
    }
}

/// A SWEEP THAT OUTLIVES THE HOUR CANNOT BE FIXED BY PHASE, SO IT IS NOT
/// PRETENDED OTHERWISE.
///
/// If half the brackets do not close inside a whole engine hour then no start
/// position delivers half, and the rule's own two-hour argument for holding
/// collapses: holding would give up this hour and buy a next one that is just
/// as short. Every phase must start immediately, and the reason on the wire
/// must say why — this is the reading that sends the work to the sweep's
/// length rather than to its phase.
///
/// t44c is the capture that shows the regime: a 106.8 ms sweep against an
/// 89.4 ms hour.
#[test]
fn a_sweep_longer_than_the_hour_starts_at_every_phase_and_names_the_reason() {
    const T44C_HOUR_US: u64 = 89_420;
    // A half-yield point past the whole hour.
    let half_yield_us = T44C_HOUR_US + 1;
    for lateness_us in (0..T44C_HOUR_US).step_by(1_000) {
        assert_eq!(
            phase_lock_decision(Some(lateness_us), Some(T44C_HOUR_US), Some(half_yield_us)),
            PhaseLockDecision::StartNow(PhaseLockStart::SweepExceedsHour),
            "at lateness {lateness_us} us"
        );
    }
    // Equality is the same case: "half the brackets close exactly as the hour
    // ends" leaves no phase with room to spare.
    assert_eq!(
        phase_lock_decision(Some(0), Some(T44C_HOUR_US), Some(T44C_HOUR_US)),
        PhaseLockDecision::StartNow(PhaseLockStart::SweepExceedsHour),
    );
}

/// UNTIL IT HAS MEASURED, IT DOES NOTHING.
///
/// Every capture opens with no period estimate and no finished build, and a
/// rule that guessed at either would be gating the whole sweep on a number it
/// invented. Each missing input alone is enough to start now.
#[test]
fn a_missing_measurement_starts_the_sweep_instead_of_guessing() {
    for inputs in [
        (None, Some(T45_HOUR_US), Some(T45_HALF_YIELD_US)),
        (Some(T45_LATENESS_US), None, Some(T45_HALF_YIELD_US)),
        (Some(T45_LATENESS_US), Some(T45_HOUR_US), None),
        (None, None, None),
    ] {
        assert_eq!(
            phase_lock_decision(inputs.0, inputs.1, inputs.2),
            PhaseLockDecision::StartNow(PhaseLockStart::Unestimated),
            "inputs {inputs:?}"
        );
    }
}

/// THE HOLD IS BOUNDED, AND WHEN THE BOUND WOULD BIND IT DOES NOT HOLD AT ALL.
///
/// A hold that cannot reach its boundary inside the cap would pay the idle and
/// collect nothing, so the rule refuses it and says so. The cap is what bounds
/// how long a stop request or a capture deadline can sit behind the
/// rendezvous.
#[test]
fn a_hold_that_would_outrun_the_cap_is_refused_rather_than_truncated() {
    // A slow game: a long hour, a sweep long enough that the rule wants to
    // hold, and more left in that hour than the cap allows waiting for.
    let period_us = 10 * PHASE_LOCK_MAX_HOLD_US;
    let half_yield_us = period_us - 1;
    let lateness_us = period_us / 2;
    assert_eq!(
        phase_lock_decision(Some(lateness_us), Some(period_us), Some(half_yield_us)),
        PhaseLockDecision::StartNow(PhaseLockStart::HoldWouldExceedCap),
    );
    // And every hold it DOES return fits inside the cap, with the slack
    // included.
    for lateness_us in (0..T45_HOUR_US).step_by(97) {
        if let PhaseLockDecision::HoldForBoundary { wait_us } = phase_lock_decision(
            Some(lateness_us),
            Some(T45_HOUR_US),
            Some(T45_HALF_YIELD_US),
        ) {
            assert!(
                wait_us <= PHASE_LOCK_MAX_HOLD_US,
                "hold of {wait_us} us at lateness {lateness_us} exceeds the cap"
            );
            assert!(
                wait_us >= PHASE_LOCK_HOLD_SLACK_US,
                "a hold always carries the slack that covers observer lag"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The period estimator
// ---------------------------------------------------------------------------

/// THE ESTIMATE IS ABSENT UNTIL IT IS MEASURED, AND THEN IT IS THE MEDIAN.
///
/// A partial window reported as a mean-of-whatever-arrived would open the gate
/// on one lucky sample. `None` sends the rule to `Unestimated`, which starts
/// the sweep — the old behaviour, which is the right thing to degrade to.
#[test]
fn the_hour_period_is_absent_until_the_window_fills_and_is_then_a_median() {
    let mut state = HourPhaseState::new();
    assert_eq!(state.period_us(), None, "no boundary, no estimate");
    for hour in 0..(HOUR_PERIOD_SAMPLES as i32) {
        // `hour` boundaries produce `hour - 1` intervals, so the window is
        // still short by one here.
        state.note_boundary(hour, u64::try_from(hour).unwrap() * T45_HOUR_US);
        assert_eq!(
            state.period_us(),
            None,
            "{hour} boundaries is fewer than {HOUR_PERIOD_SAMPLES} intervals"
        );
    }
    state.note_boundary(
        HOUR_PERIOD_SAMPLES as i32,
        u64::try_from(HOUR_PERIOD_SAMPLES).unwrap() * T45_HOUR_US,
    );
    assert_eq!(state.period_us(), Some(T45_HOUR_US));
}

/// A PAUSE DOES NOT MOVE THE PERIOD.
///
/// The clock stops, then resumes with a single one-hour step whose interval is
/// the whole pause. A mean would carry that pause for nine hours and quote an
/// hour period several times the real one; the median absorbs it and the
/// estimate is unchanged to the microsecond.
#[test]
fn a_pause_is_one_outlier_and_the_median_does_not_notice_it() {
    let mut state = period_estimator_at(T45_HOUR_US);
    assert_eq!(state.period_us(), Some(T45_HOUR_US));
    let mut at_us = u64::try_from(HOUR_PERIOD_SAMPLES).unwrap() * T45_HOUR_US;
    // Twelve seconds of paused game, then the next hour.
    at_us += 12_000_000;
    state.note_boundary(HOUR_PERIOD_SAMPLES as i32 + 1, at_us);
    assert_eq!(
        state.period_us(),
        Some(T45_HOUR_US),
        "one 12 s interval among nine must not become the hour period"
    );
    // Even four of them — the most a median of nine can absorb.
    for step in 2..=4 {
        at_us += 12_000_000;
        state.note_boundary(HOUR_PERIOD_SAMPLES as i32 + step, at_us);
    }
    assert_eq!(state.period_us(), Some(T45_HOUR_US));
}

/// A SPEED CHANGE IS TRACKED, AND THE TRANSITION NEVER READS BELOW EITHER
/// PERIOD.
///
/// The estimate has to follow the engine, because the whole rule is a
/// prediction of when the next boundary arrives. It converges once the
/// majority of the window is at the new period — five of nine — and while it
/// converges it reports one of the two real periods rather than an average of
/// them, which is what a median buys over a mean here.
#[test]
fn the_period_follows_a_speed_change_within_five_hours() {
    const SLOW_US: u64 = 404_040; // speed 4-ish: four times the hour.
    let mut state = period_estimator_at(T45_HOUR_US);
    let mut at_us = u64::try_from(HOUR_PERIOD_SAMPLES).unwrap() * T45_HOUR_US;
    let mut hour = HOUR_PERIOD_SAMPLES as i32;
    // A majority of the window, i.e. five of nine.
    let majority = HOUR_PERIOD_SAMPLES / 2 + 1;
    for step in 1..=majority {
        at_us += SLOW_US;
        hour += 1;
        state.note_boundary(hour, at_us);
        let seen = state.period_us().expect("the window stays full");
        assert!(
            seen == T45_HOUR_US || seen == SLOW_US,
            "step {step} reported {seen}, which is neither period"
        );
    }
    assert_eq!(
        state.period_us(),
        Some(SLOW_US),
        "a majority of the window at the new period carries the median"
    );
    // And back the other way, on the same terms.
    for _ in 0..majority {
        at_us += T45_HOUR_US;
        hour += 1;
        state.note_boundary(hour, at_us);
    }
    assert_eq!(state.period_us(), Some(T45_HOUR_US));
}

/// AN HOUR NOBODY OBSERVED CONTRIBUTES NO INTERVAL, AND NEITHER DOES A RELOAD.
///
/// A multi-hour advance covers hours that were never observed; a backwards step
/// is a reload. Either way the interval spans something other than one engine
/// hour, and putting it in the window would corrupt a prediction about the next
/// boundary. Both still ADVANCE THE SEQUENCE, because both are boundaries as
/// far as a sweep parked on the rendezvous is concerned — a reload that left a
/// holder asleep would be a wedge.
#[test]
fn a_skipped_hour_or_a_reload_advances_the_sequence_but_feeds_no_interval() {
    let mut state = HourPhaseState::new();
    state.note_boundary(100, 0);
    let after_first = state.seq;

    state.note_boundary(103, 3 * T45_HOUR_US); // three hours in one step
    assert_eq!(state.seq, after_first + 1, "a skipped run is still a boundary");
    state.note_boundary(50, 4 * T45_HOUR_US); // a reload, backwards
    assert_eq!(state.seq, after_first + 2, "a reload is still a boundary");

    // AND THE WINDOW IS SHORT BY EXACTLY THE TWO THEY DID NOT CONTRIBUTE.
    //
    // Counted one short of full, deliberately. Filling the window with nine
    // valid steps and then reading the median proves nothing: nine samples
    // overwrite every slot of a nine-slot ring, so a state machine that HAD
    // admitted the skip and the reload would report the same median (Codex
    // review 019fea5c-0abf-7ca2-9090-978cfeff2746, finding 2). Eight is the
    // count that can only be short if the two invalid steps contributed
    // nothing.
    let mut at_us = 4 * T45_HOUR_US;
    let mut hour = 51;
    for _ in 0..(HOUR_PERIOD_SAMPLES - 1) {
        at_us += T45_HOUR_US;
        state.note_boundary(hour, at_us);
        hour += 1;
    }
    assert_eq!(
        state.period_us(),
        None,
        "eight valid intervals is a short window; a skip or a reload that had \
         been admitted would have completed it"
    );
    at_us += T45_HOUR_US;
    state.note_boundary(hour, at_us);
    assert_eq!(
        state.period_us(),
        Some(T45_HOUR_US),
        "the window is exactly the nine one-hour intervals"
    );
}

// ---------------------------------------------------------------------------
// The sweep-shape estimator
// ---------------------------------------------------------------------------

/// The sweep's half-yield point is measured from real builds, is absent until
/// there are enough of them, and is a median so one pathological frame cannot
/// move it.
#[test]
fn the_sweep_half_yield_point_is_a_median_of_measured_builds() {
    let mut sweep = SweepShapeEstimator::new();
    for _ in 0..(SWEEP_SHAPE_SAMPLES - 1) {
        sweep.observe(T45_FANOUT_ORIGIN_US, T45_CRIT_US);
        assert_eq!(sweep.half_yield_us(), None);
    }
    sweep.observe(T45_FANOUT_ORIGIN_US, T45_CRIT_US);
    assert_eq!(sweep.half_yield_us(), Some(T45_HALF_YIELD_US));

    // One frame that took a second changes nothing.
    sweep.observe(1_000_000, 1_000_000);
    assert_eq!(sweep.half_yield_us(), Some(T45_HALF_YIELD_US));

    // A sustained change does move it.
    for _ in 0..SWEEP_SHAPE_SAMPLES {
        sweep.observe(10_000, 20_000);
    }
    assert_eq!(sweep.half_yield_us(), Some(20_000));
}

// ---------------------------------------------------------------------------
// The rendezvous primitive
// ---------------------------------------------------------------------------

/// A BOUNDARY THAT LANDED BEFORE THE WAIT BEGAN RETURNS AT ONCE.
///
/// This is the "never wait into the NEXT boundary" property, and it is the one
/// that would double the loss if it broke: a hold that missed the edge it was
/// sized for would park for a whole further hour, and the frame it eventually
/// built would be a full period late instead of a fraction of one. The
/// sequence is read under the same lock the wait re-acquires, so the boundary
/// cannot slip between them.
///
/// It is also the LATE WAKE case. An OS that deschedules the loop past the
/// boundary arrives here with a stale sequence and is told, immediately, that
/// the boundary happened.
#[test]
fn a_boundary_between_the_decision_and_the_wait_is_not_waited_through() {
    let _guard = phase_global();
    hour_phase::reset_for_test();
    let view = hour_phase::view();
    hour_phase::note_boundary(1, 1_000);
    let started = Instant::now();
    assert!(
        hour_phase::hold_for_boundary(view.seq, Duration::from_secs(30)),
        "the boundary already happened; the hold must not park for the next one"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "it returned late, so it parked on a boundary it had already been given"
    );
}

/// THE OBSERVER'S BOUNDARY REACHES THE RENDEZVOUS, not only the probe's ring.
///
/// The two instruments are two readings of one edge. If a boundary could be
/// recorded for `lw_hour_age_us` without reaching `hour_phase`, the capture
/// would MEASURE a lateness it had no mechanism to act on — and the phase-lock
/// tests would all still pass, because every one of them publishes to
/// `hour_phase` directly. Mutation M6 (delete the `hour_phase::note_boundary`
/// call from the observer's publication) SURVIVED the whole suite until this
/// test existed.
///
/// The publication is asserted through `publish_observed_boundary`, which is
/// the production function the observer thread calls — the observer's own body
/// runs only on a spawned thread reading a live engine address and cannot be
/// reached offline.
#[test]
fn one_observed_boundary_reaches_both_the_probe_ring_and_the_rendezvous() {
    const HOUR: i32 = 60_832_211;
    let _guard = phase_global();
    hour_phase::reset_for_test();
    hour_frontier_observation::reset_for_test();

    let before = hour_phase::view();
    assert_eq!(before.last_at_us, None, "the fixture starts empty");

    let at = Instant::now();
    assert_eq!(
        publish_observed_boundary(Some(HOUR - 1), Some((HOUR, 7)), at),
        Some(HOUR),
    );
    assert_eq!(
        hour_frontier_observation::observation_for(HOUR).0,
        Some(hour_frontier_observation::micros_since_origin(at)),
        "the probe's ring must hold this edge"
    );
    let after = hour_phase::view();
    assert_eq!(
        after.seq,
        before.seq + 1,
        "the rendezvous must see the SAME edge; without this the sweep can \
         measure its lateness and never act on it"
    );
    assert_eq!(
        after.last_at_us,
        Some(hour_frontier_observation::micros_since_origin(at)),
        "and at the same instant, in the same coordinate system"
    );

    // A poll that re-reads the hour already stamped publishes to NEITHER.
    assert_eq!(
        publish_observed_boundary(Some(HOUR), Some((HOUR, 8)), Instant::now()),
        None,
    );
    assert_eq!(
        hour_phase::view().seq,
        after.seq,
        "an unchanged hour is not a boundary for either instrument"
    );
}

/// A HOLD IS BOUNDED. When no boundary arrives it returns false at its
/// timeout, which is what turns a stopped clock into a late-fallback start
/// instead of a wedged capture.
#[test]
fn a_hold_with_no_boundary_times_out_rather_than_wedging() {
    let _guard = phase_global();
    hour_phase::reset_for_test();
    let view = hour_phase::view();
    let started = Instant::now();
    assert!(!hour_phase::hold_for_boundary(view.seq, Duration::from_millis(30)));
    let held = started.elapsed();
    assert!(held >= Duration::from_millis(25), "held {held:?}");
    assert!(held < Duration::from_secs(5), "held {held:?}");
}

/// A boundary published by the observer wakes a holder promptly, which is the
/// whole point of parking on a condvar rather than sleeping the predicted
/// remainder: the rendezvous lands on the edge, not on an estimate of it.
#[test]
fn a_boundary_published_from_another_thread_ends_the_hold() {
    let _guard = phase_global();
    hour_phase::reset_for_test();
    let view = hour_phase::view();
    let observed = Arc::new(AtomicU64::new(0));
    let started = Instant::now();
    let publisher = {
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            hour_phase::note_boundary(7, 7_000);
        })
    };
    let woke = hour_phase::hold_for_boundary(view.seq, Duration::from_secs(10));
    observed.store(1, Ordering::Relaxed);
    publisher.join().expect("publisher thread");
    assert!(woke, "the hold must end on the boundary, not on its timeout");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the hold outlived the boundary it was waiting for"
    );
}

// ---------------------------------------------------------------------------
// The loop, simulated
// ---------------------------------------------------------------------------

/// The frame loop over a synthetic engine clock: enough of it to exercise the
/// rule end to end, and no more.
///
/// Three things from the real loop are modelled, because the rule's behaviour
/// is meaningless without them:
///
///   * ONE BUILD PER OBSERVED HOUR. `ReadOnlyScheduler` holds one in-flight
///     request plus one pending across all domains, so an iteration that finds
///     the current hour already built has nothing to do and parks for the next
///     boundary — that is `wait_for_hour`. Without this the simulated loop
///     would rebuild the same hour four times inside one slow-speed hour and
///     report a phase no capture could have.
///   * ITERATION COST, not sweep cost. What decides the phase is how long the
///     WHOLE iteration takes against the hour: build, publication, preamble.
///     t45's frame wall p50 is 109.4 ms against a 101.0 ms hour, which is why
///     its phase free-ran.
///   * SKIPPED HOURS. An hour whose boundary passes with no build is a real
///     loss — the scheduler evicts the pending request and accounts it as a
///     `Backpressure` range — so the simulation counts them and the tests
///     assert on them.
struct SimulatedLoop {
    period_us: u64,
    fanout_origin_us: u64,
    crit_us: u64,
    iteration_cost_us: u64,
    /// Whether the run has ESTIMATES to give the rule.
    ///
    /// The control arm does NOT bypass `phase_lock_decision` — it calls it with
    /// the inputs absent, which is the production path every capture takes for
    /// its first nine hours and after any run of unobservable ones. That is
    /// deliberate: a control that skipped the production call would be a
    /// calibration assertion over the test's own model, which no production
    /// change could fail (Codex review
    /// 019fea5c-0abf-7ca2-9090-978cfeff2746, finding 3). Wired this way, a rule
    /// that ever held on absent estimates changes the control's phase and the
    /// control fails.
    estimated: bool,
    now_us: u64,
    last_built_hour: u64,
    /// Build-start lateness, one entry per build.
    starts: Vec<u64>,
    holds: u64,
    hold_timeouts: u64,
    hours_seen: u64,
    hours_with_no_build: u64,
}

impl SimulatedLoop {
    fn new(
        period_us: u64,
        fanout_origin_us: u64,
        crit_us: u64,
        iteration_cost_us: u64,
        opening_phase_us: u64,
        estimated: bool,
    ) -> Self {
        Self {
            period_us,
            fanout_origin_us,
            crit_us,
            iteration_cost_us,
            estimated,
            now_us: opening_phase_us,
            last_built_hour: opening_phase_us / period_us,
            starts: Vec::new(),
            holds: 0,
            hold_timeouts: 0,
            hours_seen: 0,
            hours_with_no_build: 0,
        }
    }

    fn half_yield_us(&self) -> u64 {
        self.fanout_origin_us + self.crit_us / 2
    }

    fn last_boundary_us(&self) -> u64 {
        (self.now_us / self.period_us) * self.period_us
    }

    fn run(&mut self, iterations: usize) {
        for _ in 0..iterations {
            let hour = self.now_us / self.period_us;
            if hour == self.last_built_hour {
                // Nothing pending: park for the boundary, which is what
                // `wait_for_hour` does once it is a boundary rendezvous.
                self.now_us = self.last_boundary_us() + self.period_us;
                continue;
            }
            let lateness_us = self.now_us - self.last_boundary_us();
            // BOTH ARMS GO THROUGH THE PRODUCTION RULE. The control differs
            // only in having no estimates to give it.
            let (period_us, half_yield_us) = if self.estimated {
                (Some(self.period_us), Some(self.half_yield_us()))
            } else {
                (None, None)
            };
            match phase_lock_decision(Some(lateness_us), period_us, half_yield_us) {
                PhaseLockDecision::HoldForBoundary { wait_us } => {
                    self.holds += 1;
                    let next_boundary_us = self.last_boundary_us() + self.period_us;
                    if next_boundary_us <= self.now_us + wait_us {
                        self.now_us = next_boundary_us;
                    } else {
                        self.hold_timeouts += 1;
                        self.now_us += wait_us;
                    }
                    continue;
                }
                PhaseLockDecision::StartNow(_) => {}
            }
            self.hours_seen += hour - self.last_built_hour;
            self.hours_with_no_build += hour - self.last_built_hour - 1;
            self.last_built_hour = hour;
            self.starts.push(lateness_us);
            self.now_us += self.iteration_cost_us;
        }
    }

    fn mean_start_lateness_us(&self) -> u64 {
        assert!(!self.starts.is_empty(), "the loop never built anything");
        self.starts.iter().sum::<u64>() / self.starts.len() as u64
    }

    /// The share of bracketed domain-hours this run delivers.
    ///
    /// A build started `lateness` into its hour has `period - lateness` before
    /// the boundary; nothing is bracketed until `fanout_origin`, and the
    /// brackets then close across `crit`, so the delivered share of ONE built
    /// hour is `(remaining - fanout_origin) / crit` clamped to `[0, 1]`. An
    /// hour with no build delivers nothing and is in the denominator.
    ///
    /// The same linear model the rule's own threshold rests on, applied to the
    /// whole run — which is what makes the comparison between the two arms an
    /// answer rather than an assertion.
    fn delivered_share(&self) -> f64 {
        let per_hour: f64 = self
            .starts
            .iter()
            .map(|lateness_us| {
                let remaining = self.period_us.saturating_sub(*lateness_us) as f64;
                (((remaining - self.fanout_origin_us as f64) / self.crit_us as f64).max(0.0))
                    .min(1.0)
            })
            .sum();
        per_hour / self.hours_seen as f64
    }
}

/// t45's own shape, both ways.
fn t45_loop(estimated: bool) -> SimulatedLoop {
    SimulatedLoop::new(
        T45_HOUR_US,
        T45_FANOUT_ORIGIN_US,
        T45_CRIT_US,
        // t45's frame wall p50, 109.4 ms: the iteration does NOT fit the hour,
        // which is what makes the phase free-run instead of settling.
        109_400,
        T45_LATENESS_US,
        estimated,
    )
}

/// THE CONTROL, and the evidence that the model is about t45 at all.
///
/// The rule with nothing measured yet — the production path of every capture's
/// first nine hours — leaves a loop whose iteration outlives the hour
/// free-running: the start walks forward by (iteration - period) every hour,
/// wraps, and its mean lateness settles at half a period. t45 measured 53.5 ms
/// against a 101.0 ms hour, and this reproduces it, which is why the delivered
/// share below can be read as a prediction rather than as an invention.
///
/// It is also a production assertion in its own right, and the one that pins
/// the degrade: a rule that ever held on absent inputs would move this phase
/// and fail here.
#[test]
fn with_nothing_measured_the_phase_free_runs_exactly_as_t45_measured() {
    let mut control = t45_loop(false);
    control.run(400);
    assert_eq!(
        control.holds, 0,
        "the rule must not hold on inputs it does not have"
    );
    let mean = control.mean_start_lateness_us();
    assert!(
        (45_000..=56_000).contains(&mean),
        "the control settled at {mean} us, not at the ~half-period t45 measured"
    );
    let delivered = control.delivered_share();
    assert!(
        (0.28..0.38).contains(&delivered),
        "the control delivered {delivered:.3}; t45's bracketed domain-hours came \
         in at 0.320 (1 - 0.680 hour_label_mismatch)"
    );
}

/// THE FIX, on the same loop.
///
/// The rule pulls every start into the front of its hour and keeps it there:
/// the settled phase never reaches the point where half the brackets would
/// fall outside the label, and the hours it gives up to hold are counted.
#[test]
fn the_rule_holds_the_start_in_the_front_of_the_hour() {
    let mut locked = t45_loop(true);
    locked.run(400);
    let worst = locked.starts.iter().rev().take(100).copied().max().unwrap();
    assert!(
        worst < T45_HOUR_US - T45_HALF_YIELD_US + PHASE_LOCK_HOLD_SLACK_US,
        "settled builds began up to {worst} us in, past the point the rule is \
         supposed to hold at"
    );
    let mean = locked.mean_start_lateness_us();
    assert!(
        mean < 20_000,
        "mean start lateness {mean} us; the control's is ~50_000"
    );
    assert!(locked.holds > 0, "the rule never fired");
    assert_eq!(
        locked.hold_timeouts, 0,
        "a steady clock and a derived period must never time a hold out"
    );
}

/// WHAT THE PHASE IS WORTH, and what it costs.
///
/// The two arms of the same loop, compared on the linear delivery model. The
/// hours the rule gives up to hold are IN the denominator, so this is the net
/// figure and not the gross one.
#[test]
fn the_phase_lock_delivers_more_domain_hours_than_it_gives_up() {
    let mut control = t45_loop(false);
    let mut locked = t45_loop(true);
    control.run(400);
    locked.run(400);
    let control_share = control.delivered_share();
    let locked_share = locked.delivered_share();
    assert!(
        locked_share > control_share * 1.8,
        "phase-locked delivered {locked_share:.3} against the control's \
         {control_share:.3}; the hold is not paying for itself"
    );
    assert!(
        locked.hours_with_no_build > 0,
        "in this regime the iteration outlives the hour, so SOME hour must be \
         given up; a run that gives up none is not modelling the cost"
    );
    assert!(
        locked.hours_with_no_build * 3 < locked.hours_seen,
        "the rule gave up {} of {} hours, which is not a phase lock but a \
         throughput cut",
        locked.hours_with_no_build,
        locked.hours_seen
    );
}

/// AN ITERATION THAT FITS THE HOUR NEEDS NO HELP, AND IS NOT GIVEN ANY.
///
/// When the loop can keep up, parking for the boundary already locks the phase
/// on its own and every hour is built. The rule must not turn that into a run
/// that skips hours: its only contribution here is the opening hour it holds
/// through, which was half gone before the capture reached it.
#[test]
fn a_loop_that_keeps_up_locks_without_giving_up_a_single_hour() {
    let mut locked = SimulatedLoop::new(
        T45_HOUR_US,
        T45_FANOUT_ORIGIN_US,
        T45_CRIT_US,
        // The sweep alone, comfortably inside the hour.
        94_300,
        T45_LATENESS_US,
        true,
    );
    locked.run(400);
    assert_eq!(
        locked.hours_with_no_build, 0,
        "after the opening hold every hour was built"
    );
    assert_eq!(
        locked.starts.iter().rev().take(100).copied().max(),
        Some(0),
        "every settled build began at its boundary"
    );
    assert!(
        (locked.delivered_share() - 1.0).abs() < 1e-9,
        "a locked loop that keeps up loses nothing to the hour label"
    );
}

/// A SPEED CHANGE MID-CAPTURE DOES NOT STRAND THE PHASE.
///
/// The hour quadruples under a loop that was locked to the fast one. The
/// estimator tracks it (proved separately, above); this proves the loop built
/// on it settles again, builds every one of the new longer hours, and never
/// times a hold out on a clock that is running steadily.
#[test]
fn the_loop_relocks_after_the_engine_changes_speed() {
    const SLOW_US: u64 = 404_040;
    let mut fast = t45_loop(true);
    fast.run(200);
    let mut slow = SimulatedLoop::new(
        SLOW_US,
        T45_FANOUT_ORIGIN_US,
        T45_CRIT_US,
        109_400,
        // The engine changed speed; the loop did not restart, so it resumes at
        // whatever phase of the new hour it was standing in.
        fast.now_us % SLOW_US,
        true,
    );
    slow.run(200);
    assert_eq!(
        slow.starts.iter().rev().take(50).copied().max(),
        Some(0),
        "the settled phase after the speed change is the boundary"
    );
    assert_eq!(
        slow.hold_timeouts, 0,
        "the period is re-derived, so no hold waits for an edge that is not \
         coming"
    );
    assert_eq!(
        slow.hours_with_no_build, 0,
        "a longer hour has room for the same iteration; nothing may be given up"
    );
}

/// THE REGIME THE RULE REFUSES TO TOUCH.
///
/// When half the brackets do not close inside a whole hour, no phase delivers
/// half and holding would give up hours to buy nothing. Every iteration starts
/// immediately, the loop behaves exactly as it did before the rule existed, and
/// the wire says `sweep_exceeds_hour` so the reading points at the sweep's
/// LENGTH rather than at its phase.
#[test]
fn a_sweep_that_outlives_the_hour_leaves_the_loop_exactly_as_it_was() {
    // t44c's regime: a sweep whose half-yield point does not fit the hour.
    let mut locked = SimulatedLoop::new(89_420, 57_500, 98_600, 112_400, 17_000, true);
    let mut control = SimulatedLoop::new(89_420, 57_500, 98_600, 112_400, 17_000, false);
    locked.run(200);
    control.run(200);
    assert!(
        locked.half_yield_us() > locked.period_us,
        "this fixture must be in the regime it is named for"
    );
    assert_eq!(locked.holds, 0, "no hold may be taken in this regime");
    assert_eq!(
        locked.starts, control.starts,
        "with the rule inactive the two arms must be the same loop"
    );
}

