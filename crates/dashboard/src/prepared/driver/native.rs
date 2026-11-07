//! Preparation of a native recording. Every family with a native backend ([`families`]) walks
//! the recording's segments in one run of the segment executor ([`segments::run_in`]): each
//! walk decodes the union of their tables once and advances one world of what its families read,
//! and each family's ordered emit publishes through a stage of its own of the one publisher
//! ([`Publisher`]), which ownership guards. The change-run index the viewer's map reads, when
//! due and of this recording, is one more lane of the union run, the walks decoding its tables
//! too ([`crate::prepared::change_runs::Riding`]), and is put in place once the run ends. A
//! family prepared ahead ([`registry::Descriptor::ahead`]) walks the whole
//! recording first, in a run of its own that decodes its tables alone ([`walk_all`]). A
//! recording-wide job with a native backend is a member of the run as a family is. Every other
//! family, and every recording-wide job without a native
//! implementation, is refused by name on every branch ([`plan::native`]). Content is loaded once,
//! on loader threads before the run ([`content`]). A family that fails, loading its content, in
//! its operators or emit, or sealing or committing a batch, fails alone ([`backend::alone`]): its
//! publication refuses by its name, from the first hour it left unpublished on each branch, with
//! its own error ([`fail`]), and every other family goes on. Only publication's stop is the
//! run's ([`publication::halted`]). A family takes up its committed publication
//! on each branch, as the recording a live recorder writes grows: it goes on from the hour after
//! its last with the state and carry it saved there, and prepares only the hours the recording
//! holds since. A recording read as timelines is prepared timeline by timeline: its main one,
//! and each other one a query asked for or its store publishes ([`timelines`]), each walked as
//! its pieces' segments ([`recording`]).

mod backend;
mod campaign;
mod content;
mod convoys;
mod families;
mod groups;
mod identities;
mod measure;
mod repairs;
mod shape;
pub(super) mod status;
#[cfg(test)]
mod tests;

pub use measure::{GOAL, Goal, Measured, Phases, Run, peaks};

use super::{
    plan,
    publication::{self, Family, Hours, Made, Publisher, Stage, Stopped},
    registry::FamilyId,
};
use crate::{
    Source,
    config::{ContentOptions, LoadContext},
    prepared::{
        change_runs::{Due, Riding, Rode},
        pipeline::Accepted,
    },
};
use backend::{Floor, Going, Member, Taken, Walked, Walking};
use content::Content;
use dashboard_base::prepared::{
    gates,
    priority::{Caller, JobPriority},
    roots::{self, Binding, Frontier},
};
use dashboard_world::{Needs, World};
use engine_allocation::labels::{self, Kind, Owner};
use measure::{Clock, Counted, Lap, Narrow, working_set};
use replay::{
    campaign::Replay,
    columnar_frames::{Recording, Segment},
    columnar_runs,
    segments::{self, Outcome, Settings},
};
use replay_data::{format::FormatVersion, rows::TableId, scope::CaptureMode};
use shape::Shape;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::PathBuf,
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

/// The run's workers when none are asked: the processors this process may run on.
#[cfg(not(test))]
fn parallelism() -> usize {
    #[cfg(windows)]
    if let Some(count) = affinity() {
        return count;
    }
    std::thread::available_parallelism().map_or(1, usize::from)
}

/// Tests run one worker unless they ask for more, so concurrent tests don't each start a
/// worker per processor.
#[cfg(test)]
fn parallelism() -> usize {
    1
}

/// The processors of this process's affinity mask; None when it spans processor groups.
#[cfg(all(windows, not(test)))]
fn affinity() -> Option<usize> {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessAffinityMask};
    let (mut process, mut system) = (0usize, 0usize);
    // A process spanning groups may read zero masks; std's count stands then.
    let read = unsafe { GetProcessAffinityMask(GetCurrentProcess(), &mut process, &mut system) };
    let count = process.count_ones() as usize;
    (read != 0 && count != 0).then_some(count)
}

/// Every family and recording-wide job with a native backend, in the order their lanes run.
pub(crate) fn names() -> Vec<&'static str> {
    families::REGISTERED
        .iter()
        .map(|registered| registered.name)
        .collect()
}

