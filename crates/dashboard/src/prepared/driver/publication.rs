//! Publication of the families the segment driver prepares. Each family's lane writes its batch
//! into a stage of its own, outside the root writer ([`Stage`]); one coordinator adopts each
//! lane's batch whole or not at all, and commits together the batches ready while it commits.
//! A batch carries the family's pages and, for each branch it touched, the family's ledger,
//! emit state, projector carry and coverage page, then its publication root ([`roots`]), last.
//! A lane cuts its family's series where each segment ends and hands it off: a sealer writes it
//! to the batch's stage at once ([`Stage::seal`]). At the batch's end the lane cuts the batch
//! and hands it off too: a sealer seals, stages and commits it, its root naming every segment
//! sealed before, while the lane appends the next ([`Stage::send`], [`Publisher::sealing`]).
//! The root's identity carries the family's version stamp, so opening a store keeps every family
//! whose stamp is unchanged, clears every other, which prepares again alone, and resumes each
//! family on each branch after its own last committed hour ([`Resume`]). Every error a stage
//! returns is its family's alone, but publication's stop ([`halted`]), which every lane meets.
//!
//! Batches end where the recording says ([`ends_batch`]): after a branch's 1st, 2nd, 4th, 8th
//! and 16th segment, then every 32nd, counted from the branch's start however preparation
//! started, resumed or grew; and earlier at a loss or refusal and at the branch's end. While the
//! recording grows, each arrival of committed hours of its open tail ends a batch: the family's
//! operators go on from their carried state hour by hour, its gate reaches the newest committed
//! hour, and a later run goes on after that hour, never repeating one.

use super::plan::{Owner, Plan};
use super::registry::{self, FamilyId};
use crate::prepared::pipeline;
use dashboard_base::prepared::{
    priority::Caller,
    roots::{self, Binding, Cursor, End, Held, Interval, Published},
};
use engine_allocation::labels::{self, Kind};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, VecDeque},
    io,
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
};

/// Segments of a branch a batch goes through at most.
const BATCH_SEGMENTS: u64 = 32;

/// Whether a batch ends after a branch's `through`th segment (counted from 1) with a budget of
/// `budget`: at every power of two below the budget, then at every multiple of it.
pub(crate) fn ends_batch(through: u64, budget: u64) -> bool {
    through != 0
        && ((through < budget && through.is_power_of_two()) || through.is_multiple_of(budget))
}

fn refused(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

/// A family the publisher prepares, under the version its registry entry gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Family {
    pub id: FamilyId,
    pub version: u32,
    /// Its version stamp ([`Plan::stamp`]): what its publications are kept under.
    pub stamp: u64,
}

impl Family {
    /// The families `plan` prepares of `names`, refusing one it lacks by name; a name of a
    /// recording-wide job beside the families is that job, prepared as one of them.
    pub(crate) fn planned(plan: &Plan, names: &[&str]) -> io::Result<Vec<Self>> {
        names
            .iter()
            .map(|&name| {
                let unknown = || {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("Unknown prepared family {name:?}"),
                    )
                };
                let (id, version) = match plan.family(name) {
                    Some(family) => (family.id, family.version),
                    None => {
                        let id = FamilyId::publisher(name)?;
                        let job = registry::job(id.name()).ok_or_else(unknown)?;
                        (id, job.version)
                    }
                };
                Ok(Self {
                    id,
                    version,
                    stamp: plan.stamp(name).ok_or_else(unknown)?,
                })
            })
            .collect()
    }
}

/// Hours of one segment a lane processed at once: the whole segment, or, while the recording
/// grows, the committed hours of its open tail that arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Hours {
    pub branch: u64,
    pub run: u32,
    /// Ordinals of the run, inclusive.
    pub first: u32,
    pub last: u32,
    pub first_hour: i32,
    pub last_hour: i32,
    /// They end their segment.
    pub ends_segment: bool,
    /// They end the branch as the recording stands.
    pub ends_branch: bool,
    /// Proof of the recorded input accepted through them.
    pub proof: [u8; 32],
}

/// What a family made of hours.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Made {
    Prepared,
    /// Prepared before `hour`, refused from it on: the family's branch ends there.
    Refused {
        hour: i32,
        reason: crate::QueryError,
        from: Option<String>,
    },
    /// None prepared. With `ends`, nothing of the branch after them is the family's either.
    Lost {
        reason: String,
        ends: bool,
    },
}

/// Why publication stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Stopped {
    Cancelled,
    Cleared,
    /// A commit failed without proving what it left: the roots found durable stand.
    Uncertain(Arc<str>),
    Failed(Arc<str>),
}

impl Stopped {
    fn error(&self) -> io::Error {
        let (kind, message): (io::ErrorKind, String) = match self {
            Self::Cancelled => (
                io::ErrorKind::Interrupted,
                "Preparation was cancelled".into(),
            ),
            Self::Cleared => (
                io::ErrorKind::Interrupted,
                "Prepared data is being cleared".into(),
            ),
            Self::Uncertain(message) => (
                io::ErrorKind::Other,
                format!(
                    "A commit's outcome was uncertain ({message}); preparation goes on from what \
                     it left durable"
                ),
            ),
            Self::Failed(message) => (io::ErrorKind::Other, message.to_string()),
        };
        io::Error::new(kind, Halted(message))
    }
}

/// Publication stopped ([`Stopped`]): what every lane meets alike, never one family's own.
#[derive(Debug)]
struct Halted(String);

impl std::fmt::Display for Halted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Halted {}

/// Whether `error` is publication's stop, which ends every family's lane: the run's own failure.
/// Any other error a stage returns is its family's alone.
pub(crate) fn halted(error: &io::Error) -> bool {
    error.get_ref().is_some_and(|inner| inner.is::<Halted>())
}

/// Where a family's own fault is injected ([`Publisher::fail_family`]).
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Own {
    /// Sealing its batch, as a series it wrote outside its coverage does.
    Seal,
    /// Adopting its sealed batch into the group commit.
    Adopt,
}

/// What a family's publication reached: from earlier runs, and committed or staged in this one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Progress {
    /// Segments its lane staged in this run, committed or not.
    pub emitted: u64,
    /// Committed: segments it processed and hours it prepared, over every branch.
    pub processed: u64,
    pub available: u64,
    /// Branches it published, and those where nothing more is its to prepare.
    pub branches: usize,
    pub terminal: usize,
}

/// Where a family's publication stands as a whole.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// No hour of it is queryable yet.
    Preparing,
    /// Some hours are queryable, and more are to come.
    Available,
    /// Nothing more is its to prepare of the recording as it stands.
    Terminal,
}

#[cfg(test)]
impl Progress {
    pub(crate) fn phase(&self) -> Phase {
        if self.branches > 0 && self.terminal == self.branches {
            Phase::Terminal
        } else if self.available > 0 {
            Phase::Available
        } else {
            Phase::Preparing
        }
    }
}

