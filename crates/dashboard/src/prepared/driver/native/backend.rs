//! A family's native backend ([`Native`]) and its member of a run ([`Bound`]): its operators
//! over each segment the run walks for it, reading the walk's shared world, and its ordered
//! emit, which keeps each branch's projected hours in that branch's state and publishes them
//! through the family's stage a batch at a time, where the recording ends one
//! ([`Stage::processed`]), with the state it goes on with ([`Native::save`]). A branch its
//! committed publication left work on goes on from there ([`Going`]): its operators from where
//! that ended, and its state from what was saved with it.

use super::{
    measure::cycles,
    shape::{Placed, Shape},
};
use crate::prepared::driver::{
    publication::{self, Hours, Made, Stage},
    registry::FamilyId,
};
use dashboard_base::prepared::{
    priority::{JobPriority, Level},
    roots::{self, Published},
};
use dashboard_world::{Needs, World};
use replay::{
    columnar_frames::{Recording, Segment},
    segments::{
        self, Carry, Chunk, Ending, Hour, Item, Lane, Loss, Manifest, Operator, Resume, Sink,
        Start, State,
    },
};
use replay_data::rows::TableId;
use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    io,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

/// An hour a family's operators project, with the hour it is of.
pub(crate) trait Projected: Item {
    fn hour(&self) -> i32;
}

/// What a family's operators project.
pub(crate) type Output<N> = <<N as Native>::Operator as Operator<World>>::Output;

/// A family's native backend: what its operators read and project, and how a branch's ordered
/// state keeps their hours and publishes them.
pub(crate) trait Native: Send + Sync {
    type Operator: Operator<World, Output: Projected>;
    type Branch: Send;

    /// The tables its operators read.
    fn tables(&self) -> &'static [TableId];

    /// What its operators read of the walk's world.
    fn needs(&self) -> io::Result<Needs>;

    fn operator(&self, segment: &Segment) -> io::Result<Self::Operator>;

    /// The state of `branch`, whose first hour it keeps is `first`.
    fn branch(&self, branch: u64, first: i32) -> io::Result<Self::Branch>;

    /// Keeps `hour` of `branch` in `state`; a family on the series engine writes it into the
    /// series `stage` opens for the batch.
    fn observe(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        hour: <Self::Operator as Operator<World>>::Output,
    ) -> io::Result<()>;

    /// Publishes through `stage` what `state` of `branch` kept since it last did; `through` is
    /// the branch's last hour once `state` kept its last segment whole.
    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        through: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()>;

    /// What it counted over the run, by name, for the run's measurement.
    fn counts(&self) -> Vec<(&'static str, u64)> {
        Vec::new()
    }

    /// What `state` goes on with after the hours its last flush published, for a later run
    /// of the recording grown to take up ([`Self::resume`]): empty when what the flush wrote
    /// is all it goes on with; None when it cannot be saved, and a branch the recording grows
    /// on is then prepared again whole.
    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        let _ = state;
        Ok(None)
    }

    /// The state of `branch` going on from `saved`, what [`Self::save`] wrote with the family's
    /// committed publication `published` there.
    fn resume(
        &self,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        published: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        let _ = (stage, branch, published, saved);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "This family's branch state is never saved",
        ))
    }
}

/// A branch a family's committed publication left work on: where it goes on, and its operator's
/// carry there.
pub(crate) struct Going {
    pub branch: u64,
    pub run: u32,
    pub ordinal: u32,
    pub published: Published,
    pub carry: Option<Vec<u8>>,
}

/// What a family takes up of the branches its committed publications hold: those it has
/// nothing left to do on, and those it goes on in, each in order.
#[derive(Default)]
pub(crate) struct Taken<'p> {
    pub passes: Vec<u64>,
    pub going: &'p [Going],
}

