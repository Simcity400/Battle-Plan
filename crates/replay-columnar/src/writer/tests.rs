use super::*;
use crate::section::Scratch;

const KINDS: [Kind; 6] = [
    Kind::U32,
    Kind::OptionalI64,
    Kind::Bytes,
    Kind::U32List,
    Kind::Opaque,
    Kind::Bool,
];

const COMPLETE: Evidence = Evidence {
    status: Status::Complete,
    read: true,
    sample: Sample::FromReadBit,
};

const TRUNCATED: Evidence = Evidence {
    status: Status::Truncated { missing: 1 },
    read: true,
    sample: Sample::FromReadBit,
};

fn row<'a>(
    value: u32,
    optional: Option<i64>,
    text: &'a [u8],
    list: &'a [u32],
    opaque: &'a [u8],
    flag: bool,
) -> [Input<'a>; 6] {
    [
        Input::Value(Cell::U32(value)),
        Input::Value(Cell::OptionalI64(optional)),
        Input::Text(text),
        Input::Value(Cell::U32List(list)),
        Input::Value(Cell::Opaque(opaque)),
        Input::Value(Cell::Bool(flag)),
    ]
}

fn prepare(
    writer: &mut TableWriter,
    dictionary: &mut Dictionary,
    coverage: Coverage<'_>,
    rows: &[(u128, [Input<'_>; 6])],
) -> io::Result<Prepared> {
    let mut hour = writer.begin(dictionary, coverage)?;
    for (key, cells) in rows {
        hour.row(*key, cells)?;
    }
    hour.finish()
}

fn hour(
    writer: &mut TableWriter,
    dictionary: &mut Dictionary,
    reader: &mut Table,
    coverage: Coverage<'_>,
    evidence: Evidence,
    at: i32,
    rows: &[(u128, [Input<'_>; 6])],
) -> Option<Changes> {
    written(writer, dictionary, reader, coverage, evidence, at, rows).1
}

fn written(
    writer: &mut TableWriter,
    dictionary: &mut Dictionary,
    reader: &mut Table,
    coverage: Coverage<'_>,
    evidence: Evidence,
    at: i32,
    rows: &[(u128, [Input<'_>; 6])],
) -> ((Census, u8), Option<Changes>) {
    let prepared = prepare(writer, dictionary, coverage, rows).unwrap();
    let truncated = matches!(evidence.status, Status::Truncated { .. });
    let applied = prepared.section.as_ref().map(|bytes| {
        let mut changes = Changes::default();
        section::apply(
            bytes,
            &KINDS,
            truncated,
            reader,
            &mut Scratch::default(),
            &mut changes,
        )
        .unwrap();
        changes
    });
    let census = (prepared.census, prepared.ops);
    writer.commit(Some(prepared), evidence, at).unwrap();
    assert_eq!(writer.table(), &*reader);
    (census, applied)
}

fn holds(reader: &Table, dictionary: &mut Dictionary, rows: &[(u128, [Input<'_>; 6])]) {
    for (key, cells) in rows {
        let slot = reader.slot(*key).unwrap();
        for (column, &input) in cells.iter().enumerate() {
            let held = reader.column(column).unwrap().cell(slot);
            match input {
                Input::Value(cell) => assert_eq!(held, Some(cell), "key {key} column {column}"),
                Input::Text(text) => {
                    let id = dictionary.intern(text).unwrap();
                    assert_eq!(held, Some(Cell::Bytes(id)), "key {key} column {column}");
                }
                Input::Unrecorded => assert!(!reader.recorded(column, slot)),
                Input::Kept => {}
            }
        }
    }
}

fn many(
    keys: impl IntoIterator<Item = u128>,
    value: impl Fn(u128) -> u32,
    flag: impl Fn(u128) -> bool,
) -> Vec<(u128, [Input<'static>; 6])> {
    keys.into_iter()
        .map(|key| (key, row(value(key), Some(1), b"m", &[], b"", flag(key))))
        .collect()
}

fn census(set: u64, add: u64, patch: u64) -> Census {
    Census {
        set,
        add,
        patch,
        ..Census::default()
    }
}

fn first_hour(writer: &mut TableWriter, dictionary: &mut Dictionary, reader: &mut Table) {
    let changes = hour(
        writer,
        dictionary,
        reader,
        Coverage::Complete,
        COMPLETE,
        1,
        &[
            (30, row(3, Some(-3), b"c", &[3], b"z", true)),
            (10, row(1, None, b"a", &[], b"", false)),
            (20, row(2, Some(2), b"b", &[2, 2], b"yy", true)),
        ],
    )
    .unwrap();
    assert_eq!(changes.inserted, [0, 1, 2]);
}

#[test]
fn hours_write_what_a_reader_applies_and_reuse_the_lowest_freed_slot() {
    let mut writer = TableWriter::new(&KINDS).unwrap();
    let mut reader = Table::new(&KINDS).unwrap();
    let mut dictionary = Dictionary::new();
    first_hour(&mut writer, &mut dictionary, &mut reader);
    assert_eq!(writer.table().slot(10), Some(0));
    assert_eq!(writer.table().slot(30), Some(2));
    assert_eq!(dictionary.take_appended().len(), 3);
    let changes = hour(
        &mut writer,
        &mut dictionary,
        &mut reader,
        Coverage::Complete,
        COMPLETE,
        2,
        &[
            (10, row(1, Some(5), b"a", &[], b"", false)),
            (30, row(3, Some(-3), b"c", &[3], b"z", true)),
            (40, row(4, None, b"d", &[4], b"w", false)),
        ],
    )
    .unwrap();
    assert_eq!(changes.deleted, [1]);
    assert_eq!(changes.inserted, [1]);
    assert_eq!(changes.changed, [0]);
    assert_eq!(writer.table().slot(40), Some(1));
    assert_eq!(writer.table().slot(20), None);
    assert_eq!(dictionary.take_appended(), vec![b"d".to_vec().into_boxed_slice()]);
}

#[test]
fn freed_slots_fill_lowest_first_then_the_extent_grows() {
    let mut writer = TableWriter::new(&KINDS).unwrap();
    let mut reader = Table::new(&KINDS).unwrap();
    let mut dictionary = Dictionary::new();
    first_hour(&mut writer, &mut dictionary, &mut reader);
    let changes = hour(
        &mut writer,
        &mut dictionary,
        &mut reader,
        Coverage::Changes { removed: &[10, 30] },
        COMPLETE,
        2,
        &[],
    )
    .unwrap();
    assert_eq!(changes.deleted, [0, 2]);
    let changes = hour(
        &mut writer,
        &mut dictionary,
        &mut reader,
        Coverage::Changes { removed: &[] },
        COMPLETE,
        3,
        &[
            (70, row(7, None, b"g", &[], b"", false)),
            (50, row(5, None, b"e", &[], b"", false)),
            (60, row(6, None, b"f", &[], b"", false)),
        ],
    )
    .unwrap();
    assert_eq!(changes.inserted, [0, 2, 3]);
    assert_eq!(writer.table().slot(50), Some(0));
    assert_eq!(writer.table().slot(60), Some(2));
    assert_eq!(writer.table().slot(70), Some(3));
}

#[test]
fn an_hour_that_changes_nothing_writes_no_section() {
    let mut writer = TableWriter::new(&KINDS).unwrap();
    let mut reader = Table::new(&KINDS).unwrap();
    let mut dictionary = Dictionary::new();
    first_hour(&mut writer, &mut dictionary, &mut reader);
    let prepared = prepare(
        &mut writer,
        &mut dictionary,
        Coverage::Complete,
        &[
            (20, row(2, Some(2), b"b", &[2, 2], b"yy", true)),
            (10, row(1, None, b"a", &[], b"", false)),
            (30, row(3, Some(-3), b"c", &[3], b"z", true)),
        ],
    )
    .unwrap();
    assert!(prepared.section.is_none());
    assert_eq!(prepared.ops, 0);
    assert!(prepared.columns.iter().all(|&word| word == 0));
    writer.commit(Some(prepared), COMPLETE, 2).unwrap();
    assert_eq!(writer.table(), &reader);
}

#[test]
fn an_unrecorded_cell_is_known_again_only_through_a_set() {
    let mut writer = TableWriter::new(&KINDS).unwrap();
    let mut reader = Table::new(&KINDS).unwrap();
    let mut dictionary = Dictionary::new();
    first_hour(&mut writer, &mut dictionary, &mut reader);
    let mut unrecorded = row(1, None, b"a", &[], b"", false);
    unrecorded[0] = Input::Unrecorded;
    let changes = hour(
        &mut writer,
        &mut dictionary,
        &mut reader,
        Coverage::Changes { removed: &[] },
        COMPLETE,
        2,
        &[(10, unrecorded)],
    )
    .unwrap();
    assert_eq!(changes.changed, [0]);
    assert!(!reader.recorded(0, 0));
    let changes = hour(
        &mut writer,
        &mut dictionary,
        &mut reader,
        Coverage::Changes { removed: &[] },
        COMPLETE,
        3,
        &[(10, row(1, None, b"a", &[], b"", false))],
    )
    .unwrap();
    assert_eq!(changes.changed, [0]);
    assert!(reader.recorded(0, 0));
    let mut inserted = row(9, Some(9), b"i", &[9], b"9", true);
    inserted[1] = Input::Unrecorded;
    let changes = hour(
        &mut writer,
        &mut dictionary,
        &mut reader,
        Coverage::Changes { removed: &[] },
        COMPLETE,
        4,
        &[(50, inserted)],
    )
    .unwrap();
    assert_eq!(changes.inserted, [3]);
    assert!(!reader.recorded(1, 3));
    assert!(reader.recorded(0, 3));
}

#[test]
fn a_partial_read_names_the_rows_it_read_and_deletes_none() {
    let mut writer = TableWriter::new(&KINDS).unwrap();
    let mut reader = Table::new(&KINDS).unwrap();
    let mut dictionary = Dictionary::new();
    first_hour(&mut writer, &mut dictionary, &mut reader);
    let changes = hour(
        &mut writer,
        &mut dictionary,
        &mut reader,
        Coverage::Partial,
        TRUNCATED,
        2,
        &[
            (20, row(7, Some(2), b"b", &[2, 2], b"yy", true)),
            (40, row(4, None, b"d", &[4], b"w", false)),
        ],
    )
    .unwrap();
    assert!(changes.truncated);
    assert_eq!(changes.read, [1]);
    assert_eq!(changes.changed, [1]);
    assert!(changes.deleted.is_empty());
    assert_eq!(changes.inserted, [3]);
    assert_eq!(writer.view(), [1, 3]);
    assert_eq!(writer.table().slot(10), Some(0));
    let changes = hour(
        &mut writer,
        &mut dictionary,
        &mut reader,
        Coverage::Partial,
        TRUNCATED,
        3,
        &[(20, row(7, Some(2), b"b", &[2, 2], b"yy", true))],
    )
    .unwrap();
    assert_eq!(changes.read, [1]);
    assert!(changes.changed.is_empty());
    assert_eq!(writer.view(), [1]);
    hour(
        &mut writer,
        &mut dictionary,
        &mut reader,
        Coverage::Changes { removed: &[] },
        COMPLETE,
        4,
        &[],
    );
    assert!(writer.view().is_empty());
}

#[test]
fn a_changes_hour_writes_the_section_a_census_writes() {
    let mut census = TableWriter::new(&KINDS).unwrap();
    let mut changes = TableWriter::new(&KINDS).unwrap();
    let mut dictionary = Dictionary::new();
    let mut readers = [Table::new(&KINDS).unwrap(), Table::new(&KINDS).unwrap()];
    first_hour(&mut census, &mut dictionary, &mut readers[0]);
    first_hour(&mut changes, &mut dictionary, &mut readers[1]);
    let whole = prepare(
        &mut census,
        &mut dictionary,
        Coverage::Complete,
        &[
            (10, row(1, Some(5), b"a", &[1], b"", false)),
            (30, row(3, Some(-3), b"c", &[3], b"z", true)),
            (40, row(4, None, b"d", &[4], b"w", false)),
        ],
    )
    .unwrap();
    let sparse = prepare(
        &mut changes,
        &mut dictionary,
        Coverage::Changes { removed: &[20] },
        &[
            (40, row(4, None, b"d", &[4], b"w", false)),
            (10, row(1, Some(5), b"a", &[1], b"", false)),
        ],
    )
    .unwrap();
    assert!(whole.section.is_some());
    assert_eq!(whole.section, sparse.section);
    assert_eq!(whole.columns, sparse.columns);
    assert_eq!(whole.ops, sparse.ops);
    assert_eq!(whole.ops, manifest::INSERTS | manifest::DELETES);
}

#[test]
fn a_restored_writer_continues_as_the_unbroken_one() {
    let mut unbroken = TableWriter::new(&KINDS).unwrap();
    let mut reader = Table::new(&KINDS).unwrap();
    let mut dictionary = Dictionary::new();
    first_hour(&mut unbroken, &mut dictionary, &mut reader);
    hour(
        &mut unbroken,
        &mut dictionary,
        &mut reader,
        Coverage::Changes { removed: &[20] },
        COMPLETE,
        2,
        &[(10, row(1, Some(5), b"a", &[], b"", false))],
    );
    let bytes = unbroken.restart().unwrap();
    let restored = restart::read(&bytes, &KINDS, &mut Scratch::default()).unwrap();
    let mut resumed = TableWriter::restore(&KINDS, restored).unwrap();
    assert_eq!(resumed.table(), unbroken.table());
    assert_eq!(resumed.acquisition(), unbroken.acquisition());
    let rows = [
        (50, row(5, Some(5), b"e", &[5], b"5", true)),
        (10, row(1, Some(5), b"a", &[], b"", false)),
        (30, row(6, Some(-3), b"c", &[3], b"z", true)),
    ];
    let expected = prepare(&mut unbroken, &mut dictionary, Coverage::Complete, &rows).unwrap();
    let actual = prepare(&mut resumed, &mut dictionary, Coverage::Complete, &rows).unwrap();
    assert_eq!(expected.section, actual.section);
    unbroken.commit(Some(expected), COMPLETE, 3).unwrap();
    resumed.commit(Some(actual), COMPLETE, 3).unwrap();
    assert_eq!(resumed.table(), unbroken.table());
    assert_eq!(resumed.table().slot(50), Some(1));
    assert_eq!(resumed.restart().unwrap(), unbroken.restart().unwrap());
}

fn refused(
    writer: &mut TableWriter,
    dictionary: &mut Dictionary,
    coverage: Coverage<'_>,
    visits: &[(u128, &[Input<'_>])],
) {
    let mut hour = writer.begin(dictionary, coverage).unwrap();
    let (last, earlier) = visits.split_last().unwrap();
    for (key, cells) in earlier {
        hour.row(*key, cells).unwrap();
    }
    assert!(hour.row(last.0, last.1).is_err());
    assert!(hour.row(30, &row(3, Some(-3), b"c", &[3], b"z", true)).is_err());
    assert!(hour.finish().is_err());
}

#[test]
fn refusals_name_their_cause_and_change_nothing_held() {
    let mut writer = TableWriter::new(&KINDS).unwrap();
    let mut reader = Table::new(&KINDS).unwrap();
    let mut dictionary = Dictionary::new();
    first_hour(&mut writer, &mut dictionary, &mut reader);
    let same = row(1, None, b"a", &[], b"", false);
    let mut wrong = same;
    wrong[0] = Input::Value(Cell::I32(1));
    let mut text = same;
    text[0] = Input::Text(b"a");
    refused(&mut writer, &mut dictionary, Coverage::Complete, &[(10, &same), (10, &same)]);
    refused(&mut writer, &mut dictionary, Coverage::Complete, &[(20, &wrong)]);
    refused(&mut writer, &mut dictionary, Coverage::Complete, &[(60, &wrong)]);
    refused(&mut writer, &mut dictionary, Coverage::Complete, &[(30, &text)]);
    refused(&mut writer, &mut dictionary, Coverage::Complete, &[(60, &same[..5])]);
    refused(&mut writer, &mut dictionary, Coverage::Changes { removed: &[20] }, &[(20, &same)]);
    let absent = writer
        .begin(&mut dictionary, Coverage::Changes { removed: &[70] })
        .unwrap();
    assert!(absent.finish().is_err());
    let stale = prepare(&mut writer, &mut dictionary, Coverage::Complete, &[]).unwrap();
    let latest = prepare(&mut writer, &mut dictionary, Coverage::Partial, &[]).unwrap();
    assert!(writer.commit(Some(stale), COMPLETE, 2).is_err());
    let mut other = TableWriter::new(&KINDS).unwrap();
    let foreign = prepare(&mut other, &mut dictionary, Coverage::Complete, &[]).unwrap();
    assert!(writer.commit(Some(foreign), COMPLETE, 2).is_err());
    assert!(writer.commit(Some(latest), COMPLETE, 2).is_err());
    assert!(writer.commit(None, TRUNCATED, 2).is_err());
    assert_eq!(writer.table(), &reader);
    assert_eq!(writer.status(), Status::Complete);
    hour(
        &mut writer,
        &mut dictionary,
        &mut reader,
        Coverage::Changes { removed: &[] },
        COMPLETE,
        3,
        &[(10, row(8, None, b"a", &[], b"", false))],
    );
}

#[test]
fn the_dictionary_interns_exact_bytes_once_in_order() {
    let mut dictionary = Dictionary::new();
    assert_eq!(dictionary.intern(b"x").unwrap(), 0);
    assert_eq!(dictionary.intern(b"y").unwrap(), 1);
    assert_eq!(dictionary.intern(b"x").unwrap(), 0);
    assert_eq!(dictionary.intern(b"").unwrap(), 2);
    assert_eq!(dictionary.len(), 3);
    let appended = dictionary.take_appended();
    assert_eq!(appended.len(), 3);
    assert_eq!(&*appended[1], b"y");
    assert!(dictionary.take_appended().is_empty());
    assert_eq!(dictionary.intern(b"y").unwrap(), 1);
    assert!(dictionary.take_appended().is_empty());
    assert_eq!(dictionary.get(0), Some(&b"x"[..]));
    assert_eq!(dictionary.get(2), Some(&b""[..]));
    assert_eq!(dictionary.get(3), None);
}

#[test]
fn samples_and_availability_follow_the_walker() {
    let mut writer = TableWriter::new(&KINDS).unwrap();
    let mut dictionary = Dictionary::new();
    let unavailable = Evidence {
        status: Status::Unavailable { reason: 4 },
        read: false,
        sample: Sample::FromReadBit,
    };
    writer.commit(None, unavailable, 1).unwrap();
    assert_eq!(
        *writer.acquisition(),
        Acquisition {
            last_read: None,
            available_from: Some(1),
            refusal: Some(4),
        }
    );
    let quiet = prepare(&mut writer, &mut dictionary, Coverage::Complete, &[]).unwrap();
    writer.commit(Some(quiet), COMPLETE, 2).unwrap();
    assert_eq!(
        *writer.acquisition(),
        Acquisition {
            last_read: Some(2),
            available_from: Some(2),
            refusal: None,
        }
    );
    let carried = Evidence {
        status: Status::Complete,
        read: false,
        sample: Sample::Carried(1),
    };
    writer.commit(None, carried, 3).unwrap();
    assert_eq!(writer.acquisition().last_read, Some(1));
    assert_eq!(writer.acquisition().available_from, Some(2));
    let reset = Evidence {
        status: Status::Complete,
        read: true,
        sample: Sample::Reset,
    };
    writer.commit(None, reset, 4).unwrap();
    assert_eq!(writer.acquisition().last_read, None);
    let unread = Evidence {
        status: Status::Complete,
        read: false,
        sample: Sample::FromReadBit,
    };
    writer.commit(None, unread, 5).unwrap();
    assert_eq!(writer.acquisition().last_read, None);
}

struct Fixture {
    writer: TableWriter,
    reader: Table,
    dictionary: Dictionary,
    at: i32,
}

impl Fixture {
    fn new(first: &[(u128, [Input<'_>; 6])]) -> Self {
        let mut fixture = Self {
            writer: TableWriter::new(&KINDS).unwrap(),
            reader: Table::new(&KINDS).unwrap(),
            dictionary: Dictionary::new(),
            at: 0,
        };
        fixture.step(Coverage::Complete, COMPLETE, first);
        fixture
    }

    fn step(
        &mut self,
        coverage: Coverage<'_>,
        evidence: Evidence,
        rows: &[(u128, [Input<'_>; 6])],
    ) -> ((Census, u8), Option<Changes>) {
        self.at += 1;
        let written = written(
            &mut self.writer,
            &mut self.dictionary,
            &mut self.reader,
            coverage,
            evidence,
            self.at,
            rows,
        );
        holds(&self.reader, &mut self.dictionary, rows);
        written
    }

    fn changes(&mut self, rows: &[(u128, [Input<'_>; 6])]) -> Census {
        let ((hour, _), _) = self.step(Coverage::Changes { removed: &[] }, COMPLETE, rows);
        hour
    }
}

#[test]
fn an_integer_delta_no_longer_than_its_value_is_an_add() {
    let mut fixture = Fixture::new(&[
        (30, row(3, Some(-3), b"c", &[3], b"z", true)),
        (10, row(1, None, b"a", &[], b"", false)),
        (20, row(2, Some(2), b"b", &[2, 2], b"yy", true)),
    ]);
    let rows = [(10, row(100, None, b"a", &[], b"", false))];
    assert_eq!(fixture.changes(&rows), census(0, 1, 0));
    let rows = [
        (10, row(200, None, b"a", &[], b"", false)),
        (30, row(1, Some(-3), b"c", &[3], b"z", true)),
    ];
    assert_eq!(fixture.changes(&rows), census(1, 1, 0));
    let rows = [
        (20, row(2, Some(-1_000), b"b", &[2, 2], b"yy", true)),
        (10, row(200, Some(5), b"a", &[], b"", false)),
        (30, row(1, None, b"c", &[3], b"z", true)),
    ];
    assert_eq!(fixture.changes(&rows), census(2, 1, 0));
    let mut unrecorded = row(2, Some(-1_000), b"b", &[2, 2], b"yy", true);
    unrecorded[0] = Input::Unrecorded;
    assert_eq!(fixture.changes(&[(20, unrecorded)]), census(0, 0, 0));
    let rows = [(20, row(3, Some(-1_000), b"b", &[2, 2], b"yy", true))];
    assert_eq!(fixture.changes(&rows), census(1, 0, 0));
    let rows = [(10, row(200, Some(5), b"a", &[], b"", true))];
    assert_eq!(fixture.changes(&rows), census(1, 0, 0));
}

#[test]
fn a_list_or_opaque_edit_shorter_than_its_cell_is_a_patch() {
    let (first, second) = ([1, 2, 3, 4, 5, 6, 7, 8], [11, 12, 13, 14, 15, 16, 17, 18]);
    let mut fixture = Fixture::new(&[
        (10, row(1, None, b"a", &first, b"abcdefghijklmnop", false)),
        (20, row(2, None, b"b", &second, b"z", false)),
    ]);
    let (first, second) = ([1, 2, 3, 4, 5, 6, 7, 9], [11, 12, 13, 14, 15, 16, 17, 19]);
    let rows = [
        (20, row(2, None, b"b", &second, b"zy", false)),
        (10, row(1, None, b"a", &first, b"aXcXefghijklmnop", false)),
    ];
    assert_eq!(fixture.changes(&rows), census(1, 0, 3));
    let grown = [1, 2, 3, 4, 5, 6, 7, 9, 10];
    let rows = [(10, row(1, None, b"a", &grown, b"aXcXefghijklmno", false))];
    assert_eq!(fixture.changes(&rows), census(0, 0, 2));
    let replaced = [21, 22, 23, 24, 25, 26, 27, 28, 29];
    let rows = [(10, row(1, None, b"a", &replaced, b"aXcXefghijklmno", false))];
    assert_eq!(fixture.changes(&rows), census(1, 0, 0));
}

#[test]
fn a_column_of_one_value_is_one_set_run_or_a_fill_of_the_live_rows() {
    let mut fixture = Fixture::new(&many(1..=20, |key| key as u32, |_| false));
    let rows = many(1..=20, |key| key as u32, |_| true);
    let ((hour, ops), changes) = fixture.step(Coverage::Complete, COMPLETE, &rows);
    let changes = changes.unwrap();
    let fill = (hour.fills, hour.fill_slots, hour.fill_exceptions);
    assert_eq!(fill, (1, 20, 0));
    assert_eq!(ops, manifest::FILLS);
    assert!(changes.changed.is_empty());
    assert_eq!(changes.fill_slots, (0..20).collect::<Vec<u32>>());
    let rows = many(1..=16, |key| key as u32, |_| false);
    let ((hour, _), _) = fixture.step(Coverage::Partial, TRUNCATED, &rows);
    assert_eq!((hour.fills, hour.fill_exceptions), (1, 4));
    let unread = many(17..=20, |key| key as u32, |_| true);
    holds(&fixture.reader, &mut fixture.dictionary, &unread);
    let flag = |key: u128| (17..=20).contains(&key);
    let rows = many(2..=22, |key| if key <= 17 { 0 } else { key as u32 }, flag);
    let ((hour, _), changes) = fixture.step(Coverage::Complete, COMPLETE, &rows);
    let changes = changes.unwrap();
    let fill = (hour.fills, hour.fill_slots, hour.fill_exceptions);
    assert_eq!(fill, (1, 16, 5));
    assert_eq!((changes.deleted.len(), changes.inserted.len()), (1, 2));
    let rows = many(
        2..=22,
        |key| match key {
            ..=17 => 0,
            18..=20 => 30,
            _ => key as u32,
        },
        flag,
    );
    let (written, _) = fixture.step(Coverage::Complete, COMPLETE, &rows);
    assert_eq!(written, (census(3, 0, 0), 0));
    let rows = many(
        2..=22,
        |key| match key {
            ..=16 => 50,
            17 => 0,
            18..=20 => 30,
            _ => key as u32,
        },
        flag,
    );
    let (written, _) = fixture.step(Coverage::Complete, COMPLETE, &rows);
    assert_eq!(written, (census(15, 0, 0), 0));
}

#[test]
fn a_fill_takes_at_least_half_the_rows_live_after_the_hour() {
    let first = many(1..=33, |key| key as u32, |_| false);
    let mut fixture = Fixture::new(&first);
    let rows = many(1..=16, |_| 100, |_| false);
    assert_eq!(fixture.changes(&rows), census(16, 0, 0));
    let rows = many(1..=16, |_| 200, |_| false);
    let coverage = Coverage::Changes { removed: &[33] };
    let ((hour, _), _) = fixture.step(coverage, COMPLETE, &rows);
    let fill = (hour.fills, hour.fill_slots, hour.fill_exceptions);
    assert_eq!(fill, (1, 16, 16));
    holds(&fixture.reader, &mut fixture.dictionary, &first[16..32]);
}

fn batched<'r, 'c: 'r>(
    batch: &mut Batch,
    kinds: &[Kind],
    rows: impl ExactSizeIterator<Item = (u128, &'r [Input<'c>])>,
) -> io::Result<()> {
    batch.begin(kinds, rows.len())?;
    for (key, cells) in rows {
        batch.key(key)?;
        for (column, &input) in cells.iter().enumerate() {
            match input {
                Input::Value(cell) => batch.value(column, cell)?,
                Input::Text(text) => batch.text(column, text)?,
                Input::Unrecorded => batch.unrecorded(column)?,
                Input::Kept => batch.kept(column)?,
            }
        }
    }
    Ok(())
}

const WIDE: [Kind; 18] = [
    Kind::U8,
    Kind::I8,
    Kind::U16,
    Kind::I16,
    Kind::U32,
    Kind::I32,
    Kind::U64,
    Kind::I64,
    Kind::Bool,
    Kind::OptionalI32,
    Kind::OptionalU32,
    Kind::OptionalI64,
    Kind::OptionalU64,
    Kind::Bytes,
    Kind::U32List,
    Kind::U64List,
    Kind::Opaque,
    Kind::Bytes,
];

#[derive(Clone)]
struct Wide {
    key: u128,
    scalars: [Cell<'static>; 13],
    names: [Vec<u8>; 2],
    u32s: Vec<u32>,
    u64s: Vec<u64>,
    opaque: Vec<u8>,
    unrecorded: u32,
}

impl Wide {
    fn new(key: u128, value: u32) -> Self {
        let signed = i64::from(value);
        Self {
            key,
            scalars: [
                Cell::U8(value as u8),
                Cell::I8((value as i8).wrapping_neg()),
                Cell::U16(value as u16 * 3),
                Cell::I16(-(value as i16) * 3),
                Cell::U32(value * 1_000),
                Cell::I32(-(value as i32) * 1_000),
                Cell::U64(u64::from(value) << 40),
                Cell::I64(-signed << 40),
                Cell::Bool(value % 2 == 0),
                Cell::OptionalI32(Some(value as i32)),
                Cell::OptionalU32((value % 3 != 0).then_some(value)),
                Cell::OptionalI64(Some(-signed)),
                Cell::OptionalU64(None),
            ],
            names: [format!("row {key}").into_bytes(), b"shared".to_vec()],
            u32s: (0..8).map(|item| item + value).collect(),
            u64s: vec![u64::from(value); 3],
            opaque: (0..16u8).map(|byte| byte ^ value as u8).collect(),
            unrecorded: 0,
        }
    }

    fn inputs(&self) -> Vec<Input<'_>> {
        let mut inputs: Vec<Input<'_>> =
            self.scalars.iter().map(|&cell| Input::Value(cell)).collect();
        inputs.extend([
            Input::Text(&self.names[0]),
            Input::Value(Cell::U32List(&self.u32s)),
            Input::Value(Cell::U64List(&self.u64s)),
            Input::Value(Cell::Opaque(&self.opaque)),
            Input::Text(&self.names[1]),
        ]);
        for (column, input) in inputs.iter_mut().enumerate() {
            if self.unrecorded >> column & 1 != 0 {
                *input = Input::Unrecorded;
            }
        }
        inputs
    }

    fn with(mut self, column: usize, cell: Cell<'static>) -> Self {
        self.scalars[column] = cell;
        self
    }
}

struct Side {
    writer: TableWriter,
    dictionary: Dictionary,
    reader: Table,
}

impl Side {
    fn new() -> Self {
        Self::sweeping(Sweep::Touched)
    }

    fn sweeping(sweep: Sweep) -> Self {
        let mut writer = TableWriter::new(&WIDE).unwrap();
        writer.set_sweep(sweep);
        Self {
            writer,
            dictionary: Dictionary::new(),
            reader: Table::new(&WIDE).unwrap(),
        }
    }

    fn rows(&mut self, coverage: Coverage<'_>, rows: &[(u128, Vec<Input<'_>>)]) -> Prepared {
        let mut hour = self.writer.begin(&mut self.dictionary, coverage).unwrap();
        for (key, cells) in rows {
            hour.row(*key, cells).unwrap();
        }
        hour.finish().unwrap()
    }

    fn batch(&mut self, coverage: Coverage<'_>, batch: &Batch) -> Prepared {
        self.writer.prepare(&mut self.dictionary, coverage, batch).unwrap()
    }

    fn commit(&mut self, prepared: Prepared, evidence: Evidence, at: i32) -> Option<Changes> {
        let truncated = matches!(evidence.status, Status::Truncated { .. });
        let applied = prepared.section.as_ref().map(|bytes| {
            let mut changes = Changes::default();
            section::apply(
                bytes,
                &WIDE,
                truncated,
                &mut self.reader,
                &mut Scratch::default(),
                &mut changes,
            )
            .unwrap();
            changes
        });
        self.writer.commit(Some(prepared), evidence, at).unwrap();
        assert_eq!(self.writer.table(), &self.reader);
        applied
    }
}

struct Twin {
    rows: Side,
    batch: Side,
    swept: [Side; 2],
    staged: Batch,
    at: i32,
}

impl Twin {
    fn new() -> Self {
        Self {
            rows: Side::new(),
            batch: Side::new(),
            swept: [Side::sweeping(Sweep::Every), Side::sweeping(Sweep::Every)],
            staged: Batch::new(),
            at: 0,
        }
    }

    fn hour(
        &mut self,
        coverage: Coverage<'_>,
        evidence: Evidence,
        rows: &[Wide],
    ) -> Option<Changes> {
        self.at += 1;
        let at = self.at;
        let inputs: Vec<(u128, Vec<Input<'_>>)> =
            rows.iter().map(|row| (row.key, row.inputs())).collect();
        let expected = self.rows.rows(coverage, &inputs);
        let cells = inputs.iter().map(|(key, cells)| (*key, cells.as_slice()));
        batched(&mut self.staged, &WIDE, cells).unwrap();
        let actual = self.batch.batch(coverage, &self.staged);
        let [swept_rows, swept_batch] = &mut self.swept;
        let swept = [
            swept_rows.rows(coverage, &inputs),
            swept_batch.batch(coverage, &self.staged),
        ];
        for prepared in std::iter::once(&actual).chain(&swept) {
            assert_eq!(prepared.section, expected.section, "hour {at}");
            assert_eq!(prepared.columns, expected.columns, "hour {at}");
            let census = (prepared.ops, prepared.census);
            assert_eq!(census, (expected.ops, expected.census), "hour {at}");
        }
        let appended = self.rows.dictionary.take_appended();
        for side in [&mut self.batch, &mut *swept_rows, &mut *swept_batch] {
            assert_eq!(side.dictionary.take_appended(), appended, "hour {at}");
        }
        let changes = self.rows.commit(expected, evidence, at);
        assert_eq!(self.batch.commit(actual, evidence, at), changes, "hour {at}");
        for (side, prepared) in [swept_rows, swept_batch].into_iter().zip(swept) {
            assert_eq!(side.commit(prepared, evidence, at), changes, "hour {at}");
        }
        for side in std::iter::once(&self.batch).chain(&self.swept) {
            assert_eq!(side.writer.table(), self.rows.writer.table(), "hour {at}");
            assert_eq!(side.writer.view(), self.rows.writer.view(), "hour {at}");
        }
        changes
    }

    fn looked_up(&self) -> bool {
        !self.batch.writer.visited.is_empty()
    }

    fn restart(&self) -> Vec<u8> {
        let bytes = self.rows.writer.restart().unwrap();
        for side in std::iter::once(&self.batch).chain(&self.swept) {
            assert_eq!(side.writer.restart().unwrap(), bytes);
        }
        bytes
    }

    fn restore(&mut self, bytes: &[u8]) {
        let [swept_rows, swept_batch] = &mut self.swept;
        for side in [&mut self.rows, &mut self.batch, swept_rows, swept_batch] {
            let sweep = side.writer.sweep();
            let restored = restart::read(bytes, &WIDE, &mut Scratch::default()).unwrap();
            side.writer = TableWriter::restore(&WIDE, restored).unwrap();
            side.writer.set_sweep(sweep);
        }
    }
}

#[test]
fn a_batch_writes_what_its_rows_write_one_at_a_time() {
    let mut twin = Twin::new();
    let first: Vec<Wide> = (1..=20).map(|key| Wide::new(key * 10, key as u32)).collect();
    twin.hour(Coverage::Complete, COMPLETE, &first);
    assert!(twin.hour(Coverage::Complete, COMPLETE, &first).is_none());
    assert!(!twin.looked_up());
    let mut third = first.clone();
    for (index, row) in third.iter_mut().enumerate() {
        row.scalars[0] = Cell::U8(200);
        if index % 3 == 0 {
            row.scalars[4] = Cell::U32((index as u32 + 1) * 1_000 + 1);
        }
    }
    third[4].scalars[7] = Cell::I64(-1);
    third[6].unrecorded = 1 << 2;
    third[2].scalars[9] = Cell::OptionalI32(None);
    third[1].scalars[10] = Cell::OptionalU32(Some(7));
    third[2].scalars[10] = Cell::OptionalU32(Some(9));
    third[3].names = [b"renamed 4".to_vec(), b"renamed 4b".to_vec()];
    third[4].names = [b"renamed 6".to_vec(), b"row 70".to_vec()];
    third[5].names[0] = b"renamed 6".to_vec();
    third[1].u32s[3] += 100;
    third[7].u32s.push(99);
    third[8].u64s = vec![1, 2, 3, 4, 5];
    third[9].opaque[2] ^= 0xff;
    third[9].opaque[4] ^= 0xff;
    third[10].opaque = vec![7; 3];
    let changes = twin.hour(Coverage::Complete, COMPLETE, &third).unwrap();
    assert!(!twin.looked_up());
    assert_eq!(changes.filled, [0]);
    let mut fourth: Vec<Wide> = third.iter().rev().filter(|row| row.key != 50).cloned().collect();
    for row in &mut fourth {
        row.unrecorded = 0;
    }
    let mut new = Wide::new(205, 21);
    new.unrecorded = 1 << 0 | 1 << 11 | 1 << 13 | 1 << 16;
    fourth.insert(3, new);
    fourth.push(Wide::new(7, 22));
    let changes = twin.hour(Coverage::Complete, COMPLETE, &fourth).unwrap();
    assert!(twin.looked_up());
    assert_eq!((changes.deleted.len(), changes.inserted.len()), (1, 2));
    let mut fifth = fourth.clone();
    fifth.sort_by_key(|row| row.key);
    assert!(twin.hour(Coverage::Complete, COMPLETE, &fifth).is_none());
    assert!(twin.looked_up());
    let mut sixth = fifth.clone();
    for row in &mut sixth {
        row.scalars[3] = Cell::I16(5);
    }
    sixth[0].names[1] = b"sixth".to_vec();
    let changes = twin.hour(Coverage::Complete, COMPLETE, &sixth).unwrap();
    assert!(!twin.looked_up());
    assert_eq!(changes.filled, [3]);
    let mut read = sixth[5..10].to_vec();
    read[1].scalars[5] = Cell::I32(4);
    read[3].u64s.clear();
    twin.hour(Coverage::Partial, TRUNCATED, &read);
    let mut seventh = sixth.clone();
    seventh[6] = read[1].clone();
    seventh[8] = read[3].clone();
    seventh[0].scalars[6] = Cell::U64(1);
    twin.hour(Coverage::Complete, COMPLETE, &seventh).unwrap();
    assert!(!twin.looked_up());
    let changed = [seventh[2].clone().with(4, Cell::U32(3))];
    let removed = [seventh[3].key];
    twin.hour(Coverage::Changes { removed: &removed }, COMPLETE, &changed);
    assert!(!twin.batch.writer.order.held);
    let mut ninth = seventh.clone();
    ninth[2] = changed[0].clone();
    ninth.remove(4);
    ninth.remove(3);
    ninth.push(Wide::new(300, 30));
    let changes = twin.hour(Coverage::Complete, COMPLETE, &ninth).unwrap();
    assert!(twin.looked_up());
    assert_eq!((changes.deleted.len(), changes.inserted.len()), (1, 1));
    let mut tenth = ninth.clone();
    for row in &mut tenth[..3] {
        row.scalars[5] = Cell::I32(77);
    }
    let changes = twin.hour(Coverage::Complete, COMPLETE, &tenth).unwrap();
    assert!(!twin.looked_up());
    assert!(changes.filled.is_empty());
    let read = [tenth[1].clone().with(1, Cell::I8(9)), Wide::new(1, 40)];
    twin.hour(Coverage::Partial, TRUNCATED, &read);
    assert!(!twin.batch.writer.order.held);
    let bytes = twin.restart();
    twin.restore(&bytes);
    let mut eleventh = tenth.clone();
    eleventh[1] = read[0].clone();
    eleventh[2].names[0] = b"restored".to_vec();
    eleventh.pop();
    eleventh.insert(0, read[1].clone());
    twin.hour(Coverage::Complete, COMPLETE, &eleventh).unwrap();
    assert!(twin.looked_up());
    assert!(twin.hour(Coverage::Complete, COMPLETE, &eleventh).is_none());
    assert!(!twin.looked_up());
    twin.restart();
}

#[test]
fn a_refused_batch_changes_nothing_held() {
    let mut writer = TableWriter::new(&KINDS).unwrap();
    let mut reader = Table::new(&KINDS).unwrap();
    let mut dictionary = Dictionary::new();
    first_hour(&mut writer, &mut dictionary, &mut reader);
    let same = row(1, None, b"a", &[], b"", false);
    let mut batch = Batch::new();
    batch.begin(&KINDS, 1).unwrap();
    assert!(batch.value(0, Cell::U32(1)).is_err());
    batch.key(10).unwrap();
    assert!(batch.value(0, Cell::I32(1)).is_err());
    assert!(batch.text(0, b"a").is_err());
    assert!(batch.value(2, Cell::Bytes(0)).is_err());
    assert!(batch.unrecorded(6).is_err());
    batch.value(0, Cell::U32(1)).unwrap();
    assert!(batch.value(0, Cell::U32(1)).is_err());
    assert!(writer.prepare(&mut dictionary, Coverage::Complete, &batch).is_err());
    batched(&mut batch, &KINDS, [(10, &same[..]), (10, &same[..])].into_iter()).unwrap();
    assert!(writer.prepare(&mut dictionary, Coverage::Complete, &batch).is_err());
    batched(&mut batch, &KINDS, [(20, &same[..])].into_iter()).unwrap();
    let removed = Coverage::Changes { removed: &[20] };
    assert!(writer.prepare(&mut dictionary, removed, &batch).is_err());
    batched(&mut batch, &KINDS[..5], [(10, &same[..5])].into_iter()).unwrap();
    assert!(writer.prepare(&mut dictionary, Coverage::Complete, &batch).is_err());
    assert_eq!(writer.table(), &reader);
    let changed = [(10, row(8, None, b"a", &[], b"", false))];
    let cells = changed.iter().map(|(key, cells)| (*key, &cells[..]));
    batched(&mut batch, &KINDS, cells).unwrap();
    let coverage = Coverage::Changes { removed: &[] };
    let prepared = writer.prepare(&mut dictionary, coverage, &batch).unwrap();
    let mut changes = Changes::default();
    let bytes = prepared.section.as_ref().unwrap();
    let scratch = &mut Scratch::default();
    section::apply(bytes, &KINDS, false, &mut reader, scratch, &mut changes).unwrap();
    writer.commit(Some(prepared), COMPLETE, 2).unwrap();
    assert_eq!(writer.table(), &reader);
    assert_eq!(changes.changed, [0]);
    holds(&reader, &mut dictionary, &changed);
}

#[test]
fn a_batch_begun_again_holds_only_its_own_cells() {
    let mut batch = Batch::new();
    let wide = Wide::new(10, 1);
    let inputs = wide.inputs();
    batched(&mut batch, &WIDE, [(10, &inputs[..])].into_iter()).unwrap();
    let narrow = row(5, None, b"n", &[1], b"o", true);
    batched(&mut batch, &KINDS, [(20, &narrow[..])].into_iter()).unwrap();
    assert!(batch.columns[..KINDS.len()].iter().all(|cells| cells.words.len() == 1));
    assert!(batch.columns[KINDS.len()..].iter().all(|cells| cells.words.is_empty()));
    batch.begin(&WIDE, 0).unwrap();
    assert!(batch.columns.iter().all(|cells| cells.words.is_empty() && cells.bytes.is_empty()));
    let mut writer = TableWriter::new(&WIDE).unwrap();
    let mut dictionary = Dictionary::new();
    let prepared = writer
        .prepare(&mut dictionary, Coverage::Changes { removed: &[] }, &batch)
        .unwrap();
    assert!(prepared.section.is_none());
    assert_eq!(writer.work(), Work { hours: 1, columns: 0 });
}

fn inputs(rows: &[Wide]) -> Vec<(u128, Vec<Input<'_>>)> {
    rows.iter().map(|row| (row.key, row.inputs())).collect()
}

fn holding(first: &[Wide]) -> [Side; 2] {
    let rows = inputs(first);
    [Side::new(), Side::sweeping(Sweep::Every)].map(|mut side| {
        let prepared = side.rows(Coverage::Complete, &rows);
        side.commit(prepared, COMPLETE, 1);
        side.dictionary.take_appended();
        side
    })
}

fn restored(side: &Side) -> Side {
    let bytes = side.writer.restart().unwrap();
    let held = restart::read(&bytes, &WIDE, &mut Scratch::default()).unwrap();
    let mut dictionary = Dictionary::new();
    for id in 0..side.dictionary.len() as u32 {
        dictionary.intern(side.dictionary.get(id).unwrap()).unwrap();
    }
    dictionary.take_appended();
    Side {
        writer: TableWriter::restore(&WIDE, held).unwrap(),
        dictionary,
        reader: side.reader.clone(),
    }
}

fn attempt(
    side: &mut Side,
    batch: Option<&Batch>,
    coverage: Coverage<'_>,
    rows: &[(u128, Vec<Input<'_>>)],
) -> io::Result<Prepared> {
    let strings = Strings::Reading(&side.dictionary);
    if let Some(batch) = batch {
        return side.writer.prepare(strings, coverage, batch);
    }
    let mut hour = side.writer.begin(strings, coverage)?;
    for (key, cells) in rows {
        hour.row(*key, cells)?;
    }
    hour.finish()
}

fn again(sides: &mut [Side; 2], hour: impl Fn(&mut Side) -> Prepared) {
    let mut fresh = restored(&sides[0]);
    let expected = hour(&mut fresh);
    let appended = fresh.dictionary.take_appended();
    for side in sides.iter_mut() {
        let prepared = hour(side);
        assert_eq!(prepared.section, expected.section);
        assert_eq!(prepared.columns, expected.columns);
        assert_eq!(
            (prepared.ops, prepared.census),
            (expected.ops, expected.census)
        );
        assert_eq!(side.dictionary.take_appended(), appended);
        side.commit(prepared, COMPLETE, 2);
    }
    fresh.commit(expected, COMPLETE, 2);
    let bytes = fresh.writer.restart().unwrap();
    for side in sides.iter() {
        assert_eq!(side.writer.restart().unwrap(), bytes);
    }
}

#[test]
fn an_hour_prepared_again_after_a_missed_string_writes_what_it_would_have() {
    let first: Vec<Wide> = (1..=40)
        .map(|key| Wide::new(key * 10, key as u32))
        .collect();
    let mut next = first.clone();
    for row in &mut next[..25] {
        row.scalars[0] = Cell::U8(200);
    }
    next[9].scalars[4] = Cell::U32(9_001);
    next[3].u32s[2] += 50;
    next[7].opaque[1] ^= 0xff;
    next[5].unrecorded = 1 << 6;
    next[30].scalars[5] = Cell::I32(5);
    next[30].names[0] = b"renamed".to_vec();
    next[35].names[1] = b"renamed again".to_vec();
    let rows = inputs(&next);
    let mut batch = Batch::new();
    let cells = rows.iter().map(|(key, cells)| (*key, cells.as_slice()));
    batched(&mut batch, &WIDE, cells).unwrap();
    for by_batch in [false, true] {
        let mut sides = holding(&first);
        for side in &mut sides {
            let staged = by_batch.then_some(&batch);
            let missed = attempt(side, staged, Coverage::Complete, &rows).unwrap_err();
            assert_eq!(
                missed.to_string(),
                "A new string is interned in table order"
            );
        }
        again(&mut sides, |side| {
            if by_batch {
                side.batch(Coverage::Complete, &batch)
            } else {
                side.rows(Coverage::Complete, &rows)
            }
        });
    }
}

#[test]
fn an_hour_after_one_refused_as_it_finishes_writes_what_it_would_have() {
    let first: Vec<Wide> = (1..=40)
        .map(|key| Wide::new(key * 10, key as u32))
        .collect();
    let mut changed = first[..24].to_vec();
    for row in &mut changed {
        row.scalars[0] = Cell::U8(200);
    }
    changed[2].u64s.push(9);
    changed[4].scalars[11] = Cell::OptionalI64(None);
    changed[6].unrecorded = 1 << 14;
    changed[8].names[0] = b"row 100".to_vec();
    let rows = inputs(&changed);
    let mut batch = Batch::new();
    let cells = rows.iter().map(|(key, cells)| (*key, cells.as_slice()));
    batched(&mut batch, &WIDE, cells).unwrap();
    let changes = Coverage::Changes { removed: &[] };
    for by_batch in [false, true] {
        let mut sides = holding(&first);
        for side in &mut sides {
            let refused = if by_batch {
                attempt(
                    side,
                    Some(&batch),
                    Coverage::Changes { removed: &[5] },
                    &rows,
                )
            } else {
                let strings = Strings::Reading(&side.dictionary);
                let mut hour = side.writer.begin(strings, changes).unwrap();
                for (key, cells) in &rows {
                    hour.row(*key, cells).unwrap();
                }
                hour.touch(first[39].key).unwrap();
                hour.finish()
            };
            let cause = if by_batch {
                "A removed row key is not held"
            } else {
                "A row is touched but was not visited this hour"
            };
            assert_eq!(refused.unwrap_err().to_string(), cause);
        }
        again(&mut sides, |side| {
            if by_batch {
                side.batch(changes, &batch)
            } else {
                side.rows(changes, &rows)
            }
        });
    }
}

#[test]
fn an_hour_after_one_dropped_unfinished_writes_what_it_would_have() {
    let first: Vec<Wide> = (1..=40)
        .map(|key| Wide::new(key * 10, key as u32))
        .collect();
    let mut dropped = first.clone();
    for row in &mut dropped[..20] {
        row.scalars[4] = Cell::U32(1);
    }
    dropped[3].opaque.truncate(4);
    dropped[5].unrecorded = 1 << 9;
    let dropped = inputs(&dropped);
    let mut next = first[..6].to_vec();
    next[1].scalars[4] = Cell::U32(7);
    next[2].opaque[0] ^= 1;
    next[5].scalars[9] = Cell::OptionalI32(Some(-1));
    let rows = inputs(&next);
    let mut sides = holding(&first);
    for side in &mut sides {
        let strings = Strings::Reading(&side.dictionary);
        let mut hour = side.writer.begin(strings, Coverage::Complete).unwrap();
        for (key, cells) in &dropped {
            hour.row(*key, cells).unwrap();
        }
        drop(hour);
    }
    let before = sides[0].writer.work();
    again(&mut sides, |side| {
        side.rows(Coverage::Changes { removed: &[] }, &rows)
    });
    let all = WIDE.len() as u64;
    let after = Work {
        hours: before.hours + 1,
        columns: before.columns + all + 3,
    };
    assert_eq!(sides[0].writer.work(), after);
}

fn patched<T: Copy>(held: &[T], edits: &[(u32, u32, u32)], new: &[T]) -> Vec<T> {
    let (mut out, mut from, mut item) = (Vec::new(), 0, 0);
    let mut inserted = Vec::new();
    let mut at = 0;
    for &(skip, _, added) in edits {
        at += skip as usize;
        inserted.extend_from_slice(&new[at..at + added as usize]);
        at += added as usize;
    }
    for &(skip, removed, added) in edits {
        out.extend_from_slice(&held[from..from + skip as usize]);
        from += (skip + removed) as usize;
        out.extend_from_slice(&inserted[item..item + added as usize]);
        item += added as usize;
    }
    out.extend_from_slice(&held[from..]);
    out
}

fn records(count: u32, first: u32) -> Vec<u8> {
    let mut bytes = count.to_le_bytes().to_vec();
    for record in first..first + count {
        bytes.extend((0..24u32).flat_map(|word| (record * 31 + word).to_le_bytes()));
    }
    bytes
}

#[test]
fn a_list_behind_a_changed_count_keeps_its_records() {
    let held = records(40, 0);
    let mut pruned = records(30, 10);
    pruned.splice(4 + 96 * 12..4 + 96 * 14, []);
    pruned[0] = 28;
    for new in [records(42, 0), records(40, 1), pruned] {
        let mut edits = Vec::new();
        assert!(hunks(&held, &new, 3, |_| 1, &mut edits).unwrap());
        assert_eq!(patched(&held, &edits, &new), new);
        let inserted: u32 = edits.iter().map(|edit| edit.2).sum();
        assert!(inserted <= 4 + 2 * 96, "{edits:?}");
    }
    let (held, new): (Vec<u64>, Vec<u64>) = ((0..100).collect(), (3..120).collect());
    let mut edits = Vec::new();
    assert!(hunks(&held, &new, 1, varint_len, &mut edits).unwrap());
    assert_eq!(patched(&held, &edits, &new), new);
    let (held, new): (Vec<u32>, Vec<u32>) = ((0..100).collect(), (3..120).collect());
    let mut edits = Vec::new();
    assert!(hunks(&held, &new, 1, |item| varint_len(item.into()), &mut edits).unwrap());
    assert_eq!(patched(&held, &edits, &new), new);
}

#[test]
fn dictionary_interns_and_reads_strings_across_table_growth() {
    let mut dict = Dictionary::new();
    let mut ids = Vec::new();
    for i in 0..200_u32 {
        let value = format!("entry-{i:04}");
        let id = dict.intern(value.as_bytes()).unwrap();
        assert_eq!(id, i);
        assert_eq!(dict.intern(value.as_bytes()).unwrap(), id);
        ids.push((id, value));
    }
    assert_eq!(dict.len(), 200);
    assert_eq!(dict.take_appended().len(), 200);
    for (id, value) in &ids {
        assert_eq!(dict.get(*id), Some(value.as_bytes()));
        let mut reading = Strings::Reading(&dict);
        assert_eq!(reading.intern(value.as_bytes()).unwrap(), *id);
    }
    let mut reading = Strings::Reading(&dict);
    assert!(reading.intern(b"absent").is_err());
}

#[test]
fn commit_releases_oversized_keyframe_insert_scratch_while_preserving_state() {
    let kinds = [Kind::U64, Kind::Bytes];
    let mut dict = Dictionary::new();
    let mut writer = TableWriter::new(&kinds).unwrap();
    let mut hour = writer.begin(&mut dict, Coverage::Complete).unwrap();
    for key in 1..=2000_u128 {
        let name = format!("row-{key}");
        hour.row(
            key,
            &[
                Input::Value(Cell::U64(key as u64)),
                Input::Text(name.as_bytes()),
            ],
        )
        .unwrap();
    }
    let prepared = hour.finish().unwrap();
    let scratch = writer.columns.as_ref().unwrap();
    assert!(scratch.inserts[0].words.capacity() >= 2000);
    assert!(scratch.insert_outs[0].u64s.capacity() >= 2000);
    assert!(writer.parts.sorted_keys.capacity() >= 2000);
    writer
        .commit(
            Some(prepared),
            Evidence {
                status: Status::Complete,
                read: true,
                sample: Sample::FromReadBit,
            },
            0,
        )
        .unwrap();
    let scratch = writer.columns.as_ref().unwrap();
    assert_eq!(scratch.inserts[0].words.capacity(), 0);
    assert_eq!(scratch.insert_outs[0].u64s.capacity(), 0);
    assert_eq!(writer.parts.sorted_keys.capacity(), 0);
    assert_eq!(writer.table().live().count(), 2000);

    let mut next = writer
        .begin(&mut dict, Coverage::Changes { removed: &[] })
        .unwrap();
    next.row(
        1,
        &[Input::Value(Cell::U64(999)), Input::Text(b"row-1")],
    )
    .unwrap();
    let prepared = next.finish().unwrap();
    writer
        .commit(
            Some(prepared),
            Evidence {
                status: Status::Complete,
                read: true,
                sample: Sample::FromReadBit,
            },
            1,
        )
        .unwrap();
    let slot = writer.table().slot(1).unwrap();
    assert_eq!(
        writer.table().column(0).unwrap().cell(slot),
        Some(Cell::U64(999))
    );
}
