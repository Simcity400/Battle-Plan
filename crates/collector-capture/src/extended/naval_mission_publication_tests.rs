use collector_certificates::slicing::SliceFingerprints;
use collector_naval_missions::extended::naval_mission_inputs::tests::{
    build_world, rows, use_fixture_certificates,
};
use engine_native_memory::walk::Outcome;
use replay_data::rows::tables;

#[test]
fn ordinary_strict_missions_grow_on_first_and_later_publications() {
    use crate::extended::{Capacities, Captured, NavalMissionLoan, OrdinaryPart};

    let mut owner = Captured::with_capacities(&Capacities::default());
    use_fixture_certificates(&mut owner.naval_mission_inputs);
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    let mut loan = NavalMissionLoan {
        force_full: true,
        fingerprints: Some(Box::new(SliceFingerprints::new())),
    };
    for (hour, count) in [5_000, 6_000].into_iter().enumerate() {
        let world = build_world(count, true);
        let (base, state, hour) = (world.base, world.game_state, hour as i32);
        let mut parts = owner.take_ordinary_parts(&countries, base, state, hour, Ok(()), loan);
        let request = owner.take_ordinary_request(&countries, base, state, hour);
        for part in &mut parts {
            if matches!(part.part, OrdinaryPart::Navies(_)) {
                part.part.capture(&world.slice(), base, state, hour, Ok(()));
            }
        }
        loan = owner.restore_ordinary_request(request, None, parts.map(|part| (part, None)));
        owner.decode_mission_tails();
        let count = count as usize;
        let captured = &owner.naval_mission_inputs;
        assert_eq!(
            &captured.outcomes()[..4],
            &[
                Outcome::Rows { len: count * 4 },
                Outcome::Rows { len: count * 3 },
                Outcome::Rows { len: count },
                Outcome::Rows { len: count * 5 },
            ]
        );
        assert_eq!(captured.headquarters.len, count);
        assert_eq!(captured.census.len, count * 5);
        assert_eq!(captured.hubs.len, count * 3);
        assert_eq!(captured.entries.len, count * 5);
        let tables = captured.tables();
        assert_eq!(
            rows::<tables::NavalMissionTaskForces>(&tables[0]).len(),
            count * 4
        );
        assert_eq!(
            rows::<tables::NavalMissionLedgers>(&tables[1]).len(),
            count * 3
        );
        assert_eq!(
            rows::<tables::NavalMissionCountries>(&tables[2]).len(),
            count
        );
        assert_eq!(
            rows::<tables::NavalMissionCommanders>(&tables[3]).len(),
            count * 5
        );
    }
}
