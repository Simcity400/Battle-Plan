
use job_priority::SpawnCarried as _;
use replay::{campaign::BlockIndex, campaign::Replay};
use replay_frame::frame::DecodedFrame;
use std::{cell::RefCell, rc::Rc, sync::mpsc};
use replay_data::rows::TableId;
use replay_frame::frame::RecordedFrame;
use std::{
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Condvar, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};


#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub static SCANNED_BLOCKS: std::cell::RefCell<Vec<u64>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

pub struct ScanHour {
    pub recorded: RecordedFrame,
    pub decoded: DecodedFrame,
}

impl std::ops::Deref for ScanHour {
    type Target = RecordedFrame;

    fn deref(&self) -> &RecordedFrame {
        &self.recorded
    }
}

impl replay_frame::frame::FrameSource for ScanHour {
    fn branch(&self) -> replay_data::BranchId {
        self.recorded.branch()
    }
    fn hour(&self) -> replay_data::Hour {
        self.recorded.hour()
    }
    fn keyframe(&self) -> bool {
        self.recorded.keyframe()
    }
    fn metrics(&self) -> &replay_frame::frame::Metrics {
        self.recorded.metrics()
    }
    fn update(&self, index: usize) -> &replay_frame::frame::TableUpdate {
        self.recorded.update(index)
    }
    fn shared_update(&self, index: usize) -> Option<&std::sync::Arc<replay_frame::frame::TableUpdate>> {
        self.recorded.shared_update(index)
    }
}

pub type BlockRead<'a> = (&'a BlockIndex, io::Result<BlockHours>);

enum Sent {
    Begin(usize),
    Hour(Box<ScanHour>),
    Failed(usize, io::Error),
    Broken(usize, io::Error),
}

struct Feed {
    received: mpsc::Receiver<Sent>,
    held: Option<Sent>,
    current: Option<usize>,
    broken: Option<io::Error>,
}

impl Feed {
    fn next(&mut self) -> Option<Sent> {
        self.held.take().or_else(|| self.received.recv().ok())
    }
}

pub struct BlockHours {
    feed: Rc<RefCell<Feed>>,
    position: usize,
}

impl Iterator for BlockHours {
    type Item = ScanHour;

    fn next(&mut self) -> Option<ScanHour> {
        let mut feed = self.feed.borrow_mut();
        if feed.current != Some(self.position) {
            return None;
        }
        match feed.next() {
            Some(Sent::Hour(hour)) => Some(*hour),
            Some(Sent::Broken(at, error)) if at == self.position => {
                feed.broken.get_or_insert(error);
                feed.current = None;
                None
            }
            other => {
                feed.held = other;
                feed.current = None;
                None
            }
        }
    }
}

pub fn scan_blocks<'a, R>(
    replay: &'a Replay,
    blocks: impl IntoIterator<Item = &'a BlockIndex>,
    tables: &[TableId],
    cancelled: &AtomicBool,
    scan: impl FnOnce(&mut dyn Iterator<Item = BlockRead<'a>>) -> io::Result<R>,
) -> io::Result<R> {
    let blocks: Vec<&'a BlockIndex> = blocks.into_iter().collect();
    #[cfg(any(test, feature = "test-support"))]
    SCANNED_BLOCKS.with_borrow_mut(|scanned| {
        scanned.extend(blocks.iter().map(|block| (u64::from(block.run) << 32) | u64::from(block.page)));
    });
    let (sender, received) = mpsc::sync_channel::<Sent>(AHEAD_HOURS);
    let feed = Rc::new(RefCell::new(Feed {
        received,
        held: None,
        current: None,
        broken: None,
    }));
    let cpu_family = replay_frame::cpu::family();
    thread::scope(|scope| {
        let walking = &blocks;
        let priority = job_priority::JobPriority::current();
        thread::Builder::new().spawn_scoped_carried(scope, move || {
            priority.enter(job_priority::Level::Normal);
            let _cpu = replay_frame::cpu::scope("scan read-ahead", cpu_family);
            let _core = cores().take();
            walk(replay, walking, tables, cancelled, &sender);
        })?;
        let mut next = 0;
        let mut iterate = std::iter::from_fn(|| {
            if next == blocks.len() {
                return None;
            }
            let position = next;
            next += 1;
            let mut held = feed.borrow_mut();
            loop {
                match held.next() {
                    Some(Sent::Hour(_)) => continue,
                    Some(Sent::Begin(at)) if at == position => {
                        held.current = Some(position);
                        drop(held);
                        return Some((
                            blocks[position],
                            Ok(BlockHours {
                                feed: Rc::clone(&feed),
                                position,
                            }),
                        ));
                    }
                    Some(Sent::Failed(at, error)) if at == position => {
                        held.current = None;
                        return Some((blocks[position], Err(error)));
                    }
                    Some(Sent::Begin(_) | Sent::Failed(..) | Sent::Broken(..)) => continue,
                    None => {
                        held.current = None;
                        let error = if cancelled.load(Ordering::Acquire) {
                            io::Error::new(io::ErrorKind::Interrupted, "the scan was cancelled")
                        } else {
                            io::Error::other("the scan's walk ended before this block")
                        };
                        return Some((blocks[position], Err(error)));
                    }
                }
            }
        });
        let result = scan(&mut iterate);
        drop(iterate);
        let broken = feed.borrow_mut().broken.take();
        drop(feed);
        match (result, broken) {
            (Ok(_), Some(error)) => Err(error),
            (result, _) => result,
        }
    })
}

