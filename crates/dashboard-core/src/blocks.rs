//! Whole-branch scans with their blocks read ahead: while a scan works on one block,
//! the next few are inflated and decoded on spare cores, and every block still
//! reaches the scan in the order it was given. Reads from every scan share one core
//! budget, so scans running together never oversubscribe the machine. A scan keeping
//! most tables reads one block at a time: its blocks are nearly whole raw blocks.

use replay::{container::BlockIndex, container::Replay};
use replay_data::rows::TABLE_COUNT;
use replay_data::rows::TableId;
use replay_frame::frame::RecordedFrame;
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

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub static SCANNED_BLOCKS: std::cell::RefCell<Vec<u64>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

/// One block as a scan receives it: what [`Replay::read_block_selected`] returned.
pub type BlockRead<'a> = (&'a BlockIndex, io::Result<Vec<RecordedFrame>>);

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
        collected = engine_allocation::vec(minimum)?;
        collected.extend(blocks);
    } else {
        for block in blocks {
            engine_allocation::reserve(&mut collected, 1)?;
            collected.push(block);
        }
    }
    let blocks = collected;
    #[cfg(any(test, feature = "test-support"))]
    SCANNED_BLOCKS.with_borrow_mut(|scanned| {
        scanned.extend(blocks.iter().map(|block| block.offset));
    });
    let largest = blocks.iter().map(|block| tables.of(block).len()).max();
    let ahead = if largest.unwrap_or(0) * 2 > TABLE_COUNT {
        0
    } else {
        AHEAD
    };
    let consumer = replay.section_consumer(
        blocks
            .first()
            .and_then(|block| replay.block_position(block))
            .unwrap_or(0),
    );
    scan_blocks_by(
        blocks,
        ahead,
        cancelled,
        &|block| {
            if let Some(position) = replay.block_position(block) {
                consumer.reading(position);
            }
            replay.read_block_recorded_selected(block, tables.of(block))
        },
        |reads| {
            scan(&mut reads.map(|(block, frames)| {
                if let Some(position) = replay.block_position(block) {
                    consumer.at(position);
                }
                (block, frames)
            }))
        },
    )
}

/// `blocks` (or any items read alike), each read by `read`, handed to `scan` in their
/// given order, with at most `ahead` blocks read on spare cores ahead of the one `scan`
/// works on (none: every block is read on the calling thread when `scan` asks for it).
/// Reads share [`scan_blocks`]'s core budget. Reading ahead stops once `scan` returns
/// or `cancelled` is set.
pub fn scan_blocks_by<'a, B: Sync, T: Send, R>(
    blocks: Vec<&'a B>,
    ahead: usize,
    cancelled: &AtomicBool,
    read_block: &(dyn Fn(&'a B) -> io::Result<T> + Sync),
    scan: impl FnOnce(&mut dyn Iterator<Item = (&'a B, io::Result<T>)>) -> io::Result<R>,
) -> io::Result<R> {
    let readers = ahead;
    let state = Ahead {
        state: Mutex::new(State {
            taken: 0,
            handed: 0,
            stop: false,
            ahead,
            reads: engine_allocation::default_vec(blocks.len())?,
        }),
        changed: Condvar::new(),
    };
    let ahead = &state;
    let read = |position: usize| {
        let _core = cores().take();
        catch_unwind(AssertUnwindSafe(|| read_block(blocks[position])))
            .unwrap_or_else(|_| Err(io::Error::other("block read failed")))
    };
    // Measurement only: the readers' CPU is the scan's family's.
    let cpu_family = replay_frame::cpu::family();
    thread::scope(|scope| {
        // Install before spawning: a refused worker must wake and join those already running.
        let _stop = Stop(ahead);
        for _ in 0..readers.min(blocks.len().saturating_sub(1)) {
            let priority = job_priority::JobPriority::current();
            thread::Builder::new().spawn_scoped(scope, || {
                priority.enter(job_priority::Level::Normal);
                let _cpu = replay_frame::cpu::scope("scan read-ahead", cpu_family);
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

struct State<T> {
    /// The next block a reader takes, and the next one handed to the scan.
    taken: usize,
    handed: usize,
    stop: bool,
    /// Blocks a reader may take beyond the one handed next.
    ahead: usize,
    reads: Vec<Option<io::Result<T>>>,
}

struct Ahead<T> {
    state: Mutex<State<T>>,
    changed: Condvar,
}

impl<T> Ahead<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
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
            if state.taken <= state.handed + state.ahead {
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

struct Stop<'a, T>(&'a Ahead<T>);

impl<T> Drop for Stop<'_, T> {
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
    use replay::spool::Record;
    use replay_frame::frame::FrameEncoder;

    #[test]
    fn read_ahead_keeps_the_callers_job() {
        use job_priority::JobPriority;
        use std::{sync::Arc, time::Duration};

        let replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![source(1, 1, None)])]).unwrap();
        let background = Arc::new(AtomicBool::new(true));
        JobPriority::run(Arc::clone(&background), || {
            for expected in [true, false] {
                background.store(expected, Ordering::Release);
                let (sent, received) = std::sync::mpsc::channel();
                let caller = thread::current().id();
                scan_blocks_by(
                    vec![&replay.index[0]; 3],
                    1,
                    &AtomicBool::new(false),
                    &|_| {
                        sent.send((thread::current().id(), JobPriority::current().background()))
                            .unwrap();
                        Ok(())
                    },
                    |reads| {
                        // Wait before taking a block: this read must be a worker's.
                        let (worker, inherited) =
                            received.recv_timeout(Duration::from_secs(10)).unwrap();
                        assert_ne!(worker, caller);
                        assert_eq!(inherited, expected);
                        for (_, read) in reads {
                            read?;
                        }
                        Ok(())
                    },
                )
                .unwrap();
            }
        });
    }

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
