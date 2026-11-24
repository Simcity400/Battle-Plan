use super::*;
use replay_columnar::{
    section::{self, Changes, Scratch},
    state::Table,
    walk,
    writer::Dictionary,
};
use replay_data::rows::{
    DivisionHistoryRow, Fields, Recorded, SharedDivisionHistoryRow, ShipNamingOperands, ShipRow,
};
use std::sync::Arc;

const READ: Sample = Sample::FromReadBit;

struct Table1<R> {
    writer: TableWriter,
    reader: Table,
    columns: Vec<Column>,
    dictionary: Dictionary,
    strings: walk::Dictionary,
    cells: RowCells,
    hour: i32,
    section: Option<Vec<u8>>,
    _rows: std::marker::PhantomData<R>,
}

impl<R: RowCodec + Fields> Table1<R> {
    fn new() -> Self {
        Self::staged(Staging::default())
    }

    fn staged(staging: Staging) -> Self {
        let columns = columns(R::SCHEMA).unwrap();
        Self {
            writer: TableWriter::new(&columns).unwrap(),
            reader: Table::new(&columns).unwrap(),
            columns,
            dictionary: Dictionary::new(),
            strings: walk::Dictionary::default(),
            cells: RowCells::new(staging),
            hour: 0,
            section: None,
            _rows: std::marker::PhantomData,
        }
    }

    fn hour(&mut self, input: TableInput<'_, R>) -> Option<Changes> {
        self.hour += 1;
        let hour = table_hour(
            &mut self.writer,
            &mut self.dictionary,
            &mut self.cells,
            input,
        )
        .unwrap();
        for string in self.dictionary.take_appended() {
            self.strings.append(&string).unwrap();
        }
        let truncated = matches!(hour.evidence.status, Status::Truncated { .. });
        let section = hour.prepared.as_ref().and_then(|prepared| prepared.section.clone());
        self.section = section.clone();
        let applied = section.map(|bytes| {
            let mut changes = Changes::default();
            section::apply(
                &bytes,
                &self.columns,
                truncated,
                &mut self.reader,
                &mut Scratch::default(),
                &mut changes,
            )
            .unwrap();
            changes
        });
        self.writer
            .commit(hour.prepared, hour.evidence, self.hour)
            .unwrap();
        assert_eq!(self.writer.table(), &self.reader);
        applied
    }

    fn holds(&self, rows: &[R]) {
        assert_eq!(self.reader.live().count(), rows.len());
        for row in rows {
            let slot = self.reader.slot(row.key()).expect("row held");
            row.visit_wire(&mut |column, field| {
                let recorded = self.reader.recorded(column, slot);
                let cell = self.reader.column(column).unwrap().cell(slot).unwrap();
                let mut wire = Vec::new();
                match field {
                    Field::NotRecorded => assert!(!recorded, "column {column}"),
                    Field::Bytes(bytes) => {
                        let Cell::Bytes(id) = cell else {
                            panic!("column {column} holds {cell:?}")
                        };
                        assert!(recorded);
                        assert_eq!(self.strings.get(id), Some(bytes));
                    }
                    Field::Opaque(_) | Field::U32List(_) | Field::U64List(_) => {
                        assert!(recorded);
                        field.encode(&mut wire).unwrap();
                        let payload = &wire[4..];
                        match cell {
                            Cell::Opaque(bytes) => assert_eq!(bytes, payload),
                            Cell::U32List(items) => {
                                let expected: Vec<u32> = payload
                                    .chunks_exact(4)
                                    .map(|item| u32::from_le_bytes(item.try_into().unwrap()))
                                    .collect();
                                assert_eq!(items, expected.as_slice());
                            }
                            Cell::U64List(items) => {
                                let expected: Vec<u64> = payload
                                    .chunks_exact(8)
                                    .map(|item| u64::from_le_bytes(item.try_into().unwrap()))
                                    .collect();
                                assert_eq!(items, expected.as_slice());
                            }
                            other => panic!("column {column} holds {other:?}"),
                        }
                    }
                    scalar => {
                        assert!(recorded);
                        let expected = match scalar {
                            Field::U8(value) => Cell::U8(value),
                            Field::I8(value) => Cell::I8(value),
                            Field::U16(value) => Cell::U16(value),
                            Field::I16(value) => Cell::I16(value),
                            Field::U32(value) => Cell::U32(value),
                            Field::I32(value) => Cell::I32(value),
                            Field::U64(value) => Cell::U64(value),
                            Field::I64(value) => Cell::I64(value),
                            Field::Bool(value) => Cell::Bool(value),
                            Field::OptionalI32(value) => Cell::OptionalI32(value),
                            Field::OptionalU32(value) => Cell::OptionalU32(value),
                            Field::OptionalI64(value) => Cell::OptionalI64(value),
                            Field::OptionalU64(value) => Cell::OptionalU64(value),
                            _ => unreachable!("lists, bytes and opaque cells above"),
                        };
                        assert_eq!(cell, expected, "column {column}");
                    }
                }
                Ok(())
            })
            .unwrap();
        }
    }
}

