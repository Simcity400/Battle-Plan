use super::*;
use crate::test_fixtures::{CAMPAIGN, records, source};
use replay::campaign::Replay;
use replay_data::rows::{ScriptVariableValueRow, TableRows};
use super::super::{ReadDates, TableRead};
use replay_frame::frame::TableView;

fn replay(last: i32) -> Replay {
    let frames = (100..=last).map(|hour| {
        let mut frame = source(1, hour, Some(1_000));
        let mut rows = vec![
            ScriptVariableValueRow { scope_kind: 1, scope_id: 1, var_id: 1, value_raw: i64::from(hour >= 103) },
            ScriptVariableValueRow { scope_kind: 1, scope_id: 2, var_id: 1, value_raw: i64::from(hour >= 105) },
        ];
        if hour >= 107 { rows.remove(0); }
        frame.tables[TABLE.index()] = if hour == 109 {
            TableView::Truncated { rows: TableRows::ScriptVariableValues(Vec::new()), missing: 1 }.into()
        } else {
            TableView::Complete(TableRows::ScriptVariableValues(rows)).into()
        };
        frame.metrics.table_ns[TABLE.index()] = 1;
        frame
    }).collect();
    Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap()
}

fn selection(scope_id: u32) -> Selection {
    let first = (1_u128 << 64) | (u128::from(scope_id) << 32);
    Selection { fields: 1, rows: Some(vec![(first, first | u128::from(u32::MAX))]), ends: false }
}

#[test]
fn new_variable_scope_selections_lookup_exact_dates_without_reopening_storage() {
    let replay = replay(110);
    let stop = AtomicBool::new(false);
    let mut dates = VariableDates::default();
    assert!(dates.dates(replay.recording(), 0, 100, 110, &selection(1), &stop).unwrap().is_some());
    for first in [100, 103, 108] {
        let _ = dates.dates(replay.recording(), 0, first, 110, &selection(1), &stop).unwrap().unwrap();
        for scope in [1, 2] {
            let selection = selection(scope);
            let expected = selection.dates(&replay, 0, TABLE, first, 110).unwrap();
            let (found, reads) = replay_recording::cells::count_reads(|| dates.dates(replay.recording(), 0, first, 110, &selection, &stop));
            let upcoming = |hours: Vec<i32>| hours.into_iter().filter(|&hour| hour > first).collect::<Vec<_>>();
            assert_eq!(upcoming(found.unwrap().unwrap()), upcoming(expected));
            assert!(reads.get(&TABLE).is_none_or(|read| read.opened == 0 && read.restores == 0));
        }
    }
    let mut single = selection(2);
    single.rows.as_mut().unwrap()[0].0 += 1;
    assert!(dates.dates(replay.recording(), 0, 100, 110, &single, &stop).unwrap().is_none());
    assert!(Index::load(replay.recording(), 0, &stop, 1).unwrap().is_none());
}

#[test]
fn a_damaged_later_restore_keeps_the_legacy_block_proof() {
    use replay_columnar::unit::{BLOCK, restore::column_stream};
    let full = replay(100 + 6 * BLOCK as i32 + 3);
    let recording = full.recording();
    let page = recording.table_entry(0, 5, TABLE.index()).unwrap();
    let restore = page.unit.restore.as_ref().unwrap();
    let (_, chunks) = restore.streams.iter().find(|(stream, _)| *stream == column_stream(0)).unwrap();
    let frame = &chunks[0].frame;
    let mut bytes = full.bytes().to_vec();
    bytes[(frame.offset + u64::from(frame.stored) / 2) as usize] ^= 0x80;
    let damaged = Replay::open(bytes).unwrap();
    let stop = AtomicBool::new(false);
    let reads = [TableRead::fields(TABLE, &["value_raw"])];
    let first = 100 + 6 * BLOCK as i32;
    let hour = first + 1;
    let last = first + 3;
    let mut indexed = ReadDates { variables: Some(Arc::default()), ..ReadDates::default() };
    assert_eq!(ReadDates::default().through(&full, 0, &reads, first, hour, last, &stop).unwrap(), last);
    assert_eq!(ReadDates::default().through(&damaged, 0, &reads, first, hour, last, &stop).unwrap(), hour);
    assert_eq!(indexed.through(&damaged, 0, &reads, first, hour, last, &stop).unwrap(), hour);
}
