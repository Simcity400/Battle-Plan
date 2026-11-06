//! A walker's state of some indexed tables, moved to any ordinal of a run, and rows looked up
//! by key.
//!
//! A move reads each table's status, sample hour, slot count and view, then its rows' strips,
//! then every column's strips on as many workers as it is given. A strip at the same page whose
//! changes between the two ordinals are none is left; one going forward on its page applies
//! the changes past where it stood; any other decodes its page at the ordinal. Every cell a
//! strip sets is compared with the one held, so the state's changes name exactly the rows and
//! columns that differ from where the cursor stood.

use super::{
    Layout, PAGE_BYTES, Page, Pages, at,
    cell::{self, Items, Position, Write},
    index::{Index, RunIndex},
};
use crate::{
    invalid,
    leaf::Reader,
    manifest::Status,
    section::Changes,
    state::{self, Bits, Cell, Column, Kind, Table},
    walk::{Dictionary, State},
};
use std::{
    io,
    sync::{
        Mutex, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
};

/// Ordinals a move may span and still run on its caller's thread alone.
const NEAR: u32 = 24;

/// Where one strip stands.
#[derive(Default)]
struct Strip {
    page: Option<usize>,
    ordinal: u32,
    position: Position,
    /// Its page's raw bytes while changes past `ordinal` remain.
    bytes: Vec<u8>,
    /// Its integers' last differences on its page.
    velocity: Vec<u64>,
    /// The offsets of its block its page set a cell at, a bit each.
    covered: Vec<u64>,
}

/// One table the cursor holds.
struct Held {
    /// Its place among the index's tables, and in the schema.
    at: usize,
    table: usize,
    /// Its strips by column then block, and its rows' by block.
    columns: Vec<Vec<Strip>>,
    rows: Vec<Strip>,
    /// The view entry its truncated view holds.
    view: Option<usize>,
    /// Each column's slots whose cell the last move changed.
    changed: Vec<Vec<u32>>,
    /// The rows the last move changed: each slot, with its key before, first.
    log: Vec<(u32, Option<u128>)>,
    /// Each slot's changed columns while a move gathers them, whether marked, and those marked.
    masks: Vec<u64>,
    marked: Vec<bool>,
    touched: Vec<u32>,
}

/// What a move read: pages, and their raw bytes.
#[derive(Default)]
struct Tally {
    pages: AtomicUsize,
    bytes: AtomicUsize,
}

impl Tally {
    fn add(&self, bytes: usize) {
        self.pages.fetch_add(1, Ordering::Relaxed);
        self.bytes.fetch_add(bytes, Ordering::Relaxed);
    }
}

/// A walker's [`State`] of some of an index's tables at one ordinal of one run; the others are
/// skipped.
pub struct Cursor {
    state: State,
    held: Vec<Held>,
    at: Option<(u32, u32)>,
    fresh: bool,
    workers: usize,
    read: (usize, usize),
    /// Each worker's list items.
    scratch: Vec<Items>,
}

/// Pushes `value` onto `into`.
fn noted<T>(into: &mut Vec<T>, value: T) -> io::Result<()> {
    if into.len() == into.capacity() {
        let more = into.capacity().max(4);
        engine_allocation::reserve_exact(into, more)?;
    }
    into.push(value);
    Ok(())
}

/// Empties `changes`.
fn clear(changes: &mut Changes) {
    changes.truncated = false;
    changes.read.clear();
    changes.changed.clear();
    changes.masks.clear();
    changes.written.clear();
    changes.deleted.clear();
    changes.deleted_keys.clear();
    changes.inserted.clear();
    changes.touched.clear();
    changes.filled.clear();
    changes.fill_slots.clear();
    changes.fill_masks.clear();
}

/// Runs `work` on each of `items`, on a thread of each of `scratch`, its list items, each
/// first running `on_worker`.
fn each<T: Send>(
    items: Vec<T>,
    scratch: &mut [Items],
    on_worker: &(dyn Fn() + Sync),
    work: &(dyn Fn(T, &mut Items) -> io::Result<()> + Sync),
) -> io::Result<()> {
    let workers = scratch.len().min(items.len());
    if workers <= 1 {
        let Some(items_read) = scratch.first_mut() else {
            return Err(invalid("A cursor's move has no worker"));
        };
        return items
            .into_iter()
            .try_for_each(|item| work(item, items_read));
    }
    let queue = Mutex::new(items.into_iter());
    let failed = Mutex::new(None);
    std::thread::scope(|scope| {
        for items_read in &mut scratch[..workers] {
            let (queue, failed) = (&queue, &failed);
            scope.spawn(move || {
                on_worker();
                loop {
                    let next = queue.lock().unwrap_or_else(PoisonError::into_inner).next();
                    let Some(item) = next else {
                        break;
                    };
                    if let Err(error) = work(item, items_read) {
                        failed
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .get_or_insert(error);
                        break;
                    }
                }
            });
        }
    });
    match failed.into_inner().unwrap_or_else(PoisonError::into_inner) {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Marks the offsets `change` sets a cell at in `covered`, of a block of `block` slots of
/// which the first `length` are the table's: a fill sets theirs. A difference is refused at an
/// offset none set.
fn cover(covered: &mut Vec<u64>, change: &Write<'_>, block: u32, length: u32) -> io::Result<()> {
    let words = block.div_ceil(64) as usize;
    if covered.len() < words {
        engine_allocation::resize(covered, words, 0)?;
    }
    let mut mark = |offset: u32| covered[(offset / 64) as usize] |= 1 << (offset % 64);
    match *change {
        Write::Cell { offset, .. } | Write::Row { offset, .. } => mark(offset),
        // A difference moves a cell the page set whole before; it sets none of its own.
        Write::Delta { offset, .. } | Write::Patch { offset, .. } => {
            if covered[(offset / 64) as usize] >> (offset % 64) & 1 == 0 {
                return Err(invalid("A strip moves a cell its page never set"));
            }
        }
        Write::Fill { except, .. } => {
            let mut except = except.iter().copied().peekable();
            for offset in 0..length {
                if except.next_if_eq(&offset).is_none() {
                    mark(offset);
                }
            }
        }
    }
    Ok(())
}

/// Whether `strip` set a cell at every slot `live` holds a row at among the `length` slots of
/// its block from `start`.
fn covers(strip: &Strip, live: &Bits, start: u32, length: u32) -> bool {
    let mut offset = 0;
    while offset < length {
        let count = (length - offset).min(64);
        let rows = state::span(live.words(), start + offset, count);
        if rows & !state::span(&strip.covered, offset, count) != 0 {
            return false;
        }
        offset += count;
    }
    true
}

/// Whether `strip` set a cell at `offset` of its block.
fn covered(strip: &Strip, offset: u32) -> bool {
    state::span(&strip.covered, offset, 1) != 0
}

/// Moves `strip` to `ordinal` along its pages `directory`, handing `write` each cell it sets:
/// `kind` is its column's, none for rows, and it is the strip of the block of `block` slots from `start` of a table of
/// `slots`, its slot counts. Whether it read its page anew.
#[allow(clippy::too_many_arguments)]
fn advance<P: AsRef<Page>>(
    strip: &mut Strip,
    directory: &[P],
    ordinal: u32,
    (kind, block, start): (Option<Kind>, u32, u32),
    slots: &[(u32, u32)],
    pages: &dyn Pages,
    tally: &Tally,
    items: &mut Items,
    write: &mut dyn FnMut(Write<'_>) -> io::Result<()>,
) -> io::Result<bool> {
    let first = |at: usize| directory.get(at).map(|page| page.as_ref().first);
    let at = match strip.page {
        Some(at)
            if first(at).is_some_and(|first| first <= ordinal)
                && first(at + 1).is_none_or(|next| next > ordinal) =>
        {
            at
        }
        _ => directory
            .partition_point(|page| page.as_ref().first <= ordinal)
            .checked_sub(1)
            .ok_or_else(|| invalid("A strip holds no page at the ordinal"))?,
    };
    let page = *directory[at].as_ref();
    // Every change it read is at or before its page's last ordinal: a move back to or past
    // that ordinal changes nothing.
    let anew = strip.page != Some(at) || (ordinal < strip.ordinal && ordinal < page.last);
    if page.location.raw > PAGE_BYTES {
        return Err(invalid("A change-run index's page passes its length"));
    }
    step(
        strip,
        (page, at, anew),
        ordinal,
        (kind, block),
        (slots, start),
        (pages, tally),
        items,
        write,
    )
}

/// [`advance`]'s move of `strip` along `page`, at `at` of its pages, read anew or not.
#[allow(clippy::too_many_arguments)]
fn step(
    strip: &mut Strip,
    (page, at, anew): (Page, usize, bool),
    ordinal: u32,
    (kind, block): (Option<Kind>, u32),
    (slots, start): (&[(u32, u32)], u32),
    (pages, tally): (&dyn Pages, &Tally),
    items: &mut Items,
    write: &mut dyn FnMut(Write<'_>) -> io::Result<()>,
) -> io::Result<bool> {
    if anew {
        strip.covered.clear();
    }
    let length = at_slots(slots, ordinal)?.saturating_sub(start).min(block);
    let Strip {
        page: held_page,
        ordinal: held_ordinal,
        position,
        bytes,
        velocity,
        covered,
    } = strip;
    let mut write = |change: Write<'_>| -> io::Result<()> {
        cover(covered, &change, block, length)?;
        write(change)
    };
    if !anew {
        if ordinal >= *held_ordinal && position.next.is_some_and(|next| next <= ordinal) {
            *position = cell::changes(
                bytes, kind, block, *position, ordinal, velocity, items, &mut write,
            )?;
            if position.next.is_none() {
                *bytes = Vec::new();
                *velocity = Vec::new();
            }
        }
        *held_ordinal = ordinal;
        return Ok(false);
    }
    *held_page = None;
    let held = at_slots(slots, page.first)?;
    let extent = held.saturating_sub(start).min(block);
    pages.read(page.location, bytes)?;
    tally.add(bytes.len());
    let from = cell::base(
        bytes,
        kind,
        (page.first, page.last),
        extent,
        velocity,
        items,
        &mut write,
    )?;
    *position = cell::changes(
        bytes, kind, block, from, ordinal, velocity, items, &mut write,
    )?;
    if position.next.is_none() {
        *bytes = Vec::new();
        *velocity = Vec::new();
    }
    *held_page = Some(at);
    *held_ordinal = ordinal;
    Ok(true)
}

/// The table's slot count at `ordinal`, as `slots`, its series, holds it.
fn at_slots(slots: &[(u32, u32)], ordinal: u32) -> io::Result<u32> {
    at(slots, ordinal)
        .map(|(_, &slots)| slots)
        .ok_or_else(|| invalid("A run index holds no slot count at a page's first ordinal"))
}

/// Sets `slot`'s row to `key`'s, or none, logging the key it held when they differ; the
/// table's slots by key are left to the move's end.
fn lifetime(
    table: &mut Table,
    slot: u32,
    key: Option<u128>,
    log: &mut Vec<(u32, Option<u128>)>,
) -> io::Result<()> {
    let held = table.key(slot);
    if held == key {
        return Ok(());
    }
    if slot as usize >= table.slots {
        return Err(invalid("A strip names a slot past its table"));
    }
    noted(log, (slot, held))?;
    match key {
        Some(key) => {
            table.live.set(slot)?;
            table.put_key(slot, key)?;
        }
        None => table.live.clear(slot),
    }
    Ok(())
}

/// Sets `slot`'s cell, keeping it in `changed` when `mark` and it held another.
#[allow(clippy::too_many_arguments)]
fn put(
    values: &mut Column,
    unrecorded: &mut Bits,
    changed: &mut Vec<u32>,
    mark: bool,
    slot: u32,
    cell: Cell<'_>,
    recorded: bool,
) -> io::Result<()> {
    if cell::set(values, unrecorded, slot, cell, recorded)? && mark {
        noted(changed, slot)?;
    }
    Ok(())
}

/// Sets the cell of the found row at `offset` of a block, `group` the found rows there by slot
/// with their places, from `start`.
#[allow(clippy::too_many_arguments)]
fn pick(
    values: &mut Column,
    unrecorded: &mut Bits,
    group: &[(u32, u32)],
    start: u32,
    offset: u32,
    cell: Cell<'_>,
    recorded: bool,
) -> io::Result<()> {
    if let Ok(at) = group.binary_search_by_key(&offset, |&(slot, _)| slot - start) {
        cell::set(values, unrecorded, group[at].1, cell, recorded)?;
    }
    Ok(())
}

/// A table's first move phase: its series and its rows.
struct Rows<'a> {
    held: &'a mut Held,
    table: &'a mut Table,
    changes: &'a mut Changes,
    status: &'a mut Status,
    sampled: &'a mut Option<i32>,
    view: &'a mut Vec<u32>,
}

impl Rows<'_> {
    #[allow(clippy::too_many_arguments)]
    fn seek(
        self,
        layout: Layout,
        run: &RunIndex,
        pages: &dyn Pages,
        ordinal: u32,
        hour: i32,
        tally: &Tally,
        items: &mut Items,
    ) -> io::Result<()> {
        let Self {
            held,
            table,
            changes,
            status,
            sampled,
            view,
        } = self;
        let series = &run.series[held.at];
        let entry = |what: &str| invalid(format!("A run index holds no {what} at the ordinal"));
        *status = *at(&series.statuses, ordinal)
            .ok_or_else(|| entry("status"))?
            .1;
        *sampled = at(&series.samples, ordinal)
            .ok_or_else(|| entry("sample hour"))?
            .1
            .at(hour);
        let slots = *at(&series.slots, ordinal)
            .ok_or_else(|| entry("slot count"))?
            .1;
        if matches!(*status, Status::Truncated { .. }) {
            let (entry_at, &location) = at(&series.views, ordinal).ok_or_else(|| entry("view"))?;
            if held.view != Some(entry_at) {
                held.view = None;
                if location.raw > PAGE_BYTES {
                    return Err(invalid("A change-run index's page passes its length"));
                }
                *view = Vec::new();
                let mut raw = Vec::new();
                pages.read(location, &mut raw)?;
                tally.add(raw.len());
                let mut reader = Reader::new(&raw);
                reader.slots_below(slots as usize, view)?;
                reader.finish()?;
                held.view = Some(entry_at);
            }
        } else {
            view.clear();
            held.view = None;
        }
        held.log.clear();
        if (slots as usize) < table.slots {
            for slot in slots..table.slots as u32 {
                lifetime(table, slot, None, &mut held.log)?;
            }
            table.slots = slots as usize;
            // Blocks past the table's end are read anew when it reaches them again.
            let blocks = layout.blocks(slots as usize);
            for strip in held.rows.iter_mut().skip(blocks) {
                *strip = Strip::default();
            }
            for strips in &mut held.columns {
                for strip in strips.iter_mut().skip(blocks) {
                    *strip = Strip::default();
                }
            }
        } else {
            table.resize(slots as usize)?;
        }
        let strips = &run.strips[held.at];
        for block in 0..layout.blocks(slots as usize) {
            if held.rows.len() <= block {
                engine_allocation::resize_default(&mut held.rows, block + 1)?;
            }
            let directory = strips.rows.get(block).map_or(&[][..], Vec::as_slice);
            let start = block as u32 * layout.block;
            let log = &mut held.log;
            advance(
                &mut held.rows[block],
                directory,
                ordinal,
                (None, layout.block, start),
                &series.slots,
                pages,
                tally,
                items,
                &mut |write| match write {
                    Write::Row { offset, key } => lifetime(table, start + offset, key, log),
                    _ => Err(invalid("A table's rows strip holds a cell")),
                },
            )?;
        }
        // The first entry of a slot holds its key before the move.
        held.log.sort_by_key(|&(slot, _)| slot);
        held.log.dedup_by_key(|&mut (slot, _)| slot);
        engine_allocation::reserve(&mut changes.deleted, held.log.len())?;
        engine_allocation::reserve(&mut changes.deleted_keys, held.log.len())?;
        engine_allocation::reserve(&mut changes.inserted, held.log.len())?;
        // Slots by key follow only the keys before and after: a block's base may hold a key
        // another block's slot holds by then. Every slot the move changed lets go of its key
        // before any takes its new one, as the slots held for the others read their keys.
        for &(slot, before) in &held.log {
            if let Some(key) = before
                && before != table.key(slot)
            {
                table.by_key.remove(key, slot)?;
                changes.deleted.push(slot);
                changes.deleted_keys.push(key);
            }
        }
        table.by_key.reserve(held.log.len())?;
        for &(slot, before) in &held.log {
            let after = table.key(slot);
            if let Some(key) = after
                && before != after
            {
                table.by_key.insert(&table.keys, key, slot)?;
                changes.inserted.push(slot);
            }
        }
        Ok(())
    }
}

/// A column's move phase: its strips.
struct Cells<'a> {
    kind: Kind,
    values: &'a mut Column,
    unrecorded: &'a mut Bits,
    strips: &'a mut Vec<Strip>,
    directory: &'a [Vec<Page>],
    slots: usize,
    /// The table's live rows, those the move inserted, ascending, and its slot counts.
    live: &'a Bits,
    inserted: &'a [u32],
    counts: &'a [(u32, u32)],
    changed: &'a mut Vec<u32>,
    /// Whether the cells it changes are kept.
    mark: bool,
}

impl Cells<'_> {
    fn seek(
        self,
        layout: Layout,
        pages: &dyn Pages,
        ordinal: u32,
        tally: &Tally,
        items: &mut Items,
    ) -> io::Result<()> {
        let Self {
            kind,
            values,
            unrecorded,
            strips,
            directory,
            slots,
            live,
            inserted,
            counts,
            changed,
            mark,
        } = self;
        changed.clear();
        for block in 0..layout.blocks(slots) {
            if strips.len() <= block {
                engine_allocation::resize_default(strips, block + 1)?;
            }
            let start = block as u32 * layout.block;
            let length = (slots - start as usize).min(layout.block as usize) as u32;
            let anew = advance(
                &mut strips[block],
                directory.get(block).map_or(&[][..], Vec::as_slice),
                ordinal,
                (Some(kind), layout.block, start),
                counts,
                pages,
                tally,
                items,
                &mut |write| match write {
                    Write::Cell {
                        offset,
                        cell,
                        recorded,
                    } => put(
                        values,
                        unrecorded,
                        changed,
                        mark,
                        start + offset,
                        cell,
                        recorded,
                    ),
                    Write::Delta {
                        offset,
                        delta,
                        recorded,
                    } => {
                        let slot = start + offset;
                        if cell::shift(values, unrecorded, slot, delta, recorded)? && mark {
                            noted(changed, slot)?;
                        }
                        Ok(())
                    }
                    Write::Patch {
                        offset,
                        front,
                        back,
                        middle,
                        recorded,
                    } => {
                        let slot = start + offset;
                        let kept = (front, back);
                        if cell::splice(values, unrecorded, slot, kept, middle, recorded)? && mark {
                            noted(changed, slot)?;
                        }
                        Ok(())
                    }
                    Write::Fill {
                        except,
                        cell,
                        recorded,
                    } => {
                        let mut except = except.iter().copied().peekable();
                        for offset in 0..length {
                            if except.next_if_eq(&offset).is_none() {
                                put(
                                    values,
                                    unrecorded,
                                    changed,
                                    mark,
                                    start + offset,
                                    cell,
                                    recorded,
                                )?;
                            }
                        }
                        Ok(())
                    }
                    Write::Row { .. } => Err(invalid("A column's strip holds rows")),
                },
            )?;
            // A live row's cell its page never set would read as the column's default.
            let strip = &strips[block];
            let new = &inserted[inserted.partition_point(|&slot| slot < start)
                ..inserted.partition_point(|&slot| slot < start + length)];
            let whole = if anew {
                covers(strip, live, start, length)
            } else {
                new.iter().all(|&slot| covered(strip, slot - start))
            };
            if !whole {
                return Err(invalid("A strip holds no cell of a live row"));
            }
        }
        Ok(())
    }
}

