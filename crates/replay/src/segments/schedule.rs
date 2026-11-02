//! Which segment each family walks or emits next, what a run may start, and what became of
//! each family.

use super::{
    Ended, Ending, Failure, Outcome, Settings, Stats, Waste, invalid,
    lane::{Credit, Reserved, Taken},
};
use crate::columnar_frames::{Reads, Recording, Segment};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
};

/// A segment's place in its branch.
#[derive(Clone, Copy)]
pub(super) struct Planned {
    pub(super) branch: u64,
    /// Its first hour.
    pub(super) first: i32,
    /// The segment before it in its branch, and after: the run's next segment, else the
    /// first of the branch's next run.
    pub(super) before: Option<usize>,
    pub(super) after: Option<usize>,
}

pub(super) struct Plan(Box<[Planned]>);

impl Plan {
    pub(super) fn new(recording: &Recording, segments: &[Segment]) -> io::Result<Self> {
        let mut planned: Vec<Planned> = engine_allocation::vec(segments.len())?;
        // Each branch's latest segment so far.
        let mut latest: HashMap<u64, usize> = HashMap::new();
        for (at, segment) in segments.iter().enumerate() {
            let run = recording.run(segment.run)?;
            let first = i32::try_from(i64::from(run.first_hour) + i64::from(segment.first))
                .map_err(|_| invalid("A segment starts past the hours"))?;
            let before = latest.insert(run.branch, at);
            if let Some(before) = before {
                planned[before].after = Some(at);
            }
            planned.push(Planned {
                branch: run.branch,
                first,
                before,
                after: None,
            });
        }
        Ok(Self(engine_allocation::boxed_slice(planned)?))
    }

    pub(super) fn get(&self, at: usize) -> &Planned {
        &self.0[at]
    }

    /// The first segment of `at`'s branch.
    fn leading(&self, mut at: usize) -> usize {
        while let Some(before) = self.0[at].before {
            at = before;
        }
        at
    }
}

/// A family's progress through a segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Status {
    /// No walk has taken it, or one gave it back not knowing whether it can start there.
    Open,
    /// It cannot start there, and waits for the segment before to hand its operator on, or for
    /// the operator parked for it to be written out.
    Waiting,
    Running,
    /// Its items wait for its emit, or were emitted.
    Sealed,
    /// Its items of the hours before it refused its branch, and the refusal, wait for its
    /// emit, or were emitted. It hands nothing on.
    Refused,
    /// Its loss waits for its emit, or was emitted. It hands nothing on.
    Lost,
    /// Closed while a walk holds it, which gives it back.
    Abandoned,
    /// Nothing of it reaches its emit: its branch ended before it, or its family failed.
    Closed,
}

/// What walking a segment reads, as far as a measure has said.
#[derive(Clone, Copy)]
enum Measured {
    Unknown,
    Measuring,
    Known(Reads),
}

/// The cells of a family a closure covers.
#[derive(Clone, Copy)]
pub(super) enum Closing {
    /// Those after a segment in its branch.
    Branch(usize),
    /// A segment's, and those after it in its branch.
    Rest(usize),
    /// Every one from a segment on, of every branch.
    From(usize),
    /// Those before a segment in its branch.
    Before(usize),
}

/// What steps of the schedule let go of, dropped only once it is let go: the cells closures
/// covered, each by family and segment with whether it filed items its emit will not take,
/// and what families made that they gave up ([`Waste`]).
#[derive(Default)]
pub(super) struct Gone {
    pub(super) cells: Vec<(usize, usize, bool)>,
    pub(super) waste: Vec<Waste>,
}

/// What became of families a walk could not start where it began.
#[derive(Default)]
pub(super) struct Deferred {
    /// Those the segment before has handed their operator on since, for the walk to take.
    pub(super) ready: Vec<usize>,
    /// Those whose operator the segment before lost, and so lost too, with that segment.
    pub(super) lost: Vec<(usize, usize)>,
}

