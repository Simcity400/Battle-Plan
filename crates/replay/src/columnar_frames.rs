//! A columnar recording's tables as the row decoder's frames.
//!
//! [`State`] holds each table's rows as typed columns at the dense slots the writer assigned,
//! and names the slots each hour changed, deleted and inserted. [`Views`] keeps the selected
//! tables' views as [`crate::frame::FrameDecoder`] reconstructs them from the row recording:
//! a table the hour leaves alone keeps its allocation; a complete table the hour changes gets
//! a new view of the hour before's rows: its inserted rows built from their columns
//! ([`crate::rows::RowCodec::from_wire`]) and its deleted keys removed, then each changed or
//! filled row given the columns the hour wrote ([`crate::rows::RowCodec::patch_from_wire`]);
//! any other gets a view built from the rows the walker holds. A complete view is sampled at
//! the walker's sample hour. Hours can be noted as the walker applies them and the views built
//! only for the hours asked ([`Views::note`], [`Views::build`]): a view built after several
//! hours holds what the hours did since it was built last.
//!
//! The manifest keeps which tables an hour read, not how long each took, so a read table's
//! [`Metrics::table_ns`] is [`READ_TIME_NOT_RECORDED`], as for recordings that keep no timings.

use crate::{
    container::Parallel,
    format::FormatVersion,
    frame::{
        DecodedFrame, Metrics, READ_TIME_NOT_RECORDED, RecordedFrame, RowChange, SharedView, Stamp,
        TableUpdate, TableView, change_credit, not_selected,
    },
    rows::{DeltaRow, Field, List, Opaque, RowChanges, TABLE_COUNT, TableId, TableRows, full_mask},
    scope::CaptureMode,
};
use engine_allocation::credit::{Credit, MemoryBudget};
#[cfg(test)]
use replay_columnar::{
    envelope::{self, Extent, IndexPage, Kind, Root, RunDirectory},
    framing,
};
use replay_columnar::{
    manifest::{Manifest, Status},
    state::{Cell, Kind as Column, Table},
    walk::{Dictionary, State},
};
use std::{
    io,
    mem::size_of,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

mod input;
mod reader;
pub use input::{Budget, Limits};
pub(crate) use reader::Reads;
pub use reader::{Damage, DamageScope, Recording, Run, Timings, Walker};

#[cfg(test)]
pub(crate) mod fixtures;
#[cfg(any(test, feature = "test-support"))]
pub mod synthetic;
#[cfg(test)]
mod tests;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// Every schema table's column kinds, in the recording's table order.
pub fn schema() -> io::Result<Vec<Vec<Column>>> {
    let mut schema = engine_allocation::vec(TableId::ALL.len())?;
    for table in TableId::ALL {
        schema.push(crate::native_columns::columns(table.schema())?);
    }
    Ok(schema)
}

/// `slot`'s cell in `column` in the wire form a row's field codec reads: a text cell's
/// dictionary bytes, an opaque cell as its payload encoded at `format`.
pub(crate) fn wire<'a>(
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

/// `table`'s rows at `slots`, sorted by key, built from their columns.
fn rows_at(
    id: TableId,
    table: &Table,
    dictionary: &Dictionary,
    slots: &mut Vec<(u128, u32)>,
    keys: &mut Vec<u128>,
    format: u16,
) -> io::Result<TableRows> {
    slots.sort_unstable();
    slots.dedup();
    keys.clear();
    engine_allocation::reserve(keys, slots.len())?;
    keys.extend(slots.iter().map(|&(key, _)| key));
    let slots = &*slots;
    id.rows_from_wire(keys, &mut |row, column| {
        wire(table, dictionary, slots[row].1, column, format)
    })
}

/// An hour's metrics as its manifest keeps them: a read table's time is
/// [`READ_TIME_NOT_RECORDED`].
pub fn metrics(manifest: &Manifest<'_>) -> Metrics {
    let mut table_ns = [0; TABLE_COUNT];
    for (index, time) in table_ns.iter_mut().enumerate() {
        if manifest
            .read
            .get(index / 64)
            .is_some_and(|word| word >> (index % 64) & 1 != 0)
        {
            *time = READ_TIME_NOT_RECORDED;
        }
    }
    Metrics {
        total_ns: manifest.total_ns,
        table_ns,
        pool_bytes: manifest.pool_bytes,
        dropped_frames: manifest.dropped_frames,
    }
}

/// The bytes `view`'s rows, or its reason, hold.
fn view_bytes(view: &TableView) -> usize {
    match view {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.heap_bytes(),
        TableView::Unavailable { reason } => reason.capacity(),
        TableView::NotRecorded { .. } => 0,
    }
}

/// The bytes `frame`'s views hold, each counted though another frame shares it.
pub fn frame_heap_bytes(frame: &DecodedFrame) -> usize {
    frame.tables.iter().map(|view| view_bytes(view)).sum()
}

/// One table's hours noted since its view was built ([`Views::note`]).
#[derive(Default)]
struct Pending {
    hours: u32,
    /// The first noted hour's status, and whether a later noted hour had another.
    status: Option<Status>,
    varied: bool,
    /// Some noted hour had a section of the table.
    sectioned: bool,
    /// The view is built anew, whatever the hours noted.
    anew: bool,
    /// Each noted section's edits, in hour order, while the table stays complete.
    edits: Vec<(u128, Edit)>,
    /// How many edits the last compaction left.
    compacted: usize,
}

/// What a section did to a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edit {
    Deleted,
    /// Inserted, every column with it.
    Inserted,
    /// These columns written.
    Written(u64),
}

