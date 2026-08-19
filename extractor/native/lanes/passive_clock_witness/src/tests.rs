use super::*;

fn healthy(phase: &str, unix_ms: i64, hour: i32) -> ClockSample {
    ClockSample {
        schema: WITNESS_SCHEMA_V2.to_string(),
        observed_at_utc: format!("1970-01-01T00:00:{:06.3}Z", unix_ms as f64 / 1000.0),
        observed_at_unix_ms: unix_ms,
        phase: phase.to_string(),
        pid: 34752,
        process_creation_filetime_utc: 134_298_727_268_108_302,
        exe_sha256: "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad".into(),
        observer_sha256: "18f51ee3f5074e0f059d4a398854dfc7f8b7c0ad40b25a2784c21323565068d2".into(),
        access: "PROCESS_QUERY_INFORMATION|PROCESS_VM_READ".into(),
        expected_speed: 5,
        speed_initial: 5,
        speed_final: 5,
        paused_initial: false,
        paused_final: false,
        pause_forced_initial: false,
        pause_forced_final: false,
        tick_inhibited_initial: false,
        tick_inhibited_final: false,
        paused_in_window: false,
        pause_forced_in_window: false,
        tick_inhibited_in_window: false,
        hour_a: hour,
        hour_b: hour + 8,
        hour_delta: 8,
        tick_progress_bits_a: 0x3f80_0000,
        tick_progress_bits_b: 0x3f80_0000,
        window_started_at_utc: format!("1970-01-01T00:00:{:06.3}Z", (unix_ms - 90) as f64 / 1000.0),
        window_started_at_unix_ms: unix_ms - 90,
        observed_window_ms: 90,
        clock_progress_waited_ms: 60,
        clock_progress_budget_ms: 3000,
        processor_milliseconds: unix_ms * 3,
        advancing: true,
    }
}

fn stalled(phase: &str, unix_ms: i64, hour: i32) -> ClockSample {
    let mut sample = healthy(phase, unix_ms, hour);
    sample.hour_b = sample.hour_a;
    sample.hour_delta = 0;
    sample.advancing = false;
    sample.observed_window_ms = sample.clock_progress_budget_ms;
    sample.window_started_at_unix_ms = sample.observed_at_unix_ms - sample.observed_window_ms;
    sample
}

fn engine_paused(phase: &str, unix_ms: i64, hour: i32) -> ClockSample {
    let mut sample = stalled(phase, unix_ms, hour);
    sample.paused_initial = true;
    sample.paused_final = true;
    sample.pause_forced_initial = true;
    sample.pause_forced_final = true;
    sample.paused_in_window = true;
    sample.pause_forced_in_window = true;
    sample
}

fn player_paused(phase: &str, unix_ms: i64, hour: i32) -> ClockSample {
    let mut sample = stalled(phase, unix_ms, hour);
    sample.paused_initial = true;
    sample.paused_final = true;
    sample.paused_in_window = true;
    sample
}

#[test]
fn a_speed_change_inside_the_graded_window_is_fatal() {
    let mut run = WitnessRun::new(StallPolicy::default());
    run.admit(healthy("capture", 0, 100)).expect("healthy");

    let mut perturbed = healthy("capture", 7_000, 108);
    perturbed.speed_final = 3;

    let failure = run
        .admit(perturbed)
        .expect_err("a speed write must be fatal");
    assert!(
        matches!(
            failure,
            WitnessFailure::ClientWriteDrift {
                speed_final: 3,
                expected_speed: 5,
                ..
            }
        ),
        "{failure:?}"
    );
    assert!(failure.to_string().contains("zero client writes"));
}

#[test]
fn a_speed_change_observed_only_at_the_start_of_a_window_is_fatal() {
    let mut run = WitnessRun::new(StallPolicy::default());
    let mut perturbed = healthy("capture", 0, 100);
    perturbed.speed_initial = 4;

    let failure = run
        .admit(perturbed)
        .expect_err("a speed write must be fatal");
    assert!(
        matches!(
            failure,
            WitnessFailure::ClientWriteDrift {
                speed_initial: 4,
                ..
            }
        ),
        "{failure:?}"
    );
}

