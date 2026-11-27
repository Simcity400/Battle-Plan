use crate::Source;
use playhead::{
    changes::Changes,
    lane::Lane,
    pages::{Pages, WorldPages},
};
use replay_columnar::unit::BLOCK;
use replay_data::Hour;
use std::{
    collections::{HashMap, HashSet},
    io,
    sync::{Arc, Mutex, PoisonError},
};

const HELD_SETS: usize = 4;

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
    let free = pool.free.lock().unwrap_or_else(PoisonError::into_inner);
    Ok(free.iter().map(|placed| {
        let lanes: Vec<_> = placed.lanes.iter().map(|lane| (lane.table(), lane.head(), lane.requested())).collect();
        format!("{:?}", (placed.run, placed.at, lanes))
    }).collect())
}

pub struct Pool {
    pub(super) pages: Arc<WorldPages>,
    free: Mutex<Vec<Placed>>,
    warming: Mutex<HashMap<(usize, usize), Vec<dashboard_follow::panel::Wake>>>,
    unplaceable: Mutex<HashSet<(usize, usize)>>,
}

pub(super) enum Warm<T> {
    Read(io::Result<T>),
    Warming,
    Cold,
}

struct Placed {
    run: usize,
    at: Option<Hour>,
    lanes: Vec<Lane>,
}

pub(super) struct Taken {
    pool: Arc<Pool>,
    placed: Option<Placed>,
}

impl Pool {
    pub fn of(source: &Source) -> io::Result<Arc<Self>> {
        let strip = STRIP.with_borrow_mut(|held| -> io::Result<_> {
            let Some(held) = held.as_mut().filter(|held| std::ptr::eq(held.source, source)) else {
                return Ok(None);
            };
            if held.pool.is_none() {
                held.pool = Some(Self::new(source)?);
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
        let mut held = state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(pool) = &held.0 {
            return Ok(Arc::clone(pool));
        }
        let pool = Self::new(source)?;
        held.0 = Some(Arc::clone(&pool));
        Ok(pool)
    }

    fn new(source: &Source) -> io::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            pages: WorldPages::shared(source.replay_ref().recording())?,
            free: Mutex::default(),
            warming: Mutex::default(),
            unplaceable: Mutex::default(),
        }))
    }

    pub(super) fn take(self: &Arc<Self>, run: usize, hour: Hour) -> Taken {
        let mut free = self.free.lock().unwrap_or_else(PoisonError::into_inner);
        let nearest = free.iter().enumerate().filter(|(_, placed)| placed.run == run).max_by_key(|(_, placed)| match placed.at {
            Some(at) if at <= hour => (true, at),
            at => (false, at.map_or(Hour::MIN, |at| -at)),
        });
        let placed = nearest.map(|(at, _)| at).map_or_else(|| Placed { run, at: None, lanes: Vec::new() }, |at| free.swap_remove(at));
        Taken { pool: Arc::clone(self), placed: Some(placed) }
    }
}

