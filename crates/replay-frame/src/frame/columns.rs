//! A columnar walk's tables read where the walker holds them: the live rows a table reads, by
//! slot in key order, and each slot's fields read straight from its columns as the row decoder
//! reads them ([`Columns`]). Nothing is copied but the fields a reader asks for: an owned row
//! holds only the fields its reader names ([`Columns::row`]).

use crate::{
    BranchId, Hour,
    rows::{Field, List, NativeValue, Opaque, RowCodec, TableId, TableRows, field_from_wire},
    scope::CaptureMode,
};
use replay_columnar::{
    manifest::Status,
    section::Changes,
    state::{Cell, Table},
    walk::{Dictionary, State},
};
use std::io;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// `slot`'s cell in `column` in the wire form a row's field codec reads: a text cell's
/// dictionary bytes, an opaque cell as its payload encoded at `format`.
pub fn wire<'a>(
    table: &'a Table,
    dictionary: &'a Dictionary,
    slot: u32,
    column: usize,
    format: u16,
) -> io::Result<Field<'a>> {
    if !table.recorded(column, slot) {
        return Ok(Field::NotRecorded);
    }
    let cell = table
        .column(column)
        .and_then(|values| values.cell(slot))
        .ok_or_else(|| invalid("A live slot holds no cell"))?;
    Ok(match cell {
        Cell::U8(value) => Field::U8(value),
        Cell::I8(value) => Field::I8(value),
        Cell::U16(value) => Field::U16(value),
        Cell::I16(value) => Field::I16(value),
        Cell::U32(value) => Field::U32(value),
        Cell::I32(value) => Field::I32(value),
        Cell::U64(value) => Field::U64(value),
        Cell::I64(value) => Field::I64(value),
        Cell::Bool(value) => Field::Bool(value),
        Cell::OptionalI32(value) => Field::OptionalI32(value),
        Cell::OptionalU32(value) => Field::OptionalU32(value),
        Cell::OptionalI64(value) => Field::OptionalI64(value),
        Cell::OptionalU64(value) => Field::OptionalU64(value),
        Cell::Bytes(id) => Field::Bytes(
            dictionary
                .get(id)
                .ok_or_else(|| invalid("A text cell names a missing dictionary entry"))?,
        ),
        Cell::U32List(items) => Field::U32List(List::Native(items)),
        Cell::U64List(items) => Field::U64List(List::Native(items)),
        Cell::Opaque(bytes) => Field::Opaque(Opaque::Encoded(bytes, format)),
    })
}

/// One table of a columnar walk as a frame reads it: the walker's columns, and the live slots
/// it reads ascending by key (every live slot of a complete table, the slots a truncated table
/// read). Copying it copies three references.
#[derive(Clone, Copy)]
pub struct Columns<'a> {
    id: TableId,
    table: &'a Table,
    dictionary: &'a Dictionary,
    order: &'a [u32],
    /// `order` is every live slot: the table is complete.
    whole: bool,
    format: u16,
}

impl std::fmt::Debug for Columns<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Columns")
            .field("table", &self.id)
            .field("rows", &self.order.len())
            .finish_non_exhaustive()
    }
}

impl<'a> Columns<'a> {
    /// `table`, `id`'s walked state, read at `order`: live slots ascending by key, every one
    /// where `whole`.
    pub fn new(
        id: TableId,
        table: &'a Table,
        dictionary: &'a Dictionary,
        (order, whole): (&'a [u32], bool),
        format: u16,
    ) -> Self {
        Self {
            id,
            table,
            dictionary,
            order,
            whole,
            format,
        }
    }

    pub fn id(self) -> TableId {
        self.id
    }

    /// The walker's state of the table, every live slot of it.
    pub fn table(self) -> &'a Table {
        self.table
    }

