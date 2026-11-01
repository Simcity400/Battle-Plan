use super::*;
use viewer_foundation::map::presentation::Continuity;

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
            next.carry_presentation(&mut refreshed, Continuity::Continue, &|_, _| {
                panic!("no launch in this fixture")
            })
            .unwrap();
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
    next.carry_presentation(&mut previous, Continuity::Continue, &|_, _| {
        panic!("no launch in this fixture")
    })
    .unwrap();
    assert_eq!((next.flight_seed(0), next.flight_seed(1)), (1, 0));
    assert_eq!(next.runs[0].grounds, [Ground::Hidden]);
    assert_eq!((next.runs[0].cycle, next.runs[1].cycle), (4, 1));
    assert_eq!(next.runs[1].start, 2.7);

    let mut final_plan = source;
    final_plan.flights.remove(0);
    final_plan.runs.truncate(1);
    final_plan
        .carry_presentation(&mut next, Continuity::Continue, &|_, _| {
            panic!("no launch in this fixture")
        })
        .unwrap();
    assert_eq!(final_plan.flight_seed(0), 1);
    assert_eq!(final_plan.runs[0].cycle, 4);
    assert_eq!(final_plan.runs[0].grounds, [Ground::Hidden]);
}

#[test]
fn changed_scenarios_get_fresh_players_and_explicit_resets_keep_no_runtime() {
    let (mut previous, _) = one_run(vec![height(3.0); 4], 1.0, false, false, Explode::Plays);
    previous.ran_at = Some(3.0);
    previous.runs[0].initialized = true;
    previous.runs[0].cycle = 4;
    previous.opened = true;
    let mut changed = previous.clone();
    changed.flights[0].outgoing.points.push([1.0; 3]);
    changed.runs[0].initialized = false;
    changed.runs[0].cycle = 0;
    changed
        .carry_presentation(&mut previous, Continuity::Continue, &|_, _| {
            panic!("no launch in this fixture")
        })
        .unwrap();
    assert_ne!(changed.flight_seed(0), previous.flight_seed(0));
    assert!(!changed.runs[0].initialized);
    assert_eq!(changed.runs[0].cycle, 0);

    let (mut reset, _) = one_run(vec![height(3.0); 4], 1.0, false, false, Explode::Plays);
    reset
        .carry_presentation(&mut previous, Continuity::Reset, &|_, _| {
            panic!("reset keeps its built lifetimes")
        })
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
    let before = previous.launched(0.75)[1];
    let mut next = AircraftPlan {
        launches: vec![launch(2), launch(3)],
        ..Default::default()
    };
    next.launches[0].length = 19.0;
    next.carry_presentation(&mut previous, Continuity::Continue, &|_, seed| {
        3.0 + (seed % 3) as f32
    })
    .unwrap();
    let after = next.launched(0.75);
    assert_eq!(
        next.launches[1].length,
        3.0 + (next.launch_seed(1) % 3) as f32
    );
    assert_eq!(after[0], before);
    assert_ne!(after[1].1.animation_key(), before.1.animation_key());
    assert_ne!(after[1].1.animation_seed, previous.launch_seed(0));
}

#[test]
fn repeated_hour_installs_without_a_frame_never_reuse_an_unretired_actor_key() {
    let (mut previous, _) = one_run(vec![height(3.0); 4], 1.0, false, false, Explode::Plays);
    let source = previous.clone();
    let mut seen = BTreeSet::from([previous.flight_seed(0)]);
    for wing in 2..=100 {
        let mut next = source.clone();
        next.flights[0].wing = wing;
        next.carry_presentation(&mut previous, Continuity::Continue, &|_, _| {
            panic!("no launch in this fixture")
        })
        .unwrap();
        assert!(seen.insert(next.flight_seed(0)));
        previous = next;
    }
}
