//! A table its producer writes while it reads the store's other tables ([`Store::lend`]): the
//! table leaves the store for the producer's run, a new row goes straight in as it is set, and the
//! cells a set changes of an older row wait, they alone, until the table is landed
//! ([`Store::land`]). Landing gives the hour's log and the unit exactly what [`super::TableChanges`]
//! would have had from the same rows set in key order: new rows at the slots key order gives them,
//! changed cells in key order, deletes last. A row set and deleted in one hour keeps its set.

use super::{Flags, ListPage, ListPolicy, PAGE, Store, Strings, TableState, TableView, Value, label, log_push, past_slots, value_of};
use crate::{
    invalid,
    state::{Cell, Kind},
};
use std::io;

/// An older row's cell as a set changed it.
enum Changed {
    Value(Value<'static>),
    List(Box<[u8]>),
}

/// A table lent to its producer ([`Store::lend`]).
pub struct Lent {
    table: usize,
    state: TableState,
    kinds: Vec<Kind>,
    /// The slot count at the lend, which an undo restores.
    slots: u32,
    /// The table held no row at the lend: every row it holds is new.
    empty: bool,
    /// The new rows' slots in the order set, ascending; not kept when `empty`.
    born: Vec<u32>,
    /// The new rows' keys ascend in the order set, the last one's held.
    ordered: bool,
    last: Option<u128>,
    /// The slots of rows set since the lend.
    set: Flags,
    /// Older rows' cells a set changed: slot, column and the cell now.
    changed: Vec<(u32, u16, Changed)>,
    deletes: Vec<u128>,
    /// A set or delete named the table.
    named: bool,
    scratch: Vec<u8>,
}

/// A lent table taken back, its hour not yet landed ([`Store::land`]).
pub struct Landing {
    table: usize,
    empty: bool,
    born: Vec<u32>,
    ordered: bool,
    set: Flags,
    changed: Vec<(u32, u16, Changed)>,
    deletes: Vec<u128>,
    named: bool,
}

impl Landing {
    /// A set or delete named the table this hour.
    pub fn named(&self) -> bool {
        self.named
    }
}

impl Lent {
    pub fn table(&self) -> usize {
        self.table
    }

    pub fn kinds(&self) -> &[Kind] {
        &self.kinds
    }

    /// The cells of older rows the sets since the lend changed.
    pub fn changed(&self) -> usize {
        self.changed.len()
    }

    /// The table as its producer wrote it so far: its older rows as the hour began, its new rows
    /// as set.
    pub fn view<'a>(&'a self, strings: &'a Strings) -> TableView<'a> {
        TableView::new(&self.state, &self.kinds, strings)
    }

    /// Row `key` holds `cells`, one per column, `None` not recorded: a new row is written at the
    /// lowest free slot, a cell an older row holds equal is nothing. False, and nothing written,
    /// when `key` was set since the lend.
    pub fn set<'c>(&mut self, key: u128, cells: impl IntoIterator<Item = Option<Cell<'c>>>) -> io::Result<bool> {
        self.named = true;
        let Self { table, state, kinds, set, changed, scratch, born, empty, ordered, last, .. } = self;
        let kind = |column: usize| kinds.get(column).copied().ok_or_else(|| invalid(format!("No column {column}")));
        if let Some(slot) = state.slot(key) {
            if set.get(slot) {
                return Ok(false);
            }
            set.set(slot);
            for (column, cell) in cells.into_iter().enumerate() {
                let value = match cell {
                    Some(cell) => value_of(kind(column)?, cell, scratch)?,
                    None => Value::Unrecorded,
                };
                if state.value(column, slot) != value {
                    let now = match value {
                        Value::List(bytes) => Changed::List(bytes.into()),
                        scalar => Changed::Value(scalar.owned_scalar()),
                    };
                    log_push(changed, (slot, column as u16, now));
                }
            }
            return Ok(true);
        }
        let slot = state.lowest_free().ok_or_else(|| past_slots(format_args!("Table {table}")))?;
        state.insert_at(slot, key)?;
        set.set(slot);
        if !*empty {
            log_push(born, slot);
        }
        *ordered &= last.is_none_or(|last| last < key);
        *last = Some(key);
        let (slots, policy) = (state.slots as usize, state.lists);
        for (column, cell) in cells.into_iter().enumerate() {
            let kind = kind(column)?;
            let held = state.columns.get_mut(column).ok_or_else(|| invalid(format!("No column {column}")))?;
            match cell {
                Some(cell) => held.put(slot, value_of(kind, cell, scratch)?, slots, policy)?,
                None => held.unrecord(slot),
            }
        }
        Ok(true)
    }

