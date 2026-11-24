
use engine_native_memory::memory::ReadCounts;
use replay_data::rows::TABLE_COUNT;
use replay_data::rows::TableId;
use std::{
    cell::RefCell,
    marker::PhantomData,
    rc::Rc,
    sync::atomic::{AtomicUsize, Ordering},
};

pub type TableCounts = [ReadCounts; TABLE_COUNT];
#[expect(
    clippy::large_const_arrays,
    reason = "The zero counts also initialize thread-local state in a const context."
)]
pub const ZERO: TableCounts = [ReadCounts::ZERO; TABLE_COUNT];

struct State {
    current: usize,
}

impl State {
    const fn new() -> Self {
        Self {
            current: TableId::Clock.index(),
        }
    }

    fn flush(&mut self) {
        #[cfg(feature = "acquisition_profile")]
        crate::acquisition_profile::boundary(self.current, ReadCounts::take_current());
        if let Some(key) = engine_sites::take_taint() {
            withhold(self.current, key);
        }
    }
}

static WITHHELD: [AtomicUsize; TABLE_COUNT] = [const { AtomicUsize::new(0) }; TABLE_COUNT];

fn withhold(table: usize, key: engine_sites::Rva) {
    if table != TableId::Clock.index() {
        let _ = WITHHELD[table].compare_exchange(0, key.0 + 1, Ordering::AcqRel, Ordering::Acquire);
    }
}

pub fn withheld(table: TableId) -> Option<engine_sites::Rva> {
    match WITHHELD[table.index()].load(Ordering::Acquire) {
        0 => None,
        key => Some(engine_sites::Rva(key - 1)),
    }
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State::new()) };
}

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

#[must_use]
pub struct Scope {
    previous: usize,
    _thread: PhantomData<Rc<()>>,
}

impl Default for Scope {
    fn default() -> Self {
        Self::new()
    }
}

impl Scope {
    pub fn new() -> Self {
        let previous = STATE.with_borrow_mut(|state| {
            state.flush();
            std::mem::replace(&mut state.current, TableId::Clock.index())
        });
        #[cfg(feature = "acquisition_profile")]
        crate::acquisition_profile::enter();
        Self {
            previous,
            _thread: PhantomData,
        }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        STATE.with_borrow_mut(|state| {
            state.flush();
            state.current = self.previous;
        });
        #[cfg(feature = "acquisition_profile")]
        crate::acquisition_profile::leave();
    }
}

#[cfg(any(test, feature = "test-support", feature = "acquisition_profile"))]
mod counted {
    use super::*;
    use engine_native_memory::read_counts::{ReadChange, ReadObserver};

    thread_local! {
        static COUNTS: RefCell<TableCounts> = const { RefCell::new(ZERO) };
    }

    fn record(change: ReadChange) {
        let table = STATE.with_borrow(|state| state.current);
        COUNTS.with_borrow_mut(|counts| match change {
            ReadChange::Add(reads) => counts[table] += reads,
            ReadChange::Defer(reads) => counts[table] -= reads,
        });
    }

    #[must_use]
    pub struct Capture {
        _scope: Scope,
        previous: Option<(TableCounts, Option<ReadObserver>)>,
    }

    impl Default for Capture {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Capture {
        pub fn new() -> Self {
            let scope = Scope::new();
            #[cfg(not(feature = "acquisition_profile"))]
            let _ = ReadCounts::take_current();
            let counts = COUNTS.replace(ZERO);
            let observer = ReadCounts::replace_observer(Some(record));
            Self {
                _scope: scope,
                previous: Some((counts, observer)),
            }
        }

        pub fn finish(mut self) -> TableCounts {
            let mut tables = ZERO;
            self.restore(Some(&mut tables));
            tables
        }

        pub fn finish_into(mut self, tables: &mut TableCounts) {
            self.restore(Some(tables));
        }

        fn restore(&mut self, tables: Option<&mut TableCounts>) {
            if let Some((previous, observer)) = self.previous.take() {
                #[cfg(not(feature = "acquisition_profile"))]
                let _ = ReadCounts::take_current();
                ReadCounts::replace_observer(observer);
                COUNTS.with_borrow_mut(|counts| {
                    if let Some(tables) = tables {
                        *tables = *counts;
                    }
                    *counts = previous;
                });
            }
        }
    }

    impl Drop for Capture {
        fn drop(&mut self) {
            self.restore(None);
        }
    }

    pub fn merge(counts: &TableCounts) {
        COUNTS.with_borrow_mut(|totals| {
            for (total, extra) in totals.iter_mut().zip(counts) {
                *total += *extra;
            }
        });
    }

    pub fn snapshot() -> TableCounts {
        let mut tables = ZERO;
        snapshot_into(&mut tables);
        tables
    }

    pub fn snapshot_into(tables: &mut TableCounts) {
        STATE.with_borrow_mut(State::flush);
        COUNTS.with_borrow(|counts| *tables = *counts);
    }
}

#[cfg(any(test, feature = "test-support", feature = "acquisition_profile"))]
pub use counted::{Capture, merge, snapshot, snapshot_into};

#[cfg(any(test, feature = "test-support"))]
pub mod fixtures {
    use super::{Capture, ReadCounts, TableCounts, ZERO};
    use engine_native_memory::{Memory, memory::{CountedMemory, SliceMemory}};
    use std::{collections::HashMap, hash::Hash, sync::Mutex};

    pub struct ReadTrace<K>(Mutex<HashMap<K, TableCounts>>);

    impl<K> Default for ReadTrace<K> {
        fn default() -> Self {
            Self(Mutex::default())
        }
    }

    impl<K: Eq + Hash> ReadTrace<K> {
        pub fn record<R>(&self, key: K, run: impl FnOnce() -> R) -> R {
            let _record = Recorded {
                trace: self,
                capture: Some(Capture::new()),
                key: Some(key),
            };
            run()
        }

        pub fn take(&self, key: K) -> TableCounts {
            self.0.lock().unwrap().remove(&key).unwrap_or(ZERO)
        }
    }

    struct Recorded<'a, K: Eq + Hash> {
        trace: &'a ReadTrace<K>,
        capture: Option<Capture>,
        key: Option<K>,
    }

    impl<K: Eq + Hash> Drop for Recorded<'_, K> {
        fn drop(&mut self) {
            let counts = self.capture.take().unwrap().finish();
            #[cfg(feature = "acquisition_profile")]
            super::merge(&counts);
            let mut by_key = self.trace.0.lock().unwrap();
            let total = by_key.entry(self.key.take().unwrap()).or_insert(ZERO);
            for (total, counts) in total.iter_mut().zip(counts) {
                *total += counts;
            }
        }
    }

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
    fn deferred_plan_reads_only_count_under_the_destination_table() {
        let capture = Capture::new();
        let mut table = table(TableId::ArmyOrders);
        read(3);
        let census_reads = ReadCounts::take_current();
        read(7);
        use engine_native_memory::{Memory, memory::{CountedMemory, SliceMemory}};

        let memory = CountedMemory::new(SliceMemory {
            base: 0,
            bytes: &[],
        });
        let plan_reads = memory.take_deferred_reads();
        census_reads.add_current();
        read(5);
        table.select(TableId::ArmyOrderPlans);
        memory.add_deferred_reads(plan_reads);
        read(11);
        drop(table);
        let mut expected = ZERO;
        expected[TableId::ArmyOrders.index()] = copies(2, 8);
        expected[TableId::ArmyOrderPlans.index()] = copies(2, 18);
        assert_eq!(capture.finish(), expected);
    }

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
