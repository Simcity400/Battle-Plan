//! Block pre-pass: workers read upcoming blocks alone, each from its keyframe,
//! once for every family reading it ([`Replay::read_block_watched`]), and run
//! each family's extract on what its own read of the block alone hands it.
//! The coordinator folds each family's block facts in block order instead of
//! observing the family every hour.
//!
//! A family that declares the fields it reads ([`Family::fields`]) watches
//! only the hours those fields change, as [`Replay::read_block_changes`]
//! visits them: its extract observes those hours' frames, with the tables the
//! reader reports changed ([`Extract::observe_reported`]), and is told of the
//! quiet spans between them ([`Extract::quiet`]). Any other family watches
//! every hour of its tables, as [`Replay::read_block_alone`] reconstructs
//! them.
//!
//! [`PrePass`] serves the hourly walk: every block once, for all its families
//! together. [`Pool`] serves families prepared on their own: one set of
//! workers reads each block once for every family whose next block it is and
//! within reach, from each family's own first block, and each family takes
//! its blocks in order on its own.

use super::frame_changes::{FrameChangeTracker, FrameChanges};
pub(crate) use dashboard_base::prepared::prepass::{Bound, Extract, Family, Folded};
use dashboard_base::prepared::priority::{JobPriority, Level};
use replay::{
    container::{
        BlockChange, BlockIndex, BlockWatch, Keep, QuietHours, Replay, TableFields, Watched,
        Watching,
    },
    format::FormatVersion,
    frame::{DecodedFrame, SharedView, TableView},
    rows::TableId,
};
use std::{
    any::Any,
    collections::BTreeMap,
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Condvar, LazyLock, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// What a block's pre-pass left for one family.
pub(crate) enum Outcome {
    /// Facts the family folds after the block's ordinary observation.
    Facts(Box<dyn Any + Send>),
    /// Its extract refused, failed or could not read the block: the family
    /// observes this block hour by hour, as unmigrated families do, so its
    /// refusals and errors are exactly the ordinary ones.
    Sequential,
}

impl std::fmt::Debug for Outcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Facts(_) => "Facts",
            Self::Sequential => "Sequential",
        })
    }
}

/// Every migrated family, in the order of every block's outcomes.
pub(crate) fn families() -> Vec<&'static Family> {
    vec![
        &crate::products::resource_balance::prepared::FAMILY,
        &crate::products::army_size::prepared::FAMILY,
    ]
}

/// Whether migrated families are prepared through the pre-pass. Tests can
/// force the hourly observation the pre-pass replaces.
pub(crate) fn enabled() -> bool {
    #[cfg(test)]
    if TEST_SEQUENTIAL.with(std::cell::Cell::get) {
        return false;
    }
    true
}

/// Workers for a machine of `parallelism` processors: those the decoder and
/// coordinator leave, at least one.
pub(crate) fn workers(parallelism: usize) -> usize {
    #[cfg(test)]
    if let Some(workers) = TEST_WORKERS.with(std::cell::Cell::get) {
        return workers;
    }
    parallelism.saturating_sub(2).max(1)
}