/// How a family takes a segment, as its committed publication leaves it ([`Resume::attach`]).
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Attach {
    /// It committed all of it, or its branch ended for it.
    Skip,
    Whole,
    /// It committed the segment's hours before this ordinal: it goes on from its carry there.
    From(u32),
}

/// Where each family goes on: its last committed frontier on each branch.
#[derive(Debug, Default)]
pub(crate) struct Resume {
    pub families: BTreeMap<FamilyId, BTreeMap<u64, roots::Frontier>>,
    /// Families prepared again from the start: their version, binding or accepted input changed.
    #[cfg(test)]
    pub again: Vec<FamilyId>,
    /// The store was replaced under the new key: nothing of what it held before is kept.
    pub replaced: bool,
}

#[cfg(test)]
impl Resume {
    /// How `family` takes the segment of run `run`, ordinals `first..=last`, on `branch`.
    pub(crate) fn attach(
        &self,
        family: FamilyId,
        branch: u64,
        run: u32,
        first: u32,
        last: u32,
    ) -> Attach {
        let Some(frontier) = self
            .families
            .get(&family)
            .and_then(|branches| branches.get(&branch))
        else {
            return Attach::Whole;
        };
        if frontier.end.is_some() {
            return Attach::Skip;
        }
        let next = frontier.next;
        match run.cmp(&next.run) {
            Ordering::Less => Attach::Skip,
            Ordering::Greater => Attach::Whole,
            Ordering::Equal if last < next.ordinal => Attach::Skip,
            Ordering::Equal if first >= next.ordinal => Attach::Whole,
            Ordering::Equal => Attach::From(next.ordinal),
        }
    }
}

/// What a sealed batch published.
#[derive(Default)]
struct Sealed {
    published: Vec<(u64, Published)>,
    /// The first publication it holds on each branch: one per segment it sealed.
    firsts: BTreeMap<u64, u64>,
    segments: u64,
    prepared: u64,
}

/// A family's sealed batch: its staged pages and what they publish.
struct Batch {
    family: FamilyId,
    staged: Vec<store::Staged>,
    sealed: Sealed,
}

/// Commits that end their transaction early, as a terminated process or a failed flush would.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fault {
    /// Adopted, then gone before the root was written.
    BeforeRoot,
    /// The root published, then an error, as from a flush after its rename.
    AfterRoot,
}

struct Inner {
    /// The root writer: None while a group commits, and once publication stopped.
    writer: Option<store::Writer>,
    committing: bool,
    stopped: Option<Stopped>,
    queue: Vec<(u64, Batch)>,
    outcomes: BTreeMap<u64, io::Result<()>>,
    tickets: u64,
    /// Stages lanes hold, whose handles keep the store's lock.
    stages: usize,
    committed: BTreeMap<(FamilyId, u64), Published>,
    progress: BTreeMap<FamilyId, Progress>,
    /// What each family's lane handed off, done in order ([`Publisher::sealing`]).
    lines: BTreeMap<FamilyId, Line>,
    /// Each batch's outcome, by the order it was handed in, until its lane takes it.
    finished: BTreeMap<u64, io::Result<()>>,
    /// Pieces of work handed off, counted.
    handed: u64,
    /// Sealers do the lines' work ([`Publisher::sealing`]).
    sealing: bool,
}

/// The one root writer of a prepared store, and the coordinator of every family's batches.
pub(crate) struct Publisher<'s> {
    store: &'s store::Store,
    key: store::SourceKey,
    binding: Binding,
    plan: &'static Plan,
    families: Vec<Family>,
    inner: Mutex<Inner>,
    changed: Condvar,
    on_commit: Option<Box<dyn Fn() + Send + Sync + 's>>,
    #[cfg(test)]
    fault: Mutex<Option<Fault>>,
    /// Each family's fault to come, and how many of those steps pass before it.
    #[cfg(test)]
    owned: Mutex<Vec<(FamilyId, Own, usize)>>,
}