#[test]
fn a_speed_change_is_fatal_even_while_the_engine_is_paused() {
    let mut run = WitnessRun::new(StallPolicy::default());
    let mut perturbed = engine_paused("capture", 0, 100);
    perturbed.speed_final = 1;

    let failure = run
        .admit(perturbed)
        .expect_err("a pause must not excuse a speed write");
    assert!(
        matches!(failure, WitnessFailure::ClientWriteDrift { .. }),
        "{failure:?}"
    );
}

#[test]
fn a_single_unlucky_window_never_ends_a_run() {
    let mut run = WitnessRun::new(StallPolicy::default());
    for (index, delta) in [10, 11, 8, 8, 4, 9, 3, 9, 4].into_iter().enumerate() {
        let mut sample = healthy(
            "ready_wait",
            index as i64 * 7_000,
            60_831_128 + index as i32 * 70,
        );
        sample.hour_b = sample.hour_a + delta;
        sample.hour_delta = delta;
        run.admit(sample).expect("an advancing window is healthy");
    }

    let state = run
        .admit(stalled("ready_wait", 63_000, 60_831_758))
        .expect("one window that saw no movement is not a verdict");

    assert_eq!(state, ClockState::Stalled);
}

#[test]
fn enough_stalled_samples_over_too_short_a_span_is_not_a_hang() {
    let mut run = WitnessRun::new(StallPolicy {
        min_consecutive_samples: 3,
        min_span_ms: 12_000,
    });
    for index in 0..6 {
        run.admit(stalled("capture", index * 200, 500))
            .expect("a short burst of stalled polls is not a hang");
    }
    assert_eq!(run.len(), 6);
}

#[test]
fn a_long_span_with_too_few_stalled_samples_is_not_a_hang() {
    let mut run = WitnessRun::new(StallPolicy {
        min_consecutive_samples: 3,
        min_span_ms: 12_000,
    });
    run.admit(stalled("capture", 0, 500)).expect("first");
    run.admit(stalled("capture", 60_000, 500)).expect("second");
    assert_eq!(run.len(), 2);
}

#[test]
fn a_sustained_stall_is_fatal_and_is_named_as_a_hang() {
    let mut run = WitnessRun::new(StallPolicy::default());
    run.admit(healthy("capture", 0, 500)).expect("healthy");
    run.admit(stalled("capture", 7_000, 508))
        .expect("first stall");
    run.admit(stalled("capture", 14_000, 508))
        .expect("second stall");

    let failure = run
        .admit(stalled("capture", 21_000, 508))
        .expect_err("a stopped clock must still fail");

    match &failure {
        WitnessFailure::EngineUnresponsive {
            consecutive_samples,
            span_ms,
            ..
        } => {
            assert_eq!(*consecutive_samples, 3);
            assert_eq!(*span_ms, 14_000);
        }
        other => panic!("{other:?}"),
    }
    let message = failure.to_string();
    assert!(
        message.contains("hang, not client drift and not a pause"),
        "{message}"
    );
}

#[test]
fn the_hang_report_distinguishes_a_spinning_process_from_an_idle_one() {
    let mut idle = WitnessRun::new(StallPolicy::default());
    for index in 0..3 {
        let mut sample = stalled("capture", index * 7_000, 500);
        sample.processor_milliseconds = 1_000;
        let outcome = idle.admit(sample);
        if let Err(WitnessFailure::EngineUnresponsive { burning_cpu, .. }) = outcome {
            assert!(!burning_cpu, "a process consuming no CPU is not spinning");
            return;
        }
    }
    panic!("three stalled windows over 14 s must escalate");
}

