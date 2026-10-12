//! Whole-branch scans with their blocks read ahead: while a scan works on one block,
//! the next few are inflated and decoded on spare cores, and every block still
//! reaches the scan in the order it was given. Reads from every scan share one core
//! budget, so scans running together never oversubscribe the machine. A scan keeping
//! most tables reads one block at a time: its blocks are nearly whole raw blocks.

use replay::{
    container::{BlockIndex, Replay},
    frame::{Frame, TABLE_COUNT},
    rows::TableId,
};
use std::{
    io,
    marker::PhantomData,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Condvar, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

/// Blocks a scan may have read ahead of the one it is working on.
const AHEAD: usize = 8;

#[cfg(test)]
thread_local! {
    pub(crate) static SCANNED_BLOCKS: std::cell::RefCell<Vec<u64>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

/// One block as a scan receives it: what [`Replay::read_block_selected`] returned.
pub type BlockRead<'a> = (&'a BlockIndex, io::Result<Vec<Frame>>);

/// The tables a scan reads from each block: one list for every block, or [`PerBlock`].
pub trait BlockTables: Sync {
    fn of(&self, block: &BlockIndex) -> &[TableId];
}

impl BlockTables for [TableId] {
    fn of(&self, _: &BlockIndex) -> &[TableId] {
        self
    }
}

impl<const N: usize> BlockTables for [TableId; N] {
    fn of(&self, _: &BlockIndex) -> &[TableId] {
        self
    }
}

impl BlockTables for Vec<TableId> {
    fn of(&self, _: &BlockIndex) -> &[TableId] {
        self
    }
}

/// Tables chosen block by block.
pub struct PerBlock<'t, F>(F, PhantomData<&'t ()>);

impl<'t, F: Fn(&BlockIndex) -> &'t [TableId] + Sync> PerBlock<'t, F> {
    pub fn new(choose: F) -> Self {
        Self(choose, PhantomData)
    }
}

impl<'t, F: Fn(&BlockIndex) -> &'t [TableId] + Sync> BlockTables for PerBlock<'t, F> {
    fn of(&self, block: &BlockIndex) -> &[TableId] {
        (self.0)(block)
    }
}

/// `blocks`, each read with only its `tables`, handed to `scan` in their given order.
/// Reading ahead stops once `scan` returns or `cancelled` is set.
pub fn scan_blocks<'a, R>(
    replay: &'a Replay,
    blocks: impl IntoIterator<Item = &'a BlockIndex>,
    tables: &(impl BlockTables + ?Sized),
    cancelled: &AtomicBool,
    scan: impl FnOnce(&mut dyn Iterator<Item = BlockRead<'a>>) -> io::Result<R>,
) -> io::Result<R> {
    let blocks = blocks.into_iter();
    let (minimum, maximum) = blocks.size_hint();
    let mut collected = Vec::new();
    if maximum == Some(minimum) {
        collected = engine::allocation::vec(minimum)?;
        collected.extend(blocks);
    } else {
        for block in blocks {
            engine::allocation::reserve(&mut collected, 1)?;
            collected.push(block);
        }
    }
    let blocks = collected;
    #[cfg(test)]
    SCANNED_BLOCKS.with_borrow_mut(|scanned| {
        scanned.extend(blocks.iter().map(|block| block.offset));
    });
    let ahead = Ahead {
        state: Mutex::new(State {
            taken: 0,
            handed: 0,
            stop: false,
            reads: engine::allocation::default_vec(blocks.len())?,
        }),
        changed: Condvar::new(),
    };
    let read = |position: usize| {
        let _core = cores().take();
        catch_unwind(AssertUnwindSafe(|| {
            replay.read_block_selected(blocks[position], tables.of(blocks[position]))
        }))
        .unwrap_or_else(|_| Err(io::Error::other("block read failed")))
    };
    thread::scope(|scope| {
        let largest = blocks.iter().map(|block| tables.of(block).len()).max();
        let readers = if largest.unwrap_or(0) * 2 > TABLE_COUNT {
            0
        } else {
            AHEAD
        };
        // Install before spawning: a refused worker must wake and join those already running.
        let _stop = Stop(&ahead);
        for _ in 0..readers.min(blocks.len().saturating_sub(1)) {
            thread::Builder::new().spawn_scoped(scope, || {
                while let Some(position) = ahead.take(cancelled) {
                    let result = read(position);
                    ahead.lock().reads[position] = Some(result);
                    ahead.changed.notify_all();
                }
            })?;
        }
        scan(&mut std::iter::from_fn(|| {
            let mut state = ahead.lock();
            let position = state.handed;
            if position == blocks.len() {
                return None;
            }
            // A block no reader has taken (they stop when cancelled) is read here.
            let result = if position == state.taken {
                state.taken += 1;
                drop(state);
                let result = read(position);
                state = ahead.lock();
                result
            } else {
                loop {
                    if let Some(result) = state.reads[position].take() {
                        break result;
                    }
                    state = ahead.changed.wait(state).unwrap_or_else(|e| e.into_inner());
                }
            };
            state.handed += 1;
            drop(state);
            ahead.changed.notify_all();
            Some((blocks[position], result))
        }))
    })
}

