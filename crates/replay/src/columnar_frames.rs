//! A recording of format 12 ([`Recording`]) and its tables at an hour as the row decoder's
//! frames ([`Frames`]): each selected table read by its own [`TableReader`], a frame's view of a
//! table made only in an hour that changed it, and kept, shared, while hours leave it alone. A
//! view made after a step of one hour is the view before with the rows the hour changed patched
//! from their cells; any other is built from every cell. With change facts, a view made after a
//! step names what the hour changed, as its reader's step gives it.

use crate::{
    cells::TableReader,
    frame::{DecodedFrame, Metrics, READ_TIME_NOT_RECORDED, RecordedFrame, RowChange, SharedView, Stamp, TableUpdate, TableView, Unsayable, cells_rows, cells_view, field, not_selected},
    rows::{DeltaRow, RowChanges, TABLE_COUNT, TableId, TableRows, full_mask},
    scope::CaptureMode,
};
use rayon::prelude::*;
use replay_columnar::{lane::ClockHour, manifest::Status, state::Kind as Column, unit::BLOCK};
use std::{io, sync::Arc, time::Instant};

mod input;
mod reader;
pub use input::{NotSaved, Source, is_not_saved};
pub use reader::{Damage, DamageScope, Recording, Run, strings_frame, strings_of, table_name};
pub use replay_columnar::container::Open;

#[cfg(any(test, feature = "test-support"))]
pub mod fixtures;
#[cfg(any(test, feature = "test-support"))]
pub mod synthetic;
#[cfg(test)]
mod tests;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// The tables whose last complete read the recorder's store keeps beside the table.
pub const LAST_COMPLETE: [TableId; 3] = [TableId::Divisions, TableId::DivisionEquipmentDeletions, TableId::SunkShips];

/// [`LAST_COMPLETE`] by table index.
pub fn keeps_last() -> [bool; TABLE_COUNT] {
    let mut keeps = [false; TABLE_COUNT];
    for table in LAST_COMPLETE {
        keeps[table.index()] = true;
    }
    keeps
}

/// Tables and columns as the schema names them, for the journal adapter's refusals.
pub fn names() -> replay_columnar::store::journal::Names {
    replay_columnar::store::journal::Names {
        table: |table| TableId::ALL.get(table).map_or_else(|| format!("table {table}"), |id| format!("{id:?}")),
        column: |table, column| TableId::ALL.get(table).and_then(|id| id.field_name(column)).map_or_else(|| format!("{column}"), str::to_owned),
    }
}

/// Every schema table's column kinds, in the recording's table order.
pub fn schema() -> io::Result<Vec<Vec<Column>>> {
    let mut schema = engine_allocation::vec(TableId::ALL.len())?;
    for table in TableId::ALL {
        schema.push(crate::native_columns::columns(table.schema())?);
    }
    Ok(schema)
}

/// The columns of each table a recording of row format `format` holds: the tables introduced by
/// then, which are this build's first (a table added later is appended, a retired one keeps its
/// place), each the fields that format wrote, a table whose layout changed since as its earlier
/// layout framed it.
pub fn columns_at(format: u16) -> io::Result<Vec<usize>> {
    engine_allocation::collect_vec(TableId::ALL.into_iter().take_while(|table| table.last_change().introduced <= format).map(|table| table.last_change().framed_fields(table, format)))
}

/// The tables of a recording of row format `format` this build reads otherwise, each with why
/// by name: those added since, which it does not hold, and those whose layout or meaning changed
/// since. Retired tables keep their framing and no reader reads them.
pub fn withheld_at(format: u16) -> io::Result<Vec<(usize, String)>> {
    let mut withheld = Vec::new();
    for table in TableId::ALL {
        let change = table.last_change();
        if change.retired.is_some() {
            continue;
        }
        if let Some(gap) = change.gap(table, format) {
            withheld.push((table.index(), gap.to_string()));
        }
    }
    Ok(withheld)
}

