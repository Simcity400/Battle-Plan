use super::*;
use engine_native_air::raid_tooltip_operands::{
    ARMY_RESOLVED,
    NAME,
    REFERENCES,
    WING_NAME,
    WING_RESOLVED,
};
use replay_data::{rows::AirWingRow, rows::TableId, rows::TableRows};
use replay_frame::frame::TableView;

const WING: u64 = (9_u64 << 32) | 70;
const RAID: u64 = (5_u64 << 32) | 4;

fn legacy_frame(bytes: &[u8]) -> DecodedFrame {
    let mut frame = crate::test_fixtures::source(1, 100, Some(1));
    *frame.tables[TableId::AirWings.index()] =
        TableView::Complete(TableRows::AirWings(vec![AirWingRow {
            id: WING,
            name: Recorded::NotRecorded,
            ..Default::default()
        }]));
    *frame.tables[TableId::Raids.index()] = TableView::Complete(TableRows::Raids(vec![RaidRow {
        id: 5,
        reference_type: 4,
        air_wing: WING,
        ..Default::default()
    }]));
    let mut operands: Vec<_> = [
        (REFERENCES, 0, 0),
        (REFERENCES, 1, WING as i64),
        (NAME, ARMY_RESOLVED, 0),
        (NAME, WING_RESOLVED, 1),
        (WING_NAME, u32::MAX, bytes.len() as i64),
    ]
    .into_iter()
    .enumerate()
    .map(|(ordinal, (kind, key, raw))| RaidTooltipOperandRow {
        raid: RAID,
        ordinal: ordinal as u32,
        kind,
        key,
        raw,
    })
    .collect();
    for (index, chunk) in bytes.chunks(8).enumerate() {
        let mut padded = [0; 8];
        padded[..chunk.len()].copy_from_slice(chunk);
        operands.push(RaidTooltipOperandRow {
            raid: RAID,
            ordinal: operands.len() as u32,
            kind: WING_NAME,
            key: index as u32,
            raw: i64::from_le_bytes(padded),
        });
    }
    *frame.tables[TableId::RaidTooltipOperands.index()] =
        TableView::Complete(TableRows::RaidTooltipOperands(operands));
    frame
}

#[test]
fn stored_names_keep_empty_text_and_full_handles() {
    let mut frame = legacy_frame(b"Raid text");
    let requested: HashSet<_> = (1..=24).map(|index| (index << 32) | 70).collect();
    let mut ids: Vec<_> = requested.iter().copied().collect();
    ids.sort_unstable();
    *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(
        ids.into_iter()
            .map(|id| AirWingRow {
                id,
                name: Recorded::Value(Some(if id == WING {
                    Vec::new()
                } else {
                    id.to_string().into_bytes()
                })),
                ..Default::default()
            })
            .collect(),
    ));
    let mut refused = Refusals::default();
    let names = recorded_names(&frame, &requested, &mut refused);
    assert_eq!(names.len(), 24);
    for &id in &requested {
        assert_eq!(
            names[&id],
            if id == WING {
                Vec::new()
            } else {
                id.to_string().into_bytes()
            }
        );
    }
    assert!(refused.lines("wing names").is_empty());

    let TableView::Complete(TableRows::AirWings(wings)) =
        &mut *frame.tables[TableId::AirWings.index()]
    else {
        unreachable!()
    };
    wings.iter_mut().find(|row| row.id == WING).unwrap().name = Recorded::Value(None);
    let mut refused = Refusals::default();
    assert!(recorded_names(&frame, &HashSet::from([WING]), &mut refused).is_empty());
    assert!(refused.lines("wing names")[0].contains("capture failed"));
}

#[test]
fn historical_names_share_authenticated_active_raid_inputs() {
    for bytes in [b"".as_slice(), b"A retained wing name".as_slice()] {
        let frame = legacy_frame(bytes);
        let mut refused = Refusals::default();
        assert_eq!(
            recorded_names(&frame, &HashSet::from([WING]), &mut refused)[&WING],
            bytes
        );
        assert_eq!(raid_name(&frame, WING).as_deref(), Ok(bytes));
        assert!(refused.lines("wing names").is_empty());
    }
    for (kind, key, raw, reason) in [
        (
            REFERENCES,
            1,
            (WING + (1 << 32)) as i64,
            "reference differs",
        ),
        (NAME, ARMY_RESOLVED, 1, "Army-before-wing"),
        (NAME, WING_RESOLVED, 0, "did not resolve"),
        (WING_NAME, u32::MAX, -2, "pointer is null"),
    ] {
        let mut frame = legacy_frame(b"Historical text");
        let TableView::Complete(TableRows::RaidTooltipOperands(rows)) =
            &mut *frame.tables[TableId::RaidTooltipOperands.index()]
        else {
            unreachable!()
        };
        rows.iter_mut()
            .find(|row| row.kind == kind && row.key == key)
            .unwrap()
            .raw = raw;
        let mut refused = Refusals::default();
        assert!(recorded_names(&frame, &HashSet::from([WING]), &mut refused).is_empty());
        assert!(refused.lines("wing names")[0].contains(reason));
    }
    let mut frame = legacy_frame(b"Ended text");
    let TableView::Complete(TableRows::Raids(rows)) = &mut *frame.tables[TableId::Raids.index()]
    else {
        unreachable!()
    };
    rows[0].phase = engine_native_air::craid_instance::PHASE_ENDED;
    let mut refused = Refusals::default();
    assert!(recorded_names(&frame, &HashSet::from([WING]), &mut refused).is_empty());
    assert!(refused.lines("wing names")[0].contains("outside selected active raids"));
}

#[test]
fn incomplete_and_missing_wing_rosters_do_not_supply_guessed_names() {
    let mut frame = legacy_frame(b"Historical text");
    let TableView::Complete(TableRows::AirWings(rows)) =
        &mut *frame.tables[TableId::AirWings.index()]
    else {
        unreachable!()
    };
    rows.clear();
    let mut refused = Refusals::default();
    assert!(recorded_names(&frame, &HashSet::from([WING]), &mut refused).is_empty());
    assert_eq!(
        refused.lines("wing names"),
        [format!(
            "1 wing names not shown: air wings lacks WING {WING:#x}; {WING}"
        )]
    );
    *frame.tables[TableId::AirWings.index()] = TableView::Unavailable {
        reason: "synthetic roster fault".into(),
    };
    let mut refused = Refusals::default();
    assert!(recorded_names(&frame, &HashSet::from([WING]), &mut refused).is_empty());
    assert!(refused.lines("wing names")[0].contains("synthetic roster fault"));
}
