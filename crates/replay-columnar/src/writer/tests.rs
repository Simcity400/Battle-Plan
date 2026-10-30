use super::*;

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

/// Prepares one hour's rows; nothing held changes.
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

/// Writes one hour, applies its section to `reader` as a walker would, commits it and
/// checks both hold the same table.
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

/// [`hour`], with what the hour's runs carried and its op flags.
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

/// The reader holds every row as given: the cells an hour wrote are the ones it was handed.
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

/// Rows of a table of many: each key's U32 value and flag, its other cells alike.
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
    // New rows take their slots in key order.
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
    // Key 20 was not visited: deleted, and its slot holds the new key.
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
    // The same value recorded again is a change: the cell was unknown.
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
    // An inserted row's unrecorded cell holds no value a reader could take.
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
    // A quiet partial read still writes the rows it read.
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
    // A complete hour clears the view.
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
    // Deleting key 20 leaves a free slot below the extent.
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
    // The new row took the freed slot.
    assert_eq!(resumed.table().slot(50), Some(1));
    assert_eq!(resumed.restart().unwrap(), unbroken.restart().unwrap());
}

/// Visits `visits` in one hour; the last is refused, and the hour then never finishes.
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
    // Only the latest hour's preparation commits, and only to its own writer.
    let stale = prepare(&mut writer, &mut dictionary, Coverage::Complete, &[]).unwrap();
    let latest = prepare(&mut writer, &mut dictionary, Coverage::Partial, &[]).unwrap();
    assert!(writer.commit(Some(stale), COMPLETE, 2).is_err());
    let mut other = TableWriter::new(&KINDS).unwrap();
    let foreign = prepare(&mut other, &mut dictionary, Coverage::Complete, &[]).unwrap();
    assert!(writer.commit(Some(foreign), COMPLETE, 2).is_err());
    // A partial read and a complete status do not come together.
    assert!(writer.commit(Some(latest), COMPLETE, 2).is_err());
    assert!(writer.commit(None, TRUNCATED, 2).is_err());
    assert_eq!(writer.table(), &reader);
    assert_eq!(writer.status(), Status::Complete);
    // A refused hour leaves the next one exact.
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

/// A writer and the walker beside it, an hour at a time.
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

    /// The next hour, as [`written`]; the reader then holds `rows` as given.
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

    /// The next hour's changed rows, and what its runs carried.
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
    // A delta as long as the value it replaces: 1 to 100 is one byte either way.
    let rows = [(10, row(100, None, b"a", &[], b"", false))];
    assert_eq!(fixture.changes(&rows), census(0, 1, 0));
    // A shorter delta is an ADD; an unsigned column's negative delta wraps to five bytes.
    let rows = [
        (10, row(200, None, b"a", &[], b"", false)),
        (30, row(1, Some(-3), b"c", &[3], b"z", true)),
    ];
    assert_eq!(fixture.changes(&rows), census(1, 1, 0));
    // A signed column's negative delta is short; an optional value appearing or going is a SET.
    let rows = [
        (20, row(2, Some(-1_000), b"b", &[2, 2], b"yy", true)),
        (10, row(200, Some(5), b"a", &[], b"", false)),
        (30, row(1, None, b"c", &[3], b"z", true)),
    ];
    assert_eq!(fixture.changes(&rows), census(2, 1, 0));
    // A cell recorded again after it was not takes a SET, however close the stale value.
    let mut unrecorded = row(2, Some(-1_000), b"b", &[2, 2], b"yy", true);
    unrecorded[0] = Input::Unrecorded;
    assert_eq!(fixture.changes(&[(20, unrecorded)]), census(0, 0, 0));
    let rows = [(20, row(3, Some(-1_000), b"b", &[2, 2], b"yy", true))];
    assert_eq!(fixture.changes(&rows), census(1, 0, 0));
    // Byte-wide values are raw either way.
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
    // One item of eight in each list, visited against slot order; two bytes of sixteen, one
    // hunk across the equal byte between them; and a two-byte cell whose edit is longer.
    let (first, second) = ([1, 2, 3, 4, 5, 6, 7, 9], [11, 12, 13, 14, 15, 16, 17, 19]);
    let rows = [
        (20, row(2, None, b"b", &second, b"zy", false)),
        (10, row(1, None, b"a", &first, b"aXcXefghijklmnop", false)),
    ];
    assert_eq!(fixture.changes(&rows), census(1, 0, 3));
    // A grown list and a shrunk opaque cell keep their common prefix.
    let grown = [1, 2, 3, 4, 5, 6, 7, 9, 10];
    let rows = [(10, row(1, None, b"a", &grown, b"aXcXefghijklmno", false))];
    assert_eq!(fixture.changes(&rows), census(0, 0, 2));
    // Edits as long as the cell are a SET.
    let replaced = [21, 22, 23, 24, 25, 26, 27, 28, 29];
    let rows = [(10, row(1, None, b"a", &replaced, b"aXcXefghijklmno", false))];
    assert_eq!(fixture.changes(&rows), census(1, 0, 0));
}

