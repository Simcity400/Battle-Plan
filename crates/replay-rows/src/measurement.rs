use std::{
    cell::Cell,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

static ENABLED: AtomicBool = AtomicBool::new(false);
static COUNTS: OnceLock<Vec<AtomicU64>> = OnceLock::new();
static SOURCES: OnceLock<Vec<AtomicU64>> = OnceLock::new();
static TABLES: AtomicUsize = AtomicUsize::new(0);
const NONE: usize = usize::MAX;
thread_local! { static TABLE: Cell<usize> = const { Cell::new(NONE) }; }

pub trait Table: Copy {
    fn position(self) -> usize;
    fn at(position: usize) -> Option<Self>;
}

#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Counter {
    CandidateHours,
    CandidateKeys,
    CandidateRows,
    WholeHours,
    WholeRows,
    RelevantSourceRows,
    RelevantSourceFields,
    PredictedRows,
    PredictedFields,
    NativeOpaqueBytes,
    ShiftedRows,
    MergedRows,
    ValidatedRows,
    KeyCopyBytes,
    ModelCopyBytes,
    DirectRetries,
    SnapshotCopies,
    SnapshotRows,
    AnchorCopies,
    AnchorRows,
    AnchorShadowBytes,
    CandidateCloneBytes,
    WholeCloneBytes,
}
impl Counter {
    pub const ALL: [Self; 23] = [
        Self::CandidateHours,
        Self::CandidateKeys,
        Self::CandidateRows,
        Self::WholeHours,
        Self::WholeRows,
        Self::RelevantSourceRows,
        Self::RelevantSourceFields,
        Self::PredictedRows,
        Self::PredictedFields,
        Self::NativeOpaqueBytes,
        Self::ShiftedRows,
        Self::MergedRows,
        Self::ValidatedRows,
        Self::KeyCopyBytes,
        Self::ModelCopyBytes,
        Self::DirectRetries,
        Self::SnapshotCopies,
        Self::SnapshotRows,
        Self::AnchorCopies,
        Self::AnchorRows,
        Self::AnchorShadowBytes,
        Self::CandidateCloneBytes,
        Self::WholeCloneBytes,
    ];
    pub const NAMES: [&'static str; 23] = [
        "candidateHours",
        "candidateKeys",
        "candidateRows",
        "wholeHours",
        "wholeRows",
        "relevantSourceRows",
        "relevantSourceFields",
        "logicalPredictedRows",
        "logicalPredictedFields",
        "nativeOpaqueBytes",
        "shiftedRows",
        "mergedRows",
        "validatedRows",
        "keyCopyBytes",
        "modelCopyBytes",
        "directRetries",
        "snapshotCopies",
        "snapshotRows",
        "anchorCopies",
        "anchorRows",
        "anchorShadowBytes",
        "candidateInlineCopyBytes",
        "wholeInlineCopyBytes",
    ];
}
#[inline]
pub fn enabled() -> bool {
    #[cfg(any(test, feature = "test-support"))]
    if OBSERVATION.with(|value| value.borrow().is_some()) {
        return true;
    }
    ENABLED.load(Ordering::Acquire)
}
pub fn enable(tables: usize) {
    TABLES.store(tables, Ordering::Release);
    COUNTS.get_or_init(|| {
        (0..(tables + 1) * Counter::ALL.len())
            .map(|_| AtomicU64::new(0))
            .collect()
    });
    SOURCES.get_or_init(|| (0..tables * tables).map(|_| AtomicU64::new(0)).collect());
    ENABLED.store(true, Ordering::Release);
}
pub struct TableScope(Option<usize>);
impl Drop for TableScope {
    fn drop(&mut self) {
        if let Some(before) = self.0 {
            TABLE.set(before);
        }
    }
}
#[inline]
pub fn table(table: impl Table) -> TableScope {
    TableScope(enabled().then(|| TABLE.replace(table.position())))
}
#[inline]
pub fn count(counter: Counter, value: usize) {
    if enabled() && value != 0 {
        #[cfg(any(test, feature = "test-support"))]
        if OBSERVATION.with(|observation| {
            let mut observation = observation.borrow_mut();
            let Some(observation) = observation.as_mut() else {
                return false;
            };
            *observation
                .counts
                .entry((TABLE.get(), counter as usize))
                .or_default() += value as u64;
            true
        }) {
            return;
        }
        let table = match TABLE.get() {
            NONE => TABLES.load(Ordering::Relaxed),
            table => table,
        };
        COUNTS.get().unwrap()[table * Counter::ALL.len() + counter as usize]
            .fetch_add(value as u64, Ordering::Relaxed);
    }
}
pub fn source<T: Table>(target: T, source: T, mask: u64) {
    if enabled() && mask != 0 {
        let _table = table(target);
        #[cfg(any(test, feature = "test-support"))]
        let captured = OBSERVATION.with(|observation| {
            let mut observation = observation.borrow_mut();
            let Some(observation) = observation.as_mut() else {
                return false;
            };
            *observation
                .sources
                .entry((target.position(), source.position()))
                .or_default() |= mask;
            true
        });
        #[cfg(not(any(test, feature = "test-support")))]
        let captured = false;
        if !captured {
            SOURCES.get().unwrap()
                [target.position() * TABLES.load(Ordering::Relaxed) + source.position()]
                .fetch_or(mask, Ordering::Relaxed);
        }
        count(Counter::RelevantSourceRows, 1);
        count(Counter::RelevantSourceFields, mask.count_ones() as usize);
    }
}
pub fn snapshot<T: Table>() -> Vec<(Option<T>, Vec<(&'static str, u64)>)> {
    let Some(counts) = COUNTS.get() else {
        return Vec::new();
    };
    (0..=TABLES.load(Ordering::Relaxed))
        .filter_map(|table| {
            let values: Vec<_> = Counter::ALL
                .iter()
                .zip(Counter::NAMES)
                .filter_map(|(&counter, name)| {
                    let value = counts[table * Counter::ALL.len() + counter as usize]
                        .load(Ordering::Relaxed);
                    (value != 0).then_some((name, value))
                })
                .collect();
            (!values.is_empty()).then(|| (T::at(table), values))
        })
        .collect()
}
pub fn source_masks<T: Table>() -> Vec<(T, T, u64)> {
    let tables = TABLES.load(Ordering::Relaxed);
    let Some(masks) = SOURCES.get() else {
        return Vec::new();
    };
    masks
        .iter()
        .enumerate()
        .filter_map(|(index, mask)| {
            let mask = mask.load(Ordering::Relaxed);
            (mask != 0).then(|| {
                (
                    T::at(index / tables).unwrap(),
                    T::at(index % tables).unwrap(),
                    mask,
                )
            })
        })
        .collect()
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static OBSERVATION: std::cell::RefCell<Option<Observation>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
#[doc(hidden)]
pub struct Observation {
    counts: std::collections::HashMap<(usize, usize), u64>,
    sources: std::collections::HashMap<(usize, usize), u64>,
}

#[cfg(any(test, feature = "test-support"))]
impl Observation {
    pub fn counter(&self, table: impl Table, counter: Counter) -> u64 {
        self.counts
            .get(&(table.position(), counter as usize))
            .copied()
            .unwrap_or(0)
    }

    pub fn source_mask<T: Table>(&self, target: T, source: T) -> u64 {
        self.sources
            .get(&(target.position(), source.position()))
            .copied()
            .unwrap_or(0)
    }
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn capture<T>(work: impl FnOnce() -> T) -> (T, Observation) {
    struct Restore(Option<Observation>);
    impl Drop for Restore {
        fn drop(&mut self) {
            OBSERVATION.set(self.0.take());
        }
    }
    let _restore = Restore(OBSERVATION.replace(Some(Observation::default())));
    let result = work();
    (result, OBSERVATION.take().unwrap())
}
