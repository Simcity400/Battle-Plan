//! A walked table's fields as typed slices by slot, for operators that read columns in place
//! ([`crate::segments`]), and a slot's row built from its columns where an operator needs
//! the row itself. A table's columns are its schema's fields, in order. A table the walk does
//! not read ([`crate::columnar_frames::Recording::select`]) has a status but no rows or
//! changes: reading them is refused.

use crate::rows::{RowCodec, TableId};
pub use replay_columnar::{
    manifest::Status,
    state::{Bools, Paged, PagedBits, Table},
};
use replay_columnar::{section::Changes, state::Column, walk::State};
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

/// Refuses `table` when the walk does not read it.
fn read(state: &State, table: TableId) -> io::Result<()> {
    if state.skips(table.index()) {
        return Err(invalid(format!("{table:?} is not read by this walk")));
    }
    Ok(())
}

/// `table`'s walked state.
pub fn table(state: &State, table: TableId) -> io::Result<&Table> {
    read(state, table)?;
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

/// The slots a truncated `table` read this hour, ascending; None when it is not truncated or
/// the walk does not read it.
pub fn truncated(state: &State, table: TableId) -> Option<&[u32]> {
    (!state.skips(table.index())
        && matches!(state.status(table.index()), Some(Status::Truncated { .. })))
    .then(|| state.truncated_view(table.index()))
    .flatten()
}

/// `table`'s last complete read and the hour of that read when known: its view when complete,
/// else, for a table every restart keeps it of ([`crate::columnar_frames::LAST_COMPLETE`]), the
/// rows it last read whole. None when no complete read is known: never read whole since the
/// run's start or the restart the walk started at.
pub fn last_complete(state: &State, table: TableId) -> io::Result<Option<(&Table, Option<i32>)>> {
    read(state, table)?;
    let index = table.index();
    if let Some(last) = state.last_complete(index) {
        return Ok(Some(last));
    }
    Ok(complete(state, table)
        .then(|| state.table(index).map(|held| (held, state.sampled_at(index))))
        .flatten())
}

/// The row `held`, one of `table`'s states the walk holds ([`last_complete`]), holds at its live
/// `slot`, as [`row`] builds it.
pub fn row_in<R: RowCodec>(
    state: &State,
    held: &Table,
    table: TableId,
    slot: u32,
    format: u16,
) -> io::Result<R> {
    read(state, table)?;
    let key = held
        .key(slot)
        .ok_or_else(|| invalid(format!("{table:?} slot {slot} holds no row")))?;
    R::from_wire(key, &mut |column| {
        crate::columnar_frames::wire(held, state.dictionary(), slot, column, format)
    })
}

/// What the hour changed in `table`.
pub fn changes(state: &State, table: TableId) -> io::Result<&Changes> {
    read(state, table)?;
    state
        .changes(table.index())
        .ok_or_else(|| invalid(format!("The walker holds no {table:?}")))
}

/// A field kind held as one value per slot, read by slot: `values.get(slot as usize)` or
/// `values[slot as usize]`. Values are held in pages ([`Paged`]), bools a bit each.
pub trait Plain: Sized + Copy {
    type Values<'a>;
    fn values(column: &Column) -> Option<Self::Values<'_>>;
    /// Slot `at`'s value of `values`; None past the slots.
    fn at(values: &Self::Values<'_>, at: usize) -> Option<Self>;
}

macro_rules! plain {
    ($($type:ty => $variant:ident),* $(,)?) => {
        $(impl Plain for $type {
            type Values<'a> = &'a Paged<$type>;
            fn values(column: &Column) -> Option<Self::Values<'_>> {
                match column {
                    Column::$variant(values) => Some(values),
                    _ => None,
                }
            }
            fn at(values: &Self::Values<'_>, at: usize) -> Option<Self> {
                values.get(at).copied()
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
);

impl Plain for bool {
    type Values<'a> = &'a Bools;
    fn values(column: &Column) -> Option<Self::Values<'_>> {
        match column {
            Column::Bool(values) => Some(values),
            _ => None,
        }
    }
    fn at(values: &Self::Values<'_>, at: usize) -> Option<Self> {
        values.get(at).copied()
    }
}

/// A field kind held as a value and a presence bit per slot: a slot without its bit holds
/// no value, never zero.
pub trait Optional: Sized {
    fn values(column: &Column) -> Option<(&Paged<Self>, &PagedBits)>;
}

macro_rules! optional {
    ($($type:ty => $variant:ident),* $(,)?) => {
        $(impl Optional for $type {
            fn values(column: &Column) -> Option<(&Paged<Self>, &PagedBits)> {
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
pub fn plain<T: Plain>(table: &Table, field: usize) -> io::Result<T::Values<'_>> {
    table
        .column(field)
        .and_then(T::values)
        .ok_or_else(|| invalid(format!("column {field} is not of the asked kind")))
}

/// Slot `slot`'s value of `field`; None past the slots.
pub fn value<T: Plain>(table: &Table, field: usize, slot: u32) -> io::Result<Option<T>> {
    Ok(T::at(&plain::<T>(table, field)?, slot as usize))
}

/// `field`'s values and presence by slot.
pub fn optional<T: Optional>(table: &Table, field: usize) -> io::Result<(&Paged<T>, &PagedBits)> {
    table
        .column(field)
        .and_then(T::values)
        .ok_or_else(|| invalid(format!("column {field} is not of the asked kind")))
}

/// `field`'s dictionary ids by slot: a text field's strings are the walker's dictionary's.
pub fn text(table: &Table, field: usize) -> io::Result<&Paged<u32>> {
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

/// The live `slots` of `table`, built from their columns and sorted by key.
pub fn rows_at(
    state: &State,
    table: TableId,
    slots: &[u32],
    format: u16,
) -> io::Result<crate::rows::TableRows> {
    let held = self::table(state, table)?;
    let mut keyed = slots
        .iter()
        .map(|&slot| {
            held.key(slot)
                .map(|key| (key, slot))
                .ok_or_else(|| invalid(format!("{table:?} slot {slot} holds no row")))
        })
        .collect::<io::Result<Vec<_>>>()?;
    keyed.sort_unstable();
    keyed.dedup();
    let keys: Vec<_> = keyed.iter().map(|&(key, _)| key).collect();
    table.rows_from_wire(&keys, &mut |row, column| {
        crate::columnar_frames::wire(held, state.dictionary(), keyed[row].1, column, format)
    })
}

/// Sets, in `rows` (`table`'s rows by key), the fields each of `patches` names from its
/// columns, as [`row`] builds them: a row's key, the live slot holding it and the fields as
/// change-mask bits, in key order.
pub fn patch(
    state: &State,
    table: TableId,
    rows: &mut crate::rows::TableRows,
    patches: &[(u128, u32, u64)],
    format: u16,
) -> io::Result<()> {
    let held = self::table(state, table)?;
    let fields: Vec<_> = patches
        .iter()
        .map(|&(key, _, fields)| (key, fields))
        .collect();
    rows.patch_from_wire(&fields, &mut |at, column| {
        crate::columnar_frames::wire(held, state.dictionary(), patches[at].1, column, format)
    })
}
