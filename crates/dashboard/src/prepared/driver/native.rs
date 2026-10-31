//! Preparation of a native recording. Every family with a native backend ([`families`]) walks
//! the recording's segments in one run of the segment executor ([`segments::run_in`]): each
//! walk decodes the union of their tables once and advances one world of what its families read,
//! and each family's ordered emit publishes through a stage of its own of the one publisher
//! ([`Publisher`]), which ownership guards. The change-run index the viewer's map reads is
//! prepared before the run ([`crate::prepared::change_runs`]). A family prepared ahead ([`registry::Descriptor::ahead`]) walks the whole
//! recording first, in a run of its own that decodes its tables alone ([`walk_all`]). Every other
//! family, and every recording-wide job without a native
//! implementation, is refused by name on every branch ([`plan::native`]). Content is loaded once,
//! on loader threads before the run ([`content`]). A family takes up its committed publication
//! on each branch, as the recording a live recorder writes grows: it goes on from the hour after
//! its last with the state and carry it saved there, and prepares only the hours the recording
//! holds since.

mod backend;
mod content;
mod families;
mod identities;
mod measure;
mod shape;
mod status;
#[cfg(test)]
mod tests;

pub use measure::{GOAL, Goal, Measured, Phases, Run};

use super::{
    plan,
    publication::{Family, Publisher, Stage, Stopped},
    registry::FamilyId,
};
use crate::{
    Source,
    config::{ContentOptions, LoadContext},
    prepared::pipeline::Accepted,
};
use backend::{Going, Member, Taken, Walked, Walking};
use content::Content;
use dashboard_base::prepared::{
    gates,
    priority::{Caller, JobPriority},
    roots::{self, Binding, Frontier},
};
use dashboard_world::{Needs, World};
use measure::{Clock, Counted, Lap, Narrow, working_set};
use replay::{
    columnar_frames::{Limits, Recording, Segment},
    columnar_runs,
    segments::{self, Outcome, Settings},
};
use replay_data::{format::FormatVersion, rows::TableId, scope::CaptureMode};
use shape::Shape;
use std::{
    collections::BTreeMap,
    io,
    sync::{
        Arc, Condvar, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU64},
    },
};

fn interrupted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Preparation was cancelled")
}

fn check_cancel(cancelled: &AtomicBool) -> io::Result<()> {
    crate::metadata::check_cancel(cancelled)
}

/// Every family with a native backend, in the order their lanes run.
pub(crate) fn names() -> Vec<&'static str> {
    families::REGISTERED
        .iter()
        .map(|registered| registered.name)
        .collect()
}

/// The native columnar recording of `source`'s replay that it is prepared from: an undamaged
/// one of its campaign's ([`crate::prepared::change_runs::native_files`]) recording exactly the
/// replay's hours, whose capture is the replay's ([`columnar_runs::captures`]). None when there
/// is none.
pub(crate) fn recording(source: &Source) -> io::Result<Option<Recording>> {
    let campaign = &source.replay_ref().campaign_id;
    let candidates = crate::prepared::change_runs::native_files(source.path(), campaign)?;
    for candidate in candidates {
        let recording = match Recording::open_path(&candidate, Limits::default()) {
            Ok(recording) => recording,
            Err(error) if error.kind() == io::ErrorKind::InvalidData => continue,
            Err(error) => return Err(error),
        };
        let captured = match recording.capture() {
            Ok(Some(capture)) => columnar_runs::recorded_runs(&recording)?.is_some_and(|runs| {
                columnar_runs::captures(&capture, &runs, source.replay_ref())
            }),
            Ok(None) | Err(_) => false,
        };
        if captured && pairs(source, &recording)? {
            return Ok(Some(recording));
        }
    }
    Ok(None)
}

