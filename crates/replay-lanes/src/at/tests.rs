use super::*;
use replay_recording::columnar_frames::Frames;
use replay_journal::fixtures;
use replay_data::rows::{CountryRow, TableRows};
use replay_columnar::envelope::UNIT_ORDINALS;

const TABLES: [TableId; 2] = [TableId::Countries, TableId::WorldTension];

fn recording() -> Recording {
    const RUNS: [(u64, i32, u32); 2] = [(1, 100, 5), (2, 50_000, 3)];
    Recording::open(fixtures::churned(recorder_derive::writers(), &RUNS).0).unwrap()
}

fn ordinals(ordinals: u32) -> Vec<u32> {
    let block = UNIT_ORDINALS;
    let mut out: Vec<u32> = [0, 1, 299, 300, 450, block - 1, block, 1000, 2 * block, 3 * block - 1, 3 * block, 3 * block + 1, 4 * block + 77]
        .into_iter()
        .chain([ordinals - 1])
        .filter(|&ordinal| ordinal < ordinals)
        .collect();
    out.dedup();
    out
}

#[test]
fn a_frame_at_an_hour_is_the_seek_of_its_tables() {
    let recording = recording();
    let reads = TABLES.map(Read::all);
    let mut compared = 0;
    for run in 0..recording.run_count() {
        let held = recording.run(run).unwrap();
        for ordinal in ordinals(held.ordinals) {
            let hour = held.first_hour + ordinal as i32;
            let mut seek = Frames::new(&recording, &TABLES).unwrap();
            seek.seek(run, ordinal).unwrap();
            let expected = seek.frame(held.branch).unwrap();
            assert_eq!(frame(&recording, run, hour, &reads).unwrap(), expected, "run {run} hour {hour}");
            compared += 1;
        }
    }
    assert!(compared > 10);
}

#[test]
fn a_cursors_frames_at_hours_in_turn_are_each_hours_frame() {
    let recording = recording();
    let asked = [4_u128, 2, 9];
    let some = [Read { table: TableId::Countries, columns: None, keys: Keys::These(&asked) }, Read::all(TableId::WorldTension)];
    let hours: Vec<(u32, i32)> = (0..recording.run_count())
        .flat_map(|run| {
            let held = recording.run(run).unwrap();
            ordinals(held.ordinals).into_iter().map(move |ordinal| (run, held.first_hour + ordinal as i32))
        })
        .collect();
    let mut compared = 0;
    for reads in [TABLES.map(Read::all), some] {
        let mut cursor = Cursor::new(&recording, &reads).unwrap();
        for &(run, hour) in hours.iter().chain(hours.iter().rev()) {
            assert_eq!(cursor.frame(run, hour).unwrap(), frame(&recording, run, hour, &reads).unwrap(), "run {run} hour {hour}");
            compared += 1;
        }
    }
    assert!(compared > 40);
    let held = recording.run(0).unwrap();
    let forward = ordinals(held.ordinals);
    let mut cursor = Cursor::new(&recording, &TABLES.map(Read::all)).unwrap();
    for &ordinal in &forward {
        cursor.frame(0, held.first_hour + ordinal as i32).unwrap();
    }
    let (_, readers) = cursor.readers.as_ref().unwrap();
    for reader in readers.iter().flatten() {
        assert!((reader.restores() as usize) < forward.len(), "{:?} restored {} times for {} hours", reader.table(), reader.restores(), forward.len());
    }
}

