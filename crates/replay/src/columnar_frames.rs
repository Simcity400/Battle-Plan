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
//! the walker's sample hour.
//!
//! The manifest keeps which tables an hour read, not how long each took, so a read table's
//! [`Metrics::table_ns`] is [`READ_TIME_NOT_RECORDED`], as for recordings that keep no timings.

use crate::{
    format::FormatVersion,
    frame::{
        DecodedFrame, Metrics, READ_TIME_NOT_RECORDED, RecordedFrame, SharedView, TableUpdate,
        TableView, not_selected,
    },
    rows::{DeltaRow, Field, List, Opaque, RowChanges, TABLE_COUNT, TableId, TableRows, full_mask},
    scope::CaptureMode,
};
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
use std::{io, sync::Arc};

mod input;
mod reader;
pub use input::{Budget, Limits};
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
    TableId::ALL
        .iter()
        .map(|table| crate::native_columns::columns(table.schema()))
        .collect()
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

/// The selected tables' views, carried from hour to hour as the walker advances.
pub struct Views {
    selected: Vec<TableId>,
    views: Vec<Option<SharedView>>,
    /// The format opaque cells were encoded at.
    format: u16,
    mode: CaptureMode,
    slots: Vec<(u128, u32)>,
    keys: Vec<u128>,
    deleted: Vec<u128>,
    /// Each patched row's key, slot and the columns it changed.
    patches: Vec<(u128, u32, u64)>,
    masks: Vec<(u128, u64)>,
    /// Whether [`Self::advance_recorded`] asked for the hour's change facts, and each selected
    /// table's, in selection order.
    record: bool,
    facts: Vec<TableUpdate>,
}

impl Views {
    /// Views of `tables` and Campaign, as a decoder of them reconstructs them.
    pub fn new(tables: &[TableId], format: FormatVersion, mode: CaptureMode) -> io::Result<Self> {
        let mut chosen = [false; TABLE_COUNT];
        chosen[TableId::Campaign.index()] = true;
        for table in tables {
            chosen[table.index()] = true;
        }
        Ok(Self {
            selected: engine_allocation::collect_vec(
                TableId::ALL
                    .into_iter()
                    .filter(|table| chosen[table.index()]),
            )?,
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
        })
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
        let mut present = [false; TABLE_COUNT];
        for section in &manifest.sections {
            *present
                .get_mut(section.table)
                .ok_or_else(|| invalid("A section names a table past the schema"))? = true;
        }
        self.frame(
            state,
            &present,
            restart,
            (manifest.branch, manifest.hour),
            metrics(manifest),
        )
    }

    /// The frame of `state` as a restart restored it, before the walk's first hour, dated
    /// `hour` of `branch`: every view is built anew, and no table reads as read.
    pub fn restored(&mut self, state: &State, branch: u64, hour: i32) -> io::Result<DecodedFrame> {
        self.frame(
            state,
            &[false; TABLE_COUNT],
            true,
            (branch, hour),
            Metrics::default(),
        )
    }

    fn frame(
        &mut self,
        state: &State,
        present: &[bool; TABLE_COUNT],
        restart: bool,
        (branch, hour): (u64, i32),
        metrics: Metrics,
    ) -> io::Result<DecodedFrame> {
        let mut tables: [SharedView; TABLE_COUNT] = std::array::from_fn(|_| not_selected());
        // A continuing walk's deltas apply to the hour before.
        let baseline = hour.saturating_sub(1);
        for at in 0..self.selected.len() {
            let table = self.selected[at];
            let index = table.index();
            let view = self.view(state, table, present[index], restart, baseline)?;
            tables[index] = view.clone();
            self.views[index] = Some(view);
        }
        Ok(DecodedFrame {
            branch,
            hour,
            tables,
            metrics,
        })
    }

