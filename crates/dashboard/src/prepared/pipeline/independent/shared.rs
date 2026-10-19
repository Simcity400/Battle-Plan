//! One read of each block for the families prepared on their own whose own
//! thread observes every block hour by hour on the block read alone
//! ([`super::Own::shared_tables`]).
//!
//! Each such block is read once ([`Replay::read_block_kept`]) for every
//! table those families may still read in that block, on reader threads
//! ahead of the families, and each family reconstructs its own tables on its
//! thread with a decoder of its own ([`Replay::reconstruct_alone`]), at its
//! own pace and concurrently with the others. The read (inflating and
//! decoding the recorded updates) is shared; the reconstruction (applying
//! them to each table's rows) is each family's. Each family therefore sees
//! exactly the frames its own read alone makes: its tables' rows, which
//! hours share a table's allocation (the change facts it derives from them),
//! availability and sample hours from its own decoder, and the recorded
//! updates and metrics of its tables, which a selected read decodes table by
//! table whatever else it selects. Its other tables are not reconstructed and
//! stay unavailable, as in its own read; the recorded updates handed with a
//! frame hold the other tables' updates too, which change facts read only for
//! tables whose reconstructed view is complete, so only its own. A block
//! whose read failed, panicked or holds a keyframe an all-table check would
//! refuse for another family's table is read by each family alone, as before,
//! so its errors and refusals are its own read's.
//!
//! The pre-pass pool's block tasks read most blocks before any of these
//! families takes them: a task reading a block one of them will take decodes
//! their tables in the same read and deposits the block's frames here
//! ([`Depot`]), so the readers read only the blocks no task deposited. A
//! deposited block's frames hold every table the task read, its members'
//! tables and these families', each table's updates those of its own read.
//! A family's tables are omitted from blocks it has passed and blocks of
//! the branch it currently refuses. A later branch still keeps its tables.
//! Tables kept only for these readers are not reconstructed by the task.
//!
//! Only the reader threads and the pool's block tasks read blocks for every
//! family; a family never does. A family takes a kept block that holds its
//! tables; or, while a reader is free to read its next block, waits for it;
//! or reads the block itself with its own tables alone
//! ([`Replay::read_block_alone`]), exactly as its own read alone, keeping
//! nothing. Each kept block knows the tables it holds (those of the families
//! preparing when it was read), so a family whose tables a kept block lacks
//! reads that block alone.
//!
//! Memory. Every block read for every family is charged by the bytes its
//! decoded updates hold ([`recorded_bytes`], as a block's facts count them)
//! from the moment it is kept until its last holder drops it: a block released
//! from those kept, so that no family takes it any more, stays charged while a
//! family still reconstructs from it. Each read under way reserves one
//! block's share of the budget, `budget / (readers + 2)`, and starts only
//! while what is charged and reserved, with its own share, fits the budget;
//! before any block was measured only one read is under way, which measures
//! the first. Reading for every family stays on only while every block
//! measured holds at most a block's share: once one holds more, the source
//! is read standalone for the rest of the run (readers and tasks read for
//! nobody, every family reads alone, and kept blocks are released as their
//! holders drop them). So, while sharing, what is charged (kept blocks,
//! released blocks families still hold, and each read's share) stays within
//! the budget: charged bytes <= budget + (reader threads + busy pool tasks +
//! 1) blocks, each at most `budget / (readers + 2)`, where the reads under
//! way when a block first measures beyond its share are the only ones that
//! may exceed it, and that ends sharing. In standalone mode only the
//! families' own reads are made. Whenever what is charged passes the budget,
//! kept blocks are released, furthest from the family taking them first. A
//! pool task's deposit released here may still be held by that task until it
//! ends, as that task's own block.
//!
//! A family takes its blocks in order and never waits for another family: it
//! wants the block it takes next and the next [`AHEAD`], and readers read the
//! wanted blocks nearest to a family's next first. A block read is kept while
//! a slower family will still take it, so it is read once for all of them. A
//! reader gives a block up, unkept, as soon as no family wants it any more: it
//! checks before each table section it decodes (each hour of an interleaved
//! block). There is one reader per family but no more than the machine's
//! hardware threads, each reading one block at a time. A family that refused
//! a block's branch wants nothing.