/// The bytes `view`'s rows, or its reason, hold.
pub fn view_heap_bytes(view: &TableView) -> usize {
    match view {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.heap_bytes(),
        TableView::Unavailable { reason } => reason.capacity(),
        TableView::NotRecorded { .. } => 0,
    }
}

/// The bytes `frame`'s views hold, each counted though another frame shares it.
pub fn frame_heap_bytes(frame: &DecodedFrame) -> usize {
    frame.tables.iter().map(|view| view_heap_bytes(view)).sum()
}

/// Threads of their own that a reader's work runs on in place of rayon's, for work that must
/// not compete with the rest at its priority, or must not outlive their owner: each started once,
/// set by `start`. Dropped, it joins every thread it started, but on one of them.
pub struct Pool {
    pool: rayon::ThreadPool,
    // Dropped after `pool`, whose drop only tells its threads to end.
    threads: Joined,
}

impl Drop for Pool {
    fn drop(&mut self) {
        // On one of its own threads no join can finish: rayon's termination ends them unjoined.
        if self.pool.current_thread_index().is_some() {
            self.threads.0.clear();
        }
    }
}

struct Joined(Vec<std::thread::JoinHandle<()>>);

impl Drop for Joined {
    fn drop(&mut self) {
        for thread in self.0.drain(..) {
            let _ = thread.join();
        }
    }
}

impl Pool {
    /// `threads` threads named `name`, each running `start` first.
    pub fn new(threads: usize, name: &'static str, start: fn()) -> io::Result<Self> {
        let count = threads.max(1);
        let mut threads = Joined(Vec::new());
        engine_allocation::reserve(&mut threads.0, count)?;
        // A failed build ends the threads it started; `threads` joins them.
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(count)
            .thread_name(move |at| format!("{name} {at}"))
            .start_handler(move |_| start())
            .spawn_handler(|thread| {
                let mut builder = std::thread::Builder::new();
                if let Some(name) = thread.name() {
                    builder = builder.name(name.to_owned());
                }
                if let Some(size) = thread.stack_size() {
                    builder = builder.stack_size(size);
                }
                threads.0.push(builder.spawn(|| thread.run())?);
                Ok(())
            })
            .build()
            .map_err(io::Error::other)?;
        Ok(Self { pool, threads })
    }

    /// `work`, its rayon work on the pool's threads.
    pub fn install<R: Send>(&self, work: impl FnOnce() -> R + Send) -> R {
        self.pool.install(work)
    }

    /// `work`, its rayon work on `pool`'s threads, or on rayon's own without one.
    pub fn on<R: Send>(pool: Option<&Self>, work: impl FnOnce() -> R + Send) -> R {
        match pool {
            Some(pool) => pool.install(work),
            None => work(),
        }
    }
}

/// Hours a reader takes in one go: ordinals `first..=last` of run `run`, read as `branch` (the
/// run's, or the timeline a view reads it for).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Segment {
    pub branch: u64,
    pub run: u32,
    /// Always none: every hour of a run is reached from its tables' own restores.
    pub restart: Option<usize>,
    pub first: u32,
    pub last: u32,
}

/// One selected table of [`Frames`]: its reader, or why it holds no rows; its view as built last.
struct Slot {
    id: TableId,
    reader: Option<TableReader>,
    /// Why the table is not read in this run: withheld, or its recorded bytes damaged.
    lost: Option<String>,
    view: Option<SharedView>,
    /// The view's hour's update, as the recording's facts give it.
    update: Option<Arc<TableUpdate>>,
    spent: Spent,
}

/// What one table's views cost so far: how many were patched (in place, or on a copy of rows a
/// frame still held) and how many built from every cell, how many Snapshot updates were the one
/// before patched; with [`Frames::timed`], the nanoseconds its reader moved and its views and
/// updates were made in, on whichever thread made them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Spent {
    pub patched: u64,
    pub in_place: u64,
    pub rebuilt: u64,
    pub snapshots_patched: u64,
    pub move_ns: u64,
    pub view_ns: u64,
    pub update_ns: u64,
}

