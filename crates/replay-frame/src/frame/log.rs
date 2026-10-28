//! Frames kept in hour order, each holding only what changed since the frame
//! before it: a reader keeping a block's frames for later keeps one whole
//! frame and the rows each later one changed, rather than a copy of every
//! table each hour it changed. Read back in order they are the frames kept.

use super::{
    BranchId, DecodedFrame, FieldStamp, Hour, Metrics, SharedView, TABLE_COUNT, TableView,
    not_selected,
};
use crate::rows::RowChanges;
use engine_allocation as allocation;
use std::{io, sync::Arc};

/// Frames kept in hour order ([`Self::push`]), read back with
/// [`Self::into_frames`].
#[derive(Default)]
pub struct FrameLog {
    entries: Vec<Entry>,
    /// The last frame kept, whole, to find what the next one changes.
    last: Option<DecodedFrame>,
}

struct Entry {
    branch: BranchId,
    hour: Hour,
    metrics: Metrics,
    /// The tables whose view differs from the frame before's, every table
    /// for the first.
    tables: Vec<(usize, Kept)>,
}

enum Kept {
    /// The view itself.
    View(SharedView),
    /// The frame before's allocation, with this frame's acquisition.
    Same(Acquisition),
    /// A new allocation of the frame before's rows with these changes.
    Changed(RowChanges, Acquisition),
}

/// A view's acquisition beside its rows. A frame log keeps no change facts.
struct Acquisition {
    sampled: Option<Hour>,
    stamp: Option<Arc<FieldStamp>>,
}

impl Acquisition {
    fn of(view: &SharedView) -> Self {
        Self {
            sampled: view.1,
            stamp: view.3.clone(),
        }
    }

    fn same(a: &SharedView, b: &SharedView) -> bool {
        a.1 == b.1
            && match (&a.3, &b.3) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }

    fn on(self, mut view: SharedView) -> SharedView {
        view.1 = self.sampled;
        view.3 = self.stamp;
        view
    }
}

impl FrameLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Keep `frame` after those kept before it. A frame whose views carry
    /// change facts is refused: a view read back is a new allocation, which
    /// the facts of the view after it do not name as its predecessor.
    pub fn push(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        if frame.tables.iter().any(|view| view.2.is_some()) {
            return Err(super::invalid("a frame log keeps no change facts"));
        }
        let mut tables = Vec::new();
        match &self.last {
            None => {
                allocation::reserve(&mut tables, TABLE_COUNT)?;
                tables.extend(
                    frame
                        .tables
                        .iter()
                        .enumerate()
                        .map(|(index, view)| (index, Kept::View(view.clone()))),
                );
            }
            Some(last) => {
                for (index, (before, now)) in last.tables.iter().zip(&frame.tables).enumerate() {
                    let kept = if SharedView::ptr_eq(before, now) {
                        if Acquisition::same(before, now) {
                            continue;
                        }
                        Kept::Same(Acquisition::of(now))
                    } else {
                        match (&*before.0, &*now.0) {
                            (TableView::Complete(old), TableView::Complete(rows)) => {
                                match rows.changes_since(old)? {
                                    Some(changes) => Kept::Changed(changes, Acquisition::of(now)),
                                    None => Kept::View(now.clone()),
                                }
                            }
                            _ => Kept::View(now.clone()),
                        }
                    };
                    allocation::reserve(&mut tables, 1)?;
                    tables.push((index, kept));
                }
            }
        }
        allocation::reserve(&mut self.entries, 1)?;
        self.entries.push(Entry {
            branch: frame.branch,
            hour: frame.hour,
            metrics: frame.metrics.clone(),
            tables,
        });
        self.last = Some(frame.clone());
        Ok(())
    }

    /// Every view the log keeps whole: each entry's and the last frame's.
    pub fn views(&self) -> impl Iterator<Item = &SharedView> {
        self.entries
            .iter()
            .flat_map(|entry| &entry.tables)
            .filter_map(|(_, kept)| match kept {
                Kept::View(view) => Some(view),
                _ => None,
            })
            .chain(self.last.iter().flat_map(|last| &last.tables))
    }

    /// The bytes the log holds beside [`Self::views`]: its entries and the
    /// rows and keys it keeps as changes.
    pub fn bytes(&self) -> usize {
        let mut bytes = self.entries.capacity() * size_of::<Entry>();
        for entry in &self.entries {
            bytes += entry.tables.capacity() * size_of::<(usize, Kept)>();
            for (_, kept) in &entry.tables {
                if let Kept::Changed(changes, _) = kept {
                    bytes += changes.heap_bytes();
                }
            }
        }
        bytes
    }

    /// The frames kept, in order. A table changed since the frame before is
    /// changed in place once the frame before is no longer held, and copied
    /// first otherwise.
    pub fn into_frames(self) -> LoggedFrames {
        LoggedFrames {
            entries: self.entries.into_iter(),
            current: None,
        }
    }
}

/// [`FrameLog::into_frames`].
pub struct LoggedFrames {
    entries: std::vec::IntoIter<Entry>,
    current: Option<Box<[SharedView; TABLE_COUNT]>>,
}

impl LoggedFrames {
    fn rebuild(&mut self, entry: Entry) -> io::Result<DecodedFrame> {
        let current = self
            .current
            .get_or_insert_with(|| Box::new(std::array::from_fn(|_| not_selected())));
        for (index, kept) in entry.tables {
            current[index] = match kept {
                Kept::View(view) => view,
                Kept::Same(acquisition) => acquisition.on(current[index].clone()),
                Kept::Changed(changes, acquisition) => {
                    let before = std::mem::replace(&mut current[index], not_selected());
                    let rows = match before.try_unwrap() {
                        Ok(TableView::Complete(rows)) => rows,
                        Ok(_) => {
                            return Err(super::invalid("kept rows changed an incomplete view"));
                        }
                        Err(shared) => match &*shared.0 {
                            TableView::Complete(rows) => rows.try_clone_validated()?,
                            _ => {
                                return Err(super::invalid("kept rows changed an incomplete view"));
                            }
                        },
                    };
                    let view = TableView::Complete(rows.apply_changes(changes)?).into();
                    acquisition.on(view)
                }
            };
        }
        Ok(DecodedFrame {
            branch: entry.branch,
            hour: entry.hour,
            metrics: entry.metrics,
            tables: std::array::from_fn(|index| current[index].clone()),
        })
    }
}

impl Iterator for LoggedFrames {
    type Item = io::Result<DecodedFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = self.entries.next()?;
        Some(self.rebuild(entry))
    }
}

#[cfg(test)]
#[path = "log_tests.rs"]
mod tests;