/// Whether `recording` records exactly the hours of `source`'s replay, of its campaign.
fn pairs(source: &Source, recording: &Recording) -> io::Result<bool> {
    let replay = source.replay_ref();
    Ok(recording.campaign() == replay.campaign_id
        && columnar_runs::recorded_runs(recording)?
            .is_some_and(|runs| columnar_runs::covers(&replay.index, &runs)))
}

/// What a native preparation prepares, and how.
#[derive(Default)]
struct Options<'a> {
    /// The families to prepare: every one with a native backend when None.
    families: Option<&'a [&'a str]>,
    /// The run's workers: the process's parallelism when None.
    threads: Option<usize>,
    measure: bool,
}

/// Prepares `source` from its native `recording`, reporting each commit to `accepted`.
pub(crate) fn prepare(
    source: &Source,
    content: Option<&ContentOptions>,
    recording: Recording,
    cancelled: &AtomicBool,
    accepted: &mut dyn FnMut(&Accepted),
) -> io::Result<Accepted> {
    let done = run(
        source,
        content,
        recording,
        &Options::default(),
        cancelled,
        accepted,
    )?;
    match done.failure {
        None => Ok(done.accepted),
        Some(failure) => {
            accepted(&done.accepted);
            Err(failure)
        }
    }
}

/// Prepares `source` from `recording`, its replay's native recording, as [`prepare`] does: of the
/// families `families` names, or every one with a native backend, on `threads` workers, measured.
/// A family that failed fails its status rows, which say why.
pub fn measure(
    source: &Source,
    content: Option<&ContentOptions>,
    recording: Recording,
    families: Option<&[&str]>,
    threads: usize,
    cancelled: &AtomicBool,
) -> io::Result<(Accepted, Measured)> {
    if !pairs(source, &recording)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "The native recording is not of this recording's campaign and hours",
        ));
    }
    let options = Options {
        families,
        threads: Some(threads),
        measure: true,
    };
    let done = run(source, content, recording, &options, cancelled, &mut |_| {})?;
    let measured = done
        .measured
        .ok_or_else(|| io::Error::other("The run was not measured"))?;
    Ok((done.accepted, measured))
}

/// What a complete native preparation of `source` from `recording` accepted, read from its
/// store; None while the store is not marked with the plan's refusals over the recording's
/// branches, or any family with a native backend has hours of a branch left to prepare, as when
/// the recording grew.
pub(crate) fn restore(
    source: &Source,
    content: Option<&ContentOptions>,
    recording: &Recording,
    cancelled: &AtomicBool,
) -> io::Result<Option<Accepted>> {
    check_cancel(cancelled)?;
    let (store, key) = crate::prepared::files(source, content)?;
    let Some(published) = gates::current(&store, &key) else {
        return Ok(None);
    };
    let plan = plan::native();
    let shape = Shape::new(recording)?;
    if published.plan() != Some(&status::mark(plan, &shape)[..]) {
        return Ok(None);
    }
    let binding = binding(recording, content);
    let families = Family::planned(plan, &names())?;
    let mut frontiers = BTreeMap::new();
    for family in &families {
        for &branch in shape.branches().keys() {
            let Some(gate) = published.gate(branch, family.id.name()) else {
                return Ok(None);
            };
            let Some(root) = gate.publication()? else {
                return Ok(None);
            };
            if root.identity != binding.identity(branch, family.stamp)
                || root.frontier.version != family.version
                || !shape.accepts(recording, branch, &root.frontier)
                || (root.frontier.end.is_none() && shape.after(branch, &root.frontier))
            {
                return Ok(None);
            }
            frontiers.insert((family.id, branch), root.frontier.clone());
        }
    }
    check_cancel(cancelled)?;
    let prepared: Vec<FamilyId> = families.iter().map(|family| family.id).collect();
    Ok(Some(status::accepted(
        plan,
        &shape,
        &prepared,
        &|family, branch| frontiers.get(&(family, branch)).cloned(),
        &BTreeMap::new(),
        status::stored(&store)?,
    )))
}