    pub fn dictionary(self) -> &'a Dictionary {
        self.dictionary
    }

    /// The recording's format, which opaque cells are encoded at.
    pub fn format(self) -> u16 {
        self.format
    }

    pub fn len(self) -> usize {
        self.order.len()
    }

    pub fn is_empty(self) -> bool {
        self.order.is_empty()
    }

    /// Whether it reads every live slot.
    pub fn is_whole(self) -> bool {
        self.whole
    }

    /// The slots it reads, ascending by key.
    pub fn slots(self) -> &'a [u32] {
        self.order
    }

    /// The slot at position `at` in key order.
    pub fn slot(self, at: usize) -> Option<u32> {
        self.order.get(at).copied()
    }

    /// `slot`'s key; 0 for a slot holding no row, which no position names.
    pub fn key(self, slot: u32) -> u128 {
        self.table.key(slot).unwrap_or(0)
    }

    /// The position of the row keyed `key` among those it reads, or where it would be.
    pub fn position(self, key: u128) -> Result<usize, usize> {
        self.order
            .binary_search_by_key(&key, |&slot| self.key(slot))
    }

    /// The rows it reads keyed from `low` through `high`.
    pub fn span(self, low: u128, high: u128) -> Self {
        let start = self.order.partition_point(|&slot| self.key(slot) < low);
        let end = self.order.partition_point(|&slot| self.key(slot) <= high);
        Self {
            order: &self.order[start..end.max(start)],
            whole: false,
            ..self
        }
    }

    /// The rows it reads at positions `at` in key order.
    pub fn within(self, at: std::ops::Range<usize>) -> Self {
        Self {
            order: &self.order[at],
            whole: false,
            ..self
        }
    }

    /// The slot of the row keyed `key` among those it reads.
    pub fn find(self, key: u128) -> Option<u32> {
        let slot = self.table.slot(key)?;
        // A truncated table reads only some live slots.
        (self.whole || self.position(key).is_ok()).then_some(slot)
    }

    /// `slot`'s cell in `column`, in its wire form.
    pub fn wire(self, slot: u32, column: usize) -> io::Result<Field<'a>> {
        wire(self.table, self.dictionary, slot, column, self.format)
    }

    /// `slot`'s field `column`, as the row decoder reads it into the row's field type.
    pub fn value<V: NativeValue>(self, slot: u32, column: usize) -> io::Result<V> {
        field_from_wire(self.id, column, self.wire(slot, column)?)
    }

    /// `slot`'s text field `column`; None where its cell was not recorded.
    pub fn bytes(self, slot: u32, column: usize) -> io::Result<Option<&'a [u8]>> {
        match self.wire(slot, column)? {
            Field::Bytes(bytes) => Ok(Some(bytes)),
            Field::NotRecorded => Ok(None),
            _ => Err(invalid(format!("{:?} column {column} is not text", self.id))),
        }
    }

    /// `slot`'s text field `column`; refused where its cell was not recorded.
    pub fn text(self, slot: u32, column: usize) -> io::Result<&'a [u8]> {
        self.bytes(slot, column)?.ok_or_else(|| {
            invalid(format!("{:?} column {column} was not recorded", self.id))
        })
    }

    /// `slot`'s list field `column` of 32-bit items; None where its cell was not recorded.
    pub fn u32s(self, slot: u32, column: usize) -> io::Result<Option<&'a [u32]>> {
        match self.wire(slot, column)? {
            Field::U32List(List::Native(items)) => Ok(Some(items)),
            Field::NotRecorded => Ok(None),
            _ => Err(invalid(format!("{:?} column {column} is not a u32 list", self.id))),
        }
    }

    /// `slot`'s list field `column` of 64-bit items; None where its cell was not recorded.
    pub fn u64s(self, slot: u32, column: usize) -> io::Result<Option<&'a [u64]>> {
        match self.wire(slot, column)? {
            Field::U64List(List::Native(items)) => Ok(Some(items)),
            Field::NotRecorded => Ok(None),
            _ => Err(invalid(format!("{:?} column {column} is not a u64 list", self.id))),
        }
    }

    /// The row `slot` holds, with the fields `mask` names as the row decoder builds them and
    /// every other default.
    pub fn row<R: RowCodec>(self, slot: u32, mask: u64) -> io::Result<R> {
        let key = self
            .table
            .key(slot)
            .ok_or_else(|| invalid(format!("{:?} slot {slot} holds no row", self.id)))?;
        R::from_wire_fields(key, mask, &mut |column| self.wire(slot, column))
    }

    /// The rows at `slots`, in their order, each holding the fields `mask` names.
    pub fn rows_at(self, slots: &[u32], mask: u64) -> io::Result<TableRows> {
        let mut keys = engine_allocation::vec(slots.len())?;
        for &slot in slots {
            keys.push(
                self.table
                    .key(slot)
                    .ok_or_else(|| invalid(format!("{:?} slot {slot} holds no row", self.id)))?,
            );
        }
        self.id.rows_from_wire_fields(&keys, mask, &mut |at, column| {
            self.wire(slots[at], column)
        })
    }

    /// Every row it reads, in key order, each holding the fields `mask` names: for a reader
    /// that keeps the table's rows itself.
    pub fn all_rows(self, mask: u64) -> io::Result<TableRows> {
        self.rows_at(self.order, mask)
    }
}

