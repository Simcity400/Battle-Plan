//! Runtime facts shared by all preparation observers of one admitted frame.
use replay_data::rows::TABLE_COUNT;
use replay_data::{BranchId, Hour, format::FormatVersion, rows::DeltaRow, rows::TableId};
use replay_frame::{
    frame::DecodedFrame, frame::IntoRecordedFrame, frame::RecordedFrame, frame::TableUpdate,
    frame::TableView, frame::ViewIdentity,
};
use std::sync::Arc;

/// Row identity and read evidence are separate: a fresh equal acquisition
/// keeps its allocation, so its read shows only in table_read/slot_read.
/// Owners borrow rows, status and supplying dates from the same decoded frame.
/// Boundary change flags are conservative, and never invent a read.
#[derive(replay_spool::Spooled)]
pub struct FrameChanges {
    pub rows_changed: [bool; TABLE_COUNT],
    pub table_read: [bool; TABLE_COUNT],
    pub slot_read: [bool; TABLE_COUNT],
    /// The hour's recorded updates, when it continues the previous hour.
    pub updates: Option<RowUpdates>,
}

/// An hour's recorded updates and the tables whose decoded rows are the
/// previous hour's changed by exactly their delta.
#[derive(replay_spool::Spooled)]
pub struct RowUpdates {
    frame: Arc<RecordedFrame>,
    exact: [bool; TABLE_COUNT],
}

/// The change-mask bits of `table`'s fields named `names`, for testing a
/// changed row's mask from [`FrameChanges::rows`]; every bit when the table
/// does not name each of them, so a renamed field is never missed.
pub fn field_mask(table: TableId, names: &[&str]) -> u64 {
    let mask = (0..64)
        .filter(|&index| {
            table
                .field_name(index)
                .is_some_and(|name| names.contains(&name))
        })
        .fold(0_u64, |mask, index| mask | (1_u64 << index));
    if mask.count_ones() as usize == names.len() {
        mask
    } else {
        u64::MAX
    }
}

/// Compare ordered snapshot rows once when no recorded delta is available.
/// Callers retain these keys until their next supplying read consumes them.
pub fn differing_keys<T: replay_data::rows::RowCodec>(
    before: &[T],
    now: &[T],
) -> std::collections::BTreeSet<u128> {
    let mut changed = std::collections::BTreeSet::new();
    let (mut a, mut b) = (0, 0);
    while a < before.len() || b < now.len() {
        match (before.get(a), now.get(b)) {
            (Some(old), Some(new)) if old.key() == new.key() => {
                if old != new {
                    changed.insert(new.key());
                }
                a += 1;
                b += 1;
            }
            (Some(old), new) if new.is_none_or(|new| old.key() < new.key()) => {
                changed.insert(old.key());
                a += 1;
            }
            (_, Some(new)) => {
                changed.insert(new.key());
                b += 1;
            }
            _ => unreachable!(),
        }
    }
    changed
}

impl FrameChanges {
    /// The hour's recorded updates. Readers share immutable update cells;
    /// keeping these facts retains this hour's cells, not its entire block.
    pub fn recorded(&self) -> Option<&Arc<RecordedFrame>> {
        self.updates.as_ref().map(|updates| &updates.frame)
    }

    /// The facts of an hour another reader observed: its tables' change, read and slot-read
    /// flags, and its recorded updates with the tables each changed exactly by (`exact`).
    pub fn observed(
        rows_changed: [bool; TABLE_COUNT],
        table_read: [bool; TABLE_COUNT],
        slot_read: [bool; TABLE_COUNT],
        updates: Option<(Arc<RecordedFrame>, [bool; TABLE_COUNT])>,
    ) -> Self {
        Self {
            rows_changed,
            table_read,
            slot_read,
            updates: updates.map(|(frame, exact)| RowUpdates { frame, exact }),
        }
    }

    /// Whether `table` changed exactly by its recorded delta this hour.
    pub fn exact(&self, table: TableId) -> bool {
        self.updates
            .as_ref()
            .is_some_and(|updates| updates.exact[table.index()])
    }

    /// The facts of an hour observed with no hour before it, as preparation
    /// observes the first hour of every block: any row may have changed.
    pub fn alone(frame: &DecodedFrame, format: FormatVersion) -> Self {
        let _cpu = ::replay_frame::cpu::scope("frame changes", None);
        Self::alone_inner(frame, format)
    }

    fn alone_inner(frame: &DecodedFrame, format: FormatVersion) -> Self {
        Self {
            rows_changed: [true; TABLE_COUNT],
            table_read: std::array::from_fn(|index| frame.metrics.table_ns[index] != 0),
            slot_read: crate::metadata::slot_reads(format, &frame.metrics, frame.hour),
            updates: None,
        }
    }

    /// The facts of an hour of a block read by its changes
    /// (`Replay::read_block_changes`), equal to those a tracker of the block
    /// read alone finds: when the hour follows the hour before (`continues`),
    /// a table changed exactly when the hour's update rebuilt it (`rebuilt`,
    /// the reader's evidence), which is when that tracker sees a new
    /// allocation; after a gap, every table, as [`Self::alone`]. The hour's
    /// recorded updates are not kept.
    pub fn rebuilt(
        frame: &DecodedFrame,
        format: FormatVersion,
        continues: bool,
        rebuilt: impl Fn(TableId) -> bool,
    ) -> Self {
        let _cpu = ::replay_frame::cpu::scope("frame changes", None);
        let mut changes = Self::alone_inner(frame, format);
        if continues {
            changes.rows_changed = TableId::ALL.map(rebuilt);
        }
        changes
    }