const AHEAD_HOURS: usize = 64;

fn walk(
    replay: &Replay,
    blocks: &[&BlockIndex],
    tables: &[TableId],
    cancelled: &AtomicBool,
    sender: &mpsc::SyncSender<Sent>,
) {
    let mut at = 0;
    while at < blocks.len() {
        let mut end = at + 1;
        while end < blocks.len()
            && replay.same_segment(blocks[end - 1], blocks[end])
            && (blocks[end].run, blocks[end].page) > (blocks[end - 1].run, blocks[end - 1].page)
        {
            end += 1;
        }
        let group = &blocks[at..end];
        let mut begun = at;
        let mut open = None;
        let mut gone = false;
        let walked = replay.visit_blocks(
            group,
            tables,
            |block, recorded, decoded| {
                if cancelled.load(Ordering::Acquire) {
                    return Err(io::Error::new(io::ErrorKind::Interrupted, "the scan was cancelled"));
                }
                if decoded.hour == block.first_hour {
                    let position = at + group.iter().position(|held| std::ptr::eq(*held, block)).unwrap_or(0);
                    if sender.send(Sent::Begin(position)).is_err() {
                        gone = true;
                        return Ok(false);
                    }
                    begun = position + 1;
                    open = Some(position);
                }
                if decoded.hour == block.last_hour {
                    open = None;
                }
                let hour = Box::new(ScanHour {
                    recorded: recorded.clone(),
                    decoded: decoded.clone(),
                });
                if sender.send(Sent::Hour(hour)).is_err() {
                    gone = true;
                    return Ok(false);
                }
                Ok(true)
            },
            || Ok(()),
        );
        if gone {
            return;
        }
        if let Err(error) = walked {
            if let Some(position) = open {
                let error = io::Error::new(error.kind(), error.to_string());
                if sender.send(Sent::Broken(position, error)).is_err() {
                    return;
                }
            }
            for position in begun..end {
                let error = io::Error::new(error.kind(), error.to_string());
                if sender.send(Sent::Failed(position, error)).is_err() {
                    return;
                }
            }
        }
        at = end;
    }
}

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
    let cpu_family = replay_frame::cpu::family();
    thread::scope(|scope| {
        let _stop = Stop(ahead);
        for _ in 0..readers.min(blocks.len().saturating_sub(1)) {
            let priority = job_priority::JobPriority::current();
            thread::Builder::new().spawn_scoped_carried(scope, || {
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
    taken: usize,
    handed: usize,
    stop: bool,
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
    use replay_capture_spool::spool::Record;
    use replay_frame::frame::FrameEncoder;

    #[test]
    fn read_ahead_keeps_the_callers_job() {
        use job_priority::JobPriority;
        use std::{sync::Arc, time::Duration};

        let replay =
            Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, vec![source(1, 1, None)])]).unwrap();
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
        let initial = records(1, vec![source(1, 1, None)]);
        let context = initial[..2].to_vec();
        let mut replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[initial]).unwrap();
        for hour in 2..=12 {
            let mut segment = context.clone();
            let frame = FrameEncoder::new().encode(source(1, hour, None), false);
            segment.push(Record::Frame(frame.unwrap()));
            replay = replay.append(recorder_derive::writers(), &[segment]).unwrap();
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
                    .map(|(block, frames)| (block.first_hour, frames.unwrap().next().unwrap().hour))
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
