//! Frames at any hour, moved to from the nearest of where it stands, a checkpoint or a restart.

use super::{Natives, Place, invalid};
use crate::{
    campaign::Parallel,
    columnar_frames::{
        Note, Pool, Recording, Segment, Spread, Steps, Views, Walker, metrics, on_pool, on_rayon,
    },
    format::FormatVersion,
    frame::{DecodedFrame, Metrics},
    rows::TableId,
    scope::CaptureMode,
};
use replay_columnar::{
    manifest::Status,
    walk::{SpanPart, State},
};
use std::{
    io,
    sync::{Arc, Mutex, PoisonError},
};

/// What a cursor's moves weigh, in hours walked, and where and how much it keeps of its walks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Costs {
    /// Restoring a restart, the views built anew after it included.
    pub restore_hours: u32,
    /// Taking up a checkpoint, the views built anew after it included.
    pub resume_hours: u32,
    /// The walk past which the views are built anew rather than noted hour by hour.
    pub rebuild_hours: u32,
    /// Whether its walks keep checkpoints.
    pub checkpoints: bool,
    /// Every how many hours of a run a checkpoint sits.
    pub checkpoint_hours: u32,
    /// The hours before the hour moved to last in which checkpoints are kept: those further
    /// back or past it are let go, so it keeps at most reach / hours + 1.
    pub checkpoint_reach: u32,
}

impl Default for Costs {
    fn default() -> Self {
        Self {
            restore_hours: 480,
            resume_hours: 120,
            rebuild_hours: 2_160,
            checkpoints: false,
            checkpoint_hours: 240,
            checkpoint_reach: 720,
        }
    }
}

/// Runs `job` for each index below `count`, one after another.
fn serial(count: usize, job: &(dyn Fn(usize) + Sync)) {
    (0..count).for_each(job);
}

/// Notes the `hour`th hour of a span `part` applied in its table's `note`.
fn note_span(note: &mut Note<'_>, part: &SpanPart<'_>, hour: usize) -> io::Result<()> {
    let (table, changes) = part.applied();
    note.hour(part.status(hour), part.section(hour).is_some(), changes, table)
}

/// The recording selecting a cursor's tables, its walker, and the steps it walks.
struct Walking<'r> {
    walker: Walker<'r>,
    steps: Option<Steps>,
}

self_cell::self_cell!(
    struct Walk {
        owner: Recording,
        #[not_covariant]
        dependent: Walking,
    }
);

/// A walk kept to start a later move from: its steps before ordinal `next`, sharing the
/// decoded units of their page, the state there, sharing its pages copy-on-write, and the
/// metrics of the hour before.
struct Checkpoint {
    run: u32,
    next: u32,
    steps: Steps,
    state: State,
    metrics: Metrics,
}

/// Where a move starts.
enum Start {
    /// From the hour applied last.
    Here,
    Checkpoint(usize),
    Restart,
}

/// Frames of some tables at any hour [`Natives`] hold, as walking the recording makes them.
pub struct Cursor {
    natives: Arc<Natives>,
    views: Views,
    /// The recording's walk, made by the first move; behind a lock only so the cursor is
    /// `Sync` while its decoders are not, and reached through `&mut` alone.
    walk: Mutex<Option<Walk>>,
    state: State,
    /// The hour `state` applied last; none before a move, or after a walk failed.
    at: Option<Place>,
    /// The metrics of the hour applied last.
    metrics: Metrics,
    /// The hour moved to last, which a build builds; none after a move or build failed.
    moved: Option<(u64, i32)>,
    costs: Costs,
    kept: Kept,
    /// Hours apply their tables apart: more than one besides Campaign.
    spreads: bool,
    /// The threads its moves and builds work on, rayon's when none ([`Self::set_pool`]).
    pool: Option<Arc<Pool>>,
    /// Restarts restored, checkpoints taken up, and hours walked, so far.
    restores: usize,
    resumes: usize,
    hours: usize,
}