/// One table of a columnar walk's hour as a frame reads it, as [`super::TableView`] holds a
/// decoded one: its rows where it holds them, else why it holds none.
#[derive(Clone, Copy, Debug)]
pub enum ColumnView<'a> {
    Complete(Columns<'a>),
    /// The rows the hour's partial read reached, and how many it did not.
    Truncated { rows: Columns<'a>, missing: u32 },
    Unavailable { reason: &'a str },
    NotRecorded { mode: CaptureMode },
}

impl<'a> ColumnView<'a> {
    /// The rows it holds: a complete table's, or those a truncated one read.
    pub fn rows(self) -> Option<Columns<'a>> {
        match self {
            Self::Complete(rows) | Self::Truncated { rows, .. } => Some(rows),
            Self::Unavailable { .. } | Self::NotRecorded { .. } => None,
        }
    }

    pub fn is_complete(self) -> bool {
        matches!(self, Self::Complete(_))
    }

    /// Whether it is the view of a table no lane of the walk reads by view.
    pub fn is_not_selected(self) -> bool {
        matches!(self, Self::Unavailable { reason } if reason == super::NOT_SELECTED)
    }

    /// Its status, as an operand reads a table's.
    pub fn status(self) -> super::OperandStatus<'a> {
        match self {
            Self::Complete(_) => super::OperandStatus::Complete,
            Self::Truncated { missing, .. } => super::OperandStatus::Truncated(missing),
            Self::Unavailable { reason } => super::OperandStatus::Unavailable(reason),
            Self::NotRecorded { mode } => super::OperandStatus::NotRecorded(mode),
        }
    }

    /// A decoded view of it, its rows holding the fields `mask` names: for a reader handing
    /// the table on whole, as where an hour restates it.
    pub fn to_table_view(self, mask: u64) -> io::Result<super::TableView> {
        Ok(match self {
            Self::Complete(rows) => super::TableView::Complete(rows.all_rows(mask)?),
            Self::Truncated { rows, missing } => super::TableView::Truncated {
                rows: rows.all_rows(mask)?,
                missing,
            },
            Self::Unavailable { reason } => super::TableView::Unavailable {
                reason: engine_allocation::string(reason)?,
            },
            Self::NotRecorded { mode } => super::TableView::NotRecorded { mode },
        })
    }

    /// The complete table's rows; refused at `hour` for `table` otherwise, saying why, as
    /// [`super::DecodedFrame::complete`] refuses.
    pub fn complete(self, hour: Hour, table: TableId) -> io::Result<Columns<'a>> {
        match self {
            Self::Complete(rows) => Ok(rows),
            view => {
                view.status().complete(hour, table)?;
                Err(invalid(format!("hour {hour} {table:?} holds no rows")))
            }
        }
    }
}

/// Each table a walk reads by view: its live slots in key order, kept from hour to hour and
/// changed only where an hour inserted or deleted rows; a truncated table's, the slots the hour
/// read. Four bytes a row, in place of the row.
#[derive(Default)]
pub struct Orders {
    /// By table index; None for a table not kept.
    tables: Vec<Option<Order>>,
    /// Slots an hour inserted or deleted, a bit each, cleared after use.
    marks: Vec<u64>,
    keyed: Vec<(u128, u32)>,
    merged: Vec<u32>,
}

#[derive(Default)]
struct Order {
    slots: Vec<u32>,
    /// The status the slots were kept under; None before the first hour.
    status: Option<Status>,
}

impl Orders {
    /// The orders of `tables`, built at the first hour.
    pub fn new(tables: impl IntoIterator<Item = TableId>) -> io::Result<Self> {
        let mut orders = Self {
            tables: engine_allocation::collect_vec((0..TableId::ALL.len()).map(|_| None))?,
            ..Self::default()
        };
        for table in tables {
            orders.tables[table.index()] = Some(Order::default());
        }
        Ok(orders)
    }

