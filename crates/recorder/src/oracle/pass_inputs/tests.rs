use super::*;
use installed_content::save::counter_inputs;
use replay::rows::{
    CountryCounterInputRow, FleetCounterInputRow, FleetRow, TaskForceCounterInputRow,
    TaskForceMapMotionRow, TaskForceMissionRow, TaskForceRow,
};

fn set(frame: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    *frame.tables[index] = TableView::Complete(rows);
}

/// Singleton rows and the fixture country's inputs, so each test exercises only the
/// tables it changes.
fn base() -> (DecodedFrame, Save) {
    let (mut frame, mut save, _, _) = super::super::tests::fixture();
    set(
        &mut frame,
        TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
            country: 101,
            owned_state_count: 1,
            controlled_state_count: 1,
            ..Default::default()
        }]),
    );
    save.counter_inputs.prides.insert(b"GER".to_vec(), None);
    (frame, save)
}

#[test]
fn first_division_matches_the_first_saved_division() {
    let (mut frame, mut save) = base();
    set(
        &mut frame,
        TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
            country: 101,
            owned_state_count: 1,
            controlled_state_count: 1,
            first_division: (7 << 32) | 4,
            ..Default::default()
        }]),
    );
    let mut report = Report::default();
    countries(&frame, &save, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("first_division"));
    save.counter_inputs
        .first_divisions
        .insert(b"GER".to_vec(), (7 << 32) | 4);
    let mut report = Report::default();
    countries(&frame, &save, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
}

#[test]
fn controlled_state_count_follows_the_saved_controller_or_owner() {
    let (frame, mut save) = base();
    let rows = |report: &Report| {
        report
            .differences
            .iter()
            .filter(|d| d.contains("controlled_state_count"))
            .count()
    };
    let mut report = Report::default();
    compare(&frame, &save, &mut report);
    assert_eq!(rows(&report), 0, "{:?}", report.differences);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("controlled_state_count"))
    );
    // Another controller takes the state: GER controls none, owns one.
    let state = save.states.values_mut().next().unwrap();
    state.controller = Some(b"ENG".to_vec());
    let mut report = Report::default();
    compare(&frame, &save, &mut report);
    assert_eq!(rows(&report), 1, "{:?}", report.differences);
    assert!(
        !report
            .differences
            .iter()
            .any(|d| d.contains("owned_state_count"))
    );
}

