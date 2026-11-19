//! Where the map's playhead takes its pages: the format-12 recording a document has open, read
//! whole, and the timeline one branch plays over them: the recording's own pieces of the branch
//! ([`Replay::pieces`]), each cut to the runs of its segment. A revision's pages are read once
//! while anyone holds them, whoever asks and for whichever branch.

use playhead::{
    Piece, Timeline,
    pages::{Pages, Run, Span, WorldPages},
};
use replay::{
    campaign::{Replay, timelines},
    columnar_frames::Recording,
};
use replay_columnar::state::Kind;
use replay_data::{BranchId, rows::TableId};
use std::{
    io,
    path::Path,
    sync::{Arc, Mutex, PoisonError, Weak},
};

#[cfg(test)]
pub mod tests;

/// A recording's pages as units, the timeline of one branch over them, and its tables' column
/// kinds.
pub struct Units {
    /// Recording identity shared by map and follow readers.
    pub name: String,
    pub pages: Arc<dyn Pages>,
    pub timeline: Timeline,
    pub kinds: Arc<Vec<Vec<Kind>>>,
}

/// A revision's pages as read, its column kinds, and the branch each of its runs records.
pub struct Read {
    pub pages: Arc<dyn Pages>,
    pub kinds: Arc<Vec<Vec<Kind>>>,
    pub branches: Arc<[BranchId]>,
}

type Held = (String, Weak<dyn Pages>, Arc<Vec<Vec<Kind>>>, Arc<[BranchId]>);

static OPENED: Mutex<Vec<Held>> = Mutex::new(Vec::new());

/// The reads under way, by name: a second caller waits for the first's read and shares it (the
/// map and follow open the same recording at once), never reads beside it.
static BUILDING: Mutex<Vec<(String, Weak<Mutex<()>>)>> = Mutex::new(Vec::new());

fn build_gate(name: &str) -> Arc<Mutex<()>> {
    let mut building = BUILDING.lock().unwrap_or_else(PoisonError::into_inner);
    building.retain(|(_, gate)| gate.strong_count() > 0);
    if let Some(gate) = building.iter().find(|(held, _)| held == name).and_then(|(_, gate)| gate.upgrade()) {
        return gate;
    }
    let gate = Arc::new(Mutex::new(()));
    building.push((name.to_owned(), Arc::downgrade(&gate)));
    gate
}

/// The revision named `name` as read: the one held, or `read`'s, read once however many ask at once.
pub fn shared(name: &str, read: impl FnOnce() -> io::Result<Read>) -> io::Result<Read> {
    if let Some(held) = held(name) {
        return Ok(held);
    }
    let gate = build_gate(name);
    let _building = gate.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(held) = held(name) {
        return Ok(held);
    }
    let read = read()?;
    let mut opened = OPENED.lock().unwrap_or_else(PoisonError::into_inner);
    opened.retain(|(_, pages, ..)| pages.strong_count() > 0);
    opened.push((name.to_owned(), Arc::downgrade(&read.pages), Arc::clone(&read.kinds), Arc::clone(&read.branches)));
    Ok(read)
}

/// The units of branch `branch` of the recording `replay` reads, its pages shared with it; a
/// branch the recording does not hold is refused by name.
pub fn of(replay: &Replay, branch: BranchId) -> io::Result<Units> {
    let pieces = replay.pieces(branch);
    if pieces.is_empty() {
        return Err(io::Error::other(format!("The recording holds no branch {branch}")));
    }
    let recording = replay.recording();
    let name = name(recording);
    let read = shared(&name, || read(recording.selecting(&TableId::ALL)?))?;
    let cut = cut(read.pages.runs(), &read.branches, &pieces);
    if cut.is_empty() {
        return Err(io::Error::other(format!("Branch {branch} holds no readable hour of the recording")));
    }
    let timeline = Timeline::new(&*read.pages, cut).map_err(|error| io::Error::new(error.kind(), format!("Branch {branch} of the recording: {error}")))?;
    Ok(Units { name, pages: read.pages, timeline, kinds: read.kinds })
}

/// The units of the recording at `path` read as its timelines, of branch `branch` or of the
/// main timeline when none is named, and that branch.
pub fn at_path(path: &Path, branch: Option<BranchId>) -> io::Result<(Units, BranchId)> {
    let replay = Replay::open_timelines(path)?;
    let branch = match branch {
        Some(branch) => branch,
        None => replay.timelines().map(|timelines| timelines.main).or_else(|| replay.index.first().map(|block| block.branch)).ok_or_else(|| io::Error::other("The recording holds no recorded hour"))?,
    };
    Ok((of(&replay, branch)?, branch))
}

/// The playhead's pieces of a timeline's `pieces`: each cut to the runs of its segment that hold
/// its hours, in rising hours. Hours no run holds (a recorded gap, play the recorder missed) stay
/// a gap between two pieces.
fn cut(runs: &[Run], branches: &[BranchId], pieces: &[timelines::Piece]) -> Vec<Piece> {
    let mut cut = Vec::new();
    for piece in pieces {
        let from = cut.len();
        for (run, (held, &branch)) in runs.iter().zip(branches).enumerate() {
            let (first, last) = (piece.first_hour.max(held.first_hour), piece.last_hour.min(held.last_hour()));
            if branch == piece.segment && first <= last {
                cut.push(Piece { run, first, last });
            }
        }
        cut[from..].sort_unstable_by_key(|piece| piece.first);
    }
    cut
}

/// The branch of `replay` whose timeline `pages` and `timeline` play, the first when several play
/// the same; none when no branch of it does.
pub fn branch_playing(replay: &Replay, pages: &dyn Pages, timeline: &Timeline) -> Option<BranchId> {
    let branches = run_branches(replay.recording());
    if branches.len() != pages.runs().len() {
        return None;
    }
    replay.branches.iter().map(|start| start.id).find(|&branch| cut(pages.runs(), &branches, &replay.pieces(branch)) == timeline.pieces())
}

/// The branch each run [`Span::Whole`] reads records: the readable runs, in order.
fn run_branches(recording: &Recording) -> Vec<BranchId> {
    (0..recording.run_count()).filter_map(|run| recording.run(run).ok().map(|run| run.branch)).collect()
}

/// `recording`'s every readable run, read; [`shared`] keeps it while anyone holds its pages.
fn read(recording: Recording) -> io::Result<Read> {
    let opened = WorldPages::over(recording, Span::Whole)?;
    let branches: Arc<[BranchId]> = run_branches(opened.recording()).into();
    let kinds = Arc::new(opened.schema().to_vec());
    let pages: Arc<dyn Pages> = Arc::new(opened);
    if branches.len() != pages.runs().len() {
        return Err(io::Error::other(format!("The recording's pages read {} runs of {} readable", pages.runs().len(), branches.len())));
    }
    Ok(Read { pages, kinds, branches })
}

/// A revision's name: its root's digest and length, whatever path reached it.
pub fn name(recording: &Recording) -> String {
    let revision = recording.revision();
    let digest: String = revision.root.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{digest}:{}", revision.length)
}

fn held(name: &str) -> Option<Read> {
    let opened = OPENED.lock().unwrap_or_else(PoisonError::into_inner);
    let (_, pages, kinds, branches) = opened.iter().find(|(held, ..)| held == name)?;
    Some(Read { pages: pages.upgrade()?, kinds: Arc::clone(kinds), branches: Arc::clone(branches) })
}
