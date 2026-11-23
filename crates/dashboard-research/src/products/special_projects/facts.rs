
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

impl Program {
    pub fn scientist(&self) -> Result<Option<u32>, String> {
        match self {
            Self::Unassigned | Self::Read { scientist: 0, .. } => Ok(None),
            Self::Read { scientist, .. } if *scientist as u32 == super::CHARACTER_REFERENCE => Ok(Some((*scientist >> 32) as u32)),
            Self::Read { .. } => Err("The scientist reference has an unknown native type.".into()),
            Self::Unknown(why) => Err(why.clone()),
            Self::Unread(id) => Err(format!("Program {id} was not read in the day up to this hour.")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Look {
    pub definition: u32,
    pub person: Person,
    pub last: Option<u32>,
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
    pub program: Program,
}

pub struct Hour<'a> {
    pub country: (u32, u32),
    pub at: i32,
    pub held: &'a dyn Fn(u32, i32, i32) -> bool,
    pub projects: &'a BTreeMap<u32, i32>,
    pub histories: &'a BTreeMap<u32, i32>,
    pub listed: &'a BTreeMap<(u32, u32), Vec<Entry>>,
    pub reads: &'a [Read<'a>],
    pub published: bool,
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
    pub scientist_read: Option<i32>,
    pub last_scientist_read: Option<i32>,
}

impl Run {
    fn same(&self, other: &Self) -> bool {
        (self.look, self.progress, self.state, self.iterations, self.scientist_read, self.last_scientist_read)
            == (other.look, other.progress, other.state, other.iterations, other.scientist_read, other.last_scientist_read)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    pub entry: Entry,
    pub read: i32,
    pub counted: bool,
    pub definition: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attempt {
    pub read: i32,
    pub since: Option<i32>,
    pub entry: Entry,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Evidence {
    pub entries: Vec<Kept>,
    pub attempts: Vec<Attempt>,
    latest: Option<(i32, Vec<Entry>)>,
    ordinals: BTreeMap<u32, (Entry, i32)>,
    most: usize,
}

fn choices_after_last_marker(entries: &[Entry]) -> i32 {
    let start = entries.iter().rposition(Entry::completion).map_or(0, |at| at + 1);
    entries[start..].iter().filter(|entry| !entry.completion()).count() as i32
}

impl Evidence {
    pub fn read(&mut self, stamp: i32, entries: &[Entry], counted: impl Fn(&Entry) -> bool, continued: impl Fn(i32) -> bool, named: Option<(u32, i32)>) -> bool {
        if let Some((held, _)) = &self.latest {
            if *held == stamp {
                return false;
            }
            if !continued(*held) {
                self.ordinals.clear();
                self.most = 0;
            }
        }
        for (at, entry) in entries.iter().enumerate() {
            if !self.entries.iter().any(|kept| kept.entry == *entry) {
                let counted = entry.completion() && counted(entry);
                let definition = named.filter(|&(_, read)| counted && at + 1 == entries.len() && read >= entry.date).map(|(definition, _)| definition);
                self.entries.push(Kept { entry: *entry, read: stamp, counted, definition });
            }
        }
        let mut proven = Vec::new();
        for (at, marker) in entries.iter().enumerate().filter(|(_, entry)| entry.completion()) {
            if let Some(next) = entries.get(at + 1).filter(|next| next.ordinal > marker.ordinal) {
                proven.push(Attempt { read: stamp, since: None, entry: *next });
            }
        }
        let changed = entries.iter().find_map(|entry| self.ordinals.get(&entry.ordinal).filter(|(old, _)| old != entry).map(|old| (entry, *old)));
        if let Some((entry, (_, since))) = changed {
            proven.push(Attempt { read: stamp, since: Some(since), entry: *entry });
            self.ordinals.clear();
            self.most = 0;
        }
        for attempt in proven {
            if !self.attempts.iter().any(|held| held.entry == attempt.entry) {
                self.attempts.push(attempt);
            }
        }
        self.ordinals.extend(entries.iter().map(|entry| (entry.ordinal, (*entry, stamp))));
        self.most = self.most.max(entries.len());
        self.latest = Some((stamp, entries.to_vec()));
        changed.is_some()
    }

    fn iterations(&self) -> Option<i32> {
        let (_, entries) = self.latest.as_ref()?;
        (entries.len() >= self.most).then(|| choices_after_last_marker(entries))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub country: (u32, u32),
    pub owner: u32,
    pub local: u32,
    pub first: (i32, ProjectState),
    pub runs: Vec<Run>,
    scientist: Option<(u32, i32)>,
}

impl Project {
    fn person(&mut self, program: &Program) -> (Person, Option<i32>, Option<(u32, i32)>) {
        let read = match program {
            Program::Read { read, .. } => Some(*read),
            _ => None,
        };
        match program.scientist() {
            Ok(Some(id)) => {
                self.scientist = Some((id, read.expect("a scientist is only named by a read")));
                (Person::Character(id), read, None)
            }
            Ok(None) => (Person::Unassigned, read, self.scientist),
            Err(why) => (Person::Unknown(why), None, self.scientist),
        }
    }
}

type Key = (u32, u32, u32, u32);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub campaign_start: i32,
    pub projects: Vec<Project>,
    index: BTreeMap<Key, usize>,
    pub evidence: BTreeMap<Key, Evidence>,
    pub looks: Vec<Look>,
    look_ids: HashMap<Look, u32>,
    seen: BTreeSet<(u32, i32, u32)>,
    pub warnings: BTreeMap<String, usize>,
    pub observed: Vec<(i32, i32)>,
    pub partial: Vec<(i32, i32)>,
    pub reads: BTreeMap<u32, Vec<(i32, i32, i32)>>,
}

pub fn extend(spans: &mut Vec<(i32, i32)>, first: i32, last: i32) {
    match spans.last_mut() {
        Some(span) if span.1.checked_add(1) == Some(first) => span.1 = last,
        _ => spans.push((first, last)),
    }
}

impl Facts {
    pub fn new(campaign_start: i32) -> Self {
        Self { campaign_start, ..Self::default() }
    }

    pub fn evidence_of(&self, project: &Project) -> Option<&Evidence> {
        self.evidence.get(&(project.country.0, project.country.1, project.owner, project.local))
    }

    pub fn unlisted(&self) -> impl Iterator<Item = (&Key, &Evidence)> {
        self.evidence.iter().filter(|(key, evidence)| !self.index.contains_key(key) && !evidence.entries.is_empty())
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

    fn histories(&mut self, hour: &Hour<'_>) {
        let (country, start) = (hour.country, self.campaign_start);
        let mut keys: BTreeSet<(u32, u32)> = hour.listed.keys().copied().collect();
        keys.extend(self.evidence.range((country.0, country.1, 0, 0)..=(country.0, country.1, u32::MAX, u32::MAX)).map(|(key, _)| (key.2, key.3)));
        keys.extend(hour.reads.iter().map(|read| (read.row.owner, read.row.local)));
        for (owner, local) in keys {
            let Some(&stamp) = hour.histories.get(&owner) else { continue };
            let entries = hour.listed.get(&(owner, local)).map_or(&[][..], Vec::as_slice);
            let held = hour.held;
            let named = hour.reads.iter().find(|read| (read.row.owner, read.row.local) == (owner, local) && read.read <= stamp && completed(read.row)).map(|read| (read.row.definition, read.read));
            self.evidence.entry((country.0, country.1, owner, local)).or_default().read(
                stamp,
                entries,
                |entry| start <= entry.date && entry.date <= stamp && held(owner, entry.date, stamp),
                |earlier| held(owner, earlier, stamp),
                named,
            );
        }
    }

    pub fn hour(&mut self, hour: &Hour<'_>) {
        let (country, at) = (hour.country, hour.at);
        self.histories(hour);
        for read in hour.reads {
            let row = read.row;
            if !visible(row) {
                continue;
            }
            let state = state(row);
            let key = (country.0, country.1, row.owner, row.local);
            let index = *self.index.entry(key).or_insert_with(|| {
                self.projects.push(Project { country, owner: row.owner, local: row.local, first: (read.read, state), runs: Vec::new(), scientist: None });
                self.projects.len() - 1
            });
            if state == ProjectState::Unknown && self.seen.insert((row.owner, read.read, row.local)) {
                *self.warnings.entry(format!("Project {} is in a state the replay does not know ({:#x})", row.local, row.offset)).or_default() += 1;
            }
            let iterations = if hour.histories.contains_key(&row.owner) { self.evidence.get(&key).and_then(Evidence::iterations) } else { None };
            let (person, scientist_read, remembered) = self.projects[index].person(&read.program);
            if !hour.published {
                continue;
            }
            let look = self.look(Look { definition: row.definition, person, last: remembered.map(|(id, _)| id) });
            let run = Run {
                first: at,
                last: at,
                read: read.read,
                look,
                progress: progress(row),
                state,
                iterations,
                scientist_read,
                last_scientist_read: remembered.map(|(_, read)| read),
            };
            let runs = &mut self.projects[index].runs;
            match runs.last_mut() {
                Some(held) if held.last.checked_add(1) == Some(at) && held.same(&run) => held.last = at,
                _ => runs.push(run),
            }
        }
        if hour.published {
            extend(&mut self.observed, at, at);
            for (&owner, &read) in hour.projects {
                let spans = self.reads.entry(owner).or_default();
                match spans.last_mut() {
                    Some(span) if span.1.checked_add(1) == Some(at) && span.2 == read => span.1 = at,
                    _ => spans.push((at, at, read)),
                }
            }
        }
    }
}
