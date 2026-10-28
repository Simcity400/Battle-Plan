//! Typed recording rows as the columnar writer's cells: each schema field in the wire
//! form the row path encodes, so a native producer and a converted recording write the
//! same columns.

use replay_columnar::{
    manifest::Status,
    state::{Cell, Kind as Column},
    writer::{Coverage, Dictionary, Evidence, Hour, Input, Prepared, Sample, TableWriter},
};
use replay_data::rows::{Field, Kind, List, Opaque, RowCodec};
use std::io;

#[cfg(test)]
mod tests;

/// A schema field's column kind.
pub fn column(kind: Kind) -> Column {
    match kind {
        Kind::U8 => Column::U8,
        Kind::I8 => Column::I8,
        Kind::U16 => Column::U16,
        Kind::I16 => Column::I16,
        Kind::U32 => Column::U32,
        Kind::I32 => Column::I32,
        Kind::U64 => Column::U64,
        Kind::I64 => Column::I64,
        Kind::Bool => Column::Bool,
        Kind::OptionalI32 => Column::OptionalI32,
        Kind::OptionalU32 => Column::OptionalU32,
        Kind::OptionalI64 => Column::OptionalI64,
        Kind::OptionalU64 => Column::OptionalU64,
        Kind::Bytes => Column::Bytes,
        Kind::U32List => Column::U32List,
        Kind::U64List => Column::U64List,
        Kind::Opaque => Column::Opaque,
    }
}

/// A table's column kinds, in schema order.
pub fn columns(schema: &[Kind]) -> io::Result<Vec<Column>> {
    engine_allocation::collect_vec(schema.iter().map(|&kind| column(kind)))
}

/// Row masks are u64: no schema has more fields.
const MAX_FIELDS: usize = 64;

/// One wire field, kept until the row's cells are handed over together.
#[derive(Clone, Copy)]
enum Entry {
    Cell(Cell<'static>),
    Text(usize, usize),
    Opaque(usize, usize),
    U32s(usize, usize),
    U64s(usize, usize),
    Unrecorded,
}

fn at<T>(values: &[T], (start, len): (usize, usize)) -> &[T] {
    &values[start..start + len]
}

/// Buffers that turn one row at a time into cells, kept between rows.
#[derive(Default)]
pub struct RowCells {
    entries: Vec<Entry>,
    bytes: Vec<u8>,
    u32s: Vec<u32>,
    u64s: Vec<u64>,
}

impl RowCells {
    fn take(&mut self, field: Field<'_>) -> io::Result<Entry> {
        Ok(match field {
            Field::Empty => return Err(invalid("A row field is missing")),
            Field::NotRecorded => Entry::Unrecorded,
            Field::U8(value) => Entry::Cell(Cell::U8(value)),
            Field::I8(value) => Entry::Cell(Cell::I8(value)),
            Field::U16(value) => Entry::Cell(Cell::U16(value)),
            Field::I16(value) => Entry::Cell(Cell::I16(value)),
            Field::U32(value) => Entry::Cell(Cell::U32(value)),
            Field::I32(value) => Entry::Cell(Cell::I32(value)),
            Field::U64(value) => Entry::Cell(Cell::U64(value)),
            Field::I64(value) => Entry::Cell(Cell::I64(value)),
            Field::Bool(value) => Entry::Cell(Cell::Bool(value)),
            Field::OptionalI32(value) => Entry::Cell(Cell::OptionalI32(value)),
            Field::OptionalU32(value) => Entry::Cell(Cell::OptionalU32(value)),
            Field::OptionalI64(value) => Entry::Cell(Cell::OptionalI64(value)),
            Field::OptionalU64(value) => Entry::Cell(Cell::OptionalU64(value)),
            Field::Bytes(bytes) => {
                let start = self.bytes.len();
                engine_allocation::reserve(&mut self.bytes, bytes.len())?;
                self.bytes.extend_from_slice(bytes);
                Entry::Text(start, bytes.len())
            }
            Field::Opaque(opaque) => {
                let start = self.bytes.len();
                match opaque {
                    // The cell is the payload a length prefix carries on the wire.
                    Opaque::Native(value) => value.encode_value(&mut self.bytes)?,
                    Opaque::Encoded(bytes, _) => {
                        engine_allocation::reserve(&mut self.bytes, bytes.len())?;
                        self.bytes.extend_from_slice(bytes);
                    }
                }
                Entry::Opaque(start, self.bytes.len() - start)
            }
            Field::U32List(list) => {
                let start = self.u32s.len();
                match list {
                    List::Native(values) => {
                        engine_allocation::reserve(&mut self.u32s, values.len())?;
                        self.u32s.extend_from_slice(values);
                    }
                    List::Encoded(bytes) => {
                        engine_allocation::reserve(&mut self.u32s, bytes.len() / 4)?;
                        self.u32s.extend(
                            bytes
                                .chunks_exact(4)
                                .map(|item| u32::from_le_bytes(item.try_into().expect("4 bytes"))),
                        );
                    }
                }
                Entry::U32s(start, self.u32s.len() - start)
            }
            Field::U64List(list) => {
                let start = self.u64s.len();
                match list {
                    List::Native(values) => {
                        engine_allocation::reserve(&mut self.u64s, values.len())?;
                        self.u64s.extend_from_slice(values);
                    }
                    List::Encoded(bytes) => {
                        engine_allocation::reserve(&mut self.u64s, bytes.len() / 8)?;
                        self.u64s.extend(
                            bytes
                                .chunks_exact(8)
                                .map(|item| u64::from_le_bytes(item.try_into().expect("8 bytes"))),
                        );
                    }
                }
                Entry::U64s(start, self.u64s.len() - start)
            }
        })
    }

