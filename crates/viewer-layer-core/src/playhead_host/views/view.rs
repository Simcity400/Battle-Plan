
use crate::playhead_host::mirror::{Cell, Mirror, Resolved};
use replay_columnar::{manifest::Status, state::Kind};
use replay_data::{rows::{Field, List, Opaque, RowChanges, TableId, TableRows}, scope::CaptureMode};
use replay_frame::frame::TableView;
use std::io;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeadFailureKind {
    BuildError(String),
    Panicked(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadFailure {
    pub table: TableId,
    pub kind: HeadFailureKind,
}

impl HeadFailure {
    pub fn format(&self) -> String {
        match &self.kind {
            HeadFailureKind::BuildError(error) => {
                format!("{:?}'s rows at the head could not be made: {error}", self.table)
            }
            HeadFailureKind::Panicked(detail) => {
                format!("{:?}'s rows at the head could not be made ({detail})", self.table)
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Made {
    pub rows: usize,
    pub untold: u64,
    pub failure: Option<HeadFailureKind>,
}

pub fn make(table: TableId, mirror: &Mirror, held: (&[Kind], u64), gap: bool, order: &mut Vec<(u128, u32)>) -> (TableView, Made) {
    make_of(table, mirror, held, gap, |_| true, order)
}

pub fn make_of(table: TableId, mirror: &Mirror, (kinds, declared): (&[Kind], u64), gap: bool, keep: impl Fn(u128) -> bool, order: &mut Vec<(u128, u32)>) -> (TableView, Made) {
    let mut made = Made::default();
    let status = if gap { Status::NotRecorded } else { mirror.status().unwrap_or(Status::NotRecorded) };
    let (view, failure) = make_rows(table, status, mirror.capture(), |_| mirror.reason().unwrap_or("unavailable for a reason the mirror does not hold").to_owned(), || rows(table, mirror, (kinds, declared), &keep, order, &mut made));
    made.failure = failure.map(|failure| failure.kind);
    (view, made)
}

pub(crate) fn make_rows(table: TableId, status: Status, capture: Option<CaptureMode>, reason: impl FnOnce(u32) -> String, rows: impl FnOnce() -> io::Result<TableRows>) -> (TableView, Option<HeadFailure>) {
    let view = match status {
        Status::NotRecorded => match capture {
            Some(mode) => TableView::NotRecorded { mode },
            None => TableView::Unavailable { reason: format!("{table:?} is not recorded here, in a recording whose capture mode the playhead was not given") },
        },
        Status::Unavailable { reason: id } => TableView::Unavailable { reason: reason(id) },
        Status::Complete | Status::Truncated { .. } => match rows() {
            Ok(rows) => match status {
                Status::Truncated { missing } => TableView::Truncated { rows, missing },
                _ => TableView::Complete(rows),
            },
            Err(error) => return failed_rows(table, error),
        },
    };
    (view, None)
}

pub(crate) fn failed_rows(table: TableId, error: io::Error) -> (TableView, Option<HeadFailure>) {
    let failure = HeadFailure { table, kind: HeadFailureKind::BuildError(error.to_string()) };
    (TableView::Unavailable { reason: failure.format() }, Some(failure))
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
    let held = resolve(mirror, kinds, declared);
    table.rows_from_wire(&keys, &mut |row, column| wire(table, mirror, &held, order[row].1, column, untold))
}

fn resolve<'m>(mirror: &'m Mirror, kinds: &[Kind], declared: u64) -> Vec<(Kind, bool, Resolved<'m>)> {
    (0..kinds.len()).map(|column| (kinds[column], column >= 64 || declared & (1 << column) != 0, mirror.resolve(column))).collect()
}

pub const EVERY: u64 = u64::MAX;

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
    let held = resolve(mirror, kinds, declared);
    if !written.is_empty() {
        let patches: Vec<(u128, u64)> = written.iter().map(|&(key, columns, _)| (key, columns)).collect();
        rows.patch_from_wire(&patches, &mut |at, column| wire(table, mirror, &held, written[at].2, column, untold))?;
    }
    if !whole.is_empty() || !deleted.is_empty() {
        let keys: Vec<u128> = whole.iter().map(|&(key, _)| key).collect();
        let fresh = table.rows_from_wire(&keys, &mut |at, column| wire(table, mirror, &held, whole[at].1, column, untold))?;
        rows = rows.apply_changes(RowChanges::new(fresh, deleted))?;
    }
    Ok(match missing {
        Some(missing) => TableView::Truncated { rows, missing },
        None => TableView::Complete(rows),
    })
}

fn wire<'m>(table: TableId, mirror: &'m Mirror, held: &[(Kind, bool, Resolved<'m>)], slot: u32, column: usize, untold: &mut u64) -> io::Result<Field<'m>> {
    let &(kind, named, resolved) = held.get(column).ok_or_else(|| io::Error::other(format!("{table:?} has no column {column}")))?;
    if !named {
        return Ok(zero(kind));
    }
    Ok(field(mirror, kind, resolved, slot, untold))
}

fn field<'m>(mirror: &'m Mirror, kind: Kind, resolved: Resolved<'m>, slot: u32, untold: &mut u64) -> Field<'m> {
    match kind {
        Kind::U32List | Kind::U64List | Kind::Opaque => match resolved.list(slot) {
            Cell::Value(bytes) => match kind {
                Kind::U32List => Field::U32List(List::Encoded(bytes)),
                Kind::U64List => Field::U64List(List::Encoded(bytes)),
                _ => Field::Opaque(Opaque::Encoded(bytes, mirror.format())),
            },
            Cell::Missing | Cell::Unrecorded => Field::NotRecorded,
        },
        Kind::Bytes => match resolved.bits(slot) {
            Cell::Value(id) => match mirror.string(id as u32) {
                Some(text) => Field::Bytes(text),
                None => {
                    *untold += 1;
                    Field::NotRecorded
                }
            },
            Cell::Missing | Cell::Unrecorded => Field::NotRecorded,
        },
        _ => match resolved.bits(slot) {
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
