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
            },
        );
    }
    for (id, province) in [(13, 2), (14, 1), (15, 1)] {
        saved.task_forces.insert(
            id,
            save::TaskForce {
                id,
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
            },
            FleetRow {
                id: 12,
                owner: 101,
                home_base: 1,
            },
            FleetRow {
                id: 18,
                owner: 101,
                home_base: 1,
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
