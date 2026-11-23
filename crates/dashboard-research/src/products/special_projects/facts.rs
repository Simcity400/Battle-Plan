
use super::ProjectState;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Entry {
    pub ordinal: u32,
    pub reward: u32,
    pub option: u32,
    pub date: i32,
}

impl Entry {
    pub fn completion(&self) -> bool {
        self.reward == COMPLETION_MARKER && self.option == COMPLETION_MARKER
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Person {
    Unassigned,
    Unknown(String),
    Character(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Program {
    Unassigned,
    Unknown(String),
    Read { read: i32, scientist: u64 },
    Unread(u32),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Look {
    pub definition: u32,
    pub person: Person,
    pub read: Option<i32>,
    pub last: Option<(u32, i32)>,
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

fn visible(row: &Row) -> bool {
    row.started || completed(row) || (state(row) == ProjectState::Cancelled && row.offset != 0x18)
}

fn progress(row: &Row) -> i32 {
    if completed(row) { 100 } else { row.prototype_progress.clamp(-100, 100) }
}

pub struct Read<'a> {
    pub row: &'a Row,
    pub read: i32,
    pub history: Option<(i32, &'a [Entry])>,
    pub program: Program,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub first: i32,
    pub last: i32,
    pub read: i32,
    pub look: u32,
    pub progress: i32,
    pub state: ProjectState,
    pub iterations: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    pub entry: Entry,
    pub read: i32,
    pub counted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attempt {
    pub read: i32,
    pub since: Option<i32>,
    pub entry: Entry,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub country: (u32, u32),
    pub owner: u32,
    pub local: u32,
    pub first: (i32, ProjectState),
    pub runs: Vec<Run>,
    pub entries: Vec<Kept>,
    pub attempts: Vec<Attempt>,
    history: Option<(i32, Vec<Entry>)>,
    most: usize,
    scientist: Option<(u32, i32)>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub campaign_start: i32,
    pub projects: Vec<Project>,
    index: BTreeMap<(u32, u32, u32, u32), usize>,
    pub looks: Vec<Look>,
    look_ids: HashMap<Look, u32>,
    seen: BTreeSet<(u32, i32, u32)>,
    pub warnings: BTreeMap<String, usize>,
    pub observed: Vec<(i32, i32)>,
    pub partial: Vec<(i32, i32)>,
}

pub fn extend(spans: &mut Vec<(i32, i32)>, first: i32, last: i32) {
    match spans.last_mut() {
        Some(span) if span.1.checked_add(1) == Some(first) => span.1 = last,
        _ => spans.push((first, last)),
    }
}

fn choices_after_last_marker(entries: &[Entry]) -> i32 {
    let start = entries.iter().rposition(Entry::completion).map_or(0, |at| at + 1);
    entries[start..].iter().filter(|entry| !entry.completion()).count() as i32
}

impl Project {
    fn history(&mut self, read: i32, entries: &[Entry], counted: impl Fn(&Entry) -> bool) {
        if self.history.as_ref().is_some_and(|(held, _)| *held == read) {
            return;
        }
        for entry in entries {
            if !self.entries.iter().any(|kept| kept.entry == *entry) {
                self.entries.push(Kept { entry: *entry, read, counted: entry.completion() && counted(entry) });
            }
        }
        let mut proven = Vec::new();
        for (at, marker) in entries.iter().enumerate().filter(|(_, entry)| entry.completion()) {
            if let Some(next) = entries.get(at + 1).filter(|next| next.ordinal > marker.ordinal) {
                proven.push(Attempt { read, since: None, entry: *next });
            }
        }
        if let Some((earlier, held)) = &self.history
            && let Some(changed) = entries.iter().find(|entry| held.iter().any(|old| old.ordinal == entry.ordinal && old != *entry))
        {
            proven.push(Attempt { read, since: Some(*earlier), entry: *changed });
            self.most = 0;
        }
        for attempt in proven {
            if !self.attempts.iter().any(|held| held.entry == attempt.entry) {
                self.attempts.push(attempt);
            }
        }
        self.most = self.most.max(entries.len());
        self.history = Some((read, entries.to_vec()));
    }

    fn iterations(&self) -> Option<i32> {
        let (_, entries) = self.history.as_ref()?;
        (entries.len() >= self.most).then(|| choices_after_last_marker(entries))
    }

    fn person(&mut self, program: &Program) -> (Person, Option<i32>, Option<(u32, i32)>) {
        match program {
            Program::Unassigned => (Person::Unassigned, None, self.scientist),
            Program::Unknown(why) => (Person::Unknown(why.clone()), None, self.scientist),
            Program::Read { read, scientist: 0 } => (Person::Unassigned, Some(*read), self.scientist),
            Program::Read { read, scientist } if *scientist as u32 == super::CHARACTER_REFERENCE => {
                let id = (*scientist >> 32) as u32;
                self.scientist = Some((id, *read));
                (Person::Character(id), Some(*read), None)
            }
            Program::Read { .. } => (Person::Unknown("The scientist reference has an unknown native type.".into()), None, self.scientist),
            Program::Unread(id) => (Person::Unknown(format!("Program {id} was not read in the day up to this hour.")), None, self.scientist),
        }
    }
}

impl Facts {
    pub fn new(campaign_start: i32) -> Self {
        Self { campaign_start, ..Self::default() }
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

    pub fn hour(&mut self, country: (u32, u32), custody: i32, first: i32, last: i32, reads: &[Read<'_>], published: bool) {
        let campaign_start = self.campaign_start;
        for read in reads {
            let row = read.row;
            if !visible(row) {
                continue;
            }
            let state = state(row);
            let key = (country.0, country.1, row.owner, row.local);
            let at = *self.index.entry(key).or_insert_with(|| {
                self.projects.push(Project {
                    country,
                    owner: row.owner,
                    local: row.local,
                    first: (read.read, state),
                    runs: Vec::new(),
                    entries: Vec::new(),
                    attempts: Vec::new(),
                    history: None,
                    most: 0,
                    scientist: None,
                });
                self.projects.len() - 1
            });
            if state == ProjectState::Unknown && self.seen.insert((row.owner, read.read, row.local)) {
                *self.warnings.entry(format!("Project {} is in a state the replay does not know ({:#x})", row.local, row.offset)).or_default() += 1;
            }
            let project = &mut self.projects[at];
            if let Some((stamp, entries)) = read.history {
                project.history(stamp, entries, |entry| campaign_start <= entry.date && entry.date <= stamp && custody <= entry.date);
            }
            let iterations = read.history.and_then(|_| project.iterations());
            let (person, person_read, remembered) = project.person(&read.program);
            if !published {
                continue;
            }
            let look = self.look(Look { definition: row.definition, person, read: person_read, last: remembered });
            let run = Run { first, last, read: read.read, look, progress: progress(row), state, iterations };
            let runs = &mut self.projects[at].runs;
            match runs.last_mut() {
                Some(held) if held.last.checked_add(1) == Some(first) && (held.look, held.progress, held.state, held.iterations) == (run.look, run.progress, run.state, run.iterations) => held.last = last,
                _ => runs.push(run),
            }
        }
        if published {
            extend(&mut self.observed, first, last);
        }
    }
}