impl<'s> Publisher<'s> {
    /// Opens `store` under `key` for `families`. A family whose publication the store holds
    /// under another stamp or binding, whose root cannot be read, or whose accepted input
    /// `accepts` no longer finds in the recording, has every section it owns cleared in a first
    /// commit and prepares again alone; every other keeps what it committed.
    pub(crate) fn open(
        store: &'s store::Store,
        key: store::SourceKey,
        binding: Binding,
        plan: &'static Plan,
        families: &[Family],
        accepts: &dyn Fn(FamilyId, u64, &roots::Frontier) -> bool,
    ) -> io::Result<(Self, Resume)> {
        let (mut writer, replaced) = match store.saved_key()? {
            Some(saved) if saved == key => (store.extend(&key)?, false),
            // Content bound otherwise, as when its game language changed, prepares anew.
            Some(saved) if key.appends(&saved) => (store.append(&saved, key.clone())?, false),
            _ => (store.replace(key.clone())?, true),
        };
        let found: Vec<(String, u64)> = writer
            .section_names()
            .filter_map(roots::root_of)
            .map(|(family, branch)| (family.to_owned(), branch))
            .collect();
        let mut again: Vec<FamilyId> = Vec::new();
        let mut kept = BTreeMap::new();
        for (name, branch) in found {
            let Some(family) = families.iter().find(|family| family.id.name() == name) else {
                continue;
            };
            if again.contains(&family.id) {
                continue;
            }
            let published = match roots::read(&mut writer, &name, branch) {
                Ok(Some(published)) => published,
                Ok(None) => continue,
                Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                    again.push(family.id);
                    continue;
                }
                Err(error) => return Err(error),
            };
            if published.identity == binding.identity(branch, family.stamp)
                && published.frontier.version == family.version
                && accepts(family.id, branch, &published.frontier)
            {
                kept.insert((family.id, branch), published);
            } else {
                again.push(family.id);
            }
        }
        kept.retain(|(family, _), _| !again.contains(family));
        if !again.is_empty() {
            let owned: Vec<String> = writer
                .section_names()
                .filter(|section| match plan.owner(section) {
                    Some(Owner::Family(id)) => again.contains(&id),
                    Some(Owner::Job(job)) => again.iter().any(|id| id.name() == job),
                    _ => false,
                })
                .map(str::to_owned)
                .collect();
            for section in owned {
                writer.clear(&section)?;
            }
            writer = writer.commit_continue()?;
        }
        let mut progress: BTreeMap<FamilyId, Progress> = BTreeMap::new();
        let mut resume = Resume {
            #[cfg(test)]
            again,
            replaced,
            ..Resume::default()
        };
        for ((family, branch), published) in &kept {
            let counted = progress.entry(*family).or_default();
            counted.processed += published.frontier.segments;
            counted.available += published.frontier.prepared;
            resume
                .families
                .entry(*family)
                .or_default()
                .insert(*branch, published.frontier.clone());
        }
        Ok((
            Self {
                store,
                key,
                binding,
                plan,
                families: families.to_vec(),
                inner: Mutex::new(Inner {
                    writer: Some(writer),
                    committing: false,
                    stopped: None,
                    queue: Vec::new(),
                    outcomes: BTreeMap::new(),
                    tickets: 0,
                    stages: 0,
                    committed: kept,
                    progress,
                    lines: BTreeMap::new(),
                    finished: BTreeMap::new(),
                    handed: 0,
                    sealing: false,
                }),
                changed: Condvar::new(),
                on_commit: None,
                #[cfg(test)]
                fault: Mutex::new(None),
                #[cfg(test)]
                owned: Mutex::new(Vec::new()),
            },
            resume,
        ))
    }

    /// Calls `on_commit` after every commit, once what it published is proven durable.
    pub(crate) fn with_commits(mut self, on_commit: impl Fn() + Send + Sync + 's) -> Self {
        self.on_commit = Some(Box::new(on_commit));
        self
    }

    #[cfg(test)]
    pub(crate) fn fail_next_commit(&self, fault: Fault) {
        *self.fault.lock().unwrap_or_else(PoisonError::into_inner) = Some(fault);
    }

    /// Fails `family`'s step `at` once `after` more of them passed.
    #[cfg(test)]
    pub(crate) fn fail_family(&self, family: FamilyId, at: Own, after: usize) {
        self.owned
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((family, at, after));
    }

    /// Whether `family`'s step `at` fails now.
    #[cfg(test)]
    fn faulted(&self, family: FamilyId, at: Own) -> bool {
        let mut owned = self.owned.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(found) = owned
            .iter()
            .position(|&(faulty, step, _)| (faulty, step) == (family, at))
        else {
            return false;
        };
        if owned[found].2 > 0 {
            owned[found].2 -= 1;
            return false;
        }
        owned.remove(found);
        true
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait_changed<'g>(&'g self, inner: MutexGuard<'g, Inner>) -> MutexGuard<'g, Inner> {
        self.changed
            .wait(inner)
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The lane of `family`'s batches.
    pub(crate) fn stage(&self, family: FamilyId) -> io::Result<Stage<'_, 's>> {
        let family = *self
            .families
            .iter()
            .find(|planned| planned.id == family)
            .ok_or_else(|| refused("A stage of a family the publisher does not prepare"))?;
        Ok(Stage {
            publisher: self,
            family,
            writer: None,
            handed: None,
            steps: BTreeMap::new(),
            branches: BTreeMap::new(),
            ends: false,
        })
    }

    /// `family`'s last committed publication on `branch`.
    pub(crate) fn committed(&self, family: FamilyId, branch: u64) -> Option<Published> {
        self.lock().committed.get(&(family, branch)).cloned()
    }

    /// Each family's progress.
    #[cfg(test)]
    pub(crate) fn progress(&self) -> BTreeMap<FamilyId, Progress> {
        let inner = self.lock();
        let mut all = inner.progress.clone();
        for ((family, _), published) in &inner.committed {
            let counted = all.entry(*family).or_default();
            counted.branches += 1;
            counted.terminal += usize::from(published.frontier.terminal());
        }
        all
    }

    /// Why publication stopped; None while it goes on.
    #[cfg(test)]
    pub(crate) fn stopped(&self) -> Option<Stopped> {
        self.lock().stopped.clone()
    }

    /// Stops publication for `stopped`: no stage forks and no batch commits after it, and the
    /// batches queued are dropped. Returns once the commit under way and every lane's stage are
    /// gone, having dropped the root writer: no handle of it holds the store, which a Clear may
    /// then remove. Lanes see the stop at their next call.
    pub(crate) fn stop(&self, stopped: Stopped) {
        let mut inner = self.lock();
        let stopped = inner.stopped.get_or_insert(stopped).clone();
        inner.queue.clear();
        // Work no one took commits nothing now; its stages go, as do those of idle lines.
        let mut unsealed = Vec::new();
        let mut idle = Vec::new();
        let state = &mut *inner;
        for line in state.lines.values_mut() {
            for (order, work) in line.work.drain(..) {
                if matches!(work, Work::Batch(_)) {
                    state.finished.insert(order, Err(stopped.error()));
                }
                unsealed.push(work);
            }
            if !line.busy {
                idle.push(std::mem::take(&mut line.kept));
            }
        }
        drop(inner);
        for work in unsealed {
            self.dropped(work);
        }
        for held in idle {
            self.let_go(held);
        }
        let mut inner = self.lock();
        self.changed.notify_all();
        while inner.committing || inner.stages > 0 {
            inner = self.wait_changed(inner);
        }
        inner.writer = None;
        drop(inner);
        self.changed.notify_all();
    }

    /// Writes what the coordinator itself publishes, as `write` does, with the root writer and
    /// commits it once no group commit is under way. Its failure stops publication.
    pub(crate) fn coordinate(
        &self,
        write: impl FnOnce(&mut store::Writer) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut inner = self.lock();
        let mut writer = loop {
            if let Some(stopped) = &inner.stopped {
                return Err(stopped.error());
            }
            if !inner.committing
                && let Some(writer) = inner.writer.take()
            {
                break writer;
            }
            inner = self.wait_changed(inner);
        };
        inner.committing = true;
        drop(inner);
        let mut leading = Leading(self, true);
        let span = replay_frame::timeline::span("commit");
        let committed = write(&mut writer).and_then(|()| self.commit_writer(writer));
        drop(span);
        leading.1 = false;
        let mut inner = self.lock();
        inner.committing = false;
        let done = match committed {
            Ok(next) => {
                inner.writer = Some(next);
                Ok(())
            }
            Err(error) => {
                inner
                    .stopped
                    .get_or_insert(Stopped::Failed(Arc::from(error.to_string())));
                inner.queue.clear();
                Err(error)
            }
        };
        drop(inner);
        self.changed.notify_all();
        done
    }

    fn fork(&self, family: FamilyId) -> io::Result<store::Writer> {
        let mut inner = self.lock();
        loop {
            if let Some(stopped) = &inner.stopped {
                return Err(stopped.error());
            }
            if let Some(writer) = inner.writer.as_mut() {
                let mut stage = writer.stage()?;
                stage.guard_sections(self.plan.guard(family.name()));
                inner.stages += 1;
                return Ok(stage);
            }
            inner = self.wait_changed(inner);
        }
    }

    fn released(&self) {
        let mut inner = self.lock();
        inner.stages = inner.stages.saturating_sub(1);
        drop(inner);
        self.changed.notify_all();
    }

    fn queue(&self, batch: Batch) -> io::Result<u64> {
        let mut inner = self.lock();
        if let Some(stopped) = &inner.stopped {
            let error = stopped.error();
            drop(inner);
            drop(batch);
            return Err(error);
        }
        let ticket = inner.tickets;
        inner.tickets += 1;
        inner.queue.push((ticket, batch));
        drop(inner);
        self.changed.notify_all();
        Ok(ticket)
    }

    /// Waits for the batch queued under `ticket` to be committed, leading the commit of every
    /// batch queued with it when no other does. Its own adoption's or validation's failure is
    /// its alone; the others' batches still commit.
    pub(crate) fn wait(&self, ticket: u64) -> io::Result<()> {
        let mut inner = self.lock();
        loop {
            if let Some(outcome) = inner.outcomes.remove(&ticket) {
                return outcome;
            }
            let queued = inner.queue.iter().any(|(queued, _)| *queued == ticket);
            if !queued && !inner.committing {
                return Err(inner.stopped.as_ref().map_or_else(
                    || io::Error::other("A queued batch was dropped"),
                    Stopped::error,
                ));
            }
            if queued && !inner.committing && inner.writer.is_some() {
                inner = self.lead(inner);
                continue;
            }
            inner = self.wait_changed(inner);
        }
    }

    /// Commits every queued batch that follows its family's last, together.
    fn lead<'g>(&'g self, mut inner: MutexGuard<'g, Inner>) -> MutexGuard<'g, Inner> {
        let Some(writer) = inner.writer.take() else {
            return inner;
        };
        inner.committing = true;
        let mut generations: BTreeMap<(FamilyId, u64), u64> = BTreeMap::new();
        let mut group = Vec::new();
        for (ticket, batch) in std::mem::take(&mut inner.queue) {
            let follows = batch.sealed.published.iter().all(|(branch, published)| {
                let key = (batch.family, *branch);
                let last = generations.get(&key).copied().or_else(|| {
                    inner
                        .committed
                        .get(&key)
                        .map(|committed| committed.publication.generation)
                });
                let first = batch.sealed.firsts.get(branch).copied();
                first.or(Some(published.publication.generation)) == last.unwrap_or(0).checked_add(1)
            });
            if follows {
                for (branch, published) in &batch.sealed.published {
                    generations.insert((batch.family, *branch), published.publication.generation);
                }
                group.push((ticket, batch));
            } else {
                inner.outcomes.insert(
                    ticket,
                    Err(refused(
                        "A family's batch does not follow its last committed one",
                    )),
                );
            }
        }
        drop(inner);
        let mut leading = Leading(self, true);
        let done = self.commit_group(writer, group);
        leading.1 = false;
        let mut inner = self.lock();
        for (ticket, outcome) in done.outcomes {
            inner.outcomes.insert(ticket, outcome);
        }
        let durable = !done.durable.is_empty();
        for (family, sealed) in done.durable {
            let counted = inner.progress.entry(family).or_default();
            counted.processed += sealed.segments;
            counted.available += sealed.prepared;
            for (branch, published) in sealed.published {
                inner.committed.insert((family, branch), published);
            }
        }
        inner.writer = done.writer;
        if let Some(stopped) = done.stopped {
            inner.stopped.get_or_insert(stopped);
            inner.queue.clear();
        }
        inner.committing = false;
        drop(inner);
        self.changed.notify_all();
        if durable && let Some(on_commit) = &self.on_commit {
            on_commit();
        }
        self.lock()
    }

    fn commit_group(&self, mut writer: store::Writer, group: Vec<(u64, Batch)>) -> Done {
        let _span = replay_frame::timeline::span("commit");
        let mut outcomes = Vec::new();
        let mut adopted = Vec::new();
        for (ticket, batch) in group {
            #[cfg(test)]
            if self.faulted(batch.family, Own::Adopt) {
                outcomes.push((ticket, Err(io::Error::other("Simulated adoption failure"))));
                continue;
            }
            match writer.adopt_all(batch.staged) {
                Ok(()) => adopted.push((ticket, batch.family, batch.sealed)),
                Err(error) => outcomes.push((ticket, Err(error))),
            }
        }
        if adopted.is_empty() {
            return Done {
                writer: Some(writer),
                outcomes,
                durable: Vec::new(),
                stopped: None,
            };
        }
        match self.commit_writer(writer) {
            Ok(next) => {
                outcomes.extend(adopted.iter().map(|(ticket, _, _)| (*ticket, Ok(()))));
                Done {
                    writer: Some(next),
                    outcomes,
                    durable: adopted
                        .into_iter()
                        .map(|(_, family, sealed)| (family, sealed))
                        .collect(),
                    stopped: None,
                }
            }
            Err(error) => {
                // The new root may be visible all the same: only what is read back stands.
                let message: Arc<str> = Arc::from(error.to_string());
                let proven = self.proven(&adopted);
                let mut durable = Vec::new();
                for ((ticket, family, sealed), proven) in adopted.into_iter().zip(proven) {
                    if proven {
                        outcomes.push((ticket, Ok(())));
                        durable.push((family, sealed));
                    } else {
                        outcomes.push((ticket, Err(Stopped::Uncertain(message.clone()).error())));
                    }
                }
                Done {
                    writer: None,
                    outcomes,
                    durable,
                    stopped: Some(Stopped::Uncertain(message)),
                }
            }
        }
    }

    fn commit_writer(&self, writer: store::Writer) -> io::Result<store::Writer> {
        #[cfg(test)]
        match self
            .fault
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            Some(Fault::BeforeRoot) => {
                drop(writer);
                return Err(io::Error::other("Simulated termination before the root"));
            }
            Some(Fault::AfterRoot) => {
                writer.commit()?;
                return Err(io::Error::other("Simulated flush failure after the root"));
            }
            None => {}
        }
        writer.commit_continue()
    }

    /// For each adopted batch, whether the store's current manifest holds every root it
    /// published.
    fn proven(&self, adopted: &[(u64, FamilyId, Sealed)]) -> Vec<bool> {
        let snapshot = match self.store.session(&self.key) {
            Ok(Some(session)) => session.snapshot(),
            _ => return vec![false; adopted.len()],
        };
        adopted
            .iter()
            .map(|(_, family, sealed)| {
                sealed.published.iter().all(|(branch, published)| {
                    let mut view = &snapshot;
                    matches!(
                        roots::read(&mut view, family.name(), *branch),
                        Ok(Some(found))
                            if found.root == published.root
                                && found.publication == published.publication
                    )
                })
            })
            .collect()
    }
}