#[test]
fn naval_country_and_supply_membership_inputs_compare_with_saved_rules() {
    let (mut frame, mut save) = base();
    let fleet = (1_u64 << 32) | 61;
    let task = (43_u64 << 32) | 51;
    let pride = (32_u64 << 32) | 51;
    set(
        &mut frame,
        TableRows::Fleets(vec![FleetRow {
            id: fleet,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::TaskForces(vec![TaskForceRow {
            id: task,
            fleet,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::FleetCounterInputs(vec![FleetCounterInputRow {
            fleet,
            mission_region_count: 3,
            task_forces: vec![task],
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
            task_force: task,
            ..Default::default()
        }]),
    );
    save.task_forces.insert(
        task,
        installed_content::save::TaskForce {
            id: task,
            fleet,
            roster_owner: b"GER".to_vec(),
            logical_owner: b"GER".to_vec(),
            expeditionary_owner: None,
            province: Some(1),
            mission: 0,
            country_intel: Vec::new(),
            path: Vec::new(),
            target: None,
            alternate_path: Vec::new(),
        },
    );
    set(
        &mut frame,
        TableRows::TaskForceCounterInputs(vec![TaskForceCounterInputRow {
            task_force: task,
            is_in_regions: 255,
            engagement_rule: 2,
            bombardment_region: 206,
            mission_region_count: 5,
            repair_parent: 7,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::TaskForceMissions(vec![TaskForceMissionRow {
            task_force: task,
            owner: 101,
            current_region: 0,
            regions: vec![206, 207, 206, 208, 209],
        }]),
    );
    set(
        &mut frame,
        TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
            country: 101,
            pride_of_the_fleet: pride,
            owned_state_count: 1,
            controlled_state_count: 1,
            ..Default::default()
        }]),
    );
    save.counter_inputs = counter_inputs::Pack {
        sent_volunteers: Default::default(),
        first_divisions: Default::default(),
        prides: [(b"GER".to_vec(), Some(pride))].into(),
        fleets: [(
            fleet,
            counter_inputs::Fleet {
                owner: b"GER".to_vec(),
                color: Some(vec![219, 219, 10]),
                strategic_regions: Some(3),
                task_forces: vec![task],
            },
        )]
        .into(),
        task_forces: [(
            task,
            counter_inputs::TaskForce {
                fleet,
                is_in_regions: Some(true),
                engagement_rule: Some(2),
                bombardment_region: Some(206),
                spotting_region: None,
                accessible_regions: Some(vec![206, 207, 206, 208, 209]),
                repair_parent: None,
                detached_activity: None,
                repair_last_mission: None,
                attached_headquarters: 0,
            },
        )]
        .into(),
        naval_transfers: Default::default(),
    };
    let mut report = Report::default();
    compare(&frame, &save, &mut report);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        report.failed_comparisons.is_empty(),
        "{:?}",
        report.failed_comparisons
    );
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("repair_parent only when"))
    );
    // Saved order, region presence, pride and state ownership are exact.
    let saved = save.counter_inputs.fleets.get_mut(&fleet).unwrap();
    saved.task_forces.push(task + (1 << 32));
    saved.strategic_regions = None;
    save.counter_inputs
        .prides
        .insert(b"GER".to_vec(), Some(pride + 1));
    save.states.clear();
    let task_row = save.counter_inputs.task_forces.get_mut(&task).unwrap();
    task_row.detached_activity = Some(b"refitting".to_vec());
    task_row.repair_last_mission = Some(3);
    // The saved naval_headquarter block's length is the attached-HQ count.
    task_row.attached_headquarters = 1;
    let mut report = Report::default();
    compare(&frame, &save, &mut report);
    // Owned and controlled state counts both differ without states.
    assert_eq!(report.differences.len(), 8, "{:?}", report.differences);
    // An omitted pride is exactly the zero reference.
    save.counter_inputs.prides.insert(b"GER".to_vec(), None);
    let mut report = Report::default();
    compare(&frame, &save, &mut report);
    assert!(
        report
            .differences
            .iter()
            .any(|d| d.contains("pride_of_the_fleet (omitted)")),
        "{:?}",
        report.differences
    );
    // The TrainInventory membership check follows owned_state_count > 0.
    let mut report = Report::default();
    save.economy.supply = Some(Default::default());
    super::super::supply_trains::compare(
        &frame,
        &save,
        &Err("membership not needed".to_string()),
        &mut report,
    );
    assert!(
        report
            .differences
            .iter()
            .any(|d| d.contains("supply-system membership")),
        "{:?}",
        report.differences
    );
}

/// The task force and naval-combat blocks as `CTaskForce::Serialize@140D66770` (through
/// `CUnit::Serialize@140BF7010`) and `CNavalCombatant::Serialize@14160FF40` write them: no
/// combat list on the unit, the side's `unit` references in the combat.
fn serialized(task_force: &str, combats: &str) -> Save {
    installed_content::save::parse(format!(
        "HOI4txt
date=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 countries={{ GER={{units={{fleet={{id={{id=1 type=61}} task_force={{id={{id=43 type=51}} {task_force} location=1 logical_country=\"GER\" mission={{mission=0}}}}}}}}}} }} states={{}} provinces={{}} combat={{{combats}}}"
    ).as_bytes())
    .unwrap()
}

fn naval_combat(id: u32, attacker: &str, defender: &str) -> String {
    format!(
        "naval_combat={{id={{id={id} type=62}} location=1 attacker={{{attacker} positioning=0 anti_air=0}} defender={{{defender} positioning=0 anti_air=0}}}}"
    )
}

#[test]
fn task_force_missions_compare_the_saved_regions_exactly() {
    let (mut frame, mut save) = base();
    let patrol = (6689_u64 << 32) | 51;
    let idle = (8739_u64 << 32) | 51;
    let spotting = (2306_u64 << 32) | 51;
    set(
        &mut frame,
        TableRows::TaskForces(
            [patrol, idle, spotting]
                .map(|id| TaskForceRow {
                    id,
                    ..Default::default()
                })
                .into(),
        ),
    );
    let saved = |spotting_region, accessible_regions| counter_inputs::TaskForce {
        spotting_region,
        accessible_regions,
        ..Default::default()
    };
    // The W3 census's JAP patrol, with a repeat added; a mission with neither field; and one
    // saving only its current region.
    save.counter_inputs.task_forces = [
        (patrol, saved(Some(79), Some(vec![76, 79, 87, 79, 177]))),
        (idle, saved(None, None)),
        (spotting, saved(Some(84), None)),
    ]
    .into();
    let row = |task_force, current_region, regions: &[u32]| TaskForceMissionRow {
        task_force,
        owner: 101,
        current_region,
        regions: regions.to_vec(),
    };
    let run = |frame: &DecodedFrame, save: &Save| {
        let mut report = Report::default();
        report.attempt_table(TableId::TaskForceMissions, |r| {
            task_force_missions(frame, save, r)
        });
        assert!(
            report.failed_comparisons.is_empty(),
            "{:?}",
            report.failed_comparisons
        );
        report.differences
    };
    let exact = [
        row(patrol, 79, &[76, 79, 87, 79, 177]),
        row(spotting, 84, &[]),
    ];
    set(&mut frame, TableRows::TaskForceMissions(exact.to_vec()));
    assert!(run(&frame, &save).is_empty());
    // Same length, a wrong id; the same ids reordered; a repeat collapsed into another id;
    // a wrong current region.
    for (current, regions, field) in [
        (79, &[76, 79, 87, 79, 178][..], "regions"),
        (79, &[79, 76, 87, 79, 177][..], "regions"),
        (79, &[76, 79, 87, 90, 177][..], "regions"),
        (87, &[76, 79, 87, 79, 177][..], "current_region"),
    ] {
        let mut rows = exact.to_vec();
        rows[0] = row(patrol, current, regions);
        set(&mut frame, TableRows::TaskForceMissions(rows));
        let differences = run(&frame, &save);
        assert!(
            differences.len() == 1
                && differences[0].contains(&format!("TaskForceMissions {patrol}.{field}")),
            "{differences:?}"
        );
    }
    // Omission: a missing row where the save wrote a field, and a row where it wrote none.
    set(
        &mut frame,
        TableRows::TaskForceMissions(vec![exact[0].clone(), row(idle, 0, &[5])]),
    );
    let differences = run(&frame, &save);
    assert_eq!(differences.len(), 2, "{differences:?}");
    let extra = format!("TaskForceMissions {idle}: ");
    assert!(
        differences
            .iter()
            .any(|d| d.contains(&extra) && d.contains("no saved spotting_region"))
            && differences
                .iter()
                .any(|d| d.contains(&format!("TaskForceMissions {spotting}"))
                    && d.contains("absent")),
        "{differences:?}"
    );
    // The key sets must be equal: an extra empty row for a task force that saves neither
    // field compares None with None field by field, but is still a row the save has no
    // mission for.
    let mut rows = exact.to_vec();
    rows.push(row(idle, 0, &[]));
    set(&mut frame, TableRows::TaskForceMissions(rows));
    let differences = run(&frame, &save);
    assert!(
        differences.len() == 1
            && differences[0].contains(&extra)
            && differences[0].contains("no saved spotting_region"),
        "{differences:?}"
    );
}

#[test]
fn task_force_motion_compares_routes_target_and_the_saved_combat_side() {
    let (mut frame, mut save) = base();
    let force = (43_u64 << 32) | 51;
    set(
        &mut frame,
        TableRows::TaskForces(vec![TaskForceRow {
            id: force,
            ..Default::default()
        }]),
    );
    let row = TaskForceMapMotionRow {
        task_force: force,
        target: 2,
        combat_role: 1,
        path: vec![2, 1],
        alternate_path: vec![1, 2],
    };
    let us = "unit={id=43 type=51}";
    let mut run = |row: &TaskForceMapMotionRow, saved: &Save| {
        set(&mut frame, TableRows::TaskForceMapMotion(vec![row.clone()]));
        save.task_forces.clone_from(&saved.task_forces);
        save.naval_combats.clone_from(&saved.naval_combats);
        let mut report = Report::default();
        report.attempt_table(TableId::TaskForceMapMotion, |r| {
            task_force_motion(&frame, &save, r)
        });
        assert!(
            report.failed_comparisons.is_empty(),
            "{:?}",
            report.failed_comparisons
        );
        report
    };
    let moving = "support_attack=2 path={ 2 1 } full_path={ 1 2 }";
    let attacking = serialized(moving, &naval_combat(9, us, ""));
    assert!(attacking.task_forces[&force].path == [2, 1]);
    let report = run(&row, &attacking);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(report.limitations.is_empty(), "{:?}", report.limitations);
    // Listed on the defending side, the first combat's role is 2.
    let defending = serialized(moving, &naval_combat(9, "", us));
    let report = run(&row, &defending);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("combat_role"));
    assert!(
        run(
            &TaskForceMapMotionRow {
                combat_role: 2,
                ..row.clone()
            },
            &defending
        )
        .differences
        .is_empty()
    );
    // In no saved combat the role is 0.
    let calm = serialized(moving, &naval_combat(9, "unit={id=44 type=51}", ""));
    assert_eq!(run(&row, &calm).differences.len(), 1);
    assert!(
        run(
            &TaskForceMapMotionRow {
                combat_role: 0,
                ..row.clone()
            },
            &calm
        )
        .differences
        .is_empty()
    );
    // The writer omits a null target and an empty alternate route, and a stored value that
    // disagrees with either is a difference, as is a different ordinary route.
    let standing = serialized("", &naval_combat(9, us, ""));
    let report = run(&row, &standing);
    assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    let still = TaskForceMapMotionRow {
        target: 0,
        path: Vec::new(),
        alternate_path: Vec::new(),
        ..row.clone()
    };
    assert!(run(&still, &standing).differences.is_empty());
    let mut unsaved = attacking.clone();
    unsaved.task_forces.clear();
    assert_eq!(run(&row, &unsaved).differences.len(), 2);
    // TryJoinUnit@1415B2EF0 refuses a unit already in a combat, so a save listing the task
    // force twice, on either side of two combats or twice on one side, is inconsistent input
    // and refused rather than compared.
    for listing in [
        format!("{} {}", naval_combat(9, us, ""), naval_combat(10, "", us)),
        format!("{} {}", naval_combat(9, us, ""), naval_combat(10, us, "")),
        naval_combat(9, &format!("{us} {us}"), ""),
    ] {
        let twice = serialized(moving, &listing);
        let mut report = Report::default();
        report.attempt_table(TableId::TaskForceMapMotion, |r| {
            task_force_motion(&frame, &twice, r)
        });
        assert!(
            report
                .failed_comparisons
                .values()
                .any(|failure| failure.contains("more than once")),
            "{listing}: {:?}",
            report.failed_comparisons
        );
    }
    // A role outside the native three values is a torn row, not a difference.
    set(
        &mut frame,
        TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
            combat_role: 3,
            ..row
        }]),
    );
    let mut report = Report::default();
    report.attempt_table(TableId::TaskForceMapMotion, |r| {
        task_force_motion(&frame, &attacking, r)
    });
    assert!(!report.failed_comparisons.is_empty());
}