pub(super) enum Task<'c> {
    /// Measures what walking segment `at` reads.
    Measure { at: usize },
    /// Emits `lane`'s chunks of segment `at`, reading the first back into `restore` when
    /// it spilled.
    Emit {
        lane: usize,
        at: usize,
        restore: Option<Taken<'c>>,
    },
    /// Walks `lanes` of segment `at`, beginning at `from`, as its share was measured.
    Walk {
        at: usize,
        from: usize,
        lanes: Vec<usize>,
        reserved: Reserved<'c>,
    },
}

/// Every family's progress through every segment, what is held for the emits, and what
/// became of each family.
pub(super) struct Schedule {
    lanes: usize,
    segments: usize,
    /// Segment-major.
    status: Vec<Status>,
    reads: Vec<Measured>,
    /// What reading each segment's own pages, and restoring its restart, takes, once measured.
    pages: Vec<Option<Reads>>,
    restarts: Vec<Option<Reads>>,
    /// The segment each segment's walk begins at: itself, or, while restarts cannot be
    /// restored, one before it in its run, walking on from there. Every restart after it, up
    /// to the segment's own, is known not to restore.
    begins: Vec<usize>,
    /// Each family's next segment to emit, and whether a worker emits it.
    next: Vec<usize>,
    emitting: Vec<bool>,
    /// The bytes reading back the spilled chunk each family's emit stopped at takes.
    restoring: Vec<Option<usize>>,
    /// Each family's segments sealed and not yet emitted.
    queued: Vec<usize>,
    /// Each segment's families running or sealed and not yet emitted.
    live: Vec<usize>,
    /// Segments with a family running or sealed and not yet emitted.
    held: usize,
    /// Whether each family's items spill, and the most a walk of it projected, spilled or
    /// not; none before one has sealed.
    spills: Vec<bool>,
    estimate: Vec<Option<usize>>,
    /// Tasks handed out and not done.
    running: usize,
    /// Each family's failures, by segment, that what comes before them may yet overtake: the
    /// earliest becomes the family's once nothing before it can end its branch.
    pending: Vec<BTreeMap<usize, Failure>>,
    /// Cells, by segment then family, whose parked operator is being written out: no walk
    /// takes them until it lands.
    writing: BTreeSet<(usize, usize)>,
    /// Cells a walk held as an operator was parked for them.
    arrived: BTreeSet<(usize, usize)>,
    /// The run stopped: no family's emit moves past what it emitted, and no failure stands.
    pub(super) halted: bool,
    outcomes: Vec<Outcome>,
    pub(super) failure: Option<io::Error>,
    pub(super) stats: Stats,
}

impl Schedule {
    /// The schedule of `segments` segments for families whose items spill as `spills` says.
    pub(super) fn new(segments: usize, spills: Vec<bool>) -> io::Result<Self> {
        let lanes = spills.len();
        let cells = segments
            .checked_mul(lanes)
            .ok_or_else(engine_allocation::oom)?;
        Ok(Self {
            lanes,
            segments,
            status: engine_allocation::filled(cells, Status::Open)?,
            reads: engine_allocation::filled(segments, Measured::Unknown)?,
            pages: engine_allocation::filled(segments, None)?,
            restarts: engine_allocation::filled(segments, None)?,
            begins: engine_allocation::collect_vec(0..segments)?,
            next: engine_allocation::filled(lanes, 0)?,
            emitting: engine_allocation::filled(lanes, false)?,
            restoring: engine_allocation::filled(lanes, None)?,
            queued: engine_allocation::filled(lanes, 0)?,
            live: engine_allocation::filled(segments, 0)?,
            held: 0,
            spills,
            estimate: engine_allocation::filled(lanes, None)?,
            running: 0,
            pending: engine_allocation::collect_vec((0..lanes).map(|_| BTreeMap::new()))?,
            writing: BTreeSet::new(),
            arrived: BTreeSet::new(),
            halted: false,
            outcomes: engine_allocation::collect_vec((0..lanes).map(|_| Outcome::default()))?,
            failure: None,
            stats: Stats::default(),
        })
    }

    pub(super) fn status(&self, at: usize, lane: usize) -> Status {
        self.status[at * self.lanes + lane]
    }

    fn set(&mut self, at: usize, lane: usize, status: Status) {
        self.status[at * self.lanes + lane] = status;
    }

    /// The first segment some family has not emitted.
    pub(super) fn base(&self) -> usize {
        self.next.iter().copied().min().unwrap_or(self.segments)
    }