    pub fn keeps(&self, table: TableId) -> bool {
        self.tables[table.index()].is_some()
    }

    /// `table`'s order, as its status this hour reads it; None where it is not kept.
    pub fn order(&self, table: TableId) -> Option<&[u32]> {
        self.tables[table.index()]
            .as_ref()
            .map(|order| order.slots.as_slice())
    }

    /// Keeps `slots` as `table`'s order: live slots of the walker's table, ascending by key,
    /// which a reader keeping its own rows of a table names itself.
    pub fn set(&mut self, table: TableId, slots: &[u32]) -> io::Result<()> {
        let order = self.tables[table.index()].get_or_insert_with(Order::default);
        order.slots.clear();
        engine_allocation::reserve(&mut order.slots, slots.len())?;
        order.slots.extend_from_slice(slots);
        Ok(())
    }

    /// Keeps no order of `table`.
    pub fn drop_table(&mut self, table: TableId) {
        self.tables[table.index()] = None;
    }

    /// The bytes it holds.
    pub fn heap_bytes(&self) -> usize {
        self.tables
            .iter()
            .flatten()
            .map(|order| order.slots.capacity() * 4)
            .sum::<usize>()
            + self.marks.capacity() * 8
            + self.keyed.capacity() * 32
            + self.merged.capacity() * 4
    }

    /// Brings each kept table's order to `state`'s hour; `anew`, as at a walk's first hour,
    /// rebuilds every one.
    pub fn hour(&mut self, state: &State, anew: bool) -> io::Result<()> {
        let Self {
            tables,
            marks,
            keyed,
            merged,
        } = self;
        for (index, order) in tables.iter_mut().enumerate() {
            let Some(order) = order else {
                continue;
            };
            let status = (!state.skips(index))
                .then(|| state.status(index))
                .flatten();
            match (status, state.table(index)) {
                (Some(Status::Complete), Some(table)) => {
                    let kept = !anew && order.status == Some(Status::Complete);
                    match state.changes(index) {
                        Some(changes) if kept => {
                            if !changes.inserted.is_empty() || !changes.deleted.is_empty() {
                                order.edit(table, changes, marks, keyed, merged)?;
                            }
                        }
                        _ => order.rebuild(table, table.live().ones(), keyed)?,
                    }
                }
                (Some(Status::Truncated { .. }), Some(table)) => {
                    let read = state.truncated_view(index).unwrap_or_default();
                    order.rebuild(table, read.iter().copied(), keyed)?;
                }
                _ => order.slots.clear(),
            }
            order.status = status;
        }
        Ok(())
    }
}

impl Order {
    /// The order of `slots`, live slots of `table`.
    fn rebuild(
        &mut self,
        table: &Table,
        slots: impl Iterator<Item = u32>,
        keyed: &mut Vec<(u128, u32)>,
    ) -> io::Result<()> {
        keyed.clear();
        for slot in slots {
            let key = table
                .key(slot)
                .ok_or_else(|| invalid("A table's read slot holds no row"))?;
            engine_allocation::reserve(keyed, 1)?;
            keyed.push((key, slot));
        }
        keyed.sort_unstable();
        self.slots.clear();
        engine_allocation::reserve(&mut self.slots, keyed.len())?;
        self.slots.extend(keyed.iter().map(|&(_, slot)| slot));
        Ok(())
    }

    /// The order after an hour of `changes` to `table`: its deleted and inserted slots taken
    /// out, then its inserted rows still live merged in by key.
    fn edit(
        &mut self,
        table: &Table,
        changes: &Changes,
        marks: &mut Vec<u64>,
        keyed: &mut Vec<(u128, u32)>,
        merged: &mut Vec<u32>,
    ) -> io::Result<()> {
        let words = table.slots().div_ceil(64);
        if marks.len() < words {
            engine_allocation::reserve(marks, words - marks.len())?;
            marks.resize(words, 0);
        }
        let moved = || changes.deleted.iter().chain(&changes.inserted).copied();
        for slot in moved() {
            if let Some(word) = marks.get_mut(slot as usize / 64) {
                *word |= 1 << (slot % 64);
            }
        }
        self.slots.retain(|&slot| {
            marks
                .get(slot as usize / 64)
                .is_none_or(|word| word >> (slot % 64) & 1 == 0)
        });
        for slot in moved() {
            if let Some(word) = marks.get_mut(slot as usize / 64) {
                *word = 0;
            }
        }
        keyed.clear();
        for &slot in &changes.inserted {
            if let Some(key) = table.key(slot) {
                engine_allocation::reserve(keyed, 1)?;
                keyed.push((key, slot));
            }
        }
        keyed.sort_unstable();
        keyed.dedup();
        merged.clear();
        engine_allocation::reserve(merged, self.slots.len() + keyed.len())?;
        let mut inserted = keyed.iter().peekable();
        for &slot in &self.slots {
            let key = table.key(slot).unwrap_or(0);
            while let Some(&&(next, at)) = inserted.peek()
                && next < key
            {
                merged.push(at);
                inserted.next();
            }
            merged.push(slot);
        }
        merged.extend(inserted.map(|&(_, slot)| slot));
        std::mem::swap(&mut self.slots, merged);
        Ok(())
    }
}

