
mod compare;
mod saved;
#[cfg(test)]
mod tests;

use recorder_derive_framework::framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns};
use replay_columnar::{leaf::Writer as Bytes, store::Value as Held};
use replay_data::rows::{
    CountryAppearanceRow, CountryCounterInputRow, CountryLinkRow, CountryRow, PeaceConferenceRow, PeaceConferenceSpanRow, PeaceOutcomeRow, PoliticalPartyRow, PoliticsRow, ProvinceRow, StateRow, SubjectRow,
    TableId, WarRelationRow,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    sync::Arc,
};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

const COUNTRY: [usize; 2] = columns::<CountryRow, 2>(["canonical_slot", "tag"]);
const LINK: [usize; 1] = columns::<CountryLinkRow, 1>(["canonical_slot"]);
const OWNER: [usize; 1] = columns::<StateRow, 1>(["owner"]);
const WAR: [usize; 2] = columns::<WarRelationRow, 2>(["start_hour", "end_hour"]);
const SUBJECT: [usize; 3] = columns::<SubjectRow, 3>(["start_hour", "end_hour", "cancel"]);
const RULING: [usize; 1] = columns::<PoliticsRow, 1>(["ruling_party"]);
const PARTY: [usize; 1] = columns::<PoliticalPartyRow, 1>(["token"]);
const IDEOLOGY: [usize; 1] = columns::<CountryAppearanceRow, 1>(["leader_ideology"]);
const STATES_OWNED: [usize; 1] = columns::<CountryCounterInputRow, 1>(["owned_state_count"]);
const PROVINCE: [usize; 2] = columns::<ProvinceRow, 2>(["state", "victory_points"]);
const CONFERENCE: [usize; 2] = columns::<PeaceConferenceRow, 2>(["current_turn", "finalized"]);

const fn read(table: TableId, columns: &'static [usize]) -> Read {
    Read { table, columns: Columns::These(columns), watch: Watch::Changed, required: false }
}

const READS: &[Read] = &[
    read(TableId::Countries, &COUNTRY),
    read(TableId::CountryLinks, &LINK),
    read(TableId::States, &OWNER),
    read(TableId::WarRelations, &WAR),
    read(TableId::Subjects, &SUBJECT),
    read(TableId::Politics, &RULING),
    read(TableId::PoliticalParties, &PARTY),
    read(TableId::CountryAppearance, &IDEOLOGY),
    read(TableId::CountryCounterInputs, &STATES_OWNED),
    read(TableId::Provinces, &PROVINCE),
    Read { table: TableId::PeaceConferences, columns: Columns::These(&CONFERENCE), watch: Watch::EveryRead, required: false },
];

