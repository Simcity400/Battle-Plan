//! Small caches for hour layers that the next hours usually rebuild unchanged.
//!
//! A decoded hour shares each table it did not change with the hour before it
//! (`replay::frame::SharedView`), so "this table did not change" is one pointer
//! comparison. A layer whose value depends only on static content and a fixed
//! list of the hour's tables can therefore be keyed on those tables' identities
//! ([`TableKey`]) and built once for every run of hours that leaves them alone.
//!
//! The list is enforced, not trusted: a keyed layer is built from
//! [`project`]ed frames that hold only the listed tables and mark every other
//! one unavailable. A builder that starts reading an unlisted table sees it
//! missing, in every build, rather than a stale value surviving in the cache.
//! The projected frame keeps the hour's branch and hour, which are not part of
//! the table key: a layer that reads them passes them in its extra key.
//!
//! Some layers read tables that change every hour for columns the layer does
//! not use: `Countries` carries hourly power values beside the tags that rarely
//! change. Their table keys never repeat, so [`Settled`] compares what the
//! layer built instead, and hands on the value an earlier hour built when the
//! two are equal. A later layer keyed on that value's identity then hits for
//! every hour that left the value alone.

use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex},
};

use replay::{
    frame::{DecodedFrame, Metrics, SharedView, TABLE_COUNT, TableView},
    rows::TableId,
};

/// The recent values of one layer, keyed by what they were built from.
pub struct Recent<K, V> {
    entries: Mutex<VecDeque<(K, Arc<Cell<V>>)>>,
    capacity: usize,
}

/// One key's value: being built, built, or failed.
struct Cell<V> {
    state: Mutex<CellState<V>>,
    done: Condvar,
}

enum CellState<V> {
    Building,
    Built(Arc<V>),
    Failed,
}

impl<K: PartialEq, V> Recent<K, V> {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Mutex::new(VecDeque::new()),
            capacity: capacity.max(1),
        }
    }

    /// The value built for `key`, building it when no recent entry has it.
    ///
    /// The first caller of a key builds it with no lock held; a caller that
    /// asks for the same key meanwhile waits for that build instead of doing
    /// the same work, so one key is built once. A failed build is not cached:
    /// its entry is removed and a waiter builds for itself.
    pub fn get_or_insert_with<E>(
        &self,
        key: K,
        build: impl FnOnce() -> Result<Arc<V>, E>,
    ) -> Result<Arc<V>, E> {
        let (cell, builder) = {
            let mut entries = self.lock();
            match entries.iter().position(|(held, _)| *held == key) {
                Some(at) => {
                    // A hit is the most recently used entry.
                    let entry = entries.remove(at).expect("found above");
                    let cell = Arc::clone(&entry.1);
                    entries.push_back(entry);
                    (cell, false)
                }
                None => {
                    let cell = Arc::new(Cell {
                        state: Mutex::new(CellState::Building),
                        done: Condvar::new(),
                    });
                    if entries.len() == self.capacity {
                        entries.pop_front();
                    }
                    entries.push_back((key, Arc::clone(&cell)));
                    (cell, true)
                }
            }
        };
        if !builder {
            let mut state = cell
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            loop {
                match &*state {
                    CellState::Built(value) => return Ok(Arc::clone(value)),
                    CellState::Failed => break,
                    CellState::Building => {
                        state = cell
                            .done
                            .wait(state)
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                    }
                }
            }
            drop(state);
            return build();
        }
        // A panic while building leaves the cell failed rather than waited on.
        let mut guard = FailOnDrop(Some(&cell));
        let built = build();
        guard.0 = None;
        let mut state = cell
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *state = match &built {
            Ok(value) => CellState::Built(Arc::clone(value)),
            Err(_) => CellState::Failed,
        };
        drop(state);
        cell.done.notify_all();
        if built.is_err() {
            self.lock().retain(|(_, held)| !Arc::ptr_eq(held, &cell));
        }
        built
    }

    /// Every built value, most recently used first, each allocation once.
    ///
    /// Taken under the entries' lock, then each cell's (the order every other
    /// path takes them in); a value still building is left out.
    fn distinct_values(&self) -> Vec<Arc<V>> {
        let entries = self.lock();
        let mut values: Vec<Arc<V>> = Vec::with_capacity(entries.len());
        for (_, cell) in entries.iter().rev() {
            let state = cell
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let CellState::Built(value) = &*state
                && !values.iter().any(|held| Arc::ptr_eq(held, value))
            {
                values.push(Arc::clone(value));
            }
        }
        values
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<(K, Arc<Cell<V>>)>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Marks a cell failed if its builder unwinds before finishing it.
struct FailOnDrop<'a, V>(Option<&'a Arc<Cell<V>>>);

