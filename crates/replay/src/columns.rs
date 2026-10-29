//! A walked table's fields as typed slices by slot, for operators that read columns in place
//! ([`crate::segments`]), and a slot's row built from its columns where an operator needs
//! the row itself. A table's columns are its schema's fields, in order.

use crate::rows::{RowCodec, TableId};
pub use replay_columnar::manifest::Status;
use replay_columnar::{
    section::Changes,
    state::{Bits, Column, Table},
    walk::State,
};
use std::io;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// The column `table`'s field `name` is held in.
pub fn field(table: TableId, name: &str) -> io::Result<usize> {
    (0..table.field_count())
        .find(|&field| table.field_name(field) == Some(name))
        .ok_or_else(|| invalid(format!("{table:?} has no field {name}")))
}

/// `table`'s walked state.
pub fn table(state: &State, table: TableId) -> io::Result<&Table> {
    state
        .table(table.index())
        .ok_or_else(|| invalid(format!("The walker holds no {table:?}")))
}

/// `table`'s status this hour: what it held, how many rows a partial read missed, or why it
/// was unavailable.
pub fn status(state: &State, table: TableId) -> Status {
    state.status(table.index()).unwrap_or(Status::NotRecorded)
}

/// Whether `table` holds every live row this hour.
pub fn complete(state: &State, table: TableId) -> bool {
    state.status(table.index()) == Some(Status::Complete)
}

/// Whether `table` is unavailable this hour, for a recorded reason.
pub fn unavailable(state: &State, table: TableId) -> bool {
    matches!(
        state.status(table.index()),
        Some(Status::Unavailable { .. })
    )
}

/// The slots a truncated `table` read this hour, ascending; None when it is not truncated.
pub fn truncated(state: &State, table: TableId) -> Option<&[u32]> {
    matches!(state.status(table.index()), Some(Status::Truncated { .. }))
        .then(|| state.truncated_view(table.index()))
        .flatten()
}

/// What the hour changed in `table`.
pub fn changes(state: &State, table: TableId) -> io::Result<&Changes> {
    state
        .changes(table.index())
        .ok_or_else(|| invalid(format!("The walker holds no {table:?}")))
}

/// A field kind held as one value per slot.
pub trait Plain: Sized {
    fn values(column: &Column) -> Option<&[Self]>;
}

macro_rules! plain {
    ($($type:ty => $variant:ident),* $(,)?) => {
        $(impl Plain for $type {
            fn values(column: &Column) -> Option<&[Self]> {
                match column {
                    Column::$variant(values) => Some(values),
                    _ => None,
                }
            }
        })*
    };
}

plain!(
    u8 => U8,
    i8 => I8,
    u16 => U16,
    i16 => I16,
    u32 => U32,
    i32 => I32,
    u64 => U64,
    i64 => I64,
    bool => Bool,
);

/// A field kind held as a value and a presence bit per slot: a slot without its bit holds
/// no value, never zero.
pub trait Optional: Sized {
    fn values(column: &Column) -> Option<(&[Self], &Bits)>;
}

macro_rules! optional {
    ($($type:ty => $variant:ident),* $(,)?) => {
        $(impl Optional for $type {
            fn values(column: &Column) -> Option<(&[Self], &Bits)> {
                match column {
                    Column::$variant(values, present) => Some((values, present)),
                    _ => None,
                }
            }
        })*
    };
}

optional!(
    i32 => OptionalI32,
    u32 => OptionalU32,
    i64 => OptionalI64,
    u64 => OptionalU64,
);

/// `field`'s values by slot. A slot past the end has never been written.
pub fn plain<T: Plain>(table: &Table, field: usize) -> io::Result<&[T]> {
    table
        .column(field)
        .and_then(T::values)
        .ok_or_else(|| invalid(format!("column {field} is not of the asked kind")))
}

/// `field`'s values and presence by slot.
pub fn optional<T: Optional>(table: &Table, field: usize) -> io::Result<(&[T], &Bits)> {
    table
        .column(field)
        .and_then(T::values)
        .ok_or_else(|| invalid(format!("column {field} is not of the asked kind")))
}

/// `field`'s dictionary ids by slot: a text field's strings are the walker's dictionary's.
pub fn text(table: &Table, field: usize) -> io::Result<&[u32]> {
    match table.column(field) {
        Some(Column::Bytes(ids)) => Ok(ids),
        _ => Err(invalid(format!("column {field} is not text"))),
    }
}

/// The row `table`'s live `slot` holds, built from its columns as the row decoder reads it;
/// opaque cells keep the recording's encoding at `format`.
pub fn row<R: RowCodec>(state: &State, table: TableId, slot: u32, format: u16) -> io::Result<R> {
    let held = self::table(state, table)?;
    let key = held
        .key(slot)
        .ok_or_else(|| invalid(format!("{table:?} slot {slot} holds no row")))?;
    R::from_wire(key, &mut |column| {
        crate::columnar_frames::wire(held, state.dictionary(), slot, column, format)
    })
}
