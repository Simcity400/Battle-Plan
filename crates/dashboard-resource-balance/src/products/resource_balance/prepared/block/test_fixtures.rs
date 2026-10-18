//! The resource scenario the block tests and the dashboard's pre-pass tests
//! fold.

use dashboard_base::test_fixtures::{CAMPAIGN, continuation_records, records, resource_frame};
use replay::container::Replay;
use replay_data::{
    rows::CountryRow, rows::LexicalTokenRow, rows::ResourceDefinitionRow, rows::TableId,
    rows::TableRows,
};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use std::sync::Arc;

fn countries(frame: &mut DecodedFrame) -> &mut Vec<CountryRow> {
    match &mut *frame.tables[TableId::Countries.index()] {
        TableView::Complete(TableRows::Countries(rows)) => rows,
        _ => unreachable!("fixture countries"),
    }
}

/// One hour of the resource scenario. Germany imports steel from Italy and,
/// from hour 106, from Spain; Italy's terms change at 115. Germany has no row
/// in the first two hours of the second block, so the carried state closes
/// it at that block's first hour and the extract first meets it mid-block,
/// with an oil membership that stays empty. Germany's holder changes at 110,
/// after a gap, and coal is defined from 112, mid-block. Deals are unread at
/// 111 and at 118, a block's first hour, and pools at 116. At `lost` the
/// dictionary does not spell oil.
pub fn scenario(hour: i32, lost: Option<i32>) -> DecodedFrame {
    let deals: &[(u64, u32, u32, i64)] = match hour {
        ..=105 => &[(51, 21, 11, 12)],
        106..=114 => &[(51, 21, 11, 12), (52, 31, 11, 5)],
        _ => &[(51, 21, 11, 13), (52, 31, 11, 5)],
    };
    let steel = if hour < 102 { -100 } else { 300 };
    let germany = if hour < 110 { 9001 } else { 9002 };
    let mut frame = resource_frame(hour, steel, germany, deals);
    if (104..106).contains(&hour) {
        countries(&mut frame).retain(|row| row.tag != b"GER");
    }
    if let TableView::Complete(TableRows::LexicalTokens(rows)) =
        &mut *frame.tables[TableId::LexicalTokens.index()]
    {
        if hour >= 112 {
            rows.push(LexicalTokenRow {
                token: 503,
                literal: b"coal".to_vec(),
            });
        }
        if lost == Some(hour) {
            rows.retain(|row| row.token != 502);
        }
    }
    if hour >= 112
        && let TableView::Complete(TableRows::ResourceDefinitions(rows)) =
            &mut *frame.tables[TableId::ResourceDefinitions.index()]
    {
        rows.push(ResourceDefinitionRow {
            id: 3,
            token: 503,
            active: 1,
            factory_cost: 12_500,
        });
    }
    let unread = |table: TableId| match hour {
        111 | 118 => table == TableId::ResourceExchanges,
        116 => table == TableId::CountryResources,
        _ => false,
    };
    for table in [TableId::ResourceExchanges, TableId::CountryResources] {
        if unread(table) {
            *frame.tables[table.index()] = TableView::Unavailable {
                reason: "synthetic missing resource read".into(),
            };
        }
    }
    frame
}

/// The scenario in five blocks, each after the first an appended segment:
/// 100-103, 104-107, 110-113 after a gap, 114-117, and 118-121 without 120.
pub fn recording(lost: Option<i32>) -> Arc<Replay> {
    let blocks: [&[i32]; 5] = [
        &[100, 101, 102, 103],
        &[104, 105, 106, 107],
        &[110, 111, 112, 113],
        &[114, 115, 116, 117],
        &[118, 119, 121],
    ];
    let frames = |hours: &[i32]| -> Vec<DecodedFrame> {
        hours.iter().map(|&hour| scenario(hour, lost)).collect()
    };
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
