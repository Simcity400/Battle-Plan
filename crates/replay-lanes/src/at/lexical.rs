use super::{BLOCK, Keys, Recording, SharedView, Status, TableId, TableReader, TableView, lanes};
use replay_columnar::container::Revision;
use replay_columnar::store::TableState;
use replay_data::rows::{LexicalTokenRow, RowChanges, TableRows, tables};
use replay_frame::frame::cells_rows;
use replay_recording::cells::Changes;
use replay_recording::columnar_frames::{Source, TableCoverage, view_heap_bytes};
use std::sync::{Arc, Weak};

#[cfg(test)]
mod tests;

const TABLE: TableId = TableId::LexicalTokens;
const RETAINED_BYTES: usize = 16 * 1024 * 1024;
const ANCHOR_BYTES: usize = 16 * 1024;

#[derive(Default)]
pub struct LexicalCursor(CompleteCursor, CheckpointProofs);

impl LexicalCursor {
    pub fn read(&mut self, recording: &Recording, run: u32, hour: i32) -> Option<(SharedView, bool, u64)> {
        self.0.read_with(recording, run, hour, TABLE, RETAINED_BYTES, Some(&self.1))
    }

    pub fn prepare(recording: &Recording, run: u32) -> Option<VerifiedAnchors> {
        CheckpointProofs::prepare(recording, run, TABLE)
    }

    pub fn use_anchors(&mut self, anchors: VerifiedAnchors) { self.1.insert(anchors); }

    pub fn release_view(&mut self) { self.0.held = None; }
}

#[derive(Default)]
pub struct CheckpointProofs(Box<[VerifiedAnchors]>);

impl CheckpointProofs {
    pub fn prepare(recording: &Recording, run: u32, table: TableId) -> Option<VerifiedAnchors> {
        let columns = table.schema().len();
        if recording.columns().get(table.index()) != Some(&columns)
            || recording.read_columns(table.index()).into_iter().ne(0..columns)
            || !matches!(recording.table_coverage(table.index(), run), TableCoverage::Valid)
        {
            return None;
        }
        let span = recording.run(run).ok()?;
        let mut blocks = Vec::new();
        let mut reader = TableReader::new(recording, run, table, None).ok()?;
        for block in 0..(span.pages() as u32).saturating_sub(1) {
            if recording.table_entry(run, block, table.index()).ok()?.unit.restore.is_none() { continue; }
            if reader.seek(recording, (block + 1) * BLOCK - 1).is_ok()
                && reader.verify_restore(recording, block).unwrap_or(false)
            {
                if (blocks.len() + 1) * size_of::<u32>() + size_of::<VerifiedAnchors>() > ANCHOR_BYTES {
                    return None;
                }
                blocks.push(block);
            }
        }
        Some(VerifiedAnchors { table, run, source: Arc::downgrade(&recording.source()), revision: recording.revision(), blocks: blocks.into_boxed_slice() })
    }

    pub fn insert(&mut self, anchors: VerifiedAnchors) {
        if anchors.blocks.is_empty() { return; }
        if self.0.first().is_some_and(|held| held.revision != anchors.revision || !held.source.ptr_eq(&anchors.source)) {
            self.0 = Box::default();
        }
        if self.0.iter().any(|held| held.run == anchors.run && held.table == anchors.table) { return; }
        let bytes = size_of_val(&*self.0) + size_of::<VerifiedAnchors>() + size_of_val(&*anchors.blocks)
            + self.0.iter().map(|held| size_of_val(&*held.blocks)).sum::<usize>();
        if bytes > ANCHOR_BYTES { return; }
        let mut kept = std::mem::take(&mut self.0).into_vec();
        kept.push(anchors);
        self.0 = kept.into_boxed_slice();
    }
}

pub struct VerifiedAnchors {
    table: TableId,
    run: u32,
    source: Weak<Source>,
    revision: Revision,
    blocks: Box<[u32]>,
}

#[derive(Default)]
pub struct CompleteCursor {
    held: Option<Held>,
}