/// The selected tables of one run at an hour, as frames: each table's cells read by its own
/// reader ([`TableReader`]), the next hour a step, a later hour of the run an advance from where
/// each table stands unless a restore of it lies between, any other hour a seek (a restore and
/// one advance). A table withheld, or whose bytes cannot be read, is unavailable by name from the hour
/// it fails, never empty, until a seek reaches an hour it reads.
///
/// A complete view made at an hour stepped to from the hour whose frame was built last, the table
/// complete at both, is that frame's view with the rows the hour inserted or wrote made from their
/// cells and those it deleted removed: in the rows' own storage when no frame holds them any more,
/// else on a copy. A Snapshot update of such an hour is the hour before's patched the same way
/// when no recorded frame holds it. Every other view is built from every cell.
///
/// With change facts ([`Self::with_change_facts`]) every complete view carries a field stamp, and
/// one built at an hour stepped to from the hour whose frame was built last also carries that
/// hour's change facts against the view it replaces; any other move gives a stamp of its own.
pub struct Frames<R: std::ops::Deref<Target = Recording>> {
    recording: R,
    slots: Vec<Slot>,
    run: Option<u32>,
    ordinal: u32,
    /// The run's first hour.
    first_hour: i32,
    /// The clock of the block under way.
    clock: Option<(u32, Vec<ClockHour>)>,
    mode: CaptureMode,
    format: u16,
    /// The next frame is a keyframe: each table built anew is a Snapshot, and no view carries
    /// change facts.
    anew: bool,
    /// Complete views carry change facts and field stamps.
    facts: bool,
    /// The run and ordinal whose frame was built last.
    built: Option<(u32, u32)>,
    /// The hour it holds was stepped to from the hour whose frame was built last.
    stepped: bool,
    /// A view stepped to is patched; off, every view is built from every cell.
    patching: bool,
    /// Its tables' [`Spent`] times are taken.
    timed: bool,
}

impl<R: std::ops::Deref<Target = Recording>> Frames<R> {
    /// Frames of `tables` of `recording`, the columns each reads as it projects them.
    pub fn new(recording: R, tables: &[TableId]) -> io::Result<Self> {
        let mut chosen = [false; TABLE_COUNT];
        for table in tables {
            chosen[table.index()] = true;
        }
        let slots = TableId::ALL.into_iter().filter(|table| chosen[table.index()]).map(|id| Slot { id, reader: None, lost: None, view: None, update: None, spent: Spent::default() }).collect();
        Ok(Self {
            slots,
            run: None,
            ordinal: 0,
            first_hour: 0,
            clock: None,
            mode: CaptureMode::from_byte(recording.root_identity().capture_mode)?,
            format: recording.format(),
            anew: true,
            facts: false,
            built: None,
            stepped: false,
            patching: true,
            timed: false,
            recording,
        })
    }

    /// Every view built from every cell, never patched: the reference a patched view is held to.
    pub fn rebuilding(mut self) -> Self {
        self.patching = false;
        self
    }

    /// Its tables' times taken, each [`Spent`]'s nanoseconds.
    pub fn timed(mut self) -> Self {
        self.timed = true;
        self
    }

