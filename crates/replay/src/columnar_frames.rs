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
use replay_columnar::{
    envelope::{self, Extent, IndexPage, Kind, Root, RunDirectory},
    framing,
    leaf::Reader,
    manifest::{Manifest, Status},
    state::{Cell, Kind as Column, Table},
    walk::{Dictionary, State},
};
use std::{io, sync::Arc};

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

/// A columnar recording (`H4COLUMN` version 3): its root and every run's pages over the
/// file's bytes ([`replay_columnar::envelope`]). A run directory or page that cannot be read
/// refuses only what it covers ([`Self::damage`]); the rest stays readable.
pub struct Recording {
    /// The file through its last footer whose root verifies.
    bytes: Vec<u8>,
    root: Root,
    /// Each run's directory, or why it cannot be read.
    runs: Vec<Result<RunDirectory, String>>,
    /// Each run's pages, each or why it cannot be read; none for a run that cannot be read.
    pages: Vec<Vec<Result<IndexPage, String>>>,
    /// Whether walks check every hour against the pages' hour digest chain.
    verify: bool,
    /// The tables walks restore and apply; every table when None.
    selected: Option<Vec<bool>>,
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

/// One piece of a stream's unit: the ordinals it covers and where its raw bytes sit in the
/// unit's concatenation.
struct Piece {
    first: u64,
    end: u64,
    start: usize,
    stop: usize,
}

/// One stream's unit of a page: its pieces' raw bytes concatenated in order.
pub struct Unit {
    pub bytes: Vec<u8>,
    pieces: Vec<Piece>,
}

impl Unit {
    /// Whether `start..stop` of the unit lies within the piece covering `ordinal`.
    fn holds(&self, ordinal: u64, start: usize, stop: usize) -> bool {
        self.pieces.iter().any(|piece| {
            (piece.first..piece.end).contains(&ordinal)
                && piece.start <= start
                && stop <= piece.stop
        })
    }
}

/// One extent's bytes decompressed, its place, digest and length checked first.
fn extent_bytes(file: &[u8], extent: &Extent) -> io::Result<Vec<u8>> {
    let stored = extent.stored_in(file)?;
    let raw = match zstd::zstd_safe::get_frame_content_size(stored) {
        Ok(Some(raw)) if extent.kind == Kind::Root || raw == extent.raw => raw,
        _ => return Err(extent.damaged("differs from its frame's length")),
    };
    let raw = usize::try_from(raw).map_err(|_| extent.damaged("exceeds memory"))?;
    let mut bytes = engine_allocation::vec(raw)?;
    zstd::bulk::Decompressor::new()?.decompress_to_buffer(stored, &mut bytes)?;
    if bytes.len() != raw {
        return Err(extent.damaged("decompresses to another length"));
    }
    Ok(bytes)
}

impl Recording {
    pub fn open(mut bytes: Vec<u8>) -> io::Result<Self> {
        // A torn tail after the last valid footer is no part of the recording.
        let (root, end) = envelope::last_root(&bytes)?;
        bytes.truncate(end);
        let root = Root::read(&extent_bytes(&bytes, &root)?)?;
        let columns = schema()?;
        if root.identity.schema != envelope::schema_digest(&columns) {
            return Err(invalid("The recording's schema differs from this build's"));
        }
        let mut runs = engine_allocation::vec(root.runs.len())?;
        let mut pages = engine_allocation::vec(root.runs.len())?;
        for (at, run) in root.runs.iter().enumerate() {
            let run = extent_bytes(&bytes, run)
                .and_then(|raw| RunDirectory::read(&raw, run, at as u32))
                .map_err(|error| error.to_string());
            let run_pages = match &run {
                Ok(run) => engine_allocation::collect_vec(run.pages.iter().map(|page| {
                    extent_bytes(&bytes, page)
                        .and_then(|raw| IndexPage::read(&raw, page, columns.len()))
                        .map_err(|error| error.to_string())
                }))?,
                Err(_) => Vec::new(),
            };
            runs.push(run);
            pages.push(run_pages);
        }
        Ok(Self {
            bytes,
            root,
            runs,
            pages,
            verify: false,
            selected: None,
        })
    }

    /// Makes every walk check each hour against the hour digest chain the pages hold.
    /// Walks restore, read and apply only `tables`, the tables their readers read; the others
    /// keep their statuses and no rows. A walk that verifies still reads every unit.
    pub fn select(&mut self, tables: &[TableId]) {
        let mut selected = vec![false; TABLE_COUNT];
        for table in tables {
            selected[table.index()] = true;
        }
        self.selected = Some(selected);
    }

