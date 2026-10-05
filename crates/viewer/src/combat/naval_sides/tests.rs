use super::*;
use replay::{
    frame::{Metrics, TableView},
    rows::{
        NavalCombatDetailsRow, NavalCombatParticipantRow, NavalCombatRow, TableRows, TaskForceRow,
    },
};

fn frame() -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 0,
        hour: 7,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "not needed".into(),
            }
            .into()
        }),
        metrics: Metrics::default(),
    };
    let mut detail = NavalCombatDetailsRow {
        combat: 62,
        ..Default::default()
    };
    detail.sides[0].units = vec![31, 30];
    detail.sides[0].summary.countries = vec![4];
    detail.sides[0].summary.secondary_countries = vec![5];
    detail.sides[1].units = vec![40];
    let set = |frame: &mut DecodedFrame, table: TableId, rows: TableRows| {
        *frame.tables[table.index()] = TableView::Complete(rows);
    };
    set(
        &mut frame,
        TableId::NavalCombats,
        TableRows::NavalCombats(vec![NavalCombatRow {
            id: 62,
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableId::NavalCombatDetails,
        TableRows::NavalCombatDetails(vec![detail]),
    );
    let member = |side, battle_slot, ship| NavalCombatParticipantRow {
        combat: 62,
        side,
        battle_slot,
        ship,
        ..Default::default()
    };
    set(
        &mut frame,
        TableId::NavalCombatParticipants,
        TableRows::NavalCombatParticipants(vec![
            member(0, 9, 0),
            member(0, 3, 0),
            member(0, 4, 77),
            member(1, 5, 0),
        ]),
    );
    let force = |id, logical_owner, roster_owner| TaskForceRow {
        id,
        logical_owner,
        roster_owner,
        ..Default::default()
    };
    set(
        &mut frame,
        TableId::TaskForces,
        TableRows::TaskForces(vec![
            force(30, 4, 4),
            force(31, 6, u32::MAX),
            force(40, 7, 7),
        ]),
    );
    frame
}

/// Units sorted, the recorded lists, the convoy slots and the lists a
/// removal rebuilds from the remaining task forces' owners.
#[test]
fn a_naval_sample_keys_both_sides_from_their_task_forces_and_convoys() {
    let sample = naval_sides(&frame()).unwrap();
    assert!(sample.complete);
    assert_eq!(sample.removal, Removal::BothSides);
    assert_eq!(
        sample.sides[&(62, 0)],
        engaged::Side {
            units: Some(vec![30, 31]),
            added: engaged::Added::Observed(vec![3, 9]),
            lists: Some((vec![4], vec![5])),
            // Logical owners in stored order; a nonpositive raw roster owner is left out.
            unit_lists: Some((vec![6, 4], vec![4])),
        }
    );
    assert_eq!(
        sample.sides[&(62, 1)].added,
        engaged::Added::Observed(vec![5])
    );
    assert_eq!(sample.sides[&(62, 1)].unit_lists, Some((vec![7], vec![7])));
}

#[test]
fn missing_inputs_leave_their_part_unknown() {
    let mut frame = frame();
    *frame.tables[TableId::NavalCombatParticipants.index()] = TableView::Unavailable {
        reason: "not captured".into(),
    };
    if let TableView::Complete(TableRows::TaskForces(rows)) =
        &mut *frame.tables[TableId::TaskForces.index()]
    {
        rows.retain(|force| force.id != 31);
    }
    let sample = naval_sides(&frame).unwrap();
    let side = &sample.sides[&(62, 0)];
    // An unrecorded census is not an hour without convoys.
    assert_eq!(side.added, engaged::Added::Unavailable);
    assert_eq!(side.unit_lists, None);
    assert_eq!(side.units, Some(vec![30, 31]));
    // A combat without its details row has no units or lists.
    *frame.tables[TableId::NavalCombatDetails.index()] =
        TableView::Complete(TableRows::NavalCombatDetails(vec![]));
    let sample = naval_sides(&frame).unwrap();
    assert_eq!(sample.sides[&(62, 1)].units, None);
    assert_eq!(sample.sides[&(62, 1)].lists, None);
}

/// 1,500 combats between 5,000 countries' task forces: one pass over each
/// table, every side keyed.
#[test]
fn fifteen_hundred_combats_of_five_thousand_countries_build_in_one_pass() {
    const COMBATS: u64 = 1_500;
    const COUNTRIES: u32 = 5_000;
    let mut frame = frame();
    let mut details = Vec::new();
    let mut members = Vec::new();
    let mut forces = Vec::new();
    for combat in 0..COMBATS {
        let mut detail = NavalCombatDetailsRow {
            combat,
            ..Default::default()
        };
        for side in 0..2_u8 {
            let force = combat * 2 + u64::from(side);
            let country = (force as u32 % COUNTRIES) + 1;
            detail.sides[usize::from(side)].units = vec![force];
            detail.sides[usize::from(side)].summary.countries = vec![country];
            forces.push(TaskForceRow {
                id: force,
                logical_owner: country,
                roster_owner: country,
                ..Default::default()
            });
            members.push(NavalCombatParticipantRow {
                combat,
                side,
                battle_slot: 1,
                ..Default::default()
            });
        }
        details.push(detail);
    }
    *frame.tables[TableId::NavalCombats.index()] = TableView::Complete(TableRows::NavalCombats(
        (0..COMBATS)
            .map(|id| NavalCombatRow {
                id,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::NavalCombatDetails.index()] =
        TableView::Complete(TableRows::NavalCombatDetails(details));
    *frame.tables[TableId::NavalCombatParticipants.index()] =
        TableView::Complete(TableRows::NavalCombatParticipants(members));
    *frame.tables[TableId::TaskForces.index()] = TableView::Complete(TableRows::TaskForces(forces));
    let started = std::time::Instant::now();
    let sample = naval_sides(&frame).unwrap();
    println!(
        "{COMBATS} naval combats: sides built in {:.3} ms",
        started.elapsed().as_secs_f64() * 1e3
    );
    assert_eq!(sample.sides.len(), 2 * COMBATS as usize);
    assert!(
        sample.sides.values().all(
            |side| side.unit_lists.is_some() && side.added == engaged::Added::Observed(vec![1])
        )
    );
}

/// 20,000 and 40,000 task forces (the ship target and twice it, a task force per ship at
/// most): an hour reads only the task forces its 1,500 combats' sides name, and an hour
/// without naval combat reads none.
#[test]
fn an_hour_reads_only_the_task_forces_its_combat_sides_name() {
    const COMBATS: u64 = 1_500;
    for total in [20_000_u64, 40_000] {
        let mut frame = frame();
        let mut details = Vec::new();
        for combat in 0..COMBATS {
            let mut detail = NavalCombatDetailsRow {
                combat,
                ..Default::default()
            };
            for side in 0..2_u8 {
                // Spread over the whole census, as a real hour's are.
                let force = (combat * 2 + u64::from(side)) * (total / (2 * COMBATS));
                detail.sides[usize::from(side)].units = vec![force];
                detail.sides[usize::from(side)].summary.countries = vec![1];
            }
            details.push(detail);
        }
        // In key order, as a decoded census is; task force 0 has no positive roster owner.
        let forces: Vec<TaskForceRow> = (0..total)
            .map(|id| TaskForceRow {
                id,
                logical_owner: if id == 0 { 9 } else { 1 },
                roster_owner: u32::from(id != 0),
                ..Default::default()
            })
            .collect();
        *frame.tables[TableId::NavalCombats.index()] =
            TableView::Complete(TableRows::NavalCombats(
                (0..COMBATS)
                    .map(|id| NavalCombatRow {
                        id,
                        ..Default::default()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::NavalCombatDetails.index()] =
            TableView::Complete(TableRows::NavalCombatDetails(details));
        *frame.tables[TableId::NavalCombatParticipants.index()] =
            TableView::Complete(TableRows::NavalCombatParticipants(Vec::new()));
        *frame.tables[TableId::TaskForces.index()] =
            TableView::Complete(TableRows::TaskForces(forces));
        let started = std::time::Instant::now();
        let sample = naval_sides(&frame).unwrap();
        let combat_hour = started.elapsed();
        assert_eq!(sample.sides.len(), 2 * COMBATS as usize);
        assert!(
            sample
                .sides
                .iter()
                .filter(|(key, _)| **key != (0, 0))
                .all(|(_, side)| side.unit_lists == Some((vec![1], vec![1])))
        );
        assert_eq!(sample.sides[&(0, 0)].unit_lists, Some((vec![9], vec![])));
        *frame.tables[TableId::NavalCombats.index()] =
            TableView::Complete(TableRows::NavalCombats(Vec::new()));
        let started = std::time::Instant::now();
        let sample = naval_sides(&frame).unwrap();
        let quiet_hour = started.elapsed();
        assert!(sample.sides.is_empty() && sample.complete);
        println!(
            "{total} task forces: an hour of {COMBATS} naval combats in {:.3} ms, an hour \
             without naval combat in {:.3} ms",
            combat_hour.as_secs_f64() * 1e3,
            quiet_hour.as_secs_f64() * 1e3
        );
    }
}