    /// Each table it reads and what its views cost so far, in table order.
    pub fn spent(&self) -> impl Iterator<Item = (TableId, Spent)> + '_ {
        self.slots.iter().map(|slot| (slot.id, slot.spent))
    }

    /// Complete views carrying change facts and field stamps, as a decoder with change facts
    /// makes them.
    pub fn with_change_facts(mut self) -> Self {
        self.facts = true;
        self
    }

    /// The tables it reads, in table order.
    pub fn selected(&self) -> impl Iterator<Item = TableId> + '_ {
        self.slots.iter().map(|slot| slot.id)
    }

    /// The run and game hour it holds; none before the first seek.
    pub fn at(&self) -> Option<(u32, i32)> {
        self.run.map(|run| (run, self.first_hour + self.ordinal as i32))
    }

    /// `table`'s reader, when it reads the table and the table is not lost.
    pub fn reader(&self, table: TableId) -> Option<&TableReader> {
        self.slots.iter().find(|slot| slot.id == table)?.reader.as_ref()
    }

    /// Every table at the hour of run ordinal `ordinal` of run `run`, each going on from where it
    /// stands when [`TableReader::go_to`] finds that cheaper than its restore.
    pub fn seek(&mut self, run: u32, ordinal: u32) -> io::Result<()> {
        let recording: &Recording = &self.recording;
        let held = recording.run(run)?;
        if ordinal >= held.ordinals {
            return Err(invalid(format!("Run {run} holds no ordinal {ordinal}")));
        }
        let new_run = self.run != Some(run);
        // A reader lost to damage is made again: whether the table reads depends on the hour sought.
        for slot in self.slots.iter_mut().filter(|slot| new_run || slot.reader.is_none()) {
            slot.lost = None;
            slot.reader = match TableReader::new(recording, run, slot.id, None) {
                Ok(reader) => Some(reader),
                Err(error) => {
                    slot.lost = Some(lost(error)?);
                    None
                }
            };
        }
        if new_run {
            self.clock = None;
        }
        self.run = Some(run);
        self.first_hour = held.first_hour;
        self.ordinal = ordinal;
        self.anew = true;
        self.stepped = false;
        let timed = self.timed;
        self.slots.par_iter_mut().try_for_each(|slot| move_slot(slot, timed, |reader| reader.go_to(recording, ordinal)))
    }

    /// Every table at the next hour of the run; false at its end.
    pub fn step(&mut self) -> io::Result<bool> {
        let recording: &Recording = &self.recording;
        let Some(run) = self.run else { return Err(invalid("Frames step before their first seek")) };
        if self.ordinal + 1 >= recording.run(run)?.ordinals {
            return Ok(false);
        }
        self.stepped = self.built == Some((run, self.ordinal));
        self.ordinal += 1;
        let timed = self.timed;
        self.slots.par_iter_mut().try_for_each(|slot| move_slot(slot, timed, |reader| reader.step(recording).map(drop)))?;
        Ok(true)
    }

    /// Every table at the hour of ordinal `ordinal` of run `run`: stepped there when it is the
    /// next hour, else sought ([`Self::seek`]).
    pub fn go_to(&mut self, run: u32, ordinal: u32) -> io::Result<()> {
        if self.run == Some(run) && ordinal == self.ordinal + 1 {
            self.step().map(drop)
        } else if self.run == Some(run) && ordinal == self.ordinal {
            Ok(())
        } else {
            self.seek(run, ordinal)
        }
    }

    /// The next frame is a keyframe, as at a block a reader starts from: each table it makes is a
    /// Snapshot and carries no change facts, so a block's hours are the same whichever blocks are
    /// read with it. Its views are the same views either way.
    pub fn keyframe(&mut self) {
        self.anew = true;
    }

    /// The hour's metrics, from its block's clock: every selected table the hour read named
    /// read, without a time ([`READ_TIME_NOT_RECORDED`]).
    pub fn metrics(&mut self) -> io::Result<Metrics> {
        let Some(run) = self.run else { return Ok(Metrics::default()) };
        let block = self.ordinal / BLOCK;
        if self.clock.as_ref().is_none_or(|(held, _)| *held != block) {
            self.clock = Some((block, self.recording.clock(run, block)?));
        }
        let hour = self.clock.as_ref().and_then(|(_, clock)| clock.get((self.ordinal % BLOCK) as usize)).copied().unwrap_or_default();
        let mut table_ns = [0; TABLE_COUNT];
        for slot in &self.slots {
            if slot.reader.as_ref().is_some_and(TableReader::was_read) {
                table_ns[slot.id.index()] = READ_TIME_NOT_RECORDED;
            }
        }
        Ok(Metrics { total_ns: hour.total_ns, table_ns, pool_bytes: hour.pool_bytes, dropped_frames: hour.dropped_frames })
    }

    /// The hour's frame, named `branch`: each selected table's view made where the hour changed
    /// it (patched after a step, else built from its cells), else the view built before.
    pub fn frame(&mut self, branch: u64) -> io::Result<DecodedFrame> {
        let hour = self.first_hour + self.ordinal as i32;
        let build = Build {
            hour,
            mode: self.mode,
            format: self.format,
            anew: self.anew,
            stepped: self.patching && self.stepped,
            facts: self.facts.then_some(self.stepped && !self.anew),
            timed: self.timed,
        };
        self.built = None;
        self.stepped = false;
        self.slots.par_iter_mut().try_for_each(|slot| build_slot(slot, &build))?;
        self.anew = false;
        self.built = self.run.map(|run| (run, self.ordinal));
        let mut tables: [SharedView; TABLE_COUNT] = std::array::from_fn(|_| not_selected());
        for slot in &self.slots {
            if let Some(view) = &slot.view {
                tables[slot.id.index()] = view.clone();
            }
        }
        Ok(DecodedFrame { branch, hour, tables, metrics: self.metrics()? })
    }

    /// The hour's frame and its recorded facts, named `branch`: a table built anew is a Snapshot
    /// of its rows, one the hour left alone Unchanged, one it changed a Delta whose rows carry
    /// their keys and the columns written (an inserted row every column) but no values.
    pub fn frame_recorded(&mut self, branch: u64) -> io::Result<(DecodedFrame, RecordedFrame)> {
        let keyframe = self.anew;
        let frame = self.frame(branch)?;
        let mut slots = [None; TABLE_COUNT];
        let mut updates = Vec::with_capacity(self.slots.len());
        for (at, slot) in self.slots.iter().enumerate() {
            slots[slot.id.index()] = Some(at);
            updates.push(slot.update.clone().ok_or_else(|| invalid("A table's update was not built"))?);
        }
        let recorded = RecordedFrame::from_updates(branch, frame.hour, keyframe, frame.metrics.clone(), Arc::new(slots), updates, Arc::new(TableUpdate::Unavailable { reason: String::new() }))?;
        Ok((frame, recorded))
    }

    /// The bytes its readers' cells hold.
    pub fn held_bytes(&self) -> usize {
        self.slots.iter().filter_map(|slot| slot.reader.as_ref()).map(TableReader::held_bytes).sum()
    }
}