    pub(super) fn finished(&self) -> bool {
        self.base() >= self.segments
    }

    /// Whether a walk may take `lane` of `at` now: none has, or it waits for the segment
    /// before, which has since handed its operator on, and no operator parked for it is being
    /// written out.
    fn claimable(&self, plan: &Plan, at: usize, lane: usize) -> bool {
        let free = match self.status(at, lane) {
            Status::Open => true,
            Status::Waiting => plan
                .get(at)
                .before
                .is_some_and(|before| self.status(before, lane) == Status::Sealed),
            Status::Running
            | Status::Sealed
            | Status::Refused
            | Status::Lost
            | Status::Abandoned
            | Status::Closed => false,
        };
        free && (self.writing.is_empty() || !self.writing.contains(&(at, lane)))
    }

    /// The parked operators of `cells`, by segment then family, are being written out: no walk
    /// takes those cells until [`Self::written`].
    pub(super) fn write(&mut self, cells: &[(usize, usize)]) {
        self.writing.extend(cells.iter().copied());
    }

    /// The parked operators of `cells` landed: their cells may be taken again.
    pub(super) fn written(&mut self, cells: &[(usize, usize)]) {
        for cell in cells {
            self.writing.remove(cell);
        }
    }

    /// `lanes` of `at`, which a walk holds, found their parked operators being written out:
    /// they are given back, to be taken once those land and the segment before sealed.
    pub(super) fn waits(
        &mut self,
        plan: &Plan,
        at: usize,
        lanes: &[usize],
        gone: &mut Gone,
    ) -> io::Result<()> {
        for &lane in lanes {
            if self.status(at, lane) == Status::Abandoned {
                self.abandoned(plan, at, lane, gone)?;
                continue;
            }
            self.set(at, lane, Status::Waiting);
            self.release(at);
        }
        Ok(())
    }

    /// An operator was parked for `lane` of `at` while a walk held it.
    pub(super) fn arrives(&mut self, at: usize, lane: usize) {
        self.arrived.insert((at, lane));
    }

    /// Whether an operator was parked for `lane` of `at` while a walk held it; forgotten once
    /// asked.
    pub(super) fn arrived(&mut self, at: usize, lane: usize) -> bool {
        !self.arrived.is_empty() && self.arrived.remove(&(at, lane))
    }

    /// The output a walk of `lanes` is taken to need, and whether that is known: for a
    /// family whose items spill, a chunk, or the most a walk of it projected when less;
    /// for another, that most, none before one has sealed.
    fn need(&self, lanes: impl IntoIterator<Item = usize>, settings: &Settings) -> (usize, bool) {
        lanes.into_iter().fold((0, true), |(need, known), lane| {
            let estimate = self.estimate[lane];
            let (bytes, measured) = if self.spills[lane] {
                let chunk = settings.chunk_bytes;
                (estimate.map_or(chunk, |bytes| bytes.min(chunk)), true)
            } else {
                (estimate.unwrap_or(0), estimate.is_some())
            };
            (need.saturating_add(bytes), known && measured)
        })
    }

    /// Whether reading back `need` bytes for `lane`'s emit fits beside what is held and
    /// reserved (`committed`): within the working allowance for the first segment some
    /// family has not emitted (`base`), else within the output budget.
    fn fits(
        &self,
        lane: usize,
        need: usize,
        base: usize,
        settings: &Settings,
        committed: usize,
    ) -> bool {
        let limit = if self.next[lane] == base {
            settings.ceiling()
        } else {
            settings.output_bytes
        };
        committed.saturating_add(need) <= limit
    }

    /// Whether an emit waits to read back a chunk that does not fit.
    fn starved(&self, base: usize, settings: &Settings, committed: usize) -> bool {
        (0..self.lanes).any(|lane| {
            self.restoring[lane]
                .is_some_and(|need| !self.fits(lane, need, base, settings, committed))
        })
    }

