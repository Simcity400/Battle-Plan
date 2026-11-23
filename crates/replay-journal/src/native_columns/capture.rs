
use replay_columnar::{
    manifest::Status,
    state::{Cell, Kind as Column},
    writer::{
        Batch, Coverage, Evidence, Hour, Input, Prepared, Sample, Strings, Sweep, TableWriter,
    },
};
use replay_data::{
    codec::invalid_static,
    rows::{Field, List, Opaque, RowCodec},
};
use std::io;

#[cfg(test)]
use replay_recording::column_kind::columns;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

const MAX_FIELDS: usize = 64;

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Staging {
    #[default]
    Columns,
    Rows,
}

#[derive(Default)]
pub struct RowCells {
    staging: Staging,
    batch: Batch,
    entries: Vec<Entry>,
    bytes: Vec<u8>,
    u32s: Vec<u32>,
    u64s: Vec<u64>,
}

impl RowCells {
    pub fn new(staging: Staging) -> Self {
        Self {
            staging,
            ..Self::default()
        }
    }

    pub fn staging(&self) -> Staging {
        self.staging
    }

    fn batch_row<R: RowCodec>(&mut self, row: &R, kept: u64) -> io::Result<()> {
        let batch = &mut self.batch;
        batch.key(row.key())?;
        if kept == 0 {
            return row.visit_wire(&mut |column, field| put(batch, column, field));
        }
        row.visit_wire(&mut |column, field| {
            if column < MAX_FIELDS && kept >> column & 1 != 0 {
                batch.kept(column)
            } else {
                put(batch, column, field)
            }
        })
    }