impl Held {
    /// The live slots, not inserted, whose cells the move changed, into `changes`.
    fn assemble(&mut self, table: &Table, changes: &mut Changes, status: Status) -> io::Result<()> {
        changes.truncated = matches!(status, Status::Truncated { .. });
        self.touched.clear();
        for (column, slots) in self.changed.iter().enumerate() {
            let bit = u32::try_from(column)
                .ok()
                .and_then(|column| 1_u64.checked_shl(column))
                .unwrap_or(0);
            for &slot in slots {
                let at = slot as usize;
                if self.masks.len() <= at {
                    engine_allocation::resize(&mut self.masks, at + 1, 0)?;
                    engine_allocation::resize(&mut self.marked, at + 1, false)?;
                }
                if !self.marked[at] {
                    self.marked[at] = true;
                    engine_allocation::reserve(&mut self.touched, 1)?;
                    self.touched.push(slot);
                }
                self.masks[at] |= bit;
            }
        }
        if self.touched.len() * 8 >= self.marked.len() {
            let marked = &self.marked;
            self.touched.clear();
            self.touched
                .extend((0..marked.len() as u32).filter(|&slot| marked[slot as usize]));
        } else {
            self.touched.sort_unstable();
        }
        engine_allocation::reserve(&mut changes.changed, self.touched.len())?;
        engine_allocation::reserve(&mut changes.masks, self.touched.len())?;
        engine_allocation::reserve(&mut changes.written, self.touched.len())?;
        for &slot in &self.touched {
            let at = slot as usize;
            let mask = std::mem::take(&mut self.masks[at]);
            self.marked[at] = false;
            if table.live().get(slot) && changes.inserted.binary_search(&slot).is_err() {
                changes.changed.push(slot);
                changes.masks.push(mask);
                changes.written.push(mask);
            }
        }
        Ok(())
    }
}

