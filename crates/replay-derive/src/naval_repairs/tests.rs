use super::*;
use replay_data::rows::*;
use replay_frame::frame::{Metrics, TableView};
fn test_frame(hour: i32) -> DecodedFrame {
    DecodedFrame {
        branch: 1,
        hour,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "unused".into(),
            }
            .into()
        }),
        metrics: Metrics::default(),
    }
}
fn put(frame: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    frame.tables[index] = TableView::Complete(rows).into();
}
#[test]
fn repair_capacity_preserves_queue_membership_repeats_criticals_and_total_level_cap() {
    let mut frame = test_frame(1);
    put(
        &mut frame,
        TableRows::NavalRepairBases(vec![NavalRepairBaseRow {
            province: 7,
            allocated_dockyards: 2,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::NavalRepairQueue(
            vec![0, 90, 1, 2, 3, 2]
                .into_iter()
                .enumerate()
                .map(|(ordinal, ship)| NavalRepairQueueRow {
                    province: 7,
                    ordinal: ordinal as u32,
                    ship,
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::Ships(
            (1..=3)
                .map(|id| ShipRow {
                    id,
                    task_force: 8,
                    strength: 100,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::ShipModels(
            (1..=3)
                .map(|ship| ShipModelRow {
                    ship,
                    has_criticals: Some(u32::from(ship == 2)).into(),
                    counter: Some(ShipCounterInputs {
                        maximum_strength: 100,
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::TaskForces(vec![TaskForceRow {
            id: 8,
            province: 7,
            roster_owner: 11,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::Provinces(vec![ProvinceRow {
            id: 7,
            controller: 11,
            ..Default::default()
        }]),
    );
    let links = BTreeMap::from([(11, 1)]);
    let mut base = NavalBaseRow {
        province: 7,
        level: 10,
        ..Default::default()
    };
    let read =
        |base: &NavalBaseRow| Sources::new(&frame, &links, Err("unused relations")).capacity(base);
    assert_eq!(
        read(&base).unwrap(),
        Capacity {
            used: 2,
            level: 10,
            colour: Some('Y')
        }
    );
    base.level = 1;
    assert_eq!(
        read(&base).unwrap(),
        Capacity {
            used: 1,
            level: 1,
            colour: Some('Y')
        }
    );
    base.countries = vec![11];
    assert_eq!(read(&base).unwrap().used, 0);
    base.countries.clear();
    base.level = 10;
    put(
        &mut frame,
        TableRows::NavalRepairBases(vec![NavalRepairBaseRow {
            province: 7,
            allocated_dockyards: 1,
            ..Default::default()
        }]),
    );
    assert_eq!(
        Sources::new(&frame, &links, Err("unused"))
            .capacity(&base)
            .unwrap(),
        Capacity {
            used: 1,
            level: 10,
            colour: Some('R')
        }
    );
}
#[test]
fn damaged_hull_does_not_need_a_critical_predicate_and_empty_queue_needs_no_ships() {
    let mut frame = test_frame(1);
    put(
        &mut frame,
        TableRows::NavalRepairBases(vec![NavalRepairBaseRow {
            province: 7,
            allocated_dockyards: 0,
            ..Default::default()
        }]),
    );
    put(&mut frame, TableRows::NavalRepairQueue(vec![]));
    let links = BTreeMap::new();
    assert_eq!(
        Sources::new(&frame, &links, Err("unused"))
            .capacity(&NavalBaseRow {
                province: 7,
                level: 10,
                ..Default::default()
            })
            .unwrap(),
        Capacity {
            used: 0,
            level: 10,
            colour: None
        }
    );
    put(
        &mut frame,
        TableRows::NavalRepairQueue(vec![NavalRepairQueueRow {
            province: 7,
            ordinal: 0,
            ship: 1,
        }]),
    );
    put(
        &mut frame,
        TableRows::Ships(vec![ShipRow {
            id: 1,
            task_force: 8,
            strength: 99,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::ShipModels(vec![ShipModelRow {
            ship: 1,
            counter: Some(ShipCounterInputs {
                maximum_strength: 100,
                ..Default::default()
            }),
            has_criticals: None.into(),
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::TaskForces(vec![TaskForceRow {
            id: 8,
            province: 7,
            roster_owner: 11,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::Provinces(vec![ProvinceRow {
            id: 7,
            controller: 11,
            ..Default::default()
        }]),
    );
    let links = BTreeMap::from([(11, 1)]);
    let base = NavalBaseRow {
        province: 7,
        level: 10,
        ..Default::default()
    };
    assert_eq!(
        Sources::new(&frame, &links, Err("unused"))
            .capacity(&base)
            .unwrap()
            .colour,
        Some('R')
    );
    put(
        &mut frame,
        TableRows::Ships(vec![ShipRow {
            id: 1,
            task_force: 8,
            strength: 100,
            ..Default::default()
        }]),
    );
    assert!(
        Sources::new(&frame, &links, Err("unused"))
            .capacity(&base)
            .unwrap_err()
            .contains("critical-damage")
    );
}

fn split_fixture(assigned: i32) -> (DecodedFrame, NavalBaseRow, BTreeMap<u32, u32>) {
    let mut frame = test_frame(1);
    put(
        &mut frame,
        TableRows::NavalRepairBases(vec![NavalRepairBaseRow {
            province: 7,
            allocated_dockyards: assigned,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::NavalRepairQueue(
            [0, 90, 1, 2, 2, 3]
                .into_iter()
                .enumerate()
                .map(|(ordinal, ship)| NavalRepairQueueRow {
                    province: 7,
                    ordinal: ordinal as u32,
                    ship,
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::Ships(
            (1..=3)
                .map(|id| ShipRow {
                    id,
                    task_force: 8,
                    strength: if id == 1 { 99 } else { 100 },
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::ShipModels(
            (1..=2)
                .map(|ship| ShipModelRow {
                    ship,
                    counter: Some(ShipCounterInputs {
                        maximum_strength: 100,
                        ..Default::default()
                    }),
                    has_criticals: if ship == 2 {
                        Some(1).into()
                    } else {
                        None.into()
                    },
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::TaskForces(vec![TaskForceRow {
            id: 8,
            province: 7,
            roster_owner: 11,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::Provinces(vec![ProvinceRow {
            id: 7,
            controller: 11,
            ..Default::default()
        }]),
    );
    (
        frame,
        NavalBaseRow {
            province: 7,
            level: 1,
            ..Default::default()
        },
        BTreeMap::from([(11, 1)]),
    )
}

#[test]
fn repair_split_assigned_prefix_preserves_repeats_criticals_and_ignores_waiting_suffix() {
    let (frame, mut base, links) = split_fixture(3);
    let sources = Sources::new(&frame, &links, Err("unused relations"));
    assert_eq!(
        sources.selected(&base).unwrap(),
        [
            SelectedRepair {
                ordinal: 2,
                ship: 1
            },
            SelectedRepair {
                ordinal: 3,
                ship: 2
            },
            SelectedRepair {
                ordinal: 4,
                ship: 2
            },
        ]
    );
    assert_eq!(
        sources.capacity(&base).unwrap().used,
        1,
        "demand uses total level; assigned selection uses assignment directly"
    );
    base.level = 3;
    let capacity = sources.capacity(&base).unwrap();
    assert_eq!(
        capacity.used as usize,
        sources.selected(&base).unwrap().len()
    );
    assert_eq!(capacity.colour, Some('Y'));
    base.level = 4;
    assert!(
        sources
            .capacity(&base)
            .unwrap_err()
            .contains("model absent")
    );
}

#[test]
fn repair_need_compares_strength_with_a_maximum_of_at_least_100() {
    for stored in [50, 0, -5] {
        let (mut frame, base, links) = split_fixture(1);
        let ships = |strength| {
            TableRows::Ships(vec![ShipRow {
                id: 1,
                task_force: 8,
                strength,
                ..Default::default()
            }])
        };
        put(
            &mut frame,
            TableRows::ShipModels(vec![ShipModelRow {
                ship: 1,
                counter: Some(ShipCounterInputs {
                    maximum_strength: stored,
                    ..Default::default()
                }),
                has_criticals: None.into(),
                ..Default::default()
            }]),
        );
        put(&mut frame, ships(99));
        assert_eq!(
            Sources::new(&frame, &links, Err("unused"))
                .selected(&base)
                .unwrap(),
            [SelectedRepair {
                ordinal: 2,
                ship: 1
            }]
        );
        put(&mut frame, ships(100));
        assert!(
            Sources::new(&frame, &links, Err("unused"))
                .selected(&base)
                .unwrap_err()
                .contains("critical-damage")
        );
    }
}

#[test]
fn repair_split_nonpositive_assignment_needs_no_queue_or_eligibility_operands() {
    for assigned in [0, -1] {
        let mut frame = test_frame(1);
        put(
            &mut frame,
            TableRows::NavalRepairBases(vec![NavalRepairBaseRow {
                province: 7,
                allocated_dockyards: assigned,
                ..Default::default()
            }]),
        );
        assert!(
            Sources::new(&frame, &BTreeMap::new(), Err("unused"))
                .selected(&NavalBaseRow {
                    province: 7,
                    countries_unreadable: true,
                    ..Default::default()
                })
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn repair_split_ally_access_prohibition_and_actual_location_are_demand_gated() {
    let (mut frame, mut base, mut links) = split_fixture(1);
    links.insert(22, 2);
    put(
        &mut frame,
        TableRows::Provinces(vec![ProvinceRow {
            id: 7,
            controller: 22,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::RelationLinks(vec![RelationLinkRow {
            country: 11,
            target_index: 2,
            links: RelationLinkRow::DOCKING_RIGHTS,
            ..Default::default()
        }]),
    );
    put(&mut frame, TableRows::FactionMembers(vec![]));
    put(&mut frame, TableRows::WarRelations(vec![]));
    put(&mut frame, TableRows::Subjects(vec![]));
    let relations = Relations::read(&frame, &links).unwrap();
    assert_eq!(
        Sources::new(&frame, &links, Ok(&relations))
            .selected(&base)
            .unwrap(),
        [SelectedRepair {
            ordinal: 2,
            ship: 1
        }]
    );
    base.countries = vec![11];
    assert!(
        Sources::new(&frame, &links, Err("unused"))
            .selected(&base)
            .unwrap()
            .is_empty()
    );
    base.countries.clear();
    put(
        &mut frame,
        TableRows::TaskForces(vec![TaskForceRow {
            id: 8,
            province: 9,
            roster_owner: 11,
            ..Default::default()
        }]),
    );
    assert!(
        Sources::new(&frame, &links, Ok(&relations))
            .selected(&base)
            .is_err()
    );
    put(
        &mut frame,
        TableRows::NavalRepairQueue(vec![NavalRepairQueueRow {
            province: 7,
            ordinal: 0,
            ship: 1,
        }]),
    );
    assert!(
        Sources::new(&frame, &links, Ok(&relations))
            .selected(&base)
            .unwrap()
            .is_empty()
    );
}
