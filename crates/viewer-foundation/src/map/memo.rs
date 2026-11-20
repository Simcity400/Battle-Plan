//! Small caches for hour layers that the next hours usually rebuild unchanged.
//!
//! A decoded hour shares each table it did not change with the hour before it
//! (`replay_frame::frame::SharedView`), so "this table did not change" is one pointer
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

use std::sync::MutexGuard;
use std::sync::{Arc, Condvar, Mutex, Weak};

use replay_data::rows::TABLE_COUNT;
use replay_data::rows::TableId;
use replay_frame::{frame::DecodedFrame, frame::Metrics, frame::SharedView, frame::TableView};

/// The values of one layer, keyed by what they were built from: the latest built or asked for,
/// and every other one while something holds it.
pub struct Recent<K, V> {
    entries: Mutex<Entries<K, V>>,
}

struct Entries<K, V> {
    /// Oldest first.
    held: Vec<(K, Arc<Cell<V>>)>,
    latest: Option<Arc<V>>,
}

/// One key's value: being built, built, or failed.
struct Cell<V> {
    state: Mutex<CellState<V>>,
    done: Condvar,
}

enum CellState<V> {
    Building,
    Built(Weak<V>),
    Failed,
}

impl<V> Cell<V> {
    fn lock(&self) -> MutexGuard<'_, CellState<V>> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl<K, V> Default for Recent<K, V> {
    fn default() -> Self {
        Self {
            entries: Mutex::new(Entries {
                held: Vec::new(),
                latest: None,
            }),
        }
    }
}

impl<K: PartialEq, V> Recent<K, V> {
    pub fn new() -> Self {
        Self::default()
    }

    /// The value built for `key`, building it when no live entry has it.
    ///
    /// The first caller of a key builds it with no lock held; a caller that
    /// asks for the same key meanwhile waits for that build instead of doing
    /// the same work, so one key is built once. A failed build is not cached:
    /// its entry is removed and a waiter builds for itself. Nor is one that read
    /// a table its rows' reader had not declared: once the table is learned,
    /// the next ask builds over rows that hold it.
    pub fn get_or_insert_with<E>(
        &self,
        key: K,
        build: impl FnOnce() -> Result<Arc<V>, E>,
    ) -> Result<Arc<V>, E> {
        self.get_or_insert(key, true, build)
    }

    /// [`Self::get_or_insert_with`] for a caller that never waits on another's build: while
    /// another caller builds `key`, it builds its own copy, for itself alone.
    pub fn get_or_insert_without_waiting<E>(
        &self,
        key: K,
        build: impl FnOnce() -> Result<Arc<V>, E>,
    ) -> Result<Arc<V>, E> {
        self.get_or_insert(key, false, build)
    }

    fn get_or_insert<E>(
        &self,
        key: K,
        wait: bool,
        build: impl FnOnce() -> Result<Arc<V>, E>,
    ) -> Result<Arc<V>, E> {
        let cell = {
            let mut entries = self.lock();
            // An entry whose value nothing holds goes, with the tables its key holds.
            entries.held.retain(|(_, cell)| match &*cell.lock() {
                CellState::Building => true,
                CellState::Built(value) => value.strong_count() > 0,
                CellState::Failed => false,
            });
            match entries.held.iter().position(|(held, _)| *held == key) {
                Some(at) => {
                    let cell = Arc::clone(&entries.held[at].1);
                    let built = match &*cell.lock() {
                        CellState::Built(value) => value.upgrade(),
                        _ => None,
                    };
                    if let Some(value) = built {
                        entries.latest = Some(Arc::clone(&value));
                        return Ok(value);
                    }
                    if !wait {
                        drop(entries);
                        return build();
                    }
                    cell
                }
                None => {
                    let cell = Arc::new(Cell {
                        state: Mutex::new(CellState::Building),
                        done: Condvar::new(),
                    });
                    entries.held.push((key, Arc::clone(&cell)));
                    drop(entries);
                    return self.build(&cell, build);
                }
            }
        };
        let mut state = cell.lock();
        loop {
            match &*state {
                CellState::Built(value) => match value.upgrade() {
                    Some(value) => return Ok(value),
                    None => break,
                },
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
        build()
    }

    /// Builds `cell`'s value, the latest once built.
    fn build<E>(
        &self,
        cell: &Arc<Cell<V>>,
        build: impl FnOnce() -> Result<Arc<V>, E>,
    ) -> Result<Arc<V>, E> {
        // A panic while building leaves the cell failed rather than waited on.
        let mut guard = FailOnDrop(Some(cell));
        let (built, undeclared) = replay_frame::frame::discovering_within(build);
        guard.0 = None;
        let kept = built.is_ok() && undeclared.is_empty();
        match &built {
            Ok(value) if kept => {
                self.lock().latest = Some(Arc::clone(value));
                *cell.lock() = CellState::Built(Arc::downgrade(value));
            }
            _ => *cell.lock() = CellState::Failed,
        }
        cell.done.notify_all();
        if !kept {
            self.lock().held.retain(|(_, held)| !Arc::ptr_eq(held, cell));
        }
        built
    }

    /// Every live value, the latest first and then the most recently built, each allocation
    /// once.
    ///
    /// Taken under the entries' lock, then each cell's (the order every other
    /// path takes them in); a value still building is left out.
    fn distinct_values(&self) -> Vec<Arc<V>> {
        let entries = self.lock();
        let mut values: Vec<Arc<V>> = entries.latest.iter().cloned().collect();
        for (_, cell) in entries.held.iter().rev() {
            if let CellState::Built(value) = &*cell.lock()
                && let Some(value) = value.upgrade()
                && !values.iter().any(|held| Arc::ptr_eq(held, &value))
            {
                values.push(value);
            }
        }
        values
    }

    fn lock(&self) -> MutexGuard<'_, Entries<K, V>> {
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
            *cell.lock() = CellState::Failed;
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
    pub fn of(frame: &dyn replay_frame::frame::DecodedSource, tables: &[TableId]) -> Self {
        Self(
            tables
                .iter()
                .map(|table| frame.shared(table.index()))
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
    pub fn new(tables: &'static [TableId]) -> Self {
        Self {
            tables,
            recent: Recent::new(),
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

    /// [`Self::get_or_build`] for a caller that never waits on another's build
    /// ([`Recent::get_or_insert_without_waiting`]).
    pub fn get_or_build_without_waiting<Error>(
        &self,
        frame: &DecodedFrame,
        extra: E,
        build: impl FnOnce(&DecodedFrame) -> Result<Arc<V>, Error>,
    ) -> Result<Arc<V>, Error> {
        self.recent
            .get_or_insert_without_waiting((TableKey::of(frame, self.tables), extra), || {
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
    pub fn new(tables: &'static [TableId]) -> Self {
        Self {
            tables,
            recent: Recent::new(),
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
mod tests;
