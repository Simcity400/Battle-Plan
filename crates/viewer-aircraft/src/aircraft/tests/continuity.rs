use super::*;
use viewer_runtime::presentation::Continuity;

fn lifetime(_: usize, seed: u32) -> f32 {
    3.0 + (seed % 3) as f32
}

#[test]
fn hourly_aircraft_refreshes_match_an_uninterrupted_early_ending_scenario() {
    let defines = FlightDefines::for_tests();
    let (source, rigs) = one_run(
        vec![height(3.0), height(1.0), height(-1.0), height(-1.0)],
        1.0,
        false,
        false,
        Explode::Plays,
    );
    let mut uninterrupted = source.clone();
    let mut refreshed = source.clone();
    for frame in 0..=300 {
        let time = frame as f32 / 30.0;
        if frame % 7 == 0 {
            let mut next = source.clone();
            next.carry_presentation(&mut refreshed, Continuity::Continue).unwrap();
            refreshed = next;
        }
        let played = |_: (usize, u32, u32)| Some(Some((0.6, 2.0)));
        for plan in [&mut uninterrupted, &mut refreshed] {
            plan.advance(&rigs, &defines, Some(BAND), time, &played);
        }
        assert_eq!(refreshed.runs[0].cycle, uninterrupted.runs[0].cycle);
        assert_eq!(refreshed.runs[0].start, uninterrupted.runs[0].start);
        assert_eq!(refreshed.runs[0].grounds, uninterrupted.runs[0].grounds);
        assert_eq!(
            refreshed.eased_runs(&rigs, &defines, time),
            uninterrupted.eased_runs(&rigs, &defines, time)
        );
        let from = (frame - 1) as f32 / 30.0;
        assert_eq!(
            refreshed.missile_explosions(from, time),
            uninterrupted.missile_explosions(from, time)
        );
    }
    assert!(refreshed.runs[0].cycle > 2);
}

#[test]
fn reordering_or_removing_scenarios_keeps_surviving_actor_keys_and_ground_state() {
    let (mut previous, _) = one_run(vec![height(3.0); 4], 1.0, false, false, Explode::Plays);
    let mut other = previous.flights[0].clone();
    other.wing = 2;
    previous.flights.push(other);
    let mut run = previous.runs[0].clone();
    run.flights = 1..2;
    previous.runs.push(run);
    previous.assign_seeds(&lifetime);
    let source = previous.clone();
    previous.ran_at = Some(2.7);
    previous.runs[0].initialized = true;
    previous.runs[0].start = 2.7;
    previous.runs[0].cycle = 1;
    previous.runs[1].initialized = true;
    previous.runs[1].grounds[0] = Ground::Hidden;
    previous.runs[1].cycle = 4;

    let mut next = source.clone();
    next.flights.swap(0, 1);
    next.assign_seeds(&lifetime);
    next.carry_presentation(&mut previous, Continuity::Continue).unwrap();
    assert_eq!((next.flight_seed(0), next.flight_seed(1)), (previous.flight_seed(1), previous.flight_seed(0)));
    assert_eq!(next.runs[0].grounds, [Ground::Hidden]);
    assert_eq!((next.runs[0].cycle, next.runs[1].cycle), (4, 1));
    assert_eq!(next.runs[1].start, 2.7);

    let mut final_plan = source;
    final_plan.flights.remove(0);
    final_plan.runs.truncate(1);
    final_plan.assign_seeds(&lifetime);
    final_plan
        .carry_presentation(&mut next, Continuity::Continue)
        .unwrap();
    assert_eq!(final_plan.flight_seed(0), next.flight_seed(0));
    assert_eq!(final_plan.runs[0].cycle, 4);
    assert_eq!(final_plan.runs[0].grounds, [Ground::Hidden]);
}

#[test]
fn changed_scenarios_start_their_run_anew_and_explicit_resets_keep_no_runtime() {
    let (mut previous, _) = one_run(vec![height(3.0); 4], 1.0, false, false, Explode::Plays);
    previous.assign_seeds(&lifetime);
    previous.ran_at = Some(3.0);
    previous.runs[0].initialized = true;
    previous.runs[0].cycle = 4;
    previous.opened = true;
    let mut changed = previous.clone();
    changed.flights[0].outgoing.points.push([1.0; 3]);
    changed.runs[0].initialized = false;
    changed.runs[0].cycle = 0;
    changed
        .carry_presentation(&mut previous, Continuity::Continue)
        .unwrap();
    assert_eq!(changed.restarts(), 1);
    assert!(!changed.runs[0].initialized);
    assert_eq!(changed.runs[0].cycle, 0);

    let (mut reset, _) = one_run(vec![height(3.0); 4], 1.0, false, false, Explode::Plays);
    reset
        .carry_presentation(&mut previous, Continuity::Reset)
        .unwrap();
    assert!(!reset.runs[0].initialized);
    assert_eq!(reset.ran_at, None);
    assert!(!reset.opened);
}