/// What every walk and emit of a run shares.
pub(crate) struct Walking<'a> {
    pub(crate) recording: &'a Recording,
    pub(crate) shape: &'a Shape,
    /// The job's cancellation, which the first task to see it carries into the run.
    pub(crate) cancelled: &'a AtomicBool,
    pub(crate) cancellation: &'a segments::Cancellation,
    pub(crate) measuring: bool,
    /// The job the run works for, which every walk enters on the worker taking it.
    pub(crate) job: JobPriority,
    /// Cycles the walks' worlds took, when measuring.
    pub(crate) world: AtomicU64,
}

impl Walking<'_> {
    /// Cancels the run once the job is cancelled, waking every wait in it.
    pub(crate) fn watch(&self) {
        if self.cancelled.load(Ordering::Relaxed) {
            self.cancellation.cancel();
        }
    }
}

/// A walk's world: the one its families read, advanced once an hour before them.
pub(crate) struct Walked<'a> {
    world: World,
    walking: &'a Walking<'a>,
}

impl<'a> Walked<'a> {
    pub(crate) fn new(world: World, walking: &'a Walking<'a>) -> Self {
        Self { world, walking }
    }
}

impl segments::World for Walked<'_> {
    fn hour(&mut self, state: &State, manifest: &Manifest<'_>) -> io::Result<()> {
        self.walking.watch();
        if !self.walking.measuring {
            return self.world.hour(state, manifest);
        }
        let started = replay_frame::cpu::stamp();
        let advanced = self.world.hour(state, manifest);
        self.walking.world.fetch_add(
            replay_frame::cpu::stamp().wrapping_sub(started),
            Ordering::Relaxed,
        );
        advanced
    }
}

/// A family's operator in a run: it reads the walk's world, and while the run is measured its
/// cycles count into `total`, less those its pushes into the run took.
struct Timed<'a, O> {
    operator: O,
    spent: u64,
    total: Option<&'a AtomicU64>,
}

impl<O> Timed<'_, O> {
    fn timed<T>(&mut self, work: impl FnOnce(&mut O) -> T) -> T {
        if self.total.is_none() {
            return work(&mut self.operator);
        }
        let started = replay_frame::cpu::stamp();
        let done = work(&mut self.operator);
        self.spent += replay_frame::cpu::stamp().wrapping_sub(started);
        done
    }
}

impl<O> Drop for Timed<'_, O> {
    fn drop(&mut self) {
        if let Some(total) = self.total {
            total.fetch_add(self.spent, Ordering::Relaxed);
        }
    }
}

/// The run's sink, and the cycles its pushes took.
struct Pushing<'s, S: ?Sized> {
    sink: &'s mut S,
    spent: u64,
}

impl<T, S: Sink<T> + ?Sized> Sink<T> for Pushing<'_, S> {
    fn push(&mut self, item: T) -> io::Result<()> {
        let started = replay_frame::cpu::stamp();
        let pushed = self.sink.push(item);
        self.spent += replay_frame::cpu::stamp().wrapping_sub(started);
        pushed
    }
}

impl<'a, O: Operator<World>> Operator<Walked<'a>> for Timed<'a, O> {
    type Output = O::Output;

    fn starts_at(&self, start: Start<'_>) -> bool {
        self.operator.starts_at(start)
    }

    fn start(&mut self, start: Start<'_>, branch: u64, first: i32) -> io::Result<()> {
        self.timed(|operator| operator.start(start, branch, first))
    }

    fn cross(&mut self, first: i32) -> io::Result<()> {
        self.timed(|operator| operator.cross(first))
    }

    fn hour(
        &mut self,
        hour: &Hour<'_, Walked<'a>>,
        out: &mut dyn Sink<O::Output>,
    ) -> io::Result<()> {
        let hour = Hour {
            branch: hour.branch,
            hour: hour.hour,
            run: hour.run,
            ordinal: hour.ordinal,
            starts: hour.starts,
            opens: hour.opens,
            state: hour.state,
            manifest: hour.manifest,
            world: &hour.world.world,
        };
        if self.total.is_none() {
            return self.operator.hour(&hour, out);
        }
        let mut out = Pushing {
            sink: out,
            spent: 0,
        };
        let done = self.timed(|operator| operator.hour(&hour, &mut out));
        self.spent = self.spent.saturating_sub(out.spent);
        done
    }

    fn end(&mut self, out: &mut dyn Sink<O::Output>) -> io::Result<()> {
        if self.total.is_none() {
            return self.operator.end(out);
        }
        let mut out = Pushing {
            sink: out,
            spent: 0,
        };
        let done = self.timed(|operator| operator.end(&mut out));
        self.spent = self.spent.saturating_sub(out.spent);
        done
    }

    fn carried(&self) -> Option<usize> {
        self.operator.carried()
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        self.operator.save(out)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.operator.load(bytes)
    }
}