/// Whether `source`'s store holds `family`'s native publication through the end of every branch
/// of the recording, as a measured preparation of that family alone leaves it.
pub(crate) fn complete(
    source: &Source,
    content: Option<&ContentOptions>,
    family: &str,
) -> io::Result<bool> {
    let family = FamilyId::parse(family)?;
    let (store, key) = crate::prepared::files(source, content)?;
    let Some(published) = gates::current(&store, &key) else {
        return Ok(false);
    };
    if published.plan().is_none() {
        return Ok(false);
    }
    for start in &source.replay_ref().branches {
        let Some(gate) = published.gate(start.id, family.name()) else {
            return Ok(false);
        };
        let Some(root) = gate.publication()? else {
            return Ok(false);
        };
        if !root.frontier.terminal() {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Whether a native preparation published to `source`'s store.
pub(crate) fn published(source: &Source, content: Option<&ContentOptions>) -> io::Result<bool> {
    let (store, _) = crate::prepared::files(source, content)?;
    Ok(status::held(&store)?.is_some())
}

/// Clears `source`'s store when a native preparation published to it, for its rows to prepare it:
/// nothing of it is theirs.
pub(crate) fn release(source: &Source, content: Option<&ContentOptions>) -> io::Result<()> {
    let (store, key) = crate::prepared::files(source, content)?;
    if status::held(&store)?.is_some() {
        store.replace(key)?.commit()?;
    }
    Ok(())
}

/// What the publications of `recording` under `content` are bound to: its campaign, and whether
/// content was read and verified.
fn binding(recording: &Recording, content: Option<&ContentOptions>) -> Binding {
    let read = [
        u8::from(content.is_some()),
        u8::from(content.is_some_and(|content| content.allow_unverified)),
    ];
    Binding::new(recording.campaign(), &read)
}

/// The families `names` selects of those with a native backend, in lane order.
fn selected(names: Option<&[&str]>) -> io::Result<Vec<&'static families::Registered>> {
    let Some(names) = names else {
        return Ok(families::REGISTERED.iter().collect());
    };
    for name in names {
        let family = FamilyId::parse(name)?;
        if !families::REGISTERED
            .iter()
            .any(|registered| registered.name == family.name())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{name} has no native backend"),
            ));
        }
    }
    Ok(families::REGISTERED
        .iter()
        .filter(|registered| {
            names
                .iter()
                .any(|name| FamilyId::parse(name).is_ok_and(|id| id.name() == registered.name))
        })
        .collect())
}

/// What `family` takes up of each branch its committed publications hold (`frontiers`): those
/// it ended on or the recording holds nothing more of, which it passes, and where it goes on in
/// the rest, with the carry its operator saved there.
fn taken(
    publisher: &Publisher<'_>,
    stage: &mut Stage<'_, '_>,
    shape: &Shape,
    family: FamilyId,
    frontiers: Option<&BTreeMap<u64, Frontier>>,
) -> io::Result<(Vec<u64>, Vec<Going>)> {
    let (mut passes, mut going) = (Vec::new(), Vec::new());
    for (&branch, frontier) in frontiers.into_iter().flatten() {
        if frontier.end.is_some() || !shape.after(branch, frontier) {
            passes.push(branch);
            continue;
        }
        let published = publisher
            .committed(family, branch)
            .ok_or_else(|| io::Error::other("A family's committed publication is missing"))?;
        let carry = roots::carry(stage.pages()?, family.name(), branch, &published)?;
        going.push(Going {
            branch,
            run: frontier.next.run,
            ordinal: frontier.next.ordinal,
            published,
            carry,
        });
    }
    Ok((passes, going))
}

/// What a run of a native preparation accepted, and what failed.
struct Done {
    accepted: Accepted,
    measured: Option<Measured>,
    /// Each family that failed and why, and the change-run index's failure.
    failure: Option<io::Error>,
}