/// `edits`, sorted by key and in hour order within one, as what they leave each key: set
/// whole (inserted since its last deletion), removed, or with the columns written since.
fn fold(
    edits: &[(u128, Edit)],
    mut each: impl FnMut(u128, Edit) -> io::Result<()>,
) -> io::Result<()> {
    let mut at = 0;
    while let Some(&(key, _)) = edits.get(at) {
        let (mut deleted, mut inserted, mut mask) = (false, false, 0);
        while let Some(&(_, edit)) = edits.get(at).filter(|&&(held, _)| held == key) {
            match edit {
                Edit::Deleted => {
                    deleted = true;
                    inserted = false;
                    mask = 0;
                }
                Edit::Inserted => {
                    inserted = true;
                    mask = 0;
                }
                Edit::Written(columns) => mask |= columns,
            }
            at += 1;
        }
        if inserted {
            each(key, Edit::Inserted)?;
        } else if deleted {
            each(key, Edit::Deleted)?;
        } else if mask != 0 {
            each(key, Edit::Written(mask))?;
        }
    }
    Ok(())
}

/// `edits` as [`fold`] leaves them, one per key, in key order, less deletions of keys
/// `previous` does not hold: they fold the same with any edits after them.
fn compact(edits: &mut Vec<(u128, Edit)>, previous: &TableRows) -> io::Result<()> {
    edits.sort_by_key(|&(key, _)| key);
    let mut folded = engine_allocation::vec(edits.len())?;
    fold(edits, |key, edit| {
        if edit != Edit::Deleted || previous.contains_key(key) {
            folded.push((key, edit));
        }
        Ok(())
    })?;
    *edits = folded;
    Ok(())
}

/// What [`Views::build`] took for each kind of view, when its builds are timed.
#[derive(Clone, Debug, Default)]
pub struct BuildTimes {
    /// Tables whose views were carried, patched, built anew and given their status, and the
    /// time they took; tables built anew share one parallel pass.
    pub carried: (u32, Duration),
    pub patched: (u32, Duration),
    pub anew: (u32, Duration),
    pub status: (u32, Duration),
    /// Each selected table's time, in selection order, the parallel pass aside.
    pub tables: Vec<(TableId, Duration)>,
}

/// What [`Views::build`] makes of a selected table.
#[derive(Clone, Copy)]
enum Plan {
    /// Its view's allocation, sampled anew.
    Carry,
    /// Its complete view with its folded edits: `changed` rows and `deleted` keys.
    Patch { changed: usize, deleted: usize },
    /// A complete view of every row the walker holds.
    Anew,
    /// A view of its status other than complete.
    Status,
}

/// `mask`'s fields of `id` and the fields a format of `recorded` fields leaves unrecorded, as
/// a decoder's change facts count them.
fn change_fields(id: TableId, mask: u64, recorded: usize) -> u64 {
    let missing = if recorded >= u64::BITS as usize {
        0
    } else {
        u64::MAX << recorded
    };
    mask | (id.full_mask() & missing)
}