/// Why a table failing with `error` holds no rows, when that is damage or a withheld table; any
/// other error goes on.
fn lost(error: io::Error) -> io::Result<String> {
    if error.kind() != io::ErrorKind::InvalidData {
        return Err(error);
    }
    Ok(error.to_string())
}

/// `slot`'s reader moved by `walk`, timed with `timed`; a reader failing on damage is lost by name.
fn move_slot(slot: &mut Slot, timed: bool, walk: impl FnOnce(&mut TableReader) -> io::Result<()>) -> io::Result<()> {
    let Some(reader) = slot.reader.as_mut() else { return Ok(()) };
    let started = timed.then(Instant::now);
    let walked = walk(reader);
    lap(&mut slot.spent.move_ns, started);
    if let Err(error) = walked {
        slot.lost = Some(lost(error)?);
        slot.reader = None;
        slot.view = None;
    }
    Ok(())
}

/// The nanoseconds since `started` added to `ns`, when timed.
fn lap(ns: &mut u64, started: Option<Instant>) {
    if let Some(started) = started {
        *ns += started.elapsed().as_nanos() as u64;
    }
}

/// How a frame's views are made at its hour.
struct Build {
    hour: i32,
    mode: CaptureMode,
    format: u16,
    /// A keyframe: every table is made, each a Snapshot.
    anew: bool,
    /// The hour was stepped to from the hour whose frame was built last, and views are patched.
    stepped: bool,
    /// With change facts: whether the hour was stepped to from the hour whose frame was built
    /// last and is no keyframe.
    facts: Option<bool>,
    timed: bool,
}