fn run(
    source: &Source,
    content: Option<&ContentOptions>,
    mut recording: Recording,
    options: &Options<'_>,
    cancelled: &AtomicBool,
    accepted: &mut dyn FnMut(&Accepted),
) -> io::Result<Done> {
    let whole = Clock::start();
    let started_with = if options.measure { working_set()? } else { 0 };
    check_cancel(cancelled)?;
    let plan = plan::native();
    let chosen = selected(options.families)?;
    let threads = options
        .threads
        .unwrap_or_else(crate::prepared::pipeline::parallelism)
        .max(1);
    let shape = Shape::new(&recording)?;
    let (store, key) = crate::prepared::files(source, content)?;
    let context = LoadContext {
        content: content.cloned(),
    };
    let extents = dashboard_text::metadata::Extents::native(source.replay_ref(), &recording)?;
    let loading = Content::new(source, &context, extents);
    let caller = Caller::current();
    let started = Clock::start();
    let loaded = content::load_all(
        threads,
        chosen
            .iter()
            .map(|registered| {
                let caller = caller.clone();
                let loading = &loading;
                Box::new(move || {
                    caller.enter();
                    (registered.load)(loading)
                }) as Box<dyn FnOnce() -> io::Result<()> + Send + '_>
            })
            .collect(),
    );
    let content_lap = started.lap();
    let loaded_with = if options.measure { working_set()? } else { 0 };
    check_cancel(cancelled)?;
    let held = status::held(&store)?;
    if held.is_none() && store.current()?.is_some() {
        // A row preparation's publication: nothing of it is a native family's.
        store.replace(key.clone())?.commit()?;
    }
    let names: Vec<&str> = chosen.iter().map(|registered| registered.name).collect();
    let families = Family::planned(plan, &names)?;
    let (publisher, resume) = Publisher::open(
        &store,
        key,
        binding(&recording, content),
        plan,
        &families,
        &|_, branch, frontier| shape.accepts(&recording, branch, frontier),
    )?;
    let commits = Arc::new(Commits::default());
    let noted = Arc::clone(&commits);
    let publisher = publisher.with_commits(move || noted.note());
    let mark = status::mark(plan, &shape);
    if resume.replaced || held.as_deref() != Some(&mark[..]) {
        publisher.coordinate(|writer| status::coordinate(writer, plan, &shape, &mark))?;
    }
    let mut failed: BTreeMap<FamilyId, String> = BTreeMap::new();
    let mut staged = Vec::new();
    // Where each family goes on, for its lane to borrow.
    let mut goings: Vec<Vec<Going>> = Vec::new();
    for ((registered, family), loaded) in chosen.iter().zip(&families).zip(loaded) {
        if let Err(error) = loaded {
            failed.insert(
                family.id,
                format!("Its content could not be loaded: {error}"),
            );
            continue;
        }
        let mut stage = publisher.stage(family.id)?;
        let frontiers = resume.families.get(&family.id);
        match taken(&publisher, &mut stage, &shape, family.id, frontiers) {
            Ok((passes, going)) => {
                goings.push(going);
                staged.push((registered, family, stage, passes));
            }
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                failed.insert(family.id, format!("Its carry could not be read: {error}"));
            }
            Err(error) => return Err(error),
        }
    }
    let mut members: Vec<Box<dyn Member + '_>> = Vec::new();
    for ((registered, family, stage, passes), going) in staged.into_iter().zip(&goings) {
        let taken = Taken { passes, going };
        match (registered.bind)(family.id, &loading, stage, taken) {
            Ok(member) => members.push(member),
            Err(error) => {
                failed.insert(family.id, format!("It could not be bound: {error}"));
            }
        }
    }
    let prepared: Vec<FamilyId> = families.iter().map(|family| family.id).collect();
    let ahead = ahead_first(plan, &mut members);
    let replay = source.replay_ref();
    let mut bytes = status::stored(&store)?;
    let report = |failed: &BTreeMap<FamilyId, String>, bytes: u64| {
        status::accepted(
            plan,
            &shape,
            &prepared,
            &|family, branch| {
                publisher
                    .committed(family, branch)
                    .map(|published| published.frontier)
            },
            failed,
            bytes,
        )
    };
    let settings = Settings::new(threads);
    let walked = {
        let scratch = segments::Folder::new(store.folder().with_extension("spill"))?;
        let walk = Walk {
            shape: &shape,
            members: &mut members,
            world: (replay.format, replay.capture_mode),
            settings: &settings,
            scratch: &scratch,
            cancelled,
            measuring: options.measure,
            commits: &commits,
            progress: &mut || {
                bytes = status::stored(&store).unwrap_or(bytes);
                accepted(&report(&failed, bytes));
            },
        };
        let walked = walk_all(&mut recording, ahead, walk);
        drop(scratch);
        // Each spill went with its handle; the folder goes once empty.
        let _ = std::fs::remove_dir(store.folder().with_extension("spill"));
        walked
    };
    let walked = match walked {
        Ok(walked) if !walked.ran.cancelled => walked,
        Ok(_) => {
            drop(members);
            publisher.stop(Stopped::Cancelled);
            return Err(interrupted());
        }
        Err(error) => {
            drop(members);
            publisher.stop(Stopped::Failed(Arc::from(error.to_string())));
            return Err(error);
        }
    };
    for (member, outcome) in members.iter().zip(&walked.ran.outcomes) {
        if let Some(failure) = outcome
            .as_ref()
            .and_then(|outcome| outcome.failure.as_ref())
        {
            let how = if failure.panicked {
                "panicked"
            } else {
                "failed"
            };
            failed.insert(
                member.id(),
                format!(
                    "Its {:?} {how} at segment {}: {}",
                    failure.operation, failure.at, failure.error
                ),
            );
        }
    }
    for member in &mut members {
        member.settle()?;
    }
    let ended_with = if options.measure { working_set()? } else { 0 };
    let spent: Vec<(&'static str, backend::Spent)> = members
        .iter()
        .map(|member| (member.id().name(), member.spent()))
        .collect();
    let counts: Vec<(&'static str, Vec<(&'static str, u64)>)> = members
        .iter()
        .map(|member| (member.id().name(), member.counts()))
        .collect();
    let narrow: Vec<&'static str> = members[..ahead]
        .iter()
        .map(|member| member.id().name())
        .collect();
    drop(members);
    let done = report(&failed, status::stored(&store)?);
    let measured = options
        .measure
        .then(|| {
            Measured::new(Counted {
                hours: recording.hours(),
                segments: shape.len(),
                threads,
                whole: whole.lap(),
                content: content_lap,
                run: walked.lap,
                world: walked.ran.world,
                families: &spent,
                counts: &counts,
                stats: walked.ran.stats,
                ahead: walked.ahead.map(|(stats, world, lap)| Narrow {
                    lap,
                    world,
                    stats,
                    families: narrow,
                }),
                working: [started_with, loaded_with, ended_with],
                walker_budget: settings.walker_bytes,
            })
        })
        .transpose()?;
    let errors: Vec<String> = failed
        .iter()
        .map(|(family, why)| format!("{}: {why}", family.name()))
        .collect();
    Ok(Done {
        accepted: done,
        measured,
        failure: (!errors.is_empty()).then(|| io::Error::other(errors.join("; "))),
    })
}