impl Cursor {
    /// Frames of `tables` and Campaign, opaque cells encoded at `format`, as a recording of
    /// `mode` holds them.
    pub fn new(
        natives: Arc<Natives>,
        tables: &[TableId],
        format: FormatVersion,
        mode: CaptureMode,
    ) -> io::Result<Self> {
        // Moves walk many hours a build: edits are kept by slot, sorted once a build.
        let views = Views::new(tables, format, mode)?.by_slot().apart();
        Ok(Self {
            spreads: views.selected().len() > 2,
            pool: None,
            views,
            walk: Mutex::new(None),
            natives,
            state: State::new(crate::columnar_frames::schema()?)?,
            at: None,
            metrics: Metrics::default(),
            moved: None,
            costs: Costs::default(),
            kept: Kept::default(),
            restores: 0,
            resumes: 0,
            hours: 0,
        })
    }

    /// Complete views carrying change facts and field stamps, as a decoder with change facts
    /// makes them.
    pub fn with_change_facts(mut self) -> Self {
        self.views = self.views.with_change_facts();
        self
    }

    /// Moves weighed, and checkpoints kept, by `costs`.
    pub fn with_costs(mut self, costs: Costs) -> Self {
        self.costs = costs;
        self.kept = Kept::default();
        self
    }

    /// Moves and builds work on `pool`'s threads from now on, or on rayon's: a reader's
    /// background work on threads that do not compete with the rest.
    pub fn set_pool(&mut self, pool: Option<Arc<Pool>>) {
        self.views.set_pool(pool.clone());
        self.pool = pool;
    }

    pub fn natives(&self) -> &Arc<Natives> {
        &self.natives
    }

    /// The tables its frames hold, Campaign among them, in table order.
    pub fn tables(&self) -> &[TableId] {
        self.views.selected()
    }

    /// The restarts restored, the checkpoints taken up and the hours walked so far.
    pub fn counts(&self) -> (usize, usize, usize) {
        (self.restores, self.resumes, self.hours)
    }

    /// The checkpoints it keeps, and the bytes they hold apart from one another and it.
    pub fn checkpoints(&self) -> (usize, usize) {
        let walk = self.walk.lock().unwrap_or_else(PoisonError::into_inner);
        let live = self.at.and_then(|_| {
            walk.as_ref()
                .and_then(|walk| walk.with_dependent(|_, walking| walking.steps.as_ref().and_then(Steps::units)))
        });
        let bytes = self.kept.bytes(&self.state, self.at, live).unwrap_or(usize::MAX);
        (self.kept.list.len(), bytes)
    }

    /// Moves to `hour` of `branch`, read from the runs of recorded segment `segment` (a
    /// timeline's hour is read from the segment its piece is of), building no frame: false
    /// when the recording holds no such hour. `cancelled` is asked before each hour walked; a
    /// move it stops is interrupted and leaves the cursor at the hour it walked last, for the
    /// next move to walk on from.
    pub fn move_to(
        &mut self,
        segment: u64,
        branch: u64,
        hour: i32,
        cancelled: &dyn Fn() -> bool,
    ) -> io::Result<bool> {
        let Some(place) = self.natives.find(segment, hour) else {
            return Ok(false);
        };
        self.moved = None;
        self.reach(place, cancelled)?;
        self.moved = Some((branch, hour));
        Ok(true)
    }

