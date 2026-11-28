use super::{BLOCK, Keys, Recording, SharedView, Status, TableId, TableReader, TableView, lanes};
use replay_columnar::container::Revision;
use replay_columnar::store::TableState;
use replay_data::rows::{LexicalTokenRow, TableRows};
use replay_frame::frame::cells_rows;
use replay_recording::cells::Changes;
use replay_recording::columnar_frames::{Source, TableCoverage, view_heap_bytes};
use std::sync::{Arc, Weak};

#[cfg(test)]
mod tests;

const TABLE: TableId = TableId::LexicalTokens;
const RETAINED_BYTES: usize = 16 * 1024 * 1024;
#[derive(Default)]
pub struct LexicalCursor(CompleteCursor);

impl LexicalCursor {
    pub fn read(&mut self, recording: &Recording, run: u32, hour: i32) -> Option<(SharedView, bool, u64)> {
        self.0.read(recording, run, hour, TABLE, RETAINED_BYTES)
    }
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
        let warm = held.filter(|held| {
            held.revision == revision && held.source.ptr_eq(&Arc::downgrade(&source))
                && held.reader.run() == run && held.reader.table() == table
                && held.reader.hour().is_some_and(|at| (0..=i64::from(BLOCK)).contains(&(i64::from(hour) - i64::from(at))))
        });
        let mut appended = None;
        let (mut reader, kept) = if let Some(mut warm) = warm
            && let Some(changed) = {
                let tail = match warm.view.held() {
                    TableView::Complete(TableRows::LexicalTokens(rows)) => {
                        appended = Some(Vec::new());
                        rows.last().map(|row| u128::from(row.token))
                    }
                    _ => None,
                };
                warm.reader.walk_exact(recording, ordinal, |changes, cells| {
                    if let Some(rows) = &mut appended && append_keys(rows, tail, changes, cells).is_none() {
                        appended = None;
                    }
                }).ok()?
            }
        {
            let kept = if changed {
                appended.and_then(|rows| append_view(warm.view, warm.view_bytes, &warm.reader, recording.format(), rows))
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