/// The cycles a member spent, by what spent them.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Spent {
    pub operators: u64,
    /// Its ordered emit keeping hours, and publishing them: serial, a segment at a time.
    pub emit: u64,
    pub publish: u64,
}

/// A member of a run, its backend's types erased.
pub(crate) trait Member: Send {
    fn id(&self) -> FamilyId;

    fn tables(&self) -> &'static [TableId];

    fn needs(&self) -> io::Result<Needs>;

    /// Whether it has nothing to do on any branch of `shape`.
    fn idle(&self, shape: &Shape) -> bool;

    fn lane<'a>(&'a mut self, walking: &'a Walking<'a>) -> Lane<'a, Walked<'a>>;

    /// Waits for the batch it queued last to be committed. What it did not queue, as when its
    /// family failed or the run stopped, goes uncommitted.
    fn settle(&mut self) -> io::Result<()>;

    fn spent(&self) -> Spent;

    /// What its backend counted over the run ([`Native::counts`]).
    fn counts(&self) -> Vec<(&'static str, u64)>;
}

/// A native backend bound to its family's stage.
pub(crate) struct Bound<'p, 's, N: Native> {
    id: FamilyId,
    native: N,
    /// The branches its committed publication leaves nothing of to prepare, in order.
    passes: Vec<u64>,
    /// Where it goes on in the branches its committed publication left work on, in order.
    resumes: Vec<Resume<'p>>,
    emit: Emit<'p, 's, N>,
    operators: AtomicU64,
}

/// A family's ordered emit.
struct Emit<'p, 's, N: Native> {
    name: &'static str,
    stage: Stage<'p, 's>,
    states: BTreeMap<u64, N::Branch>,
    /// The branches it goes on in whose state it has not taken up yet.
    going: BTreeMap<u64, &'p Going>,
    /// Where each branch it goes on in past its segment's first hour does, until processed.
    joins: BTreeMap<u64, (u32, u32)>,
    /// Branches the open batch touched, whose states publish with it.
    dirty: BTreeSet<u64>,
    /// Of those, the branches whose last segment it holds whole, those it ends, and those it
    /// ends short.
    complete: BTreeSet<u64>,
    ended: BTreeSet<u64>,
    short: BTreeSet<u64>,
    /// Branches whose batch ends partway through a segment, whose state is not saved: no later
    /// run goes on from there.
    open: BTreeSet<u64>,
    /// The segment and hour its series writers held their share at: published once a later
    /// hour's items show that hour's are all kept.
    full: Option<(usize, i32)>,
    /// What each branch's operator hands on past the segment processed last: saved past the
    /// last the recording holds, else None, and nothing goes on from a state saved there.
    carries: BTreeMap<u64, Option<Vec<u8>>>,
    spent: Spent,
}

impl<'p, 's, N: Native> Bound<'p, 's, N> {
    pub(crate) fn new(id: FamilyId, native: N, stage: Stage<'p, 's>, taken: Taken<'p>) -> Self {
        let resumes = taken
            .going
            .iter()
            .map(|going| Resume {
                branch: going.branch,
                run: going.run,
                ordinal: going.ordinal,
                carry: going.carry.as_deref(),
            })
            .collect();
        Self {
            id,
            native,
            passes: taken.passes,
            resumes,
            emit: Emit {
                name: id.name(),
                stage,
                states: BTreeMap::new(),
                going: taken
                    .going
                    .iter()
                    .map(|going| (going.branch, going))
                    .collect(),
                joins: taken
                    .going
                    .iter()
                    .map(|going| (going.branch, (going.run, going.ordinal)))
                    .collect(),
                dirty: BTreeSet::new(),
                complete: BTreeSet::new(),
                ended: BTreeSet::new(),
                short: BTreeSet::new(),
                open: BTreeSet::new(),
                full: None,
                carries: BTreeMap::new(),
                spent: Spent::default(),
            },
            operators: AtomicU64::new(0),
        }
    }
}

