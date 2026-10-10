use super::*;
use replay::{
    frame::TableView,
    rows::{
        DivisionRow, FleetRow, LandCombatRow, NavalCombatParticipantRow, NavalCombatRow, ShipRow,
        TableRows, TaskForceRow,
    },
};

fn membership() -> save::PartialMapMembership {
    super::super::state_history_tests::membership(
        "state={id=1 provinces={1} history={IF={limit={} free_operative={operative=a}}}} \
         state={id=2 provinces={2 3}}",
        3,
    )
}

fn set(frame: &mut DecodedFrame, table: TableId, rows: TableRows) {
    *frame.tables[table.index()] = TableView::Complete(rows);
}

fn zero() -> save::Decimal {
    save::Decimal::parse(b"0").unwrap()
}

#[test]
fn division_movement_style_compares_writer_truth_priority_and_local_refusals() {
    use replay::rows::{DivisionMovementRow, Recorded};
    let (mut frame, mut saved, _, _) = super::super::tests::fixture();
    let id = (7_u64 << 32) | 51;
    saved.divisions.clear();
    saved.divisions.insert(
        id,
        save::Division {
            id,
            roster_owner: b"GER".to_vec(),
            logical_owner: b"GER".to_vec(),
            expeditionary_owner: None,
            province: None,
            template: None,
            alternate_template: None,
            separate_counter: false,
            hit_points: zero(),
            organisation: zero(),
            country_intel: Vec::new(),
            presentation: Default::default(),
        },
    );
    let base = DivisionMovementRow {
        division: id,
        withdrawal: Recorded::Value(Some(255)),
        movement_priority: Recorded::Value(Some(1)),
        // The style fixture predates the independently checked fresh ETA packet.
        fresh: Recorded::NotRecorded,
        ..Default::default()
    };
    for (raw, expected) in [
        (0, Some(0)),
        (1, None),
        (2, Some(2)),
        (3, Some(3)),
        (i32::MIN, Some(0)),
    ] {
        saved
            .divisions
            .get_mut(&id)
            .unwrap()
            .presentation
            .withdrawal = true;
        saved
            .divisions
            .get_mut(&id)
            .unwrap()
            .presentation
            .movement_priority = expected;
        set(
            &mut frame,
            TableId::DivisionMovement,
            TableRows::DivisionMovement(vec![DivisionMovementRow {
                movement_priority: Recorded::Value(Some(raw)),
                ..base.clone()
            }]),
        );
        let mut report = Report::default();
        compare_division_movement(&frame, &saved, &mut report).unwrap();
        assert!(
            report.differences.is_empty(),
            "raw {raw}: {:?}",
            report.differences
        );
        assert_eq!(
            report.limitations["DivisionMovement.fresh not recorded"],
            vec![format!("division movement {id}")],
        );
        if raw == i32::MIN {
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("unknown priority word"))
            );
        }
    }
    // Unknown words serialize to front_order but cannot match player_order.
    saved
        .divisions
        .get_mut(&id)
        .unwrap()
        .presentation
        .movement_priority = Some(2);
    let mut report = Report::default();
    compare_division_movement(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("movement_priority"));

    saved
        .divisions
        .get_mut(&id)
        .unwrap()
        .presentation
        .movement_priority = None;
    for (value, differences) in [
        (Recorded::NotRecorded, 0),
        (Recorded::Value(None), 1),
        (Recorded::Value(Some(0)), 1),
    ] {
        set(
            &mut frame,
            TableId::DivisionMovement,
            TableRows::DivisionMovement(vec![DivisionMovementRow {
                withdrawal: value,
                ..base.clone()
            }]),
        );
        let mut report = Report::default();
        compare_division_movement(&frame, &saved, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
    }
    for (value, differences) in [
        (Recorded::NotRecorded, 0),
        (Recorded::Value(None), 1),
        (Recorded::Value(Some(2)), 1),
    ] {
        set(
            &mut frame,
            TableId::DivisionMovement,
            TableRows::DivisionMovement(vec![DivisionMovementRow {
                movement_priority: value,
                source: 99,
                ..base.clone()
            }]),
        );
        let mut report = Report::default();
        compare_division_movement(&frame, &saved, &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            differences + 1,
            "{:?}",
            report.differences
        );
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("source"))
        );
    }
}