/// The read-ahead budget (default [`AHEAD_BYTES`]) and test-only fault
/// injection and refusals, captured on the coordinator.
#[derive(Clone, Default)]
pub(crate) struct Knobs {
    pub budget: Option<u64>,
    #[cfg(test)]
    pub hook: Option<Hook>,
    #[cfg(test)]
    pub refusals: Vec<(&'static str, i32)>,
    /// Read every block whole, even for families that declare their fields.
    #[cfg(test)]
    pub alone: bool,
}

impl Knobs {
    /// The coordinator thread's test hook; nothing outside tests.
    pub(crate) fn current() -> Self {
        Self {
            budget: None,
            #[cfg(test)]
            hook: TEST_HOOK.with(|hook| hook.borrow().clone()),
            #[cfg(test)]
            refusals: Vec::new(),
            #[cfg(test)]
            alone: TEST_ALONE.with(std::cell::Cell::get),
        }
    }
}

/// One block's outcome for one family, with the time its extract took.
pub(crate) type Extracted = (Outcome, Duration);

/// The timing row of a [`Pool`]'s block reads (worker time beside the
/// extracts), and in tests the count of blocks read.
pub(crate) const PREPASS_READ: &str = "prepass.read";

/// Where a [`Pool`]'s block tasks deposit a block's frames for readers
/// whose families observe it hour by hour on threads of their own (the
/// shared block reader), so those readers do not read it again.
pub(crate) trait Depot: Send + Sync {
    /// The tables to read `block` with for the readers, when one of their
    /// families will take it and they have room for it: the block is then
    /// being read for them until [`Self::deposit`] or [`Self::withdraw`].
    fn offer(&self, block: usize) -> Option<Vec<TableId>>;
    /// The offered block's frames, with every offered table's updates.
    fn deposit(&self, block: usize, frames: Vec<Arc<replay::frame::RecordedFrame>>);
    /// The offered block was not read for the readers.
    fn withdraw(&self, block: usize);
}

/// The bytes of the blocks claimed and not yet taken, past which workers
/// wait: a commit budget for read-ahead memory, never a limit on blocks,
/// hours or families. Blocks count their recorded raw bytes and, in a
/// [`Pool`], the bytes their facts hold. The block the coordinator takes
/// next is always claimed.
const AHEAD_BYTES: u64 = 512 * 1024 * 1024;

struct State {
    /// The next block a worker claims, and the next the coordinator takes.
    next: usize,
    handed: usize,
    end: usize,
    /// Blocks claimed at most this far beyond the one the coordinator takes.
    ahead: usize,
    /// Each block's recorded raw bytes, and those of blocks claimed and not taken.
    sizes: Vec<u64>,
    in_flight: u64,
    budget: u64,
    alive: usize,
    /// Each block extracted and not taken, with the bytes its facts hold.
    done: BTreeMap<usize, (Vec<Extracted>, u64)>,
    peak: usize,
}

struct Shared {
    section_consumer: replay::container::SectionConsumer,
    state: Mutex<State>,
    changed: Condvar,
    stop: AtomicBool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// The next block to extract once it is within reach of the coordinator;
    /// none once every block is claimed or the pre-pass stops.
    fn claim(&self) -> Option<usize> {
        let mut state = self.lock();
        loop {
            if self.stop.load(Ordering::Acquire) || state.next >= state.end {
                return None;
            }
            let size = state.sizes[state.next];
            if state.next < state.handed + state.ahead
                && (state.next == state.handed || state.in_flight + size <= state.budget)
            {
                state.in_flight += size;
                self.section_consumer.reading(state.next);
                state.next += 1;
                state.peak = state.peak.max(state.next - state.handed);
                return Some(state.next - 1);
            }
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    /// Block `position` extracted, its facts holding `bytes`: they count
    /// against the budget until the coordinator takes them.
    fn finish(&self, position: usize, outcomes: Vec<Extracted>, bytes: u64) {
        let mut state = self.lock();
        state.in_flight = state.in_flight.saturating_add(bytes);
        state.done.insert(position, (outcomes, bytes));
        drop(state);
        self.changed.notify_all();
    }
}

/// A worker's presence: the coordinator stops waiting for a block once every
/// worker has gone.
struct Alive(Arc<Shared>);

impl Drop for Alive {
    fn drop(&mut self) {
        self.0.lock().alive -= 1;
        self.0.changed.notify_all();
    }
}

/// Upcoming blocks extracted on workers below normal priority. At most
/// `workers` blocks are extracted at once and at most one more waits
/// finished, and blocks claimed and not taken stay within [`AHEAD_BYTES`] of
/// recorded raw bytes, so memory holds the selected tables of that many
/// blocks and their facts. Dropping it stops the workers within an hour of
/// work and joins them.
pub(crate) struct PrePass {
    shared: Arc<Shared>,
    families: Vec<&'static Family>,
    workers: Vec<JoinHandle<()>>,
}

impl PrePass {
    pub(crate) fn start(
        replay: Arc<Replay>,
        from: usize,
        families: Vec<&'static Family>,
        workers: usize,
        knobs: Knobs,
    ) -> Self {
        let section_consumer = replay.section_consumer(from);
        let count = workers.max(1);
        let shared = Arc::new(Shared {
            section_consumer,
            state: Mutex::new(State {
                next: from,
                handed: from,
                end: replay.index.len(),
                ahead: count + 1,
                sizes: replay.index.iter().map(|block| block.raw_length).collect(),
                in_flight: 0,
                budget: knobs.budget.unwrap_or(AHEAD_BYTES),
                alive: 0,
                done: BTreeMap::new(),
                peak: 0,
            }),
            changed: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let mut workers = Vec::with_capacity(count);
        for worker in 0..count {
            shared.lock().alive += 1;
            let alive = Alive(Arc::clone(&shared));
            let replay = Arc::clone(&replay);
            let families = families.clone();
            let knobs = knobs.clone();
            let priority = JobPriority::current();
            // A worker that cannot start leaves its blocks to the others or,
            // with none, to hourly observation.
            if let Ok(handle) = std::thread::Builder::new()
                .name(format!("preparation block pre-pass {worker}"))
                .spawn(move || {
                    // Pre-pass work yields to the coordinators and the hourly walk.
                    priority.enter(Level::BelowNormal);
                    work(&replay, alive, &families, &knobs)
                })
            {
                workers.push(handle);
            }
        }
        Self {
            shared,
            families,
            workers,
        }
    }

    /// The pre-pass's families, in the order of every block's outcomes.
    pub(crate) fn families(&self) -> &[&'static Family] {
        &self.families
    }

    /// Block `position`'s outcomes, one per family, waiting for its extract.
    /// Blocks are taken in order.
    pub(crate) fn take(
        &self,
        position: usize,
        cancelled: &AtomicBool,
    ) -> io::Result<Vec<Extracted>> {
        self.shared.section_consumer.at(position);
        let mut state = self.shared.lock();
        if position != state.handed {
            return Err(io::Error::other("Pre-pass blocks taken out of order"));
        }
        loop {
            crate::metadata::check_cancel(cancelled)?;
            let outcomes = match state.done.remove(&position) {
                Some(outcomes) => Some(outcomes),
                None if state.alive == 0 => Some((
                    sequential(&self.families)
                        .into_iter()
                        .map(|outcome| (outcome, Duration::ZERO))
                        .collect(),
                    0,
                )),
                None => None,
            };
            if let Some((outcomes, bytes)) = outcomes {
                let held = if position < state.next {
                    state.sizes[position]
                } else {
                    0
                };
                state.in_flight = state.in_flight.saturating_sub(held.saturating_add(bytes));
                state.handed += 1;
                drop(state);
                self.shared.changed.notify_all();
                return Ok(outcomes);
            }
            // Workers do not see cancellation; look again shortly.
            state = self
                .shared
                .changed
                .wait_timeout(state, Duration::from_millis(25))
                .map_or_else(|error| error.into_inner().0, |(state, _)| state);
        }
    }

    /// The most blocks claimed and not yet taken at any time.
    #[cfg(test)]
    pub(crate) fn peak(&self) -> usize {
        self.shared.lock().peak
    }
}

impl Drop for PrePass {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        {
            // Under the lock, so no worker checks the flag and then waits unwoken.
            let _state = self.shared.lock();
            self.shared.changed.notify_all();
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn work(replay: &Replay, alive: Alive, families: &[&'static Family], knobs: &Knobs) {
    let shared = &alive.0;
    while let Some(position) = shared.claim() {
        let index = &replay.index[position];
        let _cpu = ::replay::cpu::scope("extract", Some("walk pre-pass"));
        let outcomes = catch_unwind(AssertUnwindSafe(|| {
            extract_block_measured(replay, index, families, &shared.stop, knobs).outcomes
        }))
        .unwrap_or_else(|_| {
            sequential(families)
                .into_iter()
                .map(|outcome| (outcome, Duration::ZERO, 0))
                .collect()
        });
        let bytes = outcomes.iter().map(|(_, _, bytes)| bytes).sum();
        let outcomes = outcomes
            .into_iter()
            .map(|(outcome, took, _)| (outcome, took))
            .collect();
        shared.finish(position, outcomes, bytes);
    }
}

fn sequential(families: &[&'static Family]) -> Vec<Outcome> {
    families.iter().map(|_| Outcome::Sequential).collect()
}

/// One family's blocks in a [`Pool`]: claimed in block order from its first
/// block, and taken in that order by the family alone.
struct Lane {
    section_consumer: Option<replay::container::SectionConsumer>,
    family: &'static Family,
    /// The reads its run chose from the bound content, if it chose any.
    bound: Option<Arc<Bound>>,
    /// Waiting for its family to choose them ([`Pool::bind`]): no block is
    /// claimed until it has.
    binding: bool,
    /// The next block a worker claims, the next the family takes, and the
    /// end of its claims (moved to `next` when the family retires).
    next: usize,
    handed: usize,
    end: usize,
    /// Each block extracted and not taken, with the bytes its facts hold.
    done: BTreeMap<usize, (Extracted, u64)>,
    retired: bool,
    /// Extract time of the blocks extracted so far, and their count.
    spent: Duration,
    measured: u32,
    /// The bytes the facts of the blocks extracted so far hold.
    facts: u64,
    /// The bytes each block claimed and not yet extracted reserves for its
    /// facts.
    reserved: BTreeMap<usize, u64>,
}

impl Lane {
    /// The mean extract time of its blocks so far; None before the first.
    fn mean(&self) -> Option<Duration> {
        self.spent.checked_div(self.measured)
    }

    /// The bytes a block's facts are expected to hold: the mean of those
    /// extracted so far.
    fn expected(&self) -> u64 {
        self.facts
            .checked_div(u64::from(self.measured))
            .unwrap_or_default()
    }
}

/// One block read once for the lanes claiming it together.
struct Task {
    block: usize,
    /// Each member's lane, family and the reads its run chose.
    members: Vec<(usize, &'static Family, Option<Arc<Bound>>)>,
}

struct PoolState {
    lanes: Vec<Lane>,
    /// Each lane's blocks are claimed at most this far beyond the one it
    /// takes next.
    ahead: usize,
    /// Each block's recorded raw bytes, and what the blocks claimed and not
    /// taken hold: each block's raw bytes once while any lane's claim of it
    /// is outstanding, and once extracted the bytes each lane's facts hold.
    sizes: Vec<u64>,
    in_flight: u64,
    budget: u64,
    alive: usize,
    /// The lanes' claims of each block not yet taken or released.
    holders: BTreeMap<usize, usize>,
    /// Workers running a block task.
    busy: usize,
    /// Worker time of the blocks' reads beside their extracts, and the
    /// blocks read.
    read: Duration,
    reads: u64,
}

impl PoolState {
    /// The raw bytes claiming `block` adds: none while it is held.
    fn raw(&self, block: usize) -> u64 {
        if self.holders.contains_key(&block) {
            0
        } else {
            self.sizes[block]
        }
    }

    /// Whether `lane`'s next block is within reach: always the one it takes
    /// next, others while what is claimed and not taken, the block's raw
    /// bytes and the family's expected facts stay within the budget.
    fn reaches(&self, lane: &Lane) -> bool {
        !lane.binding
            && lane.next < lane.end
            && lane.next < lane.handed + self.ahead
            && (lane.next == lane.handed
                || self.in_flight + self.raw(lane.next) + lane.expected() <= self.budget)
    }

    /// How many tasks to split a block into: one, unless fewer blocks are
    /// coming within the lanes' reach than workers wait, so that a few new
    /// blocks (a session's newest hours) do not run every family on one
    /// worker while the others idle.
    fn parts(&self) -> usize {
        let idle = self.alive.saturating_sub(self.busy).max(1);
        let mut upcoming = std::collections::BTreeSet::new();
        for lane in self.lanes.iter().filter(|lane| !lane.retired) {
            upcoming.extend(lane.next..lane.end.min(lane.handed + self.ahead));
        }
        match upcoming.len() {
            0 => 1,
            blocks if blocks >= idle => 1,
            blocks => idle.div_ceil(blocks),
        }
    }

    /// The next task: the lowest block some lane can claim now, for every
    /// lane whose next block it is and that it reaches, costliest first. A
    /// block split into parts ([`Self::parts`]) takes the lanes of about its
    /// share of their mean extract time, at least one.
    fn pick(&self) -> Option<(usize, Vec<usize>)> {
        let block = self
            .lanes
            .iter()
            .filter(|lane| self.reaches(lane))
            .map(|lane| lane.next)
            .min()?;
        let known: Vec<Duration> = self.lanes.iter().filter_map(Lane::mean).collect();
        let unknown = u32::try_from(known.len())
            .ok()
            .and_then(|count| known.iter().sum::<Duration>().checked_div(count))
            .unwrap_or(Duration::from_millis(1))
            .max(Duration::from_micros(1));
        let cost = |lane: &Lane| lane.mean().unwrap_or(unknown).max(Duration::from_micros(1));
        let mut wanting: Vec<usize> = (0..self.lanes.len())
            .filter(|&position| {
                let lane = &self.lanes[position];
                !lane.binding
                    && lane.next == block
                    && lane.next < lane.end
                    && lane.next < lane.handed + self.ahead
            })
            .collect();
        wanting.sort_by_key(|&position| std::cmp::Reverse(cost(&self.lanes[position])));
        let parts = self.parts();
        let share = wanting
            .iter()
            .map(|&position| cost(&self.lanes[position]))
            .sum::<Duration>()
            / u32::try_from(parts).unwrap_or(u32::MAX);
        let mut members = Vec::new();
        let mut held = self.raw(block);
        let mut taken = Duration::ZERO;
        for position in wanting {
            let lane = &self.lanes[position];
            if lane.next != lane.handed && self.in_flight + held + lane.expected() > self.budget {
                continue;
            }
            if parts > 1 && !members.is_empty() && taken >= share {
                break;
            }
            held += lane.expected();
            taken += cost(lane);
            members.push(position);
        }
        (!members.is_empty()).then_some((block, members))
    }

    /// One lane's claim of `block` ends: its raw bytes stop counting once no
    /// claim of it is outstanding.
    fn release(&mut self, block: usize) {
        if let Some(count) = self.holders.get_mut(&block) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.holders.remove(&block);
                self.in_flight = self.in_flight.saturating_sub(self.sizes[block]);
            }
        }
    }
}

struct PoolShared {
    state: Mutex<PoolState>,
    changed: Condvar,
    stop: AtomicBool,
    /// Where block tasks deposit the blocks the shared reader's families
    /// will take.
    depot: Option<Arc<dyn Depot>>,
}

impl PoolShared {
    fn lock(&self) -> MutexGuard<'_, PoolState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// The next block to read and the lanes it is read for
    /// ([`PoolState::pick`]); others wait for their families or the byte
    /// budget. None once every lane's blocks are claimed or the pool stops.
    /// The order of claims changes when a family's pages are ready, never
    /// what it publishes: each family folds its blocks in block order.
    fn claim(&self) -> Option<Task> {
        let mut state = self.lock();
        loop {
            if self.stop.load(Ordering::Acquire)
                || state.lanes.iter().all(|lane| lane.next >= lane.end)
            {
                return None;
            }
            if let Some((block, members)) = state.pick() {
                state.in_flight += state.raw(block);
                *state.holders.entry(block).or_default() += members.len();
                let mut task = Task {
                    block,
                    members: Vec::with_capacity(members.len()),
                };
                for position in members {
                    let expected = state.lanes[position].expected();
                    state.in_flight += expected;
                    let lane = &mut state.lanes[position];
                    if let Some(consumer) = &lane.section_consumer {
                        consumer.reading(block);
                    }
                    lane.reserved.insert(block, expected);
                    lane.next += 1;
                    task.members
                        .push((position, lane.family, lane.bound.clone()));
                }
                state.busy += 1;
                return Some(task);
            }
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    /// `task`'s block read (`measured`): each member's outcome, its facts
    /// holding their bytes, counts against the budget until the family takes
    /// it.
    fn finish(&self, task: Task, measured: Measured) {
        let mut state = self.lock();
        state.busy = state.busy.saturating_sub(1);
        state.read += measured.read;
        state.reads += measured.reads;
        let block = task.block;
        for ((lane, ..), (outcome, took, bytes)) in task.members.into_iter().zip(measured.outcomes)
        {
            let reserved = state.lanes[lane].reserved.remove(&block).unwrap_or(0);
            state.in_flight = state.in_flight.saturating_sub(reserved);
            if state.lanes[lane].retired {
                state.release(block);
            } else {
                state.in_flight = state.in_flight.saturating_add(bytes);
                let lane = &mut state.lanes[lane];
                lane.spent = lane.spent.saturating_add(took);
                lane.measured = lane.measured.saturating_add(1);
                lane.facts = lane.facts.saturating_add(bytes);
                lane.done.insert(block, ((outcome, took), bytes));
            }
        }
        drop(state);
        self.changed.notify_all();
    }
}

/// A pool worker's presence: a family stops waiting for a block once every
/// worker has gone.
struct PoolAlive(Arc<PoolShared>);

impl Drop for PoolAlive {
    fn drop(&mut self) {
        self.0.lock().alive -= 1;
        self.0.changed.notify_all();
    }
}

/// Several families' upcoming blocks, each family's from its own first
/// block, extracted on one set of workers below normal priority: no more
/// workers however many families there are. A worker reads a block once for
/// every family whose next block it is and that it is within reach of
/// ([`Replay::read_block_watched`]), and runs their extracts one after
/// another on it, each family handed exactly what its own read of the block
/// hands it; a family whose fold lags reads its later blocks in tasks of its
/// own. Each family's blocks claimed and not taken stay within `workers + 1`,
/// and all families' within [`AHEAD_BYTES`] (beyond each family's next block)
/// of recorded raw bytes, each block's once however many families claimed
/// it, and the bytes their facts hold ([`Extract::bytes`]): a block claimed
/// reserves its family's mean facts so far, replaced by its own once
/// extracted. Dropping it stops the workers within an hour of work and joins
/// them.
pub(crate) struct Pool {
    shared: Arc<PoolShared>,
    workers: Vec<JoinHandle<()>>,
}

impl Pool {
    /// `lanes`: each family, the first block it takes and whether it
    /// chooses its run's reads from the bound content, whose blocks wait for
    /// them ([`Self::bind`]). A block task reading a block `depot` takes
    /// reads its tables too and deposits the block's frames there.
    pub(crate) fn start(
        replay: Arc<Replay>,
        lanes: Vec<(&'static Family, usize, bool)>,
        workers: usize,
        knobs: Knobs,
        depot: Option<Arc<dyn Depot>>,
    ) -> Self {
        let count = workers.max(1);
        let end = replay.index.len();
        let shared = Arc::new(PoolShared {
            state: Mutex::new(PoolState {
                lanes: lanes
                    .into_iter()
                    .map(|(family, from, binding)| Lane {
                        section_consumer: (from < end).then(|| replay.section_consumer(from)),
                        family,
                        bound: None,
                        binding,
                        next: from,
                        handed: from,
                        end,
                        done: BTreeMap::new(),
                        retired: false,
                        spent: Duration::ZERO,
                        measured: 0,
                        facts: 0,
                        reserved: BTreeMap::new(),
                    })
                    .collect(),
                ahead: count + 1,
                sizes: replay.index.iter().map(|block| block.raw_length).collect(),
                in_flight: 0,
                budget: knobs.budget.unwrap_or(AHEAD_BYTES),
                alive: 0,
                holders: BTreeMap::new(),
                busy: 0,
                read: Duration::ZERO,
                reads: 0,
            }),
            changed: Condvar::new(),
            stop: AtomicBool::new(false),
            depot,
        });
        let mut workers = Vec::with_capacity(count);
        for worker in 0..count {
            shared.lock().alive += 1;
            let alive = PoolAlive(Arc::clone(&shared));
            let replay = Arc::clone(&replay);
            let knobs = knobs.clone();
            let priority = JobPriority::current();
            // A worker that cannot start leaves its blocks to the others or,
            // with none, to hourly observation.
            if let Ok(handle) = std::thread::Builder::new()
                .name(format!("preparation family pre-pass {worker}"))
                .spawn(move || {
                    priority.enter(Level::BelowNormal);
                    work_pool(&replay, alive, &knobs)
                })
            {
                workers.push(handle);
            }
        }
        Self { shared, workers }
    }

    /// Lane `lane`'s outcome for block `position`, waiting for its extract.
    /// Each lane's blocks are taken in order.
    pub(crate) fn take(
        &self,
        lane: usize,
        position: usize,
        cancelled: &AtomicBool,
    ) -> io::Result<Extracted> {
        let mut state = self.shared.lock();
        if let Some(consumer) = &state.lanes[lane].section_consumer {
            consumer.at(position);
        }
        if position != state.lanes[lane].handed {
            return Err(io::Error::other("Pre-pass blocks taken out of order"));
        }
        loop {
            crate::metadata::check_cancel(cancelled)?;
            let alive = state.alive;
            let (extracted, claimed) = {
                let lane = &mut state.lanes[lane];
                let extracted = match lane.done.remove(&position) {
                    Some(extracted) => Some(extracted),
                    None if alive == 0 => Some(((Outcome::Sequential, Duration::ZERO), 0)),
                    None => None,
                };
                let claimed = position < lane.next;
                if extracted.is_some() {
                    lane.handed += 1;
                }
                (extracted, claimed)
            };
            if let Some((extracted, bytes)) = extracted {
                if claimed {
                    state.release(position);
                }
                state.in_flight = state.in_flight.saturating_sub(bytes);
                drop(state);
                self.shared.changed.notify_all();
                return Ok(extracted);
            }
            // Workers do not see cancellation; look again shortly.
            state = self
                .shared
                .changed
                .wait_timeout(state, Duration::from_millis(25))
                .map_or_else(|error| error.into_inner().0, |(state, _)| state);
        }
    }

    /// Lane `lane`'s reads for this run, chosen from the bound content
    /// (None reads as its family declares): its blocks are claimed from now
    /// on, each read so.
    pub(crate) fn bind(&self, lane: usize, bound: Option<Bound>) {
        let mut state = self.shared.lock();
        let lane = &mut state.lanes[lane];
        lane.bound = bound.map(Arc::new);
        lane.binding = false;
        drop(state);
        self.shared.changed.notify_all();
    }

    /// Lane `lane`'s family takes no more blocks: nothing more of it is
    /// claimed, and what it left extracted is released.
    pub(crate) fn retire(&self, lane: usize) {
        let mut state = self.shared.lock();
        let left: Vec<(usize, u64)> = {
            let lane = &mut state.lanes[lane];
            lane.retired = true;
            lane.section_consumer = None;
            lane.binding = false;
            lane.end = lane.next;
            std::mem::take(&mut lane.done)
                .into_iter()
                .map(|(block, (_, bytes))| (block, bytes))
                .collect()
        };
        for (block, bytes) in left {
            state.release(block);
            state.in_flight = state.in_flight.saturating_sub(bytes);
        }
        drop(state);
        self.shared.changed.notify_all();
    }

    /// The worker time the blocks' reads took beside their extracts, and the
    /// blocks read.
    pub(crate) fn read_time(&self) -> (Duration, u64) {
        let state = self.shared.lock();
        (state.read, state.reads)
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        {
            // Under the lock, so no worker checks the flag and then waits unwoken.
            let _state = self.shared.lock();
            self.shared.changed.notify_all();
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn work_pool(replay: &Replay, alive: PoolAlive, knobs: &Knobs) {
    let shared = &alive.0;
    while let Some(task) = shared.claim() {
        let index = &replay.index[task.block];
        let readings: Vec<Reading<'_>> = task
            .members
            .iter()
            .map(|(_, family, bound)| Reading {
                family,
                bound: bound.as_deref(),
            })
            .collect();
        let cpu = ::replay::cpu::scope("extract", Some("pre-pass block"));
        let cpu_started = ::replay::cpu::thread_cycles();
        // The shared reader's tables too, when its families will take the
        // block: withdrawn unless the block's frames are deposited.
        let mut offer = shared.depot.as_deref().and_then(|depot| {
            depot.offer(task.block).map(|tables| Offer {
                depot,
                block: task.block,
                tables,
                deposited: false,
            })
        });
        let measured = catch_unwind(AssertUnwindSafe(|| {
            let kept = offer
                .as_mut()
                .map(|offer| std::mem::take(&mut offer.tables));
            let mut deposit = |frames: &[Arc<replay::frame::RecordedFrame>]| {
                if let Some(offer) = &mut offer {
                    offer.depot.deposit(offer.block, frames.to_vec());
                    offer.deposited = true;
                }
            };
            let keep = kept.as_deref().map(|tables| Keep {
                tables,
                frames: &mut deposit,
            });
            extract_bound(replay, index, &readings, &shared.stop, knobs, keep)
        }))
        .unwrap_or_else(|_| Measured::sequential(readings.len()));
        for (reading, (outcome, _, _)) in readings.iter().zip(&measured.outcomes) {
            // Each family's outcome counted; the block task's cycles below.
            ::replay::cpu::note(
                if matches!(outcome, Outcome::Sequential) {
                    "extract-unused"
                } else {
                    "extract-facts"
                },
                reading.family.name,
                ::replay::cpu::thread_cycles(),
            );
        }
        ::replay::cpu::note("extract-block", "pre-pass block", cpu_started);
        drop(cpu);
        drop(offer);
        shared.finish(task, measured);
    }
}

/// A block offered to the shared reader's depot: withdrawn when dropped
/// without its frames deposited.
struct Offer<'d> {
    depot: &'d dyn Depot,
    block: usize,
    tables: Vec<TableId>,
    deposited: bool,
}

impl Drop for Offer<'_> {
    fn drop(&mut self) {
        if !self.deposited {
            self.depot.withdraw(self.block);
        }
    }
}

/// One family's fields when it declares them: its blocks are then read only
/// at the hours those fields change. None reads every hour of its tables.
fn changes_fields(family: &Reading<'_>, knobs: &Knobs) -> Option<Vec<TableFields>> {
    #[cfg(test)]
    if knobs.alone {
        return None;
    }
    #[cfg(not(test))]
    let _ = knobs;
    family.fields()
}

/// A family as one block's read runs it: its declaration, and the reads its
/// run chose from the bound content, if it chose any.
#[derive(Clone, Copy)]
struct Reading<'a> {
    family: &'static Family,
    bound: Option<&'a Bound>,
}

impl Reading<'_> {
    fn of(family: &'static Family) -> Self {
        Self {
            family,
            bound: None,
        }
    }

    /// The tables its extract reads.
    fn tables(&self) -> &[TableId] {
        match self.bound {
            Some(bound) => &bound.tables,
            None => self.family.tables,
        }
    }

    /// The fields its extract reads, when it declares them and they can be
    /// made; None reads its blocks whole.
    fn fields(&self) -> Option<Vec<TableFields>> {
        match self.bound {
            Some(bound) => bound.fields.clone(),
            None => (self.family.fields?)().ok(),
        }
    }

    /// A fresh extract for one block.
    fn start(&self, format: FormatVersion) -> Box<dyn Extract> {
        match self.bound {
            Some(bound) => (bound.start)(format),
            None => (self.family.start)(format),
        }
    }
}

/// What an extract is given: an hour's frame or a quiet span.
#[cfg_attr(not(test), allow(dead_code))]
enum At<'a> {
    Hour(&'a DecodedFrame),
    Quiet(&'a QuietHours),
}

#[cfg(test)]
impl At<'_> {
    fn holds(&self, hour: i32) -> bool {
        match self {
            Self::Hour(frame) => frame.hour == hour,
            Self::Quiet(quiet) => quiet.hours().contains(&hour),
        }
    }
}

/// What the pre-pass follows of one extract in a block read by its changes.
#[derive(Default)]
struct Follow {
    /// Whether the read watches exactly its family's fields, so the block
    /// reader's changed tables are its own ([`Extract::observe_reported`]).
    reports: bool,
    /// The tables its family watches; those it watches whole; and those
    /// reconstructed for it (Campaign too).
    watched: Vec<TableId>,
    whole: Vec<TableId>,
    reconstructed: Vec<TableId>,
    /// The last frame it observed, and the tables whose allocation there was
    /// built for it at an hour not read and is not rebuilt since.
    last: Option<DecodedFrame>,
    built: Vec<TableId>,
}

#[cfg(test)]
thread_local! {
    static RELEASING: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

impl Follow {
    fn new(fields: &[TableFields]) -> Self {
        let mut follow = Self::default();
        for watch in fields {
            if !follow.watched.contains(&watch.table) {
                follow.watched.push(watch.table);
            }
            if watch.fields == TableFields::all(watch.table).fields
                && !follow.whole.contains(&watch.table)
            {
                follow.whole.push(watch.table);
            }
        }
        follow.reconstructed = follow.watched.clone();
        if !follow.reconstructed.contains(&TableId::Campaign) {
            follow.reconstructed.push(TableId::Campaign);
        }
        follow
    }

    /// The preceding quiet span was delivered, and the next observed frame
    /// replaces these tables. Only built allocations carried unchanged into
    /// that frame still need the preceding views.
    fn release(&mut self, replaced: &[TableId]) {
        static RELEASED: LazyLock<SharedView> = LazyLock::new(|| {
            TableView::Unavailable {
                reason: "View released before replacement".into(),
            }
            .into()
        });
        if let Some(last) = &mut self.last {
            for table in replaced {
                last.tables[table.index()] = RELEASED.clone();
            }
        }
    }

    /// `frame`, read at an hour that rebuilt `rebuilt`, with each table built
    /// for the extract and not rebuilt since at that built allocation: the
    /// hour before's, which a frame of the block read alone carries there.
    fn carried(&self, frame: &DecodedFrame, rebuilt: &[TableId]) -> Option<DecodedFrame> {
        let last = self.last.as_ref()?;
        let mut carried: Option<DecodedFrame> = None;
        for &table in &self.built {
            let index = table.index();
            if !rebuilt.contains(&table)
                && !SharedView::ptr_eq(&last.tables[index], &frame.tables[index])
            {
                carried.get_or_insert_with(|| frame.clone()).tables[index] = last.tables[index]
                    .clone()
                    .with_sampled_at(frame.tables[index].sampled_at());
            }
        }
        carried
    }

    /// Whether the extract observes the `position`th of `hours`, after the
    /// recorded hour `previous`: one that follows a gap, reaches its
    /// `deadline`, or decodes again a table its family watches whole or one
    /// that is incomplete, each of which its change facts or scan tell apart
    /// from an hour where nothing changed.
    fn observes(
        &self,
        hours: &QuietHours,
        position: usize,
        previous: Option<i32>,
        deadline: Option<i32>,
    ) -> bool {
        let hour = hours.hours()[position];
        let complete = |table: TableId| {
            self.last
                .as_ref()
                .is_some_and(|last| matches!(&*last.tables[table.index()], TableView::Complete(_)))
        };
        previous.and_then(|previous| previous.checked_add(1)) != Some(hour)
            || deadline.is_some_and(|deadline| deadline <= hour)
            || self
                .whole
                .iter()
                .any(|&table| hours.rebuilt(position, table))
            || self
                .watched
                .iter()
                .any(|&table| hours.rebuilt(position, table) && !complete(table))
    }

    /// The frame at the `position`th of `hours` as the extract would read it
    /// there: the last frame's rows with the hour's own metrics and sample
    /// hours, and a new allocation of each table its update rebuilt.
    fn build(&self, hours: &QuietHours, position: usize) -> io::Result<DecodedFrame> {
        let mut frame = self
            .last
            .clone()
            .ok_or_else(|| io::Error::other("A quiet hour came before the block's first"))?;
        frame.hour = hours.hours()[position];
        frame.metrics = hours.metrics()[position].clone();
        for &table in &self.reconstructed {
            let index = table.index();
            let view = if hours.rebuilt(position, table) {
                SharedView::from((*frame.tables[index]).clone())
            } else {
                frame.tables[index].clone()
            };
            frame.tables[index] = view.with_sampled_at(hours.sampled_at(position, table));
        }
        Ok(frame)
    }

    /// Give the extract a quiet span after the recorded hour `previous`: the
    /// runs of hours it need not observe as quiet, each hour it must observe
    /// on a frame built for it.
    fn quiet(
        &mut self,
        extract: &mut Box<dyn Extract>,
        replay: &Replay,
        hours: &QuietHours,
        mut previous: Option<i32>,
    ) -> io::Result<()> {
        let all = hours.hours();
        let mut from = 0;
        while from < all.len() {
            let deadline = extract.deadline();
            let mut position = from;
            while position < all.len() && !self.observes(hours, position, previous, deadline) {
                previous = Some(all[position]);
                position += 1;
            }
            if position > from {
                extract.quiet(replay, hours, from..position)?;
                let span = from..position;
                self.built
                    .retain(|&table| !span.clone().any(|at| hours.rebuilt(at, table)));
            }
            if position == all.len() {
                break;
            }
            let continues =
                previous.and_then(|previous| previous.checked_add(1)) == Some(all[position]);
            let frame = self.build(hours, position)?;
            let changes = FrameChanges::rebuilt(&frame, replay.format, continues, |table| {
                hours.rebuilt(position, table)
            });
            // No watched field changed at a quiet hour.
            if self.reports {
                extract.observe_reported(replay, &frame, &changes, &[])?;
            } else {
                extract.observe(replay, &frame, &changes)?;
            }
            for &table in &self.reconstructed {
                if hours.rebuilt(position, table) && !self.built.contains(&table) {
                    self.built.push(table);
                }
            }
            self.last = Some(frame);
            previous = Some(all[position]);
            from = position + 1;
        }
        Ok(())
    }
}

/// One family's extract of a block while it goes on, the worker time it
/// took, and what the pre-pass follows of it.
struct Running {
    extract: Option<Box<dyn Extract>>,
    took: Duration,
    follow: Follow,
}

/// Give one family's extract, while it goes on, one hour or quiet span
/// (`run`); one that refuses, fails or panics stops, and its family observes
/// the block hour by hour. An error once it stopped, which stops its watch.
fn visit(
    family: &'static Family,
    running: &mut Running,
    knobs: &Knobs,
    at: At<'_>,
    run: impl FnOnce(&mut Box<dyn Extract>, &mut Follow) -> io::Result<()>,
) -> io::Result<()> {
    #[cfg(not(test))]
    let _ = (knobs, &at);
    let Running {
        extract,
        took,
        follow,
    } = running;
    let Some(current) = extract else {
        return Err(io::Error::other("The block extract stopped"));
    };
    let started = Instant::now();
    let cpu = ::replay::cpu::scope("extract", Some(family.name));
    let observed = catch_unwind(AssertUnwindSafe(|| {
        #[cfg(test)]
        if let (Some(hook), At::Hour(frame)) = (&knobs.hook, &at) {
            match hook(family.name, frame) {
                Fault::Continue => {}
                Fault::Error => return Err(io::Error::other("Injected extract error")),
                Fault::Panic => panic!("Injected extract panic"),
            }
        }
        run(current, follow)
    }))
    .unwrap_or_else(|_| Err(io::Error::other("Block extract panicked")));
    *took += started.elapsed();
    drop(cpu);
    #[cfg(test)]
    let observed = observed.and_then(|()| {
        if knobs
            .refusals
            .iter()
            .any(|&(name, hour)| name == family.name && at.holds(hour))
        {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Injected refusal",
            ))
        } else {
            Ok(())
        }
    });
    if observed.is_err() {
        // Nothing left to extract: the family observes the block hourly.
        *extract = None;
        return Err(io::Error::other("The block extract stopped"));
    }
    Ok(())
}

/// One block's outcome for each family read together, with the time its
/// extract took and the bytes its facts hold ([`Extract::bytes`]; none for a
/// block observed hour by hour); the worker time the block's reads took
/// beside the extracts, and how many reads it took.
pub(crate) struct Measured {
    pub outcomes: Vec<(Outcome, Duration, u64)>,
    pub read: Duration,
    pub reads: u64,
}

impl Measured {
    /// Each of `families` families observing the block hour by hour.
    fn sequential(families: usize) -> Self {
        Self {
            outcomes: (0..families)
                .map(|_| (Outcome::Sequential, Duration::ZERO, 0))
                .collect(),
            read: Duration::ZERO,
            reads: 0,
        }
    }
}

/// [`extract_bound`] of families as they declare their reads.
fn extract_block_measured(
    replay: &Replay,
    index: &BlockIndex,
    families: &[&'static Family],
    stop: &AtomicBool,
    knobs: &Knobs,
) -> Measured {
    let readings: Vec<Reading<'_>> = families.iter().copied().map(Reading::of).collect();
    extract_bound(replay, index, &readings, stop, knobs, None)
}

/// Read one block once for `families` ([`Replay::read_block_watched`]), each
/// with the tables, fields and extract its run chose from the bound content
/// ([`Bound`]) or as it declares them, and
/// run each family's extract on what its own read of the block alone hands
/// it, with change facts of this block alone: preparation starts every block
/// with no change facts of the block before, so these are the facts its
/// observers get. A family that declares its fields watches the hours they
/// change: its extract observes the hours [`Family::fields`] names with the
/// change facts a tracker of the block read alone finds there
/// ([`FrameChanges::rebuilt`]), and is told of the quiet spans between. Any
/// other family watches every hour of its tables, with a tracker of its own.
/// A family whose extract refuses or fails, or whose own read refuses the
/// block, is observed hour by hour instead. When the one read fails, each
/// family reads the block alone, as its own read would; when that fails too,
/// or the read is cancelled, every family is observed hour by hour.
fn extract_bound(
    replay: &Replay,
    index: &BlockIndex,
    families: &[Reading<'_>],
    stop: &AtomicBool,
    knobs: &Knobs,
    keep: Option<Keep<'_>>,
) -> Measured {
    let started = Instant::now();
    let format = replay.format;
    let fields: Vec<Option<Vec<TableFields>>> = families
        .iter()
        .map(|family| changes_fields(family, knobs))
        .collect();
    let tables: Vec<Vec<TableId>> = families
        .iter()
        .map(|family| {
            let mut tables = family.tables().to_vec();
            tables.sort_unstable_by_key(|table| table.index());
            tables.dedup();
            tables
        })
        .collect();
    let mut extracts: Vec<Running> = families
        .iter()
        .zip(&fields)
        .map(|(family, fields)| {
            let started = ::replay::cpu::thread_cycles();
            let mut extract = family.start(format);
            ::replay::cpu::note("extract-start", family.family.name, started);
            let follow = match fields {
                Some(fields) => {
                    extract.read_by_changes();
                    // Its own watch: the reader's changed tables are its own.
                    let mut follow = Follow::new(fields);
                    follow.reports = true;
                    follow
                }
                None => Follow::default(),
            };
            Running {
                extract: Some(extract),
                took: Duration::ZERO,
                follow,
            }
        })
        .collect();
    let check = || {
        if stop.load(Ordering::Acquire) {
            Err(io::Error::from(io::ErrorKind::Interrupted))
        } else {
            Ok(())
        }
    };
    let read = {
        let mut watches: Vec<BlockWatch<'_>> = Vec::with_capacity(families.len());
        for (((reading, running), fields), tables) in families
            .iter()
            .zip(extracts.iter_mut())
            .zip(&fields)
            .zip(&tables)
        {
            let family = reading.family;
            watches.push(match fields {
                Some(fields) => {
                    // The last recorded hour read, visited or quiet.
                    let mut previous: Option<i32> = None;
                    let watch = BlockWatch::new(Watching::Changes(fields), move |step| {
                        if let Watched::Replacing(tables) = step {
                            running.follow.release(tables);
                            return Ok(());
                        }
                        let Watched::Change(change) = step else {
                            unreachable!("a watch of changes")
                        };
                        match change {
                            BlockChange::Changed {
                                frame,
                                tables,
                                rebuilt,
                            } => {
                                let continues = previous.and_then(|hour| hour.checked_add(1))
                                    == Some(frame.hour);
                                previous = Some(frame.hour);
                                let mut rows_rebuilt = [false; replay::frame::TABLE_COUNT];
                                for table in rebuilt {
                                    rows_rebuilt[table.index()] = true;
                                }
                                let changes =
                                    FrameChanges::rebuilt(frame, format, continues, |table| {
                                        rows_rebuilt[table.index()]
                                    });
                                visit(
                                    family,
                                    running,
                                    knobs,
                                    At::Hour(frame),
                                    |extract, follow| {
                                        let carried = follow.carried(frame, rebuilt);
                                        let frame = carried.unwrap_or_else(|| frame.clone());
                                        extract
                                            .observe_reported(replay, &frame, &changes, tables)?;
                                        follow.last = Some(frame);
                                        // A table this hour rebuilt is at the
                                        // allocation a frame of the block read
                                        // alone holds; every other built one is
                                        // carried until its table rebuilds.
                                        follow.built.retain(|table| !rebuilt.contains(table));
                                        Ok(())
                                    },
                                )
                            }
                            BlockChange::Quiet(quiet) => {
                                let before = previous;
                                previous = Some(quiet.last());
                                visit(
                                    family,
                                    running,
                                    knobs,
                                    At::Quiet(quiet),
                                    |extract, follow| follow.quiet(extract, replay, quiet, before),
                                )
                            }
                        }
                    });
                    #[cfg(test)]
                    let releasing = RELEASING.with(std::cell::Cell::get);
                    #[cfg(not(test))]
                    let releasing = true;
                    if releasing { watch.releasing() } else { watch }
                }
                None => {
                    let mut tracker = FrameChangeTracker::new();
                    BlockWatch::new(Watching::Every(tables), move |step| {
                        let Watched::Hour { updates, frame } = step else {
                            unreachable!("a watch of every hour")
                        };
                        let changes = tracker.observe_updates(frame, Arc::clone(updates), format);
                        visit(family, running, knobs, At::Hour(frame), |extract, _| {
                            extract.observe(replay, frame, &changes)
                        })
                    })
                }
            });
        }
        let read = replay.read_block_watched_keeping(index, &mut watches, keep, check);
        let own: Vec<bool> = watches
            .into_iter()
            .map(|watch| watch.finish().is_ok())
            .collect();
        read.map(|()| own)
    };
    let spent = extracts
        .iter()
        .map(|running| running.took)
        .sum::<Duration>();
    let read_time = started.elapsed().saturating_sub(spent);
    let own = match read {
        Ok(own) => own,
        Err(error) if families.len() > 1 && error.kind() != io::ErrorKind::Interrupted => {
            // A table another family reads failed the read: each reads the
            // block alone, as its own read would.
            let mut measured = Measured {
                outcomes: Vec::with_capacity(families.len()),
                read: read_time,
                reads: 1,
            };
            for family in families {
                let alone = extract_bound(replay, index, &[*family], stop, knobs, None);
                measured.outcomes.extend(alone.outcomes);
                measured.read += alone.read;
                measured.reads += alone.reads;
            }
            return measured;
        }
        Err(_) => {
            return Measured {
                read: read_time,
                reads: 1,
                ..Measured::sequential(families.len())
            };
        }
    };
    Measured {
        outcomes: dashboard_base::prepared::prepass::share_view_sizes(|| {
            extracts
                .into_iter()
                .zip(own)
                .zip(families)
                .map(|((running, own), family)| {
                    // A family whose own read refused the block observes it
                    // hour by hour, as it would have.
                    let extract = running.extract.filter(|_| own);
                    let started = ::replay::cpu::thread_cycles();
                    let bytes = extract.as_ref().map_or(0, |extract| extract.bytes());
                    ::replay::cpu::note("extract-size", family.family.name, started);
                    let started = ::replay::cpu::thread_cycles();
                    let outcome = extract
                        .and_then(|extract| extract.finish().ok())
                        .map_or(Outcome::Sequential, Outcome::Facts);
                    ::replay::cpu::note("extract-finish", family.family.name, started);
                    let bytes = if matches!(outcome, Outcome::Facts(_)) {
                        bytes
                    } else {
                        0
                    };
                    (outcome, running.took, bytes)
                })
                .collect()
        }),
        read: read_time,
        reads: 1,
    }
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum Fault {
    Continue,
    Error,
    Panic,
}
#[cfg(test)]
pub(crate) type Hook = Arc<dyn Fn(&str, &DecodedFrame) -> Fault + Send + Sync>;
#[cfg(test)]
thread_local! {
    static TEST_SEQUENTIAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static TEST_WORKERS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    static TEST_HOOK: std::cell::RefCell<Option<Hook>> = const { std::cell::RefCell::new(None) };
    static TEST_ALONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

// Fold counts are dashboard-base's (`dashboard_base::prepared::prepass::note`),
// which the families below the dashboard note too.
pub(crate) use dashboard_base::prepared::prepass::note;
#[cfg(test)]
pub(crate) use dashboard_base::prepared::prepass::{add_notes, noted, notes};

/// Run `run` with migrated families observed hour by hour, as before.
#[cfg(test)]
pub(crate) fn with_test_sequential<T>(run: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_SEQUENTIAL.with(|value| value.set(self.0));
        }
    }
    let _reset = Reset(TEST_SEQUENTIAL.with(|value| value.replace(true)));
    run()
}

/// Run `run` with every block read whole, also for families that declare
/// the fields they read.
#[cfg(test)]
pub(crate) fn with_test_alone<T>(run: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_ALONE.with(|value| value.set(self.0));
        }
    }
    let _reset = Reset(TEST_ALONE.with(|value| value.replace(true)));
    run()
}

/// Run `run` with `workers` pre-pass workers and an extract fault hook.
#[cfg(test)]
pub(crate) fn with_test_prepass<T>(
    workers: usize,
    hook: Option<Hook>,
    run: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<usize>, Option<Hook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_WORKERS.with(|value| value.set(self.0));
            TEST_HOOK.with(|value| *value.borrow_mut() = self.1.take());
        }
    }
    let _reset = Reset(
        TEST_WORKERS.with(|value| value.replace(Some(workers))),
        TEST_HOOK.with(|value| value.replace(hook)),
    );
    run()
}

#[cfg(test)]
mod tests;
