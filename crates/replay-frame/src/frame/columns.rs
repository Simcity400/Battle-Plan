
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

pub fn table_field<'a>(
    id: TableId,
    table: &TableView<'a>,
    slot: u32,
    column: usize,
    format: u16,
) -> io::Result<Field<'a>> {
    if id.column_withheld_at(column, format) {
        return Ok(Field::NotRecorded);
    }
    let read = field(table, slot, column, format)?;
    if let Field::Opaque(Opaque::Encoded(bytes, _)) = read
        && matches!(id, TableId::CountryAppearance | TableId::ReservedCountryNaming)
        && id.field_name(column) == Some("original_tag")
        && !original_tag_option_fits(bytes)
    {
        return Ok(Field::NotRecorded);
    }
    Ok(read)
}

fn original_tag_option_fits(bytes: &[u8]) -> bool {
    match bytes {
        [0] => true,
        [1, rest @ ..] => rest
            .split_first_chunk::<4>()
            .is_some_and(|(length, tag)| u32::from_le_bytes(*length) as usize == tag.len()),
        _ => false,
    }
}

pub fn field<'a>(
    table: &TableView<'a>,
    slot: u32,
    column: usize,
    format: u16,
) -> io::Result<Field<'a>> {
    let Some(&kind) = table.kinds().get(column) else {
        return Ok(Field::NotRecorded);
    };
    value_field(kind, table.value(column, slot), |id| table.string(id), format)
}

pub fn value_field<'a>(
    kind: Kind,
    value: Value<'a>,
    string: impl FnOnce(u32) -> Option<&'a [u8]>,
    format: u16,
) -> io::Result<Field<'a>> {
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
                string(id)
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

pub fn cells_rows(
    id: TableId,
    table: &TableView<'_>,
    keyed: &[(u128, u32)],
    format: u16,
) -> io::Result<TableRows> {
    cells_rows_of(id, table, keyed, format, id.full_mask())
}

pub fn cells_rows_of(
    id: TableId,
    table: &TableView<'_>,
    keyed: &[(u128, u32)],
    format: u16,
    fields: u64,
) -> io::Result<TableRows> {
    let keys = engine_allocation::collect_vec(keyed.iter().map(|&(key, _)| key))?;
    rows_of_fields(id, &keys, fields, &mut |at, column| {
        table_field(id, table, keyed[at].1, column, format)
    })
}

pub fn rows_of_fields<'a>(
    id: TableId,
    keys: &[u128],
    fields: u64,
    field: &mut dyn FnMut(usize, usize) -> io::Result<Field<'a>>,
) -> io::Result<TableRows> {
    let absent = std::cell::Cell::new(None);
    id.rows_from_wire_fields(keys, fields & id.full_mask(), &mut |at, column| {
        let held = field(at, column)?;
        absent.set(matches!(held, Field::NotRecorded).then_some((at, column)));
        Ok(held)
    })
    .map_err(|error| match absent.get() {
        Some((at, column)) => refusal(id, keys[at], column, error, &mut |other| field(at, other)),
        None => error,
    })
}

fn refusal<'a>(
    id: TableId,
    key: u128,
    column: usize,
    error: io::Error,
    field: &mut dyn FnMut(usize) -> io::Result<Field<'a>>,
) -> io::Error {
    let alone = 1_u64.checked_shl(column as u32).unwrap_or(0);
    let holds = id
        .rows_from_wire_fields(&[key], alone, &mut |_, at| {
            if at == column {
                Ok(Field::NotRecorded)
            } else {
                field(at)
            }
        })
        .is_ok();
    if holds {
        return error;
    }
    let field = id.field_name(column).map_or_else(|| format!("field {column}"), str::to_owned);
    io::Error::new(io::ErrorKind::InvalidData, Unsayable { table: id, field })
}

#[derive(Debug)]
pub struct Unsayable {
    pub table: TableId,
    pub field: String,
}

impl Unsayable {
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