/// Commits made while a run goes on, for the thread reporting them, and the run's end.
#[derive(Default)]
pub(crate) struct Commits {
    noted: Mutex<(u64, bool)>,
    changed: Condvar,
}

impl Commits {
    fn lock(&self) -> std::sync::MutexGuard<'_, (u64, bool)> {
        self.noted.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn note(&self) {
        self.lock().0 += 1;
        self.changed.notify_all();
    }

    fn finish(&self) {
        self.lock().1 = true;
        self.changed.notify_all();
    }

    /// Begins a run's account: the commits made so far, its run not ended.
    fn start(&self) -> u64 {
        let mut noted = self.lock();
        noted.1 = false;
        noted.0
    }

    /// The commits made, once more than `seen` were; None once the run ended without more.
    fn wait(&self, seen: u64) -> Option<u64> {
        let mut noted = self.lock();
        loop {
            if noted.0 > seen {
                return Some(noted.0);
            }
            if noted.1 {
                return None;
            }
            noted = self
                .changed
                .wait(noted)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// Ends the run for the thread reporting its commits, however the run ends.
struct Ended<'c>(&'c Commits);

impl Drop for Ended<'_> {
    fn drop(&mut self) {
        self.0.finish();
    }
}

/// A run of members over a recording.
pub(crate) struct Walk<'w, 'm> {
    pub shape: &'w Shape,
    pub members: &'w mut [Box<dyn Member + 'm>],
    /// The format and capture mode the walks' worlds read.
    pub world: (FormatVersion, CaptureMode),
    pub settings: &'w Settings,
    pub scratch: &'w dyn segments::Scratch,
    pub cancelled: &'w AtomicBool,
    pub measuring: bool,
    pub commits: &'w Commits,
    /// Called on this thread after commits the run made, as they are made.
    pub progress: &'w mut dyn FnMut(),
}