#[test]
fn a_column_of_one_value_is_one_set_run_or_a_fill_of_the_live_rows() {
    let mut fixture = Fixture::new(&many(1..=20, |key| key as u32, |_| false));
    // Every row's flag: a FILL with no exception, its slots apart from the changed list.
    let rows = many(1..=20, |key| key as u32, |_| true);
    let ((hour, ops), changes) = fixture.step(Coverage::Complete, COMPLETE, &rows);
    let changes = changes.unwrap();
    let fill = (hour.fills, hour.fill_slots, hour.fill_exceptions);
    assert_eq!(fill, (1, 20, 0));
    assert_eq!(ops, manifest::FILLS);
    assert!(changes.changed.is_empty());
    assert_eq!(changes.fill_slots, (0..20).collect::<Vec<u32>>());
    // A partial read's sixteen rows of twenty: the four it did not read are exceptions.
    let rows = many(1..=16, |key| key as u32, |_| false);
    let ((hour, _), _) = fixture.step(Coverage::Partial, TRUNCATED, &rows);
    assert_eq!((hour.fills, hour.fill_exceptions), (1, 4));
    let unread = many(17..=20, |key| key as u32, |_| true);
    holds(&fixture.reader, &mut fixture.dictionary, &unread);
    // Sixteen values of the 21 rows live after a delete and two inserts: the rows left and
    // the inserted ones are exceptions.
    let flag = |key: u128| (17..=20).contains(&key);
    let rows = many(2..=22, |key| if key <= 17 { 0 } else { key as u32 }, flag);
    let ((hour, _), changes) = fixture.step(Coverage::Complete, COMPLETE, &rows);
    let changes = changes.unwrap();
    let fill = (hour.fills, hour.fill_slots, hour.fill_exceptions);
    assert_eq!(fill, (1, 16, 5));
    assert_eq!((changes.deleted.len(), changes.inserted.len()), (1, 2));
    // Three of one value, whose deltas alone would be ADDs, are one SET run.
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
    // Fifteen of one value are fewer than a FILL takes.
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
    // Sixteen of 33 live rows are one SET run.
    let rows = many(1..=16, |_| 100, |_| false);
    assert_eq!(fixture.changes(&rows), census(16, 0, 0));
    // Sixteen of the 32 left after a delete are a FILL.
    let rows = many(1..=16, |_| 200, |_| false);
    let coverage = Coverage::Changes { removed: &[33] };
    let ((hour, _), _) = fixture.step(coverage, COMPLETE, &rows);
    let fill = (hour.fills, hour.fill_slots, hour.fill_exceptions);
    assert_eq!(fill, (1, 16, 16));
    holds(&fixture.reader, &mut fixture.dictionary, &first[16..32]);
}

/// `rows` as `batch`'s rows of a `kinds` table, each cell as [`Hour::row`] takes it.
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

