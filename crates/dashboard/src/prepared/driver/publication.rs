//! Publication of the families the segment driver prepares. Each family's lane writes its batch
//! into a stage of its own, outside the root writer ([`Stage`]); one coordinator adopts each
//! lane's batch whole or not at all, and commits together the batches ready while it commits.
//! A batch carries the family's pages and, for each branch it touched, the family's ledger,
//! emit state, projector carry and coverage page, then its publication root ([`roots`]), last.
//! A lane cuts its batch and hands it off: a sealer seals, stages and commits it while the lane
//! appends the next ([`Stage::send`], [`Publisher::sealing`]).
//! The root's identity carries the family's version stamp, so opening a store keeps every family
//! whose stamp is unchanged, clears every other, which prepares again alone, and resumes each
//! family on each branch after its own last committed hour ([`Resume`]). Every error a stage
//! returns is its family's alone, but publication's stop ([`halted`]), which every lane meets.
//!
//! Batches end where the recording says ([`ends_batch`]): after a branch's 1st, 2nd, 4th, 8th
//! and 16th segment, then every 32nd, counted from the branch's start however preparation
//! started, resumed or grew; and earlier at a loss or refusal, at the branch's end and at the
//! byte budget. While the recording grows, each arrival of committed hours of its open tail
//! ends a batch: the family's operators go on from their carried state hour by hour, its gate
//! reaches the newest committed hour, and a later run goes on after that hour, never repeating
//! one.

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

/// Bytes a batch stages before it ends; a commit budget, never a limit on what a family holds.
const BATCH_BYTES: u64 = dashboard_base::prepared::pages::BATCH_BYTES as u64;
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
    /// Jobs lanes handed off, oldest first, by id; those sealed, until their lanes take them.
    jobs: VecDeque<(u64, Job)>,
    finished: BTreeMap<u64, Finished>,
    jobs_handed: u64,
    /// Sealers serve the jobs ([`Publisher::sealing`]).
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
                    jobs: VecDeque::new(),
                    finished: BTreeMap::new(),
                    jobs_handed: 0,
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
        // Jobs no sealer took commit nothing now; their stages go.
        let unsealed: Vec<(u64, Job)> = inner.jobs.drain(..).collect();
        for &(id, _) in &unsealed {
            inner.finished.insert(id, Finished::failed(stopped.error()));
        }
        drop(inner);
        for (_, job) in unsealed {
            self.dropped(job);
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
                Some(published.publication.generation) == last.unwrap_or(0).checked_add(1)
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
    /// The stage the lane wrote the batch's pages in, once it needed one; else the job forks its
    /// own.
    writer: Option<store::Writer>,
    sealings: Vec<Sealing>,
    segments: u64,
    prepared: u64,
}

/// A branch's step of a job: its cut, its series segment and the hierarchy it publishes into.
struct Sealing {
    branch: u64,
    cut: roots::Cut,
    pending: dashboard_series::Pending<'static>,
    tree: dashboard_series::Tree,
}

/// What a job leaves its lane: the hierarchies it published into, by branch, and whether its
/// batch was committed.
struct Finished {
    trees: Vec<(u64, dashboard_series::Tree)>,
    outcome: io::Result<()>,
}

impl Finished {
    fn failed(error: io::Error) -> Self {
        Self {
            trees: Vec::new(),
            outcome: Err(error),
        }
    }
}

/// The stage a job seals in: its writer closes before the stage is released, however the job
/// ends, so a stop that sees every stage released finds the store's handles closed.
struct Forked<'p, 's> {
    publisher: &'p Publisher<'s>,
    writer: Option<store::Writer>,
}