fn ship(id: u64, experience: i64, required: Recorded<Option<i32>>, name: Option<&[u8]>) -> ShipRow {
    ShipRow {
        id,
        task_force: 3,
        strength: -5,
        organisation: 9,
        experience,
        category: 2,
        sunk_convoys: 1,
        class_id: -4,
        acquired_manpower: Some(11).into(),
        required_manpower: required,
        name: name.map(<[u8]>::to_vec).into(),
        naming: Some(ShipNamingOperands {
            kind: 1,
            order: id as i32,
            ordered: 1,
            override_name: b"x".to_vec(),
            programmatic: 0,
            equipment: None,
        })
        .into(),
    }
}

#[test]
fn ship_hours_land_in_the_walkers_columns() {
    let mut table = Table1::<ShipRow>::new();
    let first = [
        ship(7, 100_000, Recorded::NotRecorded, Some(b"Ark".as_slice())),
        ship(8, 0, Some(4).into(), None),
    ];
    let changes = table
        .hour(TableInput::Snapshot {
            rows: &first,
            partial: None,
            read: true,
            sample: READ,
        })
        .unwrap();
    assert_eq!(changes.inserted.len(), 2);
    table.holds(&first);
    let changed = [
        ship(7, 200_001, Some(2).into(), Some(b"Ark".as_slice())),
        ship(9, 5, Recorded::NotRecorded, Some(b"New".as_slice())),
    ];
    let changes = table
        .hour(TableInput::Changes {
            rows: &changed,
            removed: &[],
            read: true,
            sample: READ,
        })
        .unwrap();
    assert_eq!(changes.changed.len(), 1);
    assert_eq!(changes.inserted.len(), 1);
    table.holds(&[changed[0].clone(), first[1].clone(), changed[1].clone()]);
    let census = [first[1].clone(), ship(9, 5, Recorded::NotRecorded, None)];
    let changes = table
        .hour(TableInput::Snapshot {
            rows: &census,
            partial: None,
            read: true,
            sample: READ,
        })
        .unwrap();
    assert_eq!(changes.deleted.len(), 1);
    table.holds(&census);
    assert!(
        table
            .hour(TableInput::Unavailable {
                reason: "refused",
                read: false,
                sample: READ,
            })
            .is_none()
    );
    assert!(matches!(table.writer.status(), Status::Unavailable { .. }));
    table.holds(&census);
    assert!(
        table
            .hour(TableInput::Same {
                read: false,
                sample: Sample::Carried(3),
            })
            .is_none()
    );
    assert_eq!(table.writer.acquisition().last_read, Some(3));
    let read = [ship(8, 7, Some(4).into(), None)];
    let changes = table
        .hour(TableInput::Snapshot {
            rows: &read,
            partial: Some(1),
            read: true,
            sample: READ,
        })
        .unwrap();
    assert!(changes.truncated);
    assert_eq!(changes.read.len(), 1);
    table.holds(&[read[0].clone(), census[1].clone()]);
}

struct Both<R> {
    columns: Table1<R>,
    rows: Table1<R>,
}

impl<R: RowCodec + Fields> Both<R> {
    fn new() -> Self {
        Self {
            columns: Table1::staged(Staging::Columns),
            rows: Table1::staged(Staging::Rows),
        }
    }

    fn hour<'r>(&mut self, input: impl Fn() -> TableInput<'r, R>) -> Option<Changes>
    where
        R: 'r,
    {
        let changes = self.rows.hour(input());
        let at = self.rows.hour;
        assert_eq!(self.columns.hour(input()), changes, "hour {at}");
        assert_eq!(self.columns.section, self.rows.section, "hour {at}");
        assert_eq!(self.columns.strings, self.rows.strings, "hour {at}");
        let restart = self.rows.writer.restart().unwrap();
        assert_eq!(self.columns.writer.restart().unwrap(), restart, "hour {at}");
        changes
    }
}

fn snapshot<R>(rows: &[R]) -> TableInput<'_, R> {
    TableInput::Snapshot {
        rows,
        partial: None,
        read: true,
        sample: READ,
    }
}