/// Every kind, and a second Bytes column whose strings intern beside the first's.
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

/// One row of every [`WIDE`] column, owned.
#[derive(Clone)]
struct Wide {
    key: u128,
    scalars: [Cell<'static>; 13],
    names: [Vec<u8>; 2],
    u32s: Vec<u32>,
    u64s: Vec<u64>,
    opaque: Vec<u8>,
    /// A bit per column whose cell the recording did not record.
    unrecorded: u32,
}

impl Wide {
    /// Row `key`, its values drawn from `value`.
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

    /// The row with scalar column `column` holding `cell`.
    fn with(mut self, column: usize, cell: Cell<'static>) -> Self {
        self.scalars[column] = cell;
        self
    }
}

/// One path's writer and dictionary, and the walker its sections build.
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

    /// The hour's rows, one at a time.
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

    /// Applies `prepared` as a walker would, then commits it.
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

/// The same hours staged a row at a time and as one batch, by two writers, and both ways
/// again by two writers clearing and finishing every column every hour.
struct Twin {
    rows: Side,
    batch: Side,
    /// A row at a time, then as one batch, sweeping every column.
    swept: [Side; 2],
    /// One batch for every hour, as a run keeps it.
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

    /// One hour through every path: the same section, manifest entry, census and strings,
    /// and then the same held table, view and walker.
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

    /// The batch's hour found its held rows' slots by key.
    fn looked_up(&self) -> bool {
        !self.batch.writer.visited.is_empty()
    }

    /// Every writer's restart, which is the same.
    fn restart(&self) -> Vec<u8> {
        let bytes = self.rows.writer.restart().unwrap();
        for side in std::iter::once(&self.batch).chain(&self.swept) {
            assert_eq!(side.writer.restart().unwrap(), bytes);
        }
        bytes
    }