#[test]
fn a_cursor_resumed_from_anothers_readers_restores_nothing_again() {
    let recording = recording();
    let reads = TABLES.map(Read::all);
    let held = recording.run(0).unwrap();
    let hours: Vec<i32> = ordinals(held.ordinals).into_iter().map(|ordinal| held.first_hour + ordinal as i32).collect();
    let (first, second) = hours.split_at(hours.len() / 2);
    let restores = |cursor: &Cursor<'_, '_>| cursor.readers.as_ref().unwrap().1.iter().flatten().map(|reader| (reader.table(), reader.restores())).collect::<Vec<_>>();

    let mut whole = Cursor::new(&recording, &reads).unwrap();
    for &hour in &hours {
        whole.frame(0, hour).unwrap();
    }
    let mut earlier = Cursor::new(&recording, &reads).unwrap();
    for &hour in first {
        earlier.frame(0, hour).unwrap();
    }
    let left = earlier.into_readers().expect("a cursor that read leaves its readers");
    let mut later = Cursor::resume(&recording, &reads, Some(left)).unwrap();
    for &hour in second {
        assert_eq!(later.frame(0, hour).unwrap(), frame(&recording, 0, hour, &reads).unwrap(), "hour {hour}");
    }
    assert_eq!(restores(&later), restores(&whole));
    let mut back = Cursor::resume(&recording, &reads, later.into_readers()).unwrap();
    for &hour in first.iter().rev() {
        assert_eq!(back.frame(0, hour).unwrap(), frame(&recording, 0, hour, &reads).unwrap(), "hour {hour} after later hours");
    }

    let columns = [0_usize];
    let other = [Read { table: TableId::Countries, columns: Some(&columns), keys: Keys::All }, Read::all(TableId::WorldTension)];
    let mut fresh = Cursor::resume(&recording, &other, back.into_readers()).unwrap();
    assert!(fresh.readers.is_none(), "readers of other columns are not taken");
    assert_eq!(fresh.frame(0, hours[0]).unwrap(), frame(&recording, 0, hours[0], &other).unwrap());

    let mut failing = Cursor::new(&recording, &reads).unwrap();
    failing.frame(0, hours[0]).unwrap();
    assert!(failing.frame(0, held.first_hour - 1).is_err());
    assert!(failing.into_readers().is_none(), "a cursor whose frame failed hands on no reader");
}

#[test]
fn a_seekers_frames_of_changing_reads_are_each_hours_frame() {
    let recording = recording();
    let (two, four) = ([2_u128], [4_u128, 9]);
    let asked: [Vec<Read<'_>>; 4] = [
        TABLES.map(Read::all).to_vec(),
        vec![Read { table: TableId::Countries, columns: None, keys: Keys::These(&two) }],
        vec![Read::all(TableId::WorldTension), Read { table: TableId::Countries, columns: None, keys: Keys::These(&four) }],
        vec![Read { table: TableId::Countries, columns: None, keys: Keys::Range(2, 3) }],
    ];
    let hours: Vec<(u32, i32)> = (0..recording.run_count())
        .flat_map(|run| {
            let held = recording.run(run).unwrap();
            ordinals(held.ordinals).into_iter().map(move |ordinal| (run, held.first_hour + ordinal as i32))
        })
        .collect();
    let mut seeker = Seeker::new(&recording).unwrap();
    let mut compared = 0;
    for (at, &(run, hour)) in hours.iter().chain(hours.iter().rev()).enumerate() {
        let reads = &asked[at % asked.len()];
        assert_eq!(seeker.frame(run, hour, reads).unwrap(), frame(&recording, run, hour, reads).unwrap(), "run {run} hour {hour} reads {at}");
        compared += 1;
    }
    assert!(compared > 40);
    let held = recording.run(0).unwrap();
    let forward = ordinals(held.ordinals);
    let mut seeker = Seeker::new(&recording).unwrap();
    let mut cursor = Cursor::new(&recording, &TABLES.map(Read::all)).unwrap();
    for (at, &ordinal) in forward.iter().enumerate() {
        let hour = held.first_hour + ordinal as i32;
        seeker.frame(0, hour, &asked[at % 2]).unwrap();
        cursor.frame(0, hour).unwrap();
    }
    let (_, readers) = cursor.readers.as_ref().unwrap();
    let countries = |restores: &[(TableId, u32)]| restores.iter().find(|(table, _)| *table == TableId::Countries).map(|(_, count)| *count);
    let by_cursor: Vec<(TableId, u32)> = readers.iter().flatten().map(|reader| (reader.table(), reader.restores())).collect();
    let by_seeker = seeker.restores();
    assert_eq!(countries(&by_seeker), countries(&by_cursor), "a table read at every hour restores as a cursor's does, whatever its keys");
    for (table, restores) in by_seeker {
        assert!((restores as usize) < forward.len(), "{table:?} restored {restores} times for {} hours", forward.len());
    }
    assert!(seeker.frame(0, held.first_hour - 1, &asked[0]).is_err(), "an hour the run does not hold is refused");
}

fn countries(frame: &DecodedFrame) -> Vec<CountryRow> {
    match &*frame.tables[TableId::Countries.index()] {
        TableView::Complete(TableRows::Countries(rows)) => rows.clone(),
        other => panic!("Countries is not complete: {other:?}"),
    }
}