/// The selected tables' views, carried from hour to hour as the walker advances. Hours are
/// noted as the walker applies them ([`Self::note`]) and views built only when a frame is
/// asked for ([`Self::build`]), from every hour noted since the views were built last.
pub struct Views {
    selected: Vec<TableId>,
    views: Vec<Option<SharedView>>,
    /// The format opaque cells were encoded at.
    format: u16,
    mode: CaptureMode,
    slots: Vec<(u128, u32)>,
    keys: Vec<u128>,
    deleted: Vec<u128>,
    /// Each patched row's key, slot and the columns written to it.
    patches: Vec<(u128, u32, u64)>,
    masks: Vec<(u128, u64)>,
    /// Whether [`Self::advance_recorded`] asked for the hour's change facts, and each selected
    /// table's, in selection order.
    record: bool,
    facts: Vec<TableUpdate>,
    /// Each selected table's hours noted since its view was built, in selection order.
    pending: Vec<Pending>,
    /// The hour the views were built for last.
    built: Option<i32>,
    /// The owner the views' change facts and field stamps are charged to, when they carry them.
    changes: Option<MemoryBudget>,
    /// The last build's times, when builds are timed.
    timed: bool,
    times: BuildTimes,
}

impl Views {
    /// The bytes its views and buffers hold, the edits of the hours noted among them.
    pub fn heap_bytes(&self) -> usize {
        self.views
            .iter()
            .flatten()
            .map(|view| view_bytes(view))
            .sum::<usize>()
            + self.slots.capacity() * size_of::<(u128, u32)>()
            + (self.keys.capacity() + self.deleted.capacity()) * size_of::<u128>()
            + self.patches.capacity() * size_of::<(u128, u32, u64)>()
            + self.masks.capacity() * size_of::<(u128, u64)>()
            + self.facts.capacity() * size_of::<TableUpdate>()
            + self.pending.capacity() * size_of::<Pending>()
            + self
                .pending
                .iter()
                .map(|pending| pending.edits.capacity() * size_of::<(u128, Edit)>())
                .sum::<usize>()
            + self.times.tables.capacity() * size_of::<(TableId, Duration)>()
    }

    /// Views of `tables` and Campaign, as a decoder of them reconstructs them.
    pub fn new(tables: &[TableId], format: FormatVersion, mode: CaptureMode) -> io::Result<Self> {
        let mut chosen = [false; TABLE_COUNT];
        chosen[TableId::Campaign.index()] = true;
        for table in tables {
            chosen[table.index()] = true;
        }
        let selected = engine_allocation::collect_vec(
            TableId::ALL
                .into_iter()
                .filter(|table| chosen[table.index()]),
        )?;
        Ok(Self {
            pending: engine_allocation::collect_vec(selected.iter().map(|_| Pending::default()))?,
            selected,
            views: engine_allocation::collect_vec((0..TABLE_COUNT).map(|_| None))?,
            format: format.version(),
            mode,
            slots: Vec::new(),
            keys: Vec::new(),
            deleted: Vec::new(),
            patches: Vec::new(),
            masks: Vec::new(),
            record: false,
            facts: Vec::new(),
            built: None,
            changes: None,
            timed: false,
            times: BuildTimes::default(),
        })
    }

    /// Complete views carrying change facts and field stamps, as a decoder with change facts
    /// makes them, charged to `budget`.
    pub fn with_change_facts(mut self, budget: MemoryBudget) -> Self {
        self.changes = Some(budget);
        self
    }

    /// The tables viewed, Campaign among them, in table order.
    pub fn selected(&self) -> &[TableId] {
        &self.selected
    }

    /// Times each build from now on, or stops.
    pub fn time_builds(&mut self, timed: bool) {
        self.timed = timed;
    }

    /// What the last build took, when builds are timed.
    pub fn times(&self) -> &BuildTimes {
        &self.times
    }

    /// The hour the views were built for last; none before the first build or after a
    /// [`Self::reset`].
    pub fn built(&self) -> Option<i32> {
        self.built
    }

    /// Forgets the views and the hours noted: the next views are built anew, as at a restart.
    pub fn reset(&mut self) {
        for pending in &mut self.pending {
            pending.edits.clear();
            *pending = Pending {
                anew: true,
                edits: std::mem::take(&mut pending.edits),
                ..Pending::default()
            };
        }
        self.built = None;
    }

