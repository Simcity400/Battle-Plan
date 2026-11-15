//! A store's table read as the row decoder's rows: each slot's fields in the wire form a row's
//! codec reads ([`field`]), a table's view at an hour ([`cells_view`]) and a frame of tables
//! ([`stored_frame`]).

use crate::{
    BranchId, Hour,
    rows::{Field, List, Opaque, TableId, TableRows},
    scope::CaptureMode,
};
use replay_columnar::{
    manifest::Status,
    state::Kind,
    store::{TableView, Value},
};
use std::io;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// `slot`'s cell in `column` of the recorder's store table `table` in the wire form a row's
/// field codec reads: a text cell's string bytes, a list as its items' bytes, an opaque cell as
/// its payload encoded at `format`.
pub fn field<'a>(
    table: &TableView<'a>,
    slot: u32,
    column: usize,
    format: u16,
) -> io::Result<Field<'a>> {
    let kind = *table
        .kinds()
        .get(column)
        .ok_or_else(|| invalid(format!("No column {column}")))?;
    let value = table.value(column, slot);
    let number = |value: Value<'_>| match value {
        Value::Number(bits) => Ok(Some(bits)),
        Value::Missing => Ok(None),
        other => Err(invalid(format!("A {other:?} cell in a {kind:?} column"))),
    };
    if value == Value::Unrecorded {
        return Ok(Field::NotRecorded);
    }
    Ok(match kind {
        Kind::U8 => Field::U8(number(value)?.unwrap_or(0) as u8),
        Kind::I8 => Field::I8(number(value)?.unwrap_or(0) as i8),
        Kind::U16 => Field::U16(number(value)?.unwrap_or(0) as u16),
        Kind::I16 => Field::I16(number(value)?.unwrap_or(0) as i16),
        Kind::U32 => Field::U32(number(value)?.unwrap_or(0) as u32),
        Kind::I32 => Field::I32(number(value)?.unwrap_or(0) as i32),
        Kind::U64 => Field::U64(number(value)?.unwrap_or(0)),
        Kind::I64 => Field::I64(number(value)?.unwrap_or(0) as i64),
        Kind::Bool => Field::Bool(number(value)?.unwrap_or(0) != 0),
        Kind::OptionalI32 => Field::OptionalI32(number(value)?.map(|bits| bits as i32)),
        Kind::OptionalU32 => Field::OptionalU32(number(value)?.map(|bits| bits as u32)),
        Kind::OptionalI64 => Field::OptionalI64(number(value)?.map(|bits| bits as i64)),
        Kind::OptionalU64 => Field::OptionalU64(number(value)?),
        Kind::Bytes => match value {
            Value::Text(id) => Field::Bytes(
                table
                    .string(id)
                    .ok_or_else(|| invalid("A text cell names a string the store does not hold"))?,
            ),
            other => return Err(invalid(format!("A {other:?} cell in a text column"))),
        },
        Kind::U32List | Kind::U64List | Kind::Opaque => {
            let Value::List(bytes) = value else {
                return Err(invalid(format!("A {value:?} cell in a {kind:?} column")));
            };
            match kind {
                Kind::U32List => Field::U32List(List::Encoded(bytes)),
                Kind::U64List => Field::U64List(List::Encoded(bytes)),
                _ => Field::Opaque(Opaque::Encoded(bytes, format)),
            }
        }
    })
}

/// Table `id`'s decoded view of `table`'s cells at an hour of `status`: a complete table's rows,
/// or a truncated one's `read` rows, in key order with every field; an unavailable one's reason
/// as its text.
pub fn cells_view(
    id: TableId,
    table: &TableView<'_>,
    status: Status,
    read: Option<&[u32]>,
    mode: CaptureMode,
    format: u16,
) -> io::Result<super::TableView> {
    let mut keyed = engine_allocation::vec(0)?;
    let mut keep = |slot: u32| -> io::Result<()> {
        let key = table
            .key(slot)
            .ok_or_else(|| invalid(format!("{id:?} slot {slot} holds no row")))?;
        engine_allocation::reserve(&mut keyed, 1)?;
        keyed.push((key, slot));
        Ok(())
    };
    match status {
        Status::Complete => table.rows().try_for_each(&mut keep)?,
        Status::Truncated { .. } => read.unwrap_or_default().iter().try_for_each(|&slot| keep(slot))?,
        _ => {}
    }
    keyed.sort_unstable();
    let rows = || cells_rows(id, table, &keyed, format);
    Ok(match status {
        Status::NotRecorded => super::TableView::NotRecorded { mode },
        Status::Complete => super::TableView::Complete(rows()?),
        Status::Truncated { missing } => super::TableView::Truncated { rows: rows()?, missing },
        Status::Unavailable { reason } => super::TableView::Unavailable {
            reason: engine_allocation::string(
                table
                    .string(reason)
                    .and_then(|reason| std::str::from_utf8(reason).ok())
                    .unwrap_or("A table's reason is not text"),
            )?,
        },
    })
}

