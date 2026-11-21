use crate::Source;
use playhead::{
    changes::Changes,
    lane::Lane,
    pages::{Pages, WorldPages},
};
use replay_columnar::unit::{BLOCK, reader};
use replay_data::Hour;
use std::{
    io,
    sync::{Arc, Mutex, PoisonError},
};

const HELD_SETS: usize = 4;

pub struct Pool {
    pub(super) pages: Arc<WorldPages>,
    free: Mutex<Vec<Placed>>,
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
        #[derive(Default)]
        struct Held(Option<Arc<Pool>>);
        let state = source.state::<Held>();
        let mut held = state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(pool) = &held.0 {
            return Ok(Arc::clone(pool));
        }
        let pool = Arc::new(Self { pages: WorldPages::shared(source.replay_ref().recording())?, free: Mutex::default() });
        held.0 = Some(Arc::clone(&pool));
        Ok(pool)
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

impl Drop for Taken {
    fn drop(&mut self) {
        let Some(placed) = self.placed.take() else { return };
        let mut free = self.pool.free.lock().unwrap_or_else(PoisonError::into_inner);
        if !free.iter().any(|held| held.run == placed.run && held.at == placed.at) {
            free.push(placed);
            if free.len() > HELD_SETS { free.remove(0); }
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
                if reader::class_of(&run.pages, table)? {
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