    /// The table holds no row `key` once landed, unless the hour sets it.
    pub fn delete(&mut self, key: u128) {
        self.named = true;
        self.deletes.push(key);
    }

    /// The table as it was lent: every row set since gone, and nothing to land.
    pub fn undo(&mut self) -> io::Result<()> {
        let born = if self.empty { self.state.live.ones().collect() } else { std::mem::take(&mut self.born) };
        for slot in born {
            self.state.delete(slot)?;
        }
        self.state.cut(self.slots);
        self.set = Flags::default();
        self.changed = Vec::new();
        self.deletes = Vec::new();
        self.named = false;
        Ok(())
    }
}

impl TableState {
    /// The rows at `slots`, ascending, moved among those slots so their keys ascend with them.
    fn order(&mut self, slots: &[u32]) -> io::Result<()> {
        let _label = label("store: rows ordered");
        let mut by_key: Vec<u32> = (0..slots.len() as u32).collect();
        by_key.sort_unstable_by_key(|&at| self.keys.get(slots[at as usize]));
        let mut place = vec![0_u32; slots.len()];
        for (rank, &at) in by_key.iter().enumerate() {
            place[at as usize] = rank as u32;
        }
        drop(by_key);
        let mut scratch = (Vec::new(), Vec::new());
        for at in 0..slots.len() {
            while place[at] as usize != at {
                let to = place[at] as usize;
                self.swap_rows(slots[at], slots[to], &mut scratch)?;
                place.swap(at, to);
            }
        }
        Ok(())
    }

    /// The live rows at `a` and `b` trade slots.
    fn swap_rows(&mut self, a: u32, b: u32, scratch: &mut (Vec<u8>, Vec<u8>)) -> io::Result<()> {
        let (key_a, key_b) = (self.keys.get(a), self.keys.get(b));
        if let Some(index) = &mut self.index {
            index.remove(key_a, a, &self.keys);
            index.remove(key_b, b, &self.keys);
        }
        let slots = self.slots as usize;
        self.keys.set(a, key_b, slots);
        self.keys.set(b, key_a, slots);
        if let Some(index) = &mut self.index {
            index.insert(key_b, a, &self.keys);
            index.insert(key_a, b, &self.keys);
        }
        let policy = self.lists;
        for column in &mut self.columns {
            column.swap_cells(a, b, policy, scratch)?;
        }
        Ok(())
    }
}

impl super::ColumnState {
    /// The cells at `a` and `b`, both below the slot count, trade places, recorded or not.
    fn swap_cells(&mut self, a: u32, b: u32, policy: ListPolicy, scratch: &mut (Vec<u8>, Vec<u8>)) -> io::Result<()> {
        let (recorded_a, recorded_b) = (self.recorded(a), self.recorded(b));
        match self {
            Self::Number { cells, present, .. } => {
                let held = cells.get(a);
                cells.store(a, cells.get(b));
                cells.store(b, held);
                if let Some(present) = present {
                    let held = present.get(a);
                    present.put(a, present.get(b));
                    present.put(b, held);
                }
            }
            Self::Bool { cells, .. } => {
                let held = cells.get(a);
                cells.put(a, cells.get(b));
                cells.put(b, held);
            }
            Self::Text { ids, .. } => {
                let held = ids.get(a);
                ids.store(a, ids.get(b));
                ids.store(b, held);
            }
            Self::List { pages, .. } => {
                fn cell(pages: &[ListPage], slot: u32) -> &[u8] {
                    pages.get(slot as usize / PAGE).map_or(&[][..], |page| page.get(slot as usize % PAGE))
                }
                let (first, second) = scratch;
                first.clear();
                first.extend_from_slice(cell(pages, a));
                second.clear();
                second.extend_from_slice(cell(pages, b));
                for (slot, bytes) in [(a, &*second), (b, &*first)] {
                    let page = slot as usize / PAGE;
                    if pages.len() <= page {
                        pages.resize_with(page + 1, ListPage::default);
                    }
                    pages[page].set(slot as usize % PAGE, bytes, policy)?;
                }
            }
        }
        if let Some(bits) = self.unrecorded_mut() {
            bits.put(a, !recorded_b);
            bits.put(b, !recorded_a);
        }
        Ok(())
    }
}