    /// Takes `state` to `place`, from the nearest start: where it stands, a checkpoint, or
    /// the restart before it.
    fn reach(&mut self, place: Place, cancelled: &dyn Fn() -> bool) -> io::Result<()> {
        let Self {
            natives,
            views,
            walk,
            state,
            at,
            metrics: last,
            costs,
            kept,
            spreads,
            pool,
            restores,
            resumes,
            hours,
            ..
        } = self;
        let pool = pool.as_deref();
        let walk = walk.get_mut().unwrap_or_else(PoisonError::into_inner);
        if walk.is_none() {
            let recording = natives.recording().selecting(views.selected())?;
            let apart = *spreads;
            *walk = Some(Walk::try_new::<io::Error>(recording, |recording| {
                let mut walker = Walker::new(recording)?;
                if apart {
                    walker.decode_apart(on_rayon);
                }
                Ok(Walking {
                    walker,
                    steps: None,
                })
            })?);
        }
        let pooled = |count: usize, job: &(dyn Fn(usize) + Sync)| {
            on_pool(pool, || {
                use rayon::prelude::*;
                (0..count).into_par_iter().for_each(job);
            });
        };
        let spread: &Spread<'_> = if *spreads { &pooled } else { &serial };
        let walk = walk.as_mut().expect("made above");
        walk.with_dependent_mut(|recording, walking| -> io::Result<()> {
            let Walking { walker, steps } = walking;
            let segment = segment_of(recording, place)?;
            // Each start's first ordinal to apply and its cost before walking, in hours.
            let target = place.ordinal + 1;
            let mut best = (Start::Restart, segment.first, if segment.restart.is_some() { costs.restore_hours } else { costs.resume_hours });
            let weigh = |(_, next, setup): &(Start, u32, u32)| u64::from(*setup) + u64::from(target - next);
            for (at, held) in kept.list.iter().enumerate() {
                let candidate = (Start::Checkpoint(at), held.next, costs.resume_hours);
                if held.run == place.run
                    && held.next <= target
                    && weigh(&candidate) < weigh(&best)
                {
                    best = candidate;
                }
            }
            if let Some(held) = at.filter(|held| held.run == place.run && held.ordinal < target)
                && steps.is_some()
            {
                let candidate = (Start::Here, held.ordinal + 1, 0);
                if weigh(&candidate) <= weigh(&best) {
                    best = candidate;
                }
            }
            let (start, next, _) = best;
            match start {
                Start::Here => {}
                Start::Restart => {
                    *at = None;
                    if let Some(held) = steps.take() {
                        walker.stop(held);
                    }
                    on_pool(pool, || walker.begin(&segment, state))?;
                    *steps = Some(walker.steps(&segment)?);
                    state.charge_hours(false)?;
                    views.reset();
                    *restores += 1;
                }
                Start::Checkpoint(index) => {
                    *at = None;
                    if let Some(held) = steps.take() {
                        walker.stop(held);
                    }
                    let held = &kept.list[index];
                    walker.take_up()?;
                    *state = held.state.share_spread(spread)?;
                    *steps = Some(held.steps.fork()?);
                    *last = held.metrics.clone();
                    *at = held.next.checked_sub(1).map(|ordinal| Place { ordinal, ..place });
                    views.reset();
                    *resumes += 1;
                }
            }
            if target - next > costs.rebuild_hours {
                views.reset();
            }
            let keeps = costs.checkpoints && costs.checkpoint_hours > 0;
            if keeps {
                kept.window(place, target, costs.checkpoint_reach);
            }
            let walking = steps.as_mut().ok_or_else(|| invalid("A cursor walks no run"))?;
            while walking.next() < target {
                if cancelled() {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                // Hours walk table by table up to the next hour a checkpoint may be kept at.
                let mut until = target;
                if keeps {
                    let every = costs.checkpoint_hours;
                    until = until.min((walking.next() / every + 1).saturating_mul(every));
                }
                let mut notes = match views.notes(state) {
                    Ok(notes) => notes,
                    Err(error) => {
                        views.reset();
                        return Err(error);
                    }
                };
                let stepped = walker
                    .step_span(walking, state, until, spread, &mut notes, &note_span, |_, manifest| {
                        metrics(manifest)
                    })
                    .map(|stepped| {
                        stepped.map(|(walked, noted, metrics)| (walked, noted.map(|()| metrics)))
                    });
                drop(notes);
                let (walked, noted) = match stepped {
                    Ok(Some(noted)) => noted,
                    Ok(None) => (0, Err(invalid("A run holds no such hour"))),
                    Err(error) => {
                        *at = None;
                        *steps = None;
                        return Err(error);
                    }
                };
                *hours += walked as usize;
                *at = Some(Place {
                    ordinal: walking.next() - 1,
                    ..place
                });
                match noted {
                    Ok(metrics) => *last = metrics,
                    Err(error) => {
                        // The views missed an hour: the next build makes them anew.
                        views.reset();
                        return Err(error);
                    }
                }
                if keeps
                    && walking.next() % costs.checkpoint_hours == 0
                    && target - walking.next() <= costs.checkpoint_reach
                {
                    kept.keep(place, walking, state, last, spread)?;
                }
            }
            Ok(())
        })
    }

    /// The frame of the hour moved to last, as walking the recording would have built it,
    /// views built anew on `parallel`'s workers when given.
    pub fn build(&mut self, parallel: Option<&Parallel<'_>>) -> io::Result<DecodedFrame> {
        let (branch, hour) = self
            .moved
            .ok_or_else(|| invalid("A frame is built before any move"))?;
        let built = self
            .views
            .build(&self.state, (branch, hour), self.metrics.clone(), parallel);
        if built.is_err() {
            self.views.reset();
            self.moved = None;
        }
        built
    }

    /// The tables' frame at `hour` of `branch`, read from recorded segment `segment`; none when
    /// the recording holds no such hour.
    pub fn frame(
        &mut self,
        segment: u64,
        branch: u64,
        hour: i32,
        parallel: Option<&Parallel<'_>>,
    ) -> io::Result<Option<DecodedFrame>> {
        if !self.move_to(segment, branch, hour, &|| false)? {
            return Ok(None);
        }
        self.build(parallel).map(Some)
    }

    /// Whether `table`, one of its tables, is complete at the hour moved to last, as its view
    /// there would be: read whole, and not withheld.
    pub fn complete(&self, table: TableId) -> io::Result<bool> {
        if self.moved.is_none() {
            return Err(invalid("A table's status is read before any move"));
        }
        if !self.views.selected().contains(&table) {
            return Err(invalid(format!("{table:?} is not read by this cursor")));
        }
        let index = table.index();
        Ok(self.state.withheld(index).is_none() && self.state.status(index) == Some(Status::Complete))
    }
}

/// The checkpoints a cursor keeps, in the order of their runs and ordinals: only at fixed
/// ordinals in a fixed window before the hour it moved to last ([`Costs::checkpoint_reach`]).
#[derive(Default)]
struct Kept {
    list: Vec<Checkpoint>,
}

impl Kept {
    /// Lets go of every checkpoint but those of `place`'s run before ordinal `target`, the
    /// move's, by at most `reach` hours.
    fn window(&mut self, place: Place, target: u32, reach: u32) {
        self.list.retain(|held| {
            held.run == place.run
                && held.next <= target
                && target - held.next <= reach
        });
    }

