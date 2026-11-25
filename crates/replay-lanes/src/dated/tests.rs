use super::*;
use crate::at::{Read, frame};
use replay_data::rows::{CountryRow, mask};
use replay_journal::fixtures;

#[test]
fn a_dated_view_at_any_hour_is_that_hours_decoded_read_through_renames_returns_and_withheld_blocks() {
    const RUNS: [(u64, i32, u32); 2] = [(1, 100, 2), (2, 50_000, 1)];
    let recording = Recording::open(fixtures::churned(recorder_derive::writers(), &RUNS).0).unwrap();
    let tag = mask::<CountryRow>(&["tag"]);
    let asked = [3_u128, 4, 9];
    let mut compared = 0;
    for run in 0..recording.run_count() {
        let held = recording.run(run).unwrap();
        for (table, fields) in [(TableId::Countries, u64::MAX), (TableId::Countries, tag), (TableId::WorldTension, u64::MAX)] {
            let dated = Dated::read(&recording, run, table, fields, Keys::All).unwrap();
            let columns: Vec<usize> = (0..table.schema().len()).filter(|&column| fields & (1 << column) != 0).collect();
            for ordinal in 0..held.ordinals {
                let hour = held.first_hour + ordinal as i32;
                let whole = frame(&recording, run, hour, &[Read { table, columns: Some(&columns), keys: Keys::All }]).unwrap();
                let keyed = frame(&recording, run, hour, &[Read { table, columns: Some(&columns), keys: Keys::These(&asked) }]).unwrap();
                assert_eq!(*dated.view(hour).unwrap().held(), *whole.tables[table.index()].held(), "{table:?} fields {fields:#x} run {run} hour {hour}");
                assert_eq!(*dated.view_of(hour, &asked).unwrap().held(), *keyed.tables[table.index()].held(), "{table:?} keyed run {run} hour {hour}");
                compared += 1;
            }
        }
    }
    assert!(compared > 3_000);
}