type Tag = u32;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Countries {
    tags: HashMap<u32, Tag>,
    identities: HashMap<Tag, (u32, u32)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Wars {
    pairs: HashMap<(Tag, Tag), (i32, i32)>,
    unresolved: Vec<(u32, u32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Government {
    ruling: Option<u32>,
    party: Option<Box<[u8]>>,
    ideology: Option<Box<[u8]>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    hour: i32,
    roster_complete: bool,
    countries: Arc<Countries>,
    owners: Arc<HashMap<u32, u32>>,
    wars: Arc<Wars>,
    vps: Option<Arc<HashMap<u32, i64>>>,
    subjects: Option<Arc<HashMap<(Tag, Tag), i32>>>,
    governments: Option<Arc<HashMap<Tag, Government>>>,
    exists: Option<Arc<HashMap<u32, bool>>>,
}

#[derive(Default)]
struct World {
    countries: Option<Arc<Countries>>,
    owners: Option<Arc<HashMap<u32, u32>>>,
    wars: Option<Arc<Wars>>,
    subjects: Option<Arc<HashMap<(Tag, Tag), i32>>>,
    subject_expiry: Option<i32>,
    governments: Option<Arc<HashMap<Tag, Government>>>,
    exists: Option<Arc<HashMap<u32, bool>>>,
    vps: Option<Arc<HashMap<u32, i64>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Conference {
    row: PeaceConferenceSpanRow,
    before: Option<Arc<Snapshot>>,
    open: bool,
    waiting: bool,
    processed: bool,
    unassigned: bool,
}

#[derive(Clone, Copy)]
struct Head {
    observation: u8,
    conference: Option<u64>,
    before: i32,
    group: u32,
}

#[derive(Default)]
pub struct Peace {
    tags: HashMap<Box<[u8]>, Tag>,
    world: World,
    previous: Option<Arc<Snapshot>>,
    conferences: BTreeMap<u64, Conference>,
    has_active: bool,
    has_ambiguous: bool,
    first_existing: HashMap<Tag, i32>,
    living: Vec<u128>,
    last: Option<i32>,
}

fn complete(hour: &Hour<'_>, table: TableId) -> bool {
    hour.complete(table)
}

fn dirty(hour: &Hour<'_>, table: TableId, watched: &'static [usize], whole: bool) -> bool {
    whole || hour.changes(table).status_changed || hour.wrote(table, Columns::These(watched))
}

fn optional_u32(value: Held<'_>) -> Option<u32> {
    match value {
        Held::Number(bits) => Some(bits as u32),
        _ => None,
    }
}

impl Peace {
    fn tag(&mut self, bytes: &[u8]) -> Tag {
        let next = self.tags.len() as Tag;
        *self.tags.entry(bytes.into()).or_insert(next)
    }

    fn countries(&mut self, hour: &Hour<'_>) -> io::Result<Option<Countries>> {
        if !complete(hour, TableId::Countries) || !complete(hour, TableId::CountryLinks) {
            return Ok(None);
        }
        let table = hour.table(TableId::Countries);
        let mut countries = Countries::default();
        let mut slots: HashMap<u32, Tag> = HashMap::new();
        for slot in table.rows() {
            let (Some(id), Some(canonical)) = (table.key(slot), table.number(COUNTRY[0], slot)) else { return Ok(None) };
            let bytes = table.text(COUNTRY[1], slot).and_then(|text| hour.text(text)).ok_or_else(|| invalid("A country names a tag its run never added"))?;
            if bytes.is_empty() || std::str::from_utf8(bytes).is_err() {
                return Ok(None);
            }
            let tag = self.tag(bytes);
            let (id, canonical) = (id as u32, canonical as u32);
            if countries.identities.insert(tag, (canonical, id)).is_some() || slots.insert(canonical, tag).is_some() {
                return Ok(None);
            }
            countries.tags.insert(id, tag);
        }
        let links = hour.table(TableId::CountryLinks);
        for slot in links.rows() {
            let (Some(id), Some(canonical)) = (links.key(slot), links.number(LINK[0], slot)) else { continue };
            if let Some(&tag) = slots.get(&(canonical as u32)) {
                if countries.tags.get(&(id as u32)).is_some_and(|&prior| prior != tag) {
                    return Ok(None);
                }
                countries.tags.insert(id as u32, tag);
            }
        }
        Ok(Some(countries))
    }

    fn owners(&mut self, hour: &Hour<'_>, whole: bool) -> Option<Vec<u32>> {
        if !complete(hour, TableId::States) {
            self.world.owners = None;
            return None;
        }
        let table = hour.table(TableId::States);
        if whole || self.world.owners.is_none() || hour.changes(TableId::States).status_changed {
            let owners: HashMap<u32, u32> = table.rows().filter_map(|slot| Some((table.key(slot)? as u32, table.number(OWNER[0], slot)? as u32))).collect();
            let every = owners.keys().copied().collect();
            self.world.owners = Some(Arc::new(owners));
            return Some(every);
        }
        if !hour.wrote(TableId::States, Columns::These(&OWNER)) {
            return Some(Vec::new());
        }
        let owners = Arc::make_mut(self.world.owners.as_mut().expect("owners held"));
        let mut moved = Vec::new();
        for slot in hour.written_rows(TableId::States, Columns::These(&OWNER)) {
            let (Some(state), Some(owner)) = (table.key(slot), table.number(OWNER[0], slot)) else { continue };
            owners.insert(state as u32, owner as u32);
            moved.push(state as u32);
        }
        for key in hour.deleted_keys(TableId::States) {
            owners.remove(&(key as u32));
            moved.push(key as u32);
        }
        moved.sort_unstable();
        moved.dedup();
        Some(moved)
    }

    fn wars(hour: &Hour<'_>, countries: &Countries) -> Wars {
        let table = hour.table(TableId::WarRelations);
        let mut wars = Wars::default();
        let mut ambiguous = HashSet::new();
        let mut rows: Vec<(u128, u32)> = table.rows().filter_map(|slot| Some((table.key(slot)?, slot))).collect();
        rows.sort_unstable();
        for (key, slot) in rows {
            let (a, b) = ((key >> 32) as u32, key as u32);
            let (Some(start), Some(end)) = (table.number(WAR[0], slot), table.number(WAR[1], slot)) else {
                wars.unresolved.push((a, b));
                continue;
            };
            let (Some(&first), Some(&second)) = (countries.tags.get(&a), countries.tags.get(&b)) else {
                wars.unresolved.push((a, b));
                continue;
            };
            let pair = compare::pair(first, second);
            let value = (start as i32, end as i32);
            if wars.pairs.get(&pair).is_some_and(|prior| *prior != value) {
                wars.pairs.remove(&pair);
                ambiguous.insert(pair);
                wars.unresolved.push((a, b));
            }
            if !ambiguous.contains(&pair) {
                wars.pairs.insert(pair, value);
            }
        }
        wars
    }

    fn subjects(hour: &Hour<'_>, countries: &Countries, at: i32) -> (Option<HashMap<(Tag, Tag), i32>>, Option<i32>) {
        let table = hour.table(TableId::Subjects);
        let mut subjects = HashMap::new();
        let mut expiry: Option<i32> = None;
        let mut resolved = true;
        for slot in table.rows() {
            let Some(key) = table.key(slot) else { continue };
            let [start, end, cancel] = SUBJECT.map(|column| table.number(column, slot));
            let (Some(start), Some(end), Some(cancel)) = (start, end, cancel) else {
                resolved = false;
                continue;
            };
            let (start, end) = (start as i32, end as i32);
            if cancel == 0 && end > start && end > at {
                expiry = Some(expiry.map_or(end, |due| due.min(end)));
            }
            if cancel != 0 || (end > start && at >= end) {
                continue;
            }
            match (countries.tags.get(&((key >> 32) as u32)), countries.tags.get(&(key as u32))) {
                (Some(&overlord), Some(&subject)) => {
                    subjects.insert((overlord, subject), start);
                }
                _ => resolved = false,
            }
        }
        (resolved.then_some(subjects), expiry)
    }

    fn governments(hour: &Hour<'_>, countries: &Countries) -> Option<HashMap<Tag, Government>> {
        let words = |table: TableId, column: usize| -> Option<HashMap<u128, Box<[u8]>>> {
            let view = hour.table(table);
            view.rows().map(|slot| Some((view.key(slot)?, hour.text(view.text(column, slot)?)?.into()))).collect()
        };
        let parties = words(TableId::PoliticalParties, PARTY[0])?;
        let ideologies = words(TableId::CountryAppearance, IDEOLOGY[0])?;
        let mut tokens: HashMap<(Tag, u32), Box<[u8]>> = HashMap::with_capacity(parties.len());
        for (key, token) in parties {
            tokens.insert((*countries.tags.get(&((key >> 32) as u32))?, key as u32), token);
        }
        let mut leaders: HashMap<Tag, Box<[u8]>> = HashMap::with_capacity(ideologies.len());
        for (key, ideology) in ideologies {
            leaders.insert(*countries.tags.get(&(key as u32))?, ideology);
        }
        let politics = hour.table(TableId::Politics);
        let mut governments = HashMap::new();
        for slot in politics.rows() {
            let tag = *countries.tags.get(&(politics.key(slot)? as u32))?;
            let ruling = optional_u32(politics.value(RULING[0], slot));
            let party = ruling.and_then(|id| tokens.get(&(tag, id)).cloned());
            governments.insert(tag, Government { ruling, party, ideology: leaders.get(&tag).cloned() });
        }
        Some(governments)
    }

    fn exists(&mut self, hour: &Hour<'_>, whole: bool) -> Vec<u32> {
        let table = hour.table(TableId::CountryCounterInputs);
        if !complete(hour, TableId::CountryCounterInputs) {
            self.world.exists = None;
            return Vec::new();
        }
        let held = match &mut self.world.exists {
            Some(held) if !whole && !hour.changes(TableId::CountryCounterInputs).status_changed => held,
            _ => {
                let exists: HashMap<u32, bool> = table.rows().filter_map(|slot| Some((table.key(slot)? as u32, table.number(STATES_OWNED[0], slot)? > 0))).collect();
                let every = exists.keys().copied().collect();
                self.world.exists = Some(Arc::new(exists));
                return every;
            }
        };
        if !hour.wrote(TableId::CountryCounterInputs, Columns::These(&STATES_OWNED)) {
            return Vec::new();
        }
        let exists = Arc::make_mut(held);
        let mut changed = Vec::new();
        for slot in hour.written_rows(TableId::CountryCounterInputs, Columns::These(&STATES_OWNED)) {
            let (Some(country), Some(count)) = (table.key(slot), table.number(STATES_OWNED[0], slot)) else { continue };
            exists.insert(country as u32, count > 0);
            changed.push(country as u32);
        }
        for key in hour.deleted_keys(TableId::CountryCounterInputs) {
            exists.remove(&(key as u32));
        }
        changed
    }

    fn vps(hour: &Hour<'_>) -> Option<HashMap<u32, i64>> {
        if !complete(hour, TableId::Provinces) {
            return None;
        }
        let table = hour.table(TableId::Provinces);
        let mut totals: HashMap<u32, i64> = HashMap::new();
        for slot in table.rows() {
            let (Some(state), Some(points)) = (table.number(PROVINCE[0], slot), table.number(PROVINCE[1], slot)) else { return None };
            *totals.entry(state as u32).or_insert(0) += points;
        }
        Some(totals)
    }

    fn advance(&mut self, hour: &Hour<'_>, whole: bool) -> io::Result<(Option<Vec<u32>>, Vec<u32>, bool)> {
        let at = hour.hour;
        let mut holders = false;
        if dirty(hour, TableId::Countries, &COUNTRY, whole) || dirty(hour, TableId::CountryLinks, &LINK, whole) {
            let countries = self.countries(hour)?;
            if self.world.countries.as_deref() != countries.as_ref() {
                self.world.countries = countries.map(Arc::new);
                holders = true;
            }
        }
        let moved = self.owners(hour, whole);
        let changed = self.exists(hour, whole);
        if dirty(hour, TableId::Provinces, &PROVINCE, whole) {
            self.world.vps = Self::vps(hour).map(Arc::new);
        }
        let resolve = whole || holders;
        let Some(countries) = self.world.countries.clone() else {
            self.world.wars = None;
            self.world.subjects = None;
            self.world.subject_expiry = None;
            self.world.governments = None;
            return Ok((moved, changed, holders));
        };
        if resolve || dirty(hour, TableId::WarRelations, &WAR, false) {
            self.world.wars = complete(hour, TableId::WarRelations).then(|| Arc::new(Self::wars(hour, &countries)));
        }
        if resolve || dirty(hour, TableId::Subjects, &SUBJECT, false) || self.world.subject_expiry.is_some_and(|due| due <= at) {
            let (subjects, expiry) = if complete(hour, TableId::Subjects) { Self::subjects(hour, &countries, at) } else { (None, None) };
            self.world.subjects = subjects.map(Arc::new);
            self.world.subject_expiry = expiry;
        }
        if resolve || dirty(hour, TableId::Politics, &RULING, false) || dirty(hour, TableId::PoliticalParties, &PARTY, false) || dirty(hour, TableId::CountryAppearance, &IDEOLOGY, false) {
            let known = [TableId::Politics, TableId::PoliticalParties, TableId::CountryAppearance].into_iter().all(|table| complete(hour, table));
            self.world.governments = if known { Self::governments(hour, &countries).map(Arc::new) } else { None };
        }
        Ok((moved, changed, holders))
    }

    fn snapshot(&self, at: i32, roster_complete: bool) -> Option<Snapshot> {
        Some(Snapshot {
            hour: at,
            roster_complete,
            countries: self.world.countries.clone()?,
            owners: self.world.owners.clone()?,
            wars: self.world.wars.clone()?,
            vps: self.world.vps.clone(),
            subjects: self.world.subjects.clone(),
            governments: self.world.governments.clone(),
            exists: self.world.exists.clone(),
        })
    }

    fn recount(&mut self) {
        let active = || self.conferences.values().filter(|conference| conference.open && !conference.processed);
        self.has_active = active().next().is_some();
        self.has_ambiguous = active().any(|conference| conference.unassigned);
    }

    fn publish(conference: &Conference, spans: &mut BTreeMap<u64, Vec<Value>>) {
        let row = &conference.row;
        let optional = |value: Option<i32>| Value::OptionalI32(value);
        spans.insert(
            row.conference,
            vec![
                Value::I32(row.first_observed_hour),
                Value::I32(row.last_observed_hour),
                Value::I32(row.current_turn),
                Value::U8(row.finalized),
                optional(row.first_finalized_hour),
                optional(row.first_absent_hour),
                Value::U8(row.comparison),
                optional(row.before_hour),
                optional(row.compared_hour),
            ],
        );
    }

    fn write(&mut self, out: &mut Rows, at: i32, ordinal: &mut u32, head: Head, items: &[compare::Item]) {
        for item in items {
            let key = u128::from(at as u32) << 32 | u128::from(*ordinal);
            *ordinal += 1;
            out.set(
                TableId::PeaceOutcomes,
                key,
                vec![
                    Value::U8(head.observation),
                    Value::OptionalU64(head.conference),
                    Value::I32(head.before),
                    Value::U8(item.change),
                    Value::U32(head.group),
                    Value::U32(item.state),
                    Value::U32(item.from.0),
                    Value::U32(item.from.1),
                    Value::U32(item.to.0),
                    Value::U32(item.to.1),
                    Value::OptionalI64(item.vp),
                ],
            );
            self.living.push(key);
        }
    }

    fn roster(hour: &Hour<'_>) -> Option<Vec<(u64, i32, u8)>> {
        if !complete(hour, TableId::PeaceConferences) {
            return None;
        }
        let table = hour.table(TableId::PeaceConferences);
        let mut rows: Vec<(u64, i32, u8)> = table
            .rows()
            .map(|slot| Some((table.key(slot)? as u64, table.number(CONFERENCE[0], slot)? as i32, table.number(CONFERENCE[1], slot)? as u8)))
            .collect::<Option<_>>()?;
        rows.sort_unstable();
        Some(rows)
    }

    fn read_roster(&mut self, at: i32, rows: &[(u64, i32, u8)], current: Option<&Arc<Snapshot>>, spans: &mut BTreeMap<u64, Vec<Value>>) {
        let active: Vec<u64> = self.conferences.iter().filter(|(_, conference)| conference.open && !conference.processed).map(|(&id, _)| id).collect();
        if active.len() > 1 {
            for id in &active {
                self.conferences.get_mut(id).expect("active").unassigned = true;
            }
        }
        let listed: HashSet<u64> = rows.iter().map(|&(id, _, _)| id).collect();
        for (id, conference) in &mut self.conferences {
            if conference.open && !listed.contains(id) {
                conference.open = false;
                conference.row.first_absent_hour = Some(at);
                if !conference.processed {
                    conference.waiting = true;
                }
                Self::publish(conference, spans);
            }
        }
        for &(id, turn, finalized) in rows {
            let listed = Conference {
                row: PeaceConferenceSpanRow { conference: id, first_observed_hour: at, ..Default::default() },
                before: self.previous.clone(),
                open: true,
                waiting: false,
                processed: false,
                unassigned: false,
            };
            let conference = self.conferences.entry(id).or_insert_with(|| listed.clone());
            if !conference.open {
                *conference = listed;
            }
            conference.row.last_observed_hour = at;
            conference.row.current_turn = turn;
            conference.row.finalized = u8::from(finalized != 0);
            if finalized != 0 && conference.row.first_finalized_hour.is_none() {
                conference.row.first_finalized_hour = Some(at);
                if !conference.processed {
                    conference.waiting = true;
                }
            }
            if finalized == 0
                && let Some(current) = current
            {
                conference.before = if conference.unassigned { None } else { Some(Arc::clone(current)) };
            }
            Self::publish(conference, spans);
        }
        let comparisons: Vec<u64> = self.conferences.iter().filter(|(_, conference)| (conference.open && !conference.processed) || conference.waiting).map(|(&id, _)| id).collect();
        if comparisons.len() > 1 {
            for id in &comparisons {
                self.conferences.get_mut(id).expect("compared").unassigned = true;
            }
        }
        self.recount();
    }

    fn compare(&mut self, current: &Arc<Snapshot>, moved: Option<&[u32]>, had: (bool, bool), spans: &mut BTreeMap<u64, Vec<Value>>, out: &mut Rows) {
        let (had_open, had_active) = had;
        let at = current.hour;
        let mut ordinal = 0;
        let candidates: Vec<u64> = self.conferences.iter().filter(|(_, conference)| conference.waiting).map(|(&id, _)| id).collect();
        let assigned = candidates.len() == 1 && !self.conferences[&candidates[0]].unassigned;
        if assigned {
            let id = candidates[0];
            let before = self.conferences[&id].before.clone();
            match before {
                Some(before) => {
                    let transfers = compare::transfers(&before, current, true, None);
                    let items = compare::populate(&before, current, &transfers, &self.first_existing);
                    self.write(out, at, &mut ordinal, Head { observation: PeaceOutcomeRow::RECORDED, conference: Some(id), before: before.hour, group: 0 }, &items);
                    let row = &mut self.conferences.get_mut(&id).expect("assigned").row;
                    row.comparison = PeaceConferenceSpanRow::COMPARED;
                    row.before_hour = Some(before.hour);
                    row.compared_hour = Some(at);
                }
                None => self.conferences.get_mut(&id).expect("assigned").row.comparison = PeaceConferenceSpanRow::NO_BEFORE,
            }
        }
        if let Some(previous) = self.previous.clone() {
            let consecutive = previous.hour.checked_add(1) == Some(at);
            let unmatched = !assigned && ((!had_active && !self.has_active) || self.has_ambiguous || candidates.iter().any(|id| self.conferences[id].unassigned));
            if unmatched
                && consecutive
                && let Some(moved) = moved.filter(|moved| !moved.is_empty())
            {
                let transfers = compare::transfers(&previous, current, false, Some(moved));
                if !transfers.is_empty() {
                    let observation = if !had_open && !self.conferences.values().any(|conference| conference.open) && current.roster_complete && previous.roster_complete {
                        PeaceOutcomeRow::NOT_RECORDED
                    } else {
                        PeaceOutcomeRow::UNKNOWN
                    };
                    for (group, transfers) in compare::components(&transfers, current).into_iter().enumerate() {
                        let items = compare::populate(&previous, current, &transfers, &self.first_existing);
                        self.write(out, at, &mut ordinal, Head { observation, conference: None, before: previous.hour, group: group as u32 }, &items);
                    }
                }
            }
        }
        for id in &candidates {
            let conference = self.conferences.get_mut(id).expect("candidate");
            if conference.unassigned {
                conference.row.comparison = PeaceConferenceSpanRow::OVERLAPPED;
            }
            conference.waiting = false;
            conference.processed = true;
            Self::publish(conference, spans);
        }
        if !candidates.is_empty() {
            self.recount();
        }
        self.conferences.retain(|_, conference| conference.open || conference.waiting);
    }
}

impl Writer for Peace {
    fn name(&self) -> &'static str {
        "Peace"
    }

    fn version(&self) -> u32 {
        2
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::PeaceOutcomes), Output::State(TableId::PeaceConferenceSpans)]
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
        for key in self.living.drain(..) {
            out.delete(TableId::PeaceOutcomes, key);
        }
        for key in recorder_derive_framework::forces::held(out, TableId::PeaceConferenceSpans) {
            out.delete(TableId::PeaceConferenceSpans, key);
        }
        let at = hour.hour;
        let whole = hour.fresh || self.last.is_none_or(|last| last.checked_add(1) != Some(at));
        self.last = Some(at);
        let (moved, existence, holders) = self.advance(hour, whole)?;
        let roster = Self::roster(hour);
        let roster_complete = roster.is_some();
        let current = self.snapshot(at, roster_complete).map(Arc::new);
        let had = (self.conferences.values().any(|conference| conference.open), self.has_active);
        let read = hour.first || whole || hour.table(TableId::PeaceConferences).sampled_at() == Some(at);
        let mut spans = BTreeMap::new();
        if read && let Some(rows) = &roster {
            self.read_roster(at, rows, current.as_ref(), &mut spans);
        }
        if let Some(current) = &current {
            self.compare(current, moved.as_deref(), had, &mut spans, out);
            self.previous = Some(Arc::clone(current));
        }
        for (conference, cells) in spans {
            out.set(TableId::PeaceConferenceSpans, u128::from(conference) << 32 | u128::from(at as u32), cells);
        }
        if let (Some(exists), Some(countries)) = (&self.world.exists, &self.world.countries) {
            let raws: Vec<u32> = if holders || whole { exists.keys().copied().collect() } else { existence };
            for raw in raws {
                if exists.get(&raw) == Some(&true)
                    && let Some(&tag) = countries.tags.get(&raw)
                {
                    self.first_existing.entry(tag).or_insert(at);
                }
            }
        }
        Ok(())
    }

    fn prime(&mut self, hour: &Hour<'_>) {
        let events = hour.table(TableId::PeaceOutcomes);
        self.living = events.rows().filter_map(|slot| events.key(slot)).collect();
        self.world = World::default();
        if self.advance(hour, true).is_err() {
            self.world = World::default();
        }
        self.last = Some(hour.hour);
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        saved::save(self, blob)
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        *self = Self::default();
        saved::load(self, blob)
    }
}