/// Table `id`'s rows of `table`'s cells at `keyed` (each row's key and slot, ascending by key),
/// in that order with every field, each as [`cells_view`] builds it: a cell not recorded in a
/// field its row cannot leave unrecorded refused by name ([`Unsayable`]).
pub fn cells_rows(
    id: TableId,
    table: &TableView<'_>,
    keyed: &[(u128, u32)],
    format: u16,
) -> io::Result<TableRows> {
    cells_rows_of(id, table, keyed, format, id.full_mask())
}

/// [`cells_rows`] holding only the fields `fields` names, every other its row's default and never
/// decoded: for cells read in some columns only, whose others hold no recorded value.
pub fn cells_rows_of(
    id: TableId,
    table: &TableView<'_>,
    keyed: &[(u128, u32)],
    format: u16,
    fields: u64,
) -> io::Result<TableRows> {
    let keys = engine_allocation::collect_vec(keyed.iter().map(|&(key, _)| key))?;
    let absent = std::cell::Cell::new(None);
    id.rows_from_wire_fields(&keys, fields & id.full_mask(), &mut |at, column| {
        let field = field(table, keyed[at].1, column, format)?;
        absent.set(matches!(field, Field::NotRecorded).then_some((keyed[at], column)));
        Ok(field)
    })
    .map_err(|error| match absent.get() {
        Some(((key, slot), column)) => refusal(id, table, key, slot, column, format, error),
        None => error,
    })
}

/// `error`, which building the row keyed `key` (in `slot`) met just after its `column` was
/// handed as not recorded, named for its table and field when that field cannot hold the
/// absence: the row is built again with that field alone, absent.
fn refusal(
    id: TableId,
    table: &TableView<'_>,
    key: u128,
    slot: u32,
    column: usize,
    format: u16,
    error: io::Error,
) -> io::Error {
    let alone = 1_u64.checked_shl(column as u32).unwrap_or(0);
    let holds = id
        .rows_from_wire_fields(&[key], alone, &mut |_, at| {
            if at == column {
                Ok(Field::NotRecorded)
            } else {
                field(table, slot, at, format)
            }
        })
        .is_ok();
    if holds {
        return error;
    }
    let field = id.field_name(column).map_or_else(|| format!("field {column}"), str::to_owned);
    io::Error::new(io::ErrorKind::InvalidData, Unsayable { table: id, field })
}

/// [`cells_view`]'s refusal of a table holding a cell not recorded in a field its row cannot
/// leave unrecorded: the table and the field.
#[derive(Debug)]
pub struct Unsayable {
    pub table: TableId,
    pub field: String,
}

impl Unsayable {
    /// The refusal `error` carries, when it is one.
    pub fn of(error: &io::Error) -> Option<&Self> {
        error.get_ref().and_then(|inner| inner.downcast_ref::<Self>())
    }
}

impl std::fmt::Display for Unsayable {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "{:?}.{} is not recorded and its row cannot say so", self.table, self.field)
    }
}

impl std::error::Error for Unsayable {}

/// A decoded frame of `tables` at `(branch, hour)`, read from the recorder's store as a columnar
/// walk's frame of the same tables reads them ([`ColumnView::to_table_view`]): each table's rows
/// in key order with every field, a truncated table's those its hour read, an unavailable one's
/// reason as its text; every other table not selected.
pub fn stored_frame<'a>(
    tables: impl IntoIterator<Item = (TableId, TableView<'a>)>,
    (branch, hour): (BranchId, Hour),
    mode: CaptureMode,
    format: u16,
) -> io::Result<super::DecodedFrame> {
    let mut views: [super::SharedView; replay_data::rows::TABLE_COUNT] =
        std::array::from_fn(|_| super::not_selected());
    for (id, table) in tables {
        views[id.index()] = cells_view(id, &table, table.status(), table.view(), mode, format)?.into();
    }
    Ok(super::DecodedFrame {
        branch,
        hour,
        tables: views,
        metrics: super::Metrics::default(),
    })
}