    fn selects(&self, table: usize) -> bool {
        self.selected
            .as_ref()
            .is_none_or(|selected| selected.get(table).copied().unwrap_or(false))
    }

    /// Applies the selection to a walk's `state`, clearing any an earlier walk left.
    fn select_in(&self, state: &mut State) -> io::Result<()> {
        let skipped: Vec<bool> = (0..TABLE_COUNT).map(|table| !self.selects(table)).collect();
        state.skip(&skipped)
    }

    pub fn verify(&mut self, verify: bool) {
        self.verify = verify;
    }

    /// Why each run directory or page that cannot be read is refused, naming its scope.
    pub fn damage(&self) -> Vec<&str> {
        let runs = self.runs.iter().filter_map(|run| run.as_ref().err());
        let pages = self
            .pages
            .iter()
            .flatten()
            .filter_map(|page| page.as_ref().err());
        runs.chain(pages).map(String::as_str).collect()
    }

    /// Recorded hours over every run that can be read.
    pub fn hours(&self) -> u64 {
        self.runs
            .iter()
            .filter_map(|run| run.as_ref().ok())
            .map(|run| u64::from(run.ordinals))
            .sum()
    }

    /// Each run's directory, or why it cannot be read.
    pub fn runs(&self) -> &[Result<RunDirectory, String>] {
        &self.runs
    }

    /// Run `run`'s directory, or why it cannot be read.
    pub fn run(&self, run: u32) -> io::Result<&RunDirectory> {
        self.runs
            .get(run as usize)
            .ok_or_else(|| invalid("No such run"))?
            .as_ref()
            .map_err(|why| invalid(why.clone()))
    }

    /// Run `run`'s pages, in order, each or why it cannot be read.
    pub fn pages(&self, run: u32) -> &[Result<IndexPage, String>] {
        self.pages.get(run as usize).map_or(&[], Vec::as_slice)
    }

    fn page(&self, run: u32, at: usize) -> io::Result<&IndexPage> {
        self.pages(run)
            .get(at)
            .ok_or_else(|| invalid("No such page"))?
            .as_ref()
            .map_err(|why| invalid(why.clone()))
    }

    /// One extent's bytes, decompressed after its place and digest are checked.
    pub fn extent(&self, extent: &Extent) -> io::Result<Vec<u8>> {
        extent_bytes(&self.bytes, extent)
    }

    /// The file's length through its last valid footer.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// One stream's unit of `page`; None when the page holds none of it.
    fn stream(&self, page: &IndexPage, stream: u32) -> io::Result<Option<Unit>> {
        let mut unit: Option<Unit> = None;
        for extent in page.stream(stream) {
            let raw = self.extent(extent)?;
            let unit = unit.get_or_insert_with(|| Unit {
                bytes: Vec::new(),
                pieces: Vec::new(),
            });
            let start = unit.bytes.len();
            engine_allocation::reserve(&mut unit.bytes, raw.len())?;
            unit.bytes.extend_from_slice(&raw);
            engine_allocation::reserve(&mut unit.pieces, 1)?;
            unit.pieces.push(Piece {
                first: u64::from(extent.first),
                end: u64::from(extent.first) + u64::from(extent.hours),
                start,
                stop: unit.bytes.len(),
            });
        }
        Ok(unit)
    }

    /// `page`'s manifests and each table's sections, as [`Self::stream`] gives them.
    pub fn unit_streams(&self, page: &IndexPage) -> io::Result<(Unit, Vec<Option<Unit>>)> {
        let manifests = self
            .stream(page, framing::MANIFESTS)?
            .ok_or_else(|| invalid("A page has no manifests"))?;
        let mut tables: Vec<Option<Unit>> =
            engine_allocation::collect_vec((0..TABLE_COUNT).map(|_| None))?;
        let mut streams: Vec<u32> = page
            .units
            .iter()
            .map(|unit| unit.stream)
            .filter(|&stream| stream != framing::MANIFESTS)
            .collect();
        streams.dedup();
        for stream in streams {
            let table = (stream - 1) as usize;
            if !self.verify && !self.selects(table) {
                continue;
            }
            *tables
                .get_mut(table)
                .ok_or_else(|| invalid("A unit names a stream past the schema"))? =
                self.stream(page, stream)?;
        }
        Ok((manifests, tables))
    }

