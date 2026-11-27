use super::Keys;
use crate::Source;
use dashboard_follow::panel::Wake;
use playhead::{
    changes::Changes,
    lane::Lane,
    pages::{Pages, WorldPages},
};
use replay_columnar::unit::BLOCK;
use replay_data::Hour;
use replay_frame::frame::SharedView;
use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex, MutexGuard, PoisonError, Weak, atomic::{AtomicBool, Ordering}},
};

type Key = (usize, usize);

struct StripPool {
    source: *const Source,
    pool: Option<Arc<Pool>>,
}

thread_local! {
    static STRIP: std::cell::RefCell<Option<StripPool>> = const { std::cell::RefCell::new(None) };
}

pub fn with_strip_pool<T>(source: &Source, work: impl FnOnce() -> T) -> T {
    struct Restore(Option<StripPool>);
    impl Drop for Restore {
        fn drop(&mut self) {
            STRIP.set(self.0.take());
        }
    }
    let _restore = Restore(STRIP.replace(Some(StripPool {
        source: std::ptr::from_ref(source),
        pool: None,
    })));
    work()
}

#[cfg(any(test, feature = "test-support"))]
pub fn interactive_pool_snapshot(source: &Source) -> io::Result<Vec<String>> {
    let pool = Pool::interactive(source)?;
    let slots = lock(&pool.slots);
    let mut lanes = Vec::new();
    for (key, slot) in slots.iter() {
        if let Some(followed) = slot.followed.upgrade() && let Some(lane) = lock(&followed.lane).as_ref() {
            lanes.push(format!("{:?}", (key, lane.head(), lane.requested())));
        }
        if slot.exact.strong_count() != 0 { lanes.push(format!("exact {key:?}")); }
    }
    lanes.sort_unstable();
    Ok(lanes)
}

pub struct Pool {
    pub(super) pages: Arc<WorldPages>,
    slots: Mutex<HashMap<Key, Slot>>,
    strip: bool,
}

pub(super) enum Warm<T> {
    Read(io::Result<T>),
    Warming,
    Cold,
}

#[derive(Default)]
struct Slot {
    followed: Weak<Following>,
    exact: Weak<Exact>,
    strip: Option<Arc<Exact>>,
}

#[derive(Default)]
pub(super) struct Following {
    lane: Mutex<Option<Lane>>,
    warming: Mutex<Option<Vec<Wake>>>,
    unplaceable: AtomicBool,
}

#[derive(Default)]
struct Exact {
    lane: Mutex<Option<Lane>>,
    views: Mutex<Vec<Weak<Projection>>>,
}

struct Projection {
    hour: Hour,
    fields: u64,
    keys: Keys,
    view: Mutex<Option<SharedView>>,
}

#[derive(Default)]
pub(super) struct ExactReads(Vec<(Arc<Exact>, Arc<Projection>)>);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Pool {
    pub fn of(source: &Source) -> io::Result<Arc<Self>> {
        let strip = STRIP.with_borrow_mut(|held| -> io::Result<_> {
            let Some(held) = held.as_mut().filter(|held| std::ptr::eq(held.source, source)) else {
                return Ok(None);
            };
            if held.pool.is_none() {
                held.pool = Some(Self::new(source, true)?);
            }
            Ok(held.pool.clone())
        })?;
        if let Some(pool) = strip {
            return Ok(pool);
        }
        Self::interactive(source)
    }

    fn interactive(source: &Source) -> io::Result<Arc<Self>> {
        #[derive(Default)]
        struct Held(Option<Arc<Pool>>);
        let state = source.state::<Held>();
        let mut held = lock(&state);
        if let Some(pool) = &held.0 {
            return Ok(Arc::clone(pool));
        }
        let pool = Self::new(source, false)?;
        held.0 = Some(Arc::clone(&pool));
        Ok(pool)
    }

    fn new(source: &Source, strip: bool) -> io::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            pages: WorldPages::shared(source.replay_ref().recording())?,
            slots: Mutex::default(),
            strip,
        }))
    }

    fn slot<T>(&self, key: Key, get: impl FnOnce(&mut Slot) -> T) -> T {
        let mut slots = lock(&self.slots);
        slots.retain(|_, slot| slot.followed.strong_count() != 0 || slot.exact.strong_count() != 0);
        get(slots.entry(key).or_default())
    }

    pub(super) fn following(&self, key: Key) -> Arc<Following> {
        self.slot(key, |slot| slot.followed.upgrade().unwrap_or_else(|| {
            let followed = Arc::new(Following::default());
            slot.followed = Arc::downgrade(&followed);
            followed
        }))
    }

    fn position<'a>(&self, held: &'a mut Option<Lane>, (run, table): Key, columns: &[usize], hour: Hour) -> io::Result<&'a Lane> {
        if !held.as_ref().is_some_and(|lane| covers(lane, columns)) {
            if self.pages.runs().get(run).is_some_and(|run| run.blocks() > 0) && self.pages.table_entry(run, 0, table)?.log {
                return Err(invalid(format!("Table {table} is a log: its rows are read by ordinal, never at an hour")));
            }
            let mut wide = held.as_ref().map_or_else(Vec::new, |lane| lane.requested().to_vec());
            wide.extend_from_slice(columns);
            wide.sort_unstable();
            wide.dedup();
            *held = Some(Lane::new(self.pages.clone() as Arc<dyn Pages>, run, table, &wide, true)?);
        }
        if let Err(error) = place(held.as_mut().expect("lane opened"), hour) {
            *held = None;
            return Err(error);
        }
        Ok(held.as_ref().expect("lane positioned"))
    }
}