use crate::prepared::prepass;
use dashboard_base::prepared::priority::{JobPriority, Level};
use replay::container::Replay;
use replay_data::rows::TABLE_COUNT;
use replay_data::rows::TableId;
use replay_frame::{frame::DecodedFrame, frame::RecordedFrame as Frame};
use std::{
    collections::BTreeMap,
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// Blocks read ahead of the one a family takes next.
const AHEAD: usize = 1;

/// Bytes charged for blocks read for every family (each one's decoded
/// updates, kept or still held, and each read under way's share), past which
/// no read starts and kept blocks are released: a budget for read-ahead
/// memory, never a limit on blocks, hours or families.
const BUDGET: u64 = 512 * 1024 * 1024;

/// The timing row of the shared reads (worker time), and in tests the count
/// of blocks read.
pub(in crate::prepared::pipeline) const SHARED_READ: &str = "independent.shared_read";

/// The timing row counting the blocks pre-pass block tasks deposited, and in
/// tests their count.
pub(in crate::prepared::pipeline) const SHARED_DEPOSITED: &str = "independent.shared_deposited";

/// Each block a family read alone because its shared read could not serve
/// it, counted in tests.
#[cfg(test)]
pub(in crate::prepared::pipeline) const SHARED_ALONE: &str = "independent.shared_alone";

/// What a block's frames hold decoded.
fn frames_bytes(frames: &[Arc<Frame>]) -> u64 {
    let mut held = dashboard_base::prepared::prepass::HeldBytes::default();
    held.add(size_of_val(frames)).heap(frames).total()
}

/// Bytes charged while their block is alive, released when it is dropped.
struct Charge {
    held: Arc<AtomicU64>,
    bytes: u64,
}

impl Charge {
    fn new(held: &Arc<AtomicU64>, bytes: u64) -> Self {
        held.fetch_add(bytes, Ordering::AcqRel);
        Self {
            held: Arc::clone(held),
            bytes,
        }
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        self.held.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

/// A block's frames as its one read returned them, each with its recorded
/// updates of every table read; the tables read; and their charge, held
/// until the last holder drops them.
pub(super) struct Frames {
    frames: Vec<Arc<Frame>>,
    chosen: [bool; TABLE_COUNT],
    _charge: Charge,
}

impl Frames {
    /// Whether the read holds every one of `tables`.
    fn holds(&self, tables: &[TableId]) -> bool {
        tables.iter().all(|table| self.chosen[table.index()])
    }
}

enum Entry {
    /// A reader or a pool task is reading it.
    Reading,
    Read(Arc<Frames>),
    /// Its read failed: each family reads it alone.
    Alone,
}

/// One family's place among the readers.
#[derive(Clone, Copy)]
struct Consumer {
    /// Whether it reads through the shared reader at all, and now.
    member: bool,
    live: bool,
    /// The block it takes next, and whether it reads it.
    next: usize,
    reads: bool,
}

/// How a read for every family ended.
#[allow(clippy::large_enum_variant)] // The table flags move on into the frames read.
enum Ended {
    Read(Vec<Arc<Frame>>, [bool; TABLE_COUNT]),
    Failed,
    /// No family wanted it any more.
    GivenUp,
    /// A pool task did not read the block it was offered.
    Withdrawn,
}

struct State {
    consumers: Vec<Consumer>,
    entries: BTreeMap<usize, Entry>,
    /// Reads under way, each reserving a block's share; readers reading.
    in_flight: usize,
    busy_readers: usize,
    /// Whether blocks are still read for every family, and whether one was
    /// measured yet.
    sharing: bool,
    measured: bool,
    /// The tables each block offered to a pool task is read with.
    offered: BTreeMap<usize, [bool; TABLE_COUNT]>,
    /// The most bytes charged at once.
    peak: u64,
    /// Time spent reading, blocks read and blocks deposited.
    spent: Duration,
    reads: u64,
    deposits: u64,
    /// Each block read or deposited, each read given up, each block a family
    /// read alone, and a gate reads wait at before decoding, in tests.
    #[cfg(test)]
    read_blocks: Vec<usize>,
    #[cfg(test)]
    dropped: Vec<usize>,
    #[cfg(test)]
    alone_blocks: Vec<usize>,
    #[cfg(test)]
    gate: Option<Arc<tests::Gate>>,
}

impl State {
    /// The live families reading the block they take next.
    fn reading(&self) -> impl Iterator<Item = &Consumer> {
        self.consumers
            .iter()
            .filter(|consumer| consumer.member && consumer.live && consumer.reads)
    }

    /// Whether some family wants `block`: it takes it next or within
    /// [`AHEAD`] after.
    fn wanted(&self, block: usize) -> bool {
        self.reading()
            .any(|consumer| (consumer.next..=consumer.next + AHEAD).contains(&block))
    }

    /// How far `block` lies past the next block of the family that takes it
    /// first; None when no family will take it.
    fn behind(&self, block: usize) -> Option<usize> {
        self.reading()
            .filter(|consumer| consumer.next <= block)
            .map(|consumer| block - consumer.next)
            .min()
    }
}

struct Inner {
    replay: Arc<Replay>,
    /// Every table a member reads, Campaign included, as a set, and each
    /// member's tables.
    chosen: [bool; TABLE_COUNT],
    masks: Vec<[bool; TABLE_COUNT]>,
    /// The bytes of the blocks read for every family still alive.
    held: Arc<AtomicU64>,
    budget: u64,
    /// A block's share of the budget, and the reader threads running.
    share: u64,
    readers: AtomicUsize,
    state: Mutex<State>,
    changed: Condvar,
    stop: AtomicBool,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// The tables `block` is read with now: Campaign and those of every
    /// member that may still read it, as a list and a set. A member past it
    /// or refusing its branch never needs its tables from this read.
    fn selection(&self, state: &State, block: usize) -> (Vec<TableId>, [bool; TABLE_COUNT]) {
        let mut chosen = [false; TABLE_COUNT];
        chosen[TableId::Campaign.index()] = true;
        for (consumer, mask) in state.consumers.iter().zip(&self.masks) {
            if consumer.member
                && consumer.live
                && consumer.next <= block
                && (consumer.reads
                    || self.replay.index[consumer.next].branch != self.replay.index[block].branch)
            {
                for (at, read) in mask.iter().enumerate() {
                    chosen[at] |= *read;
                }
            }
        }
        let tables = TableId::ALL
            .into_iter()
            .filter(|table| chosen[table.index()])
            .collect();
        (tables, chosen)
    }

    /// What is charged: the blocks alive and each read under way's share.
    fn charged(&self, state: &State) -> u64 {
        let reserved = (state.in_flight as u64).saturating_mul(self.share);
        self.held.load(Ordering::Acquire).saturating_add(reserved)
    }

    /// Whether one more read for every family may start: while sharing,
    /// one at a time before any block was measured, and only while what is
    /// charged, with its share, fits the budget.
    fn admits(&self, state: &State) -> bool {
        state.sharing
            && (state.measured || state.in_flight == 0)
            && self.charged(state).saturating_add(self.share) <= self.budget
    }

    /// A read of `block` starts.
    fn begin(&self, state: &mut State, block: usize) {
        state.entries.insert(block, Entry::Reading);
        state.in_flight += 1;
        state.peak = state.peak.max(self.charged(state));
    }

    /// The next block a reader reads: of the blocks some family wants and
    /// nobody has read, the nearest to a family's next block, then the
    /// first, while a read may start. None once the readers stop or the
    /// source is read standalone.
    fn claim(&self) -> Option<usize> {
        let mut state = self.lock();
        loop {
            if self.stop.load(Ordering::Acquire) || !state.sharing {
                return None;
            }
            if self.admits(&state) {
                let end = self.replay.index.len();
                let next = state
                    .reading()
                    .flat_map(|consumer| {
                        (0..=AHEAD).map(move |distance| (distance, consumer.next + distance))
                    })
                    .filter(|&(_, block)| block < end && !state.entries.contains_key(&block))
                    .min()
                    .map(|(_, block)| block);
                if let Some(block) = next {
                    self.begin(&mut state, block);
                    state.busy_readers += 1;
                    return Some(block);
                }
            }
            // A block dropped by its last holder frees room unsignalled;
            // look again shortly.
            state = self
                .changed
                .wait_timeout(state, Duration::from_millis(25))
                .map_or_else(|error| error.into_inner().0, |(state, _)| state);
        }
    }

    /// Read `block` for every family still preparing. Before each table
    /// section it decodes (each hour of an interleaved block) the read stops
    /// when no family wants the block any more, and what it decoded is
    /// released unkept.
    fn read(&self, block: usize) -> Ended {
        let _cpu = ::replay_frame::cpu::scope("decode", Some("shared read"));
        let started = Instant::now();
        let index = &self.replay.index[block];
        let (tables, chosen) = self.selection(&self.lock(), block);
        let mut given_up = false;
        #[cfg(test)]
        let mut gated = false;
        let read = catch_unwind(AssertUnwindSafe(|| {
            self.replay.read_block_kept(index, &tables, || {
                #[cfg(test)]
                {
                    // Taken before waiting, so no lock is held at the gate.
                    let gate = if std::mem::replace(&mut gated, true) {
                        None
                    } else {
                        self.lock().gate.clone()
                    };
                    if let Some(gate) = gate {
                        gate.pass();
                    }
                }
                if !self.lock().wanted(block) {
                    given_up = true;
                    return Err(io::Error::from(io::ErrorKind::Interrupted));
                }
                Ok(())
            })
        }));
        let ended = match read {
            _ if given_up => Ended::GivenUp,
            Ok(Ok(frames)) => Ended::Read(frames, chosen),
            _ => Ended::Failed,
        };
        let mut state = self.lock();
        state.spent += started.elapsed();
        if !matches!(ended, Ended::GivenUp) {
            state.reads += 1;
            #[cfg(test)]
            state.read_blocks.push(block);
        }
        ended
    }

    /// A read of `block` for every family ended. A block read is charged by
    /// what its decoded updates hold and kept while reading for every family
    /// goes on; one holding more than a block's share ends it, and is not
    /// kept.
    fn end(&self, block: usize, ended: Ended) {
        let mut state = self.lock();
        state.in_flight = state.in_flight.saturating_sub(1);
        if matches!(state.entries.get(&block), Some(Entry::Reading)) {
            state.entries.remove(&block);
        }
        match ended {
            Ended::Read(frames, chosen) => {
                let bytes = frames_bytes(&frames);
                state.measured = true;
                let read = Frames {
                    frames,
                    chosen,
                    _charge: Charge::new(&self.held, bytes),
                };
                state.peak = state.peak.max(self.charged(&state));
                if bytes > self.share {
                    state.sharing = false;
                }
                if state.sharing {
                    state.entries.insert(block, Entry::Read(Arc::new(read)));
                }
            }
            Ended::Failed => {
                if state.sharing {
                    state.entries.insert(block, Entry::Alone);
                }
            }
            Ended::GivenUp => {
                #[cfg(test)]
                state.dropped.push(block);
            }
            Ended::Withdrawn => {}
        }
        self.prune(&mut state);
        drop(state);
        self.changed.notify_all();
    }

    /// Release what no family will take (or everything, once the source is
    /// read standalone), then kept blocks while what is charged passes the
    /// budget, furthest from the family taking them first. A released block
    /// stays charged while a family still holds it.
    fn prune(&self, state: &mut State) {
        let released: Vec<usize> = state
            .entries
            .iter()
            .filter(|(block, entry)| {
                !matches!(entry, Entry::Reading)
                    && (!state.sharing || state.behind(**block).is_none())
            })
            .map(|(block, _)| *block)
            .collect();
        for block in released {
            state.entries.remove(&block);
        }
        while self.charged(state) > self.budget {
            let furthest = state
                .entries
                .iter()
                .filter(|(_, entry)| matches!(entry, Entry::Read(_)))
                .max_by_key(|(block, _)| (state.behind(**block).unwrap_or(usize::MAX), **block))
                .map(|(block, _)| *block);
            let Some(block) = furthest else {
                break;
            };
            state.entries.remove(&block);
        }
    }

    fn update(&self, change: impl FnOnce(&mut State)) {
        let mut state = self.lock();
        change(&mut state);
        self.prune(&mut state);
        drop(state);
        self.changed.notify_all();
    }
}

/// The shared block reader of one preparation's families. Dropping it stops
/// its readers within a block's read and joins them.
pub(super) struct Shared {
    inner: Arc<Inner>,
    readers: Vec<JoinHandle<()>>,
}

impl Shared {
    /// The reader of `consumers` (each family's tables, empty for one that
    /// does not read through it, and its first block), none when no family
    /// does. With `live`, every member reads from the start; otherwise each
    /// from [`Self::member`] with `join`, as families prepared one after
    /// another do.
    pub(super) fn start(
        replay: Arc<Replay>,
        consumers: &[(&[TableId], usize)],
        live: bool,
    ) -> Option<Self> {
        #[cfg(test)]
        if TEST_UNSHARED.with(std::cell::Cell::get) {
            return None;
        }
        let mut chosen = [false; TABLE_COUNT];
        chosen[TableId::Campaign.index()] = true;
        let mut any = false;
        for (tables, _) in consumers {
            for table in *tables {
                chosen[table.index()] = true;
                any = true;
            }
        }
        if !any {
            return None;
        }
        let masks = consumers
            .iter()
            .map(|(tables, _)| {
                let mut mask = [false; TABLE_COUNT];
                for table in *tables {
                    mask[table.index()] = true;
                }
                mask
            })
            .collect();
        // One reader per family reading through it, no more than the
        // machine's hardware threads.
        let count = readers(
            consumers
                .iter()
                .filter(|(tables, _)| !tables.is_empty())
                .count()
                .max(1),
        );
        let budget = budget();
        let inner = Arc::new(Inner {
            state: Mutex::new(State {
                consumers: consumers
                    .iter()
                    .map(|(tables, next)| Consumer {
                        member: !tables.is_empty(),
                        live,
                        next: *next,
                        reads: true,
                    })
                    .collect(),
                entries: BTreeMap::new(),
                in_flight: 0,
                busy_readers: 0,
                sharing: true,
                measured: false,
                offered: BTreeMap::new(),
                peak: 0,
                spent: Duration::ZERO,
                reads: 0,
                deposits: 0,
                #[cfg(test)]
                read_blocks: Vec::new(),
                #[cfg(test)]
                dropped: Vec::new(),
                #[cfg(test)]
                alone_blocks: Vec::new(),
                #[cfg(test)]
                gate: TEST_GATE.with(|gate| gate.borrow().clone()),
            }),
            replay,
            chosen,
            masks,
            held: Arc::new(AtomicU64::new(0)),
            budget,
            share: (budget / (count as u64 + 2)).max(1),
            readers: AtomicUsize::new(0),
            changed: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let mut readers = Vec::with_capacity(count);
        for reader in 0..count {
            let thread = Arc::clone(&inner);
            // Counted before it starts, so a family never finds it missing.
            inner.readers.fetch_add(1, Ordering::AcqRel);
            let priority = JobPriority::current();
            // A reader that cannot start leaves its blocks to the others or
            // to the families themselves.
            let spawned = std::thread::Builder::new()
                .name(format!("preparation shared block reader {reader}"))
                .spawn(move || {
                    priority.enter(Level::BelowNormal);
                    while let Some(block) = thread.claim() {
                        let ended = thread.read(block);
                        thread.lock().busy_readers -= 1;
                        thread.end(block, ended);
                    }
                    thread.readers.fetch_sub(1, Ordering::AcqRel);
                    thread.changed.notify_all();
                });
            match spawned {
                Ok(handle) => readers.push(handle),
                Err(_) => {
                    inner.readers.fetch_sub(1, Ordering::AcqRel);
                }
            }
        }
        Some(Self { inner, readers })
    }

    /// Family `consumer`'s membership while it prepares, None when it does
    /// not read through the reader; with `join`, it reads from now on.
    /// Dropping it (however the family's preparation ends, or with the
    /// thread that would have run it) retires the family, so no family
    /// waits for it.
    pub(super) fn member(&self, consumer: usize, join: bool) -> Option<Member<'_>> {
        let mut member = false;
        self.inner.update(|state| {
            let entry = &mut state.consumers[consumer];
            member = entry.member;
            if join {
                entry.live = entry.member;
            }
        });
        member.then_some(Member {
            shared: self,
            consumer,
        })
    }

    /// The time spent reading blocks and the blocks read.
    pub(super) fn read_time(&self) -> (Duration, u64) {
        let state = self.inner.lock();
        (state.spent, state.reads)
    }

    /// The blocks pre-pass block tasks deposited.
    pub(super) fn deposits(&self) -> u64 {
        self.inner.lock().deposits
    }

    /// Where pre-pass block tasks deposit the blocks these families will
    /// take.
    pub(super) fn depot(&self) -> Arc<dyn prepass::Depot> {
        Arc::new(Depot(Arc::clone(&self.inner)))
    }

    /// The bytes charged now (blocks alive and reads' shares) and at most.
    #[cfg(test)]
    fn held(&self) -> (u64, u64) {
        let state = self.inner.lock();
        (self.inner.charged(&state), state.peak)
    }

    /// The bytes of the blocks read for every family still alive, kept or
    /// held.
    #[cfg(test)]
    fn alive_bytes(&self) -> u64 {
        self.inner.held.load(Ordering::Acquire)
    }

    /// The decoded bytes of the blocks kept now, measured from their frames.
    #[cfg(test)]
    fn kept_bytes(&self) -> u64 {
        let state = self.inner.lock();
        state
            .entries
            .values()
            .filter_map(|entry| match entry {
                Entry::Read(frames) => Some(frames_bytes(&frames.frames)),
                _ => None,
            })
            .sum()
    }

    /// The tables `block`'s kept read holds; none when it is not kept.
    #[cfg(test)]
    fn kept_tables(&self, block: usize) -> Vec<TableId> {
        match self.inner.lock().entries.get(&block) {
            Some(Entry::Read(frames)) => TableId::ALL
                .into_iter()
                .filter(|table| frames.chosen[table.index()])
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Whether blocks are still read for every family.
    #[cfg(test)]
    fn sharing(&self) -> bool {
        self.inner.lock().sharing
    }

    /// Each block read or deposited, in order.
    #[cfg(test)]
    fn read_blocks(&self) -> Vec<usize> {
        self.inner.lock().read_blocks.clone()
    }

    /// Each read given up, in order.
    #[cfg(test)]
    fn dropped(&self) -> Vec<usize> {
        self.inner.lock().dropped.clone()
    }

    /// Each block a family read alone with its own tables, in order.
    #[cfg(test)]
    fn alone_blocks(&self) -> Vec<usize> {
        self.inner.lock().alone_blocks.clone()
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        self.inner.stop.store(true, Ordering::Release);
        {
            // Under the lock, so no reader checks the flag and then waits
            // unwoken.
            let _state = self.inner.lock();
            self.inner.changed.notify_all();
        }
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
    }
}

/// The shared reader's side of the pre-pass pool's block tasks.
struct Depot(Arc<Inner>);

impl prepass::Depot for Depot {
    /// A block some live member will take within as many blocks past its
    /// next as the budget has shares, not read nor being read, while a read
    /// for every family may start: admitted as being read, with the tables
    /// of the members preparing to read with it. Members taking it wait for
    /// the task's deposit or withdrawal. Never waits: any other block is
    /// read by the readers or the families.
    fn offer(&self, block: usize) -> Option<Vec<TableId>> {
        let inner = &self.0;
        let mut state = inner.lock();
        let reach = usize::try_from(inner.budget / inner.share).unwrap_or(usize::MAX);
        let near = state
            .reading()
            .any(|consumer| (consumer.next..=consumer.next.saturating_add(reach)).contains(&block));
        if inner.stop.load(Ordering::Acquire)
            || state.entries.contains_key(&block)
            || !near
            || !inner.admits(&state)
        {
            return None;
        }
        inner.begin(&mut state, block);
        let (tables, chosen) = inner.selection(&state, block);
        state.offered.insert(block, chosen);
        Some(tables)
    }

    /// The offered block's frames, read with every table offered.
    fn deposit(&self, block: usize, frames: Vec<Arc<Frame>>) {
        let inner = &self.0;
        let chosen = {
            let mut state = inner.lock();
            state.deposits += 1;
            #[cfg(test)]
            state.read_blocks.push(block);
            state.offered.remove(&block).unwrap_or([false; TABLE_COUNT])
        };
        inner.end(block, Ended::Read(frames, chosen));
    }

    /// The offered block was not read: the readers or its members read it.
    fn withdraw(&self, block: usize) {
        self.0.lock().offered.remove(&block);
        self.0.end(block, Ended::Withdrawn);
    }
}

/// One family's membership of the shared reader.
pub(in crate::prepared::pipeline) struct Member<'s> {
    shared: &'s Shared,
    consumer: usize,
}

impl Member<'_> {
    /// The family takes `block` next, and reads it when `reads` (it has not
    /// refused the block's branch).
    pub(super) fn at(&self, block: usize, reads: bool) {
        let consumer = self.consumer;
        self.shared.inner.update(|state| {
            let entry = &mut state.consumers[consumer];
            entry.next = block;
            entry.reads = reads;
        });
    }

    /// Whether some member reads every one of `tables`.
    fn holds(&self, tables: &[TableId]) -> bool {
        tables
            .iter()
            .all(|table| self.shared.inner.chosen[table.index()])
    }

    /// `block`'s kept read holding `tables`, waiting while it is being read
    /// or while a free reader can read it; None when the family reads it
    /// alone, with its own tables: its read failed or lacks the family's
    /// tables, or nobody reads it and no reader can now.
    fn frames(
        &self,
        block: usize,
        tables: &[TableId],
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<Arc<Frames>>> {
        let inner = &self.shared.inner;
        let mut state = inner.lock();
        loop {
            check()?;
            match state.entries.get(&block) {
                Some(Entry::Read(frames)) if frames.holds(tables) => {
                    return Ok(Some(Arc::clone(frames)));
                }
                Some(Entry::Read(_) | Entry::Alone) => return Ok(None),
                Some(Entry::Reading) => {}
                None => {
                    let free = inner.readers.load(Ordering::Acquire) > state.busy_readers;
                    if !free || !inner.admits(&state) || !state.wanted(block) {
                        #[cfg(test)]
                        state.alone_blocks.push(block);
                        return Ok(None);
                    }
                }
            }
            // Cancellation is not signalled here; look again shortly.
            state = inner
                .changed
                .wait_timeout(state, Duration::from_millis(25))
                .map_or_else(|error| error.into_inner().0, |(state, _)| state);
        }
    }
}

impl Drop for Member<'_> {
    fn drop(&mut self) {
        let consumer = self.consumer;
        self.shared
            .inner
            .update(|state| state.consumers[consumer].live = false);
    }
}

/// The family's share of the shared reader for one block: its membership
/// and the block's position in the recording.
#[derive(Clone, Copy)]
pub(in crate::prepared::pipeline) struct Lease<'a> {
    pub member: &'a Member<'a>,
    pub block: usize,
}

/// `block` (at `position` in `replay`'s index) read alone with `tables`, as
/// [`Replay::read_block_alone`] reads it, handing each hour to `visit` with
/// its recorded updates; through `last` when given. From the shared read
/// when `lease` holds one that serves it, otherwise read here, with `tables`
/// alone.
pub(super) fn read_alone(
    replay: &Replay,
    index: &replay::container::BlockIndex,
    lease: Option<Lease<'_>>,
    tables: &[TableId],
    last: Option<i32>,
    mut visit: impl FnMut(Arc<Frame>, &DecodedFrame) -> io::Result<()>,
    mut check: impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    if let Some(lease) = lease.filter(|lease| lease.member.holds(tables))
        && let Some(frames) = lease.member.frames(lease.block, tables, &mut check)?
    {
        let through = last.map_or(frames.frames.len(), |last| {
            frames.frames.partition_point(|frame| frame.hour <= last)
        });
        if replay.reconstruct_alone(
            &frames.frames[..through],
            tables,
            |raw, frame| visit(Arc::clone(raw), frame),
            &mut check,
        )? {
            return Ok(());
        }
    }
    #[cfg(test)]
    if lease.is_some() {
        prepass::note(SHARED_ALONE);
    }
    let Some(last) = last else {
        return replay.read_block_recorded_alone(
            index,
            tables,
            |raw, frame| visit(raw, frame),
            check,
        );
    };
    // Through `last`: the hours after it are not visited.
    let mut passed = false;
    let read = replay.read_block_recorded_alone(
        index,
        tables,
        |raw, frame| {
            if frame.hour > last {
                passed = true;
                return Err(io::Error::other("Read through its last hour"));
            }
            visit(raw, frame)
        },
        check,
    );
    match read {
        Err(_) if passed => Ok(()),
        read => read,
    }
}

/// A block's final reconstructed frame. Earlier hours are borrowed and
/// released before the next delta; only the boundary frame is retained.
pub(super) fn read_last(
    replay: &Replay,
    index: &replay::container::BlockIndex,
    tables: &[TableId],
) -> io::Result<Option<DecodedFrame>> {
    let mut last = None;
    replay.read_block_recorded_alone(
        index,
        tables,
        |_, frame| {
            if frame.hour == index.last_hour {
                last = Some(frame.clone());
            }
            Ok(())
        },
        || Ok(()),
    )?;
    Ok(last)
}

/// Reader threads for `count` families: one each, and no more than the
/// machine's hardware threads, so the blocks being read at once are bounded
/// by the machine, never by the families. Tests may start none, leaving
/// every block to the families.
fn readers(count: usize) -> usize {
    #[cfg(test)]
    if let Some(count) = TEST_READERS.with(std::cell::Cell::get) {
        return count;
    }
    count.min(std::thread::available_parallelism().map_or(1, usize::from))
}

/// The read-ahead budget; tests may set their own.
fn budget() -> u64 {
    #[cfg(test)]
    if let Some(budget) = TEST_BUDGET.with(std::cell::Cell::get) {
        return budget;
    }
    BUDGET
}

#[cfg(test)]
thread_local! {
    static TEST_BUDGET: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
    static TEST_UNSHARED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static TEST_READERS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    static TEST_GATE: std::cell::RefCell<Option<Arc<tests::Gate>>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `run` with every read waiting at `gate` before it decodes anything.
#[cfg(test)]
fn with_test_gate<T>(gate: Arc<tests::Gate>, run: impl FnOnce() -> T) -> T {
    struct Reset(Option<Arc<tests::Gate>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_GATE.with(|value| *value.borrow_mut() = self.0.take());
        }
    }
    let _reset = Reset(TEST_GATE.with(|value| value.replace(Some(gate))));
    run()
}

/// Run `run` with `count` reader threads.
#[cfg(test)]
fn with_test_readers<T>(count: usize, run: impl FnOnce() -> T) -> T {
    struct Reset(Option<usize>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_READERS.with(|value| value.set(self.0));
        }
    }
    let _reset = Reset(TEST_READERS.with(|value| value.replace(Some(count))));
    run()
}

/// Run `run` with every family reading each block alone, as before the
/// shared reader: the reference of its parity tests.
#[cfg(test)]
pub(in crate::prepared::pipeline) fn with_test_unshared<T>(run: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_UNSHARED.with(|value| value.set(self.0));
        }
    }
    let _reset = Reset(TEST_UNSHARED.with(|value| value.replace(true)));
    run()
}

