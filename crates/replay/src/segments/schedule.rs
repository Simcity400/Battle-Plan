//! Which segment each family walks or emits next, and what a run may start.

use super::{
    Settings, Stats, invalid,
    lane::{Credit, Reserved, Taken},
};
use crate::columnar_frames::{Reads, Recording, Segment};
use std::{collections::HashMap, io};

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
}

/// A family's progress through a segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Status {
    /// No walk has taken it, or one gave it back not knowing whether it can start there.
    Open,
    /// It cannot start there, and waits for the segment before to hand its operator on.
    Waiting,
    Running,
    /// Its items wait for its emit, or were emitted.
    Sealed,
}

/// What walking a segment reads, as far as a measure has said.
#[derive(Clone, Copy)]
enum Measured {
    Unknown,
    Measuring,
    Known(Reads),
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
    Walk {
        at: usize,
        lanes: Vec<usize>,
        reserved: Reserved<'c>,
    },
}

/// Every family's progress through every segment, and what is held for the emits.
pub(super) struct Schedule {
    lanes: usize,
    segments: usize,
    /// Segment-major.
    status: Vec<Status>,
    reads: Vec<Measured>,
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
            next: engine_allocation::filled(lanes, 0)?,
            emitting: engine_allocation::filled(lanes, false)?,
            restoring: engine_allocation::filled(lanes, None)?,
            queued: engine_allocation::filled(lanes, 0)?,
            live: engine_allocation::filled(segments, 0)?,
            held: 0,
            spills,
            estimate: engine_allocation::filled(lanes, None)?,
            running: 0,
            failure: None,
            stats: Stats::default(),
        })
    }

    fn status(&self, at: usize, lane: usize) -> Status {
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
    /// before, which has since handed its operator on.
    fn claimable(&self, plan: &Plan, at: usize, lane: usize) -> bool {
        match self.status(at, lane) {
            Status::Open => true,
            Status::Waiting => plan
                .get(at)
                .before
                .is_some_and(|before| self.status(before, lane) == Status::Sealed),
            Status::Running | Status::Sealed => false,
        }
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
    /// reading back its spilled chunk needs; else a walk of the first segment with a family
    /// to take, once what it reads is measured and it may start and `hold` takes its share
    /// of the walkers' budget, taking every family it can; else a measure of a segment in the
    /// window some family has yet to walk.
    pub(super) fn pick<'c>(
        &mut self,
        plan: &Plan,
        settings: &Settings,
        credit: &'c Credit,
        hold: &mut dyn FnMut(Reads) -> bool,
    ) -> io::Result<Option<Task<'c>>> {
        let (committed, base) = (credit.committed(), self.base());
        let ready = (0..self.lanes)
            .filter(|&lane| {
                !self.emitting[lane]
                    && self.next[lane] < self.segments
                    && self.status(self.next[lane], lane) == Status::Sealed
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

    pub(super) fn measured(&mut self, at: usize, reads: Reads) {
        self.reads[at] = Measured::Known(reads);
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

    /// `lanes` of `at`, which cannot start there: those the segment before has handed on
    /// since, for the walk to take now; the rest wait for it.
    pub(super) fn refused(
        &mut self,
        plan: &Plan,
        at: usize,
        lanes: &[usize],
    ) -> io::Result<Vec<usize>> {
        let before = plan.get(at).before;
        let mut ready = Vec::new();
        for &lane in lanes {
            if before.is_some_and(|before| self.status(before, lane) == Status::Sealed) {
                engine_allocation::reserve(&mut ready, 1)?;
                ready.push(lane);
            } else {
                self.set(at, lane, Status::Waiting);
                self.release(at);
                self.stats.deferred += 1;
            }
        }
        Ok(ready)
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

    /// A family of `handed` (none of `chained`, so to be parked for `next`) whose walk of
    /// `next` has sealed: it started there without the operator handed on.
    pub(super) fn started_without(
        &self,
        next: usize,
        handed: &[usize],
        chained: &[usize],
    ) -> Option<usize> {
        handed
            .iter()
            .copied()
            .find(|&lane| !chained.contains(&lane) && self.status(next, lane) == Status::Sealed)
    }

    /// `lanes` of `at` sealed their items for their emits.
    pub(super) fn sealed(&mut self, at: usize, lanes: &[usize]) {
        for &lane in lanes {
            self.set(at, lane, Status::Sealed);
            self.queued[lane] += 1;
        }
    }

    /// `lane` emitted `at`.
    pub(super) fn emitted(&mut self, lane: usize, at: usize) {
        self.next[lane] = at + 1;
        self.emitting[lane] = false;
        self.queued[lane] -= 1;
        self.release(at);
    }

    /// `lane`'s emit stopped at a spilled chunk that reading back takes `need` bytes.
    pub(super) fn restores(&mut self, lane: usize, need: usize) {
        self.emitting[lane] = false;
        self.restoring[lane] = Some(need);
    }
}