    /// [`Self::advance`], with the hour's change facts in the row recording's shape: a table
    /// left alone is Unchanged, one the hour changed a Delta whose rows carry their keys and
    /// the columns the hour wrote (an inserted row every column) but no values, which readers
    /// take from the frame; a view built anew is a Snapshot of its rows. Tables not selected
    /// are unavailable, as in the row recording's selected reads.
    pub fn advance_recorded(
        &mut self,
        state: &State,
        manifest: &Manifest<'_>,
        restart: bool,
    ) -> io::Result<(DecodedFrame, RecordedFrame)> {
        self.record = true;
        self.facts.clear();
        engine_allocation::reserve(&mut self.facts, self.selected.len())?;
        let frame = self.advance(state, manifest, restart);
        self.record = false;
        let frame = frame?;
        let mut slots = [None; TABLE_COUNT];
        for (at, table) in self.selected.iter().enumerate() {
            slots[table.index()] = Some(at);
        }
        let updates = engine_allocation::collect_vec(self.facts.drain(..).map(Arc::new))?;
        let recorded = RecordedFrame::from_updates(
            frame.branch,
            frame.hour,
            restart,
            frame.metrics.clone(),
            Arc::new(slots),
            updates,
            Arc::new(TableUpdate::Unavailable {
                reason: String::new(),
            }),
        )?;
        Ok((frame, recorded))
    }

    /// Keeps `update` as the hour's fact for the table being viewed, when facts are asked.
    fn fact(&mut self, update: impl FnOnce() -> io::Result<TableUpdate>) -> io::Result<()> {
        if self.record {
            engine_allocation::reserve(&mut self.facts, 1)?;
            self.facts.push(update()?);
        }
        Ok(())
    }

    /// The Delta fact of a changed complete table: its inserted rows (every column), then
    /// its patched rows, in key order, and its deleted keys.
    fn delta(&self, id: TableId, baseline: i32) -> io::Result<TableUpdate> {
        // The fields the recording's format wrote, as its own deltas count them.
        let fields = id.last_change().fields(id, self.format);
        let row = |key: u128, mask: u64| DeltaRow {
            format: self.format,
            key,
            mask,
            recorded_fields: fields,
            values: Vec::new(),
        };
        let mut changed = engine_allocation::vec(self.keys.len() + self.patches.len())?;
        changed.extend(self.keys.iter().map(|&key| row(key, full_mask(fields))));
        changed.extend(self.patches.iter().map(|&(key, _, mask)| row(key, mask)));
        changed.sort_unstable_by_key(|row: &DeltaRow| row.key);
        Ok(TableUpdate::Delta {
            baseline,
            changed,
            deleted: engine_allocation::copy_vec(&self.deleted)?,
        })
    }

    /// The frame of `manifest`'s hour, once `state` applied it. A `restart` (a unit the walker
    /// started at, or a keyframe) has no hour before: every view is built anew.
    pub fn advance(
        &mut self,
        state: &State,
        manifest: &Manifest<'_>,
        restart: bool,
    ) -> io::Result<DecodedFrame> {
        self.note(state, manifest, restart)?;
        self.build(
            state,
            (manifest.branch, manifest.hour),
            metrics(manifest),
            None,
        )
    }

    /// The frame of `state` as a restart restored it, before the walk's first hour, dated
    /// `hour` of `branch`: every view is built anew, and no table reads as read.
    pub fn restored(&mut self, state: &State, branch: u64, hour: i32) -> io::Result<DecodedFrame> {
        self.reset();
        self.build(state, (branch, hour), Metrics::default(), None)
    }