#[test]
fn the_hang_report_flags_a_process_that_is_spinning() {
    let mut spinning = WitnessRun::new(StallPolicy::default());
    for index in 0..3 {
        let mut sample = stalled("capture", index * 7_000, 500);
        sample.processor_milliseconds = index * 21_000;
        let outcome = spinning.admit(sample);
        if let Err(WitnessFailure::EngineUnresponsive { burning_cpu, .. }) = outcome {
            assert!(burning_cpu, "a process consuming 3x wall time is spinning");
            return;
        }
    }
    panic!("three stalled windows over 14 s must escalate");
}

#[test]
fn an_engine_auto_pause_never_ends_a_run_however_long_it_lasts() {
    let mut run = WitnessRun::new(StallPolicy::default());
    run.admit(healthy("capture", 0, 500)).expect("healthy");
    for index in 1..40 {
        let state = run
            .admit(engine_paused("capture", index * 7_000, 508))
            .expect("an engine pause is an observation, not a fault");
        assert_eq!(state, ClockState::PausedByEngine);
    }
}

#[test]
fn an_engine_hold_breaks_a_run_of_stalled_windows() {
    let mut run = WitnessRun::new(StallPolicy::default());
    run.admit(stalled("capture", 0, 500)).expect("stall");
    run.admit(stalled("capture", 7_000, 500)).expect("stall");
    run.admit(engine_paused("capture", 14_000, 500))
        .expect("pause");
    run.admit(stalled("capture", 21_000, 500)).expect("stall");
    run.admit(stalled("capture", 28_000, 500)).expect("stall");
    assert_eq!(run.len(), 5);
}

#[test]
fn the_forced_flag_tells_an_engine_pause_from_a_player_pause() {
    assert_eq!(
        classify(&engine_paused("capture", 0, 500)),
        ClockState::PausedByEngine
    );
    assert_eq!(
        classify(&player_paused("capture", 0, 500)),
        ClockState::PausedUnforced
    );
}

#[test]
fn a_pause_seen_at_either_end_of_the_window_is_a_pause() {
    let mut entering = stalled("capture", 0, 500);
    entering.paused_final = true;
    entering.pause_forced_final = true;
    entering.paused_in_window = true;
    entering.pause_forced_in_window = true;
    assert_eq!(classify(&entering), ClockState::PausedByEngine);

    let mut leaving = stalled("capture", 0, 500);
    leaving.paused_initial = true;
    leaving.pause_forced_initial = true;
    leaving.paused_in_window = true;
    leaving.pause_forced_in_window = true;
    assert_eq!(classify(&leaving), ClockState::PausedByEngine);
}

#[test]
fn a_hold_that_began_and_ended_between_the_endpoint_reads_is_still_a_hold() {
    let mut transient_pause = stalled("capture", 0, 500);
    assert!(!transient_pause.paused_initial && !transient_pause.paused_final);
    transient_pause.paused_in_window = true;
    transient_pause.pause_forced_in_window = true;
    assert_eq!(classify(&transient_pause), ClockState::PausedByEngine);

    let mut transient_unforced = stalled("capture", 0, 500);
    assert!(!transient_unforced.paused_initial && !transient_unforced.paused_final);
    transient_unforced.paused_in_window = true;
    assert_eq!(classify(&transient_unforced), ClockState::PausedUnforced);
}

#[test]
fn the_tick_dispatch_byte_is_a_diagnostic_and_classifies_nothing() {
    let mut stalled_with_byte = stalled("capture", 0, 500);
    stalled_with_byte.tick_inhibited_initial = true;
    stalled_with_byte.tick_inhibited_final = true;
    stalled_with_byte.tick_inhibited_in_window = true;
    assert_eq!(classify(&stalled_with_byte), ClockState::Stalled);

    let mut advancing_with_byte = healthy("capture", 0, 500);
    advancing_with_byte.tick_inhibited_in_window = true;
    assert_eq!(classify(&advancing_with_byte), ClockState::Advancing);

    let mut run = WitnessRun::new(StallPolicy::default());
    for index in 0..4 {
        let mut sample = healthy("capture", index * 7_000, 500 + index as i32 * 8);
        sample.tick_inhibited_in_window = true;
        run.admit(sample).expect("advancing");
    }
    let report = run.interval_report();
    assert_eq!(
        report.advancing_with_hold_ms, 0,
        "the byte must not file advancing windows as held"
    );
    assert!(report.advancing_ms > 0, "{report:?}");
}

