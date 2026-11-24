use super::{Line, read};
use replay_columnar::{envelope::UNIT_ORDINALS, manifest::Status, store::TableView, summary::{Computed, Source}};
use replay_recording::{cells::TableReader, columnar_frames::TableCoverage};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fmt, io,
    sync::{Arc, Mutex, PoisonError},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proven {
    pub members: Arc<[String]>,
    pub from: i32,
    pub until: i32,
}

struct Segment {
    first: i32,
    last: i32,
    members: Option<Arc<[String]>>,
}

struct Walk {
    first: i32,
    last: i32,
    segments: Vec<Segment>,
}

type ProofKey = (&'static str, &'static str, usize, bool);

const WALKS_HELD: usize = 8;
const WALK_UNITS: u32 = 2;

#[derive(Default)]
pub struct Proofs {
    walks: Mutex<HashMap<ProofKey, Vec<Arc<Walk>>>>,
}

impl Clone for Proofs {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl fmt::Debug for Proofs {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("Proofs")
    }
}

impl Proofs {
    fn held(&self, key: ProofKey, hour: i32) -> Option<Arc<Walk>> {
        let walks = self.walks.lock().unwrap_or_else(PoisonError::into_inner);
        walks.get(&key)?.iter().find(|walk| walk.first <= hour && hour <= walk.last).cloned()
    }

    fn keep(&self, key: ProofKey, walk: Arc<Walk>) {
        let mut walks = self.walks.lock().unwrap_or_else(PoisonError::into_inner);
        let held = walks.entry(key).or_default();
        held.retain(|other| other.last < walk.first || walk.last < other.first);
        if held.len() >= WALKS_HELD {
            held.remove(0);
        }
        held.push(walk);
    }
}

#[derive(Clone, Copy)]
enum Entry<'h> {
    Contender(i64, &'h str),
    Unknown,
    Unrecorded,
}

type ByKey<'h> = HashMap<u128, Vec<(i32, i32, &'h str)>>;

struct Board<'h> {
    source: &'h Source,
    holders: &'h ByKey<'h>,
    absent_none: bool,
    least: bool,
    rows: HashSet<u128>,
    entries: HashMap<u128, Entry<'h>>,
    order: BTreeSet<(i64, &'h str)>,
    unknown: usize,
    unrecorded: usize,
    status: Status,
}

impl<'h> Board<'h> {
    fn tag_of(&self, key: u128, hour: i32) -> Option<&'h str> {
        self.holders.get(&key)?.iter().find(|run| run.0 <= hour && hour <= run.1).map(|run| run.2)
    }

    fn entry_of(&self, view: &TableView<'_>, key: u128, slot: Option<u32>, hour: i32) -> Option<Entry<'h>> {
        let tag = self.tag_of(key, hour);
        match slot {
            Some(slot) => match tag {
                None => Some(Entry::Unknown),
                Some(tag) => match self.source.read(view, slot) {
                    Computed::Value(value) => Some(Entry::Contender(value, tag)),
                    Computed::NotApplicable(_) => None,
                    Computed::Unrecorded => Some(Entry::Unrecorded),
                },
            },
            None => tag.filter(|_| self.absent_none).map(|tag| Entry::Contender(0, tag)),
        }
    }

    fn ordered(&self, value: i64) -> i64 {
        if self.least { value } else { !value }
    }

    fn set(&mut self, key: u128, now: Option<Entry<'h>>) {
        match self.entries.remove(&key) {
            Some(Entry::Contender(value, tag)) => {
                self.order.remove(&(self.ordered(value), tag));
            }
            Some(Entry::Unknown) => self.unknown -= 1,
            Some(Entry::Unrecorded) => self.unrecorded -= 1,
            None => {}
        }
        let Some(entry) = now else { return };
        match entry {
            Entry::Contender(value, tag) => {
                self.order.insert((self.ordered(value), tag));
            }
            Entry::Unknown => self.unknown += 1,
            Entry::Unrecorded => self.unrecorded += 1,
        }
        self.entries.insert(key, entry);
    }

    fn refill(&mut self, reader: &TableReader, hour: i32) {
        self.rows.clear();
        self.entries.clear();
        self.order.clear();
        (self.unknown, self.unrecorded, self.status) = (0, 0, reader.status());
        let view = reader.view();
        for slot in view.rows() {
            let Some(key) = view.key(slot) else { continue };
            self.rows.insert(key);
            let entry = self.entry_of(&view, key, Some(slot), hour);
            self.set(key, entry);
        }
        if self.absent_none {
            for &key in self.holders.keys() {
                if !self.rows.contains(&key) {
                    let entry = self.entry_of(&view, key, None, hour);
                    self.set(key, entry);
                }
            }
        }
    }

    fn apply(&mut self, reader: &TableReader, hour: i32) {
        let view = reader.view();
        let changes = reader.changes();
        for &(_, key) in &changes.deleted {
            self.rows.remove(&key);
            let entry = self.entry_of(&view, key, None, hour);
            self.set(key, entry);
        }
        for &slot in changes.inserted.iter().chain(changes.written.iter().flatten()) {
            let Some(key) = view.key(slot) else { continue };
            self.rows.insert(key);
            let entry = self.entry_of(&view, key, Some(slot), hour);
            self.set(key, entry);
        }
    }

    fn refused(&self) -> bool {
        self.status != Status::Complete || self.unknown > 0 || self.unrecorded > 0
    }

    fn members(&self, n: usize) -> Option<Vec<&'h str>> {
        (!self.refused()).then(|| self.order.iter().take(n).map(|&(_, tag)| tag).collect())
    }
}

