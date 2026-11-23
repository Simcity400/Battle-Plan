
use playhead::{
    Timeline,
    pages::{Pages, WorldPages},
};
use replay::campaign::Replay;
use replay_recording::columnar_frames::Recording;
use replay_columnar::state::Kind;
use replay_data::BranchId;
use std::{
    io,
    path::Path,
    sync::{Arc, Mutex, PoisonError, Weak},
};

#[cfg(test)]
pub mod tests;

pub struct Units {
    pub name: String,
    pub pages: Arc<dyn Pages>,
    pub timeline: Timeline,
    pub kinds: Arc<Vec<Vec<Kind>>>,
}

pub struct Read {
    pub pages: Arc<dyn Pages>,
    pub kinds: Arc<Vec<Vec<Kind>>>,
    pub branches: Arc<[BranchId]>,
}

type Held = (String, Weak<dyn Pages>, Arc<Vec<Vec<Kind>>>, Arc<[BranchId]>);

static OPENED: Mutex<Vec<Held>> = Mutex::new(Vec::new());

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

pub fn of(replay: &Replay, branch: BranchId) -> io::Result<Units> {
    let pieces = replay.pieces(branch);
    if pieces.is_empty() {
        return Err(io::Error::other(format!("The recording holds no branch {branch}")));
    }
    let recording = replay.recording();
    let name = name(recording);
    let read = shared(&name, || read(recording))?;
    let timeline = Timeline::branch(&*read.pages, &read.branches, branch, &pieces)?;
    Ok(Units { name, pages: read.pages, timeline, kinds: read.kinds })
}

pub fn at_path(path: &Path, branch: Option<BranchId>) -> io::Result<(Units, BranchId)> {
    let replay = Replay::open_timelines(path)?;
    let branch = match branch {
        Some(branch) => branch,
        None => replay.timelines().map(|timelines| timelines.main).or_else(|| replay.index.first().map(|block| block.branch)).ok_or_else(|| io::Error::other("The recording holds no recorded hour"))?,
    };
    Ok((of(&replay, branch)?, branch))
}

pub fn branch_playing(replay: &Replay, pages: &dyn Pages, timeline: &Timeline) -> Option<BranchId> {
    let branches = run_branches(replay.recording());
    if branches.len() != pages.runs().len() {
        return None;
    }
    replay.branches.iter().map(|start| start.id).find(|&branch| Timeline::branch(pages, &branches, branch, &replay.pieces(branch)).is_ok_and(|held| held.pieces() == timeline.pieces()))
}

fn run_branches(recording: &Recording) -> Vec<BranchId> {
    (0..recording.run_count()).filter_map(|run| recording.run(run).ok().map(|run| run.branch)).collect()
}

fn read(recording: &Recording) -> io::Result<Read> {
    let opened = WorldPages::shared(recording)?;
    let branches: Arc<[BranchId]> = opened.branches().into();
    let kinds = Arc::new(opened.schema().to_vec());
    Ok(Read { pages: opened, kinds, branches })
}

pub use playhead::pages::name;

fn held(name: &str) -> Option<Read> {
    let opened = OPENED.lock().unwrap_or_else(PoisonError::into_inner);
    let (_, pages, kinds, branches) = opened.iter().find(|(held, ..)| held == name)?;
    Some(Read { pages: pages.upgrade()?, kinds: Arc::clone(kinds), branches: Arc::clone(branches) })
}
