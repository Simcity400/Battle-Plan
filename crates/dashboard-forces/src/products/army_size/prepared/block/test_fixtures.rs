//! The army scenario the block tests and the dashboard's pre-pass tests fold.

use dashboard_base::test_fixtures::{CAMPAIGN, continuation_records, records};
use replay::container::Replay;
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use std::sync::Arc;

/// One hour of the army scenario. Origin's division loses strength at 102;
/// the host's runs at 106 and recovers at 112; the template gains a
/// battalion mid-block at 105 and loses two at 114, a block's first hour.
/// The host has no row in the first two hours of the second block, so the
/// carried state closes it there and the extract meets it again mid-block,
/// with a composition the carried state already versioned.
pub fn scenario(hour: i32) -> DecodedFrame {
    let mut frame = crate::products::army_size::test_fixtures::frame(hour);
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *frame.tables[TableId::Divisions.index()]
    {
        if hour >= 102 {
            rows[1].hit_points = 1_500_000;
        }
        if (106..112).contains(&hour) {
            rows[0].hit_points = 1_000_000;
        }
    }
    if let TableView::Complete(TableRows::DivisionTemplateIcons(rows)) =
        &mut *frame.tables[TableId::DivisionTemplateIcons.index()]
    {
        rows[0].subunits[0].count = match hour {
            ..=104 => 3,
            105..=113 => 4,
            _ => 2,
        };
    }
    if (104..106).contains(&hour)
        && let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
    {
        rows.retain(|row| row.id != 20);
    }
    frame
}

/// The scenario in five blocks: 100-103, 104-107, 110-113 after a gap,
/// 114-117, and 118-121 without 120.
pub fn recording() -> Arc<Replay> {
    let blocks: [&[i32]; 5] = [
        &[100, 101, 102, 103],
        &[104, 105, 106, 107],
        &[110, 111, 112, 113],
        &[114, 115, 116, 117],
        &[118, 119, 121],
    ];
    let frames =
        |hours: &[i32]| -> Vec<DecodedFrame> { hours.iter().map(|&hour| scenario(hour)).collect() };
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames(blocks[0]))]).unwrap();
    for &hours in &blocks[1..] {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(hours))])
            .unwrap();
    }
    assert_eq!(replay.index.len(), blocks.len());
    Arc::new(replay)
}
