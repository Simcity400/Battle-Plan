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
    frame::{DecodedFrame, Metrics, READ_TIME_NOT_RECORDED, SharedView, TableView, not_selected},
    rows::{Field, List, Opaque, RowChanges, TABLE_COUNT, TableId, TableRows},
    scope::CaptureMode,
};
use replay_columnar::{
    framing::{self, Unit},
    index::Index,
    leaf::Reader,
    manifest::{Manifest, Status},
    state::{Cell, Kind as Column, Table},
    walk::{Dictionary, State},
};
use std::{collections::BTreeMap, io};

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
fn wire<'a>(
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
        let mut tables: [SharedView; TABLE_COUNT] = std::array::from_fn(|_| not_selected());
        for at in 0..self.selected.len() {
            let table = self.selected[at];
            let index = table.index();
            let view = self.view(state, table, present[index], restart)?;
            tables[index] = view.clone();
            self.views[index] = Some(view);
        }
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
        Ok(DecodedFrame {
            branch: manifest.branch,
            hour: manifest.hour,
            tables,
            metrics: Metrics {
                total_ns: manifest.total_ns,
                table_ns,
                pool_bytes: manifest.pool_bytes,
                dropped_frames: manifest.dropped_frames,
            },
        })
    }

    fn view(
        &mut self,
        state: &State,
        id: TableId,
        present: bool,
        restart: bool,
    ) -> io::Result<SharedView> {
        let index = id.index();
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
            Status::NotRecorded => Ok(match previous {
                Some(view) if matches!(*view, TableView::NotRecorded { .. }) => view,
                _ => TableView::NotRecorded { mode: self.mode }.into(),
            }),
            Status::Unavailable { reason } => {
                let reason = dictionary
                    .get(reason)
                    .ok_or_else(|| invalid("A reason names a missing dictionary entry"))?;
                let reason = std::str::from_utf8(reason)
                    .map_err(|_| invalid("A table's reason is not text"))?;
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
                    return Ok(SharedView::from(TableView::Complete(rows)).with_sampled_at(sampled));
                };
                if !present {
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

/// A `p3-convert` recording (`H4COLUMN` version 2): its index over the file's bytes.
pub struct Recording {
    bytes: Vec<u8>,
    index: Index,
}

/// The magic and version a columnar recording starts with.
const MAGIC: &[u8; 8] = b"H4COLUMN";
const VERSION: u32 = 2;

impl Recording {
    pub fn open(bytes: Vec<u8>) -> io::Result<Self> {
        if !bytes.starts_with(MAGIC) || bytes.get(8..12) != Some(&VERSION.to_le_bytes()[..]) {
            return Err(invalid("Not a version 2 H4COLUMN recording"));
        }
        let end = bytes
            .len()
            .checked_sub(8)
            .filter(|&end| end >= 12)
            .ok_or_else(|| invalid("The recording ends before its index offset"))?;
        let at = usize::try_from(u64::from_le_bytes(
            bytes[end..].try_into().expect("eight bytes"),
        ))
        .ok()
        .filter(|&at| (12..=end).contains(&at))
        .ok_or_else(|| invalid("The index offset passes the recording"))?;
        let index = Index::read(&bytes[at..end])?;
        Ok(Self { bytes, index })
    }

    pub fn hours(&self) -> u32 {
        self.index.hours
    }

    /// One unit's decompressed bytes, checked against the index before anything is allocated.
    fn unit(&self, unit: &Unit) -> io::Result<Vec<u8>> {
        let start =
            usize::try_from(unit.offset).map_err(|_| invalid("A unit offset exceeds memory"))?;
        let length =
            usize::try_from(unit.stored).map_err(|_| invalid("A unit length exceeds memory"))?;
        let stored = start
            .checked_add(length)
            .and_then(|end| self.bytes.get(start..end))
            .ok_or_else(|| invalid("A unit passes the recording"))?;
        if zstd::zstd_safe::get_frame_content_size(stored)
            .ok()
            .flatten()
            != Some(unit.raw)
        {
            return Err(invalid(
                "A unit's frame and the index disagree on its length",
            ));
        }
        let raw = usize::try_from(unit.raw).map_err(|_| invalid("A unit length exceeds memory"))?;
        let mut bytes = engine_allocation::vec(raw)?;
        zstd::bulk::Decompressor::new()?.decompress_to_buffer(stored, &mut bytes)?;
        if bytes.len() != raw {
            return Err(invalid("A unit decompresses to another length"));
        }
        Ok(bytes)
    }

    /// Every hour from the first, in order: the walker's state once it applied the hour and
    /// that hour's manifest, with whether the hour starts the walk. `visit` stops the walk by
    /// returning false.
    pub fn walk(
        &self,
        state: &mut State,
        mut visit: impl FnMut(&State, &Manifest<'_>, bool) -> io::Result<bool>,
    ) -> io::Result<()> {
        let columns = state.columns().to_vec();
        let mut positions: BTreeMap<u32, Vec<&Unit>> = BTreeMap::new();
        for unit in &self.index.units {
            let units = positions.entry(unit.first).or_default();
            engine_allocation::reserve(units, 1)?;
            units.push(unit);
        }
        let mut first = true;
        for units in positions.values() {
            let mut manifests = None;
            let mut tables: Vec<Option<Vec<u8>>> =
                engine_allocation::collect_vec((0..columns.len()).map(|_| None))?;
            for unit in units {
                let raw = self.unit(unit)?;
                if unit.stream == framing::MANIFESTS {
                    manifests = Some(raw);
                } else {
                    *tables
                        .get_mut((unit.stream - 1) as usize)
                        .ok_or_else(|| invalid("A unit names a stream past the schema"))? =
                        Some(raw);
                }
            }
            let manifests = manifests.ok_or_else(|| invalid("A unit position has no manifests"))?;
            let mut readers: Vec<Option<Reader<'_>>> = engine_allocation::collect_vec(
                tables.iter().map(|bytes| bytes.as_deref().map(Reader::new)),
            )?;
            let mut reader = Reader::new(&manifests);
            while let Some(bytes) = framing::next_manifest(&mut reader, manifests.len())? {
                let manifest = Manifest::read(bytes, &columns)?;
                let mut sections = engine_allocation::vec(manifest.sections.len())?;
                for section in &manifest.sections {
                    let reader = readers
                        .get_mut(section.table)
                        .and_then(Option::as_mut)
                        .ok_or_else(|| invalid("A section's table has no unit"))?;
                    sections.push(framing::next_section(reader, section.bytes as usize)?);
                }
                state.apply(&manifest, |index| Ok(sections[index]))?;
                if !visit(state, &manifest, first)? {
                    return Ok(());
                }
                first = false;
            }
        }
        Ok(())
    }
}