/// `slot`'s view and update at the hour: a table the hour left alone keeps its view; a complete
/// table stepped to from a complete view is that view patched ([`patch`]); any other is built from
/// its cells. With change facts, a complete view's field stamp, and its change facts when the
/// hour was stepped to from the view held.
fn build_slot(slot: &mut Slot, build: &Build) -> io::Result<()> {
    let Slot { id, reader, lost, view, update, spent } = slot;
    let id = *id;
    let Some(reader) = reader.as_ref() else {
        unavailable(view, update, lost.clone().unwrap_or_default());
        return Ok(());
    };
    let (hour, format) = (build.hour, build.format);
    let changes = reader.changes();
    if !build.anew && view.is_some() && changes.quiet() {
        *update = Some(Arc::new(TableUpdate::Unchanged { baseline: hour - 1 }));
        return Ok(());
    }
    let started = build.timed.then(Instant::now);
    let previous = view.as_ref().map(SharedView::held);
    let complete_before = matches!(previous, Some(TableView::Complete(_)));
    // A view withheld the hour before is no baseline for a delta.
    let withheld = !matches!(previous, None | Some(TableView::Complete(_) | TableView::Truncated { .. }));
    let snapshot = build.anew || changes.anew || view.is_none() || changes.status_changed || withheld;
    let edits = hour_edits(reader);
    let stamp = match (build.facts, view.as_ref()) {
        (Some(true), Some(previous)) if !changes.anew && complete_before => Some(edited(id, &edits, previous, hour, format)?),
        (Some(_), _) => Some(Stamp::new(hour)),
        (None, _) => None,
    };
    let mut patched = None;
    if build.stepped && !changes.anew && complete_before && reader.status() == Status::Complete {
        let held = view.take().ok_or_else(|| invalid("A patched table holds no view"))?;
        let deleted = present(held.held(), &edits.deleted);
        // A row that cannot be patched leaves the table to be built from every cell, which says why.
        if let Ok((rows, in_place)) = patch(id, reader, held, &edits, deleted.clone(), format) {
            spent.patched += 1;
            spent.in_place += u64::from(in_place);
            patched = Some((TableView::Complete(rows), deleted));
        }
    }
    let (made, deleted) = match patched {
        Some((made, deleted)) => (made, Some(deleted)),
        None => {
            spent.rebuilt += 1;
            match cells_view(id, &reader.view(), reader.status(), reader.read_rows(), build.mode, format) {
                Ok(made) => (made, None),
                Err(error) if Unsayable::of(&error).is_some() => {
                    lap(&mut spent.view_ns, started);
                    unavailable(view, update, error.to_string());
                    return Ok(());
                }
                Err(error) => return Err(error),
            }
        }
    };
    lap(&mut spent.view_ns, started);
    let started = build.timed.then(Instant::now);
    let before = update.take();
    let made_update = match (&made, snapshot) {
        (TableView::Complete(rows), true) => {
            let (rows, reused) = snapshot_rows(rows, before, &edits, deleted)?;
            spent.snapshots_patched += u64::from(reused);
            TableUpdate::Snapshot(rows)
        }
        (TableView::Complete(_), false) => delta(id, &edits, hour - 1, format),
        (TableView::Truncated { rows, missing }, _) => TableUpdate::Truncated { rows: rows.try_clone_validated()?, missing: *missing },
        (TableView::Unavailable { reason }, _) => TableUpdate::Unavailable { reason: reason.clone() },
        (TableView::NotRecorded { mode }, _) => TableUpdate::NotRecorded { mode: *mode },
    };
    let complete = matches!(made, TableView::Complete(_));
    let mut made = SharedView::from(made);
    if complete {
        made = made.with_sampled_at(reader.sampled_at());
        if let Some(stamp) = stamp {
            made = made.with_stamp(stamp);
        }
    }
    *view = Some(made);
    *update = Some(Arc::new(made_update));
    lap(&mut spent.update_ns, started);
    Ok(())
}