#[test]
fn the_tick_dispatch_byte_cannot_suppress_a_hang_verdict() {
    let mut run = WitnessRun::new(StallPolicy::default());
    let mut failed = false;
    for index in 0..4 {
        let mut sample = stalled("capture", index * 11_000, 500);
        sample.tick_inhibited_in_window = true;
        if run.admit(sample).is_err() {
            failed = true;
            break;
        }
    }
    assert!(
        failed,
        "a stopped clock must still be reported as a hang while the byte is set"
    );
}

#[test]
fn either_the_hour_or_the_sub_hour_accumulator_proves_the_pacing_block_ran() {
    let mut speed_five = stalled("capture", 0, 500);
    speed_five.hour_b = speed_five.hour_a + 3;
    speed_five.hour_delta = 3;
    speed_five.advancing = true;
    let _ = &speed_five;
    assert_eq!(classify(&speed_five), ClockState::Advancing);

    let mut speed_two = stalled("capture", 0, 500);
    speed_two.tick_progress_bits_b = 0x3f00_0000;
    speed_two.advancing = true;
    assert_eq!(classify(&speed_two), ClockState::Advancing);

    let mut run = WitnessRun::new(StallPolicy::default());
    run.admit(speed_five).expect("hour movement is progress");
    run.admit(speed_two)
        .expect("sub-hour accumulator movement is progress");
}

#[test]
fn a_sample_that_contradicts_its_own_raw_fields_is_refused() {
    let mut run = WitnessRun::new(StallPolicy::default());
    let mut lying = stalled("capture", 0, 500);
    lying.advancing = true;

    let failure = run
        .admit(lying)
        .expect_err("a self-contradicting sample is not evidence");
    assert!(
        matches!(failure, WitnessFailure::UnusableSample { .. }),
        "{failure:?}"
    );
}

#[test]
fn a_sample_from_an_unrecognised_schema_is_refused() {
    let mut run = WitnessRun::new(StallPolicy::default());
    let mut old = healthy("capture", 0, 500);
    old.schema = "hoi4-passive-client-state-witness-v1".into();
    assert!(matches!(
        run.admit(old),
        Err(WitnessFailure::UnusableSample { .. })
    ));
}

#[test]
fn an_unknown_field_makes_a_sample_unparseable() {
    let mut value = serde_json::to_value(healthy("capture", 0, 500)).expect("serialize");
    value
        .as_object_mut()
        .expect("object")
        .insert("mystery".into(), serde_json::Value::Bool(true));
    assert!(serde_json::from_value::<ClockSample>(value).is_err());
}

#[test]
fn the_interval_report_separates_paused_time_from_active_time() {
    let mut run = WitnessRun::new(StallPolicy::default());
    run.admit(healthy("capture", 7_000, 500)).expect("active");
    run.admit(healthy("capture", 14_000, 508)).expect("active");
    run.admit(engine_paused("capture", 21_000, 516))
        .expect("paused");
    run.admit(engine_paused("capture", 28_000, 516))
        .expect("paused");
    run.admit(healthy("capture", 35_000, 516)).expect("active");

    let report = run.interval_report();

    assert_eq!(report.schema, "hoi4-passive-clock-intervals-v2");
    assert_eq!(INTERVALS_SCHEMA_V2, "hoi4-passive-clock-intervals-v2");
    assert_eq!(report.samples, 5);
    assert_eq!(
        report.intervals.len(),
        3,
        "active, paused, active must not be collapsed: {:?}",
        report.intervals
    );
    assert_eq!(report.intervals[1].state, ClockState::PausedByEngine);
    assert_eq!(report.intervals[1].samples, 2);
    assert_eq!(report.advancing_ms, 90 * 3);
    assert_eq!(report.paused_by_engine_ms, 3_000 * 2);
    assert_eq!(report.stalled_ms, 0);
    assert_eq!(report.wall_clock_ms, 35_000 - (7_000 - 90));
    assert_eq!(
        report.advancing_ms + report.paused_by_engine_ms + report.unobserved_ms,
        report.wall_clock_ms,
        "every millisecond is either measured into one state or unobserved"
    );
    assert_eq!(report.observed_game_hours, 8 * 3);
}

