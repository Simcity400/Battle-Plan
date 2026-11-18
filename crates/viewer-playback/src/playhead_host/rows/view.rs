//! One table's view of rows made from a whole-table mirror with the table's own row codec: every
//! live row, ascending by key, each field as the recording's wire form hands it, and the status as
//! today's frame reader makes it.

use crate::{layers::List as ListColumn, playhead_host::mirror::{Cell, Mirror}};
use replay_columnar::{manifest::Status, state::Kind};
use replay_data::{
    FORMAT_VERSION,
    rows::{Field, List, Opaque, RowChanges, TableId},
};
use replay_frame::frame::TableView;
use std::io;

/// What making a view did besides the view.
#[derive(Clone, Copy, Debug, Default)]
pub struct Made {
    /// Rows made: every row of a view made whole, those made again in a patch.
    pub rows: usize,
    /// Text cells whose string the playhead never named since the table's last whole restore:
    /// made "not recorded", never guessed.
    pub untold: u64,
}

/// `table`'s view from `mirror`, which holds the columns of `kinds` that `declared` names (a
/// column it does not name reads as its kind's zero); `gap` during a gap in the recording. The status's words (an unavailable table's reason, the capture mode a table is not
/// recorded in) are the mirror's.
pub fn make(table: TableId, mirror: &Mirror, held: (&[Kind], u64), gap: bool, order: &mut Vec<(u128, u32)>) -> (TableView, Made) {
    make_of(table, mirror, held, gap, |_| true, order)
}

/// [`make`] of the rows whose keys `keep` names alone (rows at the head by key); `order` is left
/// holding their keys and slots, ascending.
pub fn make_of(table: TableId, mirror: &Mirror, (kinds, declared): (&[Kind], u64), gap: bool, keep: impl Fn(u128) -> bool, order: &mut Vec<(u128, u32)>) -> (TableView, Made) {
    let mut made = Made::default();
    let status = if gap { Status::NotRecorded } else { mirror.status().unwrap_or(Status::NotRecorded) };
    let view = match status {
        Status::NotRecorded => match mirror.capture() {
            Some(mode) => TableView::NotRecorded { mode },
            None => TableView::Unavailable { reason: format!("{table:?} is not recorded here, in a recording whose capture mode the playhead was not given") },
        },
        Status::Unavailable { .. } => TableView::Unavailable { reason: mirror.reason().unwrap_or("unavailable for a reason the mirror does not hold").to_owned() },
        Status::Complete | Status::Truncated { .. } => match rows(table, mirror, (kinds, declared), &keep, order, &mut made) {
            Ok(rows) => match status {
                Status::Truncated { missing } => TableView::Truncated { rows, missing },
                _ => TableView::Complete(rows),
            },
            Err(error) => TableView::Unavailable { reason: format!("{table:?}'s rows at the head could not be made: {error}") },
        },
    };
    (view, made)
}

fn rows(table: TableId, mirror: &Mirror, (kinds, declared): (&[Kind], u64), keep: &impl Fn(u128) -> bool, order: &mut Vec<(u128, u32)>, made: &mut Made) -> io::Result<replay_data::rows::TableRows> {
    order.clear();
    for slot in mirror.rows() {
        let key = mirror.key(slot).ok_or_else(|| io::Error::other(format!("{table:?} is held without keys")))?;
        if keep(key) {
            order.push((key, slot));
        }
    }
    order.sort_unstable();
    made.rows = order.len();
    let keys: Vec<u128> = order.iter().map(|&(key, _)| key).collect();
    let order = &*order;
    let untold = &mut made.untold;
    table.rows_from_wire(&keys, &mut |row, column| wire(table, mirror, (kinds, declared), order[row].1, column, untold))
}

/// The columns of a row a step inserted or deleted, or wrote past the 64th column: made whole.
pub const EVERY: u64 = u64::MAX;

/// `previous`, the rows [`make`] or this made from `mirror` before the steps that touched the rows
/// of `touched` (ascending by key, each with the columns written, [`EVERY`] for a row inserted or
/// deleted), with those rows alone made again: what [`make`] gives now. Only rows are patched, and
/// only while the steps left the table's status alone; `made.rows` counts the rows made again.
pub fn patch(table: TableId, mirror: &Mirror, (kinds, declared): (&[Kind], u64), previous: TableView, touched: &[(u128, u64)], made: &mut Made) -> io::Result<TableView> {
    let (mut rows, missing) = match previous {
        TableView::Complete(rows) => (rows, None),
        TableView::Truncated { rows, missing } => (rows, Some(missing)),
        _ => return Err(io::Error::other(format!("{table:?} holds no rows to patch"))),
    };
    let (mut written, mut whole, mut deleted) = (Vec::new(), Vec::new(), Vec::new());
    for &(key, columns) in touched {
        match mirror.slot(key) {
            Some(slot) if columns != EVERY && rows.contains_key(key) => written.push((key, columns, slot)),
            Some(slot) => whole.push((key, slot)),
            None if rows.contains_key(key) => deleted.push(key),
            None => {}
        }
    }
    made.rows = written.len() + whole.len();
    let untold = &mut made.untold;
    if !written.is_empty() {
        let patches: Vec<(u128, u64)> = written.iter().map(|&(key, columns, _)| (key, columns)).collect();
        rows.patch_from_wire(&patches, &mut |at, column| wire(table, mirror, (kinds, declared), written[at].2, column, untold))?;
    }
    if !whole.is_empty() || !deleted.is_empty() {
        let keys: Vec<u128> = whole.iter().map(|&(key, _)| key).collect();
        let fresh = table.rows_from_wire(&keys, &mut |at, column| wire(table, mirror, (kinds, declared), whole[at].1, column, untold))?;
        rows = rows.apply_changes(RowChanges::new(fresh, deleted))?;
    }
    Ok(match missing {
        Some(missing) => TableView::Truncated { rows, missing },
        None => TableView::Complete(rows),
    })
}