/// What a group commit left.
struct Done {
    writer: Option<store::Writer>,
    outcomes: Vec<(u64, io::Result<()>)>,
    durable: Vec<(FamilyId, Sealed)>,
    stopped: Option<Stopped>,
}

/// Leads a group commit; one that unwinds stops publication and releases those waiting.
struct Leading<'g, 's>(&'g Publisher<'s>, bool);

impl Drop for Leading<'_, '_> {
    fn drop(&mut self) {
        if self.1 {
            let mut inner = self.0.lock();
            inner.committing = false;
            inner
                .stopped
                .get_or_insert(Stopped::Failed(Arc::from("A commit panicked")));
            inner.queue.clear();
            drop(inner);
            self.0.changed.notify_all();
        }
    }
}

/// A family's batch as its lane cut it: what a sealer, or the lane itself, seals, stages and
/// queues for the next group commit ([`Publisher::seal`]).
struct Job {
    family: Family,
    /// The stage the lane read the family's publications and wrote its own pages in, once it
    /// needed one.
    writer: Option<store::Writer>,
    sealings: Vec<Sealing>,
    segments: u64,
    prepared: u64,
}

/// A branch's step of a job: its cut, and the series segment cut with it; with the branch's
/// hierarchy while the family's line does not hold it yet.
struct Sealing {
    branch: u64,
    cut: roots::Cut,
    pending: dashboard_series::Pending<'static>,
    tree: Option<dashboard_series::Tree>,
}

