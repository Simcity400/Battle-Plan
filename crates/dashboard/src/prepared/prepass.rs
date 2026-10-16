//! Block pre-pass: workers reconstruct upcoming blocks alone, each from its
//! keyframe ([`Replay::read_block_alone`]), and run every migrated family's
//! extract on them. The coordinator folds each family's block facts in block
//! order instead of observing the family every hour.
//!
//! A family that declares the fields it reads ([`Family::fields`]) has its
//! blocks read only at the hours those fields change
//! ([`Replay::read_block_changes`]): its extract observes those hours' frames
//! and is told of the quiet spans between them ([`Extract::quiet`]).
//!
//! [`PrePass`] serves the hourly walk: every block once, for all its families
//! together. [`Pool`] serves families prepared on their own: one set of
//! workers extracts each family's blocks from that family's own first block,
//! and each family takes its blocks in order on its own.

use super::frame_changes::{FrameChangeTracker, FrameChanges};
pub(crate) use dashboard_base::prepared::prepass::{Extract, Family, Folded};
use replay::{
    container::{BlockChange, BlockIndex, QuietHours, Replay, TableFields},
    frame::{DecodedFrame, SharedView, TableView},
    rows::TableId,
};
use std::{
    any::Any,
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
        let count = workers.max(1);
        let shared = Arc::new(Shared {
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
            // A worker that cannot start leaves its blocks to the others or,
            // with none, to hourly observation.
            if let Ok(handle) = std::thread::Builder::new()
                .name(format!("preparation block pre-pass {worker}"))
                .spawn(move || work(&replay, alive, &families, &knobs))
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

/// Pre-pass work yields to the coordinators and the hourly walk.
pub(crate) fn below_normal_priority() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
        };
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

fn work(replay: &Replay, alive: Alive, families: &[&'static Family], knobs: &Knobs) {
    below_normal_priority();
    let shared = &alive.0;
    while let Some(position) = shared.claim() {
        let index = &replay.index[position];
        let outcomes = catch_unwind(AssertUnwindSafe(|| {
            extract_block_measured(replay, index, families, &shared.stop, knobs)
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
    family: &'static Family,
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
    /// The extract time of the blocks not yet claimed, at the mean of those
    /// extracted so far; none measured yet counts as none left, so every
    /// family's cost is learnt from its first block.
    fn remaining(&self) -> Duration {
        let left = u32::try_from(self.end.saturating_sub(self.next)).unwrap_or(u32::MAX);
        self.spent
            .checked_div(self.measured)
            .unwrap_or_default()
            .saturating_mul(left)
    }

    /// The bytes a block's facts are expected to hold: the mean of those
    /// extracted so far.
    fn expected(&self) -> u64 {
        self.facts
            .checked_div(u64::from(self.measured))
            .unwrap_or_default()
    }
}

struct PoolState {
    lanes: Vec<Lane>,
    /// Each lane's blocks are claimed at most this far beyond the one it
    /// takes next.
    ahead: usize,
    /// Each block's recorded raw bytes, and what every lane's blocks claimed
    /// and not taken hold: their raw bytes, and once extracted the bytes
    /// their facts hold.
    sizes: Vec<u64>,
    in_flight: u64,
    budget: u64,
    alive: usize,
}

impl PoolState {
    /// The lane whose next block to claim now: of the lanes with a block
    /// within reach, the one with the least extract work left, then the one
    /// least far ahead of its family, then the first. A lane's next block to
    /// take is always within reach; others while what is claimed and not
    /// taken, this block's raw bytes and its family's expected facts stay
    /// within the budget.
    fn pick(&self) -> Option<usize> {
        self.lanes
            .iter()
            .enumerate()
            .filter(|(_, lane)| {
                lane.next < lane.end
                    && lane.next < lane.handed + self.ahead
                    && (lane.next == lane.handed
                        || self.in_flight + self.sizes[lane.next] + lane.expected()
                            <= self.budget)
            })
            .min_by_key(|(_, lane)| (lane.remaining(), lane.next - lane.handed))
            .map(|(position, _)| position)
    }
}

struct PoolShared {
    state: Mutex<PoolState>,
    changed: Condvar,
    stop: AtomicBool,
}

impl PoolShared {
    fn lock(&self) -> MutexGuard<'_, PoolState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// The next lane, block and family to extract: of the lanes with a block
    /// within reach, the one with the least extract work left (so the
    /// cheapest families finish first and most widgets are ready soonest),
    /// then the one least far ahead of its family, then the first. A lane's
    /// next block to take is always within reach; others wait for the byte
    /// budget. None once every lane's blocks are claimed or the pool stops.
    /// The order of claims changes when a family's pages are ready, never
    /// what it publishes: each family folds its blocks in block order.
    fn claim(&self) -> Option<(usize, usize, &'static Family)> {
        let mut state = self.lock();
        loop {
            if self.stop.load(Ordering::Acquire)
                || state.lanes.iter().all(|lane| lane.next >= lane.end)
            {
                return None;
            }
            if let Some(position) = state.pick() {
                let block = state.lanes[position].next;
                let expected = state.lanes[position].expected();
                state.in_flight += state.sizes[block] + expected;
                let lane = &mut state.lanes[position];
                lane.reserved.insert(block, expected);
                lane.next += 1;
                return Some((position, block, lane.family));
            }
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    /// Block `block` of lane `lane` extracted, its facts holding `bytes`:
    /// they count against the budget until the family takes them.
    fn finish(&self, lane: usize, block: usize, extracted: Extracted, bytes: u64) {
        let mut state = self.lock();
        let reserved = state.lanes[lane].reserved.remove(&block).unwrap_or(0);
        state.in_flight = state.in_flight.saturating_sub(reserved);
        if state.lanes[lane].retired {
            state.in_flight = state.in_flight.saturating_sub(state.sizes[block]);
        } else {
            state.in_flight = state.in_flight.saturating_add(bytes);
            let lane = &mut state.lanes[lane];
            lane.spent = lane.spent.saturating_add(extracted.1);
            lane.measured = lane.measured.saturating_add(1);
            lane.facts = lane.facts.saturating_add(bytes);
            lane.done.insert(block, (extracted, bytes));
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
/// block, extracted alone on one set of workers below normal priority: no
/// more workers however many families there are. Workers first serve the
/// family with the least extract work left, by the mean extract time of its
/// blocks so far, so cheap families are ready before costly ones take the
/// workers; among equals, the family least far ahead. Each family's blocks claimed and not
/// taken stay within `workers + 1`, and all families' within [`AHEAD_BYTES`]
/// (beyond each family's next block) of recorded raw bytes and the bytes
/// their facts hold ([`Extract::bytes`]): a block claimed reserves its
/// family's mean facts so far, replaced by its own once extracted. Dropping it
/// stops the workers within an hour of work and joins them.
pub(crate) struct Pool {
    shared: Arc<PoolShared>,
    workers: Vec<JoinHandle<()>>,
}

impl Pool {
    /// `lanes`: each family and the first block it takes.
    pub(crate) fn start(
        replay: Arc<Replay>,
        lanes: Vec<(&'static Family, usize)>,
        workers: usize,
        knobs: Knobs,
    ) -> Self {
        let count = workers.max(1);
        let end = replay.index.len();
        let shared = Arc::new(PoolShared {
            state: Mutex::new(PoolState {
                lanes: lanes
                    .into_iter()
                    .map(|(family, from)| Lane {
                        family,
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
            }),
            changed: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let mut workers = Vec::with_capacity(count);
        for worker in 0..count {
            shared.lock().alive += 1;
            let alive = PoolAlive(Arc::clone(&shared));
            let replay = Arc::clone(&replay);
            let knobs = knobs.clone();
            // A worker that cannot start leaves its blocks to the others or,
            // with none, to hourly observation.
            if let Ok(handle) = std::thread::Builder::new()
                .name(format!("preparation family pre-pass {worker}"))
                .spawn(move || work_pool(&replay, alive, &knobs))
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
        if position != state.lanes[lane].handed {
            return Err(io::Error::other("Pre-pass blocks taken out of order"));
        }
        loop {
            crate::metadata::check_cancel(cancelled)?;
            let size = state.sizes[position];
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
                let held = if claimed { size } else { 0 };
                state.in_flight = state.in_flight.saturating_sub(held.saturating_add(bytes));
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

    /// Lane `lane`'s family takes no more blocks: nothing more of it is
    /// claimed, and what it left extracted is released.
    pub(crate) fn retire(&self, lane: usize) {
        let mut state = self.shared.lock();
        let left: Vec<(usize, u64)> = {
            let lane = &mut state.lanes[lane];
            lane.retired = true;
            lane.end = lane.next;
            std::mem::take(&mut lane.done)
                .into_iter()
                .map(|(block, (_, bytes))| (block, bytes))
                .collect()
        };
        for (block, bytes) in left {
            let held = state.sizes[block].saturating_add(bytes);
            state.in_flight = state.in_flight.saturating_sub(held);
        }
        drop(state);
        self.shared.changed.notify_all();
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
    below_normal_priority();
    let shared = &alive.0;
    while let Some((lane, position, family)) = shared.claim() {
        let index = &replay.index[position];
        let (outcome, took, bytes) = catch_unwind(AssertUnwindSafe(|| {
            extract_block_measured(replay, index, &[family], &shared.stop, knobs)
        }))
        .ok()
        .and_then(|mut outcomes| outcomes.pop())
        .unwrap_or((Outcome::Sequential, Duration::ZERO, 0));
        shared.finish(lane, position, (outcome, took), bytes);
    }
}

/// Each of `families`' fields, when every one declares them: their blocks
/// are then read only at the hours those fields change. None reads the
/// block whole.
fn changes_fields(families: &[&'static Family], knobs: &Knobs) -> Option<Vec<Vec<TableFields>>> {
    #[cfg(test)]
    if knobs.alone {
        return None;
    }
    #[cfg(not(test))]
    let _ = knobs;
    families
        .iter()
        .map(|family| (family.fields?)().ok())
        .collect()
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
            extract.observe(replay, &frame, &changes)?;
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

/// Give every extract still going one hour or quiet span (`run`); one that
/// refuses, fails or panics stops, and its family observes the block hour by
/// hour. An error once none is left, which ends the read.
fn visit(
    families: &[&'static Family],
    extracts: &mut [Running],
    knobs: &Knobs,
    at: At<'_>,
    mut run: impl FnMut(&mut Box<dyn Extract>, &mut Follow) -> io::Result<()>,
) -> io::Result<()> {
    #[cfg(not(test))]
    let _ = (knobs, &at);
    for (family, running) in families.iter().zip(extracts.iter_mut()) {
        #[cfg(not(test))]
        let _ = family;
        let Running {
            extract,
            took,
            follow,
        } = running;
        let Some(current) = extract else {
            continue;
        };
        let started = Instant::now();
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
            *extract = None;
        }
    }
    if extracts.iter().all(|running| running.extract.is_none()) {
        // Nothing left to extract: every family observes hourly.
        return Err(io::Error::other("Every block extract stopped"));
    }
    Ok(())
}

/// Reconstruct one block alone with the tables `families` read and run their
/// extracts on every hour, with change facts from a tracker of this block
/// alone: preparation starts every block with no change facts of the block
/// before, so these are the facts its observers get. When every family
/// declares its fields, the block is read only at the hours they change, and
/// each extract observes the hours [`Family::fields`] names with the change
/// facts that tracker finds there ([`FrameChanges::rebuilt`]), and is told
/// of the quiet spans between. A family whose extract refuses or fails, or
/// every family when the block cannot be read, is observed hour by hour
/// instead. Each family's outcome comes with the bytes its facts hold
/// ([`Extract::bytes`]; none for a block observed hour by hour).
fn extract_block_measured(
    replay: &Replay,
    index: &BlockIndex,
    families: &[&'static Family],
    stop: &AtomicBool,
    knobs: &Knobs,
) -> Vec<(Outcome, Duration, u64)> {
    let format = replay.format;
    let mut extracts: Vec<Running> = families
        .iter()
        .map(|family| Running {
            extract: Some((family.start)(format)),
            took: Duration::ZERO,
            follow: Follow::default(),
        })
        .collect();
    let check = || {
        if stop.load(Ordering::Acquire) {
            Err(io::Error::from(io::ErrorKind::Interrupted))
        } else {
            Ok(())
        }
    };
    let read = match changes_fields(families, knobs) {
        Some(lists) => {
            for (running, fields) in extracts.iter_mut().zip(&lists) {
                running.follow = Follow::new(fields);
                if let Some(extract) = &mut running.extract {
                    extract.read_by_changes();
                }
            }
            let fields: Vec<TableFields> = lists.into_iter().flatten().collect();
            // The last recorded hour read, visited or quiet.
            let mut previous: Option<i32> = None;
            replay.read_block_changes(
                index,
                &fields,
                |change| match change {
                    BlockChange::Changed { frame, rebuilt, .. } => {
                        let continues =
                            previous.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
                        previous = Some(frame.hour);
                        let changes = FrameChanges::rebuilt(frame, format, continues, |table| {
                            rebuilt.contains(&table)
                        });
                        visit(
                            families,
                            &mut extracts,
                            knobs,
                            At::Hour(frame),
                            |extract, follow| {
                                let carried = follow.carried(frame, rebuilt);
                                let frame = carried.unwrap_or_else(|| frame.clone());
                                extract.observe(replay, &frame, &changes)?;
                                follow.last = Some(frame);
                                // A table this hour rebuilt is at the
                                // allocation a frame of the block read alone
                                // holds; every other built one is carried
                                // until its table rebuilds.
                                follow.built.retain(|table| !rebuilt.contains(table));
                                Ok(())
                            },
                        )
                    }
                    BlockChange::Quiet(quiet) => {
                        let before = previous;
                        previous = Some(quiet.last());
                        visit(
                            families,
                            &mut extracts,
                            knobs,
                            At::Quiet(quiet),
                            |extract, follow| follow.quiet(extract, replay, quiet, before),
                        )
                    }
                },
                check,
            )
        }
        None => {
            let mut tables: Vec<TableId> = families
                .iter()
                .flat_map(|family| family.tables.iter().copied())
                .collect();
            tables.sort_unstable_by_key(|table| table.index());
            tables.dedup();
            let mut tracker = FrameChangeTracker::new();
            replay.read_block_alone(
                index,
                &tables,
                |raw, frame| {
                    let changes = tracker.observe_updates(frame, Arc::new(raw), format);
                    visit(
                        families,
                        &mut extracts,
                        knobs,
                        At::Hour(frame),
                        |extract, _| extract.observe(replay, frame, &changes),
                    )
                },
                check,
            )
        }
    };
    if read.is_err() {
        return sequential(families)
            .into_iter()
            .map(|outcome| (outcome, Duration::ZERO, 0))
            .collect();
    }
    extracts
        .into_iter()
        .map(|running| {
            let bytes = running.extract.as_ref().map_or(0, |extract| extract.bytes());
            let outcome = running
                .extract
                .and_then(|extract| extract.finish().ok())
                .map_or(Outcome::Sequential, Outcome::Facts);
            let bytes = if matches!(outcome, Outcome::Facts(_)) {
                bytes
            } else {
                0
            };
            (outcome, running.took, bytes)
        })
        .collect()
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
