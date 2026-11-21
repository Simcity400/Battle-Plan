use super::*;
use replay_data::{
    rows::NavalCombatDetailsRow, rows::NavalCombatParticipantRow, rows::NavalCombatRow,
    rows::TableRows, rows::TaskForceRow,
};
use replay_frame::{frame::Metrics, frame::TableView};

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
    assert_eq!(side.added, engaged::Added::Unavailable);
    assert_eq!(side.unit_lists, None);
    assert_eq!(side.units, Some(vec![30, 31]));
    *frame.tables[TableId::NavalCombatDetails.index()] =
        TableView::Complete(TableRows::NavalCombatDetails(vec![]));
    let sample = naval_sides(&frame).unwrap();
    assert_eq!(sample.sides[&(62, 1)].units, None);
    assert_eq!(sample.sides[&(62, 1)].lists, None);
}

#[test]
fn an_hour_reads_only_the_task_forces_its_combat_sides_name() {
    const COMBATS: u64 = 3;
    for total in [12_u64, 24] {
        let mut frame = frame();
        let mut details = Vec::new();
        for combat in 0..COMBATS {
            let mut detail = NavalCombatDetailsRow {
                combat,
                ..Default::default()
            };
            for side in 0..2_u8 {
                let force = (combat * 2 + u64::from(side)) * (total / (2 * COMBATS));
                detail.sides[usize::from(side)].units = vec![force];
                detail.sides[usize::from(side)].summary.countries = vec![1];
            }
            details.push(detail);
        }
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
        let sample = naval_sides(&frame).unwrap();
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
        let sample = naval_sides(&frame).unwrap();
        assert!(sample.sides.is_empty() && sample.complete);
    }
}
