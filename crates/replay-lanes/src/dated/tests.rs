use super::*;
use crate::at::{Read, frame};
use replay_data::rows::{CountryAppearanceRow, CountryRow, Recorded, mask};
use replay_journal::{
    fixtures,
    synthetic::{Input, Sample},
};

fn compare(recording: &Recording, reads: &[(TableId, u64)], asked: &[u128]) -> usize {
    let mut compared = 0;
    for run in 0..recording.run_count() {
        let held = recording.run(run).unwrap();
        for &(table, fields) in reads {
            let dated = Dated::read(recording, run, table, fields, Keys::All).unwrap();
            let columns: Vec<usize> = (0..table.schema().len()).filter(|&column| fields & (1 << column) != 0).collect();
            for ordinal in 0..held.ordinals {
                let hour = held.first_hour + ordinal as i32;
                let whole = frame(recording, run, hour, &[Read { table, columns: Some(&columns), keys: Keys::All }]).unwrap();
                let keyed = frame(recording, run, hour, &[Read { table, columns: Some(&columns), keys: Keys::These(asked) }]).unwrap();
                assert_eq!(*dated.view(recording, hour).unwrap().held(), *whole.tables[table.index()].held(), "{table:?} fields {fields:#x} run {run} hour {hour}");
                assert_eq!(*dated.view_of(recording, hour, asked).unwrap().held(), *keyed.tables[table.index()].held(), "{table:?} keyed run {run} hour {hour}");
                compared += 1;
            }
        }
    }
    compared
}

#[test]
fn a_dated_view_at_any_hour_is_that_hours_decoded_read_through_renames_returns_and_withheld_blocks() {
    const RUNS: [(u64, i32, u32); 2] = [(1, 100, 2), (2, 50_000, 1)];
    let recording = Recording::open(fixtures::churned(recorder_derive::writers(), &RUNS).0).unwrap();
    let tag = mask::<CountryRow>(&["tag"]);
    let compared = compare(&recording, &[(TableId::Countries, u64::MAX), (TableId::Countries, tag), (TableId::WorldTension, u64::MAX)], &[3, 4, 9]);
    assert!(compared > 3_000);
}