fn same(held: &Option<Arc<[String]>>, now: &Option<Vec<&str>>) -> bool {
    match (held, now) {
        (None, None) => true,
        (Some(held), Some(now)) => held.len() == now.len() && now.iter().all(|tag| held.iter().any(|other| other == tag)),
        _ => false,
    }
}

impl<'q, 'a> Line<'q, 'a> {
    pub fn top_at(&self, hour: i32, n: usize, least: bool) -> io::Result<Result<Proven, String>> {
        self.owners_only("a rank at an hour")?;
        let key: ProofKey = (self.series.kind, self.measure.name, n, least);
        let proofs = &self.query.holders.ranks;
        let walk = match proofs.held(key, hour) {
            Some(walk) => Some(walk),
            None => match self.walk(hour, n, least)? {
                Some(walk) => {
                    let walk = Arc::new(walk);
                    proofs.keep(key, Arc::clone(&walk));
                    Some(walk)
                }
                None => None,
            },
        };
        let segment = walk.as_deref().and_then(|walk| walk.segments.get(walk.segments.partition_point(|segment| segment.last < hour)));
        match segment.and_then(|segment| Some((segment, segment.members.as_ref()?))) {
            Some((segment, members)) => Ok(Ok(Proven { members: Arc::clone(members), from: segment.first, until: segment.last })),
            None => Ok(self.rank_at(hour, n, least)?.map(|tags| Proven { members: tags.into(), from: hour, until: hour })),
        }
    }

    fn walk(&self, hour: i32, n: usize, least: bool) -> io::Result<Option<Walk>> {
        let Some(stretch) = self.query.timeline.at(hour).copied() else { return Ok(None) };
        let recording = self.query.replay.recording();
        let run = recording.run(stretch.run)?;
        let valid = match recording.table_coverage(self.series.table.index(), stretch.run) {
            TableCoverage::Valid => i32::MIN,
            TableCoverage::WithheldAll(_) => return Ok(None),
            TableCoverage::WithheldBefore { ordinal, .. } => run.first_hour + ordinal as i32,
        };
        let Some(ordinal) = run.ordinal(hour).filter(|_| hour >= valid) else { return Ok(None) };
        let unit_first = run.first_hour + (ordinal / UNIT_ORDINALS * UNIT_ORDINALS) as i32;
        let first = unit_first.max(stretch.first).max(valid);
        let last = (i64::from(unit_first) + i64::from(WALK_UNITS * UNIT_ORDINALS) - 1).min(i64::from(stretch.last)).min(i64::from(run.last_hour())) as i32;
        crate::cost::storage(|| {
            let columns = read::columns_of(&self.source);
            let mut reader = TableReader::new(recording, stretch.run, self.series.table, Some(&columns))?;
            reader.seek(recording, run.ordinal(first).expect("the walk starts inside its run"))?;
            let holders = self.by_key();
            let mut board = Board {
                source: &self.source,
                holders: &holders,
                absent_none: self.series.absent_none,
                least,
                rows: HashSet::new(),
                entries: HashMap::new(),
                order: BTreeSet::new(),
                unknown: 0,
                unrecorded: 0,
                status: Status::Complete,
            };
            board.refill(&reader, first);
            let owned = |tags: Vec<&str>| -> Arc<[String]> { tags.into_iter().map(str::to_owned).collect() };
            let mut segments = vec![Segment { first, last: first, members: board.members(n).map(owned) }];
            let mut edge = self.query.holders.next_change(first);
            let mut walked = first;
            while walked < last && reader.step(recording)? {
                let at = walked + 1;
                let crossed = edge.is_some_and(|edge| edge <= at);
                if crossed {
                    edge = self.query.holders.next_change(at);
                }
                let changes = reader.changes();
                let moved = crossed || changes.anew || reader.status() != board.status;
                if moved {
                    board.refill(&reader, at);
                } else if !changes.deleted.is_empty() || !changes.inserted.is_empty() || changes.written.iter().any(|slots| !slots.is_empty()) {
                    board.apply(&reader, at);
                } else {
                    walked = at;
                    segments.last_mut().expect("a walk holds a segment").last = at;
                    continue;
                }
                let now = board.members(n);
                let held = segments.last_mut().expect("a walk holds a segment");
                if same(&held.members, &now) {
                    held.last = at;
                } else {
                    segments.push(Segment { first: at, last: at, members: now.map(owned) });
                }
                walked = at;
            }
            Ok(Some(Walk { first, last: walked, segments }))
        })
    }
}