impl Cursor {
    /// A cursor of `tables`, schema tables of `index`'s, in a state of the recording's `schema`,
    /// its moves on up to `workers` threads.
    pub fn new(
        index: &Index,
        schema: Vec<Vec<Kind>>,
        tables: &[usize],
        workers: usize,
    ) -> io::Result<Self> {
        let mut tables = engine_allocation::copy_vec(tables)?;
        tables.sort_unstable();
        tables.dedup();
        let mut state = State::new(schema)?;
        let mut skipped = engine_allocation::filled(state.columns().len(), true)?;
        let mut held = engine_allocation::vec(tables.len())?;
        for &table in &tables {
            let at = index
                .table(table)
                .ok_or_else(|| invalid("A cursor's table is not in its run index"))?;
            let kinds = &index.kinds[at];
            if state.kinds.get(table) != Some(kinds) {
                return Err(invalid("A run index's table differs from the schema's"));
            }
            skipped[table] = false;
            held.push(Held {
                at,
                table,
                columns: engine_allocation::collect_vec(kinds.iter().map(|_| Vec::new()))?,
                rows: Vec::new(),
                view: None,
                changed: engine_allocation::collect_vec(kinds.iter().map(|_| Vec::new()))?,
                log: Vec::new(),
                masks: Vec::new(),
                marked: Vec::new(),
                touched: Vec::new(),
            });
        }
        state.skip(&skipped)?;
        Ok(Self {
            state,
            held,
            at: None,
            fresh: true,
            workers: workers.max(1),
            read: (0, 0),
            scratch: vec![Items::default()],
        })
    }