/// The columnar recording `source` is prepared from: its recording's runs, read through a reader
/// of its own, walked as `timelines` ([`view`]).
pub(crate) fn recording(source: &Source, timelines: Option<&[u64]>) -> io::Result<Recording> {
    let replay = source.replay_ref();
    view(replay, replay.recording().selecting(&TableId::ALL)?, timelines)
}

/// `recording`, `replay`'s as recorded, walked as each of `timelines` in turn: of each piece, the
/// segments of its recorded branch holding its hours, the last brought earlier to its last hour,
/// read as the timeline. As it is when None: every recorded branch.
fn view(replay: &Replay, recording: Recording, timelines: Option<&[u64]>) -> io::Result<Recording> {
    let Some(timelines) = timelines else {
        return Ok(recording);
    };
    let invalid = |message: &str| io::Error::new(io::ErrorKind::InvalidData, message.to_owned());
    // Each recorded branch's segments, in order, with their run's first hour.
    let mut recorded: BTreeMap<u64, Vec<(Segment, i64)>> = BTreeMap::new();
    for segment in recording.recorded_segments() {
        let first_hour = i64::from(recording.run(segment.run)?.first_hour);
        recorded
            .entry(segment.branch)
            .or_default()
            .push((*segment, first_hour));
    }
    let mut segments = Vec::new();
    for &timeline in timelines {
        let mut reached = None;
        for piece in replay.pieces(timeline) {
            let (from, to) = (i64::from(piece.first_hour), i64::from(piece.last_hour));
            for &(segment, first_hour) in recorded.get(&piece.segment).into_iter().flatten() {
                let first = first_hour + i64::from(segment.first);
                if first_hour + i64::from(segment.last) < from || first > to {
                    continue;
                }
                if first < from {
                    return Err(invalid("A timeline's piece starts inside a segment"));
                }
                if reached.is_some_and(|reached| reached >= (segment.run, segment.first)) {
                    return Err(invalid("A timeline's pieces go back in the recording"));
                }
                let last = u32::try_from(to - first_hour)
                    .map_or(segment.last, |last| last.min(segment.last));
                segments.push(Segment {
                    branch: timeline,
                    last,
                    ..segment
                });
                reached = Some((segment.run, last));
            }
        }
    }
    recording.with_segments(segments)
}

/// The timelines a preparation of `source` prepares: its main one first, then, in order, each
/// other one a query asked for ([`ask`]) or its store publishes. None when its replay is not
/// read as timelines: every recorded branch is prepared then.
pub(crate) fn timelines(
    source: &Source,
    content: Option<&ContentOptions>,
) -> io::Result<Option<Vec<u64>>> {
    let replay = source.replay_ref();
    let Some(held) = replay.timelines() else {
        return Ok(None);
    };
    let mut others: BTreeSet<u64> = asked(source)?.into_iter().collect();
    let (store, _) = crate::prepared::files(source, content)?;
    if let Some(snapshot) = store.current()? {
        others.extend(
            snapshot
                .section_names()
                .filter_map(roots::root_of)
                .map(|(_, branch)| branch),
        );
    }
    let mut chosen = vec![held.main];
    chosen.extend(
        others
            .into_iter()
            .filter(|&id| id != held.main && held.get(id).is_some()),
    );
    Ok(Some(chosen))
}

/// Where the timelines queries asked of `source`'s recording are kept: beside its prepared store,
/// which a Clear removes with it.
fn asked_path(source: &Source) -> io::Result<PathBuf> {
    Ok(dashboard_base::prepared::recording_folder(source)?
        .join("timelines")
        .join("asked.json"))
}