impl<'p, N: Native> Member for Bound<'p, '_, N> {
    fn id(&self) -> FamilyId {
        self.id
    }

    fn tables(&self) -> &'static [TableId] {
        self.native.tables()
    }

    fn needs(&self) -> io::Result<Needs> {
        self.native.needs()
    }

    fn idle(&self, shape: &Shape) -> bool {
        shape
            .branches()
            .keys()
            .all(|branch| self.passes.binary_search(branch).is_ok())
    }

    fn lane<'a>(&'a mut self, walking: &'a Walking<'a>) -> Lane<'a, Walked<'a>> {
        let Self {
            id,
            native,
            passes,
            resumes,
            emit,
            operators,
        } = self;
        let native: &'a N = native;
        let passes: &'a Vec<u64> = passes;
        let resumes: &'a Vec<Resume<'p>> = resumes;
        let operators: &'a AtomicU64 = operators;
        let total = walking.measuring.then_some(operators);
        Lane::new(
            id.name(),
            move |segment: &Segment| {
                walking.job.clone().enter(Level::Normal);
                Ok(Timed {
                    operator: native.operator(segment)?,
                    spent: 0,
                    total,
                })
            },
            move |chunk: Chunk<'_, Output<N>>| emit.chunk(native, walking, chunk),
        )
        .reads(native.tables())
        .passes(passes)
        .resumes(resumes)
    }

    fn settle(&mut self) -> io::Result<()> {
        self.emit.stage.settle()
    }

    fn spent(&self) -> Spent {
        Spent {
            operators: self.operators.load(Ordering::Relaxed),
            ..self.emit.spent
        }
    }

    fn counts(&self) -> Vec<(&'static str, u64)> {
        self.native.counts()
    }
}

impl<N: Native> Emit<'_, '_, N> {
    /// Keeps a chunk of a segment's hours in its branch's state and, once the segment's last
    /// chunk is kept, records what the family made of it. A refusal of the hour an item is of
    /// ends the branch there, as one the family's operators make does.
    fn chunk(
        &mut self,
        native: &N,
        walking: &Walking<'_>,
        chunk: Chunk<'_, Output<N>>,
    ) -> io::Result<()> {
        walking.watch();
        let at = walking.shape.at(chunk.segment).map_err(segments::fatal)?;
        let branch = walking.shape.get(at).branch;
        if let Some((filled, full)) = self.full.take()
            && filled == at
            && !chunk.last
            && chunk.items.first().is_some_and(|item| item.hour() > full)
        {
            self.partial(native, walking, at, full)?;
        }
        let started = walking.measuring.then(cycles);
        if let Some(going) = self.going.remove(&branch) {
            let state = self.resume(native, going).map_err(resumed)?;
            self.states.insert(branch, state);
        }
        let (mut refused, mut through) = (None, None);
        for item in chunk.items {
            let hour = item.hour();
            if let Err(error) = self.observe(native, branch, item) {
                if error.kind() != io::ErrorKind::InvalidData {
                    return Err(alone(error));
                }
                refused = Some((hour, error));
                break;
            }
            through = Some(hour);
        }
        if let Some(started) = started {
            self.spent.emit += cycles().saturating_sub(started);
        }
        if let Some((hour, error)) = refused {
            let made = Made::Refused {
                hour,
                reason: error.to_string(),
                from: None,
            };
            self.processed(native, walking, at, made)?;
            return Err(segments::refused(hour, error));
        }
        if !chunk.last {
            let held = self.stage.series_held();
            if held >= roots::SERIES_BATCH_BYTES || (held > 0 && roots::series_pressed()) {
                self.full = through.map(|hour| (at, hour));
            }
            return Ok(());
        }
        let carry = match chunk.carry {
            None => None,
            Some(Carry::Saved(bytes)) => Some(Some(bytes.to_vec())),
            Some(Carry::Unsaved) => Some(None),
        };
        match carry {
            Some(carry) => self.carries.insert(branch, carry),
            None => self.carries.remove(&branch),
        };
        let made = match chunk.ending {
            None => Made::Prepared,
            Some(Ending::Refused { hour, error }) => Made::Refused {
                hour: *hour,
                reason: error.to_string(),
                from: None,
            },
            Some(Ending::Lost(loss)) => Made::Lost {
                reason: lost(loss),
                ends: true,
            },
        };
        self.processed(native, walking, at, made)
    }