/// What a run did.
pub(crate) struct Ran {
    pub stats: segments::Stats,
    /// What became of each member, in their order; None for one with nothing to do.
    pub outcomes: Vec<Option<Outcome>>,
    /// Cycles the walks' worlds took, when measured.
    pub world: u64,
    pub cancelled: bool,
}

/// Puts the members of families prepared ahead first, in lane order otherwise: how many there are.
pub(crate) fn ahead_first(plan: &plan::Plan, members: &mut [Box<dyn Member + '_>]) -> usize {
    let ahead = |id: FamilyId| {
        plan.families()
            .iter()
            .any(|family| family.id == id && family.ahead)
    };
    members.sort_by_key(|member| !ahead(member.id()));
    members
        .iter()
        .take_while(|member| ahead(member.id()))
        .count()
}

/// What a preparation's runs did ([`walk_all`]).
pub(crate) struct Walks {
    /// Every member's outcome, in their order, and the union run's stats and world.
    pub ran: Ran,
    /// The union run's time.
    pub lap: Lap,
    /// The narrow run's stats, world cycles and time, when it had something to do.
    pub ahead: Option<(segments::Stats, u64, Lap)>,
}

/// Makes `recording`'s walks decode the tables `members` with something to do read, and the
/// campaign row every world frame holds.
fn select(recording: &mut Recording, shape: &Shape, members: &[Box<dyn Member + '_>]) {
    let mut tables: Vec<TableId> = members
        .iter()
        .filter(|member| !member.idle(shape))
        .flat_map(|member| member.tables().iter().copied())
        .chain([TableId::Campaign])
        .collect();
    tables.sort_unstable_by_key(|table| table.index());
    tables.dedup();
    recording.select(&tables);
}

/// Runs `walk`'s members over `recording`, the first `ahead` of them, those of families prepared
/// ahead ([`ahead_first`]), before the rest: in a narrow run of their own over the whole recording
/// that decodes their tables alone, their last batches committed as it ends; then the rest in the
/// union run. A cancelled narrow run leaves the union run undone.
pub(crate) fn walk_all(
    recording: &mut Recording,
    ahead: usize,
    walk: Walk<'_, '_>,
) -> io::Result<Walks> {
    let Walk {
        shape,
        members,
        world,
        settings,
        scratch,
        cancelled,
        measuring,
        commits,
        progress,
    } = walk;
    let (narrow, union) = members.split_at_mut(ahead);
    let mut outcomes: Vec<Option<Outcome>> = narrow.iter().map(|_| None).collect();
    let mut first = None;
    if narrow.iter().any(|member| !member.idle(shape)) {
        select(recording, shape, narrow);
        let clock = Clock::start();
        let alone = Walk {
            shape,
            members: narrow,
            world,
            settings,
            scratch,
            cancelled,
            measuring,
            commits,
            progress: &mut *progress,
        };
        let mut ran = run_walk(recording, alone)?;
        let lap = clock.lap();
        outcomes = std::mem::take(&mut ran.outcomes);
        if ran.cancelled {
            outcomes.extend(union.iter().map(|_| None));
            return Ok(Walks {
                ran: Ran { outcomes, ..ran },
                lap,
                ahead: None,
            });
        }
        for member in narrow.iter_mut() {
            member.settle()?;
        }
        first = Some((ran.stats, ran.world, lap));
    }
    select(recording, shape, union);
    let clock = Clock::start();
    let together = Walk {
        shape,
        members: union,
        world,
        settings,
        scratch,
        cancelled,
        measuring,
        commits,
        progress,
    };
    let mut ran = run_walk(recording, together)?;
    let lap = clock.lap();
    outcomes.append(&mut ran.outcomes);
    Ok(Walks {
        ran: Ran { outcomes, ..ran },
        lap,
        ahead: first,
    })
}

/// Runs every member of `walk` with something to do over `recording` in one run of the executor,
/// on a thread of its own: this thread reports the commits it makes.
pub(crate) fn run_walk(recording: &Recording, walk: Walk<'_, '_>) -> io::Result<Ran> {
    let Walk {
        shape,
        members,
        world: (format, mode),
        settings,
        scratch,
        cancelled,
        measuring,
        commits,
        progress,
    } = walk;
    let places: Vec<usize> = (0..members.len())
        .filter(|&at| !members[at].idle(shape))
        .collect();
    let mut outcomes: Vec<Option<Outcome>> = members.iter().map(|_| None).collect();
    if places.is_empty() {
        return Ok(Ran {
            stats: segments::Stats::default(),
            outcomes,
            world: 0,
            cancelled: false,
        });
    }
    let mut needs = Vec::with_capacity(places.len());
    for &at in &places {
        needs.push(members[at].needs()?);
    }
    let cancellation = segments::Cancellation::default();
    let walking = Walking {
        recording,
        shape,
        cancelled,
        cancellation: &cancellation,
        measuring,
        job: JobPriority::current(),
        world: AtomicU64::new(0),
    };
    let caller = Caller::current();
    // Begun before the run, which may end before this thread waits.
    let mut seen = commits.start();
    let ran = std::thread::scope(|scope| {
        let running = scope.spawn(|| {
            let _ended = Ended(commits);
            caller.enter();
            let mut lanes = Vec::with_capacity(needs.len());
            let working = members
                .iter_mut()
                .enumerate()
                .filter_map(|(at, member)| places.contains(&at).then_some(member));
            for member in working {
                lanes.push(member.lane(&walking));
            }
            let worlds = |_: &Segment, visited: &[usize]| {
                let union = Needs::union(visited.iter().map(|&lane| &needs[lane]));
                World::new(&union, format, mode).map(|world| Walked::new(world, &walking))
            };
            segments::run_in(recording, settings, scratch, &cancellation, &worlds, lanes)
        });
        while let Some(count) = commits.wait(seen) {
            seen = count;
            progress();
        }
        running.join()
    });
    let ran = match ran {
        Ok(ran) => ran?,
        Err(panic) => std::panic::resume_unwind(panic),
    };
    let mut ran_outcomes = ran.outcomes.into_iter();
    for &at in &places {
        outcomes[at] = ran_outcomes.next();
    }
    Ok(Ran {
        stats: ran.stats,
        outcomes,
        world: walking.world.into_inner(),
        cancelled: ran.cancelled || cancellation.is_cancelled(),
    })
}