    fn view(
        &mut self,
        state: &State,
        id: TableId,
        present: bool,
        restart: bool,
        baseline: i32,
    ) -> io::Result<SharedView> {
        let index = id.index();
        if state.skips(index) {
            return Err(invalid(format!("{id:?} is not read by this walk")));
        }
        let status = state
            .status(index)
            .ok_or_else(|| invalid("The walker holds no such table"))?;
        let previous = self.views[index].take().filter(|_| !restart);
        let sampled = state.sampled_at(index);
        let table = state
            .table(index)
            .ok_or_else(|| invalid("The walker holds no such table"))?;
        let dictionary = state.dictionary();
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
            Status::Complete => {
                let previous = previous.filter(|view| matches!(**view, TableView::Complete(_)));
                let Some(previous) = previous else {
                    self.slots.clear();
                    engine_allocation::reserve(&mut self.slots, table.live().count())?;
                    for slot in table.live().ones() {
                        let key = table
                            .key(slot)
                            .ok_or_else(|| invalid("A live slot holds no key"))?;
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
                    self.fact(|| Ok(TableUpdate::Snapshot(rows.try_clone_validated()?)))?;
                    return Ok(SharedView::from(TableView::Complete(rows)).with_sampled_at(sampled));
                };
                if !present {
                    self.fact(|| Ok(TableUpdate::Unchanged { baseline }))?;
                    return Ok(previous.with_sampled_at(sampled));
                }
                let changes = state
                    .changes(index)
                    .ok_or_else(|| invalid("The walker holds no such table"))?;
                if changes.masks.len() != changes.changed.len()
                    || changes.fill_masks.len() != changes.fill_slots.len()
                {
                    return Err(invalid("A table's change masks miss its changed slots"));
                }
                // Inserted rows are built whole; a key deleted and inserted again is set, not removed.
                self.slots.clear();
                engine_allocation::reserve(&mut self.slots, changes.inserted.len())?;
                for &slot in &changes.inserted {
                    let key = table
                        .key(slot)
                        .ok_or_else(|| invalid("An inserted slot holds no row"))?;
                    self.slots.push((key, slot));
                }
                let inserted = rows_at(
                    id,
                    table,
                    dictionary,
                    &mut self.slots,
                    &mut self.keys,
                    self.format,
                )?;
                self.deleted.clear();
                engine_allocation::reserve(&mut self.deleted, changes.deleted_keys.len())?;
                self.deleted.extend_from_slice(&changes.deleted_keys);
                self.deleted.sort_unstable();
                self.deleted
                    .retain(|key| self.keys.binary_search(key).is_err());
                let mut held = match previous.try_unwrap() {
                    Ok(TableView::Complete(rows)) => rows,
                    Ok(_) => unreachable!("only complete views carry rows"),
                    Err(shared) => match &*shared {
                        TableView::Complete(rows) => rows.try_clone_validated()?,
                        _ => unreachable!("only complete views carry rows"),
                    },
                };
                if !self.keys.is_empty() || !self.deleted.is_empty() {
                    let deleted = engine_allocation::copy_vec(&self.deleted)?;
                    held = held.apply_changes(RowChanges::new(inserted, deleted))?;
                }
                // Every other changed or filled row gets the columns its runs wrote.
                self.patches.clear();
                let written = changes
                    .changed
                    .iter()
                    .zip(&changes.masks)
                    .chain(changes.fill_slots.iter().zip(&changes.fill_masks));
                for (&slot, &mask) in written {
                    let key = table
                        .key(slot)
                        .ok_or_else(|| invalid("A changed slot holds no row"))?;
                    if mask != 0 && self.keys.binary_search(&key).is_err() {
                        engine_allocation::reserve(&mut self.patches, 1)?;
                        self.patches.push((key, slot, mask));
                    }
                }
                if self.patches.is_empty() {
                    if self.record {
                        let delta = self.delta(id, baseline)?;
                        self.fact(|| Ok(delta))?;
                    }
                    return Ok(SharedView::from(TableView::Complete(held)).with_sampled_at(sampled));
                }
                self.patches
                    .sort_unstable_by_key(|&(key, slot, _)| (key, slot));
                self.patches.dedup_by(|later, kept| {
                    let same = later.1 == kept.1;
                    if same {
                        kept.2 |= later.2;
                    }
                    same
                });
                if self.record {
                    let delta = self.delta(id, baseline)?;
                    self.fact(|| Ok(delta))?;
                }
                self.masks.clear();
                engine_allocation::reserve(&mut self.masks, self.patches.len())?;
                self.masks
                    .extend(self.patches.iter().map(|&(key, _, mask)| (key, mask)));
                let (patches, format) = (&self.patches, self.format);
                held.patch_from_wire(&self.masks, &mut |at, column| {
                    wire(table, dictionary, patches[at].1, column, format)
                })?;
                Ok(SharedView::from(TableView::Complete(held)).with_sampled_at(sampled))
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