    /// Keeps the walk of `steps` and `state` as a checkpoint unless one is kept there already.
    fn keep(
        &mut self,
        place: Place,
        steps: &Steps,
        state: &State,
        metrics: &Metrics,
        spread: &Spread<'_>,
    ) -> io::Result<()> {
        let next = steps.next();
        let index = self
            .list
            .partition_point(|held| (held.run, held.next) < (place.run, next));
        if self
            .list
            .get(index)
            .is_some_and(|held| (held.run, held.next) == (place.run, next))
        {
            return Ok(());
        }
        let forked = steps.fork()?;
        let state = state.share_spread(spread)?;
        self.list
            .try_reserve(1)
            .map_err(|_| engine_allocation::oom())?;
        self.list.insert(
            index,
            Checkpoint {
                run: place.run,
                next,
                steps: forked,
                state,
                metrics: metrics.clone(),
            },
        );
        Ok(())
    }

    /// The bytes they hold apart from one another and from `state`, the walk standing at `at`
    /// in the page of `units`, counted now: each the pages the next of its run does not share,
    /// or the walk, and each page's decoded units once, but the walk's own.
    fn bytes(&self, state: &State, at: Option<Place>, units: Option<(usize, usize)>) -> io::Result<usize> {
        let mut bytes = 0;
        for (index, held) in self.list.iter().enumerate() {
            let after = self
                .list
                .get(index + 1)
                .filter(|later| later.run == held.run);
            let live = at.filter(|at| after.is_none() && at.run == held.run);
            bytes += held.steps.decoder_bytes()
                + match (after, live) {
                    (Some(later), _) => held.state.unshared_bytes(&later.state, &on_rayon)?,
                    (None, Some(_)) => held.state.unshared_bytes(state, &on_rayon)?,
                    (None, None) => held.state.held_bytes(),
                };
        }
        let mut pages: Vec<(usize, usize)> = self
            .list
            .iter()
            .filter_map(|held| held.steps.units())
            .filter(|&(place, _)| units.is_none_or(|(live, _)| live != place))
            .collect();
        pages.sort_unstable();
        pages.dedup();
        Ok(bytes + pages.iter().map(|&(_, held)| held).sum::<usize>())
    }
}

/// The segment of `recording` holding `place`'s hour.
fn segment_of(recording: &Recording, place: Place) -> io::Result<Segment> {
    let segments = recording.segments();
    let at = segments.partition_point(|segment| (segment.run, segment.first) <= (place.run, place.ordinal));
    at.checked_sub(1)
        .map(|at| segments[at])
        .filter(|segment| segment.run == place.run && place.ordinal <= segment.last)
        .ok_or_else(|| invalid("No segment of the recording holds the hour"))
}