/// What a family's lane hands off ([`Publisher::hand`]).
enum Work {
    /// A branch's series segment, cut where its recording segment ended within the batch:
    /// sealed at once, so its rows leave memory, and named by the batch's root.
    Segment {
        branch: u64,
        pending: dashboard_series::Pending<'static>,
        tree: Option<dashboard_series::Tree>,
    },
    /// The batch's end: sealed, staged and committed.
    Batch(Job),
}

/// What a family's line keeps from one piece of its work to the next.
#[derive(Default)]
struct Kept {
    /// The stage the open batch's segments were sealed in: forked once the batch before it was
    /// committed.
    writer: Option<store::Writer>,
    /// Each branch's series hierarchy, which its segments are added to in order.
    trees: BTreeMap<u64, dashboard_series::Tree>,
    /// The first publication the open batch sealed on each branch.
    firsts: BTreeMap<u64, u64>,
    /// Why sealing the open batch failed, which its end meets.
    failed: Option<io::Error>,
}

/// A family's work handed off: done in the order it was handed, one piece at a time.
struct Line {
    family: Family,
    /// Each piece, by the order it was handed in.
    work: VecDeque<(u64, Work)>,
    /// A sealer, or the lane, does its next piece, holding what it keeps.
    busy: bool,
    kept: Kept,
}

/// The stage a job seals in: its writer closes before the stage is released, however the job
/// ends, so a stop that sees every stage released finds the store's handles closed.
struct Forked<'p, 's> {
    publisher: &'p Publisher<'s>,
    writer: Option<store::Writer>,
}

impl Forked<'_, '_> {
    /// Its writer's batches; its stage is released.
    fn staged(&mut self) -> io::Result<Vec<store::Staged>> {
        let Some(writer) = self.writer.take() else {
            return Ok(Vec::new());
        };
        let staged = writer.into_staged();
        self.publisher.released();
        staged
    }
}

impl Drop for Forked<'_, '_> {
    fn drop(&mut self) {
        let writer = self.writer.take();
        if writer.is_some() {
            drop(writer);
            self.publisher.released();
        }
    }
}

/// Stops the sealers once the work beside them ends, however it ends.
struct Sealers<'p, 's>(&'p Publisher<'s>);

impl Drop for Sealers<'_, '_> {
    fn drop(&mut self) {
        self.0.lock().sealing = false;
        self.0.changed.notify_all();
    }
}

impl<'s> Publisher<'s> {
    /// Runs `work` beside `sealers` threads that do what lanes hand off ([`Stage::seal`],
    /// [`Stage::send`]), each lane's in order: a lane goes on appending while its segments seal
    /// and its last batch commits. What no sealer took when its lane needs it, the lane does.
    pub(crate) fn sealing<R>(&self, sealers: usize, work: impl FnOnce() -> R) -> R {
        self.lock().sealing = true;
        std::thread::scope(|scope| {
            let stop = Sealers(self);
            let caller = Caller::current();
            for _ in 0..sealers {
                let caller = caller.clone();
                job_priority::spawn_in(scope, move || {
                    caller.enter();
                    let _memory = labels::enter(labels::Owner::RUN, Kind::Publish);
                    self.serve();
                });
            }
            let done = work();
            drop(stop);
            done
        })
    }

    /// Does the work lanes handed off, the piece handed first among the lines no one works on
    /// next, until sealing stops.
    fn serve(&self) {
        let mut inner = self.lock();
        while inner.sealing {
            let next = inner
                .lines
                .iter()
                .filter(|(_, line)| !line.busy)
                .filter_map(|(family, line)| line.work.front().map(|(order, _)| (*order, *family)))
                .min();
            inner = match next {
                Some((_, family)) => self.step(inner, family),
                None => self.wait_changed(inner),
            };
        }
    }

    /// Hands `work` off after what `family`'s lane handed before; the order it was handed in,
    /// which names a batch. After a stop it is dropped at once, and a batch fails.
    fn hand(&self, family: Family, work: Work) -> u64 {
        let mut inner = self.lock();
        let order = inner.handed;
        inner.handed += 1;
        if let Some(stopped) = &inner.stopped {
            let error = stopped.error();
            if matches!(work, Work::Batch(_)) {
                inner.finished.insert(order, Err(error));
            }
            drop(inner);
            self.dropped(work);
            return order;
        }
        inner
            .lines
            .entry(family.id)
            .or_insert_with(|| Line {
                family,
                work: VecDeque::new(),
                busy: false,
                kept: Kept::default(),
            })
            .work
            .push_back((order, work));
        drop(inner);
        self.changed.notify_all();
        order
    }

    /// Lets go of work never done: its batch's stage.
    fn dropped(&self, work: Work) {
        if let Work::Batch(Job {
            writer: Some(writer),
            ..
        }) = work
        {
            drop(writer);
            self.released();
        }
    }

    /// Lets go of what a line kept: its seal stage.
    fn let_go(&self, kept: Kept) {
        if let Some(writer) = kept.writer {
            drop(writer);
            self.released();
        }
    }

