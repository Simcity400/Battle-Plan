
#[cfg(test)]
mod tests;

use recorder_derive_framework::framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns};
use replay_columnar::{
    leaf::{Reader, Writer as Bytes},
    store::TableView,
};
use replay_data::{
    cadence::STAGGER_PERIOD,
    rows::{CountryLinkRow, CountryRow, ResearchAssignmentRow, ResearchCoverageRow, ResearchInputRow, ResearchTechnologyRow, SliceReadRow, TableId},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

const CLASSIFIER_COMPLETED: u32 = 4;

const COUNTRY: [usize; 2] = columns::<CountryRow, 2>(["canonical_slot", "tag"]);
const LINK: [usize; 1] = columns::<CountryLinkRow, 1>(["canonical_slot"]);
const INPUT: [usize; 6] = columns::<ResearchInputRow, 6>(["technology_handle", "technology_present", "token", "classifier", "level", "database_index"]);
const TECHNOLOGY: [usize; 3] = columns::<ResearchTechnologyRow, 3>(["classifier", "level", "researched"]);
const SAMPLE: [usize; 1] = columns::<SliceReadRow, 1>(["sample_hour"]);
const ASSIGNMENT: [usize; 12] = columns::<ResearchAssignmentRow, 12>([
    "technology_handle",
    "database_index",
    "token",
    "level",
    "continuity_start_hour",
    "first_observed_hour",
    "started_after_hour",
    "last_observed_hour",
    "ended_by_hour",
    "outcome",
    "outcome_observed_hour",
    "awaiting",
]);
const SINCE: [usize; 1] = columns::<ResearchCoverageRow, 1>(["since"]);

const fn read(table: TableId, columns: &'static [usize]) -> Read {
    Read { table, columns: Columns::These(columns), watch: Watch::Changed, required: false }
}

const READS: &[Read] = &[
    read(TableId::ResearchInputs, &INPUT),
    read(TableId::ResearchTechnologies, &TECHNOLOGY),
    read(TableId::SliceReads, &SAMPLE),
    read(TableId::Countries, &COUNTRY),
    read(TableId::CountryLinks, &LINK),
];

const INPUTS: u32 = TableId::ResearchInputs.index() as u32;

type HolderKey = (u32, u32);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Assignment {
    handle: u32,
    occurrence: i32,
    token: u32,
    level: i32,
    continuity_start: i32,
    first_observed: i32,
    started_after: Option<i32>,
    last_observed: Option<i32>,
    ended_by: Option<i32>,
    outcome: u8,
    outcome_observed: Option<i32>,
    awaiting: bool,
}

impl Assignment {
    fn cells(&self) -> Vec<Value> {
        vec![
            Value::U32(self.handle),
            Value::I32(self.occurrence),
            Value::Bytes(self.token),
            Value::I32(self.level),
            Value::I32(self.continuity_start),
            Value::I32(self.first_observed),
            Value::OptionalI32(self.started_after),
            Value::OptionalI32(self.last_observed),
            Value::OptionalI32(self.ended_by),
            Value::U8(self.outcome),
            Value::OptionalI32(self.outcome_observed),
            Value::U8(u8::from(self.awaiting)),
        ]
    }

    fn active(&self) -> bool {
        self.outcome == ResearchAssignmentRow::ACTIVE
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Open {
    lane: u16,
    cells: Assignment,
    dirty: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Holder {
    tag: Box<[u8]>,
    last_read: Option<i32>,
    observed: Option<i32>,
    since: Option<i32>,
    reached: Option<i32>,
    continuity_start: i32,
    available: BTreeSet<u32>,
    slots: BTreeMap<u32, Vec<Open>>,
}

impl Holder {
    fn until(&self) -> Option<i32> {
        self.since?;
        self.last_read?.checked_add(STAGGER_PERIOD - 1)
    }
}

#[derive(Clone, Copy, Debug)]
struct Input {
    country: u32,
    index: u32,
    handle: u32,
    present: u8,
    token: Option<u32>,
    classifier: u32,
    level: i32,
    occurrence: Option<i32>,
}

impl Input {
    fn holds(&self) -> Option<i32> {
        self.occurrence.filter(|_| self.present != 0)
    }
}

#[derive(Clone, Copy, Debug)]
struct Technology {
    classifier: u32,
    level: i32,
    researched: bool,
}

#[derive(Default)]
struct Indexes {
    links: HashMap<u32, u32>,
    owners: HashMap<u32, Vec<u32>>,
    holders: BTreeMap<HolderKey, Box<[u8]>>,
}

#[derive(Default)]
pub struct Research {
    holders: BTreeMap<HolderKey, Holder>,
    indexes: Option<Indexes>,
    rows: Option<HashMap<u32, u32>>,
    last_bad: Option<i32>,
    recheck: BTreeSet<HolderKey>,
    closing: Vec<u128>,
    ended: Vec<(HolderKey, u32, Open)>,
    last: Option<i32>,
}

fn assignment_key(holder: HolderKey, index: u32, lane: u16) -> u128 {
    u128::from(holder.0) << 80 | u128::from(holder.1) << 48 | u128::from(index) << 16 | u128::from(lane)
}

fn coverage_key(holder: HolderKey, member: u32) -> u128 {
    u128::from(holder.0) << 64 | u128::from(holder.1) << 32 | u128::from(member)
}

fn optional(table: &TableView<'_>, column: usize, slot: u32) -> Option<i32> {
    table.number(column, slot).map(|value| value as i32)
}

impl Indexes {
    fn read(hour: &Hour<'_>) -> io::Result<Self> {
        let mut indexes = Self::default();
        let links = hour.table(TableId::CountryLinks);
        for slot in links.rows() {
            let (Some(id), Some(canonical)) = (links.key(slot), links.number(LINK[0], slot)) else { continue };
            indexes.links.insert(id as u32, canonical as u32);
            indexes.owners.entry(canonical as u32).or_default().push(id as u32);
        }
        for owners in indexes.owners.values_mut() {
            owners.sort_unstable();
        }
        let countries = hour.table(TableId::Countries);
        for slot in countries.rows() {
            let (Some(id), Some(canonical)) = (countries.key(slot), countries.number(COUNTRY[0], slot)) else { continue };
            let (id, canonical) = (id as u32, canonical as u32);
            if indexes.links.get(&id) != Some(&canonical) {
                continue;
            }
            let tag = countries.text(COUNTRY[1], slot).and_then(|text| hour.text(text)).ok_or_else(|| invalid("A country names a tag its run never added"))?;
            indexes.holders.insert((canonical, id), tag.into());
        }
        Ok(indexes)
    }
}

fn row_counts(hour: &Hour<'_>) -> HashMap<u32, u32> {
    let table = hour.table(TableId::ResearchInputs);
    let mut counts: HashMap<u32, u32> = HashMap::new();
    for slot in table.rows() {
        if let Some(key) = table.key(slot) {
            *counts.entry((key >> 32) as u32).or_default() += 1;
        }
    }
    counts
}

fn sample(hour: &Hour<'_>, owner: u32) -> Option<i32> {
    let table = hour.table(TableId::SliceReads);
    let slot = table.slot(u128::from(INPUTS) << 32 | u128::from(owner))?;
    optional(&table, SAMPLE[0], slot)
}

fn valid(hour: &Hour<'_>, inputs: &[Input]) -> bool {
    inputs.iter().enumerate().all(|(at, input)| {
        input.index as usize == at
            && input.present <= 1
            && (input.present == 0
                || (input.occurrence.is_none_or(|occurrence| occurrence > 0)
                    && input.token.and_then(|token| hour.text(token)).is_some_and(|token| !token.is_empty() && token.len() <= 512 && std::str::from_utf8(token).is_ok())))
    })
}

impl Research {
    fn inputs(&self, hour: &Hour<'_>, owners: &[u32]) -> Option<Vec<Input>> {
        let table = hour.table(TableId::ResearchInputs);
        let counts = self.rows.as_ref()?;
        let mut inputs = Vec::new();
        for &owner in owners {
            for index in 0..counts.get(&owner).copied().unwrap_or(0) {
                let slot = table.slot(u128::from(owner) << 32 | u128::from(index))?;
                let [handle, present, _, classifier, level, occurrence] = INPUT.map(|column| table.number(column, slot));
                inputs.push(Input {
                    country: owner,
                    index,
                    handle: handle? as u32,
                    present: present? as u8,
                    token: table.text(INPUT[2], slot),
                    classifier: classifier? as u32,
                    level: level? as i32,
                    occurrence: occurrence.map(|occurrence| occurrence as i32),
                });
            }
        }
        Some(inputs)
    }

    fn uncover(key: HolderKey, holder: &mut Holder, at: i32, out: &mut Rows) {
        let Some(until) = holder.until() else { return };
        out.delete(TableId::ResearchCoverage, coverage_key(key, ResearchCoverageRow::HOLDER));
        for index in std::mem::take(&mut holder.available) {
            out.delete(TableId::ResearchCoverage, coverage_key(key, index));
        }
        holder.since = None;
        holder.reached = Some(until.min(at - 1));
    }

    fn stop_waiting(&mut self, key: HolderKey, index: u32, mut open: Open, out: &mut Rows) {
        open.cells.awaiting = false;
        let row = assignment_key(key, index, open.lane);
        out.set(TableId::ResearchAssignments, row, open.cells.cells());
        if open.lane != 0 {
            self.closing.push(row);
        }
    }

    fn discontinue(&mut self, key: HolderKey, holder: &mut Holder, at: i32) {
        let observed = holder.observed;
        for (index, opens) in std::mem::take(&mut holder.slots) {
            for mut open in opens {
                if open.cells.active() {
                    open.cells.outcome = ResearchAssignmentRow::UNKNOWN;
                    open.cells.ended_by = Some(at);
                    open.cells.last_observed = observed;
                }
                self.ended.push((key, index, open));
            }
        }
    }

    fn write_ended(&mut self, out: &mut Rows<'_>) {
        let mut taken: BTreeSet<(HolderKey, u32, u16)> = BTreeSet::new();
        for (key, index, open) in &self.ended {
            taken.insert((*key, *index, open.lane));
            for open in self.holders.get(key).and_then(|holder| holder.slots.get(index)).into_iter().flatten() {
                taken.insert((*key, *index, open.lane));
            }
        }
        for (key, index, mut open) in std::mem::take(&mut self.ended) {
            let started = self.holders.get(&key).and_then(|holder| holder.slots.get(&index)).is_some_and(|opens| opens.iter().any(|held| held.lane == open.lane));
            if started {
                let mut lane = 1;
                while taken.contains(&(key, index, lane)) {
                    lane += 1;
                }
                taken.insert((key, index, lane));
                open.lane = lane;
            }
            self.stop_waiting(key, index, open, out);
        }
    }

    fn reindex(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let indexes = Indexes::read(hour)?;
        let gone: Vec<HolderKey> = self.holders.iter().filter(|(key, holder)| indexes.holders.get(key) != Some(&holder.tag)).map(|(&key, _)| key).collect();
        for key in gone {
            let mut holder = self.holders.remove(&key).expect("held");
            Self::uncover(key, &mut holder, hour.hour, out);
            self.discontinue(key, &mut holder, hour.hour);
        }
        self.indexes = Some(indexes);
        Ok(())
    }

    fn touched(&self, hour: &Hour<'_>) -> BTreeSet<HolderKey> {
        let Some(indexes) = &self.indexes else { return BTreeSet::new() };
        let table = hour.table(TableId::SliceReads);
        let mut owners: Vec<u32> = hour.written_rows(TableId::SliceReads, Columns::These(&SAMPLE)).into_iter().filter_map(|slot| table.key(slot)).chain(hour.deleted_keys(TableId::SliceReads)).filter(|key| (key >> 32) as u32 == INPUTS).map(|key| key as u32).collect();
        owners.sort_unstable();
        owners.dedup();
        let mut touched = BTreeSet::new();
        for owner in owners {
            let Some(&slot) = indexes.links.get(&owner) else { continue };
            touched.extend(indexes.holders.range((slot, 0)..=(slot, u32::MAX)).map(|(&key, _)| key));
        }
        touched
    }

    fn technology(hour: &Hour<'_>, inputs: &[Input], owner: Option<u32>, occurrence: i32) -> Option<Technology> {
        if let Some(input) = inputs.iter().rev().find(|input| input.holds() == Some(occurrence)) {
            return Some(Technology { classifier: input.classifier, level: input.level, researched: false });
        }
        let owner = owner.filter(|_| hour.complete(TableId::ResearchTechnologies))?;
        let table = hour.table(TableId::ResearchTechnologies);
        let slot = table.slot(u128::from(owner) << 32 | u128::from(occurrence as u32))?;
        let [classifier, level, researched] = TECHNOLOGY.map(|column| table.number(column, slot));
        Some(Technology { classifier: classifier? as u32, level: level? as i32, researched: researched? != 0 })
    }

    fn observe(&mut self, hour: &Hour<'_>, key: HolderKey, holder: &mut Holder, inputs: &[Input], inventory: Option<u32>, continuous: bool, out: &mut Rows) {
        let at = hour.hour;
        let previous = holder.observed.replace(at);
        let technology = |occurrence: i32| Self::technology(hour, inputs, inventory, occurrence);
        let mut finished: Vec<(u32, Open)> = Vec::new();
        if !continuous {
            holder.continuity_start = at;
            for (index, opens) in std::mem::take(&mut holder.slots) {
                for mut open in opens {
                    if open.cells.active() {
                        open.cells.outcome = ResearchAssignmentRow::UNKNOWN;
                        open.cells.ended_by = Some(at);
                        open.cells.last_observed = previous;
                    }
                    open.cells.awaiting = false;
                    finished.push((index, open));
                }
            }
        }
        for (&index, opens) in holder.slots.iter_mut() {
            let mut kept = Vec::with_capacity(opens.len());
            for mut open in std::mem::take(opens) {
                if open.cells.awaiting {
                    let (occurrence, level) = (open.cells.occurrence, open.cells.level);
                    let ended = open.cells.ended_by.unwrap_or(at);
                    let again = inputs.iter().any(|input| input.holds() == Some(occurrence)) && technology(occurrence).is_some_and(|known| known.level == level);
                    if at.saturating_sub(ended) > STAGGER_PERIOD || again {
                        open.cells.awaiting = false;
                        finished.push((index, open));
                    } else if technology(occurrence).is_some_and(|known| known.level > level || (level == 0 && known.researched) || known.classifier == CLASSIFIER_COMPLETED) {
                        open.cells.outcome = ResearchAssignmentRow::COMPLETED;
                        open.cells.outcome_observed = Some(at);
                        open.cells.awaiting = false;
                        finished.push((index, open));
                    } else {
                        kept.push(open);
                    }
                    continue;
                }
                let occurrence = open.cells.occurrence;
                let current = inputs.iter().find(|input| input.index == index);
                let known = technology(occurrence);
                let moved = inputs.iter().any(|input| input.index != index && input.holds() == Some(occurrence));
                let completed = known.is_some_and(|known| known.level > open.cells.level || known.researched || (!moved && known.classifier == CLASSIFIER_COMPLETED));
                if !completed && current.is_some_and(|input| input.holds() == Some(occurrence)) {
                    kept.push(open);
                    continue;
                }
                open.cells.ended_by = Some(at);
                open.cells.last_observed = previous;
                open.cells.outcome = if completed {
                    ResearchAssignmentRow::COMPLETED
                } else if current.is_some() && known.is_some() {
                    ResearchAssignmentRow::SWITCHED
                } else {
                    ResearchAssignmentRow::UNKNOWN
                };
                open.cells.awaiting = !completed && !moved;
                open.dirty = true;
                if open.cells.awaiting {
                    kept.push(open);
                } else {
                    finished.push((index, open));
                }
            }
            *opens = kept;
        }
        let mut started: BTreeMap<u32, Open> = BTreeMap::new();
        for input in inputs {
            let (Some(occurrence), Some(token)) = (input.holds(), input.token) else { continue };
            if input.classifier == CLASSIFIER_COMPLETED || holder.slots.get(&input.index).is_some_and(|opens| opens.iter().any(|open| open.cells.active())) {
                continue;
            }
            let cells = Assignment {
                handle: input.handle,
                occurrence,
                token,
                level: input.level,
                continuity_start: holder.continuity_start,
                first_observed: at,
                started_after: if continuous { previous } else { None },
                last_observed: None,
                ended_by: None,
                outcome: ResearchAssignmentRow::ACTIVE,
                outcome_observed: None,
                awaiting: false,
            };
            started.insert(input.index, Open { lane: 0, cells, dirty: true });
        }
        let mut indexes: BTreeSet<u32> = finished.iter().map(|(index, _)| *index).collect();
        indexes.extend(started.keys().copied());
        indexes.extend(holder.slots.iter().filter(|(_, opens)| opens.iter().any(|open| open.dirty)).map(|(&index, _)| index));
        for index in indexes {
            let opens = holder.slots.entry(index).or_default();
            let (mut done, rest): (Vec<(u32, Open)>, Vec<(u32, Open)>) = std::mem::take(&mut finished).into_iter().partition(|(of, _)| *of == index);
            finished = rest;
            if let Some(new) = started.remove(&index) {
                let mut lanes: BTreeSet<u16> = opens.iter().chain(done.iter().map(|(_, open)| open)).map(|open| open.lane).collect();
                for entry in opens.iter_mut().chain(done.iter_mut().map(|(_, open)| open)).filter(|open| open.lane == 0) {
                    let mut lane = 1;
                    while lanes.contains(&lane) {
                        lane += 1;
                    }
                    lanes.insert(lane);
                    entry.lane = lane;
                    entry.dirty = true;
                }
                opens.push(new);
            }
            for open in opens.iter_mut().filter(|open| open.dirty) {
                out.set(TableId::ResearchAssignments, assignment_key(key, index, open.lane), open.cells.cells());
                open.dirty = false;
            }
            for (_, open) in done {
                let row = assignment_key(key, index, open.lane);
                out.set(TableId::ResearchAssignments, row, open.cells.cells());
                if open.lane != 0 {
                    self.closing.push(row);
                }
            }
            opens.sort_unstable_by_key(|open| open.lane);
        }
        holder.slots.retain(|_, opens| !opens.is_empty());
    }

    fn step(&mut self, hour: &Hour<'_>, key: HolderKey, out: &mut Rows) -> io::Result<()> {
        let at = hour.hour;
        let Some(tag) = self.indexes.as_ref().and_then(|indexes| indexes.holders.get(&key)).cloned() else { return Ok(()) };
        let owners = self.indexes.as_ref().and_then(|indexes| indexes.owners.get(&key.0)).cloned().unwrap_or_default();
        let mut holder = self.holders.remove(&key).unwrap_or_else(|| Holder { tag, ..Holder::default() });
        let fresh = |owner: &u32| sample(hour, *owner) == Some(at);
        if !owners.iter().any(fresh) {
            let mut held = owners.iter().filter_map(|&owner| sample(hour, owner));
            let first = held.next();
            let proven = first.is_some() && first == holder.last_read && held.all(|read| Some(read) == first);
            if !proven {
                Self::uncover(key, &mut holder, at, out);
            }
            self.keep(key, holder);
            return Ok(());
        }
        let inputs = self.inputs(hour, &owners);
        let owner = inputs.as_ref().and_then(|inputs| inputs.first().map(|input| input.country)).or_else(|| owners.iter().copied().find(|owner| fresh(owner)));
        let read = owner.is_some_and(|owner| fresh(&owner)) && inputs.as_ref().is_some_and(|inputs| inputs.iter().all(|input| fresh(&input.country)));
        if holder.last_read.is_some_and(|last| last >= at) {
            self.keep(key, holder);
            return Ok(());
        }
        if !read {
            Self::uncover(key, &mut holder, at, out);
            self.keep(key, holder);
            return Ok(());
        }
        let inputs = inputs.unwrap_or_default();
        let covered = holder.since.is_some() && holder.until().is_some_and(|until| until >= at - 1);
        let read_after = holder.last_read.filter(|&last| self.last_bad.is_none_or(|bad| bad <= last) && at - last <= STAGGER_PERIOD);
        let continuous = holder.observed.is_some() && (covered || holder.reached.is_some_and(|reached| reached + 1 == at || read_after == Some(reached)));
        if !valid(hour, &inputs) {
            Self::uncover(key, &mut holder, at, out);
            holder.last_read = Some(at);
            self.keep(key, holder);
            return Ok(());
        }
        let inventory = owner.filter(|_| hour.complete(TableId::ResearchTechnologies));
        self.observe(hour, key, &mut holder, &inputs, inventory, continuous, out);
        if !covered {
            Self::uncover(key, &mut holder, at, out);
            holder.since = Some(at);
            out.set(TableId::ResearchCoverage, coverage_key(key, ResearchCoverageRow::HOLDER), vec![Value::I32(at)]);
        }
        let slots: BTreeSet<u32> = inputs.iter().map(|input| input.index).collect();
        for &index in holder.available.difference(&slots) {
            out.delete(TableId::ResearchCoverage, coverage_key(key, index));
        }
        for &index in &slots {
            if !covered || !holder.available.contains(&index) {
                out.set(TableId::ResearchCoverage, coverage_key(key, index), vec![Value::I32(at)]);
            }
        }
        holder.available = slots;
        holder.last_read = Some(at);
        holder.reached = None;
        if owners.iter().any(|owner| sample(hour, *owner).is_some_and(|read| read != at)) {
            self.recheck.insert(key);
        }
        self.keep(key, holder);
        Ok(())
    }

    fn keep(&mut self, key: HolderKey, holder: Holder) {
        if holder.observed.is_some() || holder.last_read.is_some() || holder.since.is_some() {
            self.holders.insert(key, holder);
        }
    }
}

impl Writer for Research {
    fn name(&self) -> &'static str {
        "Research"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::ResearchAssignments), Output::State(TableId::ResearchCoverage)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn hourly(&self) -> bool {
        true
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        for key in self.closing.drain(..) {
            out.delete(TableId::ResearchAssignments, key);
        }
        let at = hour.hour;
        let broke = hour.fresh || self.last.is_none_or(|last| last.checked_add(1) != Some(at));
        self.last = Some(at);
        let observable = [TableId::Countries, TableId::CountryLinks, TableId::ResearchInputs, TableId::SliceReads].into_iter().all(|table| hour.complete(table));
        if broke || !observable {
            self.last_bad = Some(if observable { at - 1 } else { at });
            let keys: Vec<HolderKey> = self.holders.keys().copied().collect();
            for key in keys {
                let holder = self.holders.get_mut(&key).expect("held");
                Self::uncover(key, holder, at, out);
            }
            self.recheck.clear();
            self.indexes = None;
            self.rows = None;
            if !observable {
                return Ok(());
            }
        }
        if self.indexes.is_none() || hour.wrote(TableId::Countries, Columns::These(&COUNTRY)) || hour.wrote(TableId::CountryLinks, Columns::These(&LINK)) {
            self.reindex(hour, out)?;
        }
        match self.rows.as_mut() {
            None => self.rows = Some(row_counts(hour)),
            Some(counts) => {
                let changes = hour.changes(TableId::ResearchInputs);
                let table = hour.table(TableId::ResearchInputs);
                for &slot in &changes.inserted {
                    if let Some(key) = table.key(slot) {
                        *counts.entry((key >> 32) as u32).or_default() += 1;
                    }
                }
                for &(_, key) in &changes.deleted {
                    let owner = (key >> 32) as u32;
                    if let Some(count) = counts.get_mut(&owner) {
                        *count = count.saturating_sub(1);
                        if *count == 0 {
                            counts.remove(&owner);
                        }
                    }
                }
            }
        }
        let mut touched = self.touched(hour);
        touched.append(&mut self.recheck);
        if broke {
            let indexes = self.indexes.as_ref().expect("indexed");
            touched.extend(indexes.holders.keys().copied());
        }
        for &key in &touched {
            self.step(hour, key, out)?;
        }
        let due: Vec<HolderKey> = self.holders.iter().filter(|(_, holder)| holder.until().is_some_and(|until| until < at)).map(|(&key, _)| key).collect();
        for key in due {
            let holder = self.holders.get_mut(&key).expect("held");
            Self::uncover(key, holder, at, out);
        }
        self.write_ended(out);
        Ok(())
    }

    fn prime(&mut self, hour: &Hour<'_>) {
        self.closing.clear();
        let assignments = hour.table(TableId::ResearchAssignments);
        for slot in assignments.rows() {
            let Some(row) = assignments.key(slot) else { continue };
            let (holder, index, lane) = (((row >> 80) as u32, (row >> 48) as u32), (row >> 16) as u32, row as u16);
            let number = |at: usize| assignments.number(ASSIGNMENT[at], slot);
            let (Some(handle), Some(occurrence), Some(token), Some(level), Some(continuity_start), Some(first_observed), Some(outcome), Some(awaiting)) =
                (number(0), number(1), assignments.text(ASSIGNMENT[2], slot), number(3), number(4), number(5), number(9), number(11))
            else {
                continue;
            };
            let cells = Assignment {
                handle: handle as u32,
                occurrence: occurrence as i32,
                token,
                level: level as i32,
                continuity_start: continuity_start as i32,
                first_observed: first_observed as i32,
                started_after: optional(&assignments, ASSIGNMENT[6], slot),
                last_observed: optional(&assignments, ASSIGNMENT[7], slot),
                ended_by: optional(&assignments, ASSIGNMENT[8], slot),
                outcome: outcome as u8,
                outcome_observed: optional(&assignments, ASSIGNMENT[10], slot),
                awaiting: awaiting != 0,
            };
            if !cells.awaiting && !cells.active() {
                if lane != 0 {
                    self.closing.push(row);
                }
                continue;
            }
            if let Some(held) = self.holders.get_mut(&holder) {
                held.slots.entry(index).or_default().push(Open { lane, cells, dirty: false });
            }
        }
        let coverage = hour.table(TableId::ResearchCoverage);
        for slot in coverage.rows() {
            let (Some(row), Some(since)) = (coverage.key(slot), coverage.number(SINCE[0], slot)) else { continue };
            let (holder, member) = (((row >> 64) as u32, (row >> 32) as u32), row as u32);
            let Some(held) = self.holders.get_mut(&holder) else { continue };
            if member == ResearchCoverageRow::HOLDER {
                held.since = Some(since as i32);
            } else {
                held.available.insert(member);
            }
        }
        for opens in self.holders.values_mut().flat_map(|holder| holder.slots.values_mut()) {
            opens.sort_unstable_by_key(|open| open.lane);
        }
        if hour.complete(TableId::Countries) && hour.complete(TableId::CountryLinks) {
            self.indexes = Indexes::read(hour).ok();
        }
        self.rows = hour.complete(TableId::ResearchInputs).then(|| row_counts(hour));
        self.last = Some(hour.hour);
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        let optional = |blob: &mut Bytes, value: Option<i32>| -> io::Result<()> {
            match value {
                None => blob.u8(0),
                Some(value) => {
                    blob.u8(1)?;
                    blob.varint(u64::from(value as u32))
                }
            }
        };
        optional(blob, self.last_bad)?;
        blob.varint(self.recheck.len() as u64)?;
        for &(slot, id) in &self.recheck {
            blob.varint(u64::from(slot))?;
            blob.varint(u64::from(id))?;
        }
        blob.varint(self.holders.len() as u64)?;
        for (&(slot, id), holder) in &self.holders {
            blob.varint(u64::from(slot))?;
            blob.varint(u64::from(id))?;
            blob.varint(holder.tag.len() as u64)?;
            blob.raw(&holder.tag)?;
            optional(blob, holder.last_read)?;
            optional(blob, holder.observed)?;
            optional(blob, holder.reached)?;
            blob.varint(u64::from(holder.continuity_start as u32))?;
        }
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        *self = Self::default();
        let mut reader = Reader::new(blob);
        let optional = |reader: &mut Reader<'_>| -> io::Result<Option<i32>> {
            Ok(match reader.u8()? {
                0 => None,
                _ => Some(reader.varint_u32()? as i32),
            })
        };
        self.last_bad = optional(&mut reader)?;
        let count = reader.count()?;
        if count > blob.len() {
            return Err(invalid("Research's state counts more holders than it holds"));
        }
        for _ in 0..count {
            self.recheck.insert((reader.varint_u32()?, reader.varint_u32()?));
        }
        let count = reader.count()?;
        if count > blob.len() {
            return Err(invalid("Research's state counts more holders than it holds"));
        }
        for _ in 0..count {
            let key = (reader.varint_u32()?, reader.varint_u32()?);
            let length = reader.count()?;
            let tag = reader.raw(length)?.into();
            let holder = Holder { tag, last_read: optional(&mut reader)?, observed: optional(&mut reader)?, reached: optional(&mut reader)?, continuity_start: reader.varint_u32()? as i32, ..Holder::default() };
            self.holders.insert(key, holder);
        }
        reader.finish()
    }
}