#[test]
fn unobserved_wall_clock_is_never_attributed_to_a_state() {
    let mut run = WitnessRun::new(StallPolicy::default());
    run.admit(healthy("capture", 1_000, 500)).expect("active");
    run.admit(healthy("capture", 6_000, 508)).expect("active");

    let report = run.interval_report();

    assert_eq!(report.intervals.len(), 1, "{:?}", report.intervals);
    assert_eq!(
        report.intervals[0].samples, 2,
        "the two same-state samples must merge, or the gap is never inside a span"
    );
    assert_eq!(
        report.intervals[0].span_ms, 5_090,
        "the interval spans the gap"
    );
    assert_eq!(
        report.advancing_ms, 180,
        "but only the two 90 ms windows were ever observed"
    );
    assert_eq!(report.unobserved_ms, 5_090 - 180);
    assert_eq!(report.wall_clock_ms, 5_090);
}

#[test]
fn a_gap_between_two_differently_classified_windows_belongs_to_neither() {
    let mut run = WitnessRun::new(StallPolicy::default());
    run.admit(healthy("capture", 1_000, 500)).expect("active");
    let mut brief_pause = engine_paused("capture", 6_000, 508);
    brief_pause.observed_window_ms = 20;
    brief_pause.clock_progress_waited_ms = 5;
    brief_pause.window_started_at_unix_ms = brief_pause.observed_at_unix_ms - 20;
    run.admit(brief_pause).expect("paused");

    let report = run.interval_report();

    assert_eq!(
        report.paused_by_engine_ms, 20,
        "only the 20 ms that was actually observed may be called paused"
    );
    assert_eq!(report.advancing_ms, 90);
    assert_eq!(report.unobserved_ms, report.wall_clock_ms - 110);
    assert!(
        report.unobserved_ms > 4_000,
        "the five-second gap belongs to no state: {report:?}"
    );
}

#[test]
fn a_window_that_was_paused_throughout_reports_zero_active_time() {
    let mut run = WitnessRun::new(StallPolicy::default());
    for index in 0..9 {
        run.admit(engine_paused("capture", index * 7_000, 500))
            .expect("a wholly paused capture is not a failure");
    }

    let report = run.interval_report();

    assert_eq!(report.advancing_ms, 0);
    assert_eq!(report.observed_game_hours, 0);
    assert_eq!(report.intervals.len(), 1);
    assert_eq!(report.intervals[0].state, ClockState::PausedByEngine);
}

#[test]
fn a_sample_whose_hour_delta_disagrees_with_its_hours_is_refused() {
    let mut run = WitnessRun::new(StallPolicy::default());
    let mut drifted = healthy("capture", 0, 500);
    assert_eq!(drifted.hour_b - drifted.hour_a, 8);
    drifted.hour_delta = 0;

    let failure = run
        .admit(drifted)
        .expect_err("a delta that disagrees with its own hours is not evidence");
    assert!(
        matches!(failure, WitnessFailure::UnusableSample { .. }),
        "{failure:?}"
    );
    assert!(failure.to_string().contains("hour_delta"));
}

/// leave i32. A widening bug would either panic in debug or wrap in release.
#[test]
fn an_hour_delta_that_cannot_fit_in_i32_is_refused_rather_than_wrapped() {
    let mut run = WitnessRun::new(StallPolicy::default());
    let mut extreme = healthy("capture", 0, 0);
    extreme.hour_a = i32::MIN;
    extreme.hour_b = i32::MAX;
    extreme.hour_delta = -1;

    assert!(matches!(
        run.admit(extreme),
        Err(WitnessFailure::UnusableSample { .. })
    ));
}

