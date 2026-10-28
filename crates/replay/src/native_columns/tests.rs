use super::*;
use replay_columnar::{
    section::{self, Changes, Scratch},
    state::Table,
    walk,
};
use replay_data::rows::{
    DivisionHistoryRow, Fields, Recorded, SharedDivisionHistoryRow, ShipNamingOperands, ShipRow,
};
use std::sync::Arc;

const READ: Sample = Sample::FromReadBit;

/// One table's writer beside the reader state a walker builds from its sections.
struct Table1<R> {
    writer: TableWriter,
    reader: Table,
    columns: Vec<Column>,
    dictionary: Dictionary,
    strings: walk::Dictionary,
    cells: RowCells,
    hour: i32,
    _rows: std::marker::PhantomData<R>,
}

impl<R: RowCodec + Fields> Table1<R> {
    fn new() -> Self {
        let columns = columns(R::SCHEMA).unwrap();
        Self {
            writer: TableWriter::new(&columns).unwrap(),
            reader: Table::new(&columns).unwrap(),
            columns,
            dictionary: Dictionary::new(),
            strings: walk::Dictionary::default(),
            cells: RowCells::default(),
            hour: 0,
            _rows: std::marker::PhantomData,
        }
    }

    /// Writes and commits one hour, applying its section as a walker would.
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

    /// The reader holds exactly `rows`, each cell its wire field.
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
                        // The wire carries a length or count before the payload.
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
    // Proven changes: a stepped value, a newly recorded cell and a new ship.
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
    // A census without ship 7 deletes it.
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
    // No rows this hour: nothing written, nothing held changes.
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
    // A partial walk changes what it read and keeps the rest.
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

#[test]
fn shared_history_rows_keep_their_strings_and_lists() {
    let mut table = Table1::<SharedDivisionHistoryRow>::new();
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
    table.hour(TableInput::Snapshot {
        rows: &first,
        partial: None,
        read: true,
        sample: READ,
    });
    table.holds(&first);
    let renamed = [row(1, b"Renamed", &[3, 1, 4]), first[1].clone()];
    let changes = table
        .hour(TableInput::Snapshot {
            rows: &renamed,
            partial: None,
            read: true,
            sample: READ,
        })
        .unwrap();
    assert_eq!(changes.changed.len(), 1);
    table.holds(&renamed);
    // Each distinct string is appended once: the reason, both names, the empty custom
    // name, then the new name.
    assert_eq!(table.strings.len(), 5);
}