    /// Takes in `manifest`'s hour, once `state` applied it, building no view: the next views
    /// built follow from every hour noted since the last. A `restart` (a unit the walker
    /// started at, or a keyframe) has no hour before: every view is built anew.
    pub fn note(
        &mut self,
        state: &State,
        manifest: &Manifest<'_>,
        restart: bool,
    ) -> io::Result<()> {
        let mut present = [false; TABLE_COUNT];
        for section in &manifest.sections {
            *present
                .get_mut(section.table)
                .ok_or_else(|| invalid("A section names a table past the schema"))? = true;
        }
        if restart {
            self.reset();
        }
        for (at, pending) in self.pending.iter_mut().enumerate() {
            let id = self.selected[at];
            let index = id.index();
            if state.skips(index) {
                return Err(invalid(format!("{id:?} is not read by this walk")));
            }
            let status = state
                .status(index)
                .ok_or_else(|| invalid("The walker holds no such table"))?;
            pending.hours += 1;
            match pending.status {
                None => pending.status = Some(status),
                Some(first) => pending.varied |= first != status,
            }
            if !present[index] {
                continue;
            }
            pending.sectioned = true;
            if pending.anew || pending.varied || status != Status::Complete {
                pending.edits.clear();
                continue;
            }
            let changes = state
                .changes(index)
                .ok_or_else(|| invalid("The walker holds no such table"))?;
            let table = state
                .table(index)
                .ok_or_else(|| invalid("The walker holds no such table"))?;
            if changes.masks.len() != changes.changed.len()
                || changes.fill_masks.len() != changes.fill_slots.len()
            {
                return Err(invalid("A table's change masks miss its changed slots"));
            }
            engine_allocation::reserve(
                &mut pending.edits,
                changes.deleted_keys.len()
                    + changes.inserted.len()
                    + changes.changed.len()
                    + changes.fill_slots.len(),
            )?;
            // A key deleted and inserted again in one hour is set, not removed.
            pending
                .edits
                .extend(changes.deleted_keys.iter().map(|&key| (key, Edit::Deleted)));
            for &slot in &changes.inserted {
                let key = table
                    .key(slot)
                    .ok_or_else(|| invalid("An inserted slot holds no row"))?;
                pending.edits.push((key, Edit::Inserted));
            }
            let written = changes
                .changed
                .iter()
                .zip(&changes.masks)
                .chain(changes.fill_slots.iter().zip(&changes.fill_masks));
            for (&slot, &mask) in written.filter(|(_, mask)| **mask != 0) {
                let key = table
                    .key(slot)
                    .ok_or_else(|| invalid("A changed slot holds no row"))?;
                pending.edits.push((key, Edit::Written(mask)));
            }
            // Edits stay within twice the table's slots and what the last compaction left.
            if pending.edits.len() > table.slots().max(2 * pending.compacted) {
                match self.views[index].as_deref() {
                    Some(TableView::Complete(previous)) => {
                        compact(&mut pending.edits, previous)?;
                        pending.compacted = pending.edits.len();
                    }
                    _ => pending.edits.clear(),
                }
            }
        }
        Ok(())
    }