    pub fn set_workers(&mut self, workers: usize) {
        self.workers = workers.max(1);
        self.scratch.truncate(self.workers);
    }

    /// The state at the cursor's ordinal: each table held as a walker holds it there, and its
    /// changes since where the cursor stood before.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// Tells readers of its state they cannot read `table`, and why; none clears it.
    pub fn withhold(&mut self, table: usize, reason: Option<&str>) -> io::Result<()> {
        self.state.withhold(table, reason)
    }

    /// The run and ordinal the cursor stands at; none before its first move or after a
    /// failed one.
    pub fn at(&self) -> Option<(u32, u32)> {
        self.at
    }

    /// Whether the last move started from nothing held: its changes are every row.
    pub fn fresh(&self) -> bool {
        self.fresh
    }

    /// The pages the last move read, and their raw bytes.
    pub fn read(&self) -> (usize, usize) {
        self.read
    }

    /// Moves to `ordinal` of `run` of `index`, whose pages `pages` reads, its workers each first
    /// running `on_worker`. A failed move leaves the cursor holding nothing, to start from
    /// nothing.
    pub fn seek(
        &mut self,
        index: &Index,
        pages: &dyn Pages,
        run: u32,
        ordinal: u32,
        on_worker: &(dyn Fn() + Sync),
    ) -> io::Result<()> {
        let held_run = index
            .runs
            .get(run as usize)
            .ok_or_else(|| invalid("A run index holds no such run"))?;
        let hour = *held_run
            .hours
            .get(ordinal as usize)
            .ok_or_else(|| invalid("A run index's run holds no such ordinal"))?;
        let fresh = self.at.is_none_or(|(at, _)| at != run);
        // A move of a day or less reads too little to pay for its workers.
        let workers = match self.at {
            Some((_, from)) if !fresh && from.abs_diff(ordinal) <= NEAR => 1,
            _ => self.workers,
        };
        while self.scratch.len() < workers {
            self.scratch.push(Items::default());
        }
        self.at = None;
        let moved = self.moved(
            index,
            held_run,
            pages,
            ordinal,
            hour,
            fresh,
            (workers, on_worker),
        );
        if moved.is_err() {
            self.release()?;
        }
        moved?;
        self.at = Some((run, ordinal));
        self.fresh = fresh;
        Ok(())
    }