struct Held {
    source: Weak<Source>,
    revision: Revision,
    reader: TableReader,
    view: SharedView,
    view_bytes: usize,
    retained_bytes: usize,
}

impl CompleteCursor {
    pub fn retained_bytes(&self) -> usize {
        self.held.as_ref().map_or(0, |held| held.retained_bytes)
    }

    pub fn read(&mut self, recording: &Recording, run: u32, hour: i32, table: TableId, budget: usize) -> Option<(SharedView, bool, u64)> {
        self.read_with(recording, run, hour, table, budget, None)
    }

    pub fn read_with(&mut self, recording: &Recording, run: u32, hour: i32, table: TableId, budget: usize, proofs: Option<&CheckpointProofs>) -> Option<(SharedView, bool, u64)> {
        let held = self.held.take();
        let columns = table.schema().len();
        if recording.columns().get(table.index()) != Some(&columns)
            || recording.read_columns(table.index()).into_iter().ne(0..columns)
            || !matches!(recording.table_coverage(table.index(), run), TableCoverage::Valid)
        {
            return None;
        }
        let ordinal = recording.run(run).ok()?.ordinal(hour)?;
        let source = recording.source();
        let revision = recording.revision();
        let weak = Arc::downgrade(&source);
        let known = proofs.and_then(|proofs| proofs.0.iter().find(|held| held.run == run && held.table == table
            && held.revision == revision && held.source.ptr_eq(&weak)));
        let verified = |block| known.is_some_and(|held| held.blocks.binary_search(&block).is_ok());
        let warm = held.filter(|held| {
            held.revision == revision && held.source.ptr_eq(&weak)
                && held.reader.run() == run && held.reader.table() == table
                && held.reader.hour().is_some_and(|at| (0..=i64::from(BLOCK)).contains(&(i64::from(hour) - i64::from(at))))
        });
        let mut appended = None;
        let mut touched = None;
        let (mut reader, kept) = if let Some(mut warm) = warm
            && let Some(changed) = {
                let tail = match warm.view.held() {
                    TableView::Complete(TableRows::LexicalTokens(rows)) => {
                        appended = Some(Vec::new());
                        rows.last().map(|row| u128::from(row.token))
                    }
                    TableView::Complete(TableRows::Characters(rows)) => {
                        touched = Some((Vec::new(), rows.len()));
                        None
                    }
                    _ => None,
                };
                warm.reader.walk_exact(recording, ordinal, verified, |changes, cells| {
                    if let Some(rows) = &mut appended && append_keys(rows, tail, changes, cells).is_none() {
                        appended = None;
                    }
                    if let Some((keys, limit)) = &mut touched
                        && character_keys(keys, *limit, changes, cells).is_none()
                    {
                        touched = None;
                    }
                }).ok()?
            }
        {
            let kept = if changed {
                if let Some((keys, _)) = touched {
                    patch_characters(warm.view, &warm.reader, recording.format(), keys)
                } else {
                    appended.and_then(|rows| append_view(warm.view, warm.view_bytes, &warm.reader, recording.format(), rows))
                }
            } else {
                Some((warm.view, warm.view_bytes, 0))
            };
            (warm.reader, kept)
        } else {
            let mut reader = TableReader::new(recording, run, table, None).ok()?;
            reader.seek(recording, ordinal).ok()?;
            (reader, None)
        };
        if reader.status() != Status::Complete || reader.columns().iter().copied().ne(0..columns) {
            return None;
        }
        let (view, view_bytes, rows) = match kept {
            Some(kept) => kept,
            None => {
                let chosen = lanes::asked_rows(&reader, Keys::All).ok()?;
                let view = TableView::Complete(cells_rows(table, &reader.view(), &chosen, recording.format()).ok()?);
                let bytes = view_heap_bytes(&view) + size_of::<TableView>() + 2 * size_of::<usize>();
                (SharedView::from(view), bytes, chosen.len() as u64)
            }
        };
        let view = view.with_sampled_at(reader.sampled_at());
        let read = reader.was_read();
        if let Some(retained_bytes) = reader.park_exact().map(|bytes| bytes + view_bytes + size_of::<Held>() - size_of::<TableReader>())
            .filter(|&bytes| bytes <= budget)
        {
            self.held = Some(Held { source: Arc::downgrade(&source), revision, reader, view: view.clone(), view_bytes, retained_bytes });
        }
        Some((view, read, rows))
    }
}