#[test]
fn a_window_bracket_too_short_to_contain_its_measurement_is_refused() {
    let mut run = WitnessRun::new(StallPolicy::default());
    let mut skewed = healthy("capture", 10_000, 500);
    skewed.window_started_at_unix_ms = skewed.observed_at_unix_ms - 50;
    assert!(skewed.observed_window_ms > 50);

    let failure = run
        .admit(skewed)
        .expect_err("an incoherent bracket is not evidence");
    assert!(
        failure.to_string().contains("window bracket"),
        "{failure:?}"
    );
}

#[test]
fn a_bracket_wider_than_its_measurement_is_ordinary_and_admitted() {
    let mut run = WitnessRun::new(StallPolicy::default());
    let mut descheduled = healthy("capture", 30_000, 500);
    descheduled.window_started_at_unix_ms = descheduled.observed_at_unix_ms - 20_000;

    run.admit(descheduled)
        .expect("a scheduling gap is not a broken sample");

    let report = run.interval_report();
    assert_eq!(
        report.advancing_ms, 90,
        "only the measured window is claimed"
    );
    assert_eq!(report.intervals[0].span_ms, 20_000);
    assert_eq!(report.unobserved_ms, 20_000 - 90);
}

#[test]
fn a_clock_wait_longer_than_the_window_it_sits_in_is_refused() {
    let mut run = WitnessRun::new(StallPolicy::default());
    let mut impossible = healthy("capture", 0, 500);
    impossible.clock_progress_waited_ms = impossible.observed_window_ms + 1;

    let failure = run
        .admit(impossible)
        .expect_err("a wait cannot outlast the window containing it");
    assert!(
        failure.to_string().contains("clock-progress wait"),
        "{failure:?}"
    );
}

#[test]
fn an_unforced_pause_is_reported_separately_from_an_engine_pause() {
    let mut run = WitnessRun::new(StallPolicy::default());
    run.admit(engine_paused("capture", 7_000, 500))
        .expect("engine pause");
    run.admit(player_paused("capture", 14_000, 500))
        .expect("unforced pause");

    let report = run.interval_report();

    assert_eq!(report.paused_by_engine_ms, 3_000);
    assert_eq!(report.paused_unforced_ms, 3_000);
    assert_eq!(
        report.intervals.len(),
        2,
        "the two pauses are different facts: {:?}",
        report.intervals
    );
}

#[test]
fn a_window_that_both_advanced_and_held_still_proves_the_engine_executed() {
    let mut mixed = healthy("capture", 0, 500);
    assert!(mixed.advancing);
    mixed.paused_in_window = true;
    mixed.pause_forced_in_window = true;

    assert_eq!(classify(&mixed), ClockState::AdvancingWithHold);
    assert!(ClockState::AdvancingWithHold.engine_executed());
    assert!(!ClockState::AdvancingWithHold.is_engine_hold());

    let mut run = WitnessRun::new(StallPolicy {
        min_consecutive_samples: 2,
        min_span_ms: 1,
    });
    run.admit(stalled("capture", 0, 500)).expect("stall");
    run.admit(mixed).expect("movement breaks the run");
    run.admit(stalled("capture", 20_000, 500))
        .expect("the run restarted, so one more stall is not a hang");
}

#[test]
fn a_mixed_window_is_not_spent_as_active_time() {
    let mut run = WitnessRun::new(StallPolicy::default());
    let mut mostly_held = healthy("capture", 3_000, 500);
    mostly_held.observed_window_ms = 3_000;
    mostly_held.window_started_at_unix_ms = 0;
    mostly_held.paused_in_window = true;
    mostly_held.pause_forced_in_window = true;
    run.admit(mostly_held).expect("the engine did execute");

    let report = run.interval_report();

    assert_eq!(
        report.advancing_ms, 0,
        "a window the dispatch gate held for most of its length is not active time"
    );
    assert_eq!(report.advancing_with_hold_ms, 3_000);
    assert_eq!(report.intervals[0].state, ClockState::AdvancingWithHold);
}