    /// Does the next piece of `family`'s line, the lock let go meanwhile.
    fn step<'g>(
        &'g self,
        mut inner: MutexGuard<'g, Inner>,
        family: FamilyId,
    ) -> MutexGuard<'g, Inner> {
        let Some(line) = inner.lines.get_mut(&family) else {
            return inner;
        };
        let Some((order, work)) = line.work.pop_front() else {
            return inner;
        };
        line.busy = true;
        let planned = line.family;
        let mut kept = std::mem::take(&mut line.kept);
        drop(inner);
        let outcome = self.work(planned, &mut kept, work);
        inner = self.lock();
        let state = &mut *inner;
        if let Some(outcome) = outcome {
            state.finished.insert(order, outcome);
        }
        let gone = match state.lines.get_mut(&family) {
            Some(line) => {
                line.busy = false;
                if state.stopped.is_none() {
                    line.kept = kept;
                    None
                } else {
                    Some(kept)
                }
            }
            None => Some(kept),
        };
        self.changed.notify_all();
        if let Some(kept) = gone {
            drop(inner);
            self.let_go(kept);
            inner = self.lock();
        }
        inner
    }

    /// Does `work` of `family`'s line with what the line keeps: a batch's outcome. A panic fails
    /// its batch alone.
    fn work(&self, family: Family, kept: &mut Kept, work: Work) -> Option<io::Result<()>> {
        let name = family.id.name();
        let _memory = labels::enter(labels::owner(name), Kind::Seal);
        match work {
            Work::Segment {
                branch,
                pending,
                tree,
            } => {
                let _span = replay_frame::timeline::span("seal segment").lane(name);
                if kept.failed.is_none() {
                    let sealed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        self.segment(family, kept, branch, &pending, tree)
                    }))
                    .unwrap_or_else(|_| Err(io::Error::other("A segment's seal panicked")));
                    if let Err(error) = sealed {
                        kept.failed = Some(error);
                    }
                }
                None
            }
            Work::Batch(job) => {
                let _span = replay_frame::timeline::span("seal").lane(name);
                let outcome =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.seal(job, kept)))
                        .unwrap_or_else(|_| Err(io::Error::other("A batch's seal panicked")));
                if outcome.is_err() {
                    // What its segments added to the hierarchies was never committed.
                    kept.trees.clear();
                    kept.failed = Some(io::Error::other("The family's batch before failed"));
                }
                Some(outcome)
            }
        }
    }

    /// Seals `pending`, `family`'s series segment on `branch`, into the stage its batch's
    /// segments are sealed in.
    fn segment(
        &self,
        family: Family,
        kept: &mut Kept,
        branch: u64,
        pending: &dashboard_series::Pending<'static>,
        tree: Option<dashboard_series::Tree>,
    ) -> io::Result<()> {
        if let Some(tree) = tree {
            kept.trees.insert(branch, tree);
        }
        if kept.writer.is_none() {
            kept.writer = Some(self.fork(family.id)?);
        }
        let (Some(writer), Some(tree)) = (kept.writer.as_mut(), kept.trees.get_mut(&branch)) else {
            return Err(io::Error::other("A family's series hierarchy is missing"));
        };
        roots::seal(writer, family.id.name(), branch, pending, tree)?;
        kept.firsts
            .entry(branch)
            .or_insert(pending.publication().generation);
        Ok(())
    }

    /// The outcome of batch `id` of `family`'s lane, doing its line's work up to it here when no
    /// sealer works on it.
    fn finish(&self, family: FamilyId, id: u64) -> io::Result<()> {
        let mut inner = self.lock();
        loop {
            if let Some(outcome) = inner.finished.remove(&id) {
                return outcome;
            }
            let (busy, next) = inner.lines.get(&family).map_or((false, None), |line| {
                (line.busy, line.work.front().map(|(order, _)| *order))
            });
            inner = match next {
                _ if busy => self.wait_changed(inner),
                Some(order) if order <= id => self.step(inner, family),
                _ => return Err(io::Error::other("A handed-off batch was lost")),
            };
        }
    }

    /// Drops what `family`'s lane handed off that no one took, once the work under way on its
    /// line is done, with the outcome of `handed`, its last batch.
    fn abandon(&self, family: FamilyId, handed: Option<u64>) {
        let mut inner = self.lock();
        while inner.lines.get(&family).is_some_and(|line| line.busy) {
            inner = self.wait_changed(inner);
        }
        let line = inner.lines.remove(&family);
        if let Some(id) = handed {
            inner.finished.remove(&id);
        }
        drop(inner);
        if let Some(line) = line {
            for (_, work) in line.work {
                self.dropped(work);
            }
            self.let_go(line.kept);
        }
        self.changed.notify_all();
    }

    /// Seals `job` after the segments its batch sealed before: each branch's ledger, state,
    /// carry and last series segment, then its root, and its coverage page; stages it behind
    /// the lane's own stage, queues it for the next group commit and waits for that, leading it
    /// when no other does.
    fn seal(&self, job: Job, kept: &mut Kept) -> io::Result<()> {
        let Job {
            family,
            writer,
            sealings,
            segments,
            prepared,
        } = job;
        let mut lane = Forked {
            publisher: self,
            writer,
        };
        let mut seals = Forked {
            publisher: self,
            writer: kept.writer.take(),
        };
        let firsts = std::mem::take(&mut kept.firsts);
        if let Some(error) = kept.failed.take() {
            return Err(error);
        }
        #[cfg(test)]
        if self.faulted(family.id, Own::Seal) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Simulated seal failure"));
        }
        // A batch that sealed no segment publishes in the lane's own stage.
        if seals.writer.is_none() {
            seals.writer = lane.writer.take();
        }
        if seals.writer.is_none() {
            seals.writer = Some(self.fork(family.id)?);
        }
        let writer = seals
            .writer
            .as_mut()
            .ok_or_else(|| io::Error::other("A job lost its stage"))?;
        let name = family.id.name();
        let mut sealed = Sealed {
            published: Vec::with_capacity(sealings.len()),
            firsts: BTreeMap::new(),
            segments,
            prepared,
        };
        for sealing in sealings {
            let Sealing {
                branch,
                cut,
                pending,
                tree,
            } = sealing;
            if let Some(tree) = tree {
                kept.trees.insert(branch, tree);
            }
            let tree = kept
                .trees
                .get_mut(&branch)
                .ok_or_else(|| io::Error::other("A family's series hierarchy is missing"))?;
            let first = firsts
                .get(&branch)
                .copied()
                .unwrap_or(cut.publication.generation);
            let ended = cut.frontier.end.is_some();
            let identity = self.binding.identity(branch, family.stamp);
            let published = roots::write(writer, name, branch, identity, cut, &pending, tree)?;
            drop(pending);
            pipeline::write_family_page(&mut *writer, name, branch, &published.frontier)?;
            if ended {
                // Nothing goes on from a branch that ended.
                kept.trees.remove(&branch);
            }
            sealed.firsts.insert(branch, first);
            sealed.published.push((branch, published));
        }
        // The lane's reads of the family's publications come before what replaces them.
        let mut staged = lane.staged()?;
        staged.extend(seals.staged()?);
        let ticket = self.queue(Batch {
            family: family.id,
            staged,
            sealed,
        })?;
        self.wait(ticket)
    }
}

/// What a family did on a branch in its open batch.
struct Step {
    /// Where the batch leaves it; None before its first hours of the branch.
    frontier: Option<roots::Frontier>,
    intervals: Vec<Interval>,
    state: Option<Vec<u8>>,
    carry: Option<Vec<u8>>,
    segments: u64,
    prepared: u64,
}