    /// Whether segment `at` may start, walking families that need `need` of the output
    /// budget: within the window, and the first some family has not emitted, or, with that
    /// need known, beside what is held and reserved (`committed`), with every family's slots
    /// and no emit waiting for room to read back a chunk.
    fn admits(
        &self,
        at: usize,
        need: (usize, bool),
        settings: &Settings,
        committed: usize,
    ) -> bool {
        let base = self.base();
        at < base.saturating_add(settings.window)
            && (at == base
                || (need.1
                    && committed.saturating_add(need.0) <= settings.output_bytes
                    && self.queued.iter().all(|&queued| queued < settings.slots)
                    && !self.starved(base, settings, committed)))
    }

    fn claim(&mut self, at: usize, lane: usize) {
        self.set(at, lane, Status::Running);
        self.live[at] += 1;
        if self.live[at] == 1 {
            self.held += 1;
            self.stats.segments = self.stats.segments.max(self.held);
        }
    }

    fn release(&mut self, at: usize) {
        self.live[at] -= 1;
        if self.live[at] == 0 {
            self.held -= 1;
        }
    }

    /// The next task: an emit that can go on, the family furthest behind first, taking what
    /// reading back its spilled chunk needs; else, when `ahead`, a walk of the first segment
    /// with a family to take, once what it reads is measured and it may start and `hold` takes
    /// its share of the walkers' budget, taking every family it can; else a measure of a
    /// segment in the window some family has yet to walk.
    pub(super) fn pick<'c>(
        &mut self,
        plan: &Plan,
        settings: &Settings,
        credit: &'c Credit,
        ahead: bool,
        hold: &mut dyn FnMut(Reads) -> bool,
    ) -> io::Result<Option<Task<'c>>> {
        let (committed, base) = (credit.committed(), self.base());
        let ready = (0..self.lanes)
            .filter(|&lane| {
                !self.emitting[lane]
                    && self.next[lane] < self.segments
                    && matches!(
                        self.status(self.next[lane], lane),
                        Status::Sealed | Status::Refused | Status::Lost
                    )
                    && self.restoring[lane]
                        .is_none_or(|need| self.fits(lane, need, base, settings, committed))
            })
            .min_by_key(|&lane| self.next[lane]);
        if let Some(lane) = ready {
            self.emitting[lane] = true;
            self.running += 1;
            let restore = self.restoring[lane].take().map(|need| credit.hold(need));
            return Ok(Some(Task::Emit {
                lane,
                at: self.next[lane],
                restore,
            }));
        }
        if !ahead {
            return Ok(None);
        }
        let window = base..self.segments.min(base.saturating_add(settings.window));
        for at in window.clone() {
            if !(0..self.lanes).any(|lane| self.claimable(plan, at, lane)) {
                continue;
            }
            let Measured::Known(reads) = self.reads[at] else {
                break;
            };
            let need = self.need(
                (0..self.lanes).filter(|&lane| self.claimable(plan, at, lane)),
                settings,
            );
            if !self.admits(at, need, settings, committed) || !hold(reads) {
                break;
            }
            let lanes = engine_allocation::collect_vec(
                (0..self.lanes).filter(|&lane| self.claimable(plan, at, lane)),
            )?;
            for &lane in &lanes {
                self.claim(at, lane);
            }
            self.stats.walks += 1;
            self.running += 1;
            return Ok(Some(Task::Walk {
                at,
                from: self.begins[at],
                lanes,
                reserved: credit.reserve(need.0),
            }));
        }
        for at in window {
            let unwalked = (0..self.lanes)
                .any(|lane| matches!(self.status(at, lane), Status::Open | Status::Waiting));
            if unwalked && matches!(self.reads[at], Measured::Unknown) {
                self.reads[at] = Measured::Measuring;
                self.running += 1;
                return Ok(Some(Task::Measure { at }));
            }
        }
        Ok(None)
    }

    /// What walking `at` reads, once measured.
    pub(super) fn reads(&self, at: usize) -> Option<Reads> {
        match self.reads[at] {
            Measured::Known(reads) => Some(reads),
            Measured::Unknown | Measured::Measuring => None,
        }
    }

    /// The segment the walk of `at` begins at.
    pub(super) fn begins(&self, at: usize) -> usize {
        self.begins[at]
    }

    /// What measuring the walk of `at` has yet to read: the segment it begins at, whether that
    /// one's restart, and which segments' own pages, from there to `at`, are not yet measured.
    pub(super) fn unmeasured(&self, at: usize) -> io::Result<(usize, bool, Vec<usize>)> {
        let from = self.begins[at];
        let pages =
            engine_allocation::collect_vec((from..=at).filter(|&page| self.pages[page].is_none()))?;
        Ok((from, self.restarts[from].is_none(), pages))
    }

    /// What measuring the walk of `at` from `from` read: `restart`, `from`'s, and `pages`, each
    /// segment's own. A measure from where `at` no longer begins counts only for what it read,
    /// and `at` is measured again.
    pub(super) fn measured(
        &mut self,
        at: usize,
        from: usize,
        restart: Option<Reads>,
        pages: &[(usize, Reads)],
    ) {
        if let Some(restart) = restart {
            self.restarts[from] = Some(restart);
        }
        for &(page, reads) in pages {
            self.pages[page] = Some(reads);
        }
        self.stats.measured += pages.len();
        let reads = (self.begins[at] == from)
            .then(|| {
                (from..=at).try_fold(self.restarts[from]?, |reads, page| {
                    Some(reads.max(self.pages[page]?))
                })
            })
            .flatten();
        self.reads[at] = reads.map_or(Measured::Unknown, Measured::Known);
    }

    /// The walk of `at` from `from`, holding `lanes`, could not restore `from`'s restart: its
    /// walks begin before `from` from now, measured anew, past every restart known not to
    /// restore, as do `from`'s own; and its families' cells are open again, or given back once
    /// closed, with what was parked for them ([`Gone`]).
    pub(super) fn derive(
        &mut self,
        plan: &Plan,
        at: usize,
        from: usize,
        lanes: &[usize],
        gone: &mut Gone,
    ) -> io::Result<()> {
        for &lane in lanes {
            match self.status(at, lane) {
                Status::Abandoned => {
                    self.abandoned(plan, at, lane, gone)?;
                    engine_allocation::reserve(&mut gone.cells, 1)?;
                    gone.cells.push((lane, at, false));
                }
                Status::Running => {
                    self.set(at, lane, Status::Open);
                    self.release(at);
                }
                // Failed as its operator parked, and given back then.
                _ => {}
            }
        }
        if self.begins[at] == from {
            // A segment with a restart follows another of its run.
            let mut begin = from - 1;
            while self.begins[begin] < begin {
                begin = self.begins[begin];
            }
            for target in [at, from] {
                if self.begins[target] == from {
                    self.begins[target] = begin;
                    // A measure under way finds where it began stale.
                    if !matches!(self.reads[target], Measured::Measuring) {
                        self.reads[target] = Measured::Unknown;
                    }
                }
            }
        }
        Ok(())
    }

    /// A task handed out is done.
    pub(super) fn done(&mut self) {
        self.running -= 1;
    }

    /// With no task running and none to hand out, the family furthest behind whose emit
    /// waits to read back a chunk that cannot fit, with that segment and what reading it
    /// takes: nothing will make room for it.
    pub(super) fn stuck(&self) -> Option<(usize, usize, usize)> {
        if self.running > 0 {
            return None;
        }
        (0..self.lanes)
            .filter_map(|lane| Some((lane, self.next[lane], self.restoring[lane]?)))
            .min_by_key(|&(_, at, _)| at)
    }

    /// `lanes` of `at`, which a walk holds and cannot start there: those the segment before
    /// has handed on since, for the walk to take now, and those it lost, which are lost too;
    /// the rest wait for it, and those closed meanwhile are given back.
    pub(super) fn refused(
        &mut self,
        plan: &Plan,
        at: usize,
        lanes: &[usize],
        gone: &mut Gone,
    ) -> io::Result<Deferred> {
        let before = plan.get(at).before;
        let mut deferred = Deferred::default();
        for &lane in lanes {
            if self.status(at, lane) == Status::Abandoned {
                self.abandoned(plan, at, lane, gone)?;
                continue;
            }
            match before.map(|before| (before, self.status(before, lane))) {
                Some((_, Status::Sealed)) => {
                    engine_allocation::reserve(&mut deferred.ready, 1)?;
                    deferred.ready.push(lane);
                }
                Some((before, Status::Lost)) => {
                    engine_allocation::reserve(&mut deferred.lost, 1)?;
                    deferred.lost.push((lane, before));
                }
                _ => {
                    self.set(at, lane, Status::Waiting);
                    self.release(at);
                    self.stats.deferred += 1;
                }
            }
        }
        Ok(deferred)
    }

    /// Takes what each family of a walk projected, `(family, bytes)`, into the output a walk
    /// of it is taken to need.
    pub(super) fn estimate(&mut self, projected: &[(usize, usize)]) {
        for &(lane, bytes) in projected {
            let estimate = &mut self.estimate[lane];
            *estimate = Some(estimate.map_or(bytes, |estimate| estimate.max(bytes)));
        }
    }

    /// The families of `next` a walk of the segment before goes on into: those it hands on
    /// (`handed`) that wait or are untaken, and every other it may take, once `next` is
    /// measured and may start, `hold` takes its share of the walkers' budget, and going on
    /// saves a restart (`hot`) or hands an operator on; and what that walk reserved.
    pub(super) fn chain<'c>(
        &mut self,
        plan: &Plan,
        settings: &Settings,
        credit: &'c Credit,
        next: usize,
        handed: &[usize],
        hot: bool,
        hold: &mut dyn FnMut(Reads) -> bool,
    ) -> io::Result<(Vec<usize>, Reserved<'c>)> {
        let reads = match self.reads(next) {
            Some(reads) if hot || !handed.is_empty() => reads,
            _ => return Ok((Vec::new(), credit.reserve(0))),
        };
        let mut lanes = Vec::new();
        for lane in 0..self.lanes {
            let takes = if handed.contains(&lane) {
                matches!(self.status(next, lane), Status::Open | Status::Waiting)
            } else {
                self.claimable(plan, next, lane)
            };
            if takes {
                engine_allocation::reserve(&mut lanes, 1)?;
                lanes.push(lane);
            }
        }
        let need = self.need(lanes.iter().copied(), settings);
        if lanes.is_empty()
            || !self.admits(next, need, settings, credit.committed())
            || !hold(reads)
        {
            return Ok((Vec::new(), credit.reserve(0)));
        }
        for &lane in &lanes {
            self.claim(next, lane);
        }
        self.stats.walks += 1;
        self.stats.chained += 1;
        Ok((lanes, credit.reserve(need.0)))
    }

    /// The families of `handed` (none of `chained`, so to be parked for `next`) whose walk of
    /// `next` has sealed: they started there without the operator handed on.
    pub(super) fn started_without(
        &self,
        next: usize,
        handed: &[usize],
        chained: &[usize],
    ) -> io::Result<Vec<usize>> {
        engine_allocation::collect_vec(handed.iter().copied().filter(|&lane| {
            !chained.contains(&lane)
                && matches!(self.status(next, lane), Status::Sealed | Status::Refused)
        }))
    }

    /// `lanes` of `at` sealed their items for their emits.
    pub(super) fn sealed(&mut self, at: usize, lanes: &[usize]) {
        for &lane in lanes {
            self.set(at, lane, Status::Sealed);
            self.queued[lane] += 1;
        }
    }

    /// `lane` of `at`, which a walk holds, filed its ending for its emit (`status`, refused
    /// or lost).
    pub(super) fn settle(&mut self, at: usize, lane: usize, status: Status) {
        self.set(at, lane, status);
        self.queued[lane] += 1;
    }

    /// The segment after `at` in its branch, when `lane` waits there for the operator `at`
    /// would have handed on: it is taken, for its walk to settle.
    pub(super) fn waiting_after(&mut self, plan: &Plan, at: usize, lane: usize) -> Option<usize> {
        let after = plan.get(at).after?;
        (self.status(after, lane) == Status::Waiting).then(|| {
            self.claim(after, lane);
            after
        })
    }

    /// `lane`'s family has nothing to do on `branches`, sorted: every cell of theirs closes
    /// before a walk takes any.
    pub(super) fn pass(
        &mut self,
        plan: &Plan,
        lane: usize,
        branches: &[u64],
        gone: &mut Gone,
    ) -> io::Result<()> {
        if branches.is_empty() {
            return Ok(());
        }
        for at in 0..self.segments {
            let planned = plan.get(at);
            if planned.before.is_none() && branches.binary_search(&planned.branch).is_ok() {
                self.shut(plan, lane, Closing::Rest(at), gone)?;
            }
        }
        self.advance(plan, lane, gone)
    }

    /// `lane`'s family goes on in `branch` from ordinal `ordinal` of run `run`: every cell of
    /// the branch before the segment holding it, or of the branch when none does, closes before
    /// a walk takes any. The segment it goes on in, and the ordinal it joins that walk at.
    pub(super) fn resume(
        &mut self,
        plan: &Plan,
        segments: &[Segment],
        lane: usize,
        (branch, run, ordinal): (u64, u32, u32),
        gone: &mut Gone,
    ) -> io::Result<Option<(usize, u32)>> {
        let Some(leading) = (0..self.segments)
            .find(|&at| plan.get(at).branch == branch && plan.get(at).before.is_none())
        else {
            return Ok(None);
        };
        let mut cell = Some(leading);
        let goes = loop {
            let Some(at) = cell else {
                break None;
            };
            let segment = &segments[at];
            if (segment.run, segment.last) >= (run, ordinal) {
                let joins = if segment.run == run {
                    ordinal.max(segment.first)
                } else {
                    segment.first
                };
                break Some((at, joins));
            }
            cell = plan.get(at).after;
        };
        match goes {
            Some((at, _)) if at == leading => return Ok(goes),
            Some((at, _)) => self.shut(plan, lane, Closing::Before(at), gone)?,
            None => self.shut(plan, lane, Closing::Rest(leading), gone)?,
        }
        self.advance(plan, lane, gone)?;
        Ok(goes)
    }

    /// Closes `closing`'s cells of `lane`: nothing of them reaches its emit, and what they
    /// filed or parked goes ([`Gone`]). A cell a walk holds is abandoned to it.
    pub(super) fn close(
        &mut self,
        plan: &Plan,
        lane: usize,
        closing: Closing,
        gone: &mut Gone,
    ) -> io::Result<()> {
        self.shut(plan, lane, closing, gone)?;
        self.advance(plan, lane, gone)
    }

    /// [`Self::close`], leaving `lane`'s next segment to emit where it is: a failure pending in
    /// a cell it closes was overtaken by what closes it, but for the one pending at the cell a
    /// [`Closing::Rest`] starts at.
    fn shut(
        &mut self,
        plan: &Plan,
        lane: usize,
        closing: Closing,
        gone: &mut Gone,
    ) -> io::Result<()> {
        let segments = self.segments;
        let following = |at: usize| match closing {
            Closing::Branch(_) | Closing::Rest(_) | Closing::Before(_) => plan.get(at).after,
            Closing::From(_) => (at + 1 < segments).then_some(at + 1),
        };
        let (mut cell, standing, until) = match closing {
            Closing::Branch(at) => (plan.get(at).after, None, None),
            Closing::Rest(at) => (Some(at), Some(at), None),
            Closing::From(at) => (Some(at), None, None),
            Closing::Before(at) => (Some(plan.leading(at)), None, Some(at)),
        };
        while let Some(at) = cell.filter(|&at| Some(at) != until) {
            cell = following(at);
            if standing != Some(at)
                && !self.pending[lane].is_empty()
                && let Some(overtaken) = self.pending[lane].remove(&at)
            {
                gone.waste.push(Waste::Failure(overtaken));
            }
            let (status, filed) = match self.status(at, lane) {
                Status::Closed | Status::Abandoned => continue,
                Status::Running => (Status::Abandoned, false),
                Status::Sealed | Status::Refused | Status::Lost => {
                    debug_assert!(!(self.emitting[lane] && self.next[lane] == at));
                    if self.next[lane] == at {
                        // What reading its spilled chunk back would take is no longer asked.
                        self.restoring[lane] = None;
                    }
                    self.queued[lane] -= 1;
                    self.release(at);
                    (Status::Closed, true)
                }
                Status::Open | Status::Waiting => (Status::Closed, false),
            };
            self.set(at, lane, status);
            engine_allocation::reserve(&mut gone.cells, 1)?;
            gone.cells.push((lane, at, filed));
        }
        Ok(())
    }

    /// `lane` of `at`, abandoned to the walk holding it, is given back.
    pub(super) fn abandoned(
        &mut self,
        plan: &Plan,
        at: usize,
        lane: usize,
        gone: &mut Gone,
    ) -> io::Result<()> {
        self.set(at, lane, Status::Closed);
        self.release(at);
        self.advance(plan, lane, gone)
    }

    /// Moves `lane`'s next segment to emit past those closed. Its earliest failure pending
    /// is the family's once it moves that far: nothing before can end that failure's branch
    /// first, so every one of its cells from there on closes.
    fn advance(&mut self, plan: &Plan, lane: usize, gone: &mut Gone) -> io::Result<()> {
        // What it did not emit before the run stopped stays pending, failures with it.
        if self.emitting[lane] || self.halted {
            return Ok(());
        }
        loop {
            while self.next[lane] < self.segments
                && self.status(self.next[lane], lane) == Status::Closed
            {
                self.next[lane] += 1;
            }
            let next = self.next[lane];
            let pending = &mut self.pending[lane];
            let (at, failure) = match pending.first_entry() {
                Some(earliest) if *earliest.key() <= next => earliest.remove_entry(),
                _ => return Ok(()),
            };
            // Those after it it overtakes.
            let overtaken = std::mem::take(pending);
            gone.waste
                .extend(overtaken.into_values().map(Waste::Failure));
            if let Some(replaced) = self.outcomes[lane].failure.replace(failure) {
                gone.waste.push(Waste::Failure(replaced));
            }
            self.shut(plan, lane, Closing::From(at), gone)?;
        }
    }

    /// `lane` fails as `failure` says. Its segment, and those after it in its branch, close
    /// whatever comes before: an earlier end of the branch closes them too. The rest close
    /// once the failure is the family's, the earliest left standing in segment order: an end
    /// of its branch before it, or an earlier failure, overtakes it.
    pub(super) fn fail(
        &mut self,
        plan: &Plan,
        lane: usize,
        failure: Failure,
        gone: &mut Gone,
    ) -> io::Result<()> {
        let at = failure.at;
        // What closed its segment, or failed it, first ended the family's work there.
        if self.halted
            || self.outcomes[lane].failure.is_some()
            || at < self.next[lane]
            || matches!(self.status(at, lane), Status::Closed | Status::Abandoned)
            || self.pending[lane].contains_key(&at)
        {
            gone.waste.push(Waste::Failure(failure));
            return Ok(());
        }
        // Held before anything can fail, so it never drops under the lock.
        self.pending[lane].insert(at, failure);
        self.shut(plan, lane, Closing::Rest(at), gone)?;
        self.advance(plan, lane, gone)
    }

    /// `lane` emitted `at`, of `branch`, the last of its chunks with `ending`.
    pub(super) fn emitted(
        &mut self,
        plan: &Plan,
        lane: usize,
        at: usize,
        branch: u64,
        ending: Option<Ending>,
        gone: &mut Gone,
    ) -> io::Result<()> {
        if let Some(ending) = ending {
            let ended = &mut self.outcomes[lane].ended;
            if let Err(error) = engine_allocation::reserve(ended, 1) {
                gone.waste.push(Waste::Ending(ending));
                return Err(error);
            }
            ended.push(Ended { at, branch, ending });
        }
        self.next[lane] = at + 1;
        self.emitting[lane] = false;
        self.queued[lane] -= 1;
        self.release(at);
        self.advance(plan, lane, gone)
    }

    /// `lane`'s emit stopped at a spilled chunk that reading back takes `need` bytes.
    pub(super) fn restores(&mut self, lane: usize, need: usize) {
        self.emitting[lane] = false;
        self.restoring[lane] = Some(need);
    }

    /// `lane`'s emit of its next segment stopped short.
    pub(super) fn unemit(&mut self, lane: usize) {
        self.emitting[lane] = false;
    }

    /// What became of each family: the segments it went past, from the first, settled.
    pub(super) fn outcomes(&mut self) -> Vec<Outcome> {
        let mut outcomes = std::mem::take(&mut self.outcomes);
        for (outcome, &next) in outcomes.iter_mut().zip(&self.next) {
            outcome.settled = next;
        }
        outcomes
    }
}