/// `view` unavailable for `reason`, kept when it already is for that reason, and its update.
fn unavailable(view: &mut Option<SharedView>, update: &mut Option<Arc<TableUpdate>>, reason: String) {
    if !matches!(view.as_ref().map(SharedView::held), Some(TableView::Unavailable { reason: held }) if *held == reason) {
        *view = Some(TableView::Unavailable { reason: reason.clone() }.into());
    }
    *update = Some(Arc::new(TableUpdate::Unavailable { reason }));
}

/// The keys of `deleted` that complete view `view` holds: a row inserted and deleted within the
/// hour never was.
fn present(view: &TableView, deleted: &[u128]) -> Vec<u128> {
    let TableView::Complete(rows) = view else { return Vec::new() };
    deleted.iter().copied().filter(|&key| rows.contains_key(key)).collect()
}

/// Complete view `held`, of the hour before the one `reader` stepped to last, at that hour: each
/// row the hour wrote given the fields written from its cells, each row it inserted made from
/// its cells, the keys of `deleted` removed; in its own rows when no other frame holds them (true),
/// else on a copy.
fn patch(id: TableId, reader: &TableReader, held: SharedView, edits: &Edits, deleted: Vec<u128>, format: u16) -> io::Result<(TableRows, bool)> {
    if edits.written.iter().any(|&(_, _, fields)| fields == 0) {
        return Err(invalid("A written column has no field bit"));
    }
    let (mut rows, in_place) = match held.try_unwrap() {
        Ok(TableView::Complete(rows)) => (rows, true),
        Ok(_) => return Err(invalid("A patched view is not complete")),
        Err(shared) => match shared.held() {
            TableView::Complete(rows) => (rows.try_clone_validated()?, false),
            _ => return Err(invalid("A patched view is not complete")),
        },
    };
    let cells = reader.view();
    if !edits.written.is_empty() {
        let patches = engine_allocation::collect_vec(edits.written.iter().map(|&(key, _, fields)| (key, fields)))?;
        rows.patch_from_wire(&patches, &mut |at, column| field(&cells, edits.written[at].1, column, format))?;
    }
    let inserted = cells_rows(id, &cells, &edits.inserted, format)?;
    let rows = rows.apply_changes(RowChanges::new(inserted, deleted))?;
    Ok((rows, in_place))
}

/// The rows of a Snapshot of complete rows `rows`: when they were patched from the view before
/// with `edits` and its `deleted` keys, and `before` is that view's Snapshot and no recorded frame
/// holds it, that Snapshot patched the same way (true); else a copy.
fn snapshot_rows(rows: &TableRows, before: Option<Arc<TableUpdate>>, edits: &Edits, deleted: Option<Vec<u128>>) -> io::Result<(TableRows, bool)> {
    if let (Some(deleted), Some(before)) = (deleted, before)
        && let Ok(TableUpdate::Snapshot(held)) = Arc::try_unwrap(before)
    {
        let keys = engine_allocation::collect_vec(edits.rows().map(|(key, _)| key))?;
        return Ok((held.apply_changes(rows.rows_at(&keys, deleted)?)?, true));
    }
    Ok((rows.try_clone_validated()?, false))
}

/// The stamp of the complete view the hour `edits` describes makes of complete view `previous`:
/// each row the hour inserted or wrote, with the fields written (every field for a row inserted,
/// and those its recorded format leaves unrecorded), and each key of `previous` it deleted.
fn edited(id: TableId, edits: &Edits, previous: &SharedView, hour: i32, format: u16) -> io::Result<Stamp> {
    let TableView::Complete(rows) = previous.held() else { return Err(invalid("A table's change facts follow a view that is not complete")) };
    let recorded = id.last_change().fields(id, format);
    let unrecorded = if recorded >= u64::BITS as usize { 0 } else { id.full_mask() & (u64::MAX << recorded) };
    let changed = edits.masks(recorded).map(|(key, mask)| RowChange { key, fields: mask | unrecorded, inserted: !rows.contains_key(key) }).collect();
    let deleted = present(previous.held(), &edits.deleted);
    Ok(previous.edited(changed, deleted, hour))
}

