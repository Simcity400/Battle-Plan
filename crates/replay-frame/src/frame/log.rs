//! Frames kept in hour order, each holding only what changed since the frame
//! before it: a reader keeping a block's frames for later keeps one whole
//! frame and the rows each later one changed, rather than a copy of every
//! table each hour it changed. Read back in order they are the frames kept.

use super::{
    BranchId, DecodedFrame, FieldStamp, Hour, Metrics, SharedView, TABLE_COUNT, TableView,
    not_selected,
};
use crate::rows::{RowChanges, TableRows};
use engine_allocation as allocation;
use std::{
    cell::RefCell,
    io,
    sync::{Arc, Weak},
};

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
    Changed(Arc<RowChanges>, Acquisition),
}

/// Changes this thread found lately between two allocations, which another
/// log keeping the same frames takes rather than compare the tables again.
/// Weak identities keep both addresses from being reused while it is held,
/// and only the logs keeping the changes keep them alive.
struct Found {
    before: Weak<TableView>,
    now: Weak<TableView>,
    changes: Weak<RowChanges>,
}

thread_local! {
    static FOUND: RefCell<Vec<Found>> = const { RefCell::new(Vec::new()) };
}

/// Changes found at once by other logs on this thread at most.
const FOUND_KEPT: usize = 16;

/// `now`'s rows' changes since `before`'s, found on this thread before or now.
fn changes_between(before: &SharedView, now: &SharedView) -> io::Result<Option<Arc<RowChanges>>> {
    let (TableView::Complete(old), TableView::Complete(rows)) = (&*before.0, &*now.0) else {
        return Ok(None);
    };
    let is = |found: &Found| {
        std::ptr::eq(found.before.as_ptr(), Arc::as_ptr(&before.0))
            && std::ptr::eq(found.now.as_ptr(), Arc::as_ptr(&now.0))
    };
    if let Some(changes) = FOUND.with_borrow(|found| {
        found
            .iter()
            .find(|found| is(found))
            .and_then(|found| found.changes.upgrade())
    }) {
        crate::cpu::logged("diff-shared", rows.len());
        return Ok(Some(changes));
    }
    let Some(changes) = rows.changes_since(old)? else {
        return Ok(None);
    };
    crate::cpu::logged("diff", rows.len());
    let changes = Arc::new(changes);
    FOUND.with_borrow_mut(|found| -> io::Result<()> {
        found.retain(|found| {
            found.before.strong_count() != 0
                && found.now.strong_count() != 0
                && found.changes.strong_count() != 0
        });
        if found.len() == FOUND_KEPT {
            found.remove(0);
        }
        allocation::reserve(found, 1)?;
        found.push(Found {
            before: Arc::downgrade(&before.0),
            now: Arc::downgrade(&now.0),
            changes: Arc::downgrade(&changes),
        });
        Ok(())
    })?;
    Ok(Some(changes))
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
                        match changes_between(before, now)? {
                            Some(changes) => Kept::Changed(changes, Acquisition::of(now)),
                            None => Kept::View(now.clone()),
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
    /// changed in place once the frame before is no longer held. A reader
    /// holding it makes it a copy, and the view so replaced is kept until the
    /// next frame, to catch up in place there once the reader lets it go.
    pub fn into_frames(self) -> LoggedFrames {
        LoggedFrames {
            entries: self.entries.into_iter(),
            current: None,
            spares: Vec::new(),
            spared: Vec::new(),
        }
    }
}

/// [`FrameLog::into_frames`].
pub struct LoggedFrames {
    entries: std::vec::IntoIter<Entry>,
    current: Option<Box<[SharedView; TABLE_COUNT]>>,
    spares: Vec<Option<Spare>>,
    /// The tables with a spare, which serves the next frame only.
    spared: Vec<usize>,
}

/// A view read back that a reader held when the next one was built from it,
/// and the changes that built the next one.
struct Spare {
    view: SharedView,
    changes: RowChanges,
    made: Weak<TableView>,
}

impl Spare {
    /// `current`'s rows, owned, when `current` is the view these changes made
    /// and nothing else holds the spare's view.
    fn take_rows(self, current: &SharedView) -> io::Result<Option<TableRows>> {
        let Self {
            view,
            changes,
            made,
        } = self;
        if !std::ptr::eq(made.as_ptr(), Arc::as_ptr(&current.0)) {
            return Ok(None);
        }
        match view.try_unwrap() {
            Ok(TableView::Complete(rows)) => rows.apply_changes(changes).map(Some),
            _ => Ok(None),
        }
    }
}

impl LoggedFrames {
    fn rebuild(&mut self, entry: Entry) -> io::Result<DecodedFrame> {
        let current = self
            .current
            .get_or_insert_with(|| Box::new(std::array::from_fn(|_| not_selected())));
        if self.spares.is_empty() {
            self.spares = allocation::vec(TABLE_COUNT)?;
            self.spares.resize_with(TABLE_COUNT, || None);
            self.spared = allocation::vec(TABLE_COUNT)?;
        }
        for index in self.spared.drain(..) {
            if entry
                .tables
                .binary_search_by_key(&index, |(table, _)| *table)
                .is_err()
            {
                self.spares[index] = None;
            }
        }
        for (index, kept) in entry.tables {
            let spare = self.spares[index].take();
            current[index] = match kept {
                Kept::View(view) => view,
                Kept::Same(acquisition) => acquisition.on(current[index].clone()),
                Kept::Changed(changes, acquisition) => {
                    let incomplete = || super::invalid("kept rows changed an incomplete view");
                    let changes = Arc::try_unwrap(changes).or_else(|shared| shared.try_clone())?;
                    let before = std::mem::replace(&mut current[index], not_selected());
                    let (rows, replaced) = match before.try_unwrap() {
                        Ok(TableView::Complete(rows)) => {
                            crate::cpu::logged("rebuild-in-place", rows.len());
                            (rows, None)
                        }
                        Ok(_) => return Err(incomplete()),
                        Err(shared) => {
                            let TableView::Complete(rows) = &*shared.0 else {
                                return Err(incomplete());
                            };
                            let caught = match spare {
                                Some(spare) => spare.take_rows(&shared)?,
                                None => None,
                            };
                            let rows = match caught {
                                Some(rows) => {
                                    crate::cpu::logged("rebuild-reused", rows.len());
                                    rows
                                }
                                None => {
                                    crate::cpu::logged("rebuild-copied", rows.len());
                                    rows.try_clone_validated()?
                                }
                            };
                            (rows, Some(shared))
                        }
                    };
                    let kept = match &replaced {
                        Some(_) => Some(changes.try_clone()?),
                        None => None,
                    };
                    let view: SharedView = TableView::Complete(rows.apply_changes(changes)?).into();
                    if let (Some(view_before), Some(changes)) = (replaced, kept) {
                        self.spares[index] = Some(Spare {
                            view: view_before,
                            changes,
                            made: Arc::downgrade(&view.0),
                        });
                        self.spared.push(index);
                    }
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