    /// Every writer restored from `bytes`, sweeping as it did.
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
    // The same rows in the same order: their slots are known, and nothing changes.
    assert!(twin.hour(Coverage::Complete, COMPLETE, &first).is_none());
    assert!(!twin.looked_up());
    // Every kind changes in place: ADDs, SETs and PATCHes, a FILL, an optional value going
    // and one appearing, a cell no longer recorded, and new strings in both text columns,
    // one twice and one already another column's, interned in row order.
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
    // Reversed, without key 50, with two new rows, one leaving cells unrecorded, and the
    // cell no longer recorded recorded again as it was.
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
    // The same rows in key order change nothing; that order is then the one known.
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
    // A partial read changes rows but brings or drops none, so the order still holds.
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
    // A removed row ends the order; the next complete hour finds its rows by key.
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
    // Three cells of one value are one SET run.
    let mut tenth = ninth.clone();
    for row in &mut tenth[..3] {
        row.scalars[5] = Cell::I32(77);
    }
    let changes = twin.hour(Coverage::Complete, COMPLETE, &tenth).unwrap();
    assert!(!twin.looked_up());
    assert!(changes.filled.is_empty());
    // A partial read with a new row ends the order too.
    let read = [tenth[1].clone().with(1, Cell::I8(9)), Wide::new(1, 40)];
    twin.hour(Coverage::Partial, TRUNCATED, &read);
    assert!(!twin.batch.writer.order.held);
    // Writers restored from either's restart go on as both would.
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
    // A cell before its row's key, of another kind, or a second of a column in a row is
    // refused as it is added.
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
    // A row short of a cell, a key visited twice, a removed key changed, and another table's
    // columns are refused by the hour.
    assert!(writer.prepare(&mut dictionary, Coverage::Complete, &batch).is_err());
    batched(&mut batch, &KINDS, [(10, &same[..]), (10, &same[..])].into_iter()).unwrap();
    assert!(writer.prepare(&mut dictionary, Coverage::Complete, &batch).is_err());
    batched(&mut batch, &KINDS, [(20, &same[..])].into_iter()).unwrap();
    let removed = Coverage::Changes { removed: &[20] };
    assert!(writer.prepare(&mut dictionary, removed, &batch).is_err());
    batched(&mut batch, &KINDS[..5], [(10, &same[..5])].into_iter()).unwrap();
    assert!(writer.prepare(&mut dictionary, Coverage::Complete, &batch).is_err());
    assert_eq!(writer.table(), &reader);
    // A refused hour leaves the next one exact.
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

/// Hours that change nothing clear and finish no column, whether their rows are staged whole,
/// as changes of no row or as a partial read; a changed column is finished, then cleared,
/// alone; a refused hour has the next clear every column. The writers sweeping every column
/// write the same.
#[test]
fn an_hour_works_only_the_columns_it_staged() {
    fn quiet(twin: &mut Twin, rows: &[Wide]) {
        assert!(twin.hour(Coverage::Complete, COMPLETE, rows).is_none());
    }
    let all = WIDE.len() as u64;
    let w = |hours, columns| Work { hours, columns };
    let mut twin = Twin::new();
    let mut rows: Vec<Wide> = (1..=40).map(|key| Wide::new(key * 10, key as u32)).collect();
    // The inserts stage every column; the next hour clears them all, then none.
    twin.hour(Coverage::Complete, COMPLETE, &rows);
    assert_eq!(twin.batch.writer.work(), w(1, all));
    quiet(&mut twin, &rows);
    assert_eq!(twin.batch.writer.work(), w(2, 2 * all));
    for _ in 0..10 {
        quiet(&mut twin, &rows);
    }
    assert_eq!(twin.batch.writer.work(), w(12, 2 * all));
    assert!(twin.hour(Coverage::Changes { removed: &[] }, COMPLETE, &[]).is_none());
    twin.hour(Coverage::Partial, TRUNCATED, &rows[10..15]);
    quiet(&mut twin, &rows);
    assert_eq!(twin.batch.writer.work(), w(15, 2 * all));
    // One changed cell.
    rows[7] = rows[7].clone().with(4, Cell::U32(1));
    twin.hour(Coverage::Changes { removed: &[] }, COMPLETE, &rows[7..8]);
    assert_eq!(twin.batch.writer.work(), w(16, 2 * all + 1));
    quiet(&mut twin, &rows);
    quiet(&mut twin, &rows);
    assert_eq!(twin.batch.writer.work(), w(18, 2 * all + 2));
    // A new string in a text column and a cell no longer recorded in another.
    rows[5].names[0] = b"renamed".to_vec();
    rows[3].unrecorded = 1 << 0;
    twin.hour(Coverage::Complete, COMPLETE, &rows).unwrap();
    quiet(&mut twin, &rows);
    assert_eq!(twin.batch.writer.work(), w(20, 2 * all + 6));
    assert_eq!(twin.rows.writer.work(), twin.batch.writer.work());
    for side in &twin.swept {
        assert_eq!(side.writer.work(), w(20, 40 * all));
    }
    // A refused hour may leave cells in any column.
    let inputs = rows[0].inputs();
    let twice = [(rows[0].key, &inputs[..]), (rows[0].key, &inputs[..])];
    batched(&mut twin.staged, &WIDE, twice.into_iter()).unwrap();
    let Side { writer, dictionary, .. } = &mut twin.batch;
    assert!(writer.prepare(dictionary, Coverage::Complete, &twin.staged).is_err());
    assert_eq!(writer.work(), w(21, 2 * all + 6));
    quiet(&mut twin, &rows);
    assert_eq!(twin.batch.writer.work(), w(22, 3 * all + 6));
    assert_eq!(twin.rows.writer.work(), w(21, 2 * all + 6));
    for side in &twin.swept {
        assert_eq!(side.writer.work(), w(21, 42 * all));
    }
    twin.restart();
}

/// A batch keeps no cell of an earlier table's hour: a wider table's rows, then a narrower
/// table's, then an hour of no rows, each stage only their own.
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

/// Writers holding `first`: one clearing and finishing the columns its hours touch, one every
/// column.
fn holding(first: &[Wide]) -> [Side; 2] {
    let rows = inputs(first);
    [Side::new(), Side::sweeping(Sweep::Every)].map(|mut side| {
        let prepared = side.rows(Coverage::Complete, &rows);
        side.commit(prepared, COMPLETE, 1);
        side.dictionary.take_appended();
        side
    })
}

/// A writer new from `side`'s restart, holding the same strings: it has begun no hour.
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

/// An attempt at an hour reading the run's strings, as tables prepared side by side do: its
/// rows one at a time, or `batch`.
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

/// The second hour by `sides`, which abandoned attempts at it, and by a writer new from their
/// restart, which began none: the same section, manifest entry, census and strings, then the
/// same table.
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

/// An hour that stages cells in several columns, then misses a string the run has not used
/// while reading the run's strings, is prepared again interning: a row at a time or as one
/// batch, it writes what a writer that never began it writes, as does the writer sweeping every
/// column.
#[test]
fn an_hour_prepared_again_after_a_missed_string_writes_what_it_would_have() {
    let first: Vec<Wide> = (1..=40)
        .map(|key| Wide::new(key * 10, key as u32))
        .collect();
    // A FILL, a SET, PATCHes of a list and an opaque cell and a cell no longer recorded, then
    // late in the hour a changed cell beside a new name, and another new name.
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

/// An hour refused as it finishes, after staging cells in several columns, has the next write
/// what a writer that never began it writes: a row at a time, refused for a touch of a row it
/// did not visit once its columns' runs are laid out; as one batch, for a removed row the table
/// does not hold.
#[test]
fn an_hour_after_one_refused_as_it_finishes_writes_what_it_would_have() {
    let first: Vec<Wide> = (1..=40)
        .map(|key| Wide::new(key * 10, key as u32))
        .collect();
    // A FILL, a grown list, an optional value going, a cell no longer recorded and a string the
    // run has used.
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

/// An hour begun, staged into and dropped before it finishes has the next clear every column,
/// though it marked those it staged in, and write what a writer that never began it writes.
#[test]
fn an_hour_after_one_dropped_unfinished_writes_what_it_would_have() {
    let first: Vec<Wide> = (1..=40)
        .map(|key| Wide::new(key * 10, key as u32))
        .collect();
    // A FILL, an opaque cell cut short and a cell no longer recorded, in the columns the next
    // hour changes.
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

/// Committing an hour's FILLs grows nothing once the writer's buffers are warm: their slots
/// and masks merge in the buffers its apply keeps, copying no FILL's slots.
#[test]
fn committing_fills_grows_nothing_once_warm() {
    // Every row's value changes each hour; the flag changes in all rows but the first four.
    let rows = |at: u32| many(1..=40, move |_| at % 2, move |key| at % 2 == 1 && key > 4);
    let mut writer = TableWriter::new(&KINDS).unwrap();
    let mut dictionary = Dictionary::new();
    let mut masks: Vec<u64> = vec![0b1; 4];
    masks.resize(40, 0b10_0001);
    for at in 0..8 {
        let prepared = prepare(&mut writer, &mut dictionary, Coverage::Complete, &rows(at)).unwrap();
        if at > 0 {
            let census = prepared.census;
            assert_eq!((census.fills, census.fill_slots, census.fill_exceptions), (2, 76, 4));
        }
        let before = engine_allocation::growths();
        writer.commit(Some(prepared), COMPLETE, at as i32 + 1).unwrap();
        // The first hours size the buffers; the later ones only reuse them.
        if at >= 3 {
            assert_eq!(engine_allocation::growths(), before, "hour {at}");
        }
        if at > 0 {
            assert_eq!(writer.changes.fill_slots, (0..40).collect::<Vec<u32>>());
            assert_eq!(writer.changes.fill_masks, masks);
        }
    }
}