/// The timelines queries asked of `source`'s recording ([`ask`]); none when none were kept, or
/// they cannot be read.
fn asked(source: &Source) -> io::Result<Vec<u64>> {
    match std::fs::read(asked_path(source)?) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

/// Keeps each read, change and write of the asked timelines' file whole.
static ASKING: Mutex<()> = Mutex::new(());

/// Notes that a query asked `branch` of `source`: a timeline other than the main one its store
/// publishes nothing of yet is kept as asked, and prepared whenever the recording is from now on
/// ([`timelines`]). Whether it waits for a preparation to take it up.
pub(crate) fn ask(
    source: &Source,
    content: Option<&ContentOptions>,
    branch: u64,
) -> io::Result<bool> {
    let Some(held) = source.replay_ref().timelines() else {
        return Ok(false);
    };
    if branch == held.main || held.get(branch).is_none() {
        return Ok(false);
    }
    let (store, key) = crate::prepared::files(source, content)?;
    if gates::current(&store, &key).is_some_and(|published| {
        names()
            .iter()
            .any(|name| published.gate(branch, name).is_some())
    }) {
        return Ok(false);
    }
    let _asking = ASKING.lock().unwrap_or_else(PoisonError::into_inner);
    let mut kept = asked(source)?;
    if !kept.contains(&branch) {
        kept.push(branch);
        kept.sort_unstable();
        let path = asked_path(source)?;
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let staged = path.with_extension("json.new");
        std::fs::write(&staged, serde_json::to_vec(&kept)?)?;
        std::fs::rename(staged, path)?;
    }
    Ok(true)
}

/// Whether `recording`, undamaged, is of `source`'s campaign and records every hour its replay
/// holds.
fn pairs(source: &Source, recording: &Recording) -> io::Result<bool> {
    let replay = source.replay_ref();
    if recording.campaign() != replay.campaign_id {
        return Ok(false);
    }
    let Some(runs) = columnar_runs::recorded_runs(recording)? else {
        return Ok(false);
    };
    Ok(replay.pages().iter().all(|block| {
        runs.iter().any(|&(branch, first, ordinals)| {
            branch == block.branch
                && first <= block.first_hour
                && i64::from(block.last_hour) < i64::from(first) + i64::from(ordinals)
        })
    }))
}

/// What a native preparation prepares, and how.
#[derive(Default)]
struct Options<'a> {
    /// The families to prepare: every one with a native backend when None.
    families: Option<&'a [&'a str]>,
    /// The run's workers: the process's parallelism when None.
    threads: Option<usize>,
    measure: bool,
    /// Each run walks for one lane doing no family's work ([`Floor`]).
    walk_only: bool,
}

/// Prepares `source` from its native `recording`, reporting each commit to `accepted`, and the
/// change-run index `index` of it, when due ([`Due::rides`]): as a lane of the union run, or alone
/// when the families fail before it starts. A recording whose Campaign rows name another
/// campaign than it is refused whole, before anything is published ([`campaign::check`]).
pub(crate) fn prepare(
    source: &Source,
    content: Option<&ContentOptions>,
    recording: Recording,
    mut index: Option<Due>,
    cancelled: &AtomicBool,
    accepted: &mut dyn FnMut(&Accepted),
) -> io::Result<Accepted> {
    let ran = run(
        source,
        content,
        recording,
        &Options::default(),
        &mut index,
        cancelled,
        accepted,
    );
    // An index the run never took is prepared all the same, as it was before the families.
    let done = match (ran, index) {
        (ran, None) => ran?,
        (Ok(done), Some(due)) => {
            due.alone(cancelled)?;
            done
        }
        (Err(error), Some(due)) => {
            return match due.alone(cancelled) {
                Ok(_) => Err(error),
                Err(unindexed) => Err(io::Error::new(
                    error.kind(),
                    format!("{error}; {unindexed}"),
                )),
            };
        }
    };
    match done.failure {
        None => Ok(done.accepted),
        Some(failure) => {
            accepted(&done.accepted);
            Err(failure)
        }
    }
}