#[test]
fn staging_by_column_writes_what_staging_a_row_at_a_time_writes() {
    let mut ships = Both::<ShipRow>::new();
    let first = [
        ship(7, 100_000, Recorded::NotRecorded, Some(b"Ark".as_slice())),
        ship(8, 0, Some(4).into(), None),
    ];
    ships.hour(|| snapshot(&first));
    assert!(ships.hour(|| snapshot(&first)).is_none());
    let second = [
        ship(7, 200_001, Some(2).into(), None),
        ship(8, 0, Some(4).into(), None),
    ];
    ships.hour(|| snapshot(&second)).unwrap();
    let changed = [ship(9, 5, Recorded::NotRecorded, Some(b"New".as_slice()))];
    ships.hour(|| TableInput::Changes {
        rows: &changed,
        removed: &[],
        read: true,
        sample: READ,
    });
    let census = [second[1].clone(), ship(9, 6, Recorded::NotRecorded, None)];
    ships.hour(|| snapshot(&census)).unwrap();
    ships.hour(|| TableInput::Unavailable {
        reason: "refused",
        read: false,
        sample: READ,
    });
    let read = [ship(8, 7, Some(4).into(), None)];
    ships.hour(|| TableInput::Snapshot {
        rows: &read,
        partial: Some(1),
        read: true,
        sample: READ,
    });
    let all = [read[0].clone(), census[1].clone(), ship(10, 1, Some(1).into(), None)];
    ships.hour(|| snapshot(&all)).unwrap();
    ships.hour(|| snapshot(&all));

    let mut histories = Both::<SharedDivisionHistoryRow>::new();
    let row = |division: u64, name: &[u8], provinces: &[u32]| {
        Arc::new(DivisionHistoryRow {
            division,
            name: name.to_vec(),
            reason: b"order".to_vec(),
            provinces: provinces.to_vec(),
            medal_token: Some(-2),
            ..Default::default()
        })
    };
    let first = [row(1, b"First", &[3, 1]), row(2, b"Second", &[])];
    histories.hour(|| snapshot(&first));
    assert!(histories.hour(|| snapshot(&first)).is_none());
    let renamed = [row(1, b"Renamed", &[3, 1, 4]), first[1].clone()];
    histories.hour(|| snapshot(&renamed)).unwrap();
    let all = [renamed[0].clone(), row(2, b"Second", &[9]), row(3, b"Third", &[1])];
    let keys = [RowCodec::key(&all[1]), RowCodec::key(&all[2])];
    histories.hour(|| TableInput::Keyed {
        rows: &all,
        keys: &keys,
        removed: &[],
        read: true,
        sample: READ,
    });
    assert!(histories.hour(|| snapshot(&all)).is_none());
    assert!(histories.hour(|| snapshot(&all)).is_none());
}

#[test]
fn changes_of_no_row_begin_no_hour() {
    let mut touched = Table1::<ShipRow>::new();
    let mut swept = Table1::<ShipRow>::new();
    swept.writer.set_sweep(Sweep::Every);
    let first = [ship(7, 100_000, Recorded::NotRecorded, Some(b"Ark".as_slice()))];
    for table in [&mut touched, &mut swept] {
        table.hour(snapshot(&first)).unwrap();
    }
    let begun = touched.writer.work();
    for table in [&mut touched, &mut swept] {
        let changes = TableInput::Changes {
            rows: &[],
            removed: &[],
            read: true,
            sample: READ,
        };
        assert!(table.hour(changes).is_none());
        let keyed = TableInput::Keyed {
            rows: &first,
            keys: &[],
            removed: &[],
            read: false,
            sample: READ,
        };
        assert!(table.hour(keyed).is_none());
        table.holds(&first);
    }
    assert_eq!(touched.writer.work(), begun);
    assert_eq!(swept.writer.work().hours, 3);
    assert_eq!(touched.writer.restart().unwrap(), swept.writer.restart().unwrap());
}