    /// Every readable run's segments in order: from the run's start, then from each restart
    /// a readable page holds.
    pub fn segments(&self) -> Vec<Segment> {
        let mut segments = Vec::new();
        for (run, directory) in self.runs.iter().enumerate() {
            let Ok(directory) = directory else {
                continue;
            };
            let run = run as u32;
            let mut start = (None, 0_u32);
            for (at, page) in self.pages(run).iter().enumerate() {
                if let Ok(page) = page
                    && let Some(restart) = &page.restart
                    && u64::from(restart.after) + 1 < u64::from(directory.ordinals)
                {
                    segments.push(Segment {
                        run,
                        restart: start.0,
                        first: start.1,
                        last: restart.after,
                    });
                    start = (Some(at), restart.after + 1);
                }
            }
            if start.1 < directory.ordinals {
                segments.push(Segment {
                    run,
                    restart: start.0,
                    first: start.1,
                    last: directory.ordinals - 1,
                });
            }
        }
        segments
    }

    /// The page of `segment`'s run whose restart holds the segment's last hour, the run's
    /// last restart included; None when no readable page holds one.
    pub fn restart_after(&self, segment: &Segment) -> Option<usize> {
        self.pages(segment.run).iter().position(|page| {
            page.as_ref().is_ok_and(|page| {
                page.restart
                    .as_ref()
                    .is_some_and(|restart| restart.after == segment.last)
            })
        })
    }

    /// Replaces `state` with what restart page `at` of `run` holds: the run's dictionary
    /// chunks through it, each holding the run's next ids, then each table's state.
    pub fn restore(&self, run: u32, at: usize, state: &mut State) -> io::Result<()> {
        let directory = self.run(run)?;
        let restart = self
            .page(run, at)?
            .restart
            .as_ref()
            .ok_or_else(|| invalid("The page holds no restart"))?;
        if i64::from(restart.hour) != i64::from(directory.first_hour) + i64::from(restart.after) {
            return Err(restart
                .chunk
                .damaged("belongs to a restart off its run's hours"));
        }
        let mut chunks = Vec::new();
        let mut next_id = 0_u64;
        for page in self.pages(run).get(..=at).unwrap_or_default() {
            // A page that cannot be read hides at most a chunk, whose absence the next
            // restart's ordinal and ids then show.
            let Ok(page) = page else {
                continue;
            };
            let Some(held) = &page.restart else {
                continue;
            };
            let chunk = extent_bytes(&self.bytes, &held.chunk)?;
            let count = Reader::new(&chunk).count()? as u64;
            if held.ordinal as usize != chunks.len()
                || u64::from(held.ids.0) != next_id
                || count != u64::from(held.ids.1)
            {
                return Err(held
                    .chunk
                    .damaged("does not hold the run's next dictionary ids"));
            }
            next_id += count;
            engine_allocation::reserve(&mut chunks, 1)?;
            chunks.push(chunk);
        }
        state.restore_dictionary(chunks.iter().map(Vec::as_slice))?;
        for table in restart
            .tables
            .iter()
            .filter(|table| self.selects(table.table as usize))
        {
            state.restore(
                table.table as usize,
                &extent_bytes(&self.bytes, &table.extent)?,
            )?;
        }
        self.select_in(state)
    }

