use collector_air::extended::air::{
    AirWingCopyContext, AirWingJob, Capacities, Captured, NO_WING_SOURCE, copy_air_refresh,
};
use engine_native_memory::walk::Outcome;
use replay_data::rows::TableId;

#[test]
fn worker_air_growth_hands_off_complete_mod_rosters() {
    use crate::unit_scale_tests::fixture;
    use collector_map_capture::map_capture::MapCaptured;
    const WINGS: usize = 12;
    let world = fixture::build(fixture::Shape {
        countries: 4,
        divisions: 0,
        wings: WINGS,
        planes: WINGS,
        ships: 0,
    });
    let memory = world.live();
    let mut job = AirWingJob::new();
    job.map_rows.resize(1, Default::default());
    job.sources.resize(1, Default::default());
    job.identities.resize(1, Default::default());
    job.names.resize(1, 0);
    let hour = (0..24)
        .find(|&hour| {
            !replay_data::cadence::should_walk(TableId::WingCumulative, hour, false)
                && !replay_data::cadence::should_walk(TableId::AirTheatreMembers, hour, false)
        })
        .unwrap();
    let report = copy_air_refresh(
        &memory,
        AirWingCopyContext {
            base: world.base,
            game_state: world.state,
            hour,
            force_full: false,
            limits: [1, 0, 0],
            game_score: false,
        },
        &mut job,
    );
    assert!(report.raw_ok);
    assert_eq!(job.map_outcome, Outcome::Rows { len: WINGS });
    assert_eq!(job.source_outcome, Outcome::Rows { len: WINGS });
    assert_eq!(job.identity_outcome, Outcome::Rows { len: WINGS });
    assert_eq!((job.names_len, job.name_failures), (WINGS * 4, 0));

    let mut map =
    // SAFETY: This fixture owns the map allocation and runs without a live attachment.
    unsafe { MapCaptured::new() };
    map.air_wings.rows.truncate(1);
    map.air_wing_names.truncate(1);
    let capacity = Capacities {
        wings: 1,
        ..Default::default()
    };
    let mut captured = Captured::with_capacities(&capacity);
    let old_rows = map.air_wings.rows.as_ptr();
    let old_names = map.air_wing_names.as_ptr();
    let old_identities = captured.identities.first.as_ptr();
    let owned_rows = job.map_rows.as_ptr();
    let owned_names = job.names.as_ptr();
    let owned_identities = job.identities.as_ptr();
    // SAFETY: This is the completed air job being restored to its original frame once.
    unsafe { map.install_owned_air(&mut job) };
    captured.install_owned_mission_identities(&mut job);
    assert_eq!(map.air_wings.rows.as_ptr(), owned_rows);
    assert_eq!(map.air_wing_names.as_ptr(), owned_names);
    assert_eq!(captured.identities.first.as_ptr(), owned_identities);
    assert_eq!(job.map_rows.as_ptr(), old_rows);
    assert_eq!(job.names.as_ptr(), old_names);
    assert_eq!(job.identities.as_ptr(), old_identities);
    assert_eq!(map.air_wings.outcome, Outcome::Rows { len: WINGS });
    assert_eq!(captured.identity_outcome, Outcome::Rows { len: WINGS });
    assert_eq!(map.air_wing_names_len, WINGS * 4);
    assert!(
        map.air_wings.rows[..WINGS]
            .iter()
            .enumerate()
            .all(|(ordinal, row)| row.name == Some((ordinal * 4, 4)))
    );

    assert_eq!(job.source_outcome, Outcome::Rows { len: WINGS });
    job.identity_outcome = NO_WING_SOURCE;
    let mut captured = Captured::with_capacities(&capacity);
    captured.install_owned_mission_identities(&mut job);
    assert_eq!(captured.identity_outcome, NO_WING_SOURCE);
}
