//! One read of each block for the families prepared on their own whose own
//! thread observes every block hour by hour on the block read alone
//! ([`super::Own::shared_tables`]).
//!
//! Each such block is read once ([`Replay::read_block_selected`]) for every
//! table any of those families reads, on reader threads ahead of the
//! families, and each family reconstructs it for its own tables on its own
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
//! A family takes its blocks in order and never waits for another: it wants
//! the block it takes next and the next [`AHEAD`] read, and readers read the
//! wanted blocks nearest to a family's next first. A block read is kept while
//! a slower family will still take it, so it is read once for all of them;
//! blocks read and kept stay within [`BUDGET`] of recorded raw bytes beyond
//! each family's next block, as the pre-pass pool's read-ahead does: every
//! block read, by a reader or by a family itself, is admitted through the
//! budget, and whenever the blocks held pass it the kept blocks no family
//! wants now are released, furthest from the next block of the family taking
//! them first, and read again when wanted. Beyond the budget only the blocks
//! families want now are held. Families far apart (moving at different
//! speeds, or resumed from blocks far apart) so read some blocks again on the
//! readers rather than wait for one another. A family that does not read the
//! block it takes next (it refused that block's branch) wants nothing.

#[cfg(test)]
use crate::prepared::prepass;
use dashboard_base::prepared::priority::{JobPriority, Level};
use replay::{
    container::Replay,
    frame::{DecodedFrame, Frame, TABLE_COUNT},
    rows::TableId,
};
use std::{
    collections::BTreeMap,
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// Blocks read ahead of the one a family takes next.
const AHEAD: usize = 1;

/// Recorded raw bytes of the blocks read or being read and kept, past which
/// readers release kept blocks or wait, beyond each family's next block: a
/// budget for read-ahead memory, never a limit on blocks, hours or families.
const BUDGET: u64 = 512 * 1024 * 1024;

/// The timing row of the shared reads (worker time), and in tests the count
/// of blocks read.
pub(in crate::prepared::pipeline) const SHARED_READ: &str = "independent.shared_read";

/// Each block a family read alone because its shared read could not serve
/// it, counted in tests.
#[cfg(test)]
pub(in crate::prepared::pipeline) const SHARED_ALONE: &str = "independent.shared_alone";

/// A block's frames as its one selected read returned them, each with its
/// recorded updates of every table read.
pub(super) struct Frames(Vec<Arc<Frame>>);

enum Entry {
    /// A reader or a family is reading it.
    Reading,
    Read(Arc<Frames>),
    /// Its read failed or cannot serve a family's: each reads it alone.
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

struct State {
    consumers: Vec<Consumer>,
    entries: BTreeMap<usize, Entry>,
    sizes: Vec<u64>,
    held: u64,
    budget: u64,
    /// The most recorded raw bytes held at once.
    peak: u64,
    /// Time spent reading and blocks read.
    spent: Duration,
    reads: u64,
}

impl State {
    /// The live families reading the block they take next.
    fn reading(&self) -> impl Iterator<Item = &Consumer> {
        self.consumers
            .iter()
            .filter(|consumer| consumer.member && consumer.live && consumer.reads)
    }

    /// Whether some family wants `block` now: it takes it next or within
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

    /// Room for `bytes` more within the budget, releasing kept blocks no
    /// family wants now, furthest from the family taking them first; whether
    /// they fit.
    fn room(&mut self, bytes: u64) -> bool {
        while self.held + bytes > self.budget {
            let furthest = self
                .entries
                .iter()
                .filter(|(block, entry)| matches!(entry, Entry::Read(_)) && !self.wanted(**block))
                .max_by_key(|(block, _)| self.behind(**block))
                .map(|(block, _)| *block);
            let Some(block) = furthest else {
                return false;
            };
            self.entries.remove(&block);
            self.held = self.held.saturating_sub(self.sizes[block]);
        }
        true
    }

    /// The block a reader reads next: of the blocks some family wants and
    /// nobody has read, the nearest to a family's next block, then the
    /// first. A family's next block is always read, after releasing kept
    /// blocks to make room for it; another only within the budget.
    fn unread(&mut self) -> Option<usize> {
        let end = self.sizes.len();
        let (distance, block) = self
            .reading()
            .flat_map(|consumer| {
                (0..=AHEAD).map(move |distance| (distance, consumer.next + distance))
            })
            .filter(|&(_, block)| block < end && !self.entries.contains_key(&block))
            .min()?;
        let fits = self.room(self.sizes[block]);
        (distance == 0 || fits).then_some(block)
    }

    /// `block` is being read: its bytes are held, after releasing kept
    /// blocks to make room for them. Beyond the budget, only blocks families
    /// want now are held.
    fn admit(&mut self, block: usize) {
        let size = self.sizes[block];
        self.room(size);
        self.entries.insert(block, Entry::Reading);
        self.held += size;
        self.peak = self.peak.max(self.held);
    }

    /// Release what no family will take, then kept blocks beyond the budget,
    /// furthest from the family taking them first: that family reads them
    /// again.
    fn prune(&mut self) {
        self.release();
        self.room(0);
    }

    /// Release what no family will take.
    fn release(&mut self) {
        let released: Vec<usize> = self
            .entries
            .iter()
            .filter(|(block, entry)| {
                !matches!(entry, Entry::Reading) && self.behind(**block).is_none()
            })
            .map(|(block, _)| *block)
            .collect();
        for block in released {
            if let Some(Entry::Read(_)) = self.entries.remove(&block) {
                self.held = self.held.saturating_sub(self.sizes[block]);
            }
        }
    }
}

struct Inner {
    replay: Arc<Replay>,
    /// Every table a member reads, Campaign included, and the same as a set.
    tables: Vec<TableId>,
    chosen: [bool; TABLE_COUNT],
    state: Mutex<State>,
    changed: Condvar,
    stop: AtomicBool,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// The next block a reader reads; none once the readers stop.
    fn claim(&self) -> Option<usize> {
        let mut state = self.lock();
        loop {
            if self.stop.load(Ordering::Acquire) {
                return None;
            }
            if let Some(block) = state.unread() {
                state.admit(block);
                return Some(block);
            }
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    /// Read `block` once for every member's tables; a failed or panicked
    /// read, or one an all-table keyframe check would refuse for another
    /// member's table, leaves each member to read it alone.
    fn read(&self, block: usize) -> Entry {
        let started = Instant::now();
        let index = &self.replay.index[block];
        let read = catch_unwind(AssertUnwindSafe(|| {
            let frames = self.replay.read_block_selected(index, &self.tables)?;
            engine::allocation::collect_vec(frames.into_iter().map(Arc::new))
        }));
        let entry = match read {
            Ok(Ok(frames)) => Entry::Read(Arc::new(Frames(frames))),
            _ => Entry::Alone,
        };
        let mut state = self.lock();
        state.spent += started.elapsed();
        state.reads += 1;
        entry
    }

    /// Keep `block`'s read for the families that want it.
    fn store(&self, block: usize, entry: Entry) {
        let mut state = self.lock();
        if !matches!(entry, Entry::Read(_)) {
            state.held = state.held.saturating_sub(state.sizes[block]);
        }
        state.entries.insert(block, entry);
        state.prune();
        drop(state);
        self.changed.notify_all();
    }

    fn update(&self, change: impl FnOnce(&mut State)) {
        let mut state = self.lock();
        change(&mut state);
        state.prune();
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
        let tables: Vec<TableId> = TableId::ALL
            .into_iter()
            .filter(|table| chosen[table.index()])
            .collect();
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
                sizes: replay.index.iter().map(|block| block.raw_length).collect(),
                held: 0,
                budget: budget(),
                peak: 0,
                spent: Duration::ZERO,
                reads: 0,
            }),
            replay,
            tables,
            chosen,
            changed: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        // One reader per family reading through it: each family's next
        // block and the one after can be read at once.
        let count = readers(
            consumers
                .iter()
                .filter(|(tables, _)| !tables.is_empty())
                .count()
                .max(1),
        );
        let mut readers = Vec::with_capacity(count);
        for reader in 0..count {
            let inner = Arc::clone(&inner);
            let priority = JobPriority::current();
            // A reader that cannot start leaves its blocks to the others or
            // to the families themselves.
            if let Ok(handle) = std::thread::Builder::new()
                .name(format!("preparation shared block reader {reader}"))
                .spawn(move || {
                    priority.enter(Level::BelowNormal);
                    while let Some(block) = inner.claim() {
                        let entry = inner.read(block);
                        inner.store(block, entry);
                    }
                })
            {
                readers.push(handle);
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

    /// The recorded raw bytes held now and at most.
    #[cfg(test)]
    fn held(&self) -> (u64, u64) {
        let state = self.inner.lock();
        (state.held, state.peak)
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

    /// Whether its shared read holds every one of `tables`.
    fn holds(&self, tables: &[TableId]) -> bool {
        tables
            .iter()
            .all(|table| self.shared.inner.chosen[table.index()])
    }

    /// `block`'s shared read, waiting while a reader reads it and reading it
    /// here when nobody does; None when the family reads it alone.
    fn frames(
        &self,
        block: usize,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<Arc<Frames>>> {
        let inner = &self.shared.inner;
        let mut state = inner.lock();
        loop {
            check()?;
            match state.entries.get(&block) {
                Some(Entry::Read(frames)) => return Ok(Some(Arc::clone(frames))),
                Some(Entry::Alone) => return Ok(None),
                Some(Entry::Reading) => {}
                None => {
                    state.admit(block);
                    drop(state);
                    let entry = inner.read(block);
                    let frames = match &entry {
                        Entry::Read(frames) => Some(Arc::clone(frames)),
                        _ => None,
                    };
                    inner.store(block, entry);
                    return Ok(frames);
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
/// when `lease` holds one that serves it, otherwise read here.
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
        && let Some(frames) = lease.member.frames(lease.block, &mut check)?
    {
        let through = last.map_or(frames.0.len(), |last| {
            frames.0.partition_point(|frame| frame.hour <= last)
        });
        if replay.reconstruct_alone(
            &frames.0[..through],
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
        return replay.read_block_alone(
            index,
            tables,
            |raw, frame| visit(Arc::new(raw), frame),
            check,
        );
    };
    // Through `last`: the hours after it are not visited.
    let mut passed = false;
    let read = replay.read_block_alone(
        index,
        tables,
        |raw, frame| {
            if frame.hour > last {
                passed = true;
                return Err(io::Error::other("Read through its last hour"));
            }
            visit(Arc::new(raw), frame)
        },
        check,
    );
    match read {
        Err(_) if passed => Ok(()),
        read => read,
    }
}

/// `count` reader threads; tests may start none, leaving every block to the
/// families.
fn readers(count: usize) -> usize {
    #[cfg(test)]
    if let Some(count) = TEST_READERS.with(std::cell::Cell::get) {
        return count;
    }
    count
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

/// Run `run` with a read-ahead budget of `bytes`: zero reads only each
/// family's next block ahead.
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
    use dashboard_core::test_fixtures::{CAMPAIGN, continuation_records, records, source};

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

    /// With no reader threads, a family stalled at the first block and a
    /// faster one reading every block itself: the blocks kept for the
    /// stalled family stay within the budget (four blocks, those two
    /// families want at most at once), the faster family's passed blocks
    /// released for the stalled one to read again, rather than every block
    /// the faster one read.
    #[test]
    fn blocks_kept_for_a_stalled_family_stay_within_the_budget() {
        let blocks = 24;
        let replay = Arc::new(recording(blocks));
        assert_eq!(replay.index.len(), blocks as usize);
        let largest = replay.index.iter().map(|block| block.raw_length).max().unwrap();
        let total: u64 = replay.index.iter().map(|block| block.raw_length).sum();
        let budget = 4 * largest;
        assert!(total > budget, "the recording is longer than the budget");
        let tables = [TableId::Countries];
        let (hours, reads, peak) = with_test_readers(0, || {
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
                let mut hours = 0;
                for (block, index) in replay.index.iter().enumerate() {
                    fast.at(block, true);
                    read_alone(
                        &replay,
                        index,
                        Some(Lease {
                            member: &fast,
                            block,
                        }),
                        &tables,
                        None,
                        |_, _| {
                            hours += 1;
                            Ok(())
                        },
                        || Ok(()),
                    )
                    .unwrap();
                    let (held, _) = shared.held();
                    assert!(
                        held <= budget,
                        "block {block}: {held} bytes held, budget {budget}"
                    );
                }
                drop(fast);
                let (_, peak) = shared.held();
                drop(stalled);
                (hours, shared.read_time().1, peak)
            })
        });
        assert_eq!(hours, blocks as usize * 4, "every hour read");
        assert_eq!(reads, blocks as u64, "every block read once, by the fast family");
        assert!(peak <= budget, "at most {peak} bytes held, budget {budget}");
    }
}