    /// The rows of `table` that changed this hour, each with the fields that
    /// changed, and the keys it deleted. None when any row may have changed:
    /// the table was rebuilt rather than changed by one delta from the
    /// previous hour, or an observer marked it changed itself.
    pub fn rows(&self, table: TableId) -> Option<(&[DeltaRow], &[u128])> {
        let index = table.index();
        if !self.rows_changed[index] {
            return Some((&[], &[]));
        }
        let updates = self
            .updates
            .as_ref()
            .filter(|updates| updates.exact[index])?;
        match &updates.frame.tables[index] {
            TableUpdate::Delta {
                changed, deleted, ..
            } => Some((changed, deleted)),
            _ => None,
        }
    }
}

/// The hour's changes from the hour before as a trigger evaluation's
/// inputs read them ([`crate::triggers::inputs::Since`]): an unchanged
/// table is the same view, a table changed by one delta changed exactly its
/// rows, and any other table anything.
impl crate::triggers::inputs::TableChanges for FrameChanges {
    fn table(&self, table: TableId) -> crate::triggers::inputs::TableChange<'_> {
        use crate::triggers::inputs::TableChange;
        if !self.rows_changed[table.index()] {
            return TableChange::Same;
        }
        match self.rows(table) {
            Some((changed, deleted)) => TableChange::Rows(changed, deleted),
            None => TableChange::Any,
        }
    }
}

struct Previous {
    tables: [ViewIdentity; TABLE_COUNT],
    branch: BranchId,
    hour: Hour,
}

/// Weak stamps keep no rows alive. They also prevent address reuse while the
/// comparison is pending. No frame/history or persisted observer state lives here.
pub struct FrameChangeTracker {
    previous: Option<Previous>,
}

impl Default for FrameChangeTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameChangeTracker {
    pub fn new() -> Self {
        Self { previous: None }
    }

    /// Every restored observer needs its first native join, even when the
    /// decoder carries equal rows across a same-branch publication boundary.
    pub fn invalidate(&mut self) {
        self.previous = None;
    }

    /// [`Self::observe`] with the hour's recorded updates, which give each
    /// changed table's rows ([`FrameChanges::rows`]).
    pub fn observe_updates(
        &mut self,
        frame: &DecodedFrame,
        updates: impl IntoRecordedFrame,
        format: FormatVersion,
    ) -> FrameChanges {
        let _cpu = ::replay_frame::cpu::scope("frame changes", None);
        let updates = updates.into_recorded();
        let continuous = self.continues(frame);
        let mut changes = self.observe_inner(frame, format);
        if continuous && updates.branch == frame.branch && updates.hour == frame.hour {
            // The decoder applies a delta to the previous hour's rows; a
            // failed one leaves the table unavailable instead.
            let exact = std::array::from_fn(|index| {
                matches!(updates.tables[index], TableUpdate::Delta { .. })
                    && matches!(*frame.tables[index], TableView::Complete(_))
            });
            changes.updates = Some(RowUpdates {
                frame: updates,
                exact,
            });
        }
        changes
    }

    fn continues(&self, frame: &DecodedFrame) -> bool {
        self.previous.as_ref().is_some_and(|previous| {
            previous.branch == frame.branch && previous.hour.checked_add(1) == Some(frame.hour)
        })
    }

    pub fn observe(&mut self, frame: &DecodedFrame, format: FormatVersion) -> FrameChanges {
        let _cpu = ::replay_frame::cpu::scope("frame changes", None);
        self.observe_inner(frame, format)
    }

    fn observe_inner(&mut self, frame: &DecodedFrame, format: FormatVersion) -> FrameChanges {
        #[cfg(any(test, feature = "test-support"))]
        OBSERVATIONS.with(|count| count.set(count.get() + 1));
        let continuous = self.previous.as_ref().filter(|previous| {
            previous.branch == frame.branch && previous.hour.checked_add(1) == Some(frame.hour)
        });
        let changes = FrameChanges {
            // Equal allocation means equal TableView, including rows, status,
            // missing counts and refusals. SharedView writes use Arc::make_mut
            // (frame.rs:171-174); decoder deltas construct a new SharedView
            // (frame/decoder.rs:192-205), while Unchanged returns its prior view
            // (:213). Weak identities cannot match a reused allocation.
            rows_changed: std::array::from_fn(|index| {
                continuous.is_none_or(|previous| !previous.tables[index].is(&frame.tables[index]))
            }),
            // READ_TIME_NOT_RECORDED is nonzero evidence, never a duration.
            table_read: std::array::from_fn(|index| frame.metrics.table_ns[index] != 0),
            slot_read: crate::metadata::slot_reads(format, &frame.metrics, frame.hour),
            updates: None,
        };
        self.previous = Some(Previous {
            tables: std::array::from_fn(|index| frame.tables[index].identity()),
            branch: frame.branch,
            hour: frame.hour,
        });
        changes
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static OBSERVATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(any(test, feature = "test-support"))]
pub fn observation_count() -> usize {
    OBSERVATIONS.with(std::cell::Cell::get)
}

#[cfg(test)]
mod tests;
