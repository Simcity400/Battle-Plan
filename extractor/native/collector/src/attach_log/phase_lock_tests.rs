use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use super::*;

const T45_HOUR_US: u64 = 101_010;
const T45_FANOUT_ORIGIN_US: u64 = 44_400;
const T45_CRIT_US: u64 = 49_900;
const T45_HALF_YIELD_US: u64 = T45_FANOUT_ORIGIN_US + T45_CRIT_US / 2;
const T45_LATENESS_US: u64 = 53_500;

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

#[test]
fn a_build_that_still_delivers_most_of_its_hour_is_not_traded_for_the_next_one() {
    let barely_late = phase_lock_decision(Some(8_000), Some(T45_HOUR_US), Some(T45_HALF_YIELD_US));
    assert_eq!(
        barely_late,
        PhaseLockDecision::StartNow(PhaseLockStart::HourHasRoom),
        "93 ms of hour left against a 69.35 ms half-yield point: this frame \
         delivers, and holding would give up the hour to buy a sliver"
    );
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

#[test]
fn a_sweep_longer_than_the_hour_starts_at_every_phase_and_names_the_reason() {
    const T44C_HOUR_US: u64 = 89_420;
    let half_yield_us = T44C_HOUR_US + 1;
    for lateness_us in (0..T44C_HOUR_US).step_by(1_000) {
        assert_eq!(
            phase_lock_decision(Some(lateness_us), Some(T44C_HOUR_US), Some(half_yield_us)),
            PhaseLockDecision::StartNow(PhaseLockStart::SweepExceedsHour),
            "at lateness {lateness_us} us"
        );
    }
    assert_eq!(
        phase_lock_decision(Some(0), Some(T44C_HOUR_US), Some(T44C_HOUR_US)),
        PhaseLockDecision::StartNow(PhaseLockStart::SweepExceedsHour),
    );
}

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

#[test]
fn a_hold_that_would_outrun_the_cap_is_refused_rather_than_truncated() {
    let period_us = 10 * PHASE_LOCK_MAX_HOLD_US;
    let half_yield_us = period_us - 1;
    let lateness_us = period_us / 2;
    assert_eq!(
        phase_lock_decision(Some(lateness_us), Some(period_us), Some(half_yield_us)),
        PhaseLockDecision::StartNow(PhaseLockStart::HoldWouldExceedCap),
    );
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

#[test]
fn the_hour_period_is_absent_until_the_window_fills_and_is_then_a_median() {
    let mut state = HourPhaseState::new();
    assert_eq!(state.period_us(), None, "no boundary, no estimate");
    for hour in 0..(HOUR_PERIOD_SAMPLES as i32) {
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

#[test]
fn a_pause_is_one_outlier_and_the_median_does_not_notice_it() {
    let mut state = period_estimator_at(T45_HOUR_US);
    assert_eq!(state.period_us(), Some(T45_HOUR_US));
    let mut at_us = u64::try_from(HOUR_PERIOD_SAMPLES).unwrap() * T45_HOUR_US;
    at_us += 12_000_000;
    state.note_boundary(HOUR_PERIOD_SAMPLES as i32 + 1, at_us);
    assert_eq!(
        state.period_us(),
        Some(T45_HOUR_US),
        "one 12 s interval among nine must not become the hour period"
    );
    for step in 2..=4 {
        at_us += 12_000_000;
        state.note_boundary(HOUR_PERIOD_SAMPLES as i32 + step, at_us);
    }
    assert_eq!(state.period_us(), Some(T45_HOUR_US));
}

#[test]
fn the_period_follows_a_speed_change_within_five_hours() {
    const SLOW_US: u64 = 404_040;
    let mut state = period_estimator_at(T45_HOUR_US);
    let mut at_us = u64::try_from(HOUR_PERIOD_SAMPLES).unwrap() * T45_HOUR_US;
    let mut hour = HOUR_PERIOD_SAMPLES as i32;
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
    for _ in 0..majority {
        at_us += T45_HOUR_US;
        hour += 1;
        state.note_boundary(hour, at_us);
    }
    assert_eq!(state.period_us(), Some(T45_HOUR_US));
}

#[test]
fn a_skipped_hour_or_a_reload_advances_the_sequence_but_feeds_no_interval() {
    let mut state = HourPhaseState::new();
    state.note_boundary(100, 0);
    let after_first = state.seq;

    state.note_boundary(103, 3 * T45_HOUR_US);
    assert_eq!(
        state.seq,
        after_first + 1,
        "a skipped run is still a boundary"
    );
    state.note_boundary(50, 4 * T45_HOUR_US);
    assert_eq!(state.seq, after_first + 2, "a reload is still a boundary");

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

#[test]
fn the_sweep_half_yield_point_is_a_median_of_measured_builds() {
    let mut sweep = SweepShapeEstimator::new();
    for _ in 0..(SWEEP_SHAPE_SAMPLES - 1) {
        sweep.observe(T45_FANOUT_ORIGIN_US, T45_CRIT_US);
        assert_eq!(sweep.half_yield_us(), None);
    }
    sweep.observe(T45_FANOUT_ORIGIN_US, T45_CRIT_US);
    assert_eq!(sweep.half_yield_us(), Some(T45_HALF_YIELD_US));

    sweep.observe(1_000_000, 1_000_000);
    assert_eq!(sweep.half_yield_us(), Some(T45_HALF_YIELD_US));

    for _ in 0..SWEEP_SHAPE_SAMPLES {
        sweep.observe(10_000, 20_000);
    }
    assert_eq!(sweep.half_yield_us(), Some(20_000));
}

/// sequence is read under the same lock the wait re-acquires, so the boundary
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

#[test]
fn a_hold_with_no_boundary_times_out_rather_than_wedging() {
    let _guard = phase_global();
    hour_phase::reset_for_test();
    let view = hour_phase::view();
    let started = Instant::now();
    assert!(!hour_phase::hold_for_boundary(
        view.seq,
        Duration::from_millis(30)
    ));
    let held = started.elapsed();
    assert!(held >= Duration::from_millis(25), "held {held:?}");
    assert!(held < Duration::from_secs(5), "held {held:?}");
}

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
    assert!(
        woke,
        "the hold must end on the boundary, not on its timeout"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the hold outlived the boundary it was waiting for"
    );
}

struct SimulatedLoop {
    period_us: u64,
    fanout_origin_us: u64,
    crit_us: u64,
    iteration_cost_us: u64,
    estimated: bool,
    now_us: u64,
    last_built_hour: u64,
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
                self.now_us = self.last_boundary_us() + self.period_us;
                continue;
            }
            let lateness_us = self.now_us - self.last_boundary_us();
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

fn t45_loop(estimated: bool) -> SimulatedLoop {
    SimulatedLoop::new(
        T45_HOUR_US,
        T45_FANOUT_ORIGIN_US,
        T45_CRIT_US,
        109_400,
        T45_LATENESS_US,
        estimated,
    )
}

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

#[test]
fn a_loop_that_keeps_up_locks_without_giving_up_a_single_hour() {
    let mut locked = SimulatedLoop::new(
        T45_HOUR_US,
        T45_FANOUT_ORIGIN_US,
        T45_CRIT_US,
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

#[test]
fn a_sweep_that_outlives_the_hour_leaves_the_loop_exactly_as_it_was() {
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

fn seed_global_period(period_us: u64) {
    for hour in 0..=(HOUR_PERIOD_SAMPLES as i32) {
        hour_phase::note_boundary(hour, u64::try_from(hour).unwrap() * period_us);
    }
}

fn sweep_shape_at(fanout_origin_us: u64, crit_wall_us: u64) -> SweepShapeEstimator {
    let mut shape = SweepShapeEstimator::new();
    for _ in 0..SWEEP_SHAPE_SAMPLES {
        shape.observe(fanout_origin_us, crit_wall_us);
    }
    shape
}

#[test]
fn the_guard_publishes_the_pair_it_compared_and_the_arm_that_pair_selects() {
    const PERIOD_US: u64 = 68_000;
    const FANOUT_ORIGIN_US: u64 = 35_524;
    const CRIT_WALL_US: u64 = 80_000;
    let _guard = phase_global();
    hour_phase::reset_for_test();

    seed_global_period(PERIOD_US);
    let shape = sweep_shape_at(FANOUT_ORIGIN_US, CRIT_WALL_US);
    let half_yield_us = shape.half_yield_us().expect("five builds is the estimate");
    assert!(
        half_yield_us >= PERIOD_US,
        "the fixture must be in the disarmed regime it is named for"
    );

    let outcome =
        rendezvous_with_hour_boundary(&shape, Instant::now() + Duration::from_secs(30), || 0);
    assert_eq!(
        outcome,
        PhaseLockOutcome::Started(PhaseLockStart::SweepExceedsHour),
        "half the brackets do not close inside the hour, so the guard disarms"
    );

    let guard = hour_phase::take_decision_inputs();
    assert_eq!(
        guard.period_us,
        Some(PERIOD_US),
        "the period the guard compared must be on the wire, not a bracket the \
         harness measured elsewhere"
    );
    assert_eq!(
        guard.half_yield_us,
        Some(half_yield_us),
        "and the sweep shape it compared it against"
    );
    assert_eq!(
        guard.unestimated_decisions, 0,
        "both estimates existed, so no decision may be counted as unestimated"
    );
    assert_eq!(
        guard.disarmed,
        Some(true),
        "the guard evaluated `half_yield >= period` as true and took \
         SweepExceedsHour, so the published predicate must say so"
    );
    assert!(
        guard.half_yield_us.unwrap() >= guard.period_us.unwrap(),
        "and the published operands must satisfy the published predicate"
    );
}

#[test]
fn a_sweep_that_fits_the_hour_publishes_an_armed_guard() {
    const PERIOD_US: u64 = 68_000;
    let _guard = phase_global();
    hour_phase::reset_for_test();
    seed_global_period(PERIOD_US);

    let shape = sweep_shape_at(8_000, 12_000);
    let half_yield_us = shape.half_yield_us().expect("five builds is the estimate");
    assert!(half_yield_us < PERIOD_US, "fixture check");

    let outcome =
        rendezvous_with_hour_boundary(&shape, Instant::now() + Duration::from_secs(30), || 0);
    assert!(
        !matches!(
            outcome,
            PhaseLockOutcome::Started(PhaseLockStart::SweepExceedsHour)
        ),
        "the sweep fits, so this arm must not fire; got {outcome:?}"
    );
    let guard = hour_phase::take_decision_inputs();
    assert_eq!(
        guard.disarmed,
        Some(false),
        "the sweep fits inside the hour, so the guard is ARMED and the \
         published predicate must say so"
    );
}

#[test]
fn an_unestimated_decision_publishes_no_pair_and_the_drain_does_not_repeat() {
    let _guard = phase_global();
    hour_phase::reset_for_test();

    let outcome = rendezvous_with_hour_boundary(
        &SweepShapeEstimator::new(),
        Instant::now() + Duration::from_secs(30),
        || 0,
    );
    assert_eq!(
        outcome,
        PhaseLockOutcome::Started(PhaseLockStart::Unestimated)
    );

    let guard = hour_phase::take_decision_inputs();
    assert_eq!(guard.period_us, None, "there is no period to publish");
    assert_eq!(guard.half_yield_us, None, "and no sweep shape");
    assert_eq!(
        guard.disarmed, None,
        "and NO predicate: with no pair there is no comparison, and a `false` \
         here would read as `the guard was armed`"
    );
    assert_eq!(
        guard.unestimated_decisions, 1,
        "but there WAS a decision, and it had neither"
    );

    assert_eq!(
        hour_phase::take_decision_inputs(),
        hour_phase::GuardInputs::default()
    );
}

#[test]
fn a_rendezvous_past_the_deadline_publishes_no_guard_inputs() {
    let _guard = phase_global();
    hour_phase::reset_for_test();
    seed_global_period(68_000);

    let outcome = rendezvous_with_hour_boundary(
        &sweep_shape_at(20_000, 20_000),
        Instant::now() - Duration::from_secs(1),
        || 0,
    );
    assert_eq!(outcome, PhaseLockOutcome::NotEvaluated);
    assert_eq!(
        hour_phase::take_decision_inputs(),
        hour_phase::GuardInputs::default(),
        "no decision was taken, so nothing about one may be published"
    );
}

#[test]
fn only_the_hours_between_two_stamps_count_as_skipped() {
    assert_eq!(
        hours_skipped_between(Some(10), Some(11)),
        0,
        "the ordinary case"
    );
    assert_eq!(
        hours_skipped_between(Some(10), Some(12)),
        1,
        "hour 11 was lost"
    );
    assert_eq!(hours_skipped_between(Some(10), Some(20)), 9);
    assert_eq!(
        hours_skipped_between(Some(10), Some(10)),
        0,
        "a re-read of the stamped hour is not a boundary at all"
    );
    assert_eq!(
        hours_skipped_between(Some(10), Some(3)),
        0,
        "a backwards step is a reload, not seven lost hours"
    );
    assert_eq!(hours_skipped_between(None, Some(4)), 0, "no predecessor");
    assert_eq!(
        hours_skipped_between(Some(4), None),
        0,
        "no stamp this poll"
    );
}

#[test]
fn poll_gap_buckets_are_inclusive_of_the_bound_they_are_named_for() {
    use compact_contract::diagnostic_channel::{
        poll_gap_bucket, POLL_GAP_BUCKET_COUNT, POLL_GAP_BUCKET_MAX_US, POLL_GAP_BUCKET_UNIT_NAMES,
    };
    for pair in POLL_GAP_BUCKET_MAX_US.windows(2) {
        assert!(
            pair[0] < pair[1],
            "the bucket bounds must ascend strictly; {pair:?} does not"
        );
    }
    assert_eq!(
        POLL_GAP_BUCKET_UNIT_NAMES.len(),
        POLL_GAP_BUCKET_MAX_US.len() + 1,
        "one name per bound, plus the unbounded tail"
    );
    for (index, bound) in POLL_GAP_BUCKET_MAX_US.iter().copied().enumerate() {
        assert_eq!(
            poll_gap_bucket(bound),
            index,
            "{bound} us must be INSIDE {}, which is what `le` means",
            POLL_GAP_BUCKET_UNIT_NAMES[index]
        );
        assert_eq!(
            poll_gap_bucket(bound + 1),
            index + 1,
            "{} us must fall past {}",
            bound + 1,
            POLL_GAP_BUCKET_UNIT_NAMES[index]
        );
    }
    for (index, bound) in POLL_GAP_BUCKET_MAX_US.iter().copied().enumerate() {
        assert_eq!(
            bound % 1_000,
            0,
            "bucket {index}'s bound is {bound} us, which no `leNms` name can \
             state; a bound this table cannot spell cannot be labelled"
        );
        assert_eq!(
            POLL_GAP_BUCKET_UNIT_NAMES[index],
            format!("lw_poll_gap_le{}ms", bound / 1_000),
            "bucket {index}'s name must state its own bound"
        );
    }
    assert_eq!(
        poll_gap_bucket(0),
        0,
        "a zero gap is a gap and belongs in the first bucket, not out of the table"
    );
    assert_eq!(
        POLL_GAP_BUCKET_UNIT_NAMES[poll_gap_bucket(64_000)],
        "lw_poll_gap_le64ms"
    );
    assert_eq!(
        POLL_GAP_BUCKET_UNIT_NAMES[poll_gap_bucket(64_001)],
        "lw_poll_gap_gt64ms"
    );
    assert_eq!(
        poll_gap_bucket(u64::MAX),
        POLL_GAP_BUCKET_COUNT - 1,
        "the last bucket is unbounded above; nothing may fall out of the table"
    );
}

#[test]
fn a_skip_poll_is_counted_in_both_distributions_and_the_window_drains() {
    use compact_contract::diagnostic_channel::poll_gap_bucket;
    let _guard = phase_global();
    observer_poll_gap::reset_counters();

    observer_poll_gap::record_poll(900, 900, false, 0);
    observer_poll_gap::record_poll(1_500, 1_500, false, 0);
    observer_poll_gap::record_poll(70_000, 70_000, false, 2);
    observer_poll_gap::record_poll(1_200, 1_200, true, 0);

    let window = observer_poll_gap::take_window();
    assert_eq!(
        window.gaps, 4,
        "every poll carries a gap, so `gaps` IS the poll count -- which is what \
         makes `skips` a subset of it rather than a separate population"
    );
    assert_eq!(window.total_us, 900 + 1_500 + 70_000 + 1_200);
    assert_eq!(window.max_us, 70_000);
    assert_eq!(window.unreadable, 1);
    assert_eq!(window.skips, 1);
    assert_eq!(window.skipped_hours, 2, "one poll, two hours lost");
    assert_eq!(
        window.skip_total_us, 70_000,
        "the skip's own gap, and it is ALSO in total_us above"
    );
    assert_eq!(window.skip_max_us, 70_000);

    assert_eq!(window.buckets[poll_gap_bucket(900)], 1);
    assert_eq!(
        window.buckets[poll_gap_bucket(1_500)],
        2,
        "1 200 us joins it"
    );
    assert_eq!(
        window.buckets[poll_gap_bucket(70_000)],
        1,
        "the skip gap is above the engine hour, which is the reading NEGATIVE 11 \
         needs"
    );
    assert_eq!(
        window.buckets.iter().sum::<u64>(),
        window.gaps,
        "the buckets PARTITION the gaps -- and they do so BY CONSTRUCTION, \
         because `gaps` is their sum rather than a counter kept beside them. \
         This asserts the definition still holds, not a race: a stored counter \
         is what two threads could pull apart"
    );

    assert_eq!(
        observer_poll_gap::take_window(),
        Default::default(),
        "the window is drained as it is read, so no total is published twice"
    );
}

#[test]
fn a_skip_across_a_clock_outage_is_charged_the_whole_unobserved_window() {
    let _guard = phase_global();
    observer_poll_gap::reset_counters();

    observer_poll_gap::record_poll(1_000, 70_000, true, 0);
    observer_poll_gap::record_poll(1_000, 140_000, true, 0);
    observer_poll_gap::record_poll(1_000, 210_000, false, 3);

    let window = observer_poll_gap::take_window();
    assert_eq!(window.gaps, 3);
    assert_eq!(window.unreadable, 2);
    assert_eq!(window.skips, 1);
    assert_eq!(window.skipped_hours, 3);
    assert_eq!(
        window.skip_total_us, 210_000,
        "the skip must carry the unobserved window, not the 1 ms cadence gap of \
         the poll that happened to notice it"
    );
    assert_eq!(window.skip_max_us, 210_000);
    assert_eq!(
        window.total_us, 3_000,
        "and the CADENCE total is unaffected: the histogram still describes how \
         often the thread ran, which is a different question"
    );
    assert!(
        window.skip_total_us > window.total_us,
        "the two totals measure different spans and must never be differenced; \
         this fixture is the case that proves it"
    );
}

fn rendezvous_runtime(scheduler: ReadOnlyScheduler) -> ReadOnlySchedulerRuntime {
    ReadOnlySchedulerRuntime {
        shared: Arc::new((Mutex::new(scheduler), Condvar::new())),
        shutdown: Arc::new(AtomicBool::new(false)),
        unexpected_exit: Arc::new(AtomicBool::new(false)),
        handle: Some(thread::spawn(|| {})),
        date_addr: 0,
        module_base: 0,
    }
}

fn seed_phase_for_a_hold(period_us: u64, lateness_us: u64) {
    let ladder_us = period_us * HOUR_PERIOD_SAMPLES as u64 + lateness_us;
    loop {
        let now_us = hour_frontier_observation::micros_since_origin(Instant::now());
        let Some(shortfall) = ladder_us.checked_sub(now_us) else {
            break;
        };
        thread::sleep(Duration::from_micros(shortfall + 1_000));
    }
    let newest =
        hour_frontier_observation::micros_since_origin(Instant::now()).saturating_sub(lateness_us);
    let oldest = newest - period_us * HOUR_PERIOD_SAMPLES as u64;
    for step in 0..=HOUR_PERIOD_SAMPLES as u64 {
        hour_phase::note_boundary(i32::try_from(step).unwrap(), oldest + step * period_us);
    }
}

fn t45_sweep_shape() -> SweepShapeEstimator {
    let mut shape = SweepShapeEstimator::new();
    for _ in 0..SWEEP_SHAPE_SAMPLES {
        shape.observe(T45_FANOUT_ORIGIN_US, T45_CRIT_US);
    }
    shape
}

#[test]
fn a_hold_reports_the_pending_hours_the_scheduler_gave_up_while_it_was_parked() {
    let _guard = phase_global();
    hour_phase::reset_for_test();

    hour_phase::record_hold_locked(1_000, 2);
    hour_phase::record_hold_timeout(500, 1);

    let (locked, timed_out, held_us, evicted, reasons) = hour_phase::take_counters();
    assert_eq!((locked, timed_out), (1, 1));
    assert_eq!(held_us, 1_500, "idle is still both arms together");
    assert_eq!(
        evicted, 3,
        "the disposition accumulates over the holds the frame drains, both arms"
    );
    assert_eq!(reasons, [0; 5], "no un-held start was recorded here");

    let (_, _, _, evicted_again, _) = hour_phase::take_counters();
    assert_eq!(
        evicted_again, 0,
        "a counter the probe drains must be zero on the next read, or every \
         frame after the first republishes the same hours"
    );
}

#[test]
fn the_rendezvous_brackets_the_eviction_total_around_the_park() {
    const PRODUCER: &str = include_str!("../attach_log.rs");
    let start = PRODUCER
        .find("\nfn rendezvous_with_hour_boundary(")
        .expect("the free rendezvous must still be named this");
    let end = PRODUCER[start..]
        .find("\n}")
        .expect("the free rendezvous must close at column 0")
        + start;
    let body = &PRODUCER[start..end];
    let flat: String = body.chars().filter(|c| !c.is_whitespace()).collect();

    let before = body
        .find("let evicted_before = evicted_pending_hours();")
        .expect("the total must be read BEFORE the park");
    let park = body
        .find("hour_phase::hold_for_boundary(")
        .expect("the park must still be here");
    assert!(
        before < park,
        "the opening reading is taken after the park, so the window it brackets \
         is not the park"
    );
    assert!(
        flat.contains("evicted_pending_hours().saturating_sub(evicted_before)"),
        "the second reading must be subtracted from the first, not used raw"
    );
    assert_eq!(
        flat.matches("evicted_pending_hours()").count(),
        2,
        "the bracket is exactly two readings of the counter"
    );
    for arm in [
        "hour_phase::record_hold_locked(held_us, evicted_pending_hours)",
        "hour_phase::record_hold_timeout(held_us, evicted_pending_hours)",
    ] {
        assert!(
            body.contains(arm),
            "both held arms must carry the bracketed figure; missing `{arm}`"
        );
    }
    assert!(
        PRODUCER.contains(
            "rendezvous_with_hour_boundary(sweep_shape, deadline, || self.evicted_pending_hours())"
        ),
        "the scheduler must pass its own eviction total into the rendezvous"
    );
}

#[test]
fn the_hold_disposition_reaches_the_wire_as_a_summable_per_frame_unit() {
    use std::collections::BTreeSet;
    const UNIT: &str = "lw_phase_hold_evicted_pending_hours";

    fn probe_text() -> String {
        let summary = summarize_lane_spans(&[]);
        let mut out = Vec::new();
        append_compact_lane_wall_probe_record(
            &mut out,
            7,
            123,
            5,
            900,
            125,
            Some(2),
            FrameHourAge::default(),
            &[],
            &BTreeSet::new(),
            MapTopologyShareCounts::default(),
            130,
            700,
            0,
            &OutsideBuildSpans::default(),
            &summary,
            FrameCpuSplit::default(),
        );
        String::from_utf8_lossy(&out[compact_contract::COMPACT_RECORD_HEADER_LEN..]).into_owned()
    }

    let _guard = phase_global();

    hour_phase::reset_for_test();
    hour_phase::record_hold_locked(1_000, 2);
    let carried = probe_text();
    assert!(
        carried.contains(UNIT),
        "the frame that drained a two-hour hold must carry `{UNIT}`"
    );

    hour_phase::reset_for_test();
    let quiet = probe_text();
    assert!(
        quiet.contains(UNIT),
        "a frame with NO hold at all must still carry `{UNIT}` -- \"not emitted\" \
         and \"cost nothing\" are different claims, and this is the frame every \
         conditional-emission escape was hiding behind"
    );

    use compact_contract::diagnostic_channel::{unit_accumulation, Accumulation};
    assert_eq!(
        unit_accumulation(UNIT),
        Accumulation::PerFrame,
        "the probe drains this counter, so summing frames is the whole-capture \
         figure; MAX would publish the busiest single frame as the total"
    );
    assert_eq!(
        unit_accumulation("rs_evicted_pending_hours"),
        Accumulation::Cumulative,
        "and the scheduler's running total must stay on the other side, or a \
         reader adding the two would double-count every eviction"
    );
}