impl Store {
    /// Lends table `table` to its producer, which writes it ([`Lent::set`]) while the store's
    /// other tables are read; the store holds an empty table in its place until
    /// [`Self::take_back`]. Refused for a log and for a table keeping its last complete read.
    pub fn lend(&mut self, table: usize) -> io::Result<Lent> {
        let kinds = self.kinds.get(table).ok_or_else(|| invalid(format!("No table {table}")))?.clone();
        if self.tables[table].log {
            return Err(invalid(format!("Table {table} is a log: it is appended to, never lent")));
        }
        if self.keeps.get(table).copied().unwrap_or(false) {
            return Err(invalid(format!("Table {table} keeps its last complete read: it is never lent")));
        }
        let state = std::mem::replace(&mut self.tables[table], TableState::new(&kinds));
        let (slots, empty) = (state.slots, state.live.count() == 0);
        Ok(Lent {
            table,
            state,
            kinds,
            slots,
            empty,
            born: Vec::new(),
            ordered: true,
            last: None,
            set: Flags::default(),
            changed: Vec::new(),
            deletes: Vec::new(),
            named: false,
            scratch: Vec::new(),
        })
    }

    /// The table `lent` holds back in the store as its producer wrote it, its hour landed next
    /// ([`Self::land`]).
    pub fn take_back(&mut self, lent: Lent) -> Landing {
        let Lent { table, state, empty, born, ordered, set, changed, deletes, named, .. } = lent;
        self.tables[table] = state;
        Landing { table, empty, born, ordered, set, changed, deletes, named }
    }

    /// What `landing`'s producer wrote, given to the hour's log and the unit as if each row had
    /// been set through [`Self::table`] in key order, then its deletes but of rows it set.
    pub fn land(&mut self, landing: Landing) -> io::Result<()> {
        let Landing { table, empty, born, ordered, set, mut changed, mut deletes, named } = landing;
        if !named {
            return Ok(());
        }
        self.table(table)?;
        let born = if empty { self.tables[table].live.ones().collect() } else { born };
        if !ordered {
            self.tables[table].order(&born)?;
        }
        let columns = self.kinds[table].len();
        for &slot in &born {
            if !self.whole {
                self.born[table].set(slot);
                log_push(&mut self.log.tables[table].inserted, slot);
            }
            for column in 0..columns {
                let held = &self.tables[table].columns[column];
                let recorded = held.recorded(slot);
                let text = match held.held(slot) {
                    Value::Text(id) if recorded => Some(id),
                    _ => None,
                };
                if let Some(id) = text {
                    self.hold_again(id)?;
                }
                if !self.whole {
                    log_push(&mut self.log.tables[table].born_written, (slot, column as u16));
                } else if !recorded {
                    self.unrecorded_given.push((table, column as u16, slot));
                }
            }
        }
        let keys = &self.tables[table].keys;
        changed.sort_by_key(|&(slot, column, _)| (keys.get(slot), column));
        let mut changes = self.table(table)?;
        for (slot, column, now) in &changed {
            let value = match now {
                Changed::Value(value) => *value,
                Changed::List(bytes) => Value::List(bytes),
            };
            changes.put_value(*slot, usize::from(*column), value)?;
        }
        deletes.sort_unstable();
        deletes.dedup();
        for key in deletes {
            if let Some(slot) = changes.slot(key)
                && !set.get(slot)
            {
                changes.delete(slot)?;
            }
        }
        Ok(())
    }
}