/// One cell of `slot` as the row codec reads it; a column no reader named is its kind's zero.
fn wire<'m>(table: TableId, mirror: &'m Mirror, (kinds, declared): (&[Kind], u64), slot: u32, column: usize, untold: &mut u64) -> io::Result<Field<'m>> {
    let kind = *kinds.get(column).ok_or_else(|| io::Error::other(format!("{table:?} has no column {column}")))?;
    if column < 64 && declared & (1 << column) == 0 {
        return Ok(zero(kind));
    }
    Ok(field(table, mirror, kind, column, slot, untold))
}

/// One cell as the row codec reads it from the wire.
pub fn field<'m>(table: TableId, mirror: &'m Mirror, kind: Kind, column: usize, slot: u32, untold: &mut u64) -> Field<'m> {
    match kind {
        Kind::U32List | Kind::U64List | Kind::Opaque => match mirror.list(ListColumn { table, column }, slot) {
            Cell::Value(bytes) => match kind {
                Kind::U32List => Field::U32List(List::Encoded(bytes)),
                Kind::U64List => Field::U64List(List::Encoded(bytes)),
                _ => Field::Opaque(Opaque::Encoded(bytes, FORMAT_VERSION)),
            },
            Cell::Missing | Cell::Unrecorded => Field::NotRecorded,
        },
        Kind::Bytes => match mirror.bits(column, slot) {
            Cell::Value(id) => match mirror.string(id as u32) {
                Some(text) => Field::Bytes(text),
                None => {
                    *untold += 1;
                    Field::NotRecorded
                }
            },
            Cell::Missing | Cell::Unrecorded => Field::NotRecorded,
        },
        _ => match mirror.bits(column, slot) {
            Cell::Value(bits) => number(kind, bits),
            Cell::Missing => match kind {
                Kind::OptionalI32 => Field::OptionalI32(None),
                Kind::OptionalU32 => Field::OptionalU32(None),
                Kind::OptionalI64 => Field::OptionalI64(None),
                Kind::OptionalU64 => Field::OptionalU64(None),
                _ => Field::NotRecorded,
            },
            Cell::Unrecorded => Field::NotRecorded,
        },
    }
}

/// A column no reader named, as its kind's zero: never read by a reader that names its columns.
fn zero(kind: Kind) -> Field<'static> {
    match kind {
        Kind::Bytes => Field::Bytes(&[]),
        Kind::U32List => Field::U32List(List::Native(&[])),
        Kind::U64List => Field::U64List(List::Native(&[])),
        Kind::Opaque => Field::NotRecorded,
        Kind::OptionalI32 => Field::OptionalI32(None),
        Kind::OptionalU32 => Field::OptionalU32(None),
        Kind::OptionalI64 => Field::OptionalI64(None),
        Kind::OptionalU64 => Field::OptionalU64(None),
        number => self::number(number, 0),
    }
}

fn number(kind: Kind, bits: u64) -> Field<'static> {
    match kind {
        Kind::U8 => Field::U8(bits as u8),
        Kind::I8 => Field::I8(bits as u8 as i8),
        Kind::Bool => Field::Bool(bits != 0),
        Kind::U16 => Field::U16(bits as u16),
        Kind::I16 => Field::I16(bits as u16 as i16),
        Kind::U32 => Field::U32(bits as u32),
        Kind::I32 => Field::I32(bits as u32 as i32),
        Kind::U64 => Field::U64(bits),
        Kind::I64 => Field::I64(bits as i64),
        Kind::OptionalI32 => Field::OptionalI32(Some(bits as u32 as i32)),
        Kind::OptionalU32 => Field::OptionalU32(Some(bits as u32)),
        Kind::OptionalI64 => Field::OptionalI64(Some(bits as i64)),
        Kind::OptionalU64 => Field::OptionalU64(Some(bits)),
        Kind::Bytes | Kind::U32List | Kind::U64List | Kind::Opaque => Field::NotRecorded,
    }
}
