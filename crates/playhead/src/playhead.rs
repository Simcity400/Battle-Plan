//! The playhead: every selected table's lane kept at the playback head of a timeline, stepped
//! ahead of the frame on one thread. The stepper owns the lanes and builds steps of owned change
//! sets into a ring; fetchers read and open parts ahead, run seeks and the readers' rebuilds, and
//! free what the stepper retired, so the stepper never touches the file. The frame side asks the
//! clock for the head ([`Playhead::head`]), calls [`Playhead::advance`] with it and
//! [`Consumer::take`], none of which waits on a step or a read; a part missing when its hour is
//! stepped is a stall, counted, and the next step takes it up.

use crate::{
    changes::{Changes, Texts},
    clock::Clock,
    invalid,
    lane::{
        Fetch, Fetched, Held, Lane, Sought,
        log::{self, Rows, Wanted},
    },
    pages::{Pages, Run},
    ring::Ring,
    state::{Flight, Rebuild, State},
    step::{Built, Fetching, Rebuilt, Seeking, Step},
};
use replay_columnar::unit::{BLOCK, reader};
use replay_data::Hour;
use std::{
    cmp::Ordering as Order,
    collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque},
    io,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[cfg(test)]
mod tests;

/// A step predicted to take longer than this on one thread is split over the stepper's threads.
const SPLIT_NS: f64 = 500_000.0;

/// A lane's nanoseconds a stepped hour before it has stepped: enough that a first step of many
/// lanes is split.
const FIRST_COST: f64 = 1_000.0;

/// How often what the playhead holds is summed.
const REPORT_EVERY: Duration = Duration::from_millis(100);

/// The next piece's lanes are prepared once the newest step is this much playing time from its
/// end, at the least.
const PREPARE_SECONDS: f64 = 0.5;

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// Threads a large step's lanes are split over, the stepper's own included.
    pub step_threads: usize,
    pub fetch_threads: usize,
    /// Steps of read-ahead past the newest step built.
    pub ahead: u32,
    /// Steps built past the head, at most.
    pub ring: usize,
    /// The clock's game hours a second at the start, and frames a second: a step is
    /// `speed / rate` hours, at least one.
    pub speed: f64,
    pub rate: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self { step_threads: 1, fetch_threads: 1, ahead: 4, ring: 3, speed: 1440.0, rate: 60.0 }
    }
}

/// A run's hours `first` to `last` a timeline reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub run: usize,
    pub first: Hour,
    pub last: Hour,
}

/// Pieces joined in order, in rising game hours; hours between two pieces are a gap.
#[derive(Clone, Debug)]
pub struct Timeline {
    pieces: Vec<Piece>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Region {
    Piece(usize),
    /// Hours before piece `next`.
    Gap { next: usize },
}

impl Timeline {
    /// Refused by name unless each piece lies in its run and starts after the one before ends.
    pub fn new(pages: &dyn Pages, pieces: Vec<Piece>) -> io::Result<Self> {
        if pieces.is_empty() {
            return Err(invalid("A timeline holds no piece"));
        }
        for (at, piece) in pieces.iter().enumerate() {
            let run = pages.runs().get(piece.run).ok_or_else(|| invalid(format!("Piece {at} names run {}, which the recording does not hold", piece.run)))?;
            if piece.first > piece.last || piece.first < run.first_hour || piece.last > run.last_hour() {
                return Err(invalid(format!("Piece {at} reads hours {} to {} of a run of hours {} to {}", piece.first, piece.last, run.first_hour, run.last_hour())));
            }
            if at > 0 && piece.first <= pieces[at - 1].last {
                return Err(invalid(format!("Piece {at} starts at hour {}, not after the piece before it", piece.first)));
            }
        }
        Ok(Self { pieces })
    }

    /// Every run whole, in order.
    pub fn whole(pages: &dyn Pages) -> io::Result<Self> {
        let pieces = pages.runs().iter().enumerate().map(|(run, held)| Piece { run, first: held.first_hour, last: held.last_hour() }).collect();
        Self::new(pages, pieces)
    }

    pub fn pieces(&self) -> &[Piece] {
        &self.pieces
    }

    pub fn first(&self) -> Hour {
        self.pieces[0].first
    }

    pub fn last(&self) -> Hour {
        self.pieces[self.pieces.len() - 1].last
    }

    pub fn clamp(&self, hour: Hour) -> Hour {
        hour.clamp(self.first(), self.last())
    }

    fn region(&self, hour: Hour) -> Region {
        let at = self.pieces.partition_point(|piece| piece.last < hour).min(self.pieces.len() - 1);
        if self.pieces[at].first <= hour { Region::Piece(at) } else { Region::Gap { next: at } }
    }
}

/// What one reader reads of one table: columns (indexes into the table's), the rows' keys, and
/// the rows born and gone inside a step (`passing`: a log, an event layer).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declared {
    pub table: usize,
    pub columns: Vec<usize>,
    pub keyed: bool,
    pub passing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Reader(u64);

/// A reader's tables, each with the declaration it has been declared as is since (`since`, by
/// place), and the rebuild that makes its state of one of them from its sought lane.
#[derive(Clone)]
struct Declaration {
    tables: Vec<Declared>,
    since: Vec<u64>,
    rebuild: Option<Rebuild>,
}

/// One reader's reading of one table, held as is since declaration `since`.
#[derive(Clone)]
struct Reading {
    reader: Reader,
    columns: Vec<usize>,
    keyed: bool,
    passing: bool,
    since: u64,
    rebuild: Option<Rebuild>,
}

impl Reading {
    /// Whether it is the reading the stepper applied: a table dropped and declared again between
    /// two applications is not, though its columns are.
    fn same(&self, other: &Self) -> bool {
        (self.reader, &self.columns, self.keyed, self.passing, self.since) == (other.reader, &other.columns, other.keyed, other.passing, other.since)
    }
}

/// A table's selection: the union of its readings, and the readings.
#[derive(Clone)]
struct Selected {
    columns: Vec<usize>,
    keyed: bool,
    passing: bool,
    readings: Vec<Reading>,
}

impl Selected {
    /// The rebuilds of its readers, or of those of `only`, each with the reader's columns.
    fn rebuilds(&self, only: Option<&[Reader]>) -> Vec<(Reader, Vec<usize>, Rebuild)> {
        self.readings
            .iter()
            .filter(|reading| only.is_none_or(|only| only.contains(&reading.reader)))
            .filter_map(|reading| reading.rebuild.as_ref().map(|rebuild| (reading.reader, reading.columns.clone(), Arc::clone(rebuild))))
            .collect()
    }
}

/// Read access to every lane's cells at a step's end, on the stepper's thread.
pub struct Lanes<'a> {
    slots: &'a [Slot],
}

impl<'a> Lanes<'a> {
    pub fn get(&self, table: usize) -> Option<&'a Lane> {
        self.slots.binary_search_by_key(&table, |slot| slot.table).ok().map(|at| &*self.slots[at].lane)
    }

    pub fn iter(&self) -> impl Iterator<Item = &'a Lane> + 'a {
        self.slots.iter().map(|slot| &*slot.lane)
    }
}

/// Called on the stepper's thread after each step is built, before readers can take it.
pub type Hook = Box<dyn FnMut(&Step, &Lanes<'_>) + Send>;

/// What the playhead holds, in bytes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Report {
    pub lanes: usize,
    /// The lanes stepping: cells, parts under the head and fetched ahead, their tracks.
    pub held: Held,
    /// Steps built and steps consumers hold.
    pub ring_steps: usize,
    pub ring_bytes: usize,
    pub fetches_queued: usize,
    pub jobs_running: usize,
    /// Nanoseconds the fetchers spent freeing what the stepper let go of, since the start.
    pub freed_ns: u64,
    /// Lanes sought ahead (a seek under way, a table newly selected, the next piece): count and
    /// what they hold.
    pub preparing: usize,
    pub preparing_held: Held,
    /// Readers' rebuilt states not taken yet (while their seek runs and in their reset step):
    /// count and the bytes their rebuilds reported. In the peak report, each at its own peak.
    pub states: usize,
    pub state_bytes: usize,
    pub total: usize,
}

/// Indices of [`Counters::idle_ns`].
const IDLE_RING_FULL: usize = 0;
const IDLE_ROOM_BEHIND: usize = 1;
const IDLE_FETCH: usize = 2;
const IDLE_END: usize = 3;
const IDLE_OTHER: usize = 4;

/// Counted since the playhead started.
#[derive(Clone, Copy, Debug, Default)]
pub struct Counters {
    pub steps: u64,
    /// Table steps that stopped at a part not fetched yet, and steps holding any.
    pub stalls: u64,
    pub stalled_steps: u64,
    /// Times a piece's first hour was reached before its lanes were ready.
    pub piece_waits: u64,
    pub seeks: u64,
    /// Lanes sought for a table new to the lanes or whose columns changed.
    pub joins: u64,
    /// Tables such a lane brought, each at the hour the frames were at, paused or playing.
    pub joins_at_head: u64,
    /// Not counted since no join holds a lane or a step: kept for the reports that read it.
    pub join_holds: u64,
    /// Steps of no new hours that caught a table up once its parts came.
    pub catch_ups: u64,
    pub split_steps: u64,
    /// Nanoseconds the stepper was behind the head and built nothing for want of room in the
    /// ring (the ring's size of steps no consumer had taken: every reader behind, such as the
    /// only map composing).
    pub room_held_ns: u64,
    /// Nanoseconds the stepper waited with nothing to build, by why its last build stopped: the
    /// ring full ahead of the head (the frames had not taken its steps), the ring full behind the
    /// head, a lane's part not fetched yet, the recording's end, and anything else.
    pub idle_ns: [u64; 5],
    /// Not counted since no join holds the stepper: kept for the reports that read it.
    pub join_held_ns: u64,
    /// Steps made for one consumer from the lanes in memory (a table declared that the lanes held,
    /// or [`Consumer::land`]), and the nanoseconds their readers' rebuilds took.
    pub landings: u64,
    pub landing_rebuild_ns: u64,
    /// Times the steps past the frames were rebuilt for a pace asking smaller steps
    /// ([`Step::restep`]).
    pub resteps: u64,
    /// At each landing check, the joins that could not land, by the first reason: its prepare
    /// not answered yet, a read of its lane out, its lane short of the newest step, its lane sought
    /// past the frames, the lane it replaces not whole at the frames, or not whole at a step past
    /// them.
    pub join_waits: [u64; 6],
    /// Joins' prepares answered for a join no longer there (replaced, dropped or sought again).
    pub join_answers_dropped: u64,
    /// Joins under way now (not landed, dropped or replaced), and the landing checks made with
    /// every join under way read and no read of one out: published and announced (the playhead's
    /// wake) whenever either moves, so a caller can wait for a join's read to have had its turn.
    pub joins_open: u64,
    pub ripe_checks: u64,
    /// The newest declaration the stepper has applied ([`Playhead::declarations`]): with it equal
    /// and no join open, every table declared so far has been read and landed or was held already.
    pub declarations_applied: u64,
}

struct Slot {
    table: usize,
    generation: u64,
    lane: Box<Lane>,
    fetching: bool,
    /// Nanoseconds a stepped hour, smoothed.
    cost: f64,
    /// The newest steps' last hours and where each left this table, oldest first.
    ends: VecDeque<(Hour, Option<Hour>)>,
}

impl Slot {
    fn new(generation: u64, lane: Box<Lane>) -> Self {
        Self { table: lane.table(), generation, lane, fetching: false, cost: FIRST_COST, ends: VecDeque::new() }
    }

    /// A step ending at `last` left the lane where it is now.
    fn ended(&mut self, last: Hour, keep: usize) {
        if self.ends.len() >= keep {
            self.ends.pop_front();
        }
        self.ends.push_back((last, self.lane.head()));
    }

    /// Where the step ending at `hour` left the table, when one did.
    fn head_at(&self, hour: Hour) -> Option<Hour> {
        self.ends.iter().rev().find(|(last, _)| *last <= hour).filter(|(last, _)| *last == hour).and_then(|&(_, head)| head)
    }

    /// Whether the steps through `hour` left the table at `hour`, short of none.
    fn whole_at(&self, hour: Hour) -> bool {
        self.ends.iter().rev().find(|(last, _)| *last <= hour).is_some_and(|&(last, head)| last == hour && head == Some(hour))
    }
}

/// What a read's parts are given to.
#[derive(Clone, Copy)]
enum Fed {
    Slot(usize),
    Join,
    Restep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Purpose {
    Seek,
    Join,
    Piece(usize),
    Restep,
}

struct Prepare {
    epoch: u64,
    purpose: Purpose,
    table: usize,
    generation: u64,
    run: usize,
    columns: Vec<usize>,
    keyed: bool,
    passing: bool,
    hour: Hour,
    through: Hour,
    /// A join's: sought instead to the hour the frames are at when it starts, no later than the
    /// first, and read the second's hours past it.
    frames: Option<(Hour, Hour)>,
    rebuilds: Vec<(Reader, Vec<usize>, Rebuild)>,
}

struct Prepared {
    generation: u64,
    lane: Box<Lane>,
    states: Vec<(Reader, State)>,
    sought: Sought,
    rebuild_ns: u64,
    ns: u64,
    /// Of the reads, those past the hour sought (the read-ahead).
    ahead_ns: u64,
    /// The thread that prepared it.
    thread: std::thread::ThreadId,
}

/// What the stepper let go of, freed on a fetcher.
type Retired = Box<dyn Send>;

enum Job {
    Prepare(Prepare),
    Fetch { epoch: u64, table: usize, generation: u64, fetch: Fetch },
    Drop(Retired),
    Rows(Wanted, Reply),
}

/// A request of listed rows' cells and, once a fetcher has read them, its answer.
#[derive(Default)]
struct Answer {
    rows: Mutex<Option<io::Result<Rows>>>,
    ready: Condvar,
}

/// Where a fetcher answers a request; dropped unanswered (the playhead stopped), it says so.
struct Reply(Option<Arc<Answer>>);

impl Reply {
    fn answer(answer: &Arc<Answer>, rows: io::Result<Rows>) {
        *lock(&answer.rows) = Some(rows);
        answer.ready.notify_all();
    }

    fn give(mut self, rows: io::Result<Rows>) {
        if let Some(answer) = self.0.take() {
            Self::answer(&answer, rows);
        }
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        if let Some(answer) = self.0.take() {
            Self::answer(&answer, Err(io::Error::other("The playhead stopped before it read the rows asked")));
        }
    }
}

/// What a reader may ask of a playhead's file from any thread without holding the playhead: a
/// table's class and a log's rows by ordinal. It keeps the request queue alive, not the stepper;
/// a request made once the playhead has stopped is answered with that.
#[derive(Clone)]
pub struct LogReads {
    shared: Arc<Shared>,
}

impl LogReads {
    /// Whether `table` is a log in every run of the file, as its pages say.
    pub fn is_log(&self, table: usize) -> io::Result<bool> {
        log::is_log(&*self.shared.pages, table)
    }

