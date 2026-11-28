use super::{BLOCK, Keys, Recording, SharedView, Status, TableId, TableReader, TableView, lanes};
use replay_columnar::container::Revision;
use replay_frame::frame::cells_rows;
use replay_recording::columnar_frames::{Source, TableCoverage, view_heap_bytes};
use std::sync::{Arc, Weak};

#[cfg(test)]
mod tests;

const TABLE: TableId = TableId::LexicalTokens;
const RETAINED_BYTES: usize = 16 * 1024 * 1024;
#[derive(Default)]
pub struct LexicalCursor {
    held: Option<Held>,
}

struct Held {
    source: Weak<Source>,
    revision: Revision,
    reader: TableReader,
    view: SharedView,
    view_bytes: usize,
}

impl LexicalCursor {
    pub fn read(&mut self, recording: &Recording, run: u32, hour: i32) -> Option<(SharedView, bool, u64)> {
        self.read_bounded(recording, run, hour, RETAINED_BYTES)
    }

    fn read_bounded(&mut self, recording: &Recording, run: u32, hour: i32, budget: usize) -> Option<(SharedView, bool, u64)> {
        let held = self.held.take();
        if recording.columns().get(TABLE.index()) != Some(&TABLE.schema().len())
            || recording.read_columns(TABLE.index()) != [0]
            || !matches!(recording.table_coverage(TABLE.index(), run), TableCoverage::Valid)
        {
            return None;
        }
        let ordinal = recording.run(run).ok()?.ordinal(hour)?;
        let source = recording.source();
        let revision = recording.revision();
        let warm = held.filter(|held| {
            held.revision == revision && held.source.ptr_eq(&Arc::downgrade(&source))
                && held.reader.run() == run
                && held.reader.hour().is_some_and(|at| (0..=i64::from(BLOCK)).contains(&(i64::from(hour) - i64::from(at))))
        });
        let (mut reader, kept) = if let Some(mut warm) = warm
            && let Some(changed) = warm.reader.walk_exact(recording, ordinal).ok()?
        {
            (warm.reader, (!changed).then_some((warm.view, warm.view_bytes)))
        } else {
            let mut reader = TableReader::new(recording, run, TABLE, None).ok()?;
            reader.seek(recording, ordinal).ok()?;
            (reader, None)
        };
        if reader.status() != Status::Complete || reader.columns() != [0] {
            return None;
        }
        let (view, view_bytes, rows) = match kept {
            Some((view, bytes)) => (view, bytes, 0),
            None => {
                let chosen = lanes::asked_rows(&reader, Keys::All).ok()?;
                let view = TableView::Complete(cells_rows(TABLE, &reader.view(), &chosen, recording.format()).ok()?);
                let bytes = view_heap_bytes(&view) + size_of::<TableView>() + 2 * size_of::<usize>();
                (SharedView::from(view), bytes, chosen.len() as u64)
            }
        };
        let view = view.with_sampled_at(reader.sampled_at());
        let read = reader.was_read();
        if reader.park_exact().is_some_and(|bytes| bytes + view_bytes + size_of::<Held>() - size_of::<TableReader>() <= budget) {
            self.held = Some(Held { source: Arc::downgrade(&source), revision, reader, view: view.clone(), view_bytes });
        }
        Some((view, read, rows))
    }
}