impl Drop for Forked<'_, '_> {
    fn drop(&mut self) {
        drop(self.writer.take());
        self.publisher.released();
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
    /// Runs `work` beside `sealers` threads that seal the jobs lanes hand off ([`Stage::send`]),
    /// each lane's in order: a lane goes on appending while its last batch seals and commits.
    /// A job no sealer took when its lane needs it is sealed by the lane.
    pub(crate) fn sealing<R>(&self, sealers: usize, work: impl FnOnce() -> R) -> R {
        self.lock().sealing = true;
        std::thread::scope(|scope| {
            let stop = Sealers(self);
            let caller = Caller::current();
            for _ in 0..sealers {
                let caller = caller.clone();
                scope.spawn(move || {
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

    /// Seals handed-off jobs, oldest first, until sealing stops.
    fn serve(&self) {
        let mut inner = self.lock();
        while inner.sealing {
            let Some((id, job)) = inner.jobs.pop_front() else {
                inner = self.wait_changed(inner);
                continue;
            };
            drop(inner);
            let finished = self.seal(job);
            inner = self.lock();
            inner.finished.insert(id, finished);
            self.changed.notify_all();
        }
    }

    /// Hands `job` to the sealers; after a stop it fails at once.
    fn hand(&self, job: Job) -> u64 {
        let mut inner = self.lock();
        let id = inner.jobs_handed;
        inner.jobs_handed += 1;
        if let Some(stopped) = &inner.stopped {
            let failed = Finished::failed(stopped.error());
            inner.finished.insert(id, failed);
            drop(inner);
            self.dropped(job);
            return id;
        }
        inner.jobs.push_back((id, job));
        drop(inner);
        self.changed.notify_all();
        id
    }

    /// Lets go of an unsealed job's stage.
    fn dropped(&self, job: Job) {
        if let Some(writer) = job.writer {
            drop(writer);
            self.released();
        }
    }

    /// What job `id` left, sealing it here when no sealer took it.
    fn finish(&self, id: u64) -> Finished {
        let mut inner = self.lock();
        loop {
            if let Some(finished) = inner.finished.remove(&id) {
                return finished;
            }
            if let Some(at) = inner.jobs.iter().position(|(queued, _)| *queued == id) {
                let job = inner.jobs.remove(at).map(|(_, job)| job);
                drop(inner);
                return match job {
                    Some(job) => self.seal(job),
                    None => Finished::failed(io::Error::other("A handed-off batch was lost")),
                };
            }
            inner = self.wait_changed(inner);
        }
    }

    /// Drops job `id` unsealed, or waits for the sealer sealing it.
    fn abandon(&self, id: u64) {
        let mut inner = self.lock();
        loop {
            if inner.finished.remove(&id).is_some() {
                return;
            }
            if let Some(at) = inner.jobs.iter().position(|(queued, _)| *queued == id) {
                let job = inner.jobs.remove(at);
                drop(inner);
                if let Some((_, job)) = job {
                    self.dropped(job);
                }
                return;
            }
            inner = self.wait_changed(inner);
        }
    }

    /// Seals `job`: each branch's ledger, state, carry and series segment, then its root, and
    /// its coverage page; stages it, queues it for the next group commit and waits for that,
    /// leading it when no other does. A panic fails the job alone.
    fn seal(&self, job: Job) -> Finished {
        let _span = replay_frame::timeline::span("seal").lane(job.family.id.name());
        let _memory = labels::enter(labels::owner(job.family.id.name()), Kind::Publish);
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.sealed(job))) {
            Ok(finished) => finished,
            Err(_) => Finished::failed(io::Error::other("A batch's seal panicked")),
        }
    }

    fn sealed(&self, job: Job) -> Finished {
        let Job {
            family,
            writer,
            sealings,
            segments,
            prepared,
        } = job;
        let writer = match writer {
            Some(writer) => writer,
            None => match self.fork(family.id) {
                Ok(writer) => writer,
                Err(error) => return Finished::failed(error),
            },
        };
        let mut forked = Forked {
            publisher: self,
            writer: Some(writer),
        };
        let name = family.id.name();
        let mut sealed = Sealed {
            published: Vec::with_capacity(sealings.len()),
            segments,
            prepared,
        };
        let mut trees = Vec::with_capacity(sealings.len());
        let mut written = Ok(());
        #[cfg(test)]
        if self.faulted(family.id, Own::Seal) {
            written = Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Simulated seal failure",
            ));
        }
        let sealings = if written.is_ok() { sealings } else { Vec::new() };
        let Some(writer) = forked.writer.as_mut() else {
            return Finished::failed(io::Error::other("A job lost its stage"));
        };
        for sealing in sealings {
            let Sealing {
                branch,
                cut,
                pending,
                mut tree,
            } = sealing;
            let identity = self.binding.identity(branch, family.stamp);
            let published = roots::write(writer, name, branch, identity, cut, &pending, &mut tree)
                .and_then(|published| {
                    pipeline::write_family_page(&mut *writer, name, branch, &published.frontier)
                        .map(|()| published)
                });
            drop(pending);
            match published {
                Ok(published) => {
                    trees.push((branch, tree));
                    sealed.published.push((branch, published));
                }
                Err(error) => {
                    written = Err(error);
                    break;
                }
            }
        }
        let batch = written.and_then(|()| {
            let writer = forked
                .writer
                .take()
                .ok_or_else(|| io::Error::other("A job lost its stage"))?;
            Ok(Batch {
                family: family.id,
                staged: writer.into_staged()?,
                sealed,
            })
        });
        let queued = batch.and_then(|batch| self.queue(batch));
        drop(forked);
        Finished {
            trees,
            outcome: queued.and_then(|ticket| self.wait(ticket)),
        }
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
    /// The family's output on the series engine; its hierarchy is lent to the job sealing the
    /// segment it cut last.
    series: Option<roots::Series>,
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

    /// The rows its series writers hold, appended since its batch began.
    pub(crate) fn series_held(&self) -> usize {
        self.branches
            .values()
            .filter_map(|branch| branch.series.as_ref())
            .map(roots::Series::held_bytes)
            .fold(0, usize::saturating_add)
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
            let series = match &resumes {
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
            self.branch(branch)?.series = Some(series);
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
        let bytes = self.writer.as_ref().map_or(0, store::Writer::bytes_written);
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
        let series = self.series_held();
        self.ends |= boundary
            || !hours.ends_segment
            || hours.ends_branch
            || (hours.ends_segment && ends_batch(segments, BATCH_SEGMENTS))
            || bytes >= BATCH_BYTES
            || series >= roots::SERIES_BATCH_BYTES
            || (series > 0 && roots::series_pressed());
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

    /// Hands the open batch off to be sealed and committed while the lane goes on: each branch's
    /// ledger, state, carry and coverage page and then its root. Once the series writers hold
    /// half of what every writer may, the lane waits for it.
    pub(crate) fn send(&mut self) -> io::Result<()> {
        let job = self.cut()?;
        self.handed = Some(self.publisher.hand(job));
        if roots::series_pressed() {
            self.settle()?;
        }
        Ok(())
    }

    /// Waits for the batch the lane handed off last to be committed, sealing it here when no
    /// sealer took it.
    pub(crate) fn settle(&mut self) -> io::Result<()> {
        let Some(id) = self.handed.take() else {
            return Ok(());
        };
        let finished = self.publisher.finish(id);
        self.returned(finished.trees)?;
        finished.outcome
    }

    /// Hands the open batch off and waits for it to be committed.
    #[cfg(test)]
    pub(crate) fn submit(&mut self) -> io::Result<()> {
        self.send()?;
        self.settle()
    }

    /// Cuts the open batch once the one before is committed: each branch's step after the
    /// family's last publication there, and its series segment, whose hierarchy the job takes.
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
            let (cut, checkpoint) = roots::cut(
                &mut held.last,
                roots::Step {
                    frontier,
                    intervals: &step.intervals,
                    state: step.state,
                    carry: step.carry,
                },
            )?;
            let series = held
                .series
                .as_mut()
                .ok_or_else(|| io::Error::other("A stage lost its series writer"))?;
            let checkpoint = series.bytes(&checkpoint)?;
            let pending = series.cut(cut.publication, checkpoint)?;
            let tree = series.lend_tree()?;
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

    /// Takes back the hierarchies a job published into.
    fn returned(&mut self, trees: Vec<(u64, dashboard_series::Tree)>) -> io::Result<()> {
        for (branch, tree) in trees {
            if let Some(series) = self
                .branches
                .get_mut(&branch)
                .and_then(|branch| branch.series.as_mut())
            {
                series.return_tree(tree)?;
            }
        }
        Ok(())
    }
}

impl Drop for Stage<'_, '_> {
    fn drop(&mut self) {
        if let Some(id) = self.handed.take() {
            self.publisher.abandon(id);
        }
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