    fn take(&mut self, field: Field<'_>) -> io::Result<Entry> {
        Ok(match field {
            Field::Empty => return Err(invalid_static("A row field is missing")),
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

    pub fn row<R: RowCodec>(&mut self, hour: &mut Hour<'_>, row: &R, kept: u64) -> io::Result<()> {
        self.entries.clear();
        self.bytes.clear();
        self.u32s.clear();
        self.u64s.clear();
        row.visit_wire(&mut |index, field| {
            if index != self.entries.len() || index >= MAX_FIELDS {
                return Err(invalid_static(
                    "Row fields are not visited once each in schema order",
                ));
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
        for (column, input) in inputs.iter_mut().enumerate().take(self.entries.len()) {
            if kept >> column & 1 != 0 {
                *input = Input::Kept;
            }
        }
        hour.row(row.key(), &inputs[..self.entries.len()])
    }
}

fn put(batch: &mut Batch, column: usize, field: Field<'_>) -> io::Result<()> {
    match field {
        Field::Empty => Err(invalid_static("A row field is missing")),
        Field::NotRecorded => batch.unrecorded(column),
        Field::U8(value) => batch.value(column, Cell::U8(value)),
        Field::I8(value) => batch.value(column, Cell::I8(value)),
        Field::U16(value) => batch.value(column, Cell::U16(value)),
        Field::I16(value) => batch.value(column, Cell::I16(value)),
        Field::U32(value) => batch.value(column, Cell::U32(value)),
        Field::I32(value) => batch.value(column, Cell::I32(value)),
        Field::U64(value) => batch.value(column, Cell::U64(value)),
        Field::I64(value) => batch.value(column, Cell::I64(value)),
        Field::Bool(value) => batch.value(column, Cell::Bool(value)),
        Field::OptionalI32(value) => batch.value(column, Cell::OptionalI32(value)),
        Field::OptionalU32(value) => batch.value(column, Cell::OptionalU32(value)),
        Field::OptionalI64(value) => batch.value(column, Cell::OptionalI64(value)),
        Field::OptionalU64(value) => batch.value(column, Cell::OptionalU64(value)),
        Field::Bytes(bytes) => batch.text(column, bytes),
        Field::Opaque(Opaque::Native(value)) => {
            batch.opaque_with(column, |bytes| value.encode_value(bytes))
        }
        Field::Opaque(Opaque::Encoded(bytes, _)) => batch.value(column, Cell::Opaque(bytes)),
        Field::U32List(List::Native(values)) => batch.value(column, Cell::U32List(values)),
        Field::U32List(List::Encoded(bytes)) => batch.u32s_with(column, |items| {
            engine_allocation::reserve(items, bytes.len() / 4)?;
            items.extend(
                bytes
                    .chunks_exact(4)
                    .map(|item| u32::from_le_bytes(item.try_into().expect("4 bytes"))),
            );
            Ok(())
        }),
        Field::U64List(List::Native(values)) => batch.value(column, Cell::U64List(values)),
        Field::U64List(List::Encoded(bytes)) => batch.u64s_with(column, |items| {
            engine_allocation::reserve(items, bytes.len() / 8)?;
            items.extend(
                bytes
                    .chunks_exact(8)
                    .map(|item| u64::from_le_bytes(item.try_into().expect("8 bytes"))),
            );
            Ok(())
        }),
    }
}

pub enum TableInput<'a, R> {
    Same { read: bool, sample: Sample },
    Changes {
        rows: &'a [R],
        removed: &'a [u128],
        read: bool,
        sample: Sample,
    },
    Keyed {
        rows: &'a [R],
        keys: &'a [u128],
        removed: &'a [u128],
        read: bool,
        sample: Sample,
    },
    Paired {
        rows: &'a [R],
        previous: &'a [R],
        pairs: &'a [(u32, Option<u32>)],
        removed: &'a [u128],
        read: bool,
        sample: Sample,
    },
    Aligned {
        rows: &'a [R],
        previous: &'a [R],
        read: bool,
        sample: Sample,
    },
    Snapshot {
        rows: &'a [R],
        partial: Option<u32>,
        read: bool,
        sample: Sample,
    },
    Unavailable { reason: &'a str, read: bool, sample: Sample },
    NotRecorded { read: bool, sample: Sample },
}

pub struct TableHour {
    pub prepared: Option<Prepared>,
    pub evidence: Evidence,
}

fn visit<R: RowCodec>(
    writer: &mut TableWriter,
    dictionary: Strings<'_>,
    cells: &mut RowCells,
    coverage: Coverage<'_>,
    rows: &[R],
) -> io::Result<Prepared> {
    stage(writer, dictionary, cells, coverage, rows.len(), rows.iter().map(|row| Ok((row, None))))
}

fn visit_keyed<R: RowCodec>(
    writer: &mut TableWriter,
    dictionary: Strings<'_>,
    cells: &mut RowCells,
    rows: &[R],
    keys: &[u128],
    removed: &[u128],
) -> io::Result<Prepared> {
    let changed = keys.iter().map(|&key| {
        rows.binary_search_by_key(&key, R::key)
            .map(|at| (&rows[at], None))
            .map_err(|_| invalid_static("A changed key names no row"))
    });
    stage(writer, dictionary, cells, Coverage::Changes { removed }, keys.len(), changed)
}

fn stage<'r, R: RowCodec + 'r>(
    writer: &mut TableWriter,
    dictionary: Strings<'_>,
    cells: &mut RowCells,
    coverage: Coverage<'_>,
    count: usize,
    rows: impl Iterator<Item = io::Result<(&'r R, Option<&'r R>)>>,
) -> io::Result<Prepared> {
    let heavy = heavy(writer.kinds());
    let kept = |row: &R, previous: Option<&R>| -> io::Result<u64> {
        match previous {
            Some(previous) if previous.key() != row.key() => {
                Err(invalid_static("A row's previous row has another key"))
            }
            Some(previous) if heavy != 0 => Ok(!row.differing_fields(previous) & heavy),
            _ => Ok(0),
        }
    };
    match cells.staging {
        Staging::Columns => {
            cells.batch.begin(writer.kinds(), count)?;
            for row in rows {
                let (row, previous) = row?;
                cells.batch_row(row, kept(row, previous)?)?;
            }
            writer.prepare(dictionary, coverage, &cells.batch)
        }
        Staging::Rows => {
            let mut hour = writer.begin(dictionary, coverage)?;
            for row in rows {
                let (row, previous) = row?;
                cells.row(&mut hour, row, kept(row, previous)?)?;
            }
            hour.finish()
        }
    }
}

fn heavy(kinds: &[Column]) -> u64 {
    kinds
        .iter()
        .take(MAX_FIELDS)
        .enumerate()
        .filter(|(_, kind)| {
            matches!(kind, Column::Bytes | Column::U32List | Column::U64List | Column::Opaque)
        })
        .fold(0, |mask, (column, _)| mask | 1 << column)
}

pub fn table_hour<'d, R: RowCodec>(
    writer: &mut TableWriter,
    dictionary: impl Into<Strings<'d>>,
    cells: &mut RowCells,
    input: TableInput<'_, R>,
) -> io::Result<TableHour> {
    let mut dictionary = dictionary.into();
    let evidence = |status, read, sample| Evidence {
        status,
        read,
        sample,
    };
    let unchanged = writer.sweep() == Sweep::Touched
        && match &input {
            TableInput::Changes { rows, removed, .. } => rows.is_empty() && removed.is_empty(),
            TableInput::Keyed { keys, removed, .. } => keys.is_empty() && removed.is_empty(),
            TableInput::Paired { pairs, removed, .. } => pairs.is_empty() && removed.is_empty(),
            _ => false,
        };
    Ok(match input {
        TableInput::Same { read, sample } => TableHour {
            prepared: None,
            evidence: evidence(Status::Complete, read, sample),
        },
        TableInput::Changes { read, sample, .. }
        | TableInput::Keyed { read, sample, .. }
        | TableInput::Paired { read, sample, .. }
            if unchanged =>
        {
            TableHour {
                prepared: None,
                evidence: evidence(Status::Complete, read, sample),
            }
        }
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
        TableInput::Keyed {
            rows,
            keys,
            removed,
            read,
            sample,
        } => TableHour {
            prepared: Some(visit_keyed(writer, dictionary, cells, rows, keys, removed)?),
            evidence: evidence(Status::Complete, read, sample),
        },
        TableInput::Paired {
            rows,
            previous,
            pairs,
            removed,
            read,
            sample,
        } => {
            let changed = pairs.iter().map(|&(at, before)| {
                let row = rows
                    .get(at as usize)
                    .ok_or_else(|| invalid_static("A changed row is past the table"))?;
                let previous = match before {
                    Some(before) => Some(
                        previous
                            .get(before as usize)
                            .ok_or_else(|| invalid_static("A previous row is past the table"))?,
                    ),
                    None => None,
                };
                Ok((row, previous))
            });
            TableHour {
                prepared: Some(stage(
                    writer,
                    dictionary,
                    cells,
                    Coverage::Changes { removed },
                    pairs.len(),
                    changed,
                )?),
                evidence: evidence(Status::Complete, read, sample),
            }
        }
        TableInput::Aligned {
            rows,
            previous,
            read,
            sample,
        } => {
            if rows.len() != previous.len() {
                return Err(invalid_static(
                    "Aligned rows differ in count from their previous rows",
                ));
            }
            let aligned = rows
                .iter()
                .zip(previous)
                .map(|(row, previous)| Ok((row, Some(previous))));
            TableHour {
                prepared: Some(stage(
                    writer,
                    dictionary,
                    cells,
                    Coverage::Complete,
                    rows.len(),
                    aligned,
                )?),
                evidence: evidence(Status::Complete, read, sample),
            }
        }
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