/// What a family's lane holds of a branch from batch to batch.
struct Branch {
    /// The family's last publication on the branch as the lane cut it: committed, or sealing
    /// in the job the lane handed off last. None before its first.
    last: Option<roots::Last>,
    /// The committed publication the branch was taken up from, until its series writer goes
    /// on from it.
    resumes: Option<Published>,
    /// The family's output on the series engine.
    series: Option<roots::Series>,
    /// Its hierarchy, until the first segment it cuts takes it to the family's line.
    tree: Option<dashboard_series::Tree>,
}

/// A family's lane: its open batch, written into a stage of its own once it needs one.
pub(crate) struct Stage<'p, 's> {
    publisher: &'p Publisher<'s>,
    family: Family,
    writer: Option<store::Writer>,
    /// Its batch handed off to the sealers and not yet finished: the next forks once it is.
    handed: Option<u64>,
    steps: BTreeMap<u64, Step>,
    branches: BTreeMap<u64, Branch>,
    ends: bool,
}

impl Stage<'_, '_> {
    /// The batch's stage: forked once the batch before is committed.
    fn writer(&mut self) -> io::Result<&mut store::Writer> {
        self.settle()?;
        if self.writer.is_none() {
            self.writer = Some(self.publisher.fork(self.family.id)?);
        }
        self.writer
            .as_mut()
            .ok_or_else(|| io::Error::other("A stage is missing its writer"))
    }

    /// The stage the family's pages go into; it refuses sections the family does not own.
    pub(crate) fn pages(&mut self) -> io::Result<&mut store::Writer> {
        self.writer()
    }

