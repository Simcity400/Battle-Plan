use super::*;
use crate::test_fixtures::{CAMPAIGN, records, source};
use replay::campaign::Replay;
use replay_data::rows::{ScriptVariableNameRow, ScriptVariableValueRow, TableRows};
use super::super::{ReadDates, TableRead};
use replay_frame::frame::TableView;
use std::sync::atomic::AtomicBool;

fn replay(last: i32) -> Replay {
    replay_with_names(last, 1)
}

fn replay_with_names(last: i32, names_count: u32) -> Replay {
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
        let name = if hour < 104 { b"before".to_vec() } else { b"after".to_vec() };
        let names = TableId::ScriptVariableNames.index();
        let rows = (1..=names_count).map(|id| ScriptVariableNameRow { id, name: name.clone() }).collect();
        frame.tables[names] = TableView::Complete(TableRows::ScriptVariableNames(rows)).into();
        frame.metrics.table_ns[names] = 1;
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
    let replay = Arc::new(replay(110));
    let stop = AtomicBool::new(false);
    let state = Arc::<Mutex<VariableDates>>::default();
    let (cold, reads) = replay_recording::cells::count_reads(|| state.lock().unwrap().dates(replay.recording(), 0, TABLE, 100, 110, &selection(1)));
    assert!(cold.is_none());
    assert!(!reads.contains_key(&TABLE));
    if let Some(ready) = VariableDates::prepare(&state, Arc::clone(&replay), 0).unwrap() { ready.wait().unwrap(); }
    assert!(VariableDates::prepare(&state, Arc::clone(&replay), 0).unwrap().is_none());
    let mut dates = state.lock().unwrap();
    for first in [100, 103, 108] {
        let _ = dates.dates(replay.recording(), 0, TABLE, first, 110, &selection(1)).unwrap();
        for scope in [1, 2] {
            let selection = selection(scope);
            let expected = selection.dates(&replay, 0, TABLE, first, 110).unwrap();
            let (found, reads) = replay_recording::cells::count_reads(|| dates.dates(replay.recording(), 0, TABLE, first, 110, &selection));
            let upcoming = |hours: Vec<i32>| hours.into_iter().filter(|&hour| hour > first).collect::<Vec<_>>();
            assert_eq!(upcoming(found.unwrap()), upcoming(expected));
            assert!(reads.get(&TABLE).is_none_or(|read| read.opened == 0 && read.restores == 0));
        }
    }
    let mut single = selection(2);
    single.rows.as_mut().unwrap()[0].0 += 1;
    assert!(dates.dates(replay.recording(), 0, TABLE, 100, 110, &single).is_none());
    let names = TableId::ScriptVariableNames;
    let whole = Selection { fields: 1, ..Selection::default() };
    let expected = whole.dates(&replay, 0, names, 100, 110).unwrap();
    let (found, reads) = replay_recording::cells::count_reads(|| dates.dates(replay.recording(), 0, names, 100, 110, &whole));
    assert_eq!(found, Some(expected));
    assert!(!reads.contains_key(&names));
    assert!(dates.dates(replay.recording(), 0, names, 101, 110, &whole).is_none());
    assert!(Index::load(replay.recording(), 0, 1).is_none());
    drop(dates);
    let small = Arc::new(Mutex::new(VariableDates { bytes: RETAINED_BYTES - BYTES_PER_RUN, ..VariableDates::default() }));
    if let Some(ready) = VariableDates::prepare(&small, Arc::clone(&replay), 0).unwrap() { ready.wait().unwrap(); }
    assert!(small.lock().unwrap().runs[&0].1.is_none());
    let reads = [TableRead::fields(TABLE, &["value_raw"])];
    let mut indexed = ReadDates { variables: Some(small), ..ReadDates::default() };
    assert_eq!(indexed.through(&replay, 0, &reads, 100, 101, 110, &stop).unwrap(),
        ReadDates::default().through(&replay, 0, &reads, 100, 101, 110, &stop).unwrap());
}

#[test]
fn preparation_charges_the_distinct_name_dates() {
    let replay = Arc::new(replay_with_names(110, 64));
    let names = TableId::ScriptVariableNames;
    let whole = Selection { fields: 1, ..Selection::default() };
    let dates = whole.dates(&replay, 0, names, 100, 110).unwrap();
    assert_eq!(dates, [100, 104]);
    assert!(dates.capacity() > dates.len());
    let values = Index::load(replay.recording(), 0, RETAINED_BYTES).unwrap().bytes;
    let budget = BYTES_PER_RUN + values + size_of_val(dates.as_slice());
    let kept = Arc::new(Mutex::new(VariableDates { bytes: RETAINED_BYTES - budget, ..VariableDates::default() }));
    if let Some(ready) = VariableDates::prepare(&kept, Arc::clone(&replay), 0).unwrap() { ready.wait().unwrap(); }
    let mut kept = kept.lock().unwrap();
    assert_eq!(kept.dates(replay.recording(), 0, names, 100, 110, &whole), Some(dates));
    assert_eq!(kept.bytes, RETAINED_BYTES);
}

#[test]
fn a_run_reservation_does_not_displace_an_index_that_still_fits() {
    let replay = replay(110);
    let index = Index::load(replay.recording(), 0, RETAINED_BYTES).unwrap();
    let mut kept = VariableDates {
        runs: BTreeMap::from([(0, (Arc::default(), None))]),
        bytes: RETAINED_BYTES - index.bytes - BYTES_PER_RUN,
        building: true,
    };
    let names: Box<[i32]> = Box::new([100, 104]);
    assert!(index.bytes + size_of_val(&*names) <= RETAINED_BYTES - kept.bytes);
    kept.runs.insert(1, (Arc::default(), None));
    kept.bytes += BYTES_PER_RUN;
    kept.publish(0, index, Some(names));
    let (_, index) = &kept.runs[&0];
    assert!(index.as_ref().unwrap().names.is_none());
    assert_eq!(kept.bytes, RETAINED_BYTES);
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
    let damaged = Arc::new(Replay::open(bytes).unwrap());
    let stop = AtomicBool::new(false);
    let reads = [TableRead::fields(TABLE, &["value_raw"])];
    let first = 100 + 6 * BLOCK as i32;
    let hour = first + 1;
    let last = first + 3;
    let prepared = Arc::<Mutex<VariableDates>>::default();
    if let Some(ready) = VariableDates::prepare(&prepared, Arc::clone(&damaged), 0).unwrap() { ready.wait().unwrap(); }
    assert!(prepared.lock().unwrap().runs[&0].1.is_none());
    let mut indexed = ReadDates { variables: Some(prepared), ..ReadDates::default() };
    assert_eq!(ReadDates::default().through(&full, 0, &reads, first, hour, last, &stop).unwrap(), last);
    assert_eq!(ReadDates::default().through(&damaged, 0, &reads, first, hour, last, &stop).unwrap(), hour);
    assert_eq!(indexed.through(&damaged, 0, &reads, first, hour, last, &stop).unwrap(), hour);
}