    /// Records segment `at`'s hours through hour `through` as prepared, before its last chunk,
    /// and publishes them: a family whose series writers hold their share publishes what they
    /// hold rather than holding the segment whole. The segment's rest goes on from the hour after.
    /// Its operator's carry there is unknown, so its state is not saved, as within its branch.
    fn partial(
        &mut self,
        native: &N,
        walking: &Walking<'_>,
        at: usize,
        through: i32,
    ) -> io::Result<()> {
        let started = walking.measuring.then(cycles);
        let placed = *walking.shape.get(at);
        let (first, first_hour) = self.unprocessed(&placed)?;
        let last = u32::try_from(i64::from(through) - i64::from(placed.first_hour))
            .ok()
            .and_then(|past| placed.first.checked_add(past))
            .filter(|&last| (first..placed.last).contains(&last))
            .ok_or_else(|| io::Error::other("Hours outside what is left of their segment"))?;
        let next = last + 1;
        self.dirty.insert(placed.branch);
        self.open.insert(placed.branch);
        let hours = Hours {
            branch: placed.branch,
            run: placed.run,
            first,
            last,
            first_hour,
            last_hour: through,
            ends_segment: false,
            ends_branch: false,
            proof: walking.shape.proof_through(walking.recording, at, last),
        };
        // Hours short of their segment's end always end the batch.
        self.stage
            .processed(hours, Made::Prepared, None, None)
            .map_err(alone)?;
        self.joins.insert(placed.branch, (placed.run, next));
        self.queue(native, walking)?;
        if let Some(started) = started {
            self.spent.publish += cycles().saturating_sub(started);
        }
        Ok(())
    }

