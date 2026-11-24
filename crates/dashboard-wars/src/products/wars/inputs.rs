
use super::native::{Pair, World, pair};
use dashboard_query::cost;
use replay_recording::columnar_frames::Recording;
use replay_lanes::lanes::{self, Cell, History, Keys};
use replay_data::rows::{self, LocalPlayerRow, TableId, WarOverviewCountryRow, WarOwnedRelationRow, WarRelationRow, tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Relation {
    pub start: i32,
    pub instigator: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Input {
    Relation,
    Owned,
    Faction,
    Score,
    Rule,
    Local,
    Capitulation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Row {
    Relation(Relation),
    Owned(WarOwnedRelationRow),
    Faction(u64),
    Score(WarOverviewCountryRow),
    Rule(Vec<u8>, u32),
    Local(LocalPlayerRow),
    Capitulation(u8, i32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Shown {
    Row(Row),
    Gone,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Change {
    pub hour: i32,
    pub input: Input,
    pub key: u128,
    pub shown: Shown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct State {
    pub relations: BTreeMap<u64, Relation>,
    pub owned: BTreeMap<u64, WarOwnedRelationRow>,
    pub factions: BTreeMap<u32, u64>,
    pub scores: BTreeMap<u32, WarOverviewCountryRow>,
    pub rules: BTreeMap<u32, (Vec<u8>, u32)>,
    pub local: Option<LocalPlayerRow>,
    pub capitulations: BTreeMap<u32, (u8, i32)>,
    pub unknown: BTreeSet<(Input, u128)>,
}

impl State {
    pub fn apply(&mut self, change: &Change) {
        let key = change.key;
        if matches!(change.shown, Shown::Unknown) {
            self.unknown.insert((change.input, key));
        } else {
            self.unknown.remove(&(change.input, key));
        }
        let row = match &change.shown {
            Shown::Row(row) => Some(row.clone()),
            Shown::Gone | Shown::Unknown => None,
        };
        match (change.input, row) {
            (Input::Relation, Some(Row::Relation(row))) => {
                self.relations.insert(key as u64, row);
            }
            (Input::Relation, _) => {
                self.relations.remove(&(key as u64));
            }
            (Input::Owned, Some(Row::Owned(row))) => {
                self.owned.insert(key as u64, row);
            }
            (Input::Owned, _) => {
                self.owned.remove(&(key as u64));
            }
            (Input::Faction, Some(Row::Faction(faction))) => {
                self.factions.insert(key as u32, faction);
            }
            (Input::Faction, _) => {
                self.factions.remove(&(key as u32));
            }
            (Input::Score, Some(Row::Score(row))) => {
                self.scores.insert(key as u32, row);
            }
            (Input::Score, _) => {
                self.scores.remove(&(key as u32));
            }
            (Input::Rule, Some(Row::Rule(prefix, viewpoint))) => {
                self.rules.insert(key as u32, (prefix, viewpoint));
            }
            (Input::Rule, _) => {
                self.rules.remove(&(key as u32));
            }
            (Input::Local, Some(Row::Local(row))) => self.local = Some(row),
            (Input::Local, _) => self.local = None,
            (Input::Capitulation, Some(Row::Capitulation(capitulated, hour))) => {
                self.capitulations.insert(key as u32, (capitulated, hour));
            }
            (Input::Capitulation, _) => {
                self.capitulations.remove(&(key as u32));
            }
        }
    }

    pub fn of_frame(frame: &DecodedFrame) -> Result<Self, String> {
        let mut state = Self::default();
        for r in frame.complete::<tables::WarOwnedRelations>().map_err(|e| e.to_string())? {
            state.owned.insert(u64::from(r.owner) << 32 | u64::from(r.ordinal), r.clone());
        }
        for r in frame.complete::<tables::WarRelations>().map_err(|e| e.to_string())? {
            let key = u64::from(r.first_country) << 32 | u64::from(r.second_country);
            state.relations.insert(key, Relation { start: r.start_hour, instigator: r.first_was_instigator });
        }
        for r in frame.complete::<tables::FactionMembers>().map_err(|e| e.to_string())? {
            state.factions.insert(r.country, r.faction);
        }
        let rules = frame.complete::<tables::WarNameRules>().map_err(|_| "Not recorded: war overview viewpoint")?;
        for r in rules {
            state.rules.insert(r.index, (r.prefix.clone(), r.viewpoint));
        }
        state.local = frame.complete::<tables::LocalPlayer>().ok().and_then(|rows| rows.first().cloned());
        if let Ok(scores) = frame.complete::<tables::WarOverviewCountries>() {
            for r in scores {
                state.scores.insert(r.country, r.clone());
            }
        }
        Ok(state)
    }

    pub fn capitulated(&self) -> Vec<(u32, i32)> {
        self.capitulations.iter().filter(|(_, (capitulated, _))| *capitulated != 0).map(|(&country, &(_, hour))| (country, hour)).collect()
    }
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub(crate) struct Countries {
    pub tags: HashMap<u32, String>,
    pub identities: HashMap<u32, (u32, u32)>,
    pub raw_to_slot: HashMap<u32, u32>,
}

pub(crate) trait Resolve {
    fn slot(&self, raw: u32) -> Option<u32>;
    fn holder(&self, slot: u32) -> Option<(&str, u32)>;
    fn country_slot(&self, country: &dashboard_base::products::Country) -> Option<u32>;
}

#[derive(Clone)]
pub(crate) struct Hour {
    pub world: World,
    pub tags: HashMap<u32, String>,
    pub identities: HashMap<u32, (u32, u32)>,
    pub raw_to_slot: HashMap<u32, u32>,
    pub view: u32,
    pub recorded_view: Option<u32>,
    pub goal_owned: HashMap<Pair, WarOwnedRelationRow>,
}

impl Countries {
    fn add(&mut self, resolve: &dyn Resolve, raw: u32) -> Result<u32, String> {
        if let Some(&slot) = self.raw_to_slot.get(&raw) {
            return Ok(slot);
        }
        let slot = resolve.slot(raw).ok_or_else(|| format!("Not recorded: canonical country link {raw}"))?;
        self.held(resolve, slot)?;
        self.raw_to_slot.insert(raw, slot);
        Ok(slot)
    }

    fn held(&mut self, resolve: &dyn Resolve, slot: u32) -> Result<(), String> {
        if let std::collections::hash_map::Entry::Vacant(vacant) = self.tags.entry(slot) {
            let (tag, id) = resolve.holder(slot).ok_or_else(|| format!("Not recorded: the country in slot {slot} holds no tag at this hour"))?;
            vacant.insert(tag.to_owned());
            self.identities.insert(slot, (slot, id));
        }
        Ok(())
    }
}

impl Hour {
    pub fn build(state: &State, resolve: &dyn Resolve, view: Option<&dashboard_base::products::Country>) -> Result<Self, String> {
        if !state.unknown.is_empty() {
            return Err("Not recorded: a war overview row holds a cell not recorded".into());
        }
        let mut countries = Countries::default();
        let recorded_view = state.local.as_ref().and_then(|r| countries.add(resolve, if r.country as i32 > 0 { r.country } else { r.observer_country }).ok());
        let view = match view {
            Some(country) => {
                let slot = resolve.country_slot(country).ok_or_else(|| country.tag().map_or_else(|| "Not in this recording: viewpoint".to_owned(), |tag| format!("Not in this recording: viewpoint {tag}")))?;
                countries.held(resolve, slot)?;
                slot
            }
            None => recorded_view.ok_or("Not recorded: local playing/observer country")?,
        };
        let mut owned = HashMap::<u32, Vec<u32>>::new();
        let mut goal_owned = HashMap::new();
        for r in state.owned.values() {
            let (owner, a, b) = (countries.add(resolve, r.owner)?, countries.add(resolve, r.first_country)?, countries.add(resolve, r.second_country)?);
            if a == b || (owner != a && owner != b) {
                return Err("Not recorded: owned war endpoints disagree with owner".into());
            }
            owned.entry(owner).or_default().push(if owner == a { b } else { a });
            if owner == a && r.retired == 0 && goal_owned.insert(pair(a, b), r.clone()).is_some() {
                return Err("Not recorded: multiple live owned war occurrences for one pair".into());
            }
        }
        let mut world = World { owned, relations: HashMap::new(), adjacent: HashMap::new(), factions: HashMap::new(), scores: HashMap::new() };
        for (&key, relation) in &state.relations {
            let (a, b) = (countries.add(resolve, (key >> 32) as u32)?, countries.add(resolve, key as u32)?);
            let p = pair(a, b);
            let row = WarRelationRow { first_country: a, second_country: b, start_hour: relation.start, first_was_instigator: relation.instigator, ..Default::default() };
            if p.0 == p.1 || world.relations.insert(p, row).is_some() {
                return Err("Not recorded: duplicate canonical active war pair".into());
            }
            world.adjacent.entry(p.0).or_default().push(p.1);
            world.adjacent.entry(p.1).or_default().push(p.0);
        }
        for (&raw, &faction) in &state.factions {
            let country = countries.add(resolve, raw)?;
            if world.factions.insert(country, faction).is_some() {
                return Err("Not recorded: a country has multiple faction memberships".into());
            }
        }
        let mut rules = state.rules.iter();
        let (Some((&0, &(_, viewpoint))), None) = (rules.next(), rules.next()) else {
            return Err("Not recorded: war overview viewpoint row absent or repeated".into());
        };
        if viewpoint == 0 || Some(countries.add(resolve, viewpoint).map_err(|_| "Not recorded: war overview viewpoint cannot be resolved")?) != recorded_view {
            return Err("Not recorded: war overview viewpoint differs from local player".into());
        }
        for (&raw, row) in &state.scores {
            let slot = countries.add(resolve, raw)?;
            world.scores.insert(slot, WarOverviewCountryRow { country: slot, ..row.clone() });
        }
        for (raw, _) in state.capitulated() {
            let _ = countries.add(resolve, raw);
        }
        let Countries { tags, identities, raw_to_slot } = countries;
        Ok(Self { world, tags, identities, raw_to_slot, view, recorded_view, goal_owned })
    }

    pub fn countries(&self) -> Countries {
        Countries { tags: self.tags.clone(), identities: self.identities.clone(), raw_to_slot: self.raw_to_slot.clone() }
    }
}

#[derive(Default)]
pub struct RunWars {
    pub(crate) changes: Vec<Change>,
    pub(crate) raws: BTreeSet<u32>,
    pub(crate) read: Vec<(i32, i32)>,
    pub(crate) capitulations: Vec<(i32, i32)>,
    pub(crate) goals: Stepped<Goal>,
    pub(crate) tokens: Stepped<Box<[u8]>>,
    pub(crate) states: Stepped<(Box<[u8]>, Box<[u8]>)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Goal {
    pub resolved: u8,
    pub type_token: u32,
    pub state_count: u32,
    pub first_state: u32,
    pub country_count: u32,
    pub first_country: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Held<T> {
    Value(T),
    Gone,
    Unknown,
}

#[derive(Debug)]
pub(crate) struct Stepped<T> {
    keys: HashMap<u128, Vec<(i32, Held<T>)>>,
}

impl<T> Default for Stepped<T> {
    fn default() -> Self {
        Self { keys: HashMap::new() }
    }
}

impl<T> Stepped<T> {
    fn of(history: &History, keep: impl Fn(u128) -> bool, decode: impl Fn(&History, &[Cell]) -> Option<T>) -> Self {
        let keys = history
            .keys
            .iter()
            .filter(|(key, _)| keep(*key))
            .map(|(key, steps)| {
                let steps = steps
                    .iter()
                    .map(|step| {
                        let held = match &step.cells {
                            None => Held::Gone,
                            Some(cells) => decode(history, cells).map_or(Held::Unknown, Held::Value),
                        };
                        (step.hour, held)
                    })
                    .collect();
                (*key, steps)
            })
            .collect();
        Self { keys }
    }

    pub fn at(&self, key: u128, hour: i32) -> Option<&Held<T>> {
        let steps = self.keys.get(&key).map_or(&[][..], Vec::as_slice);
        let at = steps.partition_point(|step| step.0 <= hour);
        at.checked_sub(1).map(|at| &steps[at].1)
    }

    fn values(&self) -> impl Iterator<Item = &T> {
        self.keys.values().flatten().filter_map(|(_, held)| match held {
            Held::Value(value) => Some(value),
            _ => None,
        })
    }
}

pub(crate) fn holds(runs: &[(i32, i32)], hour: i32) -> bool {
    let at = runs.partition_point(|run| run.1 < hour);
    runs.get(at).is_some_and(|run| run.0 <= hour)
}

pub(crate) fn both(a: &[(i32, i32)], b: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < a.len() && j < b.len() {
        let (first, last) = (a[i].0.max(b[j].0), a[i].1.min(b[j].1));
        if first <= last {
            out.push((first, last));
        }
        if a[i].1 < b[j].1 { i += 1 } else { j += 1 }
    }
    out
}

fn number(history: &History, cells: &[Cell], column: usize) -> Option<i64> {
    match cells.get(history.columns.binary_search(&column).ok()?)? {
        Cell::Number(value) => Some(*value),
        _ => None,
    }
}

const RELATIONS: (TableId, &[&str]) = (TableId::WarRelations, &["start_hour", "first_was_instigator"]);
const OWNED: (TableId, &[&str]) = (TableId::WarOwnedRelations, &["first_country", "second_country", "retired", "first_goal_count", "second_goal_count"]);
const FACTIONS: (TableId, &[&str]) = (TableId::FactionMembers, &["faction"]);
const SCORES: (TableId, &[&str]) = (TableId::WarOverviewCountries, &["major", "divisions", "factories", "view_subject", "score_available"]);
const RULES: (TableId, &[&str]) = (TableId::WarNameRules, &["prefix", "viewpoint"]);
const LOCAL: (TableId, &[&str]) = (TableId::LocalPlayer, &["country", "observer_country"]);
const CAPITULATIONS: (TableId, &[&str]) = (TableId::Capitulation, &["capitulated", "capitulated_hour"]);
#[cfg(test)]
pub(crate) const READ: [(TableId, &[&str]); 7] = [RELATIONS, OWNED, FACTIONS, SCORES, RULES, LOCAL, CAPITULATIONS];

pub(crate) fn columns(table: TableId, names: &[&str]) -> io::Result<Vec<usize>> {
    names.iter().map(|name| dashboard_declare::field(table, name)).collect()
}

fn steps(
    (recording, run, first, last): (&Recording, u32, i32, i32),
    ((table, names), input): ((TableId, &[&str]), Input),
    decode: impl Fn(&History, &[usize], u128, &[Cell]) -> Option<Row>,
    changes: &mut Vec<Change>,
) -> io::Result<()> {
    let columns = columns(table, names)?;
    let history = cost::storage(|| lanes::history(recording, run, table, &columns, Keys::All, first, last))?;
    for (key, steps) in &history.keys {
        for step in steps {
            let shown = match &step.cells {
                None => Shown::Gone,
                Some(cells) => decode(&history, &columns, *key, cells).map_or(Shown::Unknown, Shown::Row),
            };
            changes.push(Change { hour: step.hour, input, key: *key, shown });
        }
    }
    Ok(())
}

pub fn run_wars(recording: &Recording, run: u32) -> io::Result<RunWars> {
    let held = recording.run(run)?;
    let span = (recording, run, held.first_hour, held.last_hour());
    let mut changes = Vec::new();
    let n = |h: &History, cells: &[Cell], column: usize| number(h, cells, column);
    steps(span, (RELATIONS, Input::Relation), |h, c, _, cells| {
        Some(Row::Relation(Relation { start: n(h, cells, c[0])? as i32, instigator: n(h, cells, c[1])? as u8 }))
    }, &mut changes)?;
    steps(span, (OWNED, Input::Owned), |h, c, key, cells| {
        Some(Row::Owned(rows::WarOwnedRelationRow {
            owner: (key >> 32) as u32,
            ordinal: key as u32,
            first_country: n(h, cells, c[0])? as u32,
            second_country: n(h, cells, c[1])? as u32,
            retired: n(h, cells, c[2])? as u8,
            first_goal_count: n(h, cells, c[3])? as u32,
            second_goal_count: n(h, cells, c[4])? as u32,
        }))
    }, &mut changes)?;
    steps(span, (FACTIONS, Input::Faction), |h, c, _, cells| Some(Row::Faction(n(h, cells, c[0])? as u64)), &mut changes)?;
    steps(span, (SCORES, Input::Score), |h, c, key, cells| {
        Some(Row::Score(WarOverviewCountryRow {
            country: key as u32,
            major: n(h, cells, c[0])? as u8,
            divisions: n(h, cells, c[1])? as i32,
            factories: n(h, cells, c[2])? as i32,
            view_subject: n(h, cells, c[3])? as u8,
            score_available: n(h, cells, c[4])? as u8,
        }))
    }, &mut changes)?;
    steps(span, (RULES, Input::Rule), |h, c, _, cells| {
        let prefix = h.text(*cells.get(h.columns.binary_search(&c[0]).ok()?)?)?.to_vec();
        Some(Row::Rule(prefix, n(h, cells, c[1])? as u32))
    }, &mut changes)?;
    steps(span, (LOCAL, Input::Local), |h, c, _, cells| {
        Some(Row::Local(LocalPlayerRow { country: n(h, cells, c[0])? as u32, observer_country: n(h, cells, c[1])? as u32 }))
    }, &mut changes)?;
    steps(span, (CAPITULATIONS, Input::Capitulation), |h, c, _, cells| {
        Some(Row::Capitulation(n(h, cells, c[0])? as u8, n(h, cells, c[1])? as i32))
    }, &mut changes)?;
    changes.sort_by_key(|change| change.hour);
    let mut raws = BTreeSet::new();
    for change in &changes {
        match (&change.shown, change.input) {
            (_, Input::Relation) => raws.extend([(change.key >> 32) as u32, change.key as u32]),
            (Shown::Row(Row::Owned(row)), _) => raws.extend([row.owner, row.first_country, row.second_country]),
            (_, Input::Faction | Input::Score) => raws.extend([change.key as u32]),
            (Shown::Row(Row::Rule(_, viewpoint)), _) => raws.extend([*viewpoint]),
            (Shown::Row(Row::Local(row)), _) => raws.extend([row.country, row.observer_country]),
            (Shown::Row(Row::Capitulation(capitulated, _)), _) if *capitulated != 0 => raws.extend([change.key as u32]),
            _ => {}
        }
    }
    raws.remove(&0);
    let reads = |table| cost::storage(|| lanes::reads(recording, run, table, span.2, span.3));
    let read = both(&reads(TableId::WarRelations)?, &reads(TableId::WarOwnedRelations)?);
    let capitulations = reads(TableId::Capitulation)?;
    let (goals, tokens, states) = naming_inputs(span)?;
    Ok(RunWars { changes, raws, read, capitulations, goals, tokens, states })
}

const GOALS: (TableId, &[&str]) = (TableId::WarRelationGoals, &["resolved", "type_token", "state_count", "first_state", "country_count", "first_country"]);
const TOKENS: (TableId, &[&str]) = (TableId::LexicalTokens, &["literal"]);
const STATE_NAMES: (TableId, &[&str]) = (TableId::States, &["custom_name", "owner_name"]);

fn naming_inputs((recording, run, first, last): (&Recording, u32, i32, i32)) -> io::Result<(Stepped<Goal>, Stepped<Box<[u8]>>, Stepped<(Box<[u8]>, Box<[u8]>)>)> {
    let history = |(table, names): (TableId, &[&str]), keys: Keys<'_>| -> io::Result<(History, Vec<usize>)> {
        let columns = columns(table, names)?;
        Ok((cost::storage(|| lanes::history(recording, run, table, &columns, keys, first, last))?, columns))
    };
    let (goals, c) = history(GOALS, Keys::All)?;
    let n = |h: &History, cells: &[Cell], column: usize| number(h, cells, column);
    let goals = Stepped::of(&goals, |key| key as u32 == 0, |h, cells| {
        Some(Goal {
            resolved: n(h, cells, c[0])? as u8,
            type_token: n(h, cells, c[1])? as u32,
            state_count: n(h, cells, c[2])? as u32,
            first_state: n(h, cells, c[3])? as u32,
            country_count: n(h, cells, c[4])? as u32,
            first_country: n(h, cells, c[5])? as u32,
        })
    });
    let text = |h: &History, cells: &[Cell], column: usize| -> Option<Box<[u8]>> { Some(Box::from(h.text(*cells.get(h.columns.binary_search(&column).ok()?)?)?)) };
    let mut named: Vec<u128> = goals.values().filter(|goal| goal.resolved != 0).map(|goal| u128::from(goal.type_token)).collect();
    named.sort_unstable();
    named.dedup();
    let tokens = if named.is_empty() {
        Stepped::default()
    } else {
        let (history, c) = history(TOKENS, Keys::These(&named))?;
        Stepped::of(&history, |_| true, |h, cells| text(h, cells, c[0]))
    };
    let mut named: Vec<u128> = goals.values().filter(|goal| goal.resolved != 0 && goal.state_count != 0).map(|goal| u128::from(goal.first_state)).collect();
    named.sort_unstable();
    named.dedup();
    let states = if named.is_empty() {
        Stepped::default()
    } else {
        let (history, c) = history(STATE_NAMES, Keys::These(&named))?;
        Stepped::of(&history, |_| true, |h, cells| Some((text(h, cells, c[0])?, text(h, cells, c[1])?)))
    };
    Ok((goals, tokens, states))
}
