
use std::sync::MutexGuard;
use std::sync::{Arc, Condvar, Mutex, Weak};

use replay_data::rows::TABLE_COUNT;
use replay_data::rows::TableId;
use replay_frame::{frame::DecodedFrame, frame::Metrics, frame::SharedView, frame::TableView};

pub struct Recent<K, V> {
    entries: Mutex<Entries<K, V>>,
}

struct Entries<K, V> {
    held: Vec<(K, Arc<Cell<V>>)>,
    latest: Option<Arc<V>>,
}

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

    pub fn get_or_insert_with<E>(
        &self,
        key: K,
        build: impl FnOnce() -> Result<Arc<V>, E>,
    ) -> Result<Arc<V>, E> {
        self.get_or_insert(key, true, build)
    }

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

    fn build<E>(
        &self,
        cell: &Arc<Cell<V>>,
        build: impl FnOnce() -> Result<Arc<V>, E>,
    ) -> Result<Arc<V>, E> {
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

struct FailOnDrop<'a, V>(Option<&'a Arc<Cell<V>>>);

impl<V> Drop for FailOnDrop<'_, V> {
    fn drop(&mut self) {
        if let Some(cell) = self.0.take() {
            *cell.lock() = CellState::Failed;
            cell.done.notify_all();
        }
    }
}

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

pub const NOT_AN_INPUT: &str = "not an input of this layer";

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