impl<V> Drop for FailOnDrop<'_, V> {
    fn drop(&mut self) {
        if let Some(cell) = self.0.take() {
            *cell
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = CellState::Failed;
            cell.done.notify_all();
        }
    }
}

/// The identities of some of one hour's tables.
///
/// Two keys are equal when every listed table is the same allocation, which is
/// how a decoder hands on a table an hour left unchanged. Equal rows in two
/// allocations (a keyframe re-snapshot, or a delta that changed nothing
/// visible) compare unequal: a miss, never a wrong hit. The key holds the
/// tables, so an allocation cannot be freed and its address reused while the
/// key exists.
#[derive(Clone)]
pub struct TableKey(Vec<SharedView>);

impl TableKey {
    pub fn of(frame: &DecodedFrame, tables: &[TableId]) -> Self {
        Self(
            tables
                .iter()
                .map(|table| frame.tables[table.index()].clone())
                .collect(),
        )
    }
}

impl PartialEq for TableKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self
                .0
                .iter()
                .zip(&other.0)
                .all(|(a, b)| SharedView::ptr_eq(a, b))
    }
}

/// Why [`project`] leaves a table out.
pub const NOT_AN_INPUT: &str = "not an input of this layer";

/// `frame` with only `tables` kept and every other table unavailable.
///
/// Building a keyed layer from this frame is what makes its [`TableKey`]
/// complete: the builder cannot see a table the key does not hold. The branch
/// and hour are kept (a layer reading them keys on them too); the acquisition
/// metrics are not an input of any layer and are blanked.
pub fn project(frame: &DecodedFrame, tables: &[TableId]) -> DecodedFrame {
    let mut keep = [false; TABLE_COUNT];
    for table in tables {
        keep[table.index()] = true;
    }
    let absent = SharedView::from(TableView::Unavailable {
        reason: NOT_AN_INPUT.to_owned(),
    });
    DecodedFrame {
        branch: frame.branch,
        hour: frame.hour,
        tables: std::array::from_fn(|index| {
            if keep[index] {
                frame.tables[index].clone()
            } else {
                absent.clone()
            }
        }),
        metrics: Metrics::default(),
    }
}

/// A layer built from a fixed list of tables and cached on their identities,
/// with an extra key for anything else it reads.
pub struct Keyed<V, E = ()> {
    tables: &'static [TableId],
    recent: Recent<(TableKey, E), V>,
}

impl<V, E: PartialEq> Keyed<V, E> {
    pub fn new(tables: &'static [TableId], capacity: usize) -> Self {
        Self {
            tables,
            recent: Recent::new(capacity),
        }
    }

    /// The layer for `frame` and `extra`: a recent value when its tables are
    /// the same allocations and `extra` is equal, otherwise `build` on the
    /// projected frame.
    pub fn get_or_build<Error>(
        &self,
        frame: &DecodedFrame,
        extra: E,
        build: impl FnOnce(&DecodedFrame) -> Result<Arc<V>, Error>,
    ) -> Result<Arc<V>, Error> {
        self.recent
            .get_or_insert_with((TableKey::of(frame, self.tables), extra), || {
                build(&project(frame, self.tables))
            })
    }
}

