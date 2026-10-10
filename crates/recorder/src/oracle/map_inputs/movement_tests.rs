use super::*;
use replay::rows::{
    CountryCounterInputRow, CountryMovementInputs, CountryNavalMovementInputs, NavalPathInputs,
    PhysicalNavyNearestCache, Recorded, RegionTransportSpeed, ReservedCountryNamingRow,
    TaskForceMapMotionRow, TaskForceRow, TheatreControllerAreas,
};

fn set(frame: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    *frame.tables[index] = TableView::Complete(rows);
}

fn packet(frame: &DecodedFrame) -> CountryNavalMovementInputs {
    CountryNavalMovementInputs {
        sample_hour: frame.hour,
        nearest_caches: Some(vec![]),
        reserved_theatre: None,
    }
}

fn record(frame: &mut DecodedFrame, packet: Recorded<Option<CountryNavalMovementInputs>>) {
    set(
        frame,
        TableRows::ReservedCountryNaming(vec![ReservedCountryNamingRow {
            movement: packet,
            ..Default::default()
        }]),
    );
}

#[test]
fn stationary_and_targeted_task_forces_keep_their_owner_navy_demand() {
    let (mut frame, _, _, _) = super::super::tests::fixture();
    let stationary = (5_u64 << 32) | 51;
    let targeted = (6_u64 << 32) | 51;
    set(
        &mut frame,
        TableRows::TaskForces(vec![
            TaskForceRow {
                id: stationary,
                roster_owner: 102,
                ..Default::default()
            },
            TaskForceRow {
                id: targeted,
                roster_owner: 101,
                ..Default::default()
            },
        ]),
    );
    set(
        &mut frame,
        TableRows::TaskForceMapMotion(vec![
            TaskForceMapMotionRow {
                task_force: stationary,
                ..Default::default()
            },
            TaskForceMapMotionRow {
                task_force: targeted,
                path: vec![1],
                target: 1,
                ..Default::default()
            },
        ]),
    );
    assert_eq!(navy_owner_slots(&frame).unwrap(), BTreeSet::from([7]));
}

#[test]
fn shared_cache_closure_preserves_foreign_physical_slots_and_authenticated_nulls() {
    let (mut frame, _, _, _) = super::super::tests::fixture();
    let force = (5_u64 << 32) | 51;
    set(
        &mut frame,
        TableRows::TaskForces(vec![TaskForceRow {
            id: force,
            roster_owner: 102, // Alias of the owner whose canonical navy slot is seven.
            route_count: 1,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
            task_force: force,
            path: vec![1],
            ..Default::default()
        }]),
    );
    let owner = CountryMovementInputs {
        sample_hour: frame.hour,
        naval: Some(NavalPathInputs {
            navy_slot: 7,
            present: true,
            foreign_navy_slots: Some(vec![900, 0, 900]),
            supply_hubs: Some(vec![0, 2, 2]),
        }),
        theatre: Some(TheatreControllerAreas::default()),
        ..Default::default()
    };
    set(
        &mut frame,
        TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
            forward_visibility: replay::rows::Recorded::NotRecorded,
            country: 101,
            movement: Recorded::Value(Some(owner.clone())),
            ..Default::default()
        }]),
    );
    let mut shared = packet(&frame);
    shared.nearest_caches = Some(vec![
        PhysicalNavyNearestCache {
            slot: 0,
            present: false,
            nearest_bases: Some(vec![]),
        },
        PhysicalNavyNearestCache {
            slot: 7,
            present: true,
            nearest_bases: Some(vec![None, Some(0), Some(2)]),
        },
        PhysicalNavyNearestCache {
            slot: 900,
            present: false,
            nearest_bases: Some(vec![]),
        },
    ]);
    shared.reserved_theatre = Some(TheatreControllerAreas {
        provinces: vec![1],
        null_controller_area: true,
    });
    let mut run = |packet| {
        record(&mut frame, packet);
        let mut report = Report::default();
        reserved_movement(&frame, &mut report).unwrap();
        report
    };
    let report = run(Recorded::Value(Some(shared.clone())));
    assert!(report.differences.is_empty());
    assert!(report.failed_comparisons.is_empty());
    let mut refused = shared.clone();
    refused.nearest_caches.as_mut().unwrap()[1].nearest_bases = None;
    assert_eq!(run(Recorded::Value(Some(refused))).differences.len(), 1);
    let mut missing = shared.clone();
    missing.nearest_caches.as_mut().unwrap().pop();
    assert_eq!(run(Recorded::Value(Some(missing))).differences.len(), 1);
    let mut false_null = shared.clone();
    false_null.nearest_caches.as_mut().unwrap()[0].nearest_bases = Some(vec![None]);
    assert_eq!(run(Recorded::Value(Some(false_null))).differences.len(), 1);
    let mut duplicate = shared.clone();
    duplicate
        .nearest_caches
        .as_mut()
        .unwrap()
        .push(PhysicalNavyNearestCache {
            slot: 900,
            present: false,
            nearest_bases: Some(vec![]),
        });
    assert_eq!(
        run(Recorded::Value(Some(duplicate)))
            .failed_comparisons
            .len(),
        1
    );
    let mut stale = shared.clone();
    stale.sample_hour -= 1;
    assert_eq!(run(Recorded::Value(Some(stale))).differences.len(), 1);
    assert_eq!(run(Recorded::Value(None)).differences.len(), 1);
    assert!(run(Recorded::NotRecorded).differences.is_empty());
    let mut refused_owner = owner;
    refused_owner.naval = None;
    refused_owner.theatre = None;
    set(
        &mut frame,
        TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
            forward_visibility: replay::rows::Recorded::NotRecorded,
            country: 101,
            movement: Recorded::Value(Some(refused_owner)),
            ..Default::default()
        }]),
    );
    let mut report = Report::default();
    super::super::country_rosters::compare_movement(&frame, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2);
}