/// Prepares `source` from `recording`, its replay's native recording, as [`prepare`] does: of the
/// families `families` names, or every one with a native backend, on `threads` workers, measured,
/// with `index` as a lane of the union run when due ([`Due::rides`]). A family that failed fails
/// its status rows, which say why; what the job would fail with besides, the index's failure
/// among it, is returned. `walk_only` walks the runs for their tables and worlds alone,
/// preparing nothing ([`Floor`]).
#[allow(clippy::too_many_arguments)]
pub fn measure(
    source: &Source,
    content: Option<&ContentOptions>,
    recording: Recording,
    mut index: Option<Due>,
    families: Option<&[&str]>,
    threads: usize,
    walk_only: bool,
    cancelled: &AtomicBool,
) -> io::Result<(Accepted, Measured, Option<io::Error>)> {
    if !pairs(source, &recording)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "The native recording is not of this recording's campaign and hours",
        ));
    }
    let recording = view(
        source.replay_ref(),
        recording,
        timelines(source, content)?.as_deref(),
    )?;
    let options = Options {
        families,
        threads: Some(threads),
        measure: true,
        walk_only,
    };
    let done = run(
        source,
        content,
        recording,
        &options,
        &mut index,
        cancelled,
        &mut |_| {},
    )?;
    let measured = done
        .measured
        .ok_or_else(|| io::Error::other("The run was not measured"))?;
    // An index the run could not take is prepared alone, as `prepare` prepares it.
    let unindexed = index.map(|due| due.alone(cancelled)).transpose().err();
    let failure = match (done.failure, unindexed) {
        (failure, None) => failure,
        (None, Some(error)) => Some(error),
        (Some(failure), Some(error)) => Some(io::Error::new(
            failure.kind(),
            format!("{failure}; {error}"),
        )),
    };
    Ok((done.accepted, measured, failure))
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
    let family = FamilyId::publisher(family)?;
    let (store, key) = crate::prepared::files(source, content)?;
    let Some(published) = gates::current(&store, &key) else {
        return Ok(false);
    };
    if published.plan().is_none() {
        return Ok(false);
    }
    let shape = Shape::new(&recording(source, timelines(source, content)?.as_deref())?)?;
    for &branch in shape.branches().keys() {
        let Some(gate) = published.gate(branch, family.name()) else {
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
        let family = FamilyId::publisher(name)?;
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
                .any(|name| FamilyId::publisher(name).is_ok_and(|id| id.name() == registered.name))
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
        // It goes on past hours it lost, which carried nothing on: it starts where it can.
        let lost = frontier.holes.last().is_some_and(|hole| {
            hole.held == roots::Held::Lost && hole.last_hour == frontier.last_hour
        });
        going.push(Going {
            branch,
            run: frontier.next.run,
            ordinal: frontier.next.ordinal,
            published,
            carry,
            lost,
        });
    }
    Ok((passes, going))
}

/// What a run of a native preparation accepted, and what failed.
struct Done {
    accepted: Accepted,
    measured: Option<Measured>,
    /// Each family whose failure its publication could not refuse with, and why.
    failure: Option<io::Error>,
}

/// Each family that failed, and why.
pub(crate) type Failed = BTreeMap<FamilyId, crate::QueryError>;