#[test]
fn a_frame_of_some_keys_is_the_whole_table_filtered() {
    let recording = recording();
    let asked = [4_u128, 2, 9];
    let ranges = [(1_u128, 1), (3, 4), (6, 9)];
    let filters: [(Keys<'_>, fn(u128) -> bool); 4] = [
        (Keys::These(&asked), |key| [2, 4].contains(&key)),
        (Keys::Range(2, 3), |key| (2..=3).contains(&key)),
        (Keys::Range(5, 8), |_| false),
        (Keys::Ranges(&ranges), |key| [1, 3, 4].contains(&key)),
    ];
    let mut compared = 0;
    for run in 0..recording.run_count() {
        let held = recording.run(run).unwrap();
        for ordinal in ordinals(held.ordinals) {
            let hour = held.first_hour + ordinal as i32;
            let whole = frame(&recording, run, hour, &TABLES.map(Read::all)).unwrap();
            for (keys, wanted) in filters {
                let reads = [Read { table: TableId::Countries, columns: None, keys }, Read::all(TableId::WorldTension)];
                let filtered = frame(&recording, run, hour, &reads).unwrap();
                let expected: Vec<CountryRow> = countries(&whole).into_iter().filter(|row| wanted(u128::from(row.id))).collect();
                assert_eq!(countries(&filtered), expected, "run {run} hour {hour} {keys:?}");
                assert_eq!(filtered.tables[TableId::WorldTension.index()], whole.tables[TableId::WorldTension.index()]);
                assert_eq!(filtered.metrics, whole.metrics);
                compared += 1;
            }
        }
    }
    assert!(compared > 30);
}

fn shown(frame: &DecodedFrame) -> Option<(Vec<CountryRow>, Option<u32>)> {
    match &*frame.tables[TableId::Countries.index()] {
        TableView::Complete(TableRows::Countries(rows)) => Some((rows.clone(), None)),
        TableView::Truncated { rows: TableRows::Countries(rows), missing } => Some((rows.clone(), Some(*missing))),
        _ => None,
    }
}

#[test]
fn a_frame_over_partial_and_refused_hours_is_the_seek_filtered() {
    let recording = Recording::open(fixtures::edited(recorder_derive::writers(), 3).0).unwrap();
    let held = recording.run(0).unwrap();
    let asked = [3_u128, 12, 150, 999];
    let filters: [(Keys<'_>, fn(u128) -> bool); 2] = [(Keys::These(&asked), |key| [3, 12, 150].contains(&key)), (Keys::Range(5, 15), |key| (5..=15).contains(&key))];
    let (mut partial, mut refused) = (0, 0);
    let block = UNIT_ORDINALS;
    for ordinal in [0, 9, 10, 15, 30, 45, 91, 106, block - 1, block, block + 12, block + 30, 2 * block + 8, 2 * block + 47, 3 * block - 1] {
        let hour = held.first_hour + ordinal as i32;
        let mut seek = Frames::new(&recording, &[TableId::Countries]).unwrap();
        seek.seek(0, ordinal).unwrap();
        let whole = seek.frame(held.branch).unwrap();
        assert_eq!(frame(&recording, 0, hour, &[Read::all(TableId::Countries)]).unwrap(), whole, "hour {hour}");
        for (keys, wanted) in filters {
            let filtered = frame(&recording, 0, hour, &[Read { table: TableId::Countries, columns: None, keys }]).unwrap();
            match shown(&whole) {
                Some((rows, missing)) => {
                    let expected: Vec<CountryRow> = rows.into_iter().filter(|row| wanted(u128::from(row.id))).collect();
                    assert_eq!(shown(&filtered), Some((expected, missing)), "hour {hour} {keys:?}");
                    partial += usize::from(missing.is_some());
                }
                None => {
                    assert_eq!(filtered.tables[TableId::Countries.index()], whole.tables[TableId::Countries.index()], "hour {hour}");
                    refused += 1;
                }
            }
        }
    }
    assert!(partial > 0 && refused > 0, "{partial} partly read, {refused} refused");
}

#[test]
fn a_read_of_some_columns_decodes_those_fields_alone() {
    use replay_journal::synthetic::{Input, Sample};
    use replay_data::rows::{AirPowerContribution, AirPowerMenu, RaidAirScoreRow, Recorded};
    use std::sync::Arc;
    let table = TableId::RaidAirScores;
    let menu = AirPowerMenu { coverage: 7, reductions: [1, 2, 3, 4], contributions: [vec![AirPowerContribution { country: 9, amount: 40 }], Vec::new()] };
    let rows = [
        RaidAirScoreRow { country: 9, region: 3, friendly: 120, enemy: 80, impact: Recorded::Value(5), friendly_missions: 2, enemy_missions: 1, air_spotting_scale: Some(6), menu: Recorded::Value(Some(Arc::new(menu))) },
        RaidAirScoreRow { country: 9, region: 4, friendly: 0, enemy: 30, impact: Recorded::Value(0), friendly_missions: 0, enemy_missions: 3, air_spotting_scale: None, menu: Recorded::Value(None) },
    ];
    let (bytes, _) = fixtures::written(recorder_derive::writers(), b"some columns", &[(1, 100, 2)], &[table], |synthetic, _, _| {
        synthetic.table(table, Input::Snapshot { rows: &rows, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
    });
    let recording = Recording::open(bytes).unwrap();
    let field = |name: &str| (0..table.field_count()).find(|&field| table.field_name(field) == Some(name)).unwrap();
    let columns = [field("friendly"), field("enemy")];
    let expected: Vec<RaidAirScoreRow> = rows.iter().map(|row| RaidAirScoreRow { country: row.country, region: row.region, friendly: row.friendly, enemy: row.enemy, ..Default::default() }).collect();
    let held = recording.run(0).unwrap();
    for ordinal in 0..held.ordinals {
        let hour = held.first_hour + ordinal as i32;
        let whole = frame(&recording, 0, hour, &[Read::all(table)]).unwrap();
        assert_eq!(*whole.tables[table.index()], TableView::Complete(TableRows::RaidAirScores(rows.to_vec())), "hour {hour}");
        let some = frame(&recording, 0, hour, &[Read { table, columns: Some(&columns), keys: Keys::All }]).unwrap();
        assert_eq!(*some.tables[table.index()], TableView::Complete(TableRows::RaidAirScores(expected.clone())), "hour {hour}");
    }
}

#[test]
fn readers_left_by_one_recording_are_not_taken_up_by_another_of_its_shape() {
    use replay_journal::synthetic::{Input, Sample};
    let table = TableId::Countries;
    let shaped = |offset: i64| {
        let (bytes, _) = fixtures::written(recorder_derive::writers(), b"same shape", &[(1, 100, 2)], &[table], |synthetic, _, ordinal| {
            let rows = [CountryRow { id: 1, stability_raw: offset + i64::from(ordinal), ..Default::default() }];
            synthetic.table(table, Input::Snapshot { rows: &rows, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        });
        bytes
    };
    let bytes = shaped(0);
    let (first, second) = (Recording::open(bytes.clone()).unwrap(), Recording::open(shaped(1_000_000)).unwrap());
    let reads = [Read::all(table)];
    let held = first.run(0).unwrap();
    assert_eq!(held.ordinals, second.run(0).unwrap().ordinals);
    let hour = held.first_hour + 7;
    let mut cursor = Cursor::new(&first, &reads).unwrap();
    cursor.frame(0, hour).unwrap();

    let mut elsewhere = Cursor::resume(&second, &reads, cursor.into_readers()).unwrap();
    assert!(elsewhere.readers.is_none(), "another recording's readers are not taken");
    assert_eq!(elsewhere.frame(0, hour).unwrap(), frame(&second, 0, hour, &reads).unwrap());
    assert_ne!(frame(&first, 0, hour, &reads).unwrap(), frame(&second, 0, hour, &reads).unwrap());

    let mut again = Cursor::new(&first, &reads).unwrap();
    again.frame(0, hour).unwrap();
    let mut home = Cursor::resume(&first, &reads, again.into_readers()).unwrap();
    assert!(home.readers.is_some(), "the recording that left its readers takes them up");
    assert_eq!(home.frame(0, hour).unwrap(), frame(&first, 0, hour, &reads).unwrap());

    let copy = Recording::open(bytes).unwrap();
    assert_eq!(copy.revision(), first.revision());
    let copied = Cursor::resume(&copy, &reads, home.into_readers()).unwrap();
    assert!(copied.readers.is_none(), "another opening of the same bytes reads its own frames");
    let mut again = Cursor::new(&first, &reads).unwrap();
    again.frame(0, hour).unwrap();
    let mut narrower = first.selecting(&[table]).unwrap();
    narrower.project(table, &[0]).unwrap();
    let projected = Cursor::resume(&narrower, &reads, again.into_readers()).unwrap();
    assert!(projected.readers.is_none(), "a recording projecting other default columns takes no reader of the wider ones");
}