    /// The state of `going`'s branch, from what was saved with its committed publication.
    fn resume(&mut self, native: &N, going: &Going) -> io::Result<N::Branch> {
        let (branch, published) = (going.branch, &going.published);
        let saved = roots::state(self.stage.pages()?, self.name, branch, published)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "No state was saved"))?;
        native.resume(&mut self.stage, branch, published, &saved)
    }

    fn observe(&mut self, native: &N, branch: u64, item: Output<N>) -> io::Result<()> {
        let state = match self.states.entry(branch) {
            Entry::Occupied(state) => state.into_mut(),
            Entry::Vacant(vacant) => vacant.insert(native.branch(branch, item.hour())?),
        };
        native.observe(state, &mut self.stage, branch, item)
    }

    /// Records segment `at` as `made`, publishing the batch where the recording ends it.
    fn processed(
        &mut self,
        native: &N,
        walking: &Walking<'_>,
        at: usize,
        made: Made,
    ) -> io::Result<()> {
        let started = walking.measuring.then(cycles);
        let placed = *walking.shape.get(at);
        let whole = made == Made::Prepared;
        if whole && placed.ends_branch {
            self.complete.insert(placed.branch);
        }
        if !whole || placed.ends_branch {
            self.ended.insert(placed.branch);
        }
        if !whole {
            self.short.insert(placed.branch);
        }
        self.dirty.insert(placed.branch);
        let (first, first_hour) = self.unprocessed(&placed)?;
        let hours = Hours {
            branch: placed.branch,
            run: placed.run,
            first,
            last: placed.last,
            first_hour,
            last_hour: placed.last_hour,
            ends_segment: true,
            ends_branch: placed.ends_branch,
            proof: walking.shape.proof(walking.recording, at),
        };
        let ends = self
            .stage
            .processed(hours, made, None, None)
            .map_err(alone)?;
        if ends {
            self.queue(native, walking)?;
        }
        if let Some(started) = started {
            self.spent.publish += cycles().saturating_sub(started);
        }
        Ok(())
    }

    /// The first ordinal and hour of segment `placed` the family has not recorded: past its first
    /// where its branch goes on there from an earlier run, or its last batch ended there.
    fn unprocessed(&mut self, placed: &Placed) -> io::Result<(u32, i32)> {
        match self.joins.remove(&placed.branch) {
            Some((run, ordinal)) if run == placed.run && ordinal > placed.first => {
                let past = i32::try_from(ordinal - placed.first).map_err(io::Error::other)?;
                Ok((ordinal, placed.first_hour + past))
            }
            _ => Ok((placed.first, placed.first_hour)),
        }
    }

    /// Publishes every branch the batch touched, with the state each goes on with but where it
    /// ended short or partway through a segment, and hands the batch off to be sealed and
    /// committed while the lane goes on.
    fn queue(&mut self, native: &N, walking: &Walking<'_>) -> io::Result<()> {
        for branch in std::mem::take(&mut self.dirty) {
            let through = self
                .complete
                .remove(&branch)
                .then(|| walking.shape.branches().get(&branch))
                .flatten()
                .map(|extent| extent.last_hour);
            if let Some(state) = self.states.get_mut(&branch) {
                native
                    .flush(state, &mut self.stage, branch, through, walking.cancelled)
                    .map_err(alone)?;
            }
            let unsaved = self.short.remove(&branch) | self.open.remove(&branch);
            if !unsaved {
                let carry = self.carries.get(&branch);
                let saved = match self.states.get_mut(&branch) {
                    Some(state) if carry.is_none_or(Option::is_some) => {
                        native.save(state).map_err(alone)?
                    }
                    _ => None,
                };
                let carry = saved.as_ref().and(carry.cloned().flatten());
                self.stage.keep(branch, saved, carry).map_err(alone)?;
            }
            if self.ended.remove(&branch) {
                self.states.remove(&branch);
                self.carries.remove(&branch);
            }
        }
        self.stage.send().map_err(alone)
    }
}

/// Whether `error`, met by a family's emit, is the run's own: publication stopped, as when a
/// commit failed or the job was cancelled ([`publication::halted`]), or the job's cancellation.
fn of_the_run(error: &io::Error) -> bool {
    publication::halted(error) || error.kind() == io::ErrorKind::Interrupted
}

/// `error`, met by a family's emit in its stage or its own code as it keeps, flushes, saves,
/// seals or publishes its hours: the run's own ([`of_the_run`]), which stops every family; else
/// the family's alone, which ends it and no other, never as a refusal its emit did not date.
pub(crate) fn alone(error: io::Error) -> io::Error {
    if of_the_run(&error) {
        segments::fatal(error)
    } else if error.kind() == io::ErrorKind::InvalidData {
        io::Error::other(error.to_string())
    } else {
        error
    }
}

/// `error`, met taking up a family's saved state, as [`alone`] says.
fn resumed(error: io::Error) -> io::Error {
    if of_the_run(&error) {
        segments::fatal(error)
    } else {
        io::Error::other(format!("Its saved state could not be taken up: {error}"))
    }
}

/// Why a segment was lost.
pub(super) fn lost(loss: &Loss) -> String {
    match loss {
        Loss::Damaged(damage) => format!("The recording cannot be read here: {damage}"),
        Loss::After(before) => {
            format!("Its hours before here could not be read, from segment {before} of the run")
        }
    }
}