#[test]
fn stored_air_wing_name_oracle_compares_bytes_presence_and_capture_refusals() {
    use replay::rows::{AirWingRow, Recorded};
    let (mut frame, mut saved, _, _) = super::super::tests::fixture();
    let id = (1 << 32) | 69;
    let country = frame.complete::<tables::Countries>().unwrap()[0].clone();
    let links = BTreeMap::from([(101, &country)]);
    saved.air.wings.push(save::air::Wing {
        id,
        pool_type: b"fixture_plane".to_vec(),
        name: None,
        accidents: None,
        experience: zero(),
        equipment: save::air::EquipmentPool {
            entries: Vec::new(),
            allow_zero: false,
        },
        ace: None,
        raid: None,
    });
    for (name, expected, differences) in [
        (
            Recorded::Value(Some(vec![0xff, b'\\', b'"', b'W'])),
            Some(vec![0xff, b'\\', b'"', b'W']),
            0,
        ),
        (
            Recorded::Value(Some(vec![0xff, 0, b'W'])),
            // 1424AF5A0 stops at the first NUL; the suffix is unsaved.
            Some(vec![0xff]),
            0,
        ),
        (Recorded::Value(Some(Vec::new())), Some(Vec::new()), 0),
        (Recorded::Value(Some(Vec::new())), None, 1),
        (Recorded::Value(None), Some(Vec::new()), 1),
        (Recorded::NotRecorded, Some(b"Old save name".to_vec()), 0),
        (Recorded::NotRecorded, None, 1),
    ] {
        let suffix = matches!(&name, Recorded::Value(Some(bytes)) if bytes.contains(&0));
        saved.air_wings.insert(
            id,
            save::AirWing {
                id,
                owner: b"GER".to_vec(),
                name: expected,
                country_override: b"---".to_vec(),
                air_base: 0,
                planes: 1,
                mission_mask: 0,
                mission_region: None,
                mission_active: None,
                region_change_penalty: None,
                pool_ordinal: 0,
            },
        );
        set(
            &mut frame,
            TableId::AirWings,
            TableRows::AirWings(vec![AirWingRow {
                id,
                owner: 101,
                planes: 1,
                name,
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        compare_air_wings(&frame, &saved, &links, &mut report).unwrap();
        if suffix {
            assert!(
                report
                    .limitations
                    .values()
                    .flatten()
                    .any(|field| field.ends_with(".name"))
            );
        }
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
        if differences != 0 {
            assert!(report.differences[0].contains(".name"));
        }
    }
}

#[test]
fn division_country_ordinal_compares_the_full_saved_roster() {
    let (mut frame, mut saved, _, _) = super::super::tests::fixture();
    let division = |id: u64| save::Division {
        id: (id << 32) | 4713,
        roster_owner: b"GER".to_vec(),
        logical_owner: b"GER".to_vec(),
        expeditionary_owner: None,
        province: None,
        template: None,
        alternate_template: None,
        separate_counter: false,
        hit_points: zero(),
        organisation: zero(),
        country_intel: Vec::new(),
        presentation: save::DivisionPresentation {
            max_supply: Some(zero()),
            army_current_supply_ratio: Some(zero()),
            ..Default::default()
        },
    };
    let first = (1 << 32) | 4713;
    let second = (2 << 32) | 4713;
    saved.divisions.insert(first, division(1));
    saved.divisions.insert(second, division(2));
    saved
        .counter_inputs
        .division_orders
        .insert(b"GER".to_vec(), vec![second, first]);
    let mut rows = vec![
        replay::rows::DivisionStatusRow {
            division: first,
            country_ordinal: replay::rows::Recorded::Value(Some(1)),
            supply: replay::rows::Recorded::NotRecorded,
            ..Default::default()
        },
        replay::rows::DivisionStatusRow {
            division: second,
            country_ordinal: replay::rows::Recorded::Value(Some(0)),
            supply: replay::rows::Recorded::NotRecorded,
            ..Default::default()
        },
    ];
    set(
        &mut frame,
        TableId::DivisionStatus,
        TableRows::DivisionStatus(rows.clone()),
    );
    let mut report = Report::default();
    compare_division_status(&frame, &saved, &BTreeMap::new(), &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    rows[0].country_ordinal = replay::rows::Recorded::Value(Some(0));
    set(
        &mut frame,
        TableId::DivisionStatus,
        TableRows::DivisionStatus(rows),
    );
    compare_division_status(&frame, &saved, &BTreeMap::new(), &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("country_ordinal"));
}

#[test]
fn division_disrupted_supply_oracle_checks_positive_presence_and_preserves_failures() {
    use replay::rows::{DivisionStatusRow, Recorded, UnitSupplyInputs};
    let (mut frame, mut saved, _, _) = super::super::tests::fixture();
    let id = (1_u64 << 32) | 51;
    saved.divisions.clear();
    saved.divisions.insert(
        id,
        save::Division {
            id,
            roster_owner: b"X01".to_vec(),
            logical_owner: b"X01".to_vec(),
            expeditionary_owner: None,
            province: None,
            template: None,
            alternate_template: None,
            separate_counter: false,
            hit_points: zero(),
            organisation: zero(),
            country_intel: Vec::new(),
            presentation: save::DivisionPresentation {
                max_supply: Some(zero()),
                army_current_supply_ratio: Some(zero()),
                ..Default::default()
            },
        },
    );
    let value = |disruption| {
        Recorded::Value(Some(UnitSupplyInputs {
            disruption,
            // Independent origin acquisition failure does not erase this scalar.
            origins: None,
            ..Default::default()
        }))
    };
    for (supply, text, differences) in [
        (value(9_007_199_254_740_993), Some("90071992547.40993"), 0),
        (value(9_007_199_254_740_992), Some("90071992547.40993"), 1),
        (value(0), None, 0),
        (value(-100_001), None, 0),
        (value(1), None, 1),
        (value(0), Some("0.00001"), 1),
        (Recorded::Value(None), None, 1),
        (Recorded::Value(None), Some("0.00001"), 1),
        (Recorded::NotRecorded, Some("0.00001"), 0),
    ] {
        let old = matches!(supply, Recorded::NotRecorded);
        saved
            .divisions
            .get_mut(&id)
            .unwrap()
            .presentation
            .disrupted_supply = text.map(|text| save::Decimal::parse(text.as_bytes()).unwrap());
        set(
            &mut frame,
            TableId::DivisionStatus,
            TableRows::DivisionStatus(vec![DivisionStatusRow {
                division: id,
                supply,
                ..Default::default()
            }]),
        );
        let mut report = Report::default();
        compare_division_status(&frame, &saved, &BTreeMap::new(), &mut report).unwrap();
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
        assert!(
            report
                .differences
                .iter()
                .all(|line| line.contains("supply.disrupted_supply"))
        );
        if old {
            assert!(
                report
                    .limitations
                    .contains_key("DivisionStatus.supply not recorded")
            );
        }
    }
    // A failed appended scalar does not prevent independent prefix comparison.
    saved.divisions.get_mut(&id).unwrap().presentation.dig_in =
        Some(save::Decimal::parse(b"1").unwrap());
    set(
        &mut frame,
        TableId::DivisionStatus,
        TableRows::DivisionStatus(vec![DivisionStatusRow {
            division: id,
            supply: Recorded::Value(None),
            ..Default::default()
        }]),
    );
    let mut report = Report::default();
    compare_division_status(&frame, &saved, &BTreeMap::new(), &mut report).unwrap();
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains(".entrenchment"))
    );
}

#[test]
fn land_combats_compare_locations_and_missing_rows_despite_membership_exclusions() {
    let membership = membership();
    let (mut frame, mut saved, _, _) = super::super::tests::fixture();
    for (id, province) in [(11, 1), (12, 1), (13, 2), (14, 2), (17, 2)] {
        saved.land_combats.insert(
            id,
            save::LandCombat {
                id,
                province,
                day: 0,
                duration: 0,
                attackers: Vec::new(),
                defenders: Vec::new(),
                details: Default::default(),
            },
        );
    }
    set(
        &mut frame,
        TableId::LandCombats,
        TableRows::LandCombats(
            [(11, 1), (13, 1), (14, 2), (15, 1), (16, 2)]
                .into_iter()
                .map(|(id, province)| LandCombatRow {
                    id,
                    province,
                    day: 1,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    let mut report = Report::default();
    compare_land_combats(&frame, &saved, &membership, &mut report).unwrap();
    assert_eq!(report.land_combats, 3);
    assert_differences(
        &report,
        &[
            "type=11 id=0.day",
            "type=12 id=0",
            "type=13 id=0.province",
            "type=13 id=0.day",
            "type=14 id=0.day",
            "type=15 id=0",
            "type=16 id=0",
            "type=17 id=0",
        ],
    );
    assert!(report.membership_skips.is_empty());
}

#[test]
fn divisions_compare_locations_strength_and_missing_rows_despite_membership_exclusions() {
    let membership = membership();
    let (mut frame, mut saved, _, _) = super::super::tests::fixture();
    let owner = saved.countries.keys().next().unwrap().clone();
    for (id, province) in [(11, 1), (12, 2), (13, 2)] {
        saved.divisions.insert(
            id,
            save::Division {
                id,
                roster_owner: owner.clone(),
                logical_owner: owner.clone(),
                expeditionary_owner: None,
                province: Some(province),
                template: None,
                alternate_template: None,
                separate_counter: false,
                hit_points: zero(),
                organisation: zero(),
                country_intel: Vec::new(),
                presentation: Default::default(),
            },
        );
    }
    set(
        &mut frame,
        TableId::Divisions,
        TableRows::Divisions(
            [(12, 1), (13, 2), (14, 1)]
                .into_iter()
                .map(|(id, province)| DivisionRow {
                    id,
                    province,
                    roster_owner: 101,
                    logical_owner: 101,
                    hit_points: 1,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    let context = super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    compare_divisions(&frame, &saved, &membership, &context.links, &mut report).unwrap();
    assert_eq!(report.divisions, 2);
    assert_differences(
        &report,
        &[
            "type=11 id=0",
            "type=12 id=0.province",
            "type=12 id=0.hit_points",
            "type=13 id=0.hit_points",
            "type=14 id=0",
        ],
    );
    assert!(report.membership_skips.is_empty());
}

#[test]
fn naval_units_compare_locations_and_missing_rows_despite_membership_exclusions() {
    let membership = membership();
    let (mut frame, mut saved, _, _) = super::super::tests::fixture();
    let owner = saved.countries.keys().next().unwrap().clone();
    for (id, home_base) in [(11, 1), (12, 2), (17, 1)] {
        saved.fleets.insert(
            id,
            save::Fleet {
                id,
                owner: owner.clone(),
                home_base: Some(home_base),
                name: Some(Vec::new()),
                leader: None,
            },
        );
    }
    for (id, province) in [(13, 2), (14, 1), (15, 1)] {
        saved.task_forces.insert(
            id,
            save::TaskForce {
                id,
                name: None,
                fleet: 11,
                roster_owner: owner.clone(),
                logical_owner: owner.clone(),
                expeditionary_owner: None,
                province: Some(province),
                mission: 0,
                country_intel: Vec::new(),
                path: Vec::new(),
                target: None,
                alternate_path: Vec::new(),
                movement_progress: None,
                retreat: false,
                withdrawal: false,
                exile: false,
            },
        );
    }
    saved.ships.insert(
        15,
        save::Ship {
            id: 15,
            roster_ordinal: 0,
            task_force: 14,
            strength: zero(),
            organisation: zero(),
            experience: zero(),
            sunk_convoys: 0,
            equipment: None,
        },
    );
    set(
        &mut frame,
        TableId::Fleets,
        TableRows::Fleets(vec![
            FleetRow {
                id: 11,
                owner: 101,
                home_base: 1,
                name: Some(Vec::new()).into(),
                leader: Some(0).into(),
            },
            FleetRow {
                id: 12,
                owner: 101,
                home_base: 1,
                name: Some(Vec::new()).into(),
                leader: Some(0).into(),
            },
            FleetRow {
                id: 18,
                owner: 101,
                home_base: 1,
                name: Some(Vec::new()).into(),
                leader: Some(0).into(),
            },
        ]),
    );
    set(
        &mut frame,
        TableId::TaskForces,
        TableRows::TaskForces(
            [(13, 1), (14, 1), (16, 1)]
                .into_iter()
                .map(|(id, province)| TaskForceRow {
                    id,
                    fleet: 11,
                    province,
                    roster_owner: 101,
                    logical_owner: 101,
                    mission: 1,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        &mut frame,
        TableId::Ships,
        TableRows::Ships(vec![ShipRow {
            id: 15,
            task_force: 14,
            strength: 1,
            ..Default::default()
        }]),
    );
    let context = super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    compare_naval_units(&frame, &saved, &membership, &context.links, &mut report).unwrap();
    assert_eq!((report.fleets, report.task_forces, report.ships), (2, 2, 1));
    assert_differences(
        &report,
        &[
            "type=12 id=0.home_base",
            "fleet type=17 id=0",
            "fleet 18",
            "type=13 id=0.province",
            "type=13 id=0.mission",
            "type=14 id=0.mission",
            "task force type=15 id=0",
            "task force 16",
            "type=15 id=0.strength",
        ],
    );
    assert!(report.membership_skips.is_empty());
}

#[test]
fn naval_combats_compare_locations_and_missing_rows_despite_membership_exclusions() {
    let membership = membership();
    let (mut frame, mut saved, _, _) = super::super::tests::fixture();
    for (id, province) in [(11, 1), (12, 2), (13, 1), (14, 2)] {
        saved.naval_combats.insert(
            id,
            save::NavalCombat {
                id,
                province,
                attackers: Vec::new(),
                defenders: Vec::new(),
                attacker_positioning: zero(),
                defender_positioning: zero(),
                attacker_anti_air: zero(),
                defender_anti_air: zero(),
                details: Default::default(),
                transfers: Vec::new(),
            },
        );
    }
    saved.naval_combat_participants.insert(
        (11, 0, 0),
        save::NavalCombatParticipant {
            combat: 11,
            side: 0,
            battle_slot: 0,
            ship: None,
            convoy: None,
            owner: saved.countries.keys().next().unwrap().clone(),
            state: 0,
            details: Default::default(),
        },
    );
    set(
        &mut frame,
        TableId::NavalCombats,
        TableRows::NavalCombats(
            [(11, 1), (12, 1), (15, 1), (16, 2)]
                .into_iter()
                .enumerate()
                .map(|(ordinal, (id, province))| NavalCombatRow {
                    id,
                    province,
                    attacker_positioning: 1,
                    registry_ordinal: ordinal as u32,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        &mut frame,
        TableId::NavalCombatParticipants,
        TableRows::NavalCombatParticipants(vec![NavalCombatParticipantRow {
            combat: 11,
            side: 0,
            battle_slot: 0,
            owner: 101,
            state: 1,
            ..Default::default()
        }]),
    );
    let context = super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    compare_naval_combats(&frame, &saved, &membership, &context.links, &mut report).unwrap();
    assert_eq!(
        (report.naval_combats, report.naval_combat_participants),
        (2, 1)
    );
    assert_differences(
        &report,
        &[
            "type=11 id=0.attacker_positioning",
            "type=12 id=0.province",
            "type=12 id=0.attacker_positioning",
            "naval combat type=13 id=0",
            "naval combat type=14 id=0",
            "naval combat 15",
            "naval combat 16",
            "naval participant combat=11 side=0 slot=0.state",
        ],
    );
    assert!(report.membership_skips.is_empty());
}

fn assert_differences(report: &Report, fields: &[&str]) {
    assert_eq!(
        report.differences.len(),
        fields.len(),
        "{:?}",
        report.differences
    );
    for field in fields {
        assert!(
            report.differences.iter().any(|line| line.contains(field)),
            "{field}: {:?}",
            report.differences
        );
    }
}

#[test]
fn fleet_name_is_mandatory_in_ordinary_save_even_for_historical_recordings() {
    let (mut frame, mut saved, _, _) = super::super::tests::fixture();
    let membership = membership();
    let owner = saved.countries.keys().next().unwrap().clone();
    saved.fleets.clear();
    saved.task_forces.clear();
    saved.ships.clear();
    set(
        &mut frame,
        TableId::TaskForces,
        TableRows::TaskForces(Vec::new()),
    );
    set(&mut frame, TableId::Ships, TableRows::Ships(Vec::new()));
    for name in [Recorded::NotRecorded, Recorded::Value(Some(Vec::new()))] {
        set(
            &mut frame,
            TableId::Fleets,
            TableRows::Fleets(vec![FleetRow {
                id: 17,
                owner: 101,
                home_base: 1,
                name,
                leader: Recorded::Value(Some(0)),
            }]),
        );
        saved.fleets.insert(
            17,
            save::Fleet {
                id: 17,
                owner: owner.clone(),
                home_base: Some(1),
                name: Some(Vec::new()),
                leader: None,
            },
        );
        let context = super::super::country_context(&frame).unwrap();
        let mut report = Report::default();
        compare_naval_units(&frame, &saved, &membership, &context.links, &mut report).unwrap();
        assert!(report.differences.is_empty());
        saved.fleets.get_mut(&17).unwrap().name = None;
        assert!(
            compare_naval_units(
                &frame,
                &saved,
                &membership,
                &context.links,
                &mut Report::default()
            )
            .unwrap_err()
            .to_string()
            .contains("fleet stored name missing")
        );
    }
}