#[test]
fn shared_theatre_absence_requires_complete_empty_demand_and_keeps_other_failures() {
    let (mut frame, _, _, _) = super::super::tests::fixture();
    *frame.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "province domain refused".into(),
    };
    let shared = packet(&frame);
    record(&mut frame, Recorded::Value(Some(shared.clone())));
    let mut report = Report::default();
    reserved_movement(&frame, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(report.failed_comparisons.is_empty());
    let mut broken = shared.clone();
    broken.nearest_caches = None;
    broken.reserved_theatre = Some(TheatreControllerAreas {
        provinces: vec![1],
        null_controller_area: false,
    });
    record(&mut frame, Recorded::Value(Some(broken)));
    let mut report = Report::default();
    reserved_movement(&frame, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(
        report
            .failed_comparisons
            .values()
            .any(|reason| reason.contains("province domain refused"))
    );
    *frame.tables[TableId::TaskForceMapMotion.index()] = TableView::Unavailable {
        reason: "motion domain refused".into(),
    };
    record(&mut frame, Recorded::Value(Some(shared)));
    let mut report = Report::default();
    reserved_movement(&frame, &mut report).unwrap();
    assert!(
        report
            .failed_comparisons
            .values()
            .any(|reason| reason.contains("motion domain refused"))
    );
}

#[test]
fn absent_naval_graphs_report_demand_failures_without_losing_transport_refusals() {
    for missing_link in [false, true] {
        let (mut frame, _, _, _) = super::super::tests::fixture();
        let country = if missing_link { 999 } else { 101 };
        if !missing_link {
            *frame.tables[TableId::TaskForceMapMotion.index()] = TableView::Unavailable {
                reason: "motion demand refused".into(),
            };
        }
        let hour = frame.hour;
        set(
            &mut frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                forward_visibility: replay::rows::Recorded::NotRecorded,
                country,
                movement: Recorded::Value(Some(CountryMovementInputs {
                    sample_hour: hour,
                    transport: Some(vec![RegionTransportSpeed {
                        region: 0,
                        speed: None,
                    }]),
                    ..Default::default()
                })),
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        super::super::country_rosters::compare_movement(&frame, &mut report).unwrap();
        let failure = &report.failed_comparisons
            [&format!("CountryCounterInputs {country}.movement.naval demand")];
        assert!(failure.contains(if missing_link {
            "no positive country link"
        } else {
            "motion demand refused"
        }));
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("transport region 0"))
        );
    }
}

#[test]
fn absent_shared_graphs_refuse_missing_motion_rows_and_invalid_targets() {
    for invalid_target in [false, true] {
        let (mut frame, _, _, _) = super::super::tests::fixture();
        let force = (5_u64 << 32) | 51;
        set(
            &mut frame,
            TableRows::TaskForces(vec![TaskForceRow {
                id: force,
                roster_owner: 101,
                ..Default::default()
            }]),
        );
        if invalid_target {
            set(
                &mut frame,
                TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
                    task_force: force,
                    path: vec![1],
                    target: u32::MAX,
                    ..Default::default()
                }]),
            );
        }
        let shared = packet(&frame);
        record(&mut frame, Recorded::Value(Some(shared)));
        let mut report = Report::default();
        reserved_movement(&frame, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.failed_comparisons.len(), 2);
        assert!(
            report
                .failed_comparisons
                .values()
                .all(|reason| reason.contains(if invalid_target {
                    "unknown route/target province"
                } else {
                    "censuses disagree"
                }))
        );
    }
}

#[test]
fn naval_demand_rejects_duplicate_and_null_identities_but_preserves_raw_route_zero() {
    let (mut frame, _, _, _) = super::super::tests::fixture();
    let force = (5_u64 << 32) | 51;
    let row = TaskForceRow {
        id: force,
        roster_owner: 101,
        ..Default::default()
    };
    let motion = TaskForceMapMotionRow {
        task_force: force,
        path: vec![0],
        ..Default::default()
    };
    set(&mut frame, TableRows::TaskForces(vec![row]));
    set(
        &mut frame,
        TableRows::TaskForceMapMotion(vec![motion.clone()]),
    );
    *frame.tables[TableId::Provinces.index()] = TableView::Unavailable {
        reason: "unused province domain".into(),
    };
    assert_eq!(navy_owner_slots(&frame).unwrap(), BTreeSet::from([7]));
    set(
        &mut frame,
        TableRows::TaskForceMapMotion(vec![motion.clone(), motion]),
    );
    assert!(
        navy_owner_slots(&frame)
            .unwrap_err()
            .to_string()
            .contains("duplicate motion")
    );
    set(
        &mut frame,
        TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow::default()]),
    );
    assert!(
        navy_owner_slots(&frame)
            .unwrap_err()
            .to_string()
            .contains("null or duplicate motion")
    );
}