impl Pool {
    pub(super) fn warm<T>(self: &Arc<Self>, run: usize, table: usize, columns: &[usize], hour: Hour, wake: Option<dashboard_follow::panel::Wake>, read: impl FnOnce(&Lane) -> io::Result<T>) -> Warm<T> {
        if self.unplaceable.lock().unwrap_or_else(PoisonError::into_inner).contains(&(run, table)) { return Warm::Cold; }
        let mut free = self.free.lock().unwrap_or_else(PoisonError::into_inner);
        let near = |lane: &Lane| lane.table() == table && columns.iter().all(|column| lane.requested().contains(column))
            && lane.head().is_some_and(|head| head <= hour && hour - head <= BLOCK as Hour);
        if let Some(lane) = free.iter_mut().filter(|placed| placed.run == run).flat_map(|placed| placed.lanes.iter_mut()).filter(|lane| near(lane)).max_by_key(|lane| lane.head()) {
            return Warm::Read(place(lane, hour).and_then(|()| read(lane)));
        }
        let mut wide: Vec<usize> = free.iter().filter(|placed| placed.run == run).flat_map(|placed| &placed.lanes)
            .filter(|lane| lane.table() == table).flat_map(|lane| lane.requested().iter().copied()).chain(columns.iter().copied()).collect();
        wide.sort_unstable();
        wide.dedup();
        drop(free);
        let mut warming = self.warming.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(waiting) = warming.get_mut(&(run, table)) {
            if let Some(wake) = wake && !waiting.iter().any(|held| Arc::ptr_eq(held, &wake)) { waiting.push(wake); }
        } else {
            warming.insert((run, table), wake.into_iter().collect());
            let (pool, columns) = (Arc::clone(self), wide);
            let spawned = std::thread::Builder::new().name("product lane".into()).spawn(move || {
                let mut taken = Taken { pool: Arc::clone(&pool), placed: Some(Placed { run, at: None, lanes: Vec::new() }) };
                if taken.lane(table, &columns, hour).is_err() { pool.unplaceable.lock().unwrap_or_else(PoisonError::into_inner).insert((run, table)); }
                drop(taken);
                let waiting = pool.warming.lock().unwrap_or_else(PoisonError::into_inner).remove(&(run, table)).unwrap_or_default();
                for wake in waiting { wake(); }
            });
            if spawned.is_err() {
                warming.remove(&(run, table));
                return Warm::Cold;
            }
        }
        Warm::Warming
    }

}

impl Drop for Taken {
    fn drop(&mut self) {
        let Some(placed) = self.placed.take() else { return };
        let mut free = self.pool.free.lock().unwrap_or_else(PoisonError::into_inner);
        match free.iter_mut().find(|held| held.run == placed.run && held.at == placed.at) {
            Some(held) => {
                for lane in placed.lanes {
                    match held.lanes.iter().position(|kept| kept.table() == lane.table()) {
                        None => held.lanes.push(lane),
                        Some(at) if lane.requested().iter().all(|column| held.lanes[at].requested().contains(column)) && held.lanes[at].head() == lane.head() => {}
                        Some(at) => held.lanes[at] = lane,
                    }
                }
            }
            None => {
                free.push(placed);
                if free.len() > HELD_SETS { free.remove(0); }
            }
        }
    }
}

impl Taken {
    pub(super) fn lane(&mut self, table: usize, columns: &[usize], hour: Hour) -> io::Result<&Lane> {
        let pages = Arc::clone(&self.pool.pages);
        let placed = self.placed.as_mut().expect("held until dropped");
        placed.at = Some(hour);
        let found = placed.lanes.iter().position(|lane| lane.table() == table);
        let at = match found {
            Some(at) if columns.iter().all(|column| placed.lanes[at].requested().contains(column)) => at,
            _ => {
                let run = pages.runs().get(placed.run).ok_or_else(|| invalid(format!("No run {}", placed.run)))?;
                if run.blocks() > 0 && pages.table_entry(placed.run, 0, table)?.log {
                    return Err(invalid(format!("Table {table} is a log: its rows are read by ordinal, never at an hour")));
                }
                let mut wide = found.map_or_else(Vec::new, |at| placed.lanes[at].requested().to_vec());
                wide.extend_from_slice(columns);
                wide.sort_unstable();
                wide.dedup();
                let lane = Lane::new(pages as Arc<dyn Pages>, placed.run, table, &wide, true)?;
                match found {
                    Some(at) => {
                        placed.lanes[at] = lane;
                        at
                    }
                    None => {
                        placed.lanes.push(lane);
                        placed.lanes.len() - 1
                    }
                }
            }
        };
        let lane = &mut placed.lanes[at];
        place(lane, hour)?;
        Ok(lane)
    }
}

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
        _ => lane.seek(hour).map(drop),
    }
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
