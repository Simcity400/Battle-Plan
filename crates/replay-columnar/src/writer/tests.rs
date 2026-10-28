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
    writer.commit(Some(prepared), evidence, at).unwrap();
    assert_eq!(writer.table(), &*reader);
    applied
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
