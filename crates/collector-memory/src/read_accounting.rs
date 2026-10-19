//! Per-frame native read work, independent of copied payloads and wall time.
//!
//! CountedMemory wraps the native source below caches. These thread-local
//! scopes assign its counters without allocating or synchronizing per read.
//! Shared country authentication belongs to Countries; the shared unit census
//! belongs to Divisions. Nested detail scopes retain their existing timing
//! anchors, so consumers of owned bytes never charge the source work again.
//!
//! A Capture isolates each executor job, including callback helping and rejected
//! inline work. Collection merges its counts once, even after a caught panic.
//! The outer callback Capture includes the sole hook-entry clock and diagnostic
//! scalar reads under Clock, and the once-only player roster under Players.

use engine_native::memory::ReadCounts;
use replay_data::rows::TABLE_COUNT;
use replay_data::rows::TableId;
use std::{cell::RefCell, marker::PhantomData, mem::MaybeUninit, rc::Rc};

pub type TableCounts = [ReadCounts; TABLE_COUNT];
#[allow(
    clippy::large_const_arrays,
    reason = "The zero counts also initialize thread-local state in a const context."
)]
pub const ZERO: TableCounts = [ReadCounts::ZERO; TABLE_COUNT];

struct State {
    tables: TableCounts,
    current: usize,
}

impl State {
    const fn new() -> Self {
        Self {
            tables: ZERO,
            current: TableId::Clock.index(),
        }
    }

    fn flush(&mut self) {
        self.tables[self.current] += ReadCounts::take_current();
    }
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State::new()) };
}

/// Its pending counters and parent table belong to the opening thread.
#[must_use]
pub struct Table {
    previous: usize,
    _thread: PhantomData<Rc<()>>,
}

pub fn table(table: TableId) -> Table {
    let previous = STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.flush();
        std::mem::replace(&mut state.current, table.index())
    });
    Table {
        previous,
        _thread: PhantomData,
    }
}

impl Table {
    /// Switch a timed shared walk's phase while retaining its original parent.
    pub fn select(&mut self, table: TableId) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            if state.current != table.index() {
                state.flush();
                state.current = table.index();
            }
        });
    }
}

impl Drop for Table {
    fn drop(&mut self) {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.flush();
            state.current = self.previous;
        });
    }
}

pub fn with_table<R>(owner: TableId, work: impl FnOnce() -> R) -> R {
    let _reads = table(owner);
    work()
}

/// Keep this guard outside the job's panic boundary. Its table guards unwind
/// inside that boundary, so finish still returns all attempted native work.
/// The parent's counts are saved and restored in place: no second set of
/// counts is ever built on the opening thread's stack.
#[must_use]
pub struct Capture {
    previous: MaybeUninit<State>,
    open: bool,
    _thread: PhantomData<Rc<()>>,
}

impl Default for Capture {
    fn default() -> Self {
        Self::new()
    }
}

impl Capture {
    pub fn new() -> Self {
        let mut capture = Self {
            previous: MaybeUninit::uninit(),
            open: false,
            _thread: PhantomData,
        };
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.flush();
            let previous = capture.previous.as_mut_ptr();
            // SAFETY: every field of `previous` is written before `open` is set.
            unsafe {
                std::ptr::addr_of_mut!((*previous).tables).write(state.tables);
                std::ptr::addr_of_mut!((*previous).current).write(state.current);
            }
            state.tables = ZERO;
            state.current = TableId::Clock.index();
        });
        capture.open = true;
        capture
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn finish(mut self) -> TableCounts {
        let mut tables = ZERO;
        self.restore(Some(&mut tables));
        tables
    }

    /// `finish` into the caller's storage, without a copy on this stack.
    pub fn finish_into(mut self, tables: &mut TableCounts) {
        self.restore(Some(tables));
    }

    /// Restore the parent; this capture's counts go to `tables` when given.
    fn restore(&mut self, tables: Option<&mut TableCounts>) {
        if !std::mem::take(&mut self.open) {
            return;
        }
        // SAFETY: `open` was set only after `new` initialized `previous`.
        let previous = unsafe { self.previous.assume_init_ref() };
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            state.flush();
            if let Some(tables) = tables {
                *tables = state.tables;
            }
            state.tables = previous.tables;
            state.current = previous.current;
        });
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        // Discarded counts are never copied out.
        self.restore(None);
    }
}