    /// Hands `row` to `hour` as its wire fields' cells.
    pub fn row<R: RowCodec>(&mut self, hour: &mut Hour<'_>, row: &R) -> io::Result<()> {
        self.entries.clear();
        self.bytes.clear();
        self.u32s.clear();
        self.u64s.clear();
        row.visit_wire(&mut |index, field| {
            if index != self.entries.len() || index >= MAX_FIELDS {
                return Err(invalid("Row fields are not visited once each in schema order"));
            }
            let entry = self.take(field)?;
            engine_allocation::reserve(&mut self.entries, 1)?;
            self.entries.push(entry);
            Ok(())
        })?;
        let mut inputs = [Input::Unrecorded; MAX_FIELDS];
        for (input, &entry) in inputs.iter_mut().zip(&self.entries) {
            *input = match entry {
                Entry::Cell(cell) => Input::Value(cell),
                Entry::Text(start, len) => Input::Text(at(&self.bytes, (start, len))),
                Entry::Opaque(start, len) => {
                    Input::Value(Cell::Opaque(at(&self.bytes, (start, len))))
                }
                Entry::U32s(start, len) => {
                    Input::Value(Cell::U32List(at(&self.u32s, (start, len))))
                }
                Entry::U64s(start, len) => {
                    Input::Value(Cell::U64List(at(&self.u64s, (start, len))))
                }
                Entry::Unrecorded => Input::Unrecorded,
            };
        }
        hour.row(row.key(), &inputs[..self.entries.len()])
    }
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

/// What a native producer holds of one table this hour.
pub enum TableInput<'a, R> {
    /// Every public operand equals the accepted baseline: no rows are visited.
    Same { read: bool, sample: Sample },
    /// A complete table's proven changes: only `rows` may differ, and `removed` are gone.
    Changes {
        rows: &'a [R],
        removed: &'a [u128],
        read: bool,
        sample: Sample,
    },
    /// A captured walk: every row of a complete table, or the rows a partial walk read,
    /// with `missing` rows it could not enumerate.
    Snapshot {
        rows: &'a [R],
        partial: Option<u32>,
        read: bool,
        sample: Sample,
    },
    /// No rows this hour, for a reason the dictionary holds.
    Unavailable { reason: &'a str, read: bool, sample: Sample },
    NotRecorded { read: bool, sample: Sample },
}

/// An hour of one table: its section, if it has one, and the evidence its manifest
/// carries. [`TableWriter::commit`] applies both once the hour is accepted.
pub struct TableHour {
    pub prepared: Option<Prepared>,
    pub evidence: Evidence,
}

/// The section of one hour's `rows` under `coverage`.
fn visit<R: RowCodec>(
    writer: &mut TableWriter,
    dictionary: &mut Dictionary,
    cells: &mut RowCells,
    coverage: Coverage<'_>,
    rows: &[R],
) -> io::Result<Prepared> {
    let mut hour = writer.begin(dictionary, coverage)?;
    for row in rows {
        cells.row(&mut hour, row)?;
    }
    hour.finish()
}

/// Writes one table's hour from what its producer holds. Nothing held changes until
/// the hour commits.
pub fn table_hour<R: RowCodec>(
    writer: &mut TableWriter,
    dictionary: &mut Dictionary,
    cells: &mut RowCells,
    input: TableInput<'_, R>,
) -> io::Result<TableHour> {
    let evidence = |status, read, sample| Evidence {
        status,
        read,
        sample,
    };
    Ok(match input {
        TableInput::Same { read, sample } => TableHour {
            prepared: None,
            evidence: evidence(Status::Complete, read, sample),
        },
        TableInput::Changes {
            rows,
            removed,
            read,
            sample,
        } => TableHour {
            prepared: Some(visit(
                writer,
                dictionary,
                cells,
                Coverage::Changes { removed },
                rows,
            )?),
            evidence: evidence(Status::Complete, read, sample),
        },
        TableInput::Snapshot {
            rows,
            partial,
            read,
            sample,
        } => {
            let (coverage, status) = match partial {
                None => (Coverage::Complete, Status::Complete),
                Some(missing) => (Coverage::Partial, Status::Truncated { missing }),
            };
            TableHour {
                prepared: Some(visit(writer, dictionary, cells, coverage, rows)?),
                evidence: evidence(status, read, sample),
            }
        }
        TableInput::Unavailable {
            reason,
            read,
            sample,
        } => TableHour {
            prepared: None,
            evidence: evidence(
                Status::Unavailable {
                    reason: dictionary.intern(reason.as_bytes())?,
                },
                read,
                sample,
            ),
        },
        TableInput::NotRecorded { read, sample } => TableHour {
            prepared: None,
            evidence: evidence(Status::NotRecorded, read, sample),
        },
    })
}