/// Run `run` with a read-ahead budget of `bytes`.
#[cfg(test)]
pub(in crate::prepared::pipeline) fn with_test_budget<T>(bytes: u64, run: impl FnOnce() -> T) -> T {
    struct Reset(Option<u64>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_BUDGET.with(|value| value.set(self.0));
        }
    }
    let _reset = Reset(TEST_BUDGET.with(|value| value.replace(Some(bytes))));
    run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_core::test_fixtures::{
        CAMPAIGN, continuation_records, records, scale_frame, source,
    };

    /// `blocks` blocks of four hours on one branch.
    fn recording(blocks: i32) -> Replay {
        let frames = |block: i32| -> Vec<DecodedFrame> {
            (1..=4)
                .map(|hour| {
                    let hour = block * 4 + hour;
                    source(1, hour, Some(i64::from(hour) * 1000))
                })
                .collect()
        };
        let mut replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames(0))]).unwrap();
        for block in 1..blocks {
            replay = replay
                .append(&[continuation_records(&replay, 1, frames(block))])
                .unwrap();
        }
        replay
    }

    #[test]
    fn retaining_only_a_blocks_final_frame_avoids_shared_baseline_copies() {
        let replay = recording(1);
        let index = &replay.index[0];
        let table = TableId::Countries;
        let before = replay_frame::cpu::delta_copies()[table.index()];
        let mut expected = None;
        replay
            .read_block_alone(
                index,
                &[table],
                |_, frame| {
                    expected = Some(frame.clone());
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        let retained = replay_frame::cpu::delta_copies()[table.index()];
        assert_eq!([retained[0] - before[0], retained[1] - before[1]], [3, 3]);
        let final_frame = read_last(&replay, index, &[table]).unwrap();
        assert_eq!(replay_frame::cpu::delta_copies()[table.index()], retained);
        assert_eq!(final_frame, expected);
        let sampled = |frame: &Option<DecodedFrame>| {
            frame
                .as_ref()
                .unwrap()
                .tables
                .iter()
                .map(|view| view.sampled_at())
                .collect::<Vec<_>>()
        };
        assert_eq!(sampled(&final_frame), sampled(&expected));
    }

    /// `blocks` blocks of four hours on one branch, block `b` holding
    /// `countries(b)` countries.
    fn scaled(blocks: i32, countries: impl Fn(i32) -> u32) -> Replay {
        let frames = |block: i32| -> Vec<DecodedFrame> {
            (1..=4)
                .map(|hour| scale_frame(1, block * 4 + hour, countries(block)))
                .collect()
        };
        let mut replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames(0))]).unwrap();
        for block in 1..blocks {
            replay = replay
                .append(&[continuation_records(&replay, 1, frames(block))])
                .unwrap();
        }
        replay
    }

    /// What one block's read with `tables` (and Campaign) holds decoded.
    fn decoded(replay: &Replay, index: &replay::container::BlockIndex, tables: &[TableId]) -> u64 {
        let frames: Vec<Arc<Frame>> = replay.read_block_kept(index, tables, || Ok(())).unwrap();
        frames_bytes(&frames)
    }

    /// The largest block's [`decoded`] bytes.
    fn largest(replay: &Replay, tables: &[TableId]) -> u64 {
        replay
            .index
            .iter()
            .map(|index| decoded(replay, index, tables))
            .max()
            .unwrap()
    }

    /// Read `block` for `member` with `tables` through the shared reader,
    /// checking every hour against the family's own read alone.
    fn exact(replay: &Replay, member: &Member<'_>, block: usize, tables: &[TableId]) {
        let index = &replay.index[block];
        let mut alone = Vec::new();
        replay
            .read_block_alone(
                index,
                tables,
                |_, frame| {
                    alone.push(frame.clone());
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        let mut position = 0;
        read_alone(
            replay,
            index,
            Some(Lease { member, block }),
            tables,
            None,
            |_, frame| {
                assert_eq!(frame, &alone[position], "block {block}");
                position += 1;
                Ok(())
            },
            || Ok(()),
        )
        .unwrap();
        assert_eq!(position, alone.len(), "block {block}: every hour");
    }

    /// Wait until `ready`, at most ten seconds.
    fn until(what: &str, mut ready: impl FnMut() -> bool) {
        let started = Instant::now();
        while !ready() {
            assert!(started.elapsed() < Duration::from_secs(10), "{what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A pool extract that keeps nothing.
    struct Nothing;

    impl prepass::Extract for Nothing {
        fn observe(
            &mut self,
            _: &Replay,
            _: &DecodedFrame,
            _: &crate::prepared::frame_changes::FrameChanges,
        ) -> io::Result<()> {
            Ok(())
        }

        fn finish(self: Box<Self>) -> io::Result<Box<dyn std::any::Any + Send>> {
            Ok(Box::new(()))
        }
    }

    static COUNTRIES: prepass::Family = prepass::Family {
        name: "countries",
        extract: "countries.extract",
        sequential: "countries.sequential",
        fallback: "countries.fallback",
        tables: &[TableId::Countries],
        fields: None,
        start: |_| -> Box<dyn prepass::Extract> { Box::new(Nothing) },
    };

    /// A gate reads wait at before decoding anything, counting those that
    /// reached it.
    #[derive(Default)]
    pub(super) struct Gate {
        state: Mutex<(usize, bool)>,
        changed: Condvar,
    }

    impl Gate {
        /// A read reached the gate: wait until it opens.
        pub(super) fn pass(&self) {
            let mut state = self.state.lock().unwrap();
            state.0 += 1;
            self.changed.notify_all();
            while !state.1 {
                state = self.changed.wait(state).unwrap();
            }
        }

        /// Wait until `count` reads reached the gate.
        fn reached(&self, count: usize) {
            let started = Instant::now();
            let mut state = self.state.lock().unwrap();
            while state.0 < count {
                assert!(
                    started.elapsed() < Duration::from_secs(10),
                    "reads reach the gate"
                );
                state = self
                    .changed
                    .wait_timeout(state, Duration::from_millis(50))
                    .unwrap()
                    .0;
            }
        }

        fn open(&self) {
            self.state.lock().unwrap().1 = true;
            self.changed.notify_all();
        }
    }

    /// A family stalled at the first block and a faster one taking every
    /// block, with one reader: what is charged stays within the budget, the
    /// faster family's frames are its own read's, and sharing stays on.
    #[test]
    fn blocks_kept_for_a_stalled_family_stay_within_the_budget() {
        let blocks = 24;
        let replay = Arc::new(recording(blocks));
        let tables = [TableId::Countries];
        let budget = 3 * largest(&replay, &tables);
        with_test_readers(1, || {
            with_test_budget(budget, || {
                let shared = Shared::start(
                    Arc::clone(&replay),
                    &[(&tables[..], 0), (&tables[..], 0)],
                    true,
                )
                .unwrap();
                let stalled = shared.member(0, false).unwrap();
                stalled.at(0, true);
                let fast = shared.member(1, false).unwrap();
                for block in 0..blocks as usize {
                    fast.at(block, true);
                    exact(&replay, &fast, block, &tables);
                    let (held, peak) = shared.held();
                    assert!(
                        held <= budget && peak <= budget,
                        "block {block}: {held} bytes charged ({peak} at most), budget {budget}"
                    );
                }
                assert!(shared.sharing());
                drop(fast);
                drop(stalled);
            })
        });
    }

    /// The pre-pass pool's block tasks read the blocks a member will take
    /// with its tables and deposit them: the member reconstructs each from
    /// the deposit exactly as it reads the block alone, and no reader reads
    /// any of them.
    #[test]
    fn pool_block_tasks_deposit_the_blocks_members_take() {
        let blocks = 4;
        let replay = Arc::new(recording(blocks));
        let tables = [TableId::Clock];
        with_test_readers(0, || {
            let shared = Shared::start(Arc::clone(&replay), &[(&tables[..], 0)], true).unwrap();
            let member = shared.member(0, false).unwrap();
            member.at(0, true);
            let pool = prepass::Pool::start(
                Arc::clone(&replay),
                vec![(&COUNTRIES, 0, false)],
                1,
                prepass::Knobs::default(),
                Some(shared.depot()),
            );
            let cancelled = AtomicBool::new(false);
            for block in 0..blocks as usize {
                member.at(block, true);
                pool.take(0, block, &cancelled).unwrap();
                exact(&replay, &member, block, &tables);
            }
            drop(pool);
            assert_eq!(shared.deposits(), blocks as u64, "every block deposited");
            assert_eq!(
                shared.read_time().1,
                0,
                "the readers read no deposited block"
            );
            assert!(
                shared.alone_blocks().is_empty(),
                "every block taken from a deposit"
            );
            drop(member);
        });
    }

    /// A family stalled at the first block while the pool runs far ahead:
    /// what is deposited and kept for it stays within the budget, blocks
    /// beyond its reach are not deposited, the pool never waits for it, and
    /// every frame it then reads is its own read's.
    #[test]
    fn deposits_for_a_stalled_family_stay_within_the_budget() {
        let blocks = 24;
        let replay = Arc::new(recording(blocks));
        let tables = [TableId::Countries];
        let budget = 4 * largest(&replay, &tables);
        with_test_readers(0, || {
            with_test_budget(budget, || {
                let shared = Shared::start(Arc::clone(&replay), &[(&tables[..], 0)], true).unwrap();
                let stalled = shared.member(0, false).unwrap();
                stalled.at(0, true);
                let pool = prepass::Pool::start(
                    Arc::clone(&replay),
                    vec![(&COUNTRIES, 0, false)],
                    2,
                    prepass::Knobs::default(),
                    Some(shared.depot()),
                );
                let cancelled = AtomicBool::new(false);
                for block in 0..blocks as usize {
                    pool.take(0, block, &cancelled).unwrap();
                    let (held, _) = shared.held();
                    assert!(held <= budget, "block {block}: {held} bytes charged");
                }
                drop(pool);
                assert!(
                    shared.deposits() < blocks as u64,
                    "blocks beyond reach are not deposited"
                );
                for block in 0..blocks as usize {
                    stalled.at(block, true);
                    exact(&replay, &stalled, block, &tables);
                    assert!(shared.kept_bytes() <= budget, "block {block}");
                }
                drop(stalled);
            })
        });
    }

    /// A reader reading a family's next block, which the families pass
    /// while the read waits to decode: the read is given up before it
    /// completes and the block is not kept.
    #[test]
    fn reads_of_blocks_their_families_passed_are_given_up_unkept() {
        let replay = Arc::new(recording(16));
        let tables = [TableId::Countries];
        let gate = Arc::new(Gate::default());
        with_test_readers(2, || {
            with_test_gate(Arc::clone(&gate), || {
                let shared = Shared::start(
                    Arc::clone(&replay),
                    &[(&tables[..], 9), (&tables[..], 11)],
                    true,
                )
                .unwrap();
                let first = shared.member(0, false).unwrap();
                let second = shared.member(1, false).unwrap();
                // The first block measured is read alone: block 9.
                gate.reached(1);
                first.at(12, true);
                second.at(14, true);
                gate.open();
                until("the passed block's read given up", || {
                    shared.dropped().contains(&9)
                });
                assert!(shared.kept_tables(9).is_empty(), "block 9 is not kept");
                assert!(
                    !shared.read_blocks().contains(&9),
                    "block 9 was not read through"
                );
                drop(first);
                drop(second);
            })
        });
    }

    /// A family that finished leaves its tables out of later reads: a block
    /// read while it prepares holds them, one read after it finished does
    /// not, and the family still preparing reads both exactly.
    #[test]
    fn a_finished_familys_tables_are_left_out_of_later_reads() {
        let replay = Arc::new(recording(6));
        let (clock, countries) = ([TableId::Clock], [TableId::Countries]);
        with_test_readers(1, || {
            let shared = Shared::start(
                Arc::clone(&replay),
                &[(&clock[..], 0), (&countries[..], 0)],
                true,
            )
            .unwrap();
            let small = shared.member(0, false).unwrap();
            small.at(0, true);
            let large = shared.member(1, false).unwrap();
            large.at(0, true);
            exact(&replay, &small, 0, &clock);
            assert!(shared.kept_tables(0).contains(&TableId::Countries));
            drop(large);
            small.at(3, true);
            exact(&replay, &small, 3, &clock);
            let kept = shared.kept_tables(3);
            assert!(kept.contains(&TableId::Clock), "{kept:?}");
            assert!(!kept.contains(&TableId::Countries), "{kept:?}");
            drop(small);
        });
    }

    /// A family ahead of this block will never take it again, even while
    /// it is still preparing. Readers and pool deposits select the same
    /// tables, and the family behind still reads its own frames exactly.
    #[test]
    fn a_passed_blocks_tables_are_left_out_while_its_family_still_prepares() {
        let replay = Arc::new(recording(6));
        let (clock, countries) = ([TableId::Clock], [TableId::Countries]);
        with_test_readers(0, || {
            let shared = Shared::start(
                Arc::clone(&replay),
                &[(&clock[..], 0), (&countries[..], 3)],
                true,
            )
            .unwrap();
            let small = shared.member(0, false).unwrap();
            let large = shared.member(1, false).unwrap();
            let depot = shared.depot();
            for block in [0, 3, 4] {
                if block == 4 {
                    large.at(5, true);
                }
                small.at(block, true);
                let offered = depot.offer(block).expect("the block is within reach");
                assert_eq!(offered.contains(&TableId::Countries), block == 3);
                let Ended::Read(frames, selected) = shared.inner.read(block) else {
                    panic!("the reader reads the block");
                };
                for table in TableId::ALL {
                    assert_eq!(offered.contains(&table), selected[table.index()]);
                }
                depot.deposit(block, frames);
                exact(&replay, &small, block, &clock);
                if block == 3 {
                    exact(&replay, &large, block, &countries);
                }
            }
            drop((small, large));
        });
    }

    /// A refusal settles only that branch: its tables are omitted there,
    /// but are kept for the same family's next branch before it reaches it.
    #[test]
    fn a_refused_branchs_tables_are_left_out_without_omitting_the_next_branchs() {
        let replay = recording(2)
            .append(&[records(2, vec![source(2, 20, Some(20_000))])])
            .unwrap();
        let replay = Arc::new(replay);
        let (clock, countries) = ([TableId::Clock], [TableId::Countries]);
        with_test_readers(0, || {
            let shared = Shared::start(
                Arc::clone(&replay),
                &[(&clock[..], 0), (&countries[..], 0)],
                true,
            )
            .unwrap();
            let small = shared.member(0, false).unwrap();
            let large = shared.member(1, false).unwrap();
            large.at(0, false);
            let depot = shared.depot();
            for block in 0..replay.index.len() {
                small.at(block, true);
                let offered = depot.offer(block).expect("the block is within reach");
                assert_eq!(
                    offered.contains(&TableId::Countries),
                    replay.index[block].branch == 2
                );
                let frames = replay
                    .read_block_kept(&replay.index[block], &offered, || Ok(()))
                    .unwrap();
                depot.deposit(block, frames);
                exact(&replay, &small, block, &clock);
                if replay.index[block].branch == 2 {
                    large.at(block, true);
                    exact(&replay, &large, block, &countries);
                }
            }
            drop((small, large));
        });
    }

    /// Four small-table families at distinct next blocks, read with a large
    /// table a fifth family still preparing reads, and one reader: the first
    /// block measured outweighs a block's share, so the source is read
    /// standalone, every family reads its blocks with its own tables alone
    /// and keeps nothing, every frame is its own read's, and what is charged
    /// stays within the bound.
    #[test]
    fn a_family_reads_a_block_that_cannot_be_shared_with_its_own_tables() {
        let blocks = 16;
        let replay = Arc::new(scaled(blocks, |_| 3_000));
        let (clock, countries) = ([TableId::Clock], [TableId::Countries]);
        let union = largest(&replay, &[TableId::Clock, TableId::Countries]);
        let own = largest(&replay, &clock);
        assert!(
            union > 4 * own,
            "the shared tables outweigh each family's own"
        );
        let budget = union / 2;
        with_test_readers(1, || {
            with_test_budget(budget, || {
                let shared = Shared::start(
                    Arc::clone(&replay),
                    &[
                        (&clock[..], 0),
                        (&clock[..], 2),
                        (&clock[..], 4),
                        (&clock[..], 6),
                        (&countries[..], 0),
                    ],
                    true,
                )
                .unwrap();
                let members: Vec<Member<'_>> =
                    (0..5).map(|at| shared.member(at, false).unwrap()).collect();
                for (member, next) in members.iter().zip([0, 2, 4, 6, 0]) {
                    member.at(next, true);
                }
                until("the first block measured", || !shared.sharing());
                for (member, block) in members.iter().zip([0, 2, 4, 6]) {
                    exact(&replay, member, block, &clock);
                    let (held, peak) = shared.held();
                    assert!(
                        held <= budget && peak <= budget + union,
                        "block {block}: {held} bytes charged ({peak} at most), budget {budget}"
                    );
                }
                assert!(shared.kept_bytes() == 0, "nothing kept read standalone");
                let alone = shared.alone_blocks();
                assert!(alone.len() >= 4, "every family read alone: {alone:?}");
                drop(members);
            })
        });
    }

    /// Five families at distinct blocks and no reader: no family reads a
    /// block for every family; each reads its own with its own tables alone,
    /// exactly as its own read, and nothing is read for every family.
    #[test]
    fn no_family_reads_a_block_for_every_family() {
        let replay = Arc::new(recording(8));
        let tables = [TableId::Countries];
        with_test_readers(0, || {
            let consumers: Vec<(&[TableId], usize)> =
                (0..5).map(|next| (&tables[..], next)).collect();
            let shared = Shared::start(Arc::clone(&replay), &consumers, true).unwrap();
            let members: Vec<Member<'_>> =
                (0..5).map(|at| shared.member(at, false).unwrap()).collect();
            for (member, block) in members.iter().zip(0..5) {
                member.at(block, true);
            }
            for (member, block) in members.iter().zip(0..5) {
                exact(&replay, member, block, &tables);
            }
            assert!(
                shared.read_blocks().is_empty(),
                "nothing read for every family"
            );
            let mut alone = shared.alone_blocks();
            alone.sort_unstable();
            assert_eq!(alone, [0, 1, 2, 3, 4]);
            drop(members);
        });
    }

    /// A kept block a family took stays charged after it is released from
    /// those kept (no family takes it any more) until the family drops it.
    #[test]
    fn a_released_block_stays_charged_until_its_holder_drops_it() {
        let replay = Arc::new(recording(8));
        let tables = [TableId::Countries];
        with_test_readers(1, || {
            let shared = Shared::start(Arc::clone(&replay), &[(&tables[..], 0)], true).unwrap();
            let member = shared.member(0, false).unwrap();
            member.at(0, true);
            let frames = member
                .frames(0, &tables, &mut || Ok(()))
                .unwrap()
                .expect("the reader reads the family's next block");
            let bytes = frames_bytes(&frames.frames);
            // No family wants anything: block 0 is released from those kept.
            drop(member);
            until("block 0 released and nothing else charged", || {
                shared.kept_tables(0).is_empty() && shared.held().0 == bytes
            });
            assert_eq!(shared.alive_bytes(), bytes, "still charged while held");
            drop(frames);
            assert_eq!(shared.alive_bytes(), 0, "released once its holder drops it");
        });
    }

    /// The first block measured outweighs a block's share of the budget: the
    /// source is read standalone for the rest of the run, nothing more is
    /// read for every family, and every family's frames are its own read's.
    #[test]
    fn an_oversized_block_reads_the_source_standalone() {
        let blocks = 6;
        let replay = Arc::new(scaled(blocks, |_| 3_000));
        let tables = [TableId::Countries];
        let budget = largest(&replay, &tables);
        with_test_readers(1, || {
            with_test_budget(budget, || {
                let shared = Shared::start(
                    Arc::clone(&replay),
                    &[(&tables[..], 0), (&tables[..], 0)],
                    true,
                )
                .unwrap();
                let first = shared.member(0, false).unwrap();
                let second = shared.member(1, false).unwrap();
                for block in 0..blocks as usize {
                    first.at(block, true);
                    second.at(block, true);
                    exact(&replay, &first, block, &tables);
                    exact(&replay, &second, block, &tables);
                }
                assert!(!shared.sharing(), "read standalone");
                assert!(
                    shared.read_blocks().len() <= 1,
                    "only the block measured was read for every family: {:?}",
                    shared.read_blocks()
                );
                assert_eq!(shared.kept_bytes(), 0);
                drop(first);
                drop(second);
            })
        });
    }

    /// Small blocks and two families moving together: sharing stays on,
    /// every block is read once for both, and neither reads one alone.
    #[test]
    fn small_blocks_are_shared_and_each_read_once() {
        let blocks = 8;
        let replay = Arc::new(recording(blocks));
        let tables = [TableId::Countries];
        with_test_readers(2, || {
            let shared = Shared::start(
                Arc::clone(&replay),
                &[(&tables[..], 0), (&tables[..], 0)],
                true,
            )
            .unwrap();
            let first = shared.member(0, false).unwrap();
            let second = shared.member(1, false).unwrap();
            for block in 0..blocks as usize {
                first.at(block, true);
                second.at(block, true);
                exact(&replay, &first, block, &tables);
                exact(&replay, &second, block, &tables);
            }
            assert!(shared.sharing(), "sharing stays on");
            let mut read = shared.read_blocks();
            read.sort_unstable();
            assert_eq!(
                read,
                (0..blocks as usize).collect::<Vec<_>>(),
                "each read once"
            );
            assert!(shared.alone_blocks().is_empty(), "none read alone");
            drop(first);
            drop(second);
        });
    }

    /// [`exact`] of `block` for `member`, giving up with an error at
    /// `deadline` rather than waiting for ever.
    fn exact_by(
        replay: &Replay,
        member: &Member<'_>,
        block: usize,
        tables: &[TableId],
        deadline: Instant,
    ) -> io::Result<()> {
        let index = &replay.index[block];
        let mut alone = Vec::new();
        replay.read_block_recorded_alone(
            index,
            tables,
            |_, frame| {
                alone.push(frame.clone());
                Ok(())
            },
            || Ok(()),
        )?;
        let mut position = 0;
        read_alone(
            replay,
            index,
            Some(Lease { member, block }),
            tables,
            None,
            |_, frame| {
                assert_eq!(frame, &alone[position], "block {block}");
                position += 1;
                Ok(())
            },
            || {
                if Instant::now() > deadline {
                    Err(io::Error::from(io::ErrorKind::TimedOut))
                } else {
                    Ok(())
                }
            },
        )?;
        assert_eq!(position, alone.len(), "block {block}: every hour");
        Ok(())
    }

    /// Small blocks first, then blocks each outweighing a block's share:
    /// sharing is established on the small blocks while three families,
    /// each holding the first block it took, move through the recording on
    /// threads of their own beside two readers and two pool workers reading
    /// and depositing at once; a later block measured beyond its share
    /// switches the source to standalone. What is charged stays within the
    /// bound throughout, every family completes (none waits for a wake-up
    /// that never comes), and every family's frames are its own read's.
    #[test]
    fn a_later_oversized_block_switches_a_busy_shared_reader_to_standalone() {
        let blocks = 10;
        let small_blocks = 4;
        let replay = Arc::new(scaled(
            blocks,
            |block| {
                if block < small_blocks { 3 } else { 3_000 }
            },
        ));
        let tables = [TableId::Countries];
        let sizes: Vec<u64> = replay
            .index
            .iter()
            .map(|index| decoded(&replay, index, &tables))
            .collect();
        let small = *sizes[..small_blocks as usize].iter().max().unwrap();
        let large = *sizes.iter().max().unwrap();
        let readers = 2;
        let workers = 2;
        // A block's share is twice the largest small block.
        let budget = 2 * small * (readers as u64 + 2);
        let share = budget / (readers as u64 + 2);
        assert!(
            sizes[small_blocks as usize..]
                .iter()
                .all(|&size| size > share),
            "every later block outweighs a block's share: {sizes:?}, share {share}"
        );
        with_test_readers(readers, || {
            with_test_budget(budget, || {
                let consumers: Vec<(&[TableId], usize)> =
                    (0..3).map(|_| (&tables[..], 0)).collect();
                let shared = Shared::start(Arc::clone(&replay), &consumers, true).unwrap();
                let members: Vec<Member<'_>> =
                    (0..3).map(|at| shared.member(at, false).unwrap()).collect();
                let pool = prepass::Pool::start(
                    Arc::clone(&replay),
                    vec![(&COUNTRIES, 0, false)],
                    workers,
                    prepass::Knobs::default(),
                    Some(shared.depot()),
                );
                let deadline = Instant::now() + Duration::from_secs(60);
                let done = std::thread::scope(|scope| {
                    let families: Vec<_> = members
                        .iter()
                        .map(|member| {
                            let replay = &replay;
                            scope.spawn(move || -> io::Result<()> {
                                member.at(0, true);
                                // Held to the end: the first block stays
                                // charged while this family holds it.
                                let first = member.frames(0, &tables, &mut || Ok(()))?;
                                for block in 0..blocks as usize {
                                    member.at(block, true);
                                    exact_by(replay, member, block, &tables, deadline)?;
                                }
                                drop(first);
                                Ok(())
                            })
                        })
                        .collect();
                    let cancelled = AtomicBool::new(false);
                    for block in 0..blocks as usize {
                        pool.take(0, block, &cancelled).unwrap();
                        let (held, peak) = shared.held();
                        let bound = budget + (readers + workers + 1) as u64 * large;
                        assert!(
                            held <= bound && peak <= bound,
                            "pool at block {block}: {held} bytes charged ({peak} at most), bound {bound}"
                        );
                    }
                    families
                        .into_iter()
                        .map(|family| family.join().expect("a family's thread"))
                        .collect::<Vec<_>>()
                });
                drop(pool);
                for (at, family) in done.into_iter().enumerate() {
                    family.unwrap_or_else(|error| panic!("family {at} did not complete: {error}"));
                }
                assert!(!shared.sharing(), "a later oversized block ends sharing");
                let (_, peak) = shared.held();
                let bound = budget + (readers + workers + 1) as u64 * large;
                assert!(peak <= bound, "{peak} bytes charged at most, bound {bound}");
                let mut shared_reads = shared.read_blocks();
                shared_reads.sort_unstable();
                assert!(
                    shared_reads
                        .iter()
                        .any(|&block| block < small_blocks as usize),
                    "sharing was established on the small blocks: {shared_reads:?}"
                );
                drop(members);
                assert_eq!(shared.alive_bytes(), 0, "every block released once dropped");
            })
        });
    }
}
