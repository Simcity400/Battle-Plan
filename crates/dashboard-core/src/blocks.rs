
use job_priority::SpawnCarried as _;
use replay::{campaign::BlockIndex, campaign::Replay};
use replay_frame::frame::DecodedFrame;
use std::{cell::RefCell, rc::Rc, sync::mpsc};
use replay_data::rows::TableId;
use replay_frame::frame::RecordedFrame;
use std::{
    io,
    sync::{
        Condvar, Mutex, OnceLock,
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
    received: Option<mpsc::Receiver<Sent>>,
    held: Option<Sent>,
    current: Option<usize>,
    broken: Option<io::Error>,
}

impl Feed {
    fn next(&mut self) -> Option<Sent> {
        self.held
            .take()
            .or_else(|| self.received.as_ref()?.recv().ok())
    }
}

struct Close<'a>(&'a RefCell<Feed>);

impl Drop for Close<'_> {
    fn drop(&mut self) {
        let mut feed = self.0.borrow_mut();
        feed.received = None;
        feed.held = None;
        feed.current = None;
    }
}

pub struct BlockHours {
    feed: Rc<RefCell<Feed>>,
    position: usize,
    ended: bool,
}

impl Iterator for BlockHours {
    type Item = io::Result<ScanHour>;

    fn next(&mut self) -> Option<io::Result<ScanHour>> {
        if self.ended {
            return None;
        }
        let mut feed = self.feed.borrow_mut();
        if feed.current != Some(self.position) {
            self.ended = true;
            return Some(Err(io::Error::other(
                "this block's hours were read after the scan moved past them",
            )));
        }
        match feed.next() {
            Some(Sent::Hour(hour)) => return Some(Ok(*hour)),
            Some(Sent::Broken(at, error)) if at == self.position => {
                feed.broken.get_or_insert(error);
            }
            other => feed.held = other,
        }
        feed.current = None;
        self.ended = true;
        None
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
        received: Some(received),
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
        let _close = Close(&feed);
        let mut next = 0;
        let mut iterate = std::iter::from_fn(|| {
            if next == blocks.len() {
                feed.borrow_mut().current = None;
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
                                ended: false,
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
        let broken = feed.borrow_mut().broken.take();
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
                if sender.send(Sent::Broken(position, crate::duplicate(&error))).is_err() {
                    return;
                }
            }
            for position in begun..end {
                if sender.send(Sent::Failed(position, crate::duplicate(&error))).is_err() {
                    return;
                }
            }
        }
        at = end;
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

    fn twelve_blocks() -> Replay {
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
        replay
    }

    #[test]
    fn a_block_read_after_the_scan_moved_past_it_is_not_a_clean_end() {
        let replay = twelve_blocks();
        for blocks in [1, replay.index.len()] {
            let passed_over = scan_blocks(
                &replay,
                replay.index.iter().take(blocks),
                &[TableId::Campaign],
                &AtomicBool::new(false),
                |reads| {
                    let first = reads.next().unwrap().1?;
                    reads.next();
                    Ok(first.map(|hour| hour.err().map(|error| error.kind())).collect::<Vec<_>>())
                },
            );
            assert_eq!(
                passed_over.unwrap(),
                [Some(io::ErrorKind::Other)],
                "hours skipped by the scan must read as one error, not as a finished block or as hours"
            );
        }
    }

    #[test]
    fn a_block_kept_past_its_scan_lets_the_scan_finish() {
        let replay = crate::test_fixtures::replay((1..=200).map(|hour| source(1, hour, None)).collect());
        let mut kept = scan_blocks(
            &replay,
            replay.index.iter(),
            &[TableId::Campaign],
            &AtomicBool::new(false),
            |reads| reads.next().unwrap().1,
        )
        .unwrap();
        assert!(
            matches!(kept.next(), Some(Err(_))),
            "a block read after its scan ended must not read as a finished block"
        );
        assert!(kept.next().is_none());
    }

    #[test]
    fn blocks_arrive_in_their_given_order_and_a_scan_may_stop_early() {
        let replay = twelve_blocks();
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
                    .map(|(block, frames)| (block.first_hour, frames.unwrap().next().unwrap().unwrap().hour))
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
