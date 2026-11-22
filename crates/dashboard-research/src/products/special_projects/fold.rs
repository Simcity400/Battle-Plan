
use super::{ProjectDate, ProjectEpisode, ProjectState, SpecialProjectRun};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const COMPLETION_MARKER: u32 = 0x4c17;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub owner: u32,
    pub local: u32,
    pub definition: u32,
    pub program: u64,
    pub started: bool,
    pub offset: u32,
    pub not_completed: u8,
    pub prototype_progress: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub ordinal: u32,
    pub completion: bool,
    pub date: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Person {
    Unassigned,
    Unknown(String),
    Character(u32),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Look {
    pub definition: u32,
    pub person: Person,
    pub as_of: Option<i32>,
}

pub fn completed(row: &Row) -> bool {
    row.not_completed == 0 && row.offset == 0xe8
}

pub fn state(row: &Row) -> ProjectState {
    match row.offset {
        0x18 if row.started && row.program != 0 => ProjectState::Running,
        0x18 => ProjectState::Cancelled,
        0x168 => ProjectState::Paused,
        0x1e8 => ProjectState::Cancelled,
        0xe8 if completed(row) => ProjectState::Completed,
        _ => ProjectState::Unknown,
    }
}

fn visible(row: &Row, open: bool) -> bool {
    row.started || completed(row) || open || (state(row) == ProjectState::Cancelled && row.offset != 0x18)
}

fn progress(row: &Row) -> i32 {
    if completed(row) { 100 } else { row.prototype_progress.clamp(-100, 100) }
}

pub fn completion_date(history: &[Entry], observed: i32, campaign_start: i32) -> Option<ProjectDate> {
    history
        .iter()
        .filter(|entry| entry.completion && entry.date <= observed && entry.date >= campaign_start)
        .max_by_key(|entry| entry.ordinal)
        .map(|entry| ProjectDate { hour: entry.date, exact: true })
}

pub type Possession = (u32, u32, i32);

pub type Key = (u32, u32, u32);

pub struct Observed<'a> {
    pub row: &'a Row,
    pub read: i32,
    pub history: Option<(i32, &'a [Entry])>,
    pub person: Person,
}

#[derive(Clone, Debug)]
struct Track {
    id: usize,
    episode: Option<usize>,
    start: i32,
    since: i32,
    ended: bool,
    completion: Option<ProjectDate>,
    cancellation: Option<ProjectDate>,
    choices: Option<i32>,
    gap: bool,
}

struct Assigned<'h> {
    key: Key,
    state: ProjectState,
    history: Option<(i32, &'h [Entry])>,
    iterations: Option<i32>,
}

#[derive(Default)]
pub struct Fold {
    pub campaign_start: i32,
    pub episodes: Vec<ProjectEpisode>,
    pub looks: Vec<Look>,
    look_ids: HashMap<Look, u32>,
    open: BTreeMap<Key, Track>,
    tracks: usize,
    holder: Option<Possession>,
    watched: Option<Key>,
    seen: Vec<(i32, i32, usize)>,
    pub warnings: BTreeMap<String, usize>,
}

fn choices(entries: &[Entry]) -> i32 {
    entries.iter().filter(|entry| !entry.completion).count() as i32
}

impl Fold {
    pub fn new(campaign_start: i32) -> Self {
        Self { campaign_start, ..Self::default() }
    }

    pub fn watch(&mut self, key: Key) {
        self.watched = Some(key);
    }

    pub fn assigned(&self, hour: i32) -> Option<usize> {
        self.seen.iter().rev().find(|&&(first, last, _)| first <= hour && hour <= last).map(|&(_, _, id)| id)
    }

    pub fn open_episodes(&self) -> impl Iterator<Item = usize> + '_ {
        self.open.values().filter_map(|track| track.episode)
    }

    pub fn extend(&mut self, before: i32, last: i32) {
        for track in self.open.values() {
            if let Some(run) = track.episode.and_then(|index| self.episodes[index].runs.last_mut()).filter(|run| run.last_hour == before) {
                run.last_hour = last;
            }
        }
        if let Some(seen) = self.seen.last_mut().filter(|seen| seen.1 == before) {
            seen.1 = last;
        }
    }

    fn look(&mut self, look: Look) -> u32 {
        if let Some(&id) = self.look_ids.get(&look) {
            return id;
        }
        let id = self.looks.len() as u32;
        self.look_ids.insert(look.clone(), id);
        self.looks.push(look);
        id
    }

    pub fn hold(&mut self, holder: Possession) {
        match self.holder {
            Some(held) if (held.0, held.1) != (holder.0, holder.1) => self.open.clear(),
            Some(held) if held != holder => self.open.values_mut().for_each(|track| track.gap = true),
            _ => {}
        }
        self.holder = Some(holder);
    }

    fn continues(&self, track: &Track, state: ProjectState, observed: &Observed<'_>, hour: i32) -> bool {
        if track.ended && resumed(state) {
            return false;
        }
        if track.gap && !(track.ended && terminal(state)) {
            return false;
        }
        let history = observed.history.filter(|&(read, _)| read >= track.since);
        if let (Some(known), Some((_, entries))) = (track.choices, history)
            && choices(entries) < known
        {
            return false;
        }
        let marker = history.and_then(|(_, entries)| completion_date(entries, hour, self.campaign_start));
        !(state == ProjectState::Completed && track.completion.as_ref().zip(marker).is_some_and(|(known, marker)| marker.hour > known.hour))
    }

    fn assign<'h>(&mut self, observed: &Observed<'h>, first: i32, last: i32) -> Option<Assigned<'h>> {
        let row = observed.row;
        let state = state(row);
        let key = (row.owner, row.local, row.definition);
        let open = self.open.get(&key);
        if !visible(row, open.is_some()) {
            return None;
        }
        if !open.is_some_and(|track| self.continues(track, state, observed, first)) {
            let id = self.tracks;
            self.tracks += 1;
            self.open.insert(key, Track { id, episode: None, start: first, since: observed.read, ended: false, completion: None, cancellation: None, choices: None, gap: false });
        }
        let campaign_start = self.campaign_start;
        let track = self.open.get_mut(&key).expect("an open track");
        let history = observed.history.filter(|&(read, _)| read >= track.since);
        let iterations = history.map(|(_, entries)| choices(entries));
        if terminal(state) {
            track.ended = true;
        } else if resumed(state) {
            track.ended = false;
        }
        if iterations.is_some() {
            track.choices = iterations;
        }
        if state == ProjectState::Completed {
            let marker = history.and_then(|(_, entries)| completion_date(entries, first, campaign_start));
            track.completion = match (track.completion.take(), marker) {
                (Some(known), Some(marker)) if !known.exact => Some(marker),
                (Some(known), _) => Some(known),
                (None, marker) => Some(marker.unwrap_or(ProjectDate { hour: first, exact: false })),
            };
        }
        if state == ProjectState::Cancelled && track.cancellation.is_none() {
            track.cancellation = Some(ProjectDate { hour: first, exact: false });
        }
        track.gap = false;
        if self.watched == Some(key) {
            self.seen.push((first, last, track.id));
        }
        Some(Assigned { key, state, history, iterations })
    }

    pub fn absent(&mut self, holder: Possession, first: i32, last: i32, proven: impl Fn(u32) -> bool, present: &[Observed<'_>]) {
        self.hold(holder);
        let seen: Vec<Key> = present.iter().filter_map(|observed| self.assign(observed, first, last).map(|assigned| assigned.key)).collect();
        self.open.retain(|key, _| !proven(key.0) || seen.contains(key));
    }

    pub fn observe(&mut self, holder: Possession, first: i32, last: i32, projects: &[Observed<'_>]) {
        self.hold(holder);
        let mut next = BTreeSet::new();
        for observed in projects {
            let Some(Assigned { key, state, history, iterations }) = self.assign(observed, first, last) else { continue };
            next.insert(key);
            let row = observed.row;
            let track = &self.open[&key];
            let completion = track.completion.clone().filter(|_| state == ProjectState::Completed);
            let cancellation = track.cancellation.clone().filter(|_| state == ProjectState::Cancelled);
            let (episode, start) = (track.episode, track.start);
            let index = match episode {
                Some(index) => index,
                None => {
                    let index = self.episodes.len();
                    self.episodes.push(ProjectEpisode { id: format!("{}:{start}:{index}", row.local), owner: row.owner, local: row.local, start: ProjectDate { hour: start, exact: false }, runs: Vec::new() });
                    self.open.get_mut(&key).expect("an open track").episode = Some(index);
                    index
                }
            };
            let done = state == ProjectState::Completed;
            let iterations_read = history.and_then(|(read, entries)| (!(done && entries.iter().any(|entry| entry.completion))).then_some(read));
            let mut look = Look { definition: row.definition, person: observed.person.clone(), as_of: None };
            if done && let Some(before) = self.episodes[index].runs.last().filter(|run| run.last_hour.checked_add(1) == Some(first)) {
                let held = &self.looks[before.presentation as usize];
                if matches!(held.person, Person::Character(_)) {
                    look.person = held.person.clone();
                    look.as_of = Some(held.as_of.unwrap_or(before.last_hour));
                }
            }
            if state == ProjectState::Unknown {
                *self.warnings.entry(format!("Project {} is in a state the replay does not know ({:#x})", row.local, row.offset)).or_default() += 1;
            }
            let presentation = self.look(look);
            let run = SpecialProjectRun { first_hour: first, last_hour: last, presentation, progress: progress(row), state, completion, cancellation, iterations, iterations_read };
            let runs = &mut self.episodes[index].runs;
            match runs.last_mut() {
                Some(held) if held.last_hour.checked_add(1) == Some(first) && same(held, &run) => merge(held, &run),
                _ => runs.push(run),
            }
        }
        self.open.retain(|key, _| next.contains(key));
    }
}

fn terminal(state: ProjectState) -> bool {
    matches!(state, ProjectState::Completed | ProjectState::Cancelled)
}

fn resumed(state: ProjectState) -> bool {
    matches!(state, ProjectState::Running | ProjectState::Paused)
}

pub fn merge(held: &mut SpecialProjectRun, next: &SpecialProjectRun) {
    held.last_hour = next.last_hour;
}

pub fn same(a: &SpecialProjectRun, b: &SpecialProjectRun) -> bool {
    (a.presentation, a.progress, a.state, &a.completion, &a.cancellation, a.iterations, a.iterations_read.is_some())
        == (b.presentation, b.progress, b.state, &b.completion, &b.cancellation, b.iterations, b.iterations_read.is_some())
}