    /// [`Playhead::rows`].
    pub fn rows(&self, wanted: Wanted) -> Asked {
        let answer = Arc::new(Answer::default());
        let reply = Reply(Some(Arc::clone(&answer)));
        if lock(&self.shared.jobs).stop {
            drop(reply);
        } else {
            self.shared.queue([((3, 0), Job::Rows(wanted, reply))]);
        }
        Asked { answer }
    }
}

/// Rows asked of a log by ordinal ([`Playhead::rows`]), read on a fetcher.
pub struct Asked {
    answer: Arc<Answer>,
}

impl Asked {
    /// The rows once read, taken once; never waits.
    pub fn take(&self) -> Option<io::Result<Rows>> {
        lock(&self.answer.rows).take()
    }

    /// Waits for the rows: for a caller off the frame.
    pub fn wait(self) -> io::Result<Rows> {
        let mut rows = lock(&self.answer.rows);
        loop {
            if let Some(answer) = rows.take() {
                return answer;
            }
            rows = self.answer.ready.wait(rows).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
}

enum Done {
    Prepared { epoch: u64, purpose: Purpose, table: usize, generation: u64, result: io::Result<Box<Prepared>> },
    Fetched { epoch: u64, table: usize, generation: u64, result: io::Result<Fetched> },
}

struct Queued {
    /// Class (frees, prepares, reads) and the order inside it.
    rank: (u8, i64),
    seq: u64,
    job: Job,
}

impl PartialEq for Queued {
    fn eq(&self, other: &Self) -> bool {
        (self.rank, self.seq) == (other.rank, other.seq)
    }
}

impl Eq for Queued {}

impl PartialOrd for Queued {
    fn partial_cmp(&self, other: &Self) -> Option<Order> {
        Some(self.cmp(other))
    }
}

impl Ord for Queued {
    fn cmp(&self, other: &Self) -> Order {
        (other.rank, other.seq).cmp(&(self.rank, self.seq))
    }
}

#[derive(Default)]
struct Jobs {
    heap: BinaryHeap<Queued>,
    /// Joins' prepares: after every read the steps wait on, or on threads lent to them.
    joins: VecDeque<Job>,
    seq: u64,
    running: usize,
    stop: bool,
}

impl Jobs {
    fn push(&mut self, rank: (u8, i64), job: Job) {
        self.seq += 1;
        self.heap.push(Queued { rank, seq: self.seq, job });
    }
}

struct Control {
    version: u64,
    head: Hour,
    pace: (f64, f64),
    seek: Option<(Hour, u64)>,
    /// When the seek waiting for the stepper was asked.
    asked: Option<Instant>,
    readers: BTreeMap<Reader, Declaration>,
    next_reader: u64,
    selection: u64,
    /// Consumers that asked to land ([`Consumer::land`]).
    lands: Vec<usize>,
    done: Vec<Done>,
    newest: Option<Hour>,
    /// Frames that asked the head it already stood at ([`Playhead::advance`]).
    stood: u64,
    stop: bool,
    error: Option<String>,
}

struct Shared {
    pages: Arc<dyn Pages>,
    timeline: Timeline,
    settings: Settings,
    epoch: AtomicU64,
    freed_ns: AtomicU64,
    control: Mutex<Control>,
    wake: Condvar,
    jobs: Mutex<Jobs>,
    work: Condvar,
    ring: Mutex<Ring>,
    clock: Mutex<Clock>,
    flight: Arc<Flight>,
    report: Mutex<(Report, Report, Counters)>,
    /// [`Playhead::exact_when_paused`].
    exact: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Shared {
    fn nudge(&self, change: impl FnOnce(&mut Control)) {
        let mut control = lock(&self.control);
        change(&mut control);
        control.version += 1;
        drop(control);
        self.wake.notify_all();
    }

    fn seek(&self, hour: Hour) {
        let hour = self.timeline.clamp(hour);
        let mut clock = lock(&self.clock);
        let epoch = lock(&self.ring).abandon();
        self.epoch.store(epoch, Ordering::Release);
        clock.seek(hour, epoch);
        self.nudge(|control| {
            control.seek = Some((hour, epoch));
            control.asked = Some(Instant::now());
            control.head = hour;
            control.newest = None;
        });
        drop(clock);
    }

    /// The clock's head while it is paused and holds no seek.
    fn paused_at(&self) -> Option<Hour> {
        let clock = lock(&self.clock);
        (!clock.playing() && clock.held().is_none()).then(|| self.timeline.clamp(clock.head(Instant::now())))
    }

    fn queue(&self, jobs: impl IntoIterator<Item = ((u8, i64), Job)>) {
        let mut held = lock(&self.jobs);
        let before = held.heap.len();
        for (rank, job) in jobs {
            held.push(rank, job);
        }
        let added = held.heap.len() - before;
        drop(held);
        match added {
            0 => {}
            1 => self.work.notify_one(),
            _ => self.work.notify_all(),
        }
    }

    fn queue_joins(&self, jobs: Vec<Job>) {
        if jobs.is_empty() {
            return;
        }
        lock(&self.jobs).joins.extend(jobs);
        self.work.notify_all();
    }

    /// The next join's prepare, for a thread lent to the joins.
    fn pop_join(&self) -> Option<Job> {
        let mut jobs = lock(&self.jobs);
        if jobs.stop {
            return None;
        }
        let job = jobs.joins.pop_front()?;
        jobs.running += 1;
        Some(job)
    }

    fn free(&self, retired: impl Send + 'static) {
        self.queue([((0, 0), Job::Drop(Box::new(retired)))]);
    }

    /// Freed once every prepare queued before it has started: what a seek lets go of waits for
    /// the seek's own work.
    fn free_after_prepares(&self, retired: impl Send + 'static) {
        self.queue([((1, i64::MAX), Job::Drop(Box::new(retired)))]);
    }

    /// The next free or seek job, for a thread that helps a seek; none once only reads are left.
    fn pop_urgent(&self) -> Option<Job> {
        let mut jobs = lock(&self.jobs);
        if jobs.stop || jobs.heap.peek().is_none_or(|queued| queued.rank.0 > 1) {
            return None;
        }
        jobs.running += 1;
        Some(jobs.heap.pop().expect("peeked").job)
    }

    /// Runs one job on this thread: what it hands the stepper, if anything.
    fn run(&self, job: Job) -> Option<Done> {
        let done = match job {
            Job::Prepare(prepare) if prepare.epoch != self.epoch.load(Ordering::Acquire) => None,
            Job::Prepare(prepare) => {
                let result = self.prepare(&prepare);
                let Prepare { epoch, purpose, table, generation, .. } = prepare;
                Some(Done::Prepared { epoch, purpose, table, generation, result })
            }
            // A read for the steps a seek abandoned is never run: the seek's own work goes first.
            Job::Fetch { epoch, fetch, .. } if epoch != self.epoch.load(Ordering::Acquire) => {
                drop(fetch);
                None
            }
            Job::Fetch { epoch, table, generation, fetch } => Some(Done::Fetched { epoch, table, generation, result: fetch.run() }),
            Job::Rows(wanted, reply) => {
                reply.give(log::rows(&*self.pages, &wanted));
                None
            }
            Job::Drop(retired) => {
                let started = Instant::now();
                drop(retired);
                self.freed_ns.fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
                None
            }
        };
        lock(&self.jobs).running -= 1;
        done
    }
}

impl Shared {
    /// A table's lane sought to the hour asked and read ahead, and each reader's rebuild run on it
    /// unless the seek is abandoned meanwhile.
    fn prepare(&self, prepare: &Prepare) -> io::Result<Box<Prepared>> {
        let started = Instant::now();
        let (hour, through) = match prepare.frames {
            Some((latest, span)) => {
                let hour = lock(&self.ring).frames().map_or(prepare.hour, |(_, hour)| hour.clamp(prepare.hour, latest));
                (hour, hour + span)
            }
            None => (prepare.hour, prepare.through),
        };
        let mut lane = Box::new(Lane::new(Arc::clone(&self.pages), prepare.run, prepare.table, &prepare.columns, prepare.keyed)?);
        lane.set_passing(prepare.passing);
        let mut sought = lane.seek(hour)?;
        let (ahead_ns, _) = lane.fetch_through(through)?;
        sought.fetch_ns += ahead_ns;
        let mut states = Vec::with_capacity(prepare.rebuilds.len());
        let mut rebuild_ns = 0;
        for (reader, columns, rebuild) in &prepare.rebuilds {
            if self.epoch.load(Ordering::Acquire) != prepare.epoch {
                break;
            }
            let began = Instant::now();
            let mut state = rebuild(&lane, columns)?;
            rebuild_ns += began.elapsed().as_nanos() as u64;
            state.hold(&self.flight);
            states.push((*reader, state));
        }
        Ok(Box::new(Prepared { generation: prepare.generation, lane, states, sought, rebuild_ns, ns: started.elapsed().as_nanos() as u64, ahead_ns, thread: std::thread::current().id() }))
    }
}

/// The bytes of a table's restore before `hour` and of the parts since: what seeking it reads.
fn seek_cost(run: &Run, table: usize, hour: Hour) -> i64 {
    let target = (hour - run.first_hour).max(0) as u32;
    let block = ((target / BLOCK) as usize).min(run.pages.len() - 1);
    let Some((restored, entry)) = reader::restored_before(&run.pages, table, block) else { return 0 };
    let mut bytes: u64 = entry.streams.iter().flat_map(|(_, chunks)| chunks).map(|chunk| u64::from(chunk.frame.stored) + chunk.prefix.as_ref().map_or(0, |frame| u64::from(frame.stored))).sum();
    for page in &run.pages[restored..=block] {
        bytes += page.tables[table].parts.iter().flat_map(|part| &part.frames).map(|frame| u64::from(frame.stored)).sum::<u64>();
    }
    bytes as i64
}

/// One batch of lanes stepped by several threads: each takes the next lane, largest first.
struct Batch {
    to: Hour,
    queue: Mutex<Vec<Item>>,
    done: Mutex<Vec<Item>>,
    finished: Condvar,
    left: AtomicUsize,
    threads: AtomicUsize,
}

struct Item {
    index: usize,
    predicted: f64,
    slot: Slot,
    changes: Changes,
    result: io::Result<()>,
}

impl Batch {
    fn work(&self) {
        let mut took = false;
        loop {
            let Some(mut item) = lock(&self.queue).pop() else { break };
            took = true;
            item.result = step_slot(&mut item.slot, self.to, &mut item.changes);
            let mut done = lock(&self.done);
            done.push(item);
            let last = self.left.fetch_sub(1, Ordering::AcqRel) == 1;
            drop(done);
            if last {
                self.finished.notify_all();
            }
        }
        if took {
            self.threads.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn wait(&self) {
        let mut done = lock(&self.done);
        while self.left.load(Ordering::Acquire) > 0 {
            done = self.finished.wait(done).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
}

/// Steps a lane to `to`; the table is short by the hours it stops before.
fn step_slot(slot: &mut Slot, to: Hour, changes: &mut Changes) -> io::Result<()> {
    let from = slot.lane.head().unwrap_or(to);
    let started = Instant::now();
    slot.lane.step(to, changes)?;
    let ns = started.elapsed().as_nanos() as f64;
    let hours = (to - from).max(1) as f64;
    slot.cost = slot.cost * 0.75 + ns / hours * 0.25;
    Ok(())
}

enum Help {
    Batch(Arc<Batch>),
    Prepares,
}

/// The playback head over one timeline of one recording.
pub struct Playhead {
    shared: Arc<Shared>,
    stepper: Option<JoinHandle<Vec<Lane>>>,
    threads: Vec<JoinHandle<()>>,
}

/// One reader's frame-side view of the steps: sees every step once, and takes the states rebuilt
/// for its reader.
pub struct Consumer {
    shared: Arc<Shared>,
    id: usize,
    reader: Reader,
}

impl Consumer {
    /// Appends to `out` the steps not taken yet whose last hour `head` has reached, in order: those
    /// built, and those made for this consumer alone (a table its reader declared, landed at the
    /// hour it had taken, or [`Self::land`]), each a step of no hours. Never waits on a step; a step goes back to the ring when every holder drops it. Taking a
    /// seek's reset starts the clock from the hour sought, whichever consumer takes it first: no
    /// consumer's own work on the reset holds the clock for the others.
    pub fn take(&mut self, head: Hour, out: &mut Vec<Arc<Step>>) -> usize {
        let from = out.len();
        let taken = lock(&self.shared.ring).take(self.id, head, out);
        if taken > 0 {
            if let Some(reset) = out[from..].iter().find(|step| step.reset) {
                let now = Instant::now();
                lock(&self.shared.clock).shown(reset.epoch, now);
            }
            self.shared.nudge(|_| {});
        }
        taken
    }

    /// Appends to `out` the steps built and not taken yet, in order, each with its hours: the steps
    /// ahead of the head, to read early. Looking takes none; after a seek they are the new
    /// position's, from its reset on.
    pub fn ahead(&self, out: &mut Vec<Arc<Step>>) {
        lock(&self.shared.ring).ahead(self.id, out);
    }

    /// Whether a step it has not taken ends at or before `head`: its next take at `head` moves it.
    pub fn reaches(&self, head: Hour) -> bool {
        lock(&self.shared.ring).reaches(self.id, head)
    }

    /// Lands this consumer at the newest step built, never waiting: every step it has not taken is
    /// passed over, and the next it takes, once its head reaches that step, is a step of no hours
    /// holding each table its reader declares whole (a reset, with the reader's state), made from
    /// the lanes in memory. No lane is sought, no other consumer's step changes and the stepper
    /// builds on meanwhile. A table of its reader that a lane is still being sought for comes with
    /// that lane, and the landing waits for it.
    pub fn land(&mut self) {
        let id = self.id;
        self.shared.nudge(|control| control.lands.push(id));
    }

    /// The state rebuilt for this consumer's reader of `table` in `step`, once.
    pub fn state(&self, step: &Step, table: usize) -> Option<State> {
        let mut states = lock(&step.states);
        let at = states.iter().position(|held| held.reader == self.reader && held.table == table)?;
        let mut state = states.swap_remove(at).state;
        drop(states);
        state.release();
        Some(state)
    }

    pub fn reader(&self) -> Reader {
        self.reader
    }
}

impl Drop for Consumer {
    fn drop(&mut self) {
        lock(&self.shared.ring).leave(self.id);
        self.shared.nudge(|_| {});
    }
}

impl Playhead {
    /// A playhead over `timeline`; it holds nothing until the first [`Self::seek`].
    pub fn new(pages: Arc<dyn Pages>, timeline: Timeline, settings: Settings, hook: Option<Hook>) -> io::Result<Self> {
        if settings.step_threads == 0 || settings.fetch_threads == 0 || settings.ring == 0 {
            return Err(invalid("A playhead needs a thread of each kind and a ring of one step"));
        }
        if !(settings.speed > 0.0 && settings.rate > 0.0) {
            return Err(invalid("A playhead's speed and frame rate are positive"));
        }
        let (first, last) = (timeline.first(), timeline.last());
        let shared = Arc::new(Shared {
            pages,
            timeline,
            settings,
            epoch: AtomicU64::new(0),
            freed_ns: AtomicU64::new(0),
            control: Mutex::new(Control {
                version: 0,
                head: first,
                pace: (settings.speed, settings.rate),
                seek: None,
                asked: None,
                readers: BTreeMap::new(),
                next_reader: 0,
                selection: 0,
                lands: Vec::new(),
                done: Vec::new(),
                newest: None,
                stood: 0,
                stop: false,
                error: None,
            }),
            wake: Condvar::new(),
            jobs: Mutex::new(Jobs::default()),
            work: Condvar::new(),
            ring: Mutex::new(Ring::new(settings.ring)),
            clock: Mutex::new(Clock::new(first, last, settings.speed)),
            flight: Arc::new(Flight::default()),
            report: Mutex::new(Default::default()),
            exact: AtomicBool::new(false),
        });
        let mut threads = Vec::new();
        let spawn = |name: String, work: Box<dyn FnOnce() + Send>| std::thread::Builder::new().name(name).spawn(work);
        for at in 0..settings.fetch_threads {
            let held = Arc::clone(&shared);
            threads.push(spawn(format!("playhead fetch {at}"), Box::new(move || fetcher(&held)))?);
        }
        let mut helpers = Vec::new();
        for at in 1..settings.step_threads {
            let (send, receive) = mpsc::channel();
            helpers.push(send);
            let held = Arc::clone(&shared);
            threads.push(spawn(format!("playhead step {at}"), Box::new(move || helper(&held, &receive)))?);
        }
        let held = Arc::clone(&shared);
        let stepper = std::thread::Builder::new().name("playhead stepper".into()).spawn(move || Stepper::new(held, hook, helpers).run())?;
        Ok(Self { shared, stepper: Some(stepper), threads })
    }

    pub fn timeline(&self) -> &Timeline {
        &self.shared.timeline
    }

    /// A reader's tables and columns, and its rebuild when it keeps state by slot; the selection is
    /// the union of every reader's. The rebuild runs for each of its tables at a seek and at a
    /// piece's start, and for one table when the reader declares it anew or changes its columns.
    pub fn declare(&self, tables: Vec<Declared>, rebuild: Option<Rebuild>) -> io::Result<Reader> {
        self.check(&tables)?;
        let mut id = 0;
        self.shared.nudge(|control| {
            id = control.next_reader;
            control.next_reader += 1;
            control.selection += 1;
            let since = vec![control.selection; tables.len()];
            control.readers.insert(Reader(id), Declaration { tables, since, rebuild });
        });
        Ok(Reader(id))
    }

    /// The reader's own selection change: its tables and columns are now these; its rebuild stays.
    pub fn redeclare(&self, reader: Reader, tables: Vec<Declared>) -> io::Result<()> {
        self.check(&tables)?;
        let mut found = false;
        self.shared.nudge(|control| {
            let stamp = control.selection + 1;
            if let Some(held) = control.readers.get_mut(&reader) {
                held.since = tables.iter().map(|declared| held.tables.iter().position(|before| before == declared).map_or(stamp, |at| held.since[at])).collect();
                held.tables = tables;
                control.selection = stamp;
                found = true;
            }
        });
        if found { Ok(()) } else { Err(invalid(format!("Reader {} is retired", reader.0))) }
    }

    fn check(&self, tables: &[Declared]) -> io::Result<()> {
        for (at, declared) in tables.iter().enumerate() {
            let kinds = self.shared.pages.kinds(declared.table).ok_or_else(|| invalid(format!("No table {} in the schema", declared.table)))?;
            if let Some(column) = declared.columns.iter().find(|&&column| column >= kinds.len()) {
                return Err(invalid(format!("Table {} holds no column {column}", declared.table)));
            }
            if tables[..at].iter().any(|before| before.table == declared.table) {
                return Err(invalid(format!("Table {} is declared twice by one reader", declared.table)));
            }
            if declared.passing && log::is_log(&*self.shared.pages, declared.table)? {
                return Err(invalid(format!("Table {} is a log: every row it appends passes once, in its steps' appended rows; it is not declared passing", declared.table)));
            }
        }
        Ok(())
    }

    /// The cells of a log's rows by ordinal, read on a fetcher after the reads the steps wait on;
    /// the answer is taken from [`Asked`] off the frame or when it is ready.
    pub fn rows(&self, wanted: Wanted) -> Asked {
        self.log_reads().rows(wanted)
    }

    /// Its file's log reads, for a reader that asks them from any thread, the stepper's included.
    pub fn log_reads(&self) -> LogReads {
        LogReads { shared: Arc::clone(&self.shared) }
    }

    pub fn retire(&self, reader: Reader) {
        lock(&self.shared.ring).forget(reader);
        self.shared.nudge(|control| {
            if control.readers.remove(&reader).is_some() {
                control.selection += 1;
            }
        });
    }

    /// A new consumer for `reader`, which takes the steps from those still in the ring on, and
    /// those made for its reader alone that no consumer of it has taken.
    pub fn consumer(&self, reader: Reader) -> Consumer {
        let id = lock(&self.shared.ring).join(reader);
        Consumer { shared: Arc::clone(&self.shared), id, reader }
    }

    /// The frame's head at `now`, from the clock; never waits. Held at a seek's hour until the
    /// frame that asked takes its reset; the clock runs from that frame's `now`.
    pub fn head(&self, now: Instant) -> Hour {
        lock(&self.shared.clock).frame(now)
    }

    /// Plays at `speed` game hours a second, which also sets a step's length ([`Self::pace`]).
    pub fn play(&self, speed: f64) {
        if speed > 0.0 {
            lock(&self.shared.clock).play(speed, Instant::now());
            self.pace(speed);
        }
    }

    /// The pace the steps are sized for, `speed` game hours a second, playing or paused: the steps
    /// built ahead of a paused head are those the clock will play. When it asks smaller steps than
    /// those built past the hour the frames have taken, those are built again at its size.
    pub fn pace(&self, speed: f64) {
        if speed > 0.0 {
            let rate = self.shared.settings.rate;
            self.shared.nudge(|control| control.pace = (speed, rate));
        }
    }

    pub fn pause(&self) {
        lock(&self.shared.clock).pause(Instant::now());
        self.shared.nudge(|_| {});
    }

    /// From now on, while the clock is paused, every step and every view the hook is shown is of
    /// the head's hour: no step is built past the head, and a pause with steps built past it resets
    /// the lanes at it, as a seek there. A hook's answer made while paused is then the clock's.
    pub fn exact_when_paused(&self) {
        self.shared.exact.store(true, Ordering::Release);
        self.shared.nudge(|_| {});
    }

    /// The seek whose reset the clock is held for, if any.
    pub fn holding(&self) -> Option<u64> {
        lock(&self.shared.clock).held()
    }

    /// The newest seek's epoch, which its steps carry ([`Step::epoch`]): a reader composing steps
    /// of an older one knows a seek has superseded them.
    pub fn epoch(&self) -> u64 {
        self.shared.epoch.load(Ordering::Acquire)
    }

    /// Nanoseconds the fetchers have spent freeing what the stepper let go of, since the start.
    pub fn freed_ns(&self) -> u64 {
        self.shared.freed_ns.load(Ordering::Relaxed)
    }

    /// The head's game hour, as often as the frame likes; never waits.
    pub fn advance(&self, head: Hour) {
        let head = self.shared.timeline.clamp(head);
        let mut control = lock(&self.shared.control);
        if control.head != head {
            control.head = head;
        } else {
            // The head stands: a landing past it is not reached until it moves.
            control.stood += 1;
        }
        control.version += 1;
        drop(control);
        self.shared.wake.notify_all();
    }

    /// The frames stand at the head they last asked, as a frame asking it again does
    /// ([`Self::advance`]): for a frame held on work that waits on a landing past its head.
    pub fn stand(&self) {
        let mut control = lock(&self.shared.control);
        control.stood += 1;
        control.version += 1;
        drop(control);
        self.shared.wake.notify_all();
    }

    /// Abandons every step built and every read under way; the next step taken is a reset of
    /// every table at `hour`, and the clock holds there until it is taken. A seek under way is
    /// replaced.
    pub fn seek(&self, hour: Hour) {
        self.shared.seek(hour);
    }

    /// The last hour of the newest step built since the last seek.
    pub fn newest(&self) -> Option<Hour> {
        lock(&self.shared.control).newest
    }

    /// Waits until the newest step built since the last seek reaches `hour`; the error that
    /// stopped the playhead first, by name.
    #[cfg(feature = "test-support")]
    pub fn wait_newest(&self, hour: Hour) -> Result<(), String> {
        let mut control = lock(&self.shared.control);
        loop {
            if control.newest.is_some_and(|newest| newest >= hour) {
                return Ok(());
            }
            if let Some(error) = &control.error {
                return Err(error.clone());
            }
            control = self.shared.wake.wait(control).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    /// The newest declaration made ([`Self::declare`], [`Self::redeclare`]), as a stamp the
    /// stepper reports applied in [`Counters::declarations_applied`].
    pub fn declarations(&self) -> u64 {
        lock(&self.shared.control).selection
    }

    /// What it holds now and at its peak, and its counters.
    pub fn held(&self) -> (Report, Report, Counters) {
        *lock(&self.shared.report)
    }

    /// The error that stopped it, by name.
    pub fn error(&self) -> Option<String> {
        lock(&self.shared.control).error.clone()
    }

    /// Stops every thread; the lanes as they stand.
    pub fn finish(mut self) -> Vec<Lane> {
        self.stop()
    }

    fn stop(&mut self) -> Vec<Lane> {
        self.shared.nudge(|control| control.stop = true);
        lock(&self.shared.jobs).stop = true;
        self.shared.work.notify_all();
        let lanes = self.stepper.take().map(|stepper| stepper.join().unwrap_or_default()).unwrap_or_default();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
        lanes
    }
}

#[cfg(test)]
impl Playhead {
    /// Waits until `ready` holds, asked again each time the playhead's state moves; fails, naming
    /// the caller, when the playhead stops.
    #[track_caller]
    fn wait_until(&self, mut ready: impl FnMut() -> bool) {
        let caller = std::panic::Location::caller();
        loop {
            let version = lock(&self.shared.control).version;
            if ready() {
                return;
            }
            let mut control = lock(&self.shared.control);
            while control.version == version {
                if let Some(error) = &control.error {
                    panic!("the playhead stopped during the wait at {caller}: {error}");
                }
                control = self.shared.wake.wait(control).unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        }
    }

    /// The steps built past the head: the ring is full at its size.
    fn ring_len(&self) -> usize {
        let head = lock(&self.shared.control).head;
        lock(&self.shared.ring).past(head)
    }

    /// The steps the ring keeps, built and not yet taken by every consumer.
    fn kept(&self) -> usize {
        lock(&self.shared.ring).steps_kept()
    }
}

impl Drop for Playhead {
    fn drop(&mut self) {
        if self.stepper.is_some() {
            self.stop();
        }
    }
}

fn fetcher(shared: &Shared) {
    loop {
        let job = {
            let mut jobs = lock(&shared.jobs);
            loop {
                if jobs.stop {
                    return;
                }
                if let Some(job) = jobs.heap.pop().map(|queued| queued.job).or_else(|| jobs.joins.pop_front()) {
                    jobs.running += 1;
                    break job;
                }
                jobs = shared.work.wait(jobs).unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        if let Some(done) = shared.run(job) {
            shared.nudge(|control| control.done.push(done));
        }
    }
}

fn helper(shared: &Shared, receive: &Receiver<Help>) {
    while let Ok(help) = receive.recv() {
        match help {
            Help::Batch(batch) => batch.work(),
            Help::Prepares => {
                while let Some(job) = shared.pop_urgent() {
                    if let Some(done) = shared.run(job) {
                        shared.nudge(|control| control.done.push(done));
                    }
                }
            }
        }
    }
}

struct Seek {
    hour: Hour,
    gap: bool,
    piece: usize,
    /// When the seek was asked, and when the stepper began it.
    asked: Instant,
    started: Instant,
    pending: BTreeMap<usize, u64>,
    prepared: Vec<Prepared>,
}

/// The lanes sought from the file to the hour the frames are at, for the pace's `quantum`: each
/// table's prepared lane and whether a read is out for it, and the tables still being sought.
struct Restep {
    quantum: Hour,
    pending: BTreeMap<usize, u64>,
    ready: BTreeMap<usize, (Box<Prepared>, bool)>,
}

struct NextPiece {
    piece: usize,
    pending: BTreeMap<usize, u64>,
    prepared: Vec<Prepared>,
}

/// A table new to the lanes, or whose readers' columns changed: a lane of the union sought from
/// the file to the hour the frames are at, which lands there for `readers` (those that declared it
/// anew; their states are rebuilt on it) once it holds the parts through the newest step
/// ([`Stepper::land_joins`]). The lane it replaces steps on meanwhile.
struct Join {
    generation: u64,
    columns: Vec<usize>,
    keyed: bool,
    passing: bool,
    readers: Vec<Reader>,
    prepared: Option<Box<Prepared>>,
    fetching: bool,
    /// Landings from memory its readers stand short of, withdrawn when it lands for them.
    revoke: Vec<Arc<Step>>,
}

/// A landing from memory at the newest step built, given to a reader's consumers while the clock
/// played: its tables, its hour, and the frames that had asked a standing head when it was given.
struct Given {
    reader: Reader,
    tables: Vec<usize>,
    step: Arc<Step>,
    hour: Hour,
    stood: u64,
    read: bool,
}

struct Stepper {
    shared: Arc<Shared>,
    hook: Option<Hook>,
    helpers: Vec<Sender<Help>>,
    epoch: u64,
    head: Hour,
    pace: (f64, f64),
    seek: Option<Seek>,
    slots: Vec<Slot>,
    piece: usize,
    newest: Option<Hour>,
    selection: BTreeMap<usize, Selected>,
    /// The selection the lanes stepping and those prepared were made for.
    made: BTreeMap<usize, Selected>,
    selection_seen: u64,
    selection_applied: bool,
    generation: u64,
    joining: BTreeMap<usize, Join>,
    /// Tables each reader declared anew that the lanes hold as it reads them: each lands for the
    /// reader's consumers alone, from memory ([`Stepper::land_owed`]).
    owed: BTreeMap<Reader, BTreeSet<usize>>,
    /// Consumers that asked to land, not landed yet.
    lands: Vec<usize>,
    /// The steps past the frames rebuilt so far ([`Step::restep`]), the quantum the newest step of
    /// hours was built for, and the lanes being sought to rebuild them at the pace's.
    restep: u64,
    paced: Hour,
    /// The pace asked smaller steps than those built: they are rebuilt.
    coarse: bool,
    restepping: Option<Restep>,
    /// The last hour the hook saw before the steps past the frames were rebuilt: it sees no step
    /// through it again.
    hidden: Option<Hour>,
    next_piece: Option<NextPiece>,
    waiting_piece: bool,
    fetching: Fetching,
    counters: Counters,
    /// The joins under way and the ripe checks as last published.
    joins_published: (u64, u64, u64, u64),
    /// Frames that asked a head it stood at, as last read, and the landings from memory not yet taken.
    stood: u64,
    given: Vec<Given>,
    report_at: Instant,
    peak: Report,
    failed: bool,
    /// Since when the stepper has been behind the head building nothing for want of room.
    holding: Option<Instant>,
    /// Why its last build stopped, an index of [`Counters::idle_ns`].
    why: usize,
}

impl Stepper {
    fn new(shared: Arc<Shared>, hook: Option<Hook>, helpers: Vec<Sender<Help>>) -> Self {
        let settings = shared.settings;
        Self {
            head: shared.timeline.first(),
            shared,
            hook,
            helpers,
            epoch: 0,
            pace: (settings.speed, settings.rate),
            seek: None,
            slots: Vec::new(),
            piece: 0,
            newest: None,
            selection: BTreeMap::new(),
            made: BTreeMap::new(),
            selection_seen: 0,
            selection_applied: true,
            generation: 0,
            joining: BTreeMap::new(),
            owed: BTreeMap::new(),
            lands: Vec::new(),
            restep: 0,
            paced: 0,
            coarse: false,
            restepping: None,
            hidden: None,
            next_piece: None,
            waiting_piece: false,
            fetching: Fetching::default(),
            counters: Counters::default(),
            joins_published: (0, 0, 0, 0),
            stood: 0,
            given: Vec::new(),
            report_at: Instant::now(),
            peak: Report::default(),
            failed: false,
            holding: None,
            why: IDLE_OTHER,
        }
    }

    /// Behind the head, the stepper builds nothing for want of room: timed from the first time
    /// until a step is published.
    fn hold(&mut self) {
        if self.newest.is_some_and(|newest| newest < self.head) && self.holding.is_none() {
            self.holding = Some(Instant::now());
        }
    }

    fn release(&mut self) {
        if let Some(since) = self.holding.take() {
            self.counters.room_held_ns += since.elapsed().as_nanos() as u64;
        }
    }

    fn quantum(&self) -> Hour {
        ((self.pace.0 / self.pace.1).round() as Hour).max(1)
    }

    fn run(mut self) -> Vec<Lane> {
        let mut seen = u64::MAX;
        let mut idle = false;
        loop {
            let (seek, done, readers, lands) = {
                let mut control = lock(&self.shared.control);
                if idle {
                    let started = Instant::now();
                    while control.version == seen && !control.stop {
                        control = self.shared.wake.wait(control).unwrap_or_else(std::sync::PoisonError::into_inner);
                    }
                    self.counters.idle_ns[self.why] += started.elapsed().as_nanos() as u64;
                }
                if control.stop {
                    break;
                }
                seen = control.version;
                self.head = control.head;
                self.stood = control.stood;
                self.pace = control.pace;
                let readers = (control.selection != self.selection_seen).then(|| {
                    self.selection_seen = control.selection;
                    control.readers.iter().map(|(&reader, declaration)| (reader, declaration.clone())).collect::<Vec<_>>()
                });
                let seek = control.seek.take().map(|(hour, epoch)| (hour, epoch, control.asked.take().unwrap_or_else(Instant::now)));
                (seek, std::mem::take(&mut control.done), readers, std::mem::take(&mut control.lands))
            };
            if self.failed {
                done.into_iter().for_each(|done| self.discard(done));
                idle = true;
                continue;
            }
            let mut moved = !lands.is_empty();
            self.lands.extend(lands);
            if let Some(readers) = readers {
                self.selection = union(readers);
                self.selection_applied = false;
                self.retire();
                moved = true;
            }
            self.why = IDLE_OTHER;
            let outcome = (|| {
                if let Some((hour, epoch, asked)) = seek {
                    self.begin_seek(hour, epoch, asked);
                    moved = true;
                }
                for done in done {
                    self.receive(done)?;
                    moved = true;
                }
                if self.seek.is_some() {
                    moved |= self.help_seek()?;
                    if self.seek.as_ref().is_some_and(|seek| seek.pending.is_empty()) {
                        self.finish_seek()?;
                        moved = true;
                    }
                } else if self.reset_at_pause() {
                    moved = true;
                } else if self.newest.is_some() {
                    if !self.selection_applied {
                        if !self.apply_selection() {
                            self.hook_declared();
                        }
                        moved = true;
                    }
                    moved |= self.read_for_standing();
                    moved |= self.land_joins()?;
                    moved |= self.restep_ahead()?;
                    moved |= self.land_owed()?;
                    self.schedule();
                    // A declaration since this loop began is applied before the next step is built.
                    let declared = lock(&self.shared.control).selection != self.selection_seen;
                    if declared {
                        moved = true;
                    } else {
                        moved |= self.build()?;
                    }
                    self.schedule();
                    self.prepare_next_piece();
                }
                Ok::<(), io::Error>(())
            })();
            if let Err(error) = outcome {
                self.failed = true;
                self.shared.nudge(|control| control.error = Some(error.to_string()));
            }
            self.publish_joins();
            self.report(false);
            idle = !moved;
        }
        self.slots.into_iter().map(|slot| *slot.lane).collect()
    }

    /// Kept exact while paused: steps built past the paused head are abandoned for a reset there,
    /// which the hook sees in place of them. Whether it sought.
    fn reset_at_pause(&self) -> bool {
        if !self.shared.exact.load(Ordering::Acquire) {
            return false;
        }
        let (Some(newest), Some(head)) = (self.newest, self.shared.paused_at()) else { return false };
        if newest <= head {
            return false;
        }
        self.shared.seek(head);
        true
    }

    fn discard(&self, done: Done) {
        match done {
            Done::Prepared { result: Ok(prepared), .. } => self.shared.free(vec![prepared]),
            Done::Fetched { result: Ok(fetched), .. } => self.shared.free(fetched),
            _ => {}
        }
    }

    /// Every prepared lane not taken in, its states with it, for the caller to free.
    fn abandon_prepared(&mut self) -> Vec<Box<Prepared>> {
        let mut abandoned: Vec<Box<Prepared>> = std::mem::take(&mut self.joining).into_values().filter_map(|join| join.prepared).collect();
        self.counters.joins_open = 0;
        if let Some(next) = self.next_piece.take() {
            abandoned.extend(next.prepared.into_iter().map(Box::new));
        }
        if let Some(seek) = self.seek.take() {
            abandoned.extend(seek.prepared.into_iter().map(Box::new));
        }
        if let Some(restep) = self.restepping.take() {
            abandoned.extend(restep.ready.into_values().map(|(prepared, _)| prepared));
        }
        abandoned
    }

    fn begin_seek(&mut self, hour: Hour, epoch: u64, asked: Instant) {
        self.epoch = epoch;
        self.given.clear();
        self.counters.declarations_applied = self.selection_seen;
        self.counters.seeks += 1;
        self.release();
        let lanes: Vec<Box<Lane>> = std::mem::take(&mut self.slots).into_iter().map(|slot| slot.lane).collect();
        let abandoned = self.abandon_prepared();
        if !lanes.is_empty() || !abandoned.is_empty() {
            self.shared.free_after_prepares((lanes, abandoned));
        }
        self.newest = None;
        (self.hidden, self.coarse) = (None, false);
        self.selection_applied = true;
        self.made = self.selection.clone();
        self.owed.clear();
        self.lands.clear();
        let timeline = &self.shared.timeline;
        let (piece, lanes_at, gap) = match timeline.region(hour) {
            Region::Piece(piece) => (piece, hour, false),
            Region::Gap { next } => (next - 1, timeline.pieces[next - 1].last, true),
        };
        self.piece = piece;
        let run = timeline.pieces[piece].run;
        let through = lanes_at + self.shared.settings.ahead as Hour * self.quantum();
        let mut pending = BTreeMap::new();
        let mut jobs = Vec::new();
        let held_run = &self.shared.pages.runs()[run];
        for (&table, selected) in &self.selection {
            self.generation += 1;
            pending.insert(table, self.generation);
            let cost = seek_cost(held_run, table, lanes_at);
            let prepare = Prepare {
                epoch,
                purpose: Purpose::Seek,
                table,
                generation: self.generation,
                run,
                columns: selected.columns.clone(),
                keyed: selected.keyed,
                passing: selected.passing,
                hour: lanes_at,
                through,
                frames: None,
                rebuilds: selected.rebuilds(None),
            };
            jobs.push(((1, -cost), Job::Prepare(prepare)));
        }
        let tables = jobs.len();
        self.shared.queue(jobs);
        for helper in &self.helpers {
            let _ = helper.send(Help::Prepares);
        }
        self.lend_seek_threads(tables);
        self.seek = Some(Seek { hour, gap, piece, asked, started: Instant::now(), pending, prepared: Vec::new() });
    }

    /// A seek's tables are prepared one a thread, and the frame waits on the last: besides the
    /// stepper, its helpers and the fetchers, a thread more for each table while the machine has
    /// one, each ending once no seek job is left to start.
    fn lend_seek_threads(&self, tables: usize) {
        let own = 1 + self.helpers.len() + self.shared.settings.fetch_threads;
        let width = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        for _ in 0..tables.saturating_sub(own).min(width.saturating_sub(own)) {
            let shared = Arc::clone(&self.shared);
            let lent = std::thread::Builder::new().name("playhead seek".into()).spawn(move || {
                while let Some(job) = shared.pop_urgent() {
                    if let Some(done) = shared.run(job) {
                        shared.nudge(|control| control.done.push(done));
                    }
                }
            });
            if lent.is_err() {
                break;
            }
        }
    }

    /// Runs seek jobs on this thread until none is left to start or another seek arrives.
    fn help_seek(&mut self) -> io::Result<bool> {
        let mut moved = false;
        while self.seek.as_ref().is_some_and(|seek| !seek.pending.is_empty()) {
            if lock(&self.shared.control).seek.is_some() {
                break;
            }
            let Some(job) = self.shared.pop_urgent() else { break };
            if let Some(done) = self.shared.run(job) {
                self.receive(done)?;
            }
            moved = true;
        }
        Ok(moved)
    }

    fn finish_seek(&mut self) -> io::Result<()> {
        let seek = self.seek.take().expect("a seek under way");
        let mut sought = Seeking { tables: seek.prepared.len() as u32, ..Seeking::default() };
        let mut prepared = seek.prepared;
        prepared.sort_by_key(|held| held.lane.table());
        let mut states = Vec::new();
        let mut threads = Vec::new();
        for held in prepared {
            let Prepared { generation, lane, states: rebuilt, sought: lane_sought, rebuild_ns, ns, ahead_ns, thread } = held;
            if !threads.contains(&thread) {
                threads.push(thread);
            }
            sought.ahead_ns += ahead_ns;
            sought.restore_ns += lane_sought.restore_ns;
            sought.restore_most_ns = sought.restore_most_ns.max(lane_sought.restore_ns);
            sought.replay_ns += lane_sought.replay_ns;
            sought.replay_most_ns = sought.replay_most_ns.max(lane_sought.replay_ns);
            sought.fetch_ns += lane_sought.fetch_ns;
            sought.rebuild_ns += rebuild_ns;
            sought.rebuild_most_ns = sought.rebuild_most_ns.max(rebuild_ns);
            sought.table_most_ns = sought.table_most_ns.max(ns);
            sought.states += rebuilt.len() as u32;
            sought.state_bytes += rebuilt.iter().map(|(_, state)| state.bytes() as u64).sum::<u64>();
            let table = lane.table();
            states.extend(rebuilt.into_iter().map(|(reader, state)| Rebuilt { reader, table, state }));
            self.slots.push(Slot::new(generation, lane));
        }
        sought.ns = seek.started.elapsed().as_nanos() as u64;
        sought.waited_ns = seek.started.saturating_duration_since(seek.asked).as_nanos() as u64;
        sought.threads = threads.len() as u32;
        self.piece = seek.piece;
        let piece = (!seek.gap).then_some(seek.piece);
        let built = Built { sought: Some(sought), ..Built::default() };
        self.publish_reset(seek.hour, piece, states, built)?;
        self.report(true);
        Ok(())
    }

    /// The step of every lane's reset at `hour`, carrying the readers' states.
    fn publish_reset(&mut self, hour: Hour, piece: Option<usize>, states: Vec<Rebuilt>, mut built: Built) -> io::Result<()> {
        let mut step = self.fresh_step();
        let held = Arc::get_mut(&mut step).expect("a fresh step is held once");
        let mut buffers = std::mem::take(&mut held.tables).into_iter();
        let mut tables = Vec::with_capacity(self.slots.len());
        for slot in &self.slots {
            let mut changes = buffers.next().unwrap_or_default();
            slot.lane.reset_into(&mut changes)?;
            tables.push(changes);
        }
        self.retire_changes(buffers.collect());
        held.tables = tables;
        *lock(&held.states) = states;
        built.fetched = std::mem::take(&mut self.fetching);
        let keep = self.ends_kept();
        self.slots.iter_mut().for_each(|slot| slot.ended(hour, keep));
        (held.first, held.last, held.piece, held.reset, held.built) = (hour, hour, piece, true, built);
        self.publish(step);
        self.slots.iter_mut().for_each(|slot| slot.lane.clear_texts());
        Ok(())
    }

    /// A job's result. One a newer seek abandoned (the stepper may not have begun that seek yet), or
    /// that no lane, join or restep waits for any more, is let go before it is looked at: its error
    /// is no error of the playhead's. So is the error of a table no reader declares now, whose jobs
    /// the next pass retires.
    fn receive(&mut self, done: Done) -> io::Result<()> {
        match done {
            Done::Fetched { epoch, table, generation, result } => {
                let slot = self.slots.iter().position(|slot| slot.table == table && slot.generation == generation);
                let wanted = if self.obsolete(epoch) {
                    None
                } else if let Some(at) = slot {
                    Some(Fed::Slot(at))
                } else if self.joining.get(&table).is_some_and(|join| join.generation == generation) {
                    Some(Fed::Join)
                } else if self.restepping.as_ref().and_then(|restep| restep.ready.get(&table)).is_some_and(|(prepared, _)| prepared.generation == generation) {
                    Some(Fed::Restep)
                } else {
                    None
                };
                let Some(wanted) = wanted else {
                    if let Ok(fetched) = result {
                        self.shared.free(fetched);
                    }
                    return Ok(());
                };
                let fetched = match result {
                    Ok(fetched) => fetched,
                    Err(_) if !self.declared_now(table) => return Ok(()),
                    Err(error) => return Err(error),
                };
                let read = Fetching { jobs: 1, parts: fetched.parts() as u64, tracks: fetched.tracks as u64, stored: fetched.stored as u64, decompressed: fetched.decompressed as u64, ns: fetched.ns };
                self.fetching.add(&read);
                match wanted {
                    Fed::Slot(at) => {
                        let slot = &mut self.slots[at];
                        slot.fetching = false;
                        slot.lane.give(fetched)?;
                    }
                    Fed::Join => {
                        let join = self.joining.get_mut(&table).expect("a join waits");
                        join.fetching = false;
                        join.prepared.as_mut().expect("a join reads once prepared").lane.give(fetched)?;
                    }
                    Fed::Restep => {
                        let (prepared, fetching) = self.restepping.as_mut().and_then(|restep| restep.ready.get_mut(&table)).expect("a restep waits");
                        *fetching = false;
                        prepared.lane.give(fetched)?;
                    }
                }
            }
            Done::Prepared { epoch, purpose, table, generation, result } => {
                let wanted = !self.obsolete(epoch)
                    && match purpose {
                        Purpose::Seek => self.seek.as_ref().is_some_and(|seek| seek.pending.get(&table) == Some(&generation)),
                        Purpose::Piece(piece) => self.next_piece.as_ref().is_some_and(|next| next.piece == piece && next.pending.get(&table) == Some(&generation)),
                        Purpose::Restep => self.restepping.as_ref().is_some_and(|restep| restep.pending.get(&table) == Some(&generation)),
                        Purpose::Join => self.joining.get(&table).is_some_and(|join| join.generation == generation),
                    };
                if !wanted {
                    if matches!(purpose, Purpose::Join) && !self.obsolete(epoch) {
                        self.counters.join_answers_dropped += 1;
                    }
                    if let Ok(prepared) = result {
                        self.shared.free(vec![prepared]);
                    }
                    return Ok(());
                }
                let prepared = match result {
                    Ok(prepared) => prepared,
                    Err(_) if !self.declared_now(table) => return Ok(()),
                    Err(error) => return Err(error),
                };
                match purpose {
                    Purpose::Seek => {
                        let seek = self.seek.as_mut().expect("a seek waits");
                        seek.pending.remove(&table);
                        seek.prepared.push(*prepared);
                    }
                    Purpose::Piece(_) => {
                        let next = self.next_piece.as_mut().expect("a piece waits");
                        next.pending.remove(&table);
                        next.prepared.push(*prepared);
                    }
                    Purpose::Restep => {
                        let restep = self.restepping.as_mut().expect("a restep waits");
                        restep.pending.remove(&table);
                        restep.ready.insert(table, (prepared, false));
                    }
                    Purpose::Join => self.joining.get_mut(&table).expect("a join waits").prepared = Some(prepared),
                }
            }
        }
        Ok(())
    }

    /// Whether a job of `epoch` was abandoned: by a seek this stepper began, or by one asked since.
    fn obsolete(&self, epoch: u64) -> bool {
        epoch != self.epoch || epoch != self.shared.epoch.load(Ordering::Acquire)
    }

    /// The selection as declared now, a table no reader declares any more already retired
    /// ([`Self::retire`]). A table
    /// new to the lanes, or whose union of columns (or keys, or passing rows) changed, is joined: a
    /// lane of the union sought from the file lands for the readers that declared it anew
    /// ([`Self::land_joins`]), and the lane it replaces steps on meanwhile. A table the lanes hold
    /// as a reader now reads it is owed to that reader from memory ([`Self::land_owed`]). No step
    /// waits for either.
    /// Whether any table joins.
    fn apply_selection(&mut self) -> bool {
        self.selection_applied = true;
        if self.newest.is_none() {
            return false;
        }
        let selection = self.selection.clone();
        let mut abandoned: Vec<Box<Prepared>> = Vec::new();
        if let Some(next) = self.next_piece.take() {
            abandoned.extend(next.prepared.into_iter().map(Box::new));
        }
        let reads = |reader: &Reader, table: &usize| selection.get(table).is_some_and(|selected| selected.readings.iter().any(|reading| reading.reader == *reader));
        self.owed.retain(|reader, tables| {
            tables.retain(|table| reads(reader, table));
            !tables.is_empty()
        });
        let mut jobs = Vec::new();
        for (&table, selected) in &selection {
            let made = self.made.get(&table);
            let fresh = selected.readings.iter().filter(|reading| made.is_none_or(|made| !made.readings.iter().any(|held| held.same(reading))));
            let union = (selected.columns.as_slice(), selected.keyed, selected.passing);
            let held = match (self.joining.get(&table), self.slot_of(table)) {
                (Some(join), _) => Some((join.columns.as_slice(), join.keyed, join.passing)),
                (None, Some(at)) => Some(self.slots[at].lane.declared()),
                (None, None) => None,
            };
            if held == Some(union) {
                // A reader keeping no state of its own reads the change sets from the next step on.
                for reading in fresh.filter(|reading| reading.rebuild.is_some()) {
                    self.owed.entry(reading.reader).or_default().insert(table);
                }
                continue;
            }
            let mut readers: Vec<Reader> = if held.is_none() { selected.readings.iter().map(|reading| reading.reader).collect() } else { fresh.map(|reading| reading.reader).collect() };
            if let Some(join) = self.joining.remove(&table) {
                abandoned.extend(join.prepared);
                readers.extend(join.readers);
            }
            for (reader, tables) in &mut self.owed {
                if tables.remove(&table) {
                    readers.push(*reader);
                }
            }
            readers.sort_unstable();
            readers.dedup();
            readers.retain(|reader| selected.readings.iter().any(|reading| reading.reader == *reader));
            jobs.push(self.join(table, selected, readers));
        }
        self.owed.retain(|_, tables| !tables.is_empty());
        self.made = selection;
        self.counters.joins_open = self.joining.len() as u64;
        self.counters.declarations_applied = self.selection_seen;
        let count = jobs.len();
        let joins = count > 0;
        self.shared.queue_joins(jobs);
        self.lend_join_threads(count);
        if !abandoned.is_empty() {
            self.shared.free(abandoned);
        }
        joins
    }

    /// A table no reader declares any more is let go with every job of it, before any job's result
    /// is looked at: its lane, its join, and its place in a seek, a restep and the next piece.
    fn retire(&mut self) {
        let selection = &self.selection;
        let kept = |table: &usize| selection.contains_key(table);
        let (slots, gone): (Vec<Slot>, Vec<Slot>) = std::mem::take(&mut self.slots).into_iter().partition(|slot| kept(&slot.table));
        self.slots = slots;
        let lanes: Vec<Box<Lane>> = gone.into_iter().map(|slot| slot.lane).collect();
        let mut abandoned: Vec<Box<Prepared>> = Vec::new();
        let joins: Vec<usize> = self.joining.keys().copied().filter(|table| !kept(table)).collect();
        for table in joins {
            abandoned.extend(self.joining.remove(&table).and_then(|join| join.prepared));
        }
        self.counters.joins_open = self.joining.len() as u64;
        let mut unprepared = |pending: &mut BTreeMap<usize, u64>, prepared: &mut Vec<Prepared>| {
            pending.retain(|table, _| kept(table));
            let (held, gone): (Vec<Prepared>, Vec<Prepared>) = std::mem::take(prepared).into_iter().partition(|held| kept(&held.lane.table()));
            *prepared = held;
            abandoned.extend(gone.into_iter().map(Box::new));
        };
        if let Some(seek) = &mut self.seek {
            unprepared(&mut seek.pending, &mut seek.prepared);
        }
        if let Some(next) = &mut self.next_piece {
            unprepared(&mut next.pending, &mut next.prepared);
        }
        if let Some(restep) = &mut self.restepping {
            restep.pending.retain(|table, _| kept(table));
            let gone: Vec<usize> = restep.ready.keys().copied().filter(|table| !kept(table)).collect();
            abandoned.extend(gone.into_iter().filter_map(|table| restep.ready.remove(&table)).map(|(prepared, _)| prepared));
        }
        if !lanes.is_empty() {
            self.shared.free(lanes);
        }
        if !abandoned.is_empty() {
            self.shared.free(abandoned);
        }
    }

    /// Whether a reader declares `table` now, the stepper's selection being behind or not.
    fn declared_now(&self, table: usize) -> bool {
        lock(&self.shared.control).readers.values().any(|declaration| declaration.tables.iter().any(|declared| declared.table == table))
    }

    /// A declaration that joins no table (what it reads is stepped already, or it reads none) is
    /// shown to the hook at once, in a step of no hours after the newest: paused, no step may come.
    fn hook_declared(&mut self) {
        let (Some(newest), Some(hook)) = (self.newest, &mut self.hook) else { return };
        let piece = match self.shared.timeline.region(newest) {
            Region::Piece(piece) => Some(piece),
            Region::Gap { .. } => None,
        };
        let step = Step { epoch: self.epoch, first: newest + 1, last: newest, piece, ..Step::default() };
        hook(&step, &Lanes { slots: &self.slots });
    }

    fn slot_of(&self, table: usize) -> Option<usize> {
        self.slots.binary_search_by_key(&table, |slot| slot.table).ok()
    }

    /// A join of `table` for `readers`, recorded; its prepare, sought to the hour the frames are at
    /// in the piece stepped and read the ring's and the read-ahead's steps past it.
    fn join(&mut self, table: usize, selected: &Selected, readers: Vec<Reader>) -> Job {
        self.generation += 1;
        self.counters.joins += 1;
        let piece = self.shared.timeline.pieces[self.piece];
        let span = (self.shared.settings.ring as Hour + self.shared.settings.ahead as Hour) * self.quantum();
        let prepare = Prepare {
            epoch: self.epoch,
            purpose: Purpose::Join,
            table,
            generation: self.generation,
            run: piece.run,
            columns: selected.columns.clone(),
            keyed: selected.keyed,
            passing: selected.passing,
            hour: piece.first,
            through: piece.first + span,
            frames: Some((piece.last, span)),
            rebuilds: selected.rebuilds(Some(&readers)),
        };
        self.joining.insert(table, Join { generation: self.generation, columns: selected.columns.clone(), keyed: selected.keyed, passing: selected.passing, readers, prepared: None, fetching: false, revoke: Vec::new() });
        self.counters.joins_open = self.joining.len() as u64;
        Job::Prepare(prepare)
    }

    /// Joins' tables are prepared one a thread at once: on threads lent to them while the machine
    /// has one beside the stepper, its helpers and the fetchers (one at the least), each ending once
    /// no join is left to start. A fetcher takes one only when no read waits.
    fn lend_join_threads(&self, joins: usize) {
        let own = 1 + self.helpers.len() + self.shared.settings.fetch_threads;
        let width = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        for _ in 0..joins.min(width.saturating_sub(own).max(1)) {
            let shared = Arc::clone(&self.shared);
            let lent = std::thread::Builder::new().name("playhead join".into()).spawn(move || {
                while let Some(job) = shared.pop_join() {
                    if let Some(done) = shared.run(job) {
                        shared.nudge(|control| control.done.push(done));
                    }
                }
            });
            if lent.is_err() {
                break;
            }
        }
    }

    /// The joins under way, the ripe landing checks, the declarations applied and the answers
    /// dropped, published and announced when they moved: an answer dropped while the stepper builds
    /// nothing (a head standing, the ring full) is seen at once.
    fn publish_joins(&mut self) {
        let now = (self.joining.len() as u64, self.counters.ripe_checks, self.counters.declarations_applied, self.counters.join_answers_dropped);
        if now != self.joins_published {
            self.joins_published = now;
            self.counters.joins_open = now.0;
            lock(&self.shared.report).2 = self.counters;
            self.shared.nudge(|_| {});
        }
    }

    /// How many step ends a slot keeps: enough to see the steps past the frames.
    fn ends_kept(&self) -> usize {
        2 * self.shared.settings.ring + 4
    }

    /// Lands every join whose lane holds the parts through the newest step, at the place the frames
    /// are ([`Ring::frames`]), paused or playing; no step waits for it. The lane, sought to the hour
    /// the frames were at when its prepare started, is stepped in memory to the hour they are at
    /// now, then through each step built past them. The readers that declared it anew get, for
    /// themselves alone and before those steps, a step of no hours with its reset and their states,
    /// then its change sets of the hours the frames passed since. The steps past the frames take
    /// its change sets of their hours: a table new to the lanes, or in place of the old lane's (the
    /// same cells of the columns it held, and those added), so no other reader of a replaced table
    /// sees a reset. The new lane then steps for every reader. A replaced lane that stopped short
    /// of an hour among those steps keeps its join for a later loop.
    fn land_joins(&mut self) -> io::Result<bool> {
        let Some(newest) = self.newest else { return Ok(false) };
        if self.shared.epoch.load(Ordering::Acquire) != self.epoch || self.joining.is_empty() {
            return Ok(false);
        }
        let Some((_, hour, spans)) = lock(&self.shared.ring).plan(self.epoch) else { return Ok(false) };
        let piece = self.piece;
        let at = hour.min(self.shared.timeline.pieces[piece].last);
        // The frames go back only when the consumer furthest on leaves: a lane sought past them is sought again.
        let past: Vec<usize> = self.joining.iter().filter(|(_, join)| join.prepared.as_ref().is_some_and(|prepared| prepared.lane.head().is_some_and(|head| head > at))).map(|(&table, _)| table).collect();
        if !past.is_empty() {
            let mut jobs = Vec::with_capacity(past.len());
            for table in past {
                let join = self.joining.remove(&table).expect("joining");
                self.shared.free(join.prepared);
                if let Some(selected) = self.selection.get(&table).cloned() {
                    jobs.push(self.join(table, &selected, join.readers));
                }
            }
            let count = jobs.len();
            self.shared.queue_joins(jobs);
            self.lend_join_threads(count);
        }
        let waits = |table: usize, join: &Join| -> Option<usize> {
            let Some(prepared) = join.prepared.as_ref() else { return Some(0) };
            if join.fetching {
                return Some(1);
            }
            if !prepared.lane.holds_through(newest) {
                return Some(2);
            }
            if prepared.lane.head().is_none_or(|head| head > at) {
                return Some(3);
            }
            let old = self.slot_of(table)?;
            if !self.slots[old].whole_at(at) {
                return Some(4);
            }
            (!spans.iter().filter(|span| !span.gap).all(|span| self.slots[old].whole_at(span.last))).then_some(5)
        };
        let mut tally = [0u64; 6];
        let ripe = self.joining.values().all(|join| join.prepared.is_some() && !join.fetching);
        let able = |table: usize, join: &Join| waits(table, join).is_none();
        for (&table, join) in &self.joining {
            if let Some(why) = waits(table, join) {
                tally[why] += 1;
            }
        }
        // A reader that declared several tables at once takes them together: a join waits for every
        // join a reader of it waits for.
        let mut held: BTreeSet<usize> = self.joining.iter().filter(|&(&table, join)| !able(table, join)).map(|(&table, _)| table).collect();
        loop {
            let waiting: BTreeSet<Reader> = held.iter().flat_map(|table| self.joining[table].readers.iter().copied()).collect();
            let more: Vec<usize> = self.joining.iter().filter(|(table, join)| !held.contains(*table) && join.readers.iter().any(|reader| waiting.contains(reader))).map(|(&table, _)| table).collect();
            if more.is_empty() {
                break;
            }
            held.extend(more);
        }
        let ready: Vec<usize> = self.joining.keys().copied().filter(|table| !held.contains(table)).collect();
        for (counted, waited) in self.counters.join_waits.iter_mut().zip(tally) {
            *counted += waited;
        }
        self.counters.ripe_checks += u64::from(ripe);
        if ready.is_empty() {
            lock(&self.shared.report).2 = self.counters;
            return Ok(false);
        }
        let keep = self.ends_kept();
        let mut landed = Vec::with_capacity(ready.len());
        let mut resets = Vec::with_capacity(ready.len());
        let mut fresh = Vec::with_capacity(ready.len());
        for &table in &ready {
            let join = self.joining.remove(&table).expect("ready");
            let Prepared { lane, states, .. } = *join.prepared.expect("ready");
            let mut reset = Changes::default();
            lane.reset_into(&mut reset)?;
            resets.push(reset);
            let mut slot = Slot::new(join.generation, lane);
            if let Some(head) = slot.lane.head() {
                slot.ended(head, keep);
            }
            match self.slot_of(table) {
                Some(old) => {
                    let old = std::mem::replace(&mut self.slots[old], slot);
                    self.shared.free(vec![old.lane]);
                    fresh.push(false);
                }
                None => {
                    let place = self.slots.partition_point(|held| held.table < table);
                    self.slots.insert(place, slot);
                    fresh.push(true);
                }
            }
            landed.push((table, join.readers, states, join.revoke));
        }
        let places: Vec<usize> = ready.iter().map(|&table| self.slot_of(table).expect("landed")).collect();
        let mut view = Step { epoch: self.epoch, first: hour + 1, last: hour, piece: Some(piece), tables: resets, ..Step::default() };
        hook_fresh(&mut self.hook, &self.slots, &mut view, &fresh);
        places.iter().for_each(|&place| self.slots[place].lane.clear_texts());
        // Each round: its span's seq when it is a step built past the frames, and a change set a table.
        let mut rounds: Vec<(Option<u64>, Vec<Option<Changes>>)> = vec![(None, std::mem::take(&mut view.tables).into_iter().map(Some).collect())];
        loop {
            let behind: Vec<bool> = places.iter().map(|&place| self.slots[place].lane.head().is_some_and(|head| head < at)).collect();
            if !behind.contains(&true) {
                break;
            }
            let mut round = Vec::with_capacity(places.len());
            let mut moved = false;
            for (&place, &behind) in places.iter().zip(&behind) {
                if !behind {
                    round.push(None);
                    continue;
                }
                let before = self.slots[place].lane.head();
                let mut changes = Changes::default();
                step_slot(&mut self.slots[place], at, &mut changes)?;
                moved |= self.slots[place].lane.head() != before;
                round.push(Some(changes));
            }
            self.view_round(hour + 1, hour, &places, &fresh, &mut round);
            rounds.push((None, round));
            if !moved {
                break;
            }
        }
        for &place in &places {
            self.slots[place].ended(at, keep);
        }
        let mut end = hour;
        for span in &spans {
            let mut round = Vec::with_capacity(places.len());
            for &place in &places {
                if span.gap {
                    round.push(None);
                    continue;
                }
                let mut changes = Changes::default();
                step_slot(&mut self.slots[place], span.last, &mut changes)?;
                self.counters.stalls += u64::from(changes.stalled());
                self.slots[place].ended(span.last, keep);
                round.push(Some(changes));
            }
            self.view_round(span.first, span.last, &places, &fresh, &mut round);
            rounds.push((Some(span.seq), round));
            end = span.last;
        }
        // The hook saw a replaced lane's table through the newest step already: it sees the new lane from there.
        if fresh.contains(&false)
            && let Some(hook) = &mut self.hook
        {
            let mut view = Step { epoch: self.epoch, first: end + 1, last: end, piece: Some(piece), ..Step::default() };
            for (&place, _) in places.iter().zip(&fresh).filter(|(_, fresh)| !**fresh) {
                let mut changes = Changes::default();
                self.slots[place].lane.reset_into(&mut changes)?;
                view.tables.push(changes);
            }
            hook(&view, &Lanes { slots: &self.slots });
        }
        let mut cursors = Vec::new();
        places.iter().for_each(|&place| self.slots[place].lane.retire_into(&mut cursors));
        if !cursors.is_empty() {
            self.shared.free(cursors);
        }
        let mut ring = lock(&self.shared.ring);
        let Some((position, now)) = ring.frames().filter(|_| ring.epoch == self.epoch) else {
            drop(ring);
            self.shared.free((rounds, landed));
            self.counters.joins_open = self.joining.len() as u64;
            return Ok(true);
        };
        let mut shared = Vec::new();
        let mut own: Vec<Vec<Option<Changes>>> = Vec::new();
        for (seq, round) in rounds {
            match seq {
                Some(seq) if seq >= position => shared.push((seq, round.into_iter().flatten().collect())),
                _ => own.push(round),
            }
        }
        let mut readers: BTreeMap<Reader, Vec<usize>> = BTreeMap::new();
        for (index, (_, held, ..)) in landed.iter().enumerate() {
            for &reader in held {
                readers.entry(reader).or_default().push(index);
            }
        }
        // A table's sets go to its last reader, and a copy to each before it.
        let lasts: Vec<Option<Reader>> = landed.iter().map(|(_, held, ..)| held.iter().max().copied()).collect();
        // A landing from memory a read was made for is withdrawn as this lands; one its consumer took
        // meanwhile (its head moved on) leaves this landing unneeded for that reader.
        let revokes: Vec<Vec<Arc<Step>>> = landed.iter().map(|(.., revoke)| revoke.clone()).collect();
        let mut states: BTreeMap<Reader, Vec<Rebuilt>> = BTreeMap::new();
        for (table, _, rebuilt, _) in landed {
            for (reader, state) in rebuilt {
                states.entry(reader).or_default().push(Rebuilt { reader, table, state });
            }
        }
        for (reader, indexes) in readers {
            let mut withdrawn: Vec<&Arc<Step>> = indexes.iter().flat_map(|&index| &revokes[index]).collect();
            withdrawn.dedup_by(|a, b| Arc::ptr_eq(a, b));
            if !withdrawn.iter().all(|step| ring.holds_own(step)) {
                self.counters.join_answers_dropped += 1;
                states.remove(&reader);
                continue;
            }
            for step in withdrawn {
                ring.revoke(step);
            }
            let mut steps = Vec::with_capacity(own.len());
            for (round, sets) in own.iter_mut().enumerate() {
                let mut tables = Vec::with_capacity(indexes.len());
                for &index in &indexes {
                    let held = &mut sets[index];
                    tables.extend(if lasts[index] == Some(reader) { held.take() } else { held.clone() });
                }
                if tables.is_empty() && round > 0 {
                    continue;
                }
                let step = Step { epoch: self.epoch, seq: position, first: now + 1, last: now, piece: Some(piece), tables, ..Step::default() };
                if round == 0 {
                    *lock(&step.states) = states.remove(&reader).unwrap_or_default();
                }
                steps.push(Arc::new(step));
            }
            ring.give(reader, position, &steps);
        }
        let replaced = ring.complete(shared);
        drop(ring);
        self.retire_changes(replaced);
        self.counters.joins_at_head += ready.len() as u64;
        self.counters.joins_open = self.joining.len() as u64;
        lock(&self.shared.report).2 = self.counters;
        self.shared.nudge(|_| {});
        Ok(true)
    }

    /// A landing from memory (at the newest step built, given while the clock played) that a
    /// consumer's head stands short of: a frame has asked the head it stood at since the landing was
    /// given, and the landing lies past that head, which takes no step past it until it moves. Its
    /// tables are read for that reader at the hour the frames are at; the head does not move while
    /// the read is out, so the read lands exactly there and withdraws the landing from memory
    /// ([`Self::land_joins`]). A landing its consumer took meanwhile (the head moved on) needs no
    /// read: the read made for it is dropped, its answer let go.
    fn read_for_standing(&mut self) -> bool {
        if self.given.is_empty() && self.joining.values().all(|join| join.revoke.is_empty()) {
            return false;
        }
        let ring = lock(&self.shared.ring);
        self.given.retain(|given| ring.holds_own(&given.step));
        let unneeded: Vec<usize> = self.joining.iter().filter(|(_, join)| !join.revoke.is_empty() && !join.revoke.iter().any(|step| ring.holds_own(step))).map(|(&table, _)| table).collect();
        drop(ring);
        let mut moved = false;
        for table in unneeded {
            let join = self.joining.remove(&table).expect("joining");
            if let Some(prepared) = join.prepared {
                self.counters.join_answers_dropped += 1;
                self.shared.free(vec![prepared]);
            }
            moved = true;
        }
        let (head, stood) = (self.head, self.stood);
        let mut jobs = Vec::new();
        for at in 0..self.given.len() {
            let given = &self.given[at];
            if given.read || stood <= given.stood || given.hour <= head {
                continue;
            }
            if given.tables.iter().any(|table| self.joining.contains_key(table) || self.slot_of(*table).is_none() || !self.selection.contains_key(table)) {
                continue;
            }
            let (reader, tables, step) = (given.reader, given.tables.clone(), Arc::clone(&given.step));
            for table in tables {
                let selected = self.selection.get(&table).cloned().expect("selected");
                jobs.push(self.join(table, &selected, vec![reader]));
                self.joining.get_mut(&table).expect("joined").revoke.push(Arc::clone(&step));
            }
            self.given[at].read = true;
        }
        if !jobs.is_empty() {
            let count = jobs.len();
            self.shared.queue_joins(jobs);
            self.lend_join_threads(count);
            moved = true;
        }
        if moved {
            self.counters.joins_open = self.joining.len() as u64;
        }
        moved
    }

    /// What is owed, joined for its readers: the clock is held short of the newest step.
    fn join_owed(&mut self) -> bool {
        let mut wanted: BTreeMap<usize, Vec<Reader>> = BTreeMap::new();
        for (reader, tables) in std::mem::take(&mut self.owed) {
            for table in tables {
                if self.joining.contains_key(&table) {
                    self.owed.entry(reader).or_default().insert(table);
                } else if self.slot_of(table).is_some() {
                    wanted.entry(table).or_default().push(reader);
                }
            }
        }
        let asked: Vec<(usize, Option<Reader>)> = {
            let ring = lock(&self.shared.ring);
            std::mem::take(&mut self.lands).into_iter().map(|id| (id, ring.reader_of(id))).collect()
        };
        for (id, reader) in asked {
            let Some(reader) = reader else { continue };
            let tables: Vec<usize> = self.selection.iter().filter(|(_, selected)| selected.readings.iter().any(|reading| reading.reader == reader)).map(|(&table, _)| table).collect();
            if tables.iter().any(|table| self.joining.contains_key(table)) {
                self.lands.push(id);
                continue;
            }
            for table in tables.into_iter().filter(|&table| self.slot_of(table).is_some()) {
                wanted.entry(table).or_default().push(reader);
            }
        }
        let mut jobs = Vec::with_capacity(wanted.len());
        for (table, mut readers) in wanted {
            let Some(selected) = self.selection.get(&table).cloned() else { continue };
            readers.sort_unstable();
            readers.dedup();
            jobs.push(self.join(table, &selected, readers));
        }
        let count = jobs.len();
        self.shared.queue_joins(jobs);
        self.lend_join_threads(count);
        count > 0
    }

    /// When the pace asks smaller steps than those built past the hour the frames have taken, they
    /// are built again at its size. No consumer took them; the steps a consumer took are never
    /// touched. The lanes stand at the newest step built and step only forward, and the change
    /// sets in the ring carry no old cells, so the state at the frames' hour is not in memory:
    /// every stepping table is sought again from the file to the frames' hour, as a seek's tables
    /// are, but on threads lent to it and after the reads the steps wait on, while the old steps
    /// serve the frames. Once all are sought, each lane is stepped in memory to where the step
    /// ending at the frames' hour left its table; the steps past the frames leave the ring; the new
    /// lanes replace the old, holding the same cells; and the stepper builds on from there at the
    /// pace's size, each step marked ([`Step::restep`]) so a consumer knows the steps lent it ahead
    /// before are void. A landing placed past the frames is landed again; the hook sees no hour
    /// twice. A pace asking larger steps keeps the smaller ones built.
    fn restep_ahead(&mut self) -> io::Result<bool> {
        if self.newest.is_none() || self.shared.epoch.load(Ordering::Acquire) != self.epoch {
            return Ok(false);
        }
        let quantum = self.quantum();
        if let Some(restep) = self.restepping.take_if(|restep| restep.quantum != quantum) {
            self.shared.free(restep.ready.into_values().map(|(prepared, _)| prepared).collect::<Vec<_>>());
        }
        self.coarse |= quantum < self.paced;
        let Some(restep) = &self.restepping else {
            if !self.coarse {
                return Ok(false);
            }
            let Some((_, _, spans)) = lock(&self.shared.ring).plan(self.epoch) else { return Ok(false) };
            if !spans.iter().any(|span| !span.gap && span.last - span.first + 1 > quantum) {
                self.coarse = false;
                return Ok(false);
            }
            let piece = self.shared.timeline.pieces[self.piece];
            let span = (self.shared.settings.ring as Hour + self.shared.settings.ahead as Hour) * quantum;
            let (mut pending, mut jobs) = (BTreeMap::new(), Vec::with_capacity(self.slots.len()));
            for slot in &self.slots {
                self.generation += 1;
                pending.insert(slot.table, self.generation);
                let lane = &slot.lane;
                jobs.push(Job::Prepare(Prepare {
                    epoch: self.epoch,
                    purpose: Purpose::Restep,
                    table: slot.table,
                    generation: self.generation,
                    run: piece.run,
                    columns: lane.columns().to_vec(),
                    keyed: lane.keyed(),
                    passing: lane.passing(),
                    hour: piece.first,
                    through: piece.first + span,
                    frames: Some((piece.last, span)),
                    rebuilds: Vec::new(),
                }));
            }
            let count = jobs.len();
            self.shared.queue_joins(jobs);
            self.lend_join_threads(count);
            self.restepping = Some(Restep { quantum, pending, ready: BTreeMap::new() });
            return Ok(true);
        };
        if !restep.pending.is_empty() {
            return Ok(false);
        }
        let Some((position, hour, _)) = lock(&self.shared.ring).plan(self.epoch) else { return Ok(false) };
        let at = hour.min(self.shared.timeline.pieces[self.piece].last);
        // A table joined since, a step end not recorded, or a lane sought past it: sought again.
        let mut heads = Vec::with_capacity(self.slots.len());
        for slot in &self.slots {
            let restep = self.restepping.as_ref().expect("restepping");
            let fits = match (restep.ready.get(&slot.table), slot.head_at(at)) {
                (Some((prepared, _)), Some(head)) if prepared.lane.head().is_some_and(|sought| sought <= head) && prepared.lane.declared() == slot.lane.declared() => Some(head),
                _ => None,
            };
            let Some(head) = fits else {
                let restep = self.restepping.take().expect("restepping");
                self.shared.free(restep.ready.into_values().map(|(prepared, _)| prepared).collect::<Vec<_>>());
                return Ok(true);
            };
            heads.push(head);
        }
        let restep = self.restepping.as_mut().expect("restepping");
        let mut cursors = Vec::new();
        let mut waiting = false;
        for (slot, &head) in self.slots.iter().zip(&heads) {
            let (prepared, fetching) = restep.ready.get_mut(&slot.table).expect("ready");
            let lane = &mut prepared.lane;
            while lane.head().is_some_and(|reached| reached < head) {
                if *fetching || !lane.holds_through(head) {
                    waiting = true;
                    break;
                }
                let before = lane.head();
                let mut passed = Changes::default();
                lane.step(head, &mut passed)?;
                lane.retire_into(&mut cursors);
                if lane.head() == before {
                    waiting = true;
                    break;
                }
            }
        }
        if !cursors.is_empty() {
            self.shared.free(cursors);
        }
        if waiting {
            return Ok(false);
        }
        let mut ring = lock(&self.shared.ring);
        if ring.epoch != self.epoch || ring.frames() != Some((position, hour)) {
            return Ok(true);
        }
        let (owed, landing) = ring.cut(position);
        drop(ring);
        let mut ready = self.restepping.take().expect("restepping").ready;
        let keep = self.ends_kept();
        let mut old = Vec::with_capacity(self.slots.len());
        for slot in &mut self.slots {
            let (prepared, _) = ready.remove(&slot.table).expect("ready");
            let Prepared { generation, mut lane, .. } = *prepared;
            lane.clear_texts();
            old.push(std::mem::replace(&mut slot.lane, lane));
            (slot.generation, slot.fetching) = (generation, false);
            slot.ends.clear();
            slot.ended(at, keep);
        }
        self.shared.free((old, ready.into_values().map(|(prepared, _)| prepared).collect::<Vec<_>>()));
        let newest = self.newest.expect("stepping");
        self.hidden = Some(newest).filter(|&newest| newest > hour).max(self.hidden);
        self.newest = Some(hour);
        self.restep += 1;
        self.coarse = false;
        self.counters.resteps += 1;
        for (reader, tables) in owed {
            self.owed.entry(reader).or_default().extend(tables);
        }
        self.lands.extend(landing);
        lock(&self.shared.report).2 = self.counters;
        self.shared.nudge(|control| control.newest = Some(hour));
        Ok(true)
    }

    /// The hook over the change sets of one round of landed lanes (`places`), the tables new to the
    /// lanes alone; each set goes back to its place in `round`.
    fn view_round(&mut self, first: Hour, last: Hour, places: &[usize], fresh: &[bool], round: &mut [Option<Changes>]) {
        if self.hook.is_none() || !fresh.iter().zip(round.iter()).any(|(&fresh, held)| fresh && held.is_some()) {
            return;
        }
        let shown: Vec<usize> = (0..places.len()).filter(|&index| round[index].is_some()).collect();
        let mut view = Step { epoch: self.epoch, first, last, piece: Some(self.piece), tables: shown.iter().map(|&index| round[index].take().expect("shown")).collect(), ..Step::default() };
        let mask: Vec<bool> = shown.iter().map(|&index| fresh[index]).collect();
        hook_fresh(&mut self.hook, &self.slots, &mut view, &mask);
        for (index, changes) in shown.into_iter().zip(view.tables) {
            round[index] = Some(changes);
        }
    }

    /// Lands what is owed: each table a reader declared anew that the lanes hold as it reads them,
    /// for the reader's consumers alone; and each consumer that asked to land, every table its
    /// reader declares once none is still joining. With the clock running, each comes from memory
    /// in a step of no hours at the newest step's end, taken after it: a reset of each table with
    /// the reader's state rebuilt from the lane there, shown every string the lane named since its
    /// seek. Nothing is read from the file, no other consumer's step changes, and the stepper holds
    /// nothing for it. With the clock held (paused, or at a seek) where the frames stand short of
    /// the newest step, they would never reach it: each table is joined instead
    /// ([`Self::land_joins`]), from the file at the frames' hour, for those readers alone.
    fn land_owed(&mut self) -> io::Result<bool> {
        let Some(newest) = self.newest else { return Ok(false) };
        if (self.owed.is_empty() && self.lands.is_empty()) || self.shared.epoch.load(Ordering::Acquire) != self.epoch {
            return Ok(false);
        }
        let last = self.shared.timeline.pieces[self.piece].last;
        let held = {
            let clock = lock(&self.shared.clock);
            !clock.playing() || clock.held().is_some()
        };
        if held && lock(&self.shared.ring).frames().is_some_and(|(_, hour)| hour.min(last) < newest.min(last)) {
            return Ok(self.join_owed());
        }
        let mut targets: Vec<(Option<usize>, Reader, Vec<usize>)> = Vec::new();
        for (reader, tables) in std::mem::take(&mut self.owed) {
            let (now, later): (BTreeSet<usize>, BTreeSet<usize>) = tables.into_iter().partition(|table| !self.joining.contains_key(table));
            if !later.is_empty() {
                self.owed.insert(reader, later);
            }
            let now: Vec<usize> = now.into_iter().filter(|&table| self.slot_of(table).is_some()).collect();
            if !now.is_empty() {
                targets.push((None, reader, now));
            }
        }
        let asked: Vec<(usize, Option<Reader>)> = {
            let ring = lock(&self.shared.ring);
            self.lands.iter().map(|&id| (id, ring.reader_of(id))).collect()
        };
        self.lands.clear();
        for (id, reader) in asked {
            let Some(reader) = reader else { continue };
            let tables: Vec<usize> = self.selection.iter().filter(|(_, selected)| selected.readings.iter().any(|reading| reading.reader == reader)).map(|(&table, _)| table).collect();
            if tables.iter().any(|table| self.joining.contains_key(table)) {
                self.lands.push(id);
                continue;
            }
            targets.push((Some(id), reader, tables.into_iter().filter(|&table| self.slot_of(table).is_some()).collect()));
        }
        if targets.is_empty() {
            return Ok(false);
        }
        let mut work: Vec<(usize, usize, Reader, Vec<usize>, Rebuild)> = Vec::new();
        for (target, (_, reader, tables)) in targets.iter().enumerate() {
            for &table in tables {
                let reading = self.selection.get(&table).and_then(|selected| selected.readings.iter().find(|reading| reading.reader == *reader));
                if let Some(reading) = reading
                    && let Some(rebuild) = &reading.rebuild
                {
                    work.push((target, self.slot_of(table).expect("held"), *reader, reading.columns.clone(), Arc::clone(rebuild)));
                }
            }
        }
        let mut shown: Vec<usize> = work.iter().map(|(_, place, ..)| *place).collect();
        shown.sort_unstable();
        shown.dedup();
        let pending: Vec<Texts> = shown.iter().map(|&place| self.slots[place].lane.show_known()).collect();
        let started = Instant::now();
        let rebuilt = rebuild_all(&mut self.slots, &work);
        self.counters.landing_rebuild_ns += started.elapsed().as_nanos() as u64;
        for (&place, texts) in shown.iter().zip(pending) {
            self.slots[place].lane.hide_known(texts);
        }
        let mut states: Vec<Vec<Rebuilt>> = (0..targets.len()).map(|_| Vec::new()).collect();
        for ((target, place, reader, ..), state) in work.iter().zip(rebuilt) {
            let mut state = state?;
            state.hold(&self.shared.flight);
            states[*target].push(Rebuilt { reader: *reader, table: self.slots[*place].table, state });
        }
        let mut steps = Vec::with_capacity(targets.len());
        for ((consumer, reader, tables), states) in targets.into_iter().zip(states) {
            let mut sets = Vec::with_capacity(tables.len());
            for &table in &tables {
                let mut changes = Changes::default();
                self.slots[self.slot_of(table).expect("held")].lane.reset_into(&mut changes)?;
                sets.push(changes);
            }
            let step = Step { epoch: self.epoch, first: newest + 1, last: newest, piece: Some(self.piece), tables: sets, states: Mutex::new(states), ..Step::default() };
            steps.push((consumer, reader, step));
        }
        let mut ring = lock(&self.shared.ring);
        if ring.epoch != self.epoch {
            drop(ring);
            self.shared.free(steps.into_iter().map(|(_, _, step)| step).collect::<Vec<_>>());
            return Ok(true);
        }
        let before = ring.next_seq();
        self.counters.landings += steps.len() as u64;
        for (consumer, reader, mut step) in steps {
            step.seq = before;
            match consumer {
                Some(id) => ring.jump(id, Arc::new(step)),
                None => {
                    let tables = step.tables.iter().map(Changes::table).collect();
                    let step = Arc::new(step);
                    ring.give(reader, before, &[Arc::clone(&step)]);
                    self.given.push(Given { reader, tables, step, hour: newest, stood: self.stood, read: false });
                }
            }
        }
        drop(ring);
        lock(&self.shared.report).2 = self.counters;
        self.shared.nudge(|_| {});
        Ok(true)
    }

    /// Reads for every lane that needs parts, a join's prepared lane among them: those covering
    /// `ahead` steps past the newest built, and at least the next part.
    fn schedule(&mut self) {
        let Some(newest) = self.newest else { return };
        let through = newest + self.shared.settings.ahead as Hour * self.quantum();
        let mut jobs = Vec::new();
        let slots = self.slots.iter_mut().map(|slot| (slot.table, slot.generation, &mut slot.fetching, &mut slot.lane));
        let joins = self.joining.iter_mut().filter_map(|(&table, join)| join.prepared.as_mut().map(|prepared| (table, join.generation, &mut join.fetching, &mut prepared.lane)));
        let resteps = self.restepping.iter_mut().flat_map(|restep| restep.ready.iter_mut()).map(|(&table, (prepared, fetching))| (table, prepared.generation, fetching, &mut prepared.lane));
        for (table, generation, fetching, lane) in slots.chain(joins).chain(resteps) {
            if *fetching {
                continue;
            }
            if let Some(fetch) = lane.want(through) {
                *fetching = true;
                let first = i64::from(fetch.first_hour());
                jobs.push(((2, first), Job::Fetch { epoch: self.epoch, table, generation, fetch }));
            }
        }
        self.shared.queue(jobs);
    }

    /// Prepares the next piece's lanes once the newest step is near its piece's end.
    fn prepare_next_piece(&mut self) {
        let (Some(newest), None) = (self.newest, &self.next_piece) else { return };
        let pieces = &self.shared.timeline.pieces;
        let next = self.piece + 1;
        if next >= pieces.len() {
            return;
        }
        let quantum = self.quantum();
        let lead = (quantum * (self.shared.settings.ring as Hour + self.shared.settings.ahead as Hour)).max((self.pace.0 * PREPARE_SECONDS) as Hour);
        if pieces[self.piece].last - newest > lead {
            return;
        }
        let (run, hour) = (pieces[next].run, pieces[next].first);
        let through = hour + self.shared.settings.ahead as Hour * quantum;
        let held_run = &self.shared.pages.runs()[run];
        let mut pending = BTreeMap::new();
        let mut jobs = Vec::new();
        for (&table, selected) in &self.selection {
            self.generation += 1;
            pending.insert(table, self.generation);
            let prepare = Prepare {
                epoch: self.epoch,
                purpose: Purpose::Piece(next),
                table,
                generation: self.generation,
                run,
                columns: selected.columns.clone(),
                keyed: selected.keyed,
                passing: selected.passing,
                hour,
                through,
                frames: None,
                rebuilds: selected.rebuilds(None),
            };
            jobs.push(((1, -seek_cost(held_run, table, hour)), Job::Prepare(prepare)));
        }
        self.shared.queue(jobs);
        self.next_piece = Some(NextPiece { piece: next, pending, prepared: Vec::new() });
    }

    /// A step no consumer holds, its buffers kept, or a new one; states left in it are freed.
    fn fresh_step(&self) -> Arc<Step> {
        let (step, spare) = lock(&self.shared.ring).reclaim();
        if !spare.is_empty() {
            self.shared.free(spare);
        }
        let mut step = step.unwrap_or_default();
        let held = Arc::get_mut(&mut step).expect("a reclaimed step is held once");
        let left = std::mem::take(&mut *lock(&held.states));
        if !left.is_empty() {
            self.shared.free(left);
        }
        held.epoch = self.epoch;
        held.restep = self.restep;
        held.built = Built::default();
        step
    }

    fn retire_changes(&self, changes: Vec<Changes>) {
        if !changes.is_empty() {
            self.shared.free(changes);
        }
    }

    fn publish(&mut self, step: Arc<Step>) {
        // The hook saw the hours of the steps a new pace rebuilt already: it sees the steps after them.
        if let Some(hook) = &mut self.hook
            && self.hidden.is_none_or(|hidden| step.last > hidden)
        {
            hook(&step, &Lanes { slots: &self.slots });
        }
        if self.hidden.is_some_and(|hidden| step.last >= hidden) {
            self.hidden = None;
        }
        let last = step.last;
        let held = lock(&self.shared.ring).publish(step);
        match held {
            Ok(()) => {
                self.release();
                self.newest = Some(last);
                self.counters.steps += 1;
                lock(&self.shared.report).2 = self.counters;
                self.shared.nudge(|control| control.newest = Some(last));
            }
            Err(step) => self.shared.free(step),
        }
    }

    /// Builds the next step when the ring has room: hours after the newest step through one step
    /// past the newest (or through the head, when the head has run past it, so a frame held there
    /// takes it), within one piece and one block, or a gap. A table behind the newest step whose
    /// parts have come is first caught up by a step of no new hours, which a frame at the newest
    /// hour takes (a paused head, the timeline's end); while one waits for a read and no frame
    /// waits for a step, none is built.
    fn build(&mut self) -> io::Result<bool> {
        let Some(newest) = self.newest else { return Ok(false) };
        if !lock(&self.shared.ring).room(self.head) {
            self.why = if newest < self.head { IDLE_ROOM_BEHIND } else { IDLE_RING_FULL };
            self.hold();
            return Ok(false);
        }
        let timeline = &self.shared.timeline;
        if timeline.region(newest) == Region::Piece(self.piece) {
            let behind = |slot: &&Slot| slot.lane.head().is_some_and(|head| head < newest);
            let (mut waiting, mut ready) = (false, false);
            for slot in self.slots.iter().filter(behind) {
                if slot.fetching { waiting = true } else { ready = true }
            }
            if ready {
                self.counters.catch_ups += 1;
                self.step_to(newest + 1, newest, self.piece)?;
                return Ok(true);
            }
            if waiting && self.head <= newest {
                self.why = IDLE_FETCH;
                return Ok(false);
            }
        }
        if newest >= timeline.last() {
            self.why = IDLE_END;
            return Ok(false);
        }
        if newest >= self.head && self.shared.exact.load(Ordering::Acquire) && !lock(&self.shared.clock).playing() {
            return Ok(false);
        }
        let quantum = self.quantum();
        self.paced = quantum;
        let start = newest + 1;
        let target = if newest < self.head { self.head } else { newest + quantum };
        // Rebuilt for a new pace, a step ends at the newest hour the hook saw, so it sees none twice.
        let target = self.hidden.filter(|&hidden| hidden > newest).map_or(target, |hidden| target.min(hidden));
        match timeline.region(start) {
            Region::Gap { next } => {
                let end = target.min(timeline.pieces[next].first - 1);
                let mut step = self.fresh_step();
                let held = Arc::get_mut(&mut step).expect("a fresh step is held once");
                self.retire_changes(std::mem::take(&mut held.tables));
                (held.first, held.last, held.piece, held.reset) = (start, end, None, false);
                held.built.fetched = std::mem::take(&mut self.fetching);
                self.publish(step);
                Ok(true)
            }
            Region::Piece(piece) if piece != self.piece => self.enter_piece(piece),
            Region::Piece(piece) => {
                let held = timeline.pieces[piece];
                let run = &self.shared.pages.runs()[held.run];
                let block_end = run.first_hour + (((start - run.first_hour) as u32 / BLOCK + 1) * BLOCK) as Hour - 1;
                let end = target.min(held.last).min(block_end);
                self.step_to(start, end, piece)?;
                Ok(true)
            }
        }
    }

    /// At a piece's first hour: its prepared lanes replace the last piece's, and the step is a
    /// reset of every table with every reader's state; when they are not ready, nothing is built.
    fn enter_piece(&mut self, piece: usize) -> io::Result<bool> {
        let ready = self.next_piece.as_ref().is_some_and(|next| next.piece == piece && next.pending.is_empty());
        if !ready {
            if !self.waiting_piece {
                self.waiting_piece = true;
                self.counters.piece_waits += 1;
            }
            return Ok(false);
        }
        self.waiting_piece = false;
        let next = self.next_piece.take().expect("ready");
        let lanes: Vec<Box<Lane>> = std::mem::take(&mut self.slots).into_iter().map(|slot| slot.lane).collect();
        if !lanes.is_empty() {
            self.shared.free(lanes);
        }
        let abandoned = self.abandon_prepared();
        if !abandoned.is_empty() {
            self.shared.free(abandoned);
        }
        // The piece's reset brings every reader its state of every table.
        self.owed.clear();
        let mut prepared = next.prepared;
        prepared.sort_by_key(|held| held.lane.table());
        let mut states = Vec::new();
        for held in prepared {
            let Prepared { generation, lane, states: rebuilt, .. } = held;
            let table = lane.table();
            states.extend(rebuilt.into_iter().map(|(reader, state)| Rebuilt { reader, table, state }));
            self.slots.push(Slot::new(generation, lane));
        }
        self.piece = piece;
        let hour = self.shared.timeline.pieces[piece].first;
        self.publish_reset(hour, Some(piece), states, Built::default())?;
        Ok(true)
    }

    /// Every lane stepped to `end`.
    fn step_to(&mut self, start: Hour, end: Hour, piece: usize) -> io::Result<()> {
        let started = Instant::now();
        let mut step = self.fresh_step();
        let held = Arc::get_mut(&mut step).expect("a fresh step is held once");
        let mut buffers = std::mem::take(&mut held.tables);
        let mut changes: Vec<Changes> = Vec::with_capacity(self.slots.len());
        for at in 0..self.slots.len() {
            changes.push(buffers.get_mut(at).map(std::mem::take).unwrap_or_default());
        }
        self.retire_changes(buffers.into_iter().filter(|changes| changes.bytes_held() > 0).collect());
        let threads = self.step_lanes(end, &mut changes)?;
        let mut cursors = Vec::new();
        let keep = self.ends_kept();
        for slot in &mut self.slots {
            slot.lane.retire_into(&mut cursors);
            slot.ended(end, keep);
        }
        if !cursors.is_empty() {
            self.shared.free(cursors);
        }
        let mut built = Built { threads, ..Built::default() };
        for table in &changes {
            built.cells += table.cells() as u64;
            built.rows += table.rows() as u64;
            built.bytes += table.payload() as u64;
            built.stalled += u32::from(table.stalled());
            if table.short() > 0 {
                built.short += 1;
                built.most_short = built.most_short.max(table.short());
            }
        }
        self.counters.stalls += u64::from(built.stalled);
        self.counters.stalled_steps += u64::from(built.stalled > 0);
        built.fetched = std::mem::take(&mut self.fetching);
        held.tables = changes;
        (held.first, held.last, held.piece, held.reset) = (start, end, Some(piece), false);
        built.ns = started.elapsed().as_nanos() as u64;
        held.built = built;
        self.publish(step);
        Ok(())
    }

    /// Steps every lane, split over the stepper's threads when the step is predicted large; the
    /// threads that stepped.
    fn step_lanes(&mut self, end: Hour, changes: &mut [Changes]) -> io::Result<u32> {
        let predicted = |slot: &Slot| slot.cost * f64::from((end - slot.lane.head().unwrap_or(end)).max(0) as u32);
        let total: f64 = self.slots.iter().map(predicted).sum();
        if self.helpers.is_empty() || total < SPLIT_NS {
            for (slot, changes) in self.slots.iter_mut().zip(changes.iter_mut()) {
                step_slot(slot, end, changes)?;
            }
            return Ok(1);
        }
        self.counters.split_steps += 1;
        let mut items = Vec::with_capacity(self.slots.len());
        for (index, (slot, buffer)) in std::mem::take(&mut self.slots).into_iter().zip(changes.iter_mut()).enumerate() {
            items.push(Item { index, predicted: predicted(&slot), slot, changes: std::mem::take(buffer), result: Ok(()) });
        }
        items.sort_by(|a, b| a.predicted.total_cmp(&b.predicted));
        let wanted = ((total / SPLIT_NS).ceil() as usize).clamp(2, self.helpers.len() + 1);
        let batch = Arc::new(Batch { to: end, left: AtomicUsize::new(items.len()), queue: Mutex::new(items), done: Mutex::new(Vec::new()), finished: Condvar::new(), threads: AtomicUsize::new(0) });
        for helper in self.helpers.iter().take(wanted - 1) {
            let _ = helper.send(Help::Batch(Arc::clone(&batch)));
        }
        batch.work();
        batch.wait();
        let threads = batch.threads.load(Ordering::Relaxed) as u32;
        let mut done = std::mem::take(&mut *lock(&batch.done));
        done.sort_by_key(|item| item.index);
        let mut slots = Vec::with_capacity(done.len());
        let mut failed = None;
        for item in done {
            if let Err(error) = item.result {
                failed.get_or_insert(error);
            }
            changes[item.index] = item.changes;
            slots.push(item.slot);
        }
        self.slots = slots;
        match failed {
            Some(error) => Err(error),
            None => Ok(threads),
        }
    }

    /// Sums what is held, at most every [`REPORT_EVERY`] unless `force`.
    fn report(&mut self, force: bool) {
        if !force && self.report_at.elapsed() < REPORT_EVERY {
            return;
        }
        self.report_at = Instant::now();
        let mut report = Report { lanes: self.slots.len(), ..Report::default() };
        for slot in &self.slots {
            report.held.add(&slot.lane.held());
        }
        let steps = lock(&self.shared.ring).steps();
        report.ring_steps = steps.len();
        report.ring_bytes = steps.iter().map(|step| step.bytes_held()).sum();
        drop(steps);
        {
            let jobs = lock(&self.shared.jobs);
            report.fetches_queued = jobs.heap.iter().filter(|queued| queued.rank.0 == 2).count();
            report.jobs_running = jobs.running;
        }
        report.freed_ns = self.shared.freed_ns.load(Ordering::Relaxed);
        let preparing = self.joining.values().filter_map(|join| join.prepared.as_deref()).chain(self.next_piece.iter().flat_map(|next| next.prepared.iter())).chain(self.seek.iter().flat_map(|seek| seek.prepared.iter()));
        for prepared in preparing {
            report.preparing += 1;
            report.preparing_held.add(&prepared.lane.held());
        }
        let ((states, state_bytes), (peak_states, peak_state_bytes)) = self.shared.flight.read();
        (report.states, report.state_bytes) = (states, state_bytes);
        report.total = report.held.total() + report.ring_bytes + report.preparing_held.total() + report.state_bytes;
        if report.total > self.peak.total {
            self.peak = report;
        }
        (self.peak.states, self.peak.state_bytes) = (peak_states, peak_state_bytes);
        *lock(&self.shared.report) = (report, self.peak, self.counters);
    }
}

/// Runs each rebuild of `work` (a target, a slot's place, the reader, its columns, its rebuild) on
/// the slot's lane: the slots go to as many threads as the machine has, one slot to one thread at a
/// time, and come back in place. The states in the order of `work`.
fn rebuild_all(slots: &mut Vec<Slot>, work: &[(usize, usize, Reader, Vec<usize>, Rebuild)]) -> Vec<io::Result<State>> {
    let mut by_place: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (index, (_, place, ..)) in work.iter().enumerate() {
        by_place.entry(*place).or_default().push(index);
    }
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get).min(by_place.len());
    let mut held: Vec<Option<Slot>> = std::mem::take(slots).into_iter().map(Some).collect();
    let queue: Mutex<Vec<(usize, Slot, Vec<usize>)>> = Mutex::new(by_place.into_iter().map(|(place, indexes)| (place, held[place].take().expect("a slot"), indexes)).collect());
    let done: Mutex<Vec<(usize, Slot, Vec<(usize, io::Result<State>)>)>> = Mutex::new(Vec::new());
    let work_on = || {
        loop {
            let Some((place, slot, indexes)) = lock(&queue).pop() else { break };
            let states = indexes.into_iter().map(|index| (index, (work[index].4)(&slot.lane, &work[index].3))).collect();
            lock(&done).push((place, slot, states));
        }
    };
    std::thread::scope(|scope| {
        for _ in 1..threads {
            scope.spawn(&work_on);
        }
        work_on();
    });
    let mut states: Vec<Option<io::Result<State>>> = (0..work.len()).map(|_| None).collect();
    for (place, slot, rebuilt) in done.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner) {
        held[place] = Some(slot);
        for (index, state) in rebuilt {
            states[index] = Some(state);
        }
    }
    *slots = held.into_iter().map(|slot| slot.expect("given back")).collect();
    states.into_iter().map(|state| state.expect("rebuilt")).collect()
}

/// The hook over `step` holding only the change sets of the tables new to the lanes (`fresh`, by
/// place); the others are set aside meanwhile and put back in place.
fn hook_fresh(hook: &mut Option<Hook>, slots: &[Slot], step: &mut Step, fresh: &[bool]) {
    let Some(hook) = hook else { return };
    if !fresh.contains(&true) {
        return;
    }
    if !fresh.contains(&false) {
        hook(step, &Lanes { slots });
        return;
    }
    let mut aside = Vec::new();
    for (changes, &new) in std::mem::take(&mut step.tables).into_iter().zip(fresh) {
        if new { step.tables.push(changes) } else { aside.push(changes) }
    }
    hook(step, &Lanes { slots });
    let (mut shown, mut aside) = (std::mem::take(&mut step.tables).into_iter(), aside.into_iter());
    step.tables = fresh.iter().map(|&new| if new { shown.next() } else { aside.next() }.expect("one change set a table")).collect();
}

fn union(readers: Vec<(Reader, Declaration)>) -> BTreeMap<usize, Selected> {
    let mut selection: BTreeMap<usize, Selected> = BTreeMap::new();
    for (reader, declaration) in readers {
        for (held, since) in declaration.tables.into_iter().zip(declaration.since) {
            let entry = selection.entry(held.table).or_insert_with(|| Selected { columns: Vec::new(), keyed: false, passing: false, readings: Vec::new() });
            entry.columns.extend_from_slice(&held.columns);
            entry.keyed |= held.keyed;
            entry.passing |= held.passing;
            let mut columns = held.columns;
            columns.sort_unstable();
            columns.dedup();
            entry.readings.push(Reading { reader, columns, keyed: held.keyed, passing: held.passing, since, rebuild: declaration.rebuild.clone() });
        }
    }
    for selected in selection.values_mut() {
        selected.columns.sort_unstable();
        selected.columns.dedup();
    }
    selection
}