struct State {
    /// The next block a reader takes, and the next one handed to the scan.
    taken: usize,
    handed: usize,
    stop: bool,
    reads: Vec<Option<io::Result<Vec<Frame>>>>,
}

struct Ahead {
    state: Mutex<State>,
    changed: Condvar,
}

impl Ahead {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// The next block to read once it is within reach of the scan, or none when
    /// every block is taken or reading should stop.
    fn take(&self, cancelled: &AtomicBool) -> Option<usize> {
        let mut state = self.lock();
        loop {
            if state.stop || state.taken == state.reads.len() || cancelled.load(Ordering::Acquire) {
                return None;
            }
            if state.taken <= state.handed + AHEAD {
                state.taken += 1;
                return Some(state.taken - 1);
            }
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}

struct Stop<'a>(&'a Ahead);

impl Drop for Stop<'_> {
    fn drop(&mut self) {
        self.0.lock().stop = true;
        self.0.changed.notify_all();
    }
}

/// Block reads running at once across every scan: one per core.
struct Cores {
    free: Mutex<usize>,
    freed: Condvar,
}

struct Core(&'static Cores);

fn cores() -> &'static Cores {
    static CORES: OnceLock<Cores> = OnceLock::new();
    CORES.get_or_init(|| Cores {
        free: Mutex::new(thread::available_parallelism().map_or(1, |cores| cores.get())),
        freed: Condvar::new(),
    })
}

impl Cores {
    fn take(&'static self) -> Core {
        let mut free = self.free.lock().unwrap_or_else(|error| error.into_inner());
        while *free == 0 {
            free = self
                .freed
                .wait(free)
                .unwrap_or_else(|error| error.into_inner());
        }
        *free -= 1;
        Core(self)
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        *self
            .0
            .free
            .lock()
            .unwrap_or_else(|error| error.into_inner()) += 1;
        self.0.freed.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{CAMPAIGN, records, source};
    use replay::{frame::FrameEncoder, spool::Record};

    #[test]
    fn blocks_arrive_in_their_given_order_and_a_scan_may_stop_early() {
        // Each appended segment starts a keyframe, so each hour is its own block.
        let initial = records(1, vec![source(1, 1, None)]);
        let context = initial[..2].to_vec();
        let mut replay = Replay::from_records(CAMPAIGN.into(), &[initial]).unwrap();
        for hour in 2..=12 {
            let mut segment = context.clone();
            let frame = FrameEncoder::new().encode(source(1, hour, None), false);
            segment.push(Record::Frame(frame.unwrap()));
            replay = replay.append(&[segment]).unwrap();
        }
        assert_eq!(replay.index.len(), 12);
        let blocks: Vec<_> = replay.index.iter().rev().collect();
        let tables = [TableId::Campaign];
        let cancelled = AtomicBool::new(false);
        let hours = scan_blocks(
            &replay,
            blocks.iter().copied(),
            &tables,
            &cancelled,
            |reads| {
                Ok(reads
                    .map(|(block, frames)| (block.first_hour, frames.unwrap()[0].hour))
                    .collect::<Vec<_>>())
            },
        );
        let expected: Vec<_> = blocks
            .iter()
            .map(|block| (block.first_hour, block.first_hour))
            .collect();
        assert_eq!(hours.unwrap(), expected);
        let first = scan_blocks(
            &replay,
            blocks.iter().copied(),
            &tables,
            &cancelled,
            |reads| Ok(reads.next().map(|(block, _)| block.first_hour)),
        );
        assert_eq!(first.unwrap(), Some(blocks[0].first_hour));
        cancelled.store(true, Ordering::Release);
        let all = scan_blocks(
            &replay,
            blocks.iter().copied(),
            &tables,
            &cancelled,
            |reads| Ok(reads.count()),
        );
        assert_eq!(
            all.unwrap(),
            blocks.len(),
            "a cancelled scan that keeps asking still gets every block"
        );
    }
}