#[test]
fn an_hour_written_again_after_a_refusal_writes_what_it_would_have() {
    let row = |division: u64, amount: i32, name: &[u8]| {
        Arc::new(DivisionHistoryRow {
            division,
            amount,
            name: name.to_vec(),
            reason: b"order".to_vec(),
            provinces: vec![1, amount as u32],
            ..Default::default()
        })
    };
    let first: Vec<_> = (1..=20).map(|division| row(division, 1, b"Held")).collect();
    let mut changed = first.clone();
    for (division, history) in (1..).zip(&mut changed[..12]) {
        *history = row(division, 5, b"Held");
    }
    changed[10] = row(11, 5, b"Renamed");
    let unknown = RowCodec::key(&row(99, 0, b""));
    let keys: Vec<u128> = changed[..10]
        .iter()
        .map(RowCodec::key)
        .chain([unknown])
        .collect();
    for staging in [Staging::Columns, Staging::Rows] {
        let mut tables: [Table1<SharedDivisionHistoryRow>; 3] =
            std::array::from_fn(|_| Table1::staged(staging));
        tables[1].writer.set_sweep(Sweep::Every);
        for table in &mut tables {
            table.hour(snapshot(&first)).unwrap();
        }
        let [refused, swept, never] = &mut tables;
        for table in [&mut *refused, &mut *swept] {
            let keyed = TableInput::Keyed {
                rows: &changed,
                keys: &keys,
                removed: &[],
                read: true,
                sample: READ,
            };
            let strings = Strings::Reading(&table.dictionary);
            assert!(table_hour(&mut table.writer, strings, &mut table.cells, keyed).is_err());
            let strings = Strings::Reading(&table.dictionary);
            let missed = table_hour(
                &mut table.writer,
                strings,
                &mut table.cells,
                snapshot(&changed),
            );
            assert!(missed.is_err());
        }
        let changes = never.hour(snapshot(&changed));
        for table in [refused, swept] {
            assert_eq!(table.hour(snapshot(&changed)), changes, "{staging:?}");
            assert_eq!(table.section, never.section, "{staging:?}");
            assert_eq!(table.strings, never.strings, "{staging:?}");
            let restart = never.writer.restart().unwrap();
            assert_eq!(table.writer.restart().unwrap(), restart, "{staging:?}");
        }
    }
}

#[test]
fn paired_and_aligned_sparse_staging_matches_row_staging() {
    let mut ships = Both::<ShipRow>::new();
    let h1 = [
        ship(7, 100_000, Recorded::NotRecorded, Some(b"Ark".as_slice())),
        ship(8, 0, Some(4).into(), None),
        ship(10, 50, Some(9).into(), Some(b"Hood".as_slice())),
    ];
    ships.hour(|| snapshot(&h1)).unwrap();

    let h2 = [
        ship(7, 100_000, Recorded::NotRecorded, Some(b"Ark".as_slice())),
        ship(8, 77, Some(4).into(), None),
        ship(10, 50, Recorded::NotRecorded, Some(b"Hood".as_slice())),
    ];
    ships
        .hour(|| TableInput::Aligned {
            rows: &h2,
            previous: &h1,
            read: true,
            sample: READ,
        })
        .unwrap();
    ships.columns.holds(&h2);

    assert!(
        ships
            .hour(|| TableInput::Aligned {
                rows: &h2,
                previous: &h2,
                read: true,
                sample: READ,
            })
            .is_none()
    );

    let h3 = [
        ship(7, 100_000, Some(12).into(), Some(b"Ark Royal".as_slice())),
        ship(8, 77, Some(4).into(), None),
        ship(11, 9, Recorded::NotRecorded, Some(b"New".as_slice())),
    ];
    let pairs = [Pair::new(0u32, Some(0u32)), Pair::new(2u32, None)];
    let removed = [RowCodec::key(&h2[2])];
    ships
        .hour(|| TableInput::Paired {
            rows: &h3,
            previous: &h2,
            pairs: &pairs,
            removed: &removed,
            uniform_optional_i32: None,
            read: true,
            sample: READ,
        })
        .unwrap();
    ships.columns.holds(&h3);

    let mut histories = Both::<SharedDivisionHistoryRow>::new();
    let row = |division: u64, amount: i32, name: &[u8], provinces: &[u32]| {
        Arc::new(DivisionHistoryRow {
            division,
            amount,
            name: name.to_vec(),
            reason: b"order".to_vec(),
            provinces: provinces.to_vec(),
            medal_token: Some(-2),
            ..Default::default()
        })
    };
    let a = [row(1, 1, b"First", &[3, 1]), row(2, 2, b"Second", &[])];
    histories.hour(|| snapshot(&a)).unwrap();
    let b = [
        row(1, 5, b"First", &[3, 1, 9]),
        a[1].clone(),
        row(3, 7, b"Third", &[4]),
    ];
    let pairs = [Pair::new(0u32, Some(0u32)), Pair::new(2u32, None)];
    histories
        .hour(|| TableInput::Paired {
            rows: &b,
            previous: &a,
            pairs: &pairs,
            removed: &[],
            uniform_optional_i32: None,
            read: true,
            sample: READ,
        })
        .unwrap();
    histories.columns.holds(&b);
}
