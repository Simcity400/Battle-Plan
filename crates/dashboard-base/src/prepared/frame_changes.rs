//! Runtime facts shared by all preparation observers of one admitted frame.
use replay::{
    BranchId, Hour,
    format::FormatVersion,
    frame::{DecodedFrame, TABLE_COUNT, ViewIdentity},
    rows::TableId,
};

/// Row identity and read evidence are separate: a fresh equal acquisition
/// keeps its allocation, so its read shows only in table_read/slot_read.
/// Owners borrow rows, status and supplying dates from the same decoded frame.
/// Boundary change flags are conservative, and never invent a read.
pub struct FrameChanges {
    pub rows_changed: [bool; TABLE_COUNT],
    pub table_read: [bool; TABLE_COUNT],
    pub slot_read: [bool; TABLE_COUNT],
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

    pub fn observe(&mut self, frame: &DecodedFrame, format: FormatVersion) -> FrameChanges {
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
            slot_read: TableId::ALL
                .map(|table| crate::metadata::slot_read(format, &frame.metrics, table, frame.hour)),
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