#[test]
fn launch_effects_keep_their_actor_key_when_other_launches_change_order() {
    let launch = |wing| launches::Launch {
        mesh: 0,
        wing,
        position: [wing as f32, 0.0, 0.0],
        spawn: 0.5,
        length: 2.0,
        seed: wing as u32,
        explosions: None,
    };
    let mut previous = AircraftPlan {
        launches: vec![launch(1), launch(2)],
        ..Default::default()
    };
    previous.assign_seeds(&lifetime);
    let before = previous.launched(0.75)[1];
    let mut next = AircraftPlan {
        launches: vec![launch(2), launch(3)],
        ..Default::default()
    };
    next.assign_seeds(&lifetime);
    next.carry_presentation(&mut previous, Continuity::Continue).unwrap();
    let after = next.launched(0.75);
    assert_eq!(next.launches[1].length, lifetime(0, next.launch_seed(1)));
    assert_eq!(after[0], before);
    assert_ne!(after[1].1.animation_key(), before.1.animation_key());
    assert_ne!(after[1].1.animation_seed, previous.launch_seed(0));
}

#[test]
fn repeated_hour_installs_without_a_frame_never_reuse_an_unretired_actor_key() {
    let (mut previous, _) = one_run(vec![height(3.0); 4], 1.0, false, false, Explode::Plays);
    previous.assign_seeds(&lifetime);
    let source = previous.clone();
    let mut seen = BTreeSet::from([previous.flight_seed(0)]);
    for wing in 2..=100 {
        let mut next = source.clone();
        next.flights[0].wing = wing;
        next.assign_seeds(&lifetime);
        next.carry_presentation(&mut previous, Continuity::Continue).unwrap();
        assert!(seen.insert(next.flight_seed(0)));
        previous = next;
    }
}

#[test]
fn a_plan_that_changes_one_wings_flights_starts_that_run_anew() {
    let defines = FlightDefines::for_tests();
    let (mut previous, rigs) = one_run(vec![height(3.0), height(1.0), height(-1.0), height(-1.0)], 1.0, false, false, Explode::Plays);
    let mut other = previous.flights[0].clone();
    other.wing = 2;
    previous.flights.push(other);
    let mut run = previous.runs[0].clone();
    run.flights = 1..2;
    previous.runs.push(run);
    previous.assign_seeds(&lifetime);
    let source = previous.clone();
    let played = |_: (usize, u32, u32)| Some(Some((0.6, 2.0)));
    let at = 1.5;
    for frame in 0..=45 {
        let time = frame as f32 / 30.0;
        previous.advance(&rigs, &defines, Some(BAND), time, &played);
        previous.eased_runs(&rigs, &defines, time);
    }
    let drawn = previous.clone().eased_runs(&rigs, &defines, at);
    let mut restarted = source;
    restarted.flights[1].origin = [6.0, 10.0, 6.0];
    restarted.assign_seeds(&lifetime);
    restarted.carry_presentation(&mut previous.clone(), Continuity::Continue).unwrap();
    restarted.advance(&rigs, &defines, Some(BAND), at, &played);
    assert_eq!(restarted.restarts(), 1, "the changed run starts anew");
    assert_ne!(restarted.flight_seed(1), previous.flight_seed(1), "with fresh players");
    let unchanged = restarted.eased_runs(&rigs, &defines, at);
    assert_eq!(unchanged[0], drawn[0], "the other run's flight is where it was");
}

#[test]
fn a_plans_seeds_do_not_depend_on_the_plans_before_it() {
    let (source, _) = one_run(vec![height(3.0); 4], 1.0, false, false, Explode::Plays);
    let built = |wing| {
        let mut plan = source.clone();
        plan.flights[0].wing = wing;
        plan.launches.push(launches::Launch { mesh: 0, wing, position: [1.0, 2.0, 3.0], spawn: 0.5, length: 0.0, seed: 4, explosions: None });
        plan.assign_seeds(&lifetime);
        plan
    };
    let opened = built(1);
    let mut played = built(2);
    for wing in 3..=40 {
        let mut next = built(if wing == 40 { 1 } else { wing });
        next.carry_presentation(&mut played, Continuity::Continue).unwrap();
        played = next;
    }
    assert_eq!(played.flight_seeds, opened.flight_seeds);
    assert_eq!(played.launch_seeds, opened.launch_seeds);
    assert_eq!(played.launches[0].length, opened.launches[0].length);
}

#[test]
fn a_run_advanced_straight_to_a_time_stands_where_playing_to_it_stands() {
    let defines = FlightDefines::for_tests();
    let (source, rigs) = one_run(vec![height(3.0); 4], 1.0, false, false, Explode::Plays);
    let stand = |_: (usize, u32, u32)| Some(Some((0.6, 2.0)));
    let at = 240.0 / 30.0;
    let mut played = source.clone();
    played.assign_seeds(&lifetime);
    for frame in 0..=240 {
        let mut next = source.clone();
        next.assign_seeds(&lifetime);
        next.carry_presentation(&mut played, Continuity::Continue).unwrap();
        played = next;
        played.advance(&rigs, &defines, None, frame as f32 / 30.0, &stand);
    }
    let mut sought = source;
    sought.assign_seeds(&lifetime);
    sought.advance(&rigs, &defines, None, at, &stand);
    assert!(played.runs[0].cycle > 0, "the fixture passes a cycle");
    assert_eq!((sought.runs[0].cycle, sought.runs[0].start), (played.runs[0].cycle, played.runs[0].start));
    assert_eq!(sought.placed_runs(&rigs, &defines, at), played.placed_runs(&rigs, &defines, at));
}