/// A layer built from a fixed list of tables whose equal values keep one
/// identity from hour to hour.
///
/// A hit on the tables' identities returns the recent value without building,
/// and callers asking for the same tables at once share one build ([`Recent`]).
/// Otherwise the layer is built from the [`project`]ed frame and compared with
/// the distinct recent values: an equal one is returned in its place, so a
/// value keeps its allocation for as long as the hours leave it unchanged,
/// even when its tables are new allocations every hour.
pub struct Settled<V> {
    tables: &'static [TableId],
    recent: Recent<TableKey, V>,
}

impl<V: PartialEq> Settled<V> {
    pub fn new(tables: &'static [TableId], capacity: usize) -> Self {
        Self {
            tables,
            recent: Recent::new(capacity),
        }
    }

    /// The value for `frame`: a recent one when its tables are the same
    /// allocations, else `build` on the projected frame, replaced by an equal
    /// recent value when there is one.
    ///
    /// Each set of tables is one entry, and each value may be held by several:
    /// stepping back to an earlier hour's allocations hits without building.
    /// Two different sets of tables built at once with equal values may settle
    /// on two allocations when neither build finished before the other
    /// compared; a layer keyed on the value's identity then misses once.
    pub fn get_or_build<Error>(
        &self,
        frame: &DecodedFrame,
        build: impl FnOnce(&DecodedFrame) -> Result<V, Error>,
    ) -> Result<Arc<V>, Error> {
        self.recent
            .get_or_insert_with(TableKey::of(frame, self.tables), || {
                let built = build(&project(frame, self.tables))?;
                Ok(self
                    .recent
                    .distinct_values()
                    .into_iter()
                    .find(|held| **held == built)
                    .unwrap_or_else(|| Arc::new(built)))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::{TableRows, WeatherProvinceRow};

    fn frame(hour: i32, weather: &SharedView, other: &SharedView) -> DecodedFrame {
        let mut tables: [SharedView; TABLE_COUNT] = std::array::from_fn(|_| {
            SharedView::from(TableView::Unavailable {
                reason: "not recorded".into(),
            })
        });
        tables[TableId::WeatherProvinces.index()] = weather.clone();
        tables[TableId::Countries.index()] = other.clone();
        DecodedFrame {
            branch: 1,
            hour,
            tables,
            metrics: Metrics::default(),
        }
    }

    fn weather(rows: usize) -> SharedView {
        SharedView::from(TableView::Complete(TableRows::WeatherProvinces(
            (0..rows)
                .map(|id| WeatherProvinceRow {
                    id: id as u32,
                    ..Default::default()
                })
                .collect(),
        )))
    }

    #[test]
    fn a_keyed_layer_builds_once_while_its_tables_are_shared() {
        let layer = Keyed::<usize>::new(&[TableId::WeatherProvinces], 4);
        let shared = weather(3);
        let (a, b) = (weather(1), weather(2));
        let builds = std::cell::Cell::new(0);
        let get = |frame: &DecodedFrame| {
            layer
                .get_or_build(frame, (), |projected| {
                    builds.set(builds.get() + 1);
                    let rows = match &*projected.tables[TableId::WeatherProvinces.index()] {
                        TableView::Complete(rows) => rows.len(),
                        _ => 0,
                    };
                    Ok::<_, ()>(Arc::new(rows))
                })
                .unwrap()
        };
        let first = get(&frame(10, &shared, &a));
        // Another table changed, the keyed one did not: the same value.
        let second = get(&frame(11, &shared, &b));
        assert!(Arc::ptr_eq(&first, &second));
        // Equal rows in a new allocation are a miss, not a guess.
        let third = get(&frame(12, &weather(3), &b));
        assert!(!Arc::ptr_eq(&first, &third));
        assert_eq!(*third, 3);
        assert_eq!(builds.get(), 2);
    }

    #[test]
    fn the_extra_key_separates_values_of_the_same_tables() {
        let layer = Keyed::<i32, i32>::new(&[TableId::WeatherProvinces], 4);
        let shared = weather(1);
        let at = |hour: i32| {
            layer
                .get_or_build(&frame(hour, &shared, &shared), hour, |projected| {
                    Ok::<_, ()>(Arc::new(projected.hour))
                })
                .unwrap()
        };
        assert_eq!(*at(7), 7);
        assert_eq!(*at(8), 8);
        assert!(Arc::ptr_eq(&at(7), &at(7)));
    }

    #[test]
    fn a_projected_frame_holds_only_the_listed_tables() {
        let shared = weather(2);
        let mut full = frame(5, &shared, &weather(1));
        full.metrics.total_ns = 9;
        let projected = project(&full, &[TableId::WeatherProvinces]);
        assert_eq!((projected.branch, projected.hour), (1, 5));
        assert_eq!(projected.metrics, Metrics::default());
        assert!(SharedView::ptr_eq(
            &projected.tables[TableId::WeatherProvinces.index()],
            &shared
        ));
        assert!(matches!(
            &*projected.tables[TableId::Countries.index()],
            TableView::Unavailable { reason } if reason == NOT_AN_INPUT
        ));
    }

    #[test]
    fn a_settled_layer_keeps_one_value_while_its_content_is_unchanged() {
        let layer = Settled::<usize>::new(&[TableId::WeatherProvinces], 4);
        let other = weather(1);
        let builds = std::cell::Cell::new(0);
        let get = |weather: &SharedView| {
            layer
                .get_or_build(&frame(10, weather, &other), |projected| {
                    builds.set(builds.get() + 1);
                    // The unlisted table is not visible to the build.
                    assert!(matches!(
                        &*projected.tables[TableId::Countries.index()],
                        TableView::Unavailable { reason } if reason == NOT_AN_INPUT
                    ));
                    match &*projected.tables[TableId::WeatherProvinces.index()] {
                        TableView::Complete(rows) => Ok::<_, ()>(rows.len()),
                        _ => Err(()),
                    }
                })
                .unwrap()
        };
        let shared = weather(3);
        let first = get(&shared);
        // The same allocation hits without building.
        assert!(Arc::ptr_eq(&first, &get(&shared)));
        assert_eq!(builds.get(), 1);
        // A new allocation builds, and settles on the equal value.
        let equal = weather(3);
        assert!(Arc::ptr_eq(&first, &get(&equal)));
        assert_eq!(builds.get(), 2);
        // A changed value is a new one.
        let changed = get(&weather(2));
        assert_eq!(*changed, 2);
        assert!(!Arc::ptr_eq(&first, &changed));
        // Each allocation got its own entry: both earlier ones still hit.
        assert!(Arc::ptr_eq(&first, &get(&shared)));
        assert!(Arc::ptr_eq(&first, &get(&equal)));
        assert_eq!(builds.get(), 3);
    }

    /// A settled layer of `WeatherProvinces` row counts, and how many builds
    /// it made.
    fn counting(capacity: usize) -> (Settled<usize>, std::sync::atomic::AtomicU32) {
        (
            Settled::new(&[TableId::WeatherProvinces], capacity),
            std::sync::atomic::AtomicU32::new(0),
        )
    }

    fn settle(
        layer: &Settled<usize>,
        builds: &std::sync::atomic::AtomicU32,
        weather: &SharedView,
    ) -> Arc<usize> {
        let other = self::weather(0);
        layer
            .get_or_build(&frame(1, weather, &other), |projected| {
                builds.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                match &*projected.tables[TableId::WeatherProvinces.index()] {
                    TableView::Complete(rows) => Ok::<_, ()>(rows.len()),
                    _ => Err(()),
                }
            })
            .unwrap()
    }

    #[test]
    fn a_settled_layer_settles_back_onto_an_older_value() {
        let (layer, builds) = counting(4);
        let three = settle(&layer, &builds, &weather(3));
        let two = settle(&layer, &builds, &weather(2));
        assert!(!Arc::ptr_eq(&three, &two));
        // The latest value is two rows; a new allocation of three rows still
        // settles on the older value.
        assert!(Arc::ptr_eq(&three, &settle(&layer, &builds, &weather(3))));
        assert_eq!(builds.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn a_settled_layer_evicts_its_least_recently_used_tables() {
        let (layer, builds) = counting(2);
        let count = || builds.load(std::sync::atomic::Ordering::SeqCst);
        let (a, b, c) = (weather(1), weather(2), weather(3));
        settle(&layer, &builds, &a);
        settle(&layer, &builds, &b);
        // A hit makes `a` the most recently used, so `c` evicts `b`.
        settle(&layer, &builds, &a);
        assert_eq!(count(), 2);
        settle(&layer, &builds, &c);
        assert_eq!(count(), 3);
        settle(&layer, &builds, &a);
        assert_eq!(count(), 3);
        settle(&layer, &builds, &b);
        assert_eq!(count(), 4);
    }

    #[test]
    fn concurrent_callers_of_one_settled_hour_share_one_build() {
        let (layer, builds) = counting(4);
        let (shared, other) = (weather(3), weather(0));
        let started = std::sync::Barrier::new(4);
        let values: Vec<_> = std::thread::scope(|scope| {
            let threads: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        started.wait();
                        // The build outlasts the other callers' arrival, so
                        // they wait on it rather than hit a finished value.
                        layer
                            .get_or_build(&frame(1, &shared, &other), |_| {
                                builds.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                std::thread::sleep(std::time::Duration::from_millis(50));
                                Ok::<_, ()>(3)
                            })
                            .unwrap()
                    })
                })
                .collect();
            threads.into_iter().map(|t| t.join().unwrap()).collect()
        });
        assert_eq!(builds.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(values.iter().all(|value| Arc::ptr_eq(value, &values[0])));
    }

    #[test]
    fn distinct_values_hold_each_allocation_once_most_recent_first() {
        let (layer, builds) = counting(4);
        let one = settle(&layer, &builds, &weather(1));
        let two = settle(&layer, &builds, &weather(2));
        // A second allocation of one row is a second entry for the same value.
        settle(&layer, &builds, &weather(1));
        let values = layer.recent.distinct_values();
        assert_eq!(values.len(), 2);
        assert!(Arc::ptr_eq(&values[0], &one));
        assert!(Arc::ptr_eq(&values[1], &two));
    }

    #[test]
    fn a_settled_layer_does_not_keep_failed_builds() {
        let layer = Settled::<usize>::new(&[TableId::WeatherProvinces], 2);
        let shared = weather(1);
        let other = weather(1);
        let hour = frame(1, &shared, &other);
        assert!(
            layer
                .get_or_build(&hour, |_| Err::<usize, _>("failed"))
                .is_err()
        );
        assert_eq!(*layer.get_or_build(&hour, |_| Ok::<_, ()>(5)).unwrap(), 5);
    }

    #[test]
    fn recent_values_are_bounded_and_failures_are_not_kept() {
        let recent = Recent::<u32, u32>::new(2);
        for key in 0..3 {
            recent
                .get_or_insert_with(key, || Ok::<_, ()>(Arc::new(key)))
                .unwrap();
        }
        // Key 0 was evicted, so it builds again.
        let mut built = false;
        recent
            .get_or_insert_with(0, || {
                built = true;
                Ok::<_, ()>(Arc::new(0))
            })
            .unwrap();
        assert!(built);
        assert!(
            recent
                .get_or_insert_with(9, || Err::<Arc<u32>, _>("failed"))
                .is_err()
        );
        let mut rebuilt = false;
        recent
            .get_or_insert_with(9, || {
                rebuilt = true;
                Ok::<_, ()>(Arc::new(9))
            })
            .unwrap();
        assert!(rebuilt);
    }

    #[test]
    fn concurrent_callers_of_one_key_share_one_build() {
        let recent = Arc::new(Recent::<u32, u32>::new(4));
        let builds = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let started = Arc::new(std::sync::Barrier::new(4));
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let (recent, builds, started) = (
                    Arc::clone(&recent),
                    Arc::clone(&builds),
                    Arc::clone(&started),
                );
                std::thread::spawn(move || {
                    started.wait();
                    recent
                        .get_or_insert_with(1, || {
                            builds.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            std::thread::sleep(std::time::Duration::from_millis(50));
                            Ok::<_, ()>(Arc::new(42))
                        })
                        .unwrap()
                })
            })
            .collect();
        let values: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert_eq!(builds.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(values.iter().all(|value| Arc::ptr_eq(value, &values[0])));
    }
}