fn run(
    source: &Source,
    content: Option<&ContentOptions>,
    mut recording: Recording,
    options: &Options<'_>,
    due: &mut Option<Due>,
    cancelled: &AtomicBool,
    accepted: &mut dyn FnMut(&Accepted),
) -> io::Result<Done> {
    let whole = Clock::start();
    let started_with = if options.measure { working_set()? } else { 0 };
    check_cancel(cancelled)?;
    let span = replay_frame::timeline::span("campaign");
    campaign::check(&mut recording, source.replay_ref(), cancelled)?;
    drop(span);
    // Families draw artwork on their ordered emits: the renderer is created apart, from the start.
    if content.is_some() {
        shader_pipeline::artwork::warm();
    }
    let span = replay_frame::timeline::span("open");
    let plan = plan::native();
    let chosen = selected(options.families)?;
    let threads = options.threads.unwrap_or_else(parallelism).max(1);
    let shape = Shape::new(&recording)?;
    let (store, key) = crate::prepared::files(source, content)?;
    let context = LoadContext {
        content: content.cloned(),
    };
    let memory = labels::enter(Owner::RUN, Kind::Content);
    let extents = dashboard_text::metadata::Extents::native(source.replay_ref(), &recording)?;
    let loading = Content::new(source, &context, extents, recording.selecting(&[])?);
    let caller = Caller::current();
    drop(span);
    let span = replay_frame::timeline::span("content");
    let started = Clock::start();
    let loaded = content::load_all(
        threads,
        chosen
            .iter()
            .map(|registered| {
                let caller = caller.clone();
                let loading = &loading;
                let owner = labels::owner(registered.name);
                Box::new(move || {
                    caller.enter();
                    let _memory = labels::enter(owner, Kind::Content);
                    (registered.load)(loading)
                }) as Box<dyn FnOnce() -> io::Result<()> + Send + '_>
            })
            .collect(),
    );
    let content_lap = started.lap();
    drop(span);
    drop(memory);
    let loaded_with = if options.measure { working_set()? } else { 0 };
    check_cancel(cancelled)?;
    let span = replay_frame::timeline::span("publisher open");
    let held = status::held(&store)?;
    if held.is_none() && store.current()?.is_some() {
        // A publication no preparation marked: nothing of it is a family's.
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
        // A timeline it does not prepare now keeps what was published of it.
        &|_, branch, frontier| {
            !shape.branches().contains_key(&branch) || shape.accepts(&recording, branch, frontier)
        },
    )?;
    let commits = Arc::new(Commits::default());
    let noted = Arc::clone(&commits);
    let publisher = publisher.with_commits(move || noted.note());
    let mark = status::mark(plan, &shape);
    if resume.replaced || held.as_deref() != Some(&mark[..]) {
        publisher.coordinate(|writer| status::coordinate(writer, plan, &shape, &mark))?;
    }
    drop(span);
    let span = replay_frame::timeline::span("bind");
    let mut failed = Failed::new();
    let mut staged = Vec::new();
    // Where each family goes on, for its lane to borrow.
    let mut goings: Vec<Vec<Going>> = Vec::new();
    for ((registered, family), loaded) in chosen.iter().zip(&families).zip(loaded) {
        if let Err(error) = loaded {
            failed.insert(
                family.id,
                crate::QueryError::from(error).context("Its content could not be loaded"),
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
            Err(error) if publication::halted(&error) => return Err(error),
            Err(error) => {
                failed.insert(
                    family.id,
                    crate::QueryError::from(error).context("Its carry could not be read"),
                );
            }
        }
    }
    let mut members: Vec<Box<dyn Member + '_>> = Vec::new();
    for ((registered, family, stage, passes), going) in staged.into_iter().zip(&goings) {
        let taken = Taken { passes, going };
        let memory = labels::enter(labels::owner(registered.name), Kind::Emit);
        let bound = (registered.bind)(family.id, &loading, stage, taken);
        drop(memory);
        match bound {
            // What it reads of the world is declared before the run, which reads it again.
            Ok(member) => match member.needs() {
                Ok(_) => members.push(member),
                Err(error) => {
                    failed.insert(
                        family.id,
                        crate::QueryError::from(error).context("Its reads could not be declared"),
                    );
                }
            },
            Err(error) => {
                failed.insert(
                    family.id,
                    crate::QueryError::from(error).context("It could not be bound"),
                );
            }
        }
    }
    let prepared: Vec<FamilyId> = families.iter().map(|family| family.id).collect();
    // Those that failed before the run refuse at once: none of their queries waits on it. One
    // whose refusal could not be published yet is tried again after the run.
    refuse(&publisher, &shape, &recording, &mut failed)?;
    let mut ahead = ahead_first(plan, &mut members);
    let narrow: Vec<&'static str> = members[..ahead]
        .iter()
        .map(|member| member.id().name())
        .collect();
    // The families stay bound, publishing nothing, while their floors walk in their place.
    let mut bound = Vec::new();
    if options.walk_only {
        let floors = Floor::over(&members, ahead, &shape)?;
        ahead = usize::from(ahead > 0);
        bound = std::mem::replace(&mut members, floors);
    }
    drop(span);
    let replay = source.replay_ref();
    let mut bytes = status::stored(&store)?;
    let report = |failed: &Failed, bytes: u64| {
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
    // Taken now: an index the run never starts with, or cannot ride, is prepared apart
    // ([`prepare`]).
    let mut index = due
        .take_if(|due| due.rides(&recording))
        .map(Due::ride)
        .transpose()?;
    // Each lane's batch seals and commits beside it while it goes on with the next.
    let sealers = members.len().min(threads);
    let walked = publisher.sealing(sealers, || {
        // Emptied as each run starts and removed as it ends.
        let spool = store.folder().with_extension("spool");
        let walk = Walk {
            shape: &shape,
            members: &mut members,
            index: index.as_mut(),
            world: (replay.format, replay.capture_mode),
            settings: &settings,
            spool: &spool,
            cancelled,
            measuring: options.measure,
            commits: &commits,
            progress: &mut || {
                bytes = status::stored(&store).unwrap_or(bytes);
                accepted(&report(&failed, bytes));
            },
        };
        walk_all(&mut recording, ahead, walk)
    });
    let mut walked = match walked {
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
    failed.append(&mut walked.failed);
    // The index the union run wrote goes in place once its lane went past every segment.
    let indexed = match index {
        Some(index) => {
            let span = replay_frame::timeline::span("index");
            let rode = match walked.ran.index.take() {
                Some(outcome) => Rode::Ran(outcome),
                None => Rode::Stopped,
            };
            let finished = index.finish(&recording, rode);
            drop(span);
            finished
        }
        None => Ok(()),
    };
    let ended_with = if options.measure { working_set()? } else { 0 };
    let worked = if options.walk_only { &[][..] } else { &members[..] };
    let spent: Vec<(&'static str, backend::Spent)> = worked
        .iter()
        .map(|member| (member.id().name(), member.spent()))
        .collect();
    let counts: Vec<(&'static str, Vec<(&'static str, u64)>)> = worked
        .iter()
        .map(|member| (member.id().name(), member.counts()))
        .collect();
    let largest: Vec<(&'static str, Option<usize>)> = worked
        .iter()
        .zip(&walked.ran.outcomes)
        .map(|(member, outcome)| {
            let largest = outcome.as_ref().and_then(|outcome| outcome.largest_output);
            (member.id().name(), largest)
        })
        .collect();
    drop(members);
    drop(bound);
    let span = replay_frame::timeline::span("refuse");
    let unshown = refuse(&publisher, &shape, &recording, &mut failed)?;
    drop(span);
    let done = report(&failed, status::stored(&store)?);
    let measured = options
        .measure
        .then(|| {
            Measured::new(Counted {
                hours: recording.hours(),
                segments: shape.len(),
                threads,
                walk_only: options.walk_only,
                whole: whole.lap(),
                content: content_lap,
                run: walked.lap,
                world: walked.ran.world,
                families: &spent,
                counts: &counts,
                largest: &largest,
                stats: walked.ran.stats,
                tables: walked.ran.tables,
                ahead: walked.ahead.map(|(stats, world, lap, tables)| Narrow {
                    lap,
                    world,
                    stats,
                    tables,
                    families: narrow,
                }),
                working: [started_with, loaded_with, ended_with],
            })
        })
        .transpose()?;
    let failure = match (unshown.is_empty(), indexed) {
        (true, Ok(())) => None,
        (true, Err(error)) => Some(error),
        (false, Ok(())) => Some(io::Error::other(unshown.join("; "))),
        (false, Err(error)) => Some(io::Error::other(format!(
            "{}; {error}",
            unshown.join("; ")
        ))),
    };
    Ok(Done {
        accepted: done,
        measured,
        failure,
    })
}

/// Publishes each family of `failed` as its refusal ([`fail`]). One its publication now refuses
/// with leaves `failed`: its branches' ends say why. One with nothing left to refuse stays, for
/// its status rows to say why; so does one whose refusal could not be published, which is
/// returned by name, with why, for the job to say. Publication's stop is the run's own failure.
fn refuse(
    publisher: &Publisher<'_>,
    shape: &Shape,
    recording: &Recording,
    failed: &mut Failed,
) -> io::Result<Vec<String>> {
    let mut unshown = Vec::new();
    let mut shown = Vec::new();
    for (&family, why) in failed.iter() {
        match fail(publisher, shape, recording, family, why) {
            Ok(true) => shown.push(family),
            Ok(false) => {}
            Err(error) if publication::halted(&error) => return Err(error),
            Err(error) => unshown.push(format!(
                "{}: {why}; its refusal could not be published: {error}",
                family.name()
            )),
        }
    }
    for family in shown {
        failed.remove(&family);
    }
    Ok(unshown)
}

/// Publishes `family`'s failure, `why`, through a stage of its own, as its refusal from the first
/// hour it left unpublished on each branch of `shape` it had not ended: its queries answer what
/// it published before and refuse the rest by its name, never waiting on a run it left. Whether
/// it refused any.
fn fail(
    publisher: &Publisher<'_>,
    shape: &Shape,
    recording: &Recording,
    family: FamilyId,
    why: &crate::QueryError,
) -> io::Result<bool> {
    let mut stage = publisher.stage(family)?;
    let mut refused = false;
    for &branch in shape.branches().keys() {
        let committed = publisher.committed(family, branch);
        let frontier = committed.as_ref().map(|published| &published.frontier);
        if frontier.is_some_and(|frontier| frontier.end.is_some()) {
            continue;
        }
        let Some((at, first)) = shape.unpublished(branch, frontier) else {
            continue;
        };
        let placed = *shape.get(at);
        let first_hour = i32::try_from(first - placed.first)
            .ok()
            .and_then(|past| placed.first_hour.checked_add(past))
            .ok_or_else(|| io::Error::other("A family's next hour passes the clock"))?;
        let hours = Hours {
            branch,
            run: placed.run,
            first,
            last: placed.last,
            first_hour,
            last_hour: placed.last_hour,
            ends_segment: true,
            ends_branch: placed.ends_branch,
            proof: shape.proof(recording, at),
        };
        let made = Made::Refused {
            hour: first_hour,
            reason: why.clone(),
            from: None,
        };
        stage.processed(hours, made, None, None)?;
        refused = true;
    }
    if refused {
        stage.send()?;
        stage.settle()?;
    }
    Ok(refused)
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
    /// The change-run index the union run writes as one more lane, when due.
    pub index: Option<&'w mut Riding>,
    /// The format and capture mode the walks' worlds read.
    pub world: (FormatVersion, CaptureMode),
    pub settings: &'w Settings,
    /// The folder the run's items wait in for their emits.
    pub spool: &'w std::path::Path,
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
    /// What became of the change-run index's lane, when the run carried it.
    pub index: Option<Outcome>,
    /// Cycles the walks' worlds took, when measured.
    pub world: u64,
    /// The most each segment's walks charged their tables, by segment.
    pub tables: Vec<usize>,
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
    /// The narrow run's stats, world cycles, time and tables by segment, when it had something
    /// to do.
    pub ahead: Option<(segments::Stats, u64, Lap, Vec<usize>)>,
    /// Each family that failed in its run, or committing its last batch, and why.
    pub failed: Failed,
}

/// Waits for each of `members`' last batch to be committed, noting in `failed` each one that
/// failed in its run, as `outcomes` give them, or committing that batch: its failure is its own.
/// Publication's stop is the run's.
fn settle(
    members: &mut [Box<dyn Member + '_>],
    outcomes: &[Option<Outcome>],
    failed: &mut Failed,
) -> io::Result<()> {
    for (member, outcome) in members.iter_mut().zip(outcomes) {
        let _span = replay_frame::timeline::span("settle").lane(member.id().name());
        let _memory = labels::enter(labels::owner(member.id().name()), Kind::Publish);
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
                crate::QueryError::from(&failure.error).context(&format!(
                    "Its {:?} {how} at segment {}",
                    failure.operation, failure.at
                )),
            );
        }
        match member.settle() {
            Ok(()) => {}
            Err(error) if publication::halted(&error) => return Err(error),
            Err(error) => {
                failed.entry(member.id()).or_insert_with(|| {
                    crate::QueryError::from(error).context("Its last batch was not committed")
                });
            }
        }
    }
    Ok(())
}

/// The tables a walk of families reading `reads` decodes: theirs, and the campaign row every
/// world frame holds.
fn decoded(reads: impl IntoIterator<Item = TableId>) -> Vec<TableId> {
    let mut tables: Vec<TableId> = reads.into_iter().chain([TableId::Campaign]).collect();
    tables.sort_unstable_by_key(|table| table.index());
    tables.dedup();
    tables
}

/// Makes `recording`'s walks decode what `members` with something to do read, and `also`
/// ([`decoded`]).
fn select(
    recording: &mut Recording,
    shape: &Shape,
    members: &[Box<dyn Member + '_>],
    also: &[TableId],
) {
    recording.select(&decoded(
        members
            .iter()
            .filter(|member| !member.idle(shape))
            .flat_map(|member| member.tables().iter().copied())
            .chain(also.iter().copied()),
    ));
}

/// Runs `walk`'s members over `recording`, the first `ahead` of them, those of families prepared
/// ahead ([`ahead_first`]), before the rest: in a narrow run of their own over the whole recording
/// that decodes their tables alone, their last batches committed as it ends; then the rest, and
/// the change-run index when it rides, in the union run, theirs committed as it ends
/// ([`settle`]). A cancelled narrow run leaves the union run undone.
pub(crate) fn walk_all(
    recording: &mut Recording,
    ahead: usize,
    walk: Walk<'_, '_>,
) -> io::Result<Walks> {
    let Walk {
        shape,
        members,
        index,
        world,
        settings,
        spool,
        cancelled,
        measuring,
        commits,
        progress,
    } = walk;
    let (narrow, union) = members.split_at_mut(ahead);
    let mut outcomes: Vec<Option<Outcome>> = narrow.iter().map(|_| None).collect();
    let mut failed = Failed::new();
    let mut first = None;
    if narrow.iter().any(|member| !member.idle(shape)) {
        labels::phase("ahead run");
        select(recording, shape, narrow, &[]);
        let span = replay_frame::timeline::span("ahead run");
        let clock = Clock::start();
        let alone = Walk {
            shape,
            members: narrow,
            index: None,
            world,
            settings,
            spool,
            cancelled,
            measuring,
            commits,
            progress: &mut *progress,
        };
        let mut ran = run_walk(recording, alone)?;
        let lap = clock.lap();
        drop(span);
        outcomes = std::mem::take(&mut ran.outcomes);
        if ran.cancelled {
            outcomes.extend(union.iter().map(|_| None));
            return Ok(Walks {
                ran: Ran { outcomes, ..ran },
                lap,
                ahead: None,
                failed,
            });
        }
        settle(narrow, &outcomes, &mut failed)?;
        first = Some((ran.stats, ran.world, lap, ran.tables));
    }
    labels::phase("union run");
    let indexed = index.as_ref().map_or(&[][..], |index| index.tables());
    select(recording, shape, union, indexed);
    let span = replay_frame::timeline::span("union run");
    let clock = Clock::start();
    let together = Walk {
        shape,
        members: union,
        index,
        world,
        settings,
        spool,
        cancelled,
        measuring,
        commits,
        progress,
    };
    let mut ran = run_walk(recording, together)?;
    let lap = clock.lap();
    drop(span);
    if !ran.cancelled {
        settle(union, &ran.outcomes, &mut failed)?;
    }
    labels::phase("run end");
    outcomes.append(&mut ran.outcomes);
    Ok(Walks {
        ran: Ran { outcomes, ..ran },
        lap,
        ahead: first,
        failed,
    })
}

/// Runs every member of `walk` with something to do, and its change-run index's lane when it has
/// one, over `recording` in one run of the executor, on a thread of its own: this thread reports
/// the commits it makes.
pub(crate) fn run_walk(recording: &Recording, walk: Walk<'_, '_>) -> io::Result<Ran> {
    let Walk {
        shape,
        members,
        mut index,
        world: (format, mode),
        settings,
        spool,
        cancelled,
        measuring,
        commits,
        progress,
    } = walk;
    let places: Vec<usize> = (0..members.len())
        .filter(|&at| !members[at].idle(shape))
        .collect();
    let mut outcomes: Vec<Option<Outcome>> = members.iter().map(|_| None).collect();
    if places.is_empty() && index.is_none() {
        return Ok(Ran {
            stats: segments::Stats::default(),
            outcomes,
            index: None,
            world: 0,
            tables: Vec::new(),
            cancelled: false,
        });
    }
    let mut needs = Vec::with_capacity(places.len() + 1);
    for &at in &places {
        needs.push(members[at].needs()?);
    }
    // The index's lane, last, reads none of the world.
    if index.is_some() {
        needs.push(Needs::default());
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
        let running = scope.spawn(|| -> io::Result<segments::Ran> {
            let _ended = Ended(commits);
            let _memory = labels::enter(Owner::RUN, Kind::Executor);
            caller.enter();
            let mut lanes = Vec::with_capacity(needs.len());
            let working = members
                .iter_mut()
                .enumerate()
                .filter_map(|(at, member)| places.contains(&at).then_some(member));
            for member in working {
                lanes.push(
                    member
                        .lane(&walking)
                        .refuses(|error| crate::refusal(error).is_some()),
                );
            }
            if let Some(index) = index.as_deref_mut() {
                lanes.push(index.lane(cancelled, walking.job.clone())?);
            }
            let worlds = |_: &Segment, visited: &[usize]| {
                let union = Needs::union(visited.iter().map(|&lane| &needs[lane]));
                World::new(&union, format, mode).map(|world| Walked::new(world, &walking))
            };
            segments::run_in(recording, settings, spool, &cancellation, &worlds, lanes)
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
    let indexed = if index.is_some() {
        ran_outcomes.next()
    } else {
        None
    };
    Ok(Ran {
        stats: ran.stats,
        outcomes,
        index: indexed,
        world: walking.world.into_inner(),
        tables: ran.tables,
        cancelled: ran.cancelled || cancellation.is_cancelled(),
    })
}