/// A columnar walk's hour read through views: each kept table's rows in key order where the
/// walker holds them ([`Orders`]), its status, and the hour's metrics. It borrows the walker's
/// state: it is gone before the walk goes on.
#[derive(Clone, Copy)]
pub struct ColumnFrame<'a> {
    pub branch: BranchId,
    pub hour: Hour,
    pub metrics: &'a super::Metrics,
    state: &'a State,
    orders: &'a Orders,
    /// A reader's own orders of tables it reads only some rows of, read in place of the walk's.
    own: Option<&'a Orders>,
    mode: CaptureMode,
    format: u16,
}

impl<'a> ColumnFrame<'a> {
    pub fn new(
        (branch, hour): (BranchId, Hour),
        metrics: &'a super::Metrics,
        state: &'a State,
        orders: &'a Orders,
        (mode, format): (CaptureMode, u16),
    ) -> Self {
        Self {
            branch,
            hour,
            metrics,
            state,
            orders,
            own: None,
            mode,
            format,
        }
    }

    /// The frame with each table `own` keeps an order of read at it in place of the walk's: a
    /// table holding only the rows a reader names itself.
    pub fn with_own(self, own: &'a Orders) -> Self {
        Self {
            own: Some(own),
            ..self
        }
    }

    /// The walker's state the views read.
    pub fn state(&self) -> &'a State {
        self.state
    }

    pub fn format(&self) -> u16 {
        self.format
    }

    /// `table` this hour; a withheld table is refused for its reason, as a decoder refuses it,
    /// and one the walk keeps no order of is not selected, as a decoder leaves a table it does
    /// not reconstruct.
    pub fn view(&self, table: TableId) -> ColumnView<'a> {
        super::view_reads::note_walked(table);
        let index = table.index();
        if let Some(reason) = self.state.withheld(index) {
            return ColumnView::Unavailable { reason };
        }
        let not_selected = ColumnView::Unavailable {
            reason: super::NOT_SELECTED,
        };
        let kept = match self.own.and_then(|own| own.order(table)) {
            Some(order) => Some((order, false)),
            None => self.orders.order(table).map(|order| (order, true)),
        };
        let (Some((order, every)), false) = (kept, self.state.skips(index)) else {
            return not_selected;
        };
        let Some(held) = self.state.table(index) else {
            return not_selected;
        };
        let columns = |whole| {
            Columns::new(
                table,
                held,
                self.state.dictionary(),
                (order, whole),
                self.format,
            )
        };
        match self.state.status(index) {
            None => not_selected,
            Some(Status::NotRecorded) => ColumnView::NotRecorded { mode: self.mode },
            Some(Status::Complete) => ColumnView::Complete(columns(every)),
            Some(Status::Truncated { missing }) => ColumnView::Truncated {
                rows: columns(false),
                missing,
            },
            Some(Status::Unavailable { reason }) => ColumnView::Unavailable {
                reason: self
                    .state
                    .dictionary()
                    .get(reason)
                    .and_then(|reason| std::str::from_utf8(reason).ok())
                    .unwrap_or("A table's reason is not text"),
            },
        }
    }

    /// `table`'s rows when complete; refused, saying why, otherwise.
    pub fn complete(&self, table: TableId) -> io::Result<Columns<'a>> {
        self.view(table).complete(self.hour, table)
    }

    /// The hour of `table`'s last complete acquisition the walk knows.
    pub fn sampled_at(&self, table: TableId) -> Option<Hour> {
        self.state.sampled_at(table.index())
    }
}