/// Merge a collected completion after restoring its caller's Capture. Borrowed,
/// so a collector merges in place without copying the counts onto its stack.
pub fn merge(counts: &TableCounts) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        for (total, extra) in state.tables.iter_mut().zip(counts) {
            *total += *extra;
        }
    });
}

/// Called after all worker collections and live reads, before frame ownership
/// transfers to the decode queue. Taking another snapshot never adds work.
#[cfg(any(test, feature = "test-support"))]
pub fn snapshot() -> TableCounts {
    let mut tables = ZERO;
    snapshot_into(&mut tables);
    tables
}

/// `snapshot` into the caller's storage, without a copy on this stack.
pub fn snapshot_into(tables: &mut TableCounts) {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.flush();
        *tables = state.tables;
    })
}

#[cfg(any(test, feature = "test-support"))]
pub mod fixtures {
    use super::ReadCounts;
    use engine_native::{Memory, memory::CountedMemory, memory::SliceMemory};

    pub fn read(len: usize) {
        let source = [7; 32];
        let memory = CountedMemory::new(SliceMemory {
            base: 0x1000,
            bytes: &source,
        });
        assert!(memory.copy(0x1000, &mut [0; 32][..len]));
    }

    pub fn copies(calls: u64, bytes: u64) -> ReadCounts {
        ReadCounts {
            copy_calls: calls,
            spans: calls,
            bytes,
            ..ReadCounts::ZERO
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{copies, read};
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn repeated_phase_switches_and_nested_visitors_partition_every_request_once() {
        let capture = Capture::new();
        read(1);
        {
            let mut phase = table(TableId::Fleets);
            for _ in 0..2 {
                read(2);
                phase.select(TableId::Fleets);
                phase.select(TableId::TaskForces);
                read(3);
                phase.select(TableId::TaskForces);
                phase.select(TableId::Ships);
                read(4);
                with_table(TableId::ShipModels, || read(5));
                read(6);
                phase.select(TableId::Fleets);
                read(7);
            }
        }
        read(8);
        let actual = capture.finish();
        let mut expected = ZERO;
        expected[TableId::Clock.index()] = copies(2, 9);
        expected[TableId::Fleets.index()] = copies(4, 18);
        expected[TableId::TaskForces.index()] = copies(2, 6);
        expected[TableId::Ships.index()] = copies(4, 20);
        expected[TableId::ShipModels.index()] = copies(2, 10);
        assert_eq!(actual, expected);
    }

    #[test]
    fn callback_helping_is_isolated_until_collection_and_panic_keeps_attempted_reads() {
        let frame = Capture::new();
        let owner = table(TableId::Countries);
        read(2);
        let job = Capture::new();
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _job_owner = table(TableId::Divisions);
            read(3);
            with_table(TableId::DivisionEquipment, || read(4));
            panic!("synthetic runner panic after reads");
        }));
        assert!(result.is_err());
        let job_counts = job.finish();
        assert_eq!(job_counts[TableId::Divisions.index()], copies(1, 3));
        assert_eq!(job_counts[TableId::DivisionEquipment.index()], copies(1, 4));
        assert_eq!(snapshot()[TableId::Divisions.index()], ReadCounts::ZERO);
        read(5);
        merge(&job_counts);
        // Actual job table guards unwind before their executor Capture ends.
        drop(owner);
        let mut expected = job_counts;
        expected[TableId::Countries.index()] = copies(2, 7);
        assert_eq!(frame.finish(), expected);
    }

    #[test]
    fn discarded_capture_restores_pending_parent_and_does_not_leak_to_next_frame() {
        let parent = Capture::new();
        read(2);
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let _discarded = Capture::new();
            let _owner = table(TableId::Players);
            read(3);
            panic!("synthetic dropped callback");
        }));
        assert!(outcome.is_err());
        read(4);
        let next = Capture::new();
        assert_eq!(snapshot(), ZERO);
        read(5);
        let mut expected = ZERO;
        expected[TableId::Clock.index()] = copies(1, 5);
        assert_eq!(next.finish(), expected);
        expected[TableId::Clock.index()] = copies(2, 6);
        assert_eq!(parent.finish(), expected);
    }
}