    /// The frame of `hour` of `branch` from every hour noted since the views were built last:
    /// a table no noted hour touched keeps its allocation; a complete table that stayed
    /// complete gets the noted sections' edits, its inserted rows built from their columns,
    /// its deleted keys removed and each written row given the columns written since; any
    /// other gets a view built anew from the rows the walker holds, complete ones on
    /// `parallel`'s workers when given. Views carrying change facts are admitted their credit
    /// before any view changes: a refusal leaves the views and the hours noted as they were.
    pub fn build(
        &mut self,
        state: &State,
        (branch, hour): (u64, i32),
        metrics: Metrics,
        parallel: Option<&Parallel<'_>>,
    ) -> io::Result<DecodedFrame> {
        let mut plans = engine_allocation::vec(self.selected.len())?;
        let mut required = 0_usize;
        for (at, pending) in self.pending.iter_mut().enumerate() {
            let id = self.selected[at];
            let index = id.index();
            if state.skips(index) {
                return Err(invalid(format!("{id:?} is not read by this walk")));
            }
            let status = state
                .status(index)
                .ok_or_else(|| invalid("The walker holds no such table"))?;
            let constant =
                pending.hours == 0 || (!pending.varied && pending.status == Some(status));
            let previous = self.views[index]
                .as_ref()
                .filter(|_| !pending.anew && constant);
            let plan = match (status, previous.map(|view| &**view)) {
                (Status::Complete, Some(TableView::Complete(rows))) => {
                    if pending.sectioned {
                        pending.edits.sort_by_key(|&(key, _)| key);
                        let (mut changed, mut deleted) = (0, 0);
                        fold(&pending.edits, |key, edit| {
                            match edit {
                                Edit::Deleted => deleted += usize::from(rows.contains_key(key)),
                                Edit::Inserted | Edit::Written(_) => changed += 1,
                            }
                            Ok(())
                        })?;
                        Plan::Patch { changed, deleted }
                    } else {
                        Plan::Carry
                    }
                }
                (Status::Complete, _) => Plan::Anew,
                _ => Plan::Status,
            };
            if self.changes.is_some() {
                required = engine_allocation::add(
                    required,
                    match plan {
                        Plan::Anew => Stamp::BYTES,
                        Plan::Patch { changed, deleted } => Stamp::edit_bytes(changed, deleted)?,
                        Plan::Carry | Plan::Status => 0,
                    },
                )?;
            }
            plans.push(plan);
        }
        let mut credit = match &self.changes {
            Some(budget) => {
                // Facts a replaced view no longer needs go back before more are asked for.
                for (at, plan) in plans.iter().enumerate() {
                    if !matches!(plan, Plan::Carry)
                        && let Some(view) = self.views[self.selected[at].index()].as_mut()
                    {
                        view.reclaim_changes();
                    }
                }
                Some(change_credit(budget, required, None)?)
            }
            None => None,
        };
        let anew = engine_allocation::collect_vec(
            plans
                .iter()
                .zip(&self.selected)
                .filter(|(plan, _)| matches!(plan, Plan::Anew))
                .map(|(_, &id)| id),
        )?;
        if self.timed {
            self.times.tables.clear();
            engine_allocation::reserve(&mut self.times.tables, self.selected.len())?;
            let tables = std::mem::take(&mut self.times.tables);
            self.times = BuildTimes {
                tables,
                ..BuildTimes::default()
            };
        }
        let started = self.timed.then(Instant::now);
        let mut anew = self.rows_anew(state, &anew, parallel)?.into_iter();
        if let Some(started) = started {
            self.times.anew.1 = started.elapsed();
        }
        // A continuing walk's deltas apply to the hour the views were built for.
        let baseline = self.built.unwrap_or(hour.saturating_sub(1));
        let mut tables: [SharedView; TABLE_COUNT] = std::array::from_fn(|_| not_selected());
        for (at, plan) in plans.into_iter().enumerate() {
            let id = self.selected[at];
            let index = id.index();
            let started = self.timed.then(Instant::now);
            let mut pending = std::mem::take(&mut self.pending[at]);
            let previous = self.views[index].take().filter(|_| !pending.anew);
            let view = self.view(
                state,
                id,
                plan,
                &pending,
                previous,
                &mut anew,
                (baseline, hour),
                credit.as_mut(),
            );
            pending.edits.clear();
            self.pending[at] = Pending {
                edits: pending.edits,
                ..Pending::default()
            };
            let view = view?;
            tables[index] = view.clone();
            self.views[index] = Some(view);
            if let Some(started) = started {
                let took = started.elapsed();
                let tally = match plan {
                    Plan::Carry => &mut self.times.carried,
                    Plan::Patch { .. } => &mut self.times.patched,
                    Plan::Anew => &mut self.times.anew,
                    Plan::Status => &mut self.times.status,
                };
                tally.0 += 1;
                tally.1 += took;
                self.times.tables.push((id, took));
            }
        }
        self.built = Some(hour);
        Ok(DecodedFrame {
            branch,
            hour,
            tables,
            metrics,
        })
    }