/// The Delta fact of a changed complete table: its inserted rows (every column) and written
/// rows (the columns written), in key order, and its deleted keys.
fn delta(id: TableId, edits: &Edits, baseline: i32, format: u16) -> TableUpdate {
    let fields = id.last_change().fields(id, format);
    let changed = edits.masks(fields).map(|(key, mask)| DeltaRow { format, key, mask, recorded_fields: fields, values: Vec::new() }).collect();
    TableUpdate::Delta { baseline, changed, deleted: edits.deleted.clone() }
}

/// What the hour a reader stepped to last did to its table, each list in key order: the rows it
/// inserted (key and slot), the older rows it wrote (key, slot and the columns written), and the
/// keys it deleted and did not insert again.
struct Edits {
    inserted: Vec<(u128, u32)>,
    written: Vec<(u128, u32, u64)>,
    deleted: Vec<u128>,
}

impl Edits {
    /// Each row inserted or written, in key order, and whether it was inserted.
    fn rows(&self) -> impl Iterator<Item = (u128, bool)> + '_ {
        let mut inserted = self.inserted.iter().map(|&(key, _)| key).peekable();
        let mut written = self.written.iter().map(|&(key, _, _)| key).peekable();
        std::iter::from_fn(move || match (inserted.peek(), written.peek()) {
            (Some(&new), Some(&old)) if new <= old => inserted.next().map(|key| (key, true)),
            (Some(_), Some(_)) | (None, Some(_)) => written.next().map(|key| (key, false)),
            (Some(_), None) => inserted.next().map(|key| (key, true)),
            (None, None) => None,
        })
    }

    /// Each row inserted or written, in key order, with the fields written of a table of
    /// `fields` recorded fields: every one for a row inserted.
    fn masks(&self, fields: usize) -> impl Iterator<Item = (u128, u64)> + '_ {
        let mut written = self.written.iter();
        self.rows().map(move |(key, inserted)| {
            if inserted {
                return (key, full_mask(fields));
            }
            let mask = written.next().map_or(0, |&(_, _, mask)| mask);
            (key, mask)
        })
    }
}

/// What the hour `reader` stepped to last did to its table ([`Edits`]): from its changes, with
/// nothing compared.
fn hour_edits(reader: &TableReader) -> Edits {
    let changes = reader.changes();
    let cells = reader.cells();
    let mut inserted: Vec<(u128, u32)> = changes.inserted.iter().filter_map(|&slot| Some((cells.key(slot)?, slot))).collect();
    inserted.sort_unstable();
    let mut written: Vec<(u128, u32, u64)> = Vec::new();
    for (at, slots) in changes.written.iter().enumerate() {
        let column = reader.columns()[at];
        let bit = if column < 64 { 1_u64 << column } else { 0 };
        for &slot in slots {
            if changes.inserted.binary_search(&slot).is_ok() {
                continue;
            }
            if let Some(key) = cells.key(slot) {
                written.push((key, slot, bit));
            }
        }
    }
    written.sort_unstable_by_key(|&(key, _, _)| key);
    written.dedup_by(|later, kept| {
        let same = later.0 == kept.0;
        if same {
            kept.2 |= later.2;
        }
        same
    });
    let mut deleted: Vec<u128> = changes.deleted.iter().map(|&(_, key)| key).filter(|key| inserted.binary_search_by_key(key, |&(held, _)| held).is_err()).collect();
    deleted.sort_unstable();
    deleted.dedup();
    Edits { inserted, written, deleted }
}

/// Whether `status` lets a reader count a table's rows.
pub fn available(status: Status) -> bool {
    matches!(status, Status::Complete | Status::Truncated { .. })
}