#[test]
fn a_dated_view_holds_a_truncated_first_hours_read_rows_and_each_written_list() {
    const RUNS: [(u64, i32, u32); 1] = [(1, 100, 1)];
    let (bytes, _) = fixtures::written(recorder_derive::writers(), b"truncated start", &RUNS, &[TableId::Countries, TableId::CountryAppearance], |synthetic, _, ordinal| {
        let countries: Vec<CountryRow> = (1..=6).filter(|id| id % 3 != 0 || ordinal % 40 < 20).map(|id| CountryRow { id, tag: format!("tag {id} {}", ordinal / 90).into_bytes(), ..Default::default() }).collect();
        let input = match ordinal % 61 {
            0 | 30 => Input::Snapshot { rows: &countries[..3], partial: Some(2), read: true, sample: Sample::FromReadBit },
            45 => Input::Unavailable { reason: "refused for an hour", read: false, sample: Sample::FromReadBit },
            _ => Input::Snapshot { rows: &countries, partial: None, read: true, sample: Sample::FromReadBit },
        };
        synthetic.table(TableId::Countries, input).unwrap();
        let looks: Vec<CountryAppearanceRow> = (1..=4)
            .map(|country| CountryAppearanceRow { country, subjects: Recorded::Value((0..(ordinal / 50 + country) % 4).map(|at| at as i32 * 7).collect()), name_tag: format!("look {}", ordinal / 70).into_bytes(), ..Default::default() })
            .collect();
        synthetic.table(TableId::CountryAppearance, Input::Snapshot { rows: &looks, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
    });
    let recording = Recording::open(bytes).unwrap();
    let looks = mask::<CountryAppearanceRow>(&["subjects", "name_tag"]);
    let compared = compare(&recording, &[(TableId::Countries, u64::MAX), (TableId::CountryAppearance, looks)], &[1, 3, 4]);
    assert!(compared > 1_000);
}

#[test]
fn a_dated_view_past_a_damaged_block_refuses_as_the_decoded_read_does_and_reads_before_it() {
    let (bytes, _) = fixtures::churned(recorder_derive::writers(), &[(1, 0, 3)]);
    let clean = Recording::open(bytes.clone()).unwrap();
    let entry = clean.table_entry(0, 1, TableId::Countries.index()).unwrap();
    let damaged_frame = &entry.unit.parts[0].frames[0];
    let mut damaged = bytes;
    damaged[(damaged_frame.offset + u64::from(damaged_frame.stored) / 2) as usize] ^= 0x40;
    let recording = Recording::open(damaged).unwrap();
    let compared = compare(&recording, &[(TableId::Countries, u64::MAX), (TableId::WorldTension, u64::MAX)], &[2, 4]);
    assert!(compared > 4_000);
}

#[test]
fn a_dated_view_before_a_tables_derived_start_refuses_with_the_withholding_reason() {
    let (bytes, _) = fixtures::churned(recorder_derive::writers(), &[(1, 0, 1)]);
    let recording = Recording::open(bytes).unwrap().with_coverage_from(TableId::Countries, 0, 300, "derived from a later read");
    let compared = compare(&recording, &[(TableId::Countries, u64::MAX), (TableId::Countries, mask::<CountryRow>(&["tag"]))], &[1, 2]);
    assert!(compared > 1_000);
}

#[test]
fn a_dated_view_after_a_damaged_block_drops_the_rows_that_left_during_it() {
    let (bytes, _) = fixtures::edited(recorder_derive::writers(), 3);
    let clean = Recording::open(bytes.clone()).unwrap();
    let entry = clean.table_entry(0, 1, TableId::Countries.index()).unwrap();
    let damaged_frame = &entry.unit.parts[0].frames[0];
    let mut damaged = bytes;
    damaged[(damaged_frame.offset + u64::from(damaged_frame.stored) / 2) as usize] ^= 0x40;
    let recording = Recording::open(damaged).unwrap();
    let compared = compare(&recording, &[(TableId::Countries, u64::MAX), (TableId::Countries, mask::<CountryRow>(&["tag"]))], &[12, 16, 105]);
    assert!(compared > 4_000);
}

#[test]
fn a_tables_statuses_from_its_block_manifests_are_each_hours_decoded_status() {
    let kind = |view: &TableView| match view {
        TableView::Complete(_) => Status::Complete,
        TableView::Truncated { missing, .. } => Status::Truncated { missing: *missing },
        TableView::Unavailable { .. } => Status::Unavailable { reason: 0 },
        TableView::NotRecorded { .. } => Status::NotRecorded,
    };
    let flatten = |status: Status| match status {
        Status::Unavailable { .. } => Status::Unavailable { reason: 0 },
        other => other,
    };
    let mut compared = 0;
    for (recording, table) in [
        (Recording::open(fixtures::churned(recorder_derive::writers(), &[(1, 100, 2)]).0).unwrap(), TableId::WorldTension),
        (Recording::open(fixtures::edited(recorder_derive::writers(), 2).0).unwrap(), TableId::Countries),
    ] {
        let held = recording.run(0).unwrap();
        let statuses = lanes::table_statuses(&recording, 0, table).unwrap();
        for ordinal in 0..held.ordinals {
            let hour = held.first_hour + ordinal as i32;
            let decoded = frame(&recording, 0, hour, &[Read::all(table)]).unwrap();
            let at = statuses.iter().find(|run| run.first <= hour && hour <= run.last).map(|run| flatten(run.status));
            assert_eq!(at, Some(kind(&decoded.tables[table.index()])), "{table:?} hour {hour}");
            compared += 1;
        }
    }
    assert!(compared > 2_000);
}