    /// Every row the walker holds of each of `tables`, built from its columns: on `parallel`'s
    /// workers when given, the largest tables first, each table on one worker.
    fn rows_anew(
        &mut self,
        state: &State,
        tables: &[TableId],
        parallel: Option<&Parallel<'_>>,
    ) -> io::Result<Vec<TableRows>> {
        let format = self.format;
        let build = |id: TableId,
                     slots: &mut Vec<(u128, u32)>,
                     keys: &mut Vec<u128>|
         -> io::Result<TableRows> {
            let table = state
                .table(id.index())
                .ok_or_else(|| invalid("The walker holds no such table"))?;
            slots.clear();
            engine_allocation::reserve(slots, table.live().count())?;
            for slot in table.live().ones() {
                let key = table
                    .key(slot)
                    .ok_or_else(|| invalid("A live slot holds no key"))?;
                slots.push((key, slot));
            }
            rows_at(id, table, state.dictionary(), slots, keys, format)
        };
        let workers = parallel.map_or(1, |parallel| parallel.workers.min(tables.len()));
        let Some(parallel) = parallel.filter(|_| workers > 1) else {
            let (slots, keys) = (&mut self.slots, &mut self.keys);
            return tables.iter().map(|&id| build(id, slots, keys)).collect();
        };
        let mut order = engine_allocation::collect_vec(0..tables.len())?;
        order.sort_by_key(|&at| {
            std::cmp::Reverse(
                state
                    .table(tables[at].index())
                    .map_or(0, |table| table.live().count()),
            )
        });
        let next = AtomicUsize::new(0);
        let built = engine_allocation::collect_vec(
            (0..tables.len()).map(|_| Mutex::new(None::<io::Result<TableRows>>)),
        )?;
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    (parallel.on_worker)();
                    let (mut slots, mut keys) = (Vec::new(), Vec::new());
                    while let Some(&at) = order.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let rows = build(tables[at], &mut slots, &mut keys);
                        *built[at].lock().unwrap_or_else(PoisonError::into_inner) = Some(rows);
                    }
                });
            }
        });
        built
            .into_iter()
            .map(|cell| {
                cell.into_inner()
                    .unwrap_or_else(PoisonError::into_inner)
                    .unwrap_or_else(|| Err(invalid("A worker left a table unbuilt")))
            })
            .collect()
    }

    /// Table `id`'s view as `plan` makes it from `previous`, its view as built last unless it
    /// is built anew, and `pending`, its hours noted since; `anew` holds the rows of the
    /// tables built anew, in selection order.
    #[allow(clippy::too_many_arguments)]
    fn view(
        &mut self,
        state: &State,
        id: TableId,
        plan: Plan,
        pending: &Pending,
        previous: Option<SharedView>,
        anew: &mut impl Iterator<Item = TableRows>,
        (baseline, hour): (i32, i32),
        credit: Option<&mut Credit>,
    ) -> io::Result<SharedView> {
        let index = id.index();
        let status = state
            .status(index)
            .ok_or_else(|| invalid("The walker holds no such table"))?;
        let sampled = state.sampled_at(index);
        let table = state
            .table(index)
            .ok_or_else(|| invalid("The walker holds no such table"))?;
        let dictionary = state.dictionary();
        match plan {
            Plan::Carry => {
                self.fact(|| Ok(TableUpdate::Unchanged { baseline }))?;
                let previous = previous.ok_or_else(|| invalid("A carried table has no view"))?;
                Ok(previous.with_sampled_at(sampled))
            }
            Plan::Anew => {
                let rows = anew
                    .next()
                    .ok_or_else(|| invalid("A table built anew has no rows"))?;
                self.fact(|| Ok(TableUpdate::Snapshot(rows.try_clone_validated()?)))?;
                let view = SharedView::from(TableView::Complete(rows)).with_sampled_at(sampled);
                Ok(match credit {
                    Some(credit) => {
                        view.with_stamp(Stamp::new(hour, credit.split_off(Stamp::BYTES))?)
                    }
                    None => view,
                })
            }
            Plan::Status => {
                // An hour since the views were built under another status forbids a carry.
                let previous = previous.filter(|_| !pending.varied);
                match status {
                    Status::NotRecorded => {
                        let mode = self.mode;
                        self.fact(|| Ok(TableUpdate::NotRecorded { mode }))?;
                        Ok(match previous {
                            Some(view) if matches!(*view, TableView::NotRecorded { .. }) => view,
                            _ => TableView::NotRecorded { mode }.into(),
                        })
                    }
                    Status::Unavailable { reason } => {
                        let reason = dictionary
                            .get(reason)
                            .ok_or_else(|| invalid("A reason names a missing dictionary entry"))?;
                        let reason = std::str::from_utf8(reason)
                            .map_err(|_| invalid("A table's reason is not text"))?;
                        self.fact(|| {
                            Ok(TableUpdate::Unavailable {
                                reason: engine_allocation::string(reason)?,
                            })
                        })?;
                        Ok(match previous {
                            Some(view) if matches!(&*view, TableView::Unavailable { reason: held } if held == reason) => {
                                view
                            }
                            _ => TableView::Unavailable {
                                reason: engine_allocation::string(reason)?,
                            }
                            .into(),
                        })
                    }
                    Status::Truncated { missing } => {
                        let view = state
                            .truncated_view(index)
                            .ok_or_else(|| invalid("A truncated table names no rows it read"))?;
                        self.slots.clear();
                        engine_allocation::reserve(&mut self.slots, view.len())?;
                        for &slot in view {
                            let key = table
                                .key(slot)
                                .ok_or_else(|| invalid("A truncated view names an empty slot"))?;
                            self.slots.push((key, slot));
                        }
                        let rows = rows_at(
                            id,
                            table,
                            dictionary,
                            &mut self.slots,
                            &mut self.keys,
                            self.format,
                        )?;
                        self.fact(|| {
                            Ok(TableUpdate::Truncated {
                                rows: rows.try_clone_validated()?,
                                missing,
                            })
                        })?;
                        Ok(TableView::Truncated { rows, missing }.into())
                    }
                    Status::Complete => Err(invalid("A complete table has no status view")),
                }
            }
            Plan::Patch { changed, deleted } => {
                let previous = previous.ok_or_else(|| invalid("A patched table has no view"))?;
                let TableView::Complete(rows) = &*previous else {
                    return Err(invalid("A patched table's view is not complete"));
                };
                self.slots.clear();
                self.deleted.clear();
                self.patches.clear();
                {
                    let (slots, removed, patches) =
                        (&mut self.slots, &mut self.deleted, &mut self.patches);
                    fold(&pending.edits, |key, edit| {
                        match edit {
                            Edit::Inserted => {
                                let slot = table
                                    .slot(key)
                                    .ok_or_else(|| invalid("An inserted key holds no slot"))?;
                                engine_allocation::reserve(slots, 1)?;
                                slots.push((key, slot));
                            }
                            Edit::Deleted => {
                                if rows.contains_key(key) {
                                    engine_allocation::reserve(removed, 1)?;
                                    removed.push(key);
                                }
                            }
                            Edit::Written(mask) => {
                                let slot = table
                                    .slot(key)
                                    .ok_or_else(|| invalid("A changed key holds no slot"))?;
                                engine_allocation::reserve(patches, 1)?;
                                patches.push((key, slot, mask));
                            }
                        }
                        Ok(())
                    })?;
                }
                // Inserted rows are built whole; a key deleted and inserted again is set.
                let inserted = rows_at(
                    id,
                    table,
                    dictionary,
                    &mut self.slots,
                    &mut self.keys,
                    self.format,
                )?;
                if self.keys.len() + self.patches.len() != changed || self.deleted.len() != deleted
                {
                    return Err(invalid("A table's edits differ from their plan"));
                }
                let stamp = match credit {
                    Some(credit) => {
                        let recorded = id.last_change().fields(id, self.format);
                        let mut rows_changed = engine_allocation::vec(changed)?;
                        for &key in &self.keys {
                            rows_changed.push(RowChange {
                                key,
                                fields: change_fields(id, full_mask(recorded), recorded),
                                inserted: !rows.contains_key(key),
                            });
                        }
                        for &(key, _, mask) in &self.patches {
                            rows_changed.push(RowChange {
                                key,
                                fields: change_fields(id, mask, recorded),
                                inserted: false,
                            });
                        }
                        rows_changed.sort_unstable_by_key(|row| row.key);
                        let bytes = Stamp::edit_bytes(changed, deleted)?;
                        Some(previous.edited(
                            rows_changed,
                            engine_allocation::copy_vec(&self.deleted)?,
                            hour,
                            credit.split_off(bytes),
                        )?)
                    }
                    None => None,
                };
                if self.record {
                    let delta = self.delta(id, baseline)?;
                    self.fact(|| Ok(delta))?;
                }
                let mut held = match previous.try_unwrap() {
                    Ok(TableView::Complete(rows)) => rows,
                    Ok(_) => unreachable!("only complete views are patched"),
                    Err(shared) => match &*shared {
                        TableView::Complete(rows) => rows.try_clone_validated()?,
                        _ => unreachable!("only complete views are patched"),
                    },
                };
                if !self.keys.is_empty() || !self.deleted.is_empty() {
                    let deleted = engine_allocation::copy_vec(&self.deleted)?;
                    held = held.apply_changes(RowChanges::new(inserted, deleted))?;
                }
                // Every other changed or filled row gets the columns written to it.
                if !self.patches.is_empty() {
                    self.masks.clear();
                    engine_allocation::reserve(&mut self.masks, self.patches.len())?;
                    self.masks
                        .extend(self.patches.iter().map(|&(key, _, mask)| (key, mask)));
                    let (patches, format) = (&self.patches, self.format);
                    held.patch_from_wire(&self.masks, &mut |at, column| {
                        wire(table, dictionary, patches[at].1, column, format)
                    })?;
                }
                let view = SharedView::from(TableView::Complete(held)).with_sampled_at(sampled);
                Ok(match stamp {
                    Some(stamp) => view.with_stamp(stamp),
                    None => view,
                })
            }
        }
    }
}

/// Hours a walk can take on its own: from its run's start or from a restart, through the
/// hour before the run's next restart. Ordinals of the run, inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub run: u32,
    /// The page whose restart the walk starts from; None from the run's start.
    pub restart: Option<usize>,
    pub first: u32,
    pub last: u32,
}