    /// The name of the family whose lane it is.
    pub(crate) fn family(&self) -> &'static str {
        self.family.id.name()
    }

    /// What the lane holds of `branch`: taken up from the family's committed publication there
    /// when the lane first meets it.
    fn branch(&mut self, branch: u64) -> io::Result<&mut Branch> {
        if !self.branches.contains_key(&branch) {
            let committed = self.publisher.committed(self.family.id, branch);
            let last = match &committed {
                Some(published) => {
                    if published.identity
                        != self.publisher.binding.identity(branch, self.family.stamp)
                    {
                        return Err(refused(
                            "A family's publication goes on under another identity",
                        ));
                    }
                    let name = self.family.id.name();
                    Some(roots::Last::of(self.writer()?, name, branch, published)?)
                }
                None => None,
            };
            self.branches.insert(
                branch,
                Branch {
                    last,
                    resumes: committed,
                    series: None,
                    tree: None,
                },
            );
        }
        self.branches
            .get_mut(&branch)
            .ok_or_else(|| io::Error::other("A stage lost a branch"))
    }

    /// The family's open batch on `branch`, following its last publication there.
    fn step(&mut self, branch: u64) -> io::Result<&mut Step> {
        if !self.steps.contains_key(&branch) {
            let frontier = self
                .branch(branch)?
                .last
                .as_ref()
                .map(|last| last.frontier.clone());
            self.steps.insert(
                branch,
                Step {
                    frontier,
                    intervals: Vec::new(),
                    state: None,
                    carry: None,
                    segments: 0,
                    prepared: 0,
                },
            );
        }
        self.steps
            .get_mut(&branch)
            .ok_or_else(|| io::Error::other("A stage lost a step"))
    }

    /// Whether the series writer of `branch` holds anything appended since it was last cut.
    pub(crate) fn series_holds(&self, branch: u64) -> bool {
        self.branches
            .get(&branch)
            .and_then(|branch| branch.series.as_ref())
            .is_some_and(roots::Series::holds)
    }

    /// The series writer the family's output on `branch` goes into, its hours starting at
    /// `first_hour` when the branch has no publication yet; the batch's cut ends its segment
    /// once the family's emit appended what is open through its last hour.
    pub(crate) fn series(
        &mut self,
        branch: u64,
        first_hour: i32,
    ) -> io::Result<&mut roots::Series> {
        let _memory = labels::kind(Kind::Publish);
        if self.branch(branch)?.series.is_none() {
            let identity = self.publisher.binding.identity(branch, self.family.stamp);
            let resumes = self.branch(branch)?.resumes.take();
            let mut series = match &resumes {
                Some(published) => {
                    let name = self.family.id.name();
                    roots::writer(
                        self.writer()?,
                        name,
                        branch,
                        identity,
                        Some(published),
                        first_hour,
                    )?
                }
                None => roots::fresh(identity, first_hour)?,
            };
            let tree = series.lend_tree()?;
            let held = self.branch(branch)?;
            held.series = Some(series);
            held.tree = Some(tree);
        }
        self.branch(branch)?
            .series
            .as_mut()
            .ok_or_else(|| io::Error::other("A stage lost its series writer"))
    }

    /// Records `hours`, which follow the family's last on their branch, as `made`, with the
    /// emit state and projector carry it goes on with after them; whether the batch ends
    /// after them.
    pub(crate) fn processed(
        &mut self,
        hours: Hours,
        made: Made,
        state: Option<Vec<u8>>,
        carry: Option<Vec<u8>>,
    ) -> io::Result<bool> {
        if let Some(stopped) = &self.publisher.lock().stopped {
            return Err(stopped.error());
        }
        if hours.first > hours.last || hours.first_hour > hours.last_hour {
            return Err(refused("Hours that run backwards"));
        }
        if let Made::Refused { hour, .. } = &made
            && !(hours.first_hour..=hours.last_hour).contains(hour)
        {
            return Err(refused("A refusal outside its hours"));
        }
        let next = hours
            .last
            .checked_add(1)
            .ok_or_else(|| refused("Ordinal overflow"))?;
        let publisher = self.publisher;
        let family = self.family;
        let step = self.step(hours.branch)?;
        if let Some(frontier) = &step.frontier {
            if frontier.end.is_some() {
                return Err(refused("A family's branch goes on after it ended"));
            }
            let follows = match hours.run.cmp(&frontier.next.run) {
                Ordering::Less => false,
                Ordering::Equal => hours.first == frontier.next.ordinal,
                Ordering::Greater => true,
            };
            if !follows || hours.first_hour <= frontier.last_hour {
                return Err(refused(
                    "Hours that do not follow the family's last on their branch",
                ));
            }
        }
        let frontier = step.frontier.get_or_insert_with(|| roots::Frontier {
            version: family.version,
            next: Cursor {
                run: hours.run,
                ordinal: hours.first,
            },
            segments: 0,
            first_hour: hours.first_hour,
            last_hour: hours.first_hour,
            prepared: 0,
            proof: [0; 32],
            ledger_pages: 0,
            first_hole: None,
            state: None,
            carry: None,
            end: None,
            complete: false,
        });
        frontier.next = Cursor {
            run: hours.run,
            ordinal: next,
        };
        frontier.last_hour = hours.last_hour;
        frontier.proof = hours.proof;
        frontier.complete = hours.ends_branch && hours.ends_segment;
        if hours.ends_segment {
            frontier.segments += 1;
            step.segments += 1;
        }
        let boundary = made != Made::Prepared;
        match made {
            Made::Prepared => {
                step.prepared += span(hours.first_hour, hours.last_hour);
                step.intervals.push(interval(
                    hours.first_hour,
                    hours.last_hour,
                    Held::Prepared,
                    String::new(),
                ));
            }
            Made::Refused { hour, reason, from } => {
                if hour > hours.first_hour {
                    step.prepared += span(hours.first_hour, hour - 1);
                    step.intervals.push(interval(
                        hours.first_hour,
                        hour - 1,
                        Held::Prepared,
                        String::new(),
                    ));
                }
                step.intervals.push(interval(
                    hour,
                    hours.last_hour,
                    Held::Refused,
                    reason.to_string(),
                ));
                frontier.end = Some(End {
                    held: Held::Refused,
                    hour,
                    reason,
                    from,
                });
            }
            Made::Lost { reason, ends } => {
                step.intervals.push(interval(
                    hours.first_hour,
                    hours.last_hour,
                    Held::Lost,
                    reason.clone(),
                ));
                if ends {
                    frontier.end = Some(End {
                        held: Held::Lost,
                        hour: hours.first_hour,
                        reason: reason.into(),
                        from: None,
                    });
                }
            }
        }
        let segments = frontier.segments;
        step.state = state;
        step.carry = carry;
        self.ends |= boundary
            || !hours.ends_segment
            || hours.ends_branch
            || (hours.ends_segment && ends_batch(segments, BATCH_SEGMENTS));
        if hours.ends_segment {
            let mut inner = publisher.lock();
            inner.progress.entry(family.id).or_default().emitted += 1;
        }
        Ok(self.ends)
    }

    /// The emit state and projector carry the family goes on with on `branch` after the open
    /// batch's hours there, in place of those [`Self::processed`] recorded with them.
    pub(crate) fn keep(
        &mut self,
        branch: u64,
        state: Option<Vec<u8>>,
        carry: Option<Vec<u8>>,
    ) -> io::Result<()> {
        let step = self
            .steps
            .get_mut(&branch)
            .ok_or_else(|| refused("A branch the open batch holds no hours of"))?;
        step.state = state;
        step.carry = carry;
        Ok(())
    }

    /// Whether the open batch ends by the recording: the lane submits it.
    pub(crate) fn ends(&self) -> bool {
        self.ends
    }

    /// Cuts `branch`'s series segment through the open batch's last hour there, which ended its
    /// recording segment without ending the batch, and hands it off to be sealed at once while
    /// the lane goes on; the batch's root names it ([`Self::send`]).
    pub(crate) fn seal(&mut self, branch: u64) -> io::Result<()> {
        let _memory = labels::kind(Kind::Publish);
        let last = self
            .steps
            .get(&branch)
            .and_then(|step| step.frontier.as_ref())
            .map(|frontier| frontier.last_hour)
            .ok_or_else(|| refused("A branch the open batch holds no hours of"))?;
        let held = self
            .branches
            .get_mut(&branch)
            .ok_or_else(|| io::Error::other("A stage lost a branch"))?;
        let series = held
            .series
            .as_mut()
            .ok_or_else(|| refused("A branch with no series writer"))?;
        let pending = series.cut_through(last)?;
        let tree = held.tree.take();
        self.publisher.hand(
            self.family,
            Work::Segment {
                branch,
                pending,
                tree,
            },
        );
        Ok(())
    }

    /// Hands the open batch off to be sealed and committed while the lane goes on: each branch's
    /// ledger, state, carry and coverage page and then its root.
    pub(crate) fn send(&mut self) -> io::Result<()> {
        let job = self.cut()?;
        self.handed = Some(self.publisher.hand(self.family, Work::Batch(job)));
        Ok(())
    }

    /// Waits for the batch the lane handed off last to be committed, doing the work its family
    /// handed off up to it here when no sealer does.
    pub(crate) fn settle(&mut self) -> io::Result<()> {
        let Some(id) = self.handed.take() else {
            return Ok(());
        };
        self.publisher.finish(self.family.id, id)
    }

    /// Hands the open batch off and waits for it to be committed.
    #[cfg(test)]
    pub(crate) fn submit(&mut self) -> io::Result<()> {
        self.send()?;
        self.settle()
    }

    /// Cuts the open batch once the one before is committed: each branch's step after the
    /// family's last publication there, and its series segment.
    fn cut(&mut self) -> io::Result<Job> {
        self.settle()?;
        if self.steps.is_empty() {
            return Err(refused("A batch publishes no hours"));
        }
        let steps = std::mem::take(&mut self.steps);
        self.ends = false;
        let mut sealings = Vec::with_capacity(steps.len());
        let (mut segments, mut prepared) = (0, 0);
        for (branch, step) in steps {
            let Some(frontier) = step.frontier else {
                continue;
            };
            self.series(branch, frontier.first_hour)?;
            let held = self.branch(branch)?;
            let previous = held.series.as_ref().and_then(roots::Series::published);
            let (cut, checkpoint) = roots::cut(
                &mut held.last,
                roots::Step {
                    frontier,
                    intervals: &step.intervals,
                    state: step.state,
                    carry: step.carry,
                },
                previous,
            )?;
            let series = held
                .series
                .as_mut()
                .ok_or_else(|| io::Error::other("A stage lost its series writer"))?;
            let checkpoint = series.bytes(&checkpoint)?;
            let pending = series.cut(cut.publication, checkpoint)?;
            let tree = held.tree.take();
            // Nothing goes on from a branch that ended.
            if cut.frontier.end.is_some() {
                held.series = None;
            }
            segments += step.segments;
            prepared += step.prepared;
            sealings.push(Sealing {
                branch,
                cut,
                pending,
                tree,
            });
        }
        Ok(Job {
            family: self.family,
            writer: self.writer.take(),
            sealings,
            segments,
            prepared,
        })
    }
}

impl Drop for Stage<'_, '_> {
    fn drop(&mut self) {
        self.publisher.abandon(self.family.id, self.handed.take());
        if let Some(writer) = self.writer.take() {
            drop(writer);
            self.publisher.released();
        }
    }
}

fn interval(first_hour: i32, last_hour: i32, held: Held, reason: String) -> Interval {
    Interval {
        first_hour,
        last_hour,
        held,
        reason,
    }
}

fn span(first: i32, last: i32) -> u64 {
    (i64::from(last) - i64::from(first) + 1) as u64
}

#[cfg(test)]
mod tests;
