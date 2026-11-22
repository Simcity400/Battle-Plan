
use super::{ProjectDate, ProjectEpisode, ProjectState, SpecialProjectRun};
use std::collections::{BTreeMap, HashMap};

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

pub struct Observed<'a> {
    pub row: &'a Row,
    pub read: i32,
    pub history: Option<(i32, &'a [Entry])>,
    pub person: Person,
}

#[derive(Default)]
pub struct Fold {
    pub campaign_start: i32,
    pub episodes: Vec<ProjectEpisode>,
    pub looks: Vec<Look>,
    look_ids: HashMap<Look, u32>,
    open: BTreeMap<(u32, u32, u32), usize>,
    holder: Option<(u32, u32)>,
    last_iterations: HashMap<usize, i32>,
    since: HashMap<usize, i32>,
    pub warnings: BTreeMap<String, usize>,
}

impl Fold {
    pub fn new(campaign_start: i32) -> Self {
        Self { campaign_start, ..Self::default() }
    }

    pub fn open_episodes(&self) -> impl Iterator<Item = usize> + '_ {
        self.open.values().copied()
    }

    pub fn extend(&mut self, before: i32, last: i32) {
        for index in self.open.values() {
            if let Some(run) = self.episodes[*index].runs.last_mut().filter(|run| run.last_hour == before) { run.last_hour = last; }
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

    pub fn observe(&mut self, holder: (u32, u32), first: i32, last: i32, projects: &[Observed<'_>]) {
        if self.holder.is_some_and(|held| held != holder) {
            self.open.clear();
        }
        self.holder = Some(holder);
        let mut next = BTreeMap::new();
        for observed in projects {
            let row = observed.row;
            let state = state(row);
            let key = (row.owner, row.local, row.definition);
            if !visible(row, self.open.contains_key(&key)) {
                continue;
            }
            let done = state == ProjectState::Completed;
            let restarted = self.open.get(&key).is_some_and(|&index| {
                self.episodes[index].runs.last().is_some_and(|run| {
                    matches!(run.state, ProjectState::Completed | ProjectState::Cancelled) && matches!(state, ProjectState::Running | ProjectState::Paused)
                })
            });
            if restarted {
                self.open.remove(&key);
            }
            let held = self.open.get(&key).map(|index| (*index, self.since[index]));
            let since = held.map_or(observed.read, |(_, since)| since);
            let history = observed.history.filter(|&(read, _)| read >= since);
            let iterations = history.map(|(_, entries)| entries.iter().filter(|entry| !entry.completion).count() as i32);
            if held.and_then(|(index, _)| self.last_iterations.get(&index)).zip(iterations).is_some_and(|(&old, new)| old > new) {
                self.open.remove(&key);
            }
            let index = match self.open.get(&key) {
                Some(&index) => index,
                None => {
                    let index = self.episodes.len();
                    self.episodes.push(ProjectEpisode { id: format!("{}:{first}:{index}", row.local), owner: row.owner, local: row.local, start: ProjectDate { hour: first, exact: false }, runs: Vec::new() });
                    self.since.insert(index, history.map_or(observed.read, |(read, _)| read.min(observed.read)));
                    index
                }
            };
            next.insert(key, index);
            let previous = self.episodes[index].runs.last().cloned();
            let completion = done.then(|| {
                history
                    .and_then(|(_, entries)| completion_date(entries, first, self.campaign_start))
                    .or_else(|| previous.as_ref().and_then(|run| run.completion.clone()))
                    .unwrap_or(ProjectDate { hour: first, exact: false })
            });
            let cancellation = (state == ProjectState::Cancelled).then(|| previous.as_ref().and_then(|run| run.cancellation.clone()).unwrap_or(ProjectDate { hour: first, exact: false }));
            let mut look = Look { definition: row.definition, person: observed.person.clone(), as_of: None };
            if done && let Some(before) = previous.as_ref().filter(|run| run.last_hour.checked_add(1) == Some(first)) {
                let held = &self.looks[before.presentation as usize];
                if matches!(held.person, Person::Character(_)) {
                    look.person = held.person.clone();
                    look.as_of = Some(held.as_of.unwrap_or(before.last_hour));
                }
            }
            if let Some(count) = iterations {
                self.last_iterations.insert(index, count);
            }
            if state == ProjectState::Unknown {
                *self.warnings.entry(format!("Project {} is in a state the replay does not know ({:#x})", row.local, row.offset)).or_default() += 1;
            }
            let presentation = self.look(look);
            let run = SpecialProjectRun { first_hour: first, last_hour: last, presentation, progress: progress(row), state, completion, cancellation, iterations };
            let runs = &mut self.episodes[index].runs;
            match runs.last_mut() {
                Some(held) if held.last_hour.checked_add(1) == Some(first) && same(held, &run) => held.last_hour = last,
                _ => runs.push(run),
            }
        }
        self.open = next;
    }
}

pub fn same(a: &SpecialProjectRun, b: &SpecialProjectRun) -> bool {
    (a.presentation, a.progress, a.state, &a.completion, &a.cancellation, a.iterations) == (b.presentation, b.progress, b.state, &b.completion, &b.cancellation, b.iterations)
}