fn character_keys(keys: &mut Vec<u128>, limit: usize, changes: &Changes, cells: &TableState) -> Option<()> {
    if changes.anew { return None; }
    let removed = changes.deleted.iter().map(|&(_, key)| key);
    let live = changes.inserted.iter().chain(changes.written.iter().flatten()).filter_map(|&slot| cells.key(slot));
    for key in removed.chain(live) {
        if let Err(at) = keys.binary_search(&key) {
            if keys.len() == limit { return None; }
            keys.try_reserve(1).ok()?;
            keys.insert(at, key);
        }
    }
    Some(())
}

fn character_slots(cells: &TableState, touched: &[u128]) -> Option<Vec<(u128, u32)>> {
    let mut changed = Vec::new();
    changed.try_reserve_exact(touched.len()).ok()?;
    for slot in cells.rows() {
        if let Some(key) = cells.key(slot)
            && touched.binary_search(&key).is_ok()
        {
            if changed.len() == touched.len() { return None; }
            changed.push((key, slot));
        }
    }
    changed.sort_unstable();
    if changed.windows(2).any(|pair| pair[0].0 == pair[1].0) { return None; }
    Some(changed)
}

fn patch_characters(view: SharedView, reader: &TableReader, format: u16, touched: Vec<u128>) -> Option<(SharedView, usize, u64)> {
    let TableView::Complete(mut rows) = view.try_unwrap().ok()? else { return None };
    let changed = character_slots(reader.cells(), &touched)?;
    let mut removed = Vec::new();
    removed.try_reserve_exact(touched.len()).ok()?;
    removed.extend(touched.into_iter().filter(|&key| {
        rows.contains_key(key) && changed.binary_search_by_key(&key, |&(key, _)| key).is_err()
    }));
    let inserted = changed.iter().filter(|&&(key, _)| !rows.contains_key(key)).count();
    rows.rows_mut::<tables::Characters>()?.try_reserve_exact(inserted).ok()?;
    let decoded = cells_rows(TableId::Characters, &reader.view(), &changed, format).ok()?;
    let view = TableView::Complete(rows.apply_changes(RowChanges::new(decoded, removed)).ok()?);
    let bytes = view_heap_bytes(&view) + size_of::<TableView>() + 2 * size_of::<usize>();
    Some((SharedView::from(view), bytes, changed.len() as u64))
}

fn append_keys(keys: &mut Vec<(u128, u32)>, tail: Option<u128>, changes: &Changes, cells: &TableState) -> Option<()> {
    if changes.anew || !changes.deleted.is_empty() || changes.written.iter().any(|slots| !slots.is_empty()) {
        return None;
    }
    keys.try_reserve(changes.inserted.len()).ok()?;
    for &slot in &changes.inserted {
        let key = cells.key(slot).filter(|&key| tail.is_none_or(|tail| key > tail))?;
        keys.push((key, slot));
    }
    Some(())
}

fn append_view(view: SharedView, bytes: usize, reader: &TableReader, format: u16, mut keys: Vec<(u128, u32)>) -> Option<(SharedView, usize, u64)> {
    let TableView::Complete(TableRows::LexicalTokens(mut held)) = view.try_unwrap().ok()? else { return None };
    keys.sort_unstable();
    let TableRows::LexicalTokens(rows) = cells_rows(TABLE, &reader.view(), &keys, format).ok()? else { return None };
    let previous_capacity = held.capacity();
    held.try_reserve_exact(rows.len()).ok()?;
    let bytes = bytes + (held.capacity() - previous_capacity) * size_of::<LexicalTokenRow>()
        + rows.iter().map(|row| row.literal.capacity()).sum::<usize>();
    let count = rows.len() as u64;
    held.extend(rows);
    Some((TableView::Complete(TableRows::LexicalTokens(held)).into(), bytes, count))
}