    /// Drops every table, strip and view it holds: its next move starts from nothing.
    fn release(&mut self) -> io::Result<()> {
        for held in &mut self.held {
            self.state.tables[held.table] = Table::new(&self.state.kinds[held.table])?;
            self.state.views[held.table] = Vec::new();
            self.state.changes[held.table] = Changes::default();
            for strips in &mut held.columns {
                *strips = Vec::new();
            }
            for changed in &mut held.changed {
                *changed = Vec::new();
            }
            held.rows = Vec::new();
            held.view = None;
            (held.log, held.masks, held.marked, held.touched) =
                (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        }
        self.scratch.iter_mut().for_each(Items::clear);
        self.at = None;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn moved(
        &mut self,
        index: &Index,
        run: &RunIndex,
        pages: &dyn Pages,
        ordinal: u32,
        hour: i32,
        fresh: bool,
        (workers, on_worker): (usize, &(dyn Fn() + Sync)),
    ) -> io::Result<()> {
        let layout = index.layout;
        if fresh {
            self.release()?;
            self.state.dictionary =
                Dictionary::run(pages.chunks(&run.chunks)?, run.strings as usize);
        }
        let present = std::mem::take(&mut self.state.present);
        for &table in &present {
            clear(&mut self.state.changes[table]);
        }
        self.state.present = present;
        self.state.present.clear();
        let tally = Tally::default();
        {
            let State {
                tables,
                changes,
                statuses,
                sampled,
                views,
                ..
            } = &mut self.state;
            let mut held = self.held.iter_mut().peekable();
            let mut work = engine_allocation::vec(held.len())?;
            let rows = tables
                .iter_mut()
                .zip(changes.iter_mut())
                .zip(statuses.iter_mut())
                .zip(sampled.iter_mut())
                .zip(views.iter_mut())
                .enumerate();
            for (table, ((((values, changes), status), sampled), view)) in rows {
                if let Some(held) = held.next_if(|held| held.table == table) {
                    work.push(Rows {
                        held,
                        table: values,
                        changes,
                        status,
                        sampled,
                        view,
                    });
                }
            }
            each(
                work,
                &mut self.scratch[..workers],
                on_worker,
                &|item, items| item.seek(layout, run, pages, ordinal, hour, &tally, items),
            )?;
        }
        {
            let mut work = Vec::new();
            let mut held = self.held.iter_mut().peekable();
            let State {
                tables, changes, ..
            } = &mut self.state;
            for (table, (values, changes)) in tables.iter_mut().zip(changes.iter()).enumerate() {
                let Some(held) = held.next_if(|held| held.table == table) else {
                    continue;
                };
                let strips = &run.strips[held.at];
                let counts = &run.series[held.at].slots;
                let kinds = &index.kinds[held.at];
                let slots = values.slots;
                let Table {
                    columns,
                    unrecorded,
                    live,
                    ..
                } = values;
                let live: &Bits = live;
                let cells = columns
                    .iter_mut()
                    .zip(unrecorded.iter_mut())
                    .zip(held.columns.iter_mut())
                    .zip(held.changed.iter_mut())
                    .zip(kinds)
                    .zip(&strips.columns);
                engine_allocation::reserve(&mut work, kinds.len())?;
                for (((((values, unrecorded), strips), changed), &kind), directory) in cells {
                    work.push(Cells {
                        kind,
                        values,
                        unrecorded,
                        strips,
                        directory,
                        slots,
                        live,
                        inserted: &changes.inserted,
                        counts,
                        changed,
                        mark: !fresh,
                    });
                }
            }
            each(
                work,
                &mut self.scratch[..workers],
                on_worker,
                &|item, items| item.seek(layout, pages, ordinal, &tally, items),
            )?;
        }
        let State {
            tables,
            changes,
            statuses,
            present,
            ..
        } = &mut self.state;
        for held in &mut self.held {
            let changes = &mut changes[held.table];
            held.assemble(&tables[held.table], changes, statuses[held.table])?;
            if !(changes.changed.is_empty()
                && changes.deleted.is_empty()
                && changes.inserted.is_empty())
            {
                engine_allocation::reserve(present, 1)?;
                present.push(held.table);
            }
        }
        // The run's dictionary holds what the tables name, read from the chunks holding it.
        for held in &self.held {
            self.state.hold_text(held.table)?;
        }
        self.state.trim_dictionary()?;
        self.read = (
            tally.pages.load(Ordering::Relaxed),
            tally.bytes.load(Ordering::Relaxed),
        );
        Ok(())
    }
}

/// A table's rows at some keys at one ordinal of a run.
#[derive(Debug)]
pub struct Keyed {
    pub status: Status,
    /// The complete table's sample hour; none for an incomplete read.
    pub sampled: Option<i32>,
    /// The rows live there at the keys asked, in a truncated table those its view holds, at
    /// slots `0..` in key order.
    pub table: Table,
    /// Pages read, and their raw bytes.
    pub read: (usize, usize),
}

/// Schema table `table`'s rows at `keys` at `ordinal` of `run` of `index`, whose pages
/// `pages` reads: of each block, the rows page at the ordinal when its keys may hold one of
/// them, then one page of each column where they lie.
pub fn rows_at(
    index: &Index,
    pages: &dyn Pages,
    run: u32,
    ordinal: u32,
    table: usize,
    keys: &[u128],
) -> io::Result<Keyed> {
    let at_table = index
        .table(table)
        .ok_or_else(|| invalid("A lookup's table is not in its run index"))?;
    let held_run = index
        .runs
        .get(run as usize)
        .ok_or_else(|| invalid("A run index holds no such run"))?;
    let hour = *held_run
        .hours
        .get(ordinal as usize)
        .ok_or_else(|| invalid("A run index's run holds no such ordinal"))?;
    let series = &held_run.series[at_table];
    let entry = |what: &str| invalid(format!("A run index holds no {what} at the ordinal"));
    let status = *at(&series.statuses, ordinal)
        .ok_or_else(|| entry("status"))?
        .1;
    let sampled = at(&series.samples, ordinal)
        .ok_or_else(|| entry("sample hour"))?
        .1
        .at(hour)
        .filter(|_| status == Status::Complete);
    let slots = *at(&series.slots, ordinal)
        .ok_or_else(|| entry("slot count"))?
        .1 as usize;
    let tally = Tally::default();
    let mut view = Vec::new();
    if matches!(status, Status::Truncated { .. }) {
        let (_, &location) = at(&series.views, ordinal).ok_or_else(|| entry("view"))?;
        let mut raw = Vec::new();
        pages.read(location, &mut raw)?;
        tally.add(raw.len());
        let mut reader = Reader::new(&raw);
        reader.slots(&mut view)?;
        reader.finish()?;
    }
    let mut wanted = engine_allocation::copy_vec(keys)?;
    wanted.sort_unstable();
    wanted.dedup();
    let layout = index.layout;
    let strips = &held_run.strips[at_table];
    // A lookup holds what it reads only while it answers.
    let mut items = Items::default();
    let mut rows: Vec<Option<u128>> = engine_allocation::filled(layout.block as usize, None)?;
    let mut found: Vec<(u128, u32)> = Vec::new();
    for block in 0..layout.blocks(slots) {
        let directory = strips.rows.get(block).map_or(&[][..], Vec::as_slice);
        let Some(page) = directory
            .partition_point(|page| page.page.first <= ordinal)
            .checked_sub(1)
            .map(|at| directory[at])
        else {
            return Err(invalid("A strip holds no page at the ordinal"));
        };
        let low = wanted.partition_point(|&key| key < page.low);
        if wanted.get(low).is_none_or(|&key| key > page.high) {
            continue;
        }
        rows.fill(None);
        let start = block as u32 * layout.block;
        advance(
            &mut Strip::default(),
            directory,
            ordinal,
            (None, layout.block, start),
            &series.slots,
            pages,
            &tally,
            &mut items,
            &mut |write| match write {
                Write::Row { offset, key } => {
                    *rows
                        .get_mut(offset as usize)
                        .ok_or_else(|| invalid("A strip names a slot past its block"))? = key;
                    Ok(())
                }
                _ => Err(invalid("A table's rows strip holds a cell")),
            },
        )?;
        for (offset, key) in rows.iter().enumerate() {
            if let Some(key) = *key
                && wanted.binary_search(&key).is_ok()
            {
                engine_allocation::reserve(&mut found, 1)?;
                found.push((key, start + offset as u32));
            }
        }
    }
    if matches!(status, Status::Truncated { .. }) {
        found.retain(|&(_, slot)| view.binary_search(&slot).is_ok());
    }
    found.sort_unstable();
    let kinds = &index.kinds[at_table];
    let mut out = Table::new(kinds)?;
    out.resize(found.len())?;
    out.by_key.reserve(found.len())?;
    for (dense, &(key, _)) in found.iter().enumerate() {
        out.put_key(dense as u32, key)?;
        out.live.set(dense as u32)?;
        out.by_key.insert(&out.keys, key, dense as u32)?;
    }
    let mut by_slot = engine_allocation::collect_vec(
        found
            .iter()
            .enumerate()
            .map(|(dense, &(_, slot))| (slot, dense as u32)),
    )?;
    by_slot.sort_unstable();
    let Table {
        columns,
        unrecorded,
        ..
    } = &mut out;
    for (column, ((values, unrecorded), &kind)) in columns
        .iter_mut()
        .zip(unrecorded.iter_mut())
        .zip(kinds)
        .enumerate()
    {
        let mut rest = &by_slot[..];
        while let Some(&(slot, _)) = rest.first() {
            let block = slot / layout.block;
            let start = block * layout.block;
            let here = rest.partition_point(|&(slot, _)| slot / layout.block == block);
            let (group, after) = rest.split_at(here);
            rest = after;
            let mut strip = Strip::default();
            advance(
                &mut strip,
                strips
                    .columns
                    .get(column)
                    .and_then(|blocks| blocks.get(block as usize))
                    .map_or(&[][..], Vec::as_slice),
                ordinal,
                (Some(kind), layout.block, start),
                &series.slots,
                pages,
                &tally,
                &mut items,
                &mut |write| match write {
                    Write::Cell {
                        offset,
                        cell,
                        recorded,
                    } => pick(values, unrecorded, group, start, offset, cell, recorded),
                    Write::Delta {
                        offset,
                        delta,
                        recorded,
                    } => {
                        if let Ok(at) =
                            group.binary_search_by_key(&offset, |&(slot, _)| slot - start)
                        {
                            cell::shift(values, unrecorded, group[at].1, delta, recorded)?;
                        }
                        Ok(())
                    }
                    Write::Patch {
                        offset,
                        front,
                        back,
                        middle,
                        recorded,
                    } => {
                        if let Ok(at) =
                            group.binary_search_by_key(&offset, |&(slot, _)| slot - start)
                        {
                            let (dense, kept) = (group[at].1, (front, back));
                            cell::splice(values, unrecorded, dense, kept, middle, recorded)?;
                        }
                        Ok(())
                    }
                    Write::Fill {
                        except,
                        cell,
                        recorded,
                    } => {
                        for &(slot, _) in group {
                            if except.binary_search(&(slot - start)).is_err() {
                                pick(
                                    values,
                                    unrecorded,
                                    group,
                                    start,
                                    slot - start,
                                    cell,
                                    recorded,
                                )?;
                            }
                        }
                        Ok(())
                    }
                    Write::Row { .. } => Err(invalid("A column's strip holds rows")),
                },
            )?;
            if !group.iter().all(|&(slot, _)| covered(&strip, slot - start)) {
                return Err(invalid("A strip holds no cell of a live row"));
            }
        }
    }
    Ok(Keyed {
        status,
        sampled,
        table: out,
        read: (
            tally.pages.load(Ordering::Relaxed),
            tally.bytes.load(Ordering::Relaxed),
        ),
    })
}