    /// Every readable run's hours in order, each run from empty tables: the walker's state
    /// once it applied the hour and that hour's manifest, with whether the hour starts its
    /// run. `visit` stops the walk by returning false; a page that cannot be read refuses
    /// the walk where it reaches it.
    pub fn walk(
        &self,
        state: &mut State,
        mut visit: impl FnMut(&State, &Manifest<'_>, bool) -> io::Result<bool>,
    ) -> io::Result<()> {
        let mut going = true;
        for segment in self.segments() {
            if !going {
                break;
            }
            if segment.restart.is_none() {
                *state = State::new(schema()?)?;
                self.select_in(state)?;
            }
            self.walk_hours(&segment, state, &mut |state, manifest, starts| {
                going = visit(state, manifest, starts && segment.restart.is_none())?;
                Ok(going)
            })?;
        }
        Ok(())
    }

    /// `segment`'s hours from its restart restored into `state`, or from empty tables at its
    /// run's start, as [`Self::walk`] visits them; `starts` is the segment's first hour.
    pub fn walk_segment(
        &self,
        segment: &Segment,
        state: &mut State,
        visit: impl FnMut(&State, &Manifest<'_>, bool) -> io::Result<bool>,
    ) -> io::Result<()> {
        self.begin_segment(segment, state)?;
        self.walk_begun(segment, state, visit)
    }

    /// Puts into `state` what `segment`'s first hour applies to: its restart's tables, or
    /// empty tables at its run's start.
    pub fn begin_segment(&self, segment: &Segment, state: &mut State) -> io::Result<()> {
        match segment.restart {
            Some(at) => self.restore(segment.run, at, state),
            None => {
                *state = State::new(schema()?)?;
                self.select_in(state)
            }
        }
    }

    /// `segment`'s hours from `state` as [`Self::begin_segment`] left it, as
    /// [`Self::walk_segment`] visits them.
    pub fn walk_begun(
        &self,
        segment: &Segment,
        state: &mut State,
        mut visit: impl FnMut(&State, &Manifest<'_>, bool) -> io::Result<bool>,
    ) -> io::Result<()> {
        self.walk_hours(segment, state, &mut visit)
    }

    /// The hours of `segment`'s ordinals, page by page: one manifest per ordinal at its run's
    /// branch and hour, every manifest and section within the piece covering its ordinal,
    /// every unit read to its end, and the chain checked when asked.
    fn walk_hours(
        &self,
        segment: &Segment,
        state: &mut State,
        visit: &mut dyn FnMut(&State, &Manifest<'_>, bool) -> io::Result<bool>,
    ) -> io::Result<()> {
        let run = self.run(segment.run)?;
        let columns = state.columns().to_vec();
        let mut chain = match segment.restart {
            Some(at) => self.page(segment.run, at)?.chain,
            None => envelope::chain_start(&self.root.identity.chain_bytes()?, segment.run),
        };
        let hour = |ordinal: u64| i64::from(run.first_hour) + ordinal as i64;
        let mut first = true;
        for (at, page) in self.pages(segment.run).iter().enumerate() {
            if !(segment.first..=segment.last).contains(&run.pages[at].first) {
                continue;
            }
            // A page that cannot be read refuses only the walks of its segment.
            let page = page.as_ref().map_err(|why| invalid(why.clone()))?;
            let refused = |what: &str| {
                invalid(format!(
                    "Run {} from ordinal {} {what}",
                    page.run, page.first
                ))
            };
            let end = u64::from(page.first) + u64::from(page.hours);
            if i64::from(page.first_hour) != hour(page.first.into())
                || i64::from(page.last_hour) != hour(end - 1)
            {
                return Err(refused("names hours off its ordinals"));
            }
            let (manifests, tables) = self.unit_streams(page)?;
            let mut readers: Vec<Option<(Reader<'_>, &Unit)>> = engine_allocation::collect_vec(
                tables
                    .iter()
                    .map(|unit| unit.as_ref().map(|unit| (Reader::new(&unit.bytes), unit))),
            )?;
            let mut reader = Reader::new(&manifests.bytes);
            let mut ordinal = u64::from(page.first);
            loop {
                let start = reader.at();
                let Some(bytes) = framing::next_manifest(&mut reader, manifests.bytes.len())?
                else {
                    break;
                };
                if ordinal >= end || !manifests.holds(ordinal, start, reader.at()) {
                    return Err(refused("holds a manifest off its ordinals"));
                }
                let manifest = Manifest::read(bytes, &columns)?;
                if manifest.branch != run.branch || i64::from(manifest.hour) != hour(ordinal) {
                    return Err(refused("holds an hour off its run's branch and hours"));
                }
                let mut sections = engine_allocation::vec(manifest.sections.len())?;
                for section in &manifest.sections {
                    if !self.verify && state.skips(section.table) {
                        sections.push(&[][..]);
                        continue;
                    }
                    let (reader, unit) = readers
                        .get_mut(section.table)
                        .and_then(Option::as_mut)
                        .ok_or_else(|| invalid("A section's table has no unit"))?;
                    let bytes = framing::next_section(reader, section.bytes as usize)?;
                    if !unit.holds(ordinal, reader.at() - bytes.len(), reader.at()) {
                        return Err(refused("holds a section outside its hour's piece"));
                    }
                    sections.push(bytes);
                }
                if self.verify {
                    chain = envelope::chain_hour(&chain, bytes, sections.iter().copied());
                }
                state.apply(&manifest, |index| Ok(sections[index]))?;
                if !visit(state, &manifest, first)? {
                    return Ok(());
                }
                first = false;
                ordinal += 1;
            }
            if ordinal != end
                || readers
                    .iter()
                    .flatten()
                    .any(|(reader, _)| reader.finish().is_err())
            {
                return Err(refused("does not hold exactly its hours"));
            }
            if self.verify && chain != page.chain {
                return Err(refused("differs from its hour digest chain"));
            }
        }
        Ok(())
    }
}