impl Following {
    pub(super) fn read<T>(&self, pool: &Pool, key: Key, columns: &[usize], hour: Hour, read: impl FnOnce(&Lane) -> io::Result<T>) -> io::Result<T> {
        pool.position(&mut lock(&self.lane), key, columns, hour).and_then(read)
    }

    pub(super) fn warm<T>(self: &Arc<Self>, pool: &Arc<Pool>, key: Key, columns: &[usize], hour: Hour, wake: Option<Wake>, read: impl FnOnce(&Lane) -> io::Result<T>) -> Warm<T> {
        if self.unplaceable.load(Ordering::Acquire) { return Warm::Cold; }
        let mut warming = lock(&self.warming);
        if let Some(waiting) = warming.as_mut() {
            if let Some(wake) = wake && !waiting.iter().any(|held| Arc::ptr_eq(held, &wake)) { waiting.push(wake); }
            return Warm::Warming;
        }
        if let Ok(mut held) = self.lane.try_lock()
            && let Some(lane) = held.as_mut().filter(|lane| covers(lane, columns))
            && lane.head().is_some_and(|head| head <= hour && hour - head <= BLOCK as Hour) {
            drop(warming);
            return Warm::Read(pool.position(&mut held, key, columns, hour).and_then(read));
        }
        *warming = Some(wake.into_iter().collect());
        let (followed, pool, columns) = (self.clone(), pool.clone(), columns.to_vec());
        let spawned = std::thread::Builder::new().name("product lane".into()).spawn(move || {
            if followed.read(&pool, key, &columns, hour, |_| Ok(())).is_err() {
                followed.unplaceable.store(true, Ordering::Release);
            }
            let waiting = lock(&followed.warming).take().unwrap_or_default();
            for wake in waiting { wake(); }
        });
        if spawned.is_err() {
            *warming = None;
            return Warm::Cold;
        }
        Warm::Warming
    }
}

impl ExactReads {
    pub(super) fn read(&mut self, pool: &Pool, key: Key, columns: &[usize], (hour, fields, keys): (Hour, u64, &Keys), read: impl FnOnce(&Lane) -> io::Result<SharedView>) -> io::Result<SharedView> {
        let (exact, followed) = pool.slot(key, |slot| {
            let exact = slot.strip.clone().or_else(|| slot.exact.upgrade()).unwrap_or_else(|| {
                let exact = Arc::new(Exact::default());
                slot.exact = Arc::downgrade(&exact);
                if pool.strip { slot.strip = Some(Arc::clone(&exact)); }
                exact
            });
            (exact, slot.followed.upgrade())
        });
        let shared = exact.projection(hour, fields, keys);
        let mut held = lock(&shared.view);
        if held.is_none() {
            let followed = followed.as_ref().and_then(|followed| followed.lane.try_lock().ok())
                .filter(|held| held.as_ref().is_some_and(|lane| lane.head() == Some(hour) && covers(lane, columns)));
            let view = match followed.as_deref().and_then(Option::as_ref) {
                Some(lane) => read(lane)?,
                None => read(pool.position(&mut lock(&exact.lane), key, columns, hour)?)?,
            };
            *held = Some(view);
        }
        let view = held.as_ref().expect("projection made").clone();
        drop(held);
        self.0.push((exact, shared));
        Ok(view)
    }
}

impl Exact {
    fn projection(&self, hour: Hour, fields: u64, keys: &Keys) -> Arc<Projection> {
        let mut views = lock(&self.views);
        views.retain(|view| view.strong_count() != 0);
        views.iter().filter_map(Weak::upgrade)
            .find(|view| view.hour == hour && view.fields == fields && view.keys == *keys)
            .unwrap_or_else(|| {
                let view = Arc::new(Projection { hour, fields, keys: keys.clone(), view: Mutex::default() });
                views.push(Arc::downgrade(&view));
                view
            })
    }
}

fn covers(lane: &Lane, columns: &[usize]) -> bool {
    columns.iter().all(|column| lane.requested().contains(column))
}

#[cfg(test)]
thread_local! { static RESTORES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) }; }

#[cfg(test)]
pub(super) fn restores() -> u64 { RESTORES.get() }

fn place(lane: &mut Lane, hour: Hour) -> io::Result<()> {
    match lane.head() {
        Some(head) if head == hour => Ok(()),
        Some(head) if head < hour && hour - head <= BLOCK as Hour => {
            let (mut changes, mut passed) = (Changes::default(), Vec::new());
            while let Some(head) = lane.head().filter(|&head| head < hour) {
                lane.fetch_through(hour)?;
                if lane.step(hour, &mut changes)? == head {
                    return Err(invalid(format!("Table {}'s lane cannot step past hour {head}", lane.table())));
                }
                lane.retire_into(&mut passed);
                passed.clear();
            }
            Ok(())
        }
        _ => {
            #[cfg(test)]
            RESTORES.set(RESTORES.get() + 1);
            lane.seek(hour).map(drop)
        }
    }
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_joined_projection_outlives_the_request_that_made_it() {
        let exact = Exact::default();
        let first = exact.projection(4, 3, &Keys::All);
        let waiting = exact.projection(4, 3, &Keys::All);
        let made = SharedView::from(replay_frame::frame::TableView::Unavailable { reason: "example".into() });
        *lock(&first.view) = Some(made.clone());
        drop(first);
        assert!(SharedView::ptr_eq(lock(&waiting.view).as_ref().unwrap(), &made));
    }
}
