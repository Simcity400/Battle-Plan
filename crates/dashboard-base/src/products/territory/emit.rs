//! Territory's emit: each tag's twelve measures and its native owned-state counter as runs on
//! the family's series publication, each measure's rank by holder and by position, each tag's
//! composite runs (their first hour, the run as an event) and presence, the complete rosters,
//! the world's composite runs, each regional series' runs and the regional reads' coverage.
//!
//! A tag holds a run while its unambiguous holder holds a known value: a holder change, an
//! hour not walked and an hour after none start a segment. A value series breaks where its
//! composite run starts a segment; a series going on after an hour it held nothing breaks too,
//! as the row accumulator's did. Ranks order the known values descending, ties by tag.

use super::{
    columns,
    facts::{self, Lineage, Rates, RegionKey, RegionValues, Values},
};
use crate::products::territory_breakdown::TerritoryRegion;
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
    hash::FastMap,
};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap},
    io,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// A tag's value by channel ([`VALUE_CHANNELS`], [`NATIVE_OWNED_STATES`]), its rank by
/// column, its composite run's first hour (the run as a [`kind::RUN`] event) and its presence.
pub const VALUE: u64 = 0;
pub const RANK: u64 = 1;
pub const FACTS: u64 = 2;
pub const PRESENT: u64 = 3;
/// The family's keys: the complete rosters, the world's composite run, each regional column's
/// complete reads, each regional series' run (by index), and each column's holder by position
/// (`POSITIONS + column`).
pub const ROSTERS: u64 = 4;
pub const WORLD: u64 = 5;
pub const OBSERVED: u64 = 6;
pub const REGIONS: u64 = 7;
pub const POSITIONS: u64 = 16;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
/// Family scalars: the area unit, its refusal and the content's warnings; the regional content
/// or why it is unavailable; the counts; and each regional series' key, by index.
pub const META: u64 = 0;
pub const REGIONAL: u64 = 1;
pub const COUNTS: u64 = 2;
pub const REGION_KEYS: u64 = 1 << 20;
/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;

/// The native owned-state counter's channel, beside the measures'.
pub const NATIVE_OWNED_STATES: usize = 8;
/// Each wire column's channel; the native counter has no column.
pub const VALUE_CHANNELS: [usize; columns::COUNT] = [0, 1, 2, 3, 4, 5, 6, 7, 9, 10, 11, 12];
const CHANNELS: usize = 13;
const REGIONAL_COLUMNS: usize = 8;

/// Event kinds.
pub mod kind {
    /// `[Integer | Null; 12, Boolean starts_segment]`, on a tag's FACTS key and the WORLD key.
    pub const RUN: u64 = 0;
    /// `[Integer | Null; 6, (Null | List [List [Unsigned denominator, Unsigned numerator]…]); 2,
    /// Boolean starts_segment]`, on a regional series' key.
    pub const REGION: u64 = 1;
}

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved territory emit is not valid")
}

/// What the emit reads of the recorded content.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    pub area_unit: Option<String>,
    pub area_reason: Option<String>,
    pub warnings: Vec<String>,
}

/// The regional content: its regions and warnings, or why it is unavailable.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionalMeta {
    pub reason: Option<String>,
    pub regions: Vec<TerritoryRegion>,
    pub warnings: Vec<String>,
}

/// The content the emit publishes with its pages.
#[derive(Clone, Debug, Default)]
pub struct Content {
    pub areas: bool,
    pub meta: Meta,
    pub regional: RegionalMeta,
}

impl Content {
    /// Whether regional series are prepared.
    fn regions(&self) -> bool {
        self.regional.reason.is_none()
    }
}

/// A tag's keys.
#[derive(Clone)]
struct Keys {
    values: [KeyHandle; CHANNELS],
    ranks: [KeyHandle; columns::COUNT],
    facts: KeyHandle,
    present: KeyHandle,
}

/// What a tag held at the last complete hour.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Country {
    present: bool,
    /// Its unambiguous holder's identity and dense index.
    entry: Option<((u32, u32), u32)>,
    /// Its holder's identity, ambiguous or not.
    found: Option<(u32, u32)>,
    /// Its composite run: its first hour and values.
    run: Option<(i32, Values)>,
    #[serde(skip)]
    keys: Option<Box<Keys>>,
}

/// A column's ranking: the ranked tags in rank order with their values, each one's value by
/// tag, and each position's key.
#[derive(Default)]
struct Ranking {
    order: Vec<(u64, i64)>,
    amounts: FastMap<u64, i64>,
    positions: Vec<KeyHandle>,
}

/// A regional series.
#[derive(Clone, Serialize, Deserialize)]
struct Series {
    key: RegionKey,
    /// Its tag's actor.
    tag: Option<u64>,
    /// Its sums while it holds rows.
    values: Option<Arc<RegionValues>>,
    active: bool,
    /// Its run's first hour and values, while one covers the last complete hour.
    run: Option<(i32, Arc<RegionValues>)>,
    #[serde(skip)]
    handle: Option<KeyHandle>,
}

/// The family's keys.
#[derive(Clone, Copy)]
struct FamilyKeys {
    rosters: KeyHandle,
    world: KeyHandle,
    observed: [KeyHandle; REGIONAL_COLUMNS],
}

/// The family's emit on one branch.
pub struct Emit {
    content: Arc<Content>,
    actors: Actors,
    carry: Carry,
    labels: Vec<Arc<[u8]>>,
    countries: Vec<Country>,
    /// The last complete hour's lineage, and whether the last hour was complete.
    lineage: Option<Arc<Lineage>>,
    complete: bool,
    /// Each lineage tag's actor, each dense holder's actor and each actor's lineage tag.
    tag_actors: Vec<u64>,
    dense: Vec<u64>,
    held: Vec<Option<u32>>,
    values: Vec<Values>,
    /// The owned-state counters by canonical slot, none while they are not complete, and the
    /// actors whose holder each canonical slot is.
    counts: Option<HashMap<u32, Option<i64>>>,
    found: HashMap<u32, Vec<u64>>,
    world_values: Values,
    world_run: Option<(i32, Values)>,
    fresh: [bool; 2],
    rates: Rates,
    rankings: Vec<Ranking>,
    family: Option<FamilyKeys>,
    series: Vec<Series>,
    index: FastMap<RegionKey, u32>,
    /// Each segment-local series number's index.
    local: Vec<u32>,
    missing_states: BTreeSet<u32>,
    missing_provinces: BTreeSet<u32>,
    ambiguous: u64,
    overflowed: u64,
    incomplete: u64,
    /// Regional keys catalogued, and whether the scalars changed since the last cut.
    keyed: usize,
    stale: bool,
    last: Option<i32>,
    /// The first hour since the last cut.
    first: Option<i32>,
}

/// The order of `left` before `right`: larger first, ties by tag.
fn before(labels: &[Arc<[u8]>], left: (u64, i64), right: (u64, i64)) -> Ordering {
    right
        .1
        .cmp(&left.1)
        .then_with(|| labels[left.0 as usize].cmp(&labels[right.0 as usize]))
}

/// A composite run's values as event fields.
fn run_cells(
    writer: &mut Writer<'_>,
    values: &Values,
    starts: bool,
) -> io::Result<dashboard_series::Blob> {
    writer.cells(
        values
            .iter()
            .map(|value| value.map_or(Cell::Null, |value| Cell::Integer(i128::from(value))))
            .chain(std::iter::once(Cell::Boolean(starts))),
    )
}

/// A regional run's values as event fields.
fn region_cells(
    writer: &mut Writer<'_>,
    values: &RegionValues,
    starts: bool,
) -> io::Result<dashboard_series::Blob> {
    let mut cells: Vec<Cell> = values
        .values
        .iter()
        .map(|value| value.map_or(Cell::Null, |value| Cell::Integer(i128::from(value))))
        .collect();
    for terms in &values.states {
        cells.push(match terms {
            None => Cell::Null,
            Some(terms) => {
                let mut pairs = Vec::with_capacity(terms.len());
                for (&denominator, &numerator) in terms {
                    let pair = writer.cells([
                        Cell::Unsigned(u64::from(denominator)),
                        Cell::Unsigned(numerator),
                    ])?;
                    pairs.push(Cell::List(pair));
                }
                Cell::List(writer.cells(pairs)?)
            }
        });
    }
    cells.push(Cell::Boolean(starts));
    writer.cells(cells)
}

impl Emit {
    pub fn new(content: Arc<Content>) -> Self {
        Self {
            content,
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            labels: Vec::new(),
            countries: Vec::new(),
            lineage: None,
            complete: false,
            tag_actors: Vec::new(),
            dense: Vec::new(),
            held: Vec::new(),
            values: Vec::new(),
            counts: None,
            found: HashMap::new(),
            world_values: [None; columns::COUNT],
            world_run: None,
            fresh: [false; 2],
            rates: Rates::default(),
            rankings: (0..columns::COUNT).map(|_| Ranking::default()).collect(),
            family: None,
            series: Vec::new(),
            index: FastMap::default(),
            local: Vec::new(),
            missing_states: BTreeSet::new(),
            missing_provinces: BTreeSet::new(),
            ambiguous: 0,
            overflowed: 0,
            incomplete: 0,
            keyed: 0,
            stale: true,
            last: None,
            first: None,
        }
    }

    fn key(&mut self, writer: &mut Writer<'_>, place: Place) -> io::Result<KeyHandle> {
        self.actors
            .key(writer, place, (RunMode::Exact, 0), Some(DOMAIN))
    }

    fn family_keys(&mut self, writer: &mut Writer<'_>) -> io::Result<FamilyKeys> {
        if let Some(keys) = self.family {
            return Ok(keys);
        }
        let place = |measure, dimension| Place {
            actor: FAMILY,
            measure,
            dimension,
        };
        let rosters = self.key(writer, place(ROSTERS, None))?;
        let world = self.key(writer, place(WORLD, None))?;
        let mut observed = [0; REGIONAL_COLUMNS];
        for (column, key) in observed.iter_mut().enumerate() {
            *key = self.key(writer, place(OBSERVED, Some(column as u64)))?;
        }
        let keys = FamilyKeys {
            rosters,
            world,
            observed,
        };
        self.family = Some(keys);
        Ok(keys)
    }

    /// The tag actor named `tag`.
    fn country(&mut self, writer: &mut Writer<'_>, tag: &str, hour: i32) -> io::Result<u64> {
        if let Some(actor) = self.actors.find(tag.as_bytes()) {
            return Ok(actor);
        }
        let actor = self.actors.actor(writer, tag.as_bytes(), hour)?;
        let index = actor as usize;
        if index >= self.countries.len() {
            self.countries.resize(index + 1, Country::default());
            self.labels.resize(index + 1, Arc::from(&[][..]));
            self.held.resize(index + 1, None);
        }
        self.labels[index] = Arc::from(tag.as_bytes());
        Ok(actor)
    }

    /// `actor`'s keys.
    fn keys(&mut self, writer: &mut Writer<'_>, actor: u64) -> io::Result<Keys> {
        if let Some(keys) = &self.countries[actor as usize].keys {
            return Ok((**keys).clone());
        }
        let place = |measure, dimension| Place {
            actor,
            measure,
            dimension,
        };
        let mut values = [0; CHANNELS];
        for (channel, key) in values.iter_mut().enumerate() {
            *key = self.key(writer, place(VALUE, Some(channel as u64)))?;
        }
        let mut ranks = [0; columns::COUNT];
        for (column, key) in ranks.iter_mut().enumerate() {
            *key = self.key(writer, place(RANK, Some(column as u64)))?;
        }
        let keys = Keys {
            values,
            ranks,
            facts: self.key(writer, place(FACTS, None))?,
            present: self.key(writer, place(PRESENT, None))?,
        };
        self.countries[actor as usize].keys = Some(Box::new(keys.clone()));
        Ok(keys)
    }

    /// `key` holds `value` from `at`: a run going on after an hour it held nothing breaks,
    /// as the row accumulator's did.
    fn set(
        &mut self,
        writer: &mut Writer<'_>,
        key: KeyHandle,
        at: i32,
        value: Option<i128>,
        starts: bool,
    ) -> io::Result<()> {
        let held = self.carry.held(key);
        let starts = value.is_some() && (starts || held.is_none());
        if held == value && !starts {
            return Ok(());
        }
        self.carry.value(writer, key, at, value, starts)?;
        Ok(())
    }

    fn event(
        writer: &mut Writer<'_>,
        key: KeyHandle,
        at: i32,
        kind: u64,
        fields: dashboard_series::Blob,
    ) -> io::Result<()> {
        let sequence = writer.sequence()?;
        writer.event(Event {
            key,
            native_hour: at,
            read_hour: None,
            sequence,
            kind,
            fields,
        })
    }

    /// Takes up a lineage: each tag's actor, and each dense holder's.
    fn install(&mut self, writer: &mut Writer<'_>, lineage: &Lineage, at: i32) -> io::Result<()> {
        for held in &mut self.held {
            *held = None;
        }
        self.tag_actors.clear();
        self.dense.clear();
        self.found.clear();
        for (index, held) in lineage.tags.iter().enumerate() {
            let actor = self.country(writer, &held.tag, at)?;
            self.tag_actors.push(actor);
            self.held[actor as usize] = Some(index as u32);
            if let Some((slot, _)) = held.identity {
                self.found.entry(slot).or_default().push(actor);
            }
            if let Some(entry) = held.entry {
                let at = entry as usize;
                if self.dense.len() <= at {
                    self.dense.resize(at + 1, u64::MAX);
                }
                self.dense[at] = actor;
            }
        }
        self.values.resize(self.dense.len(), [None; columns::COUNT]);
        // Regional series of a tag name its actor.
        for series in &mut self.series {
            if series.tag.is_none()
                && let Some(tag) = &series.key.tag
            {
                series.tag = self.actors.find(tag.as_bytes());
            }
        }
        Ok(())
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("territory.emit", None);
        let at = hour.hour;
        let adjacent = match self.last {
            Some(last) if last.checked_add(1) == Some(at) => true,
            Some(last) if at <= last => {
                return Err(dashboard_core::invalid(
                    "A territory hour does not follow the hour before",
                ));
            }
            // An hour not walked ends every run.
            Some(last) => {
                self.close(writer, last)?;
                false
            }
            None => false,
        };
        self.last = Some(at);
        self.first.get_or_insert(at);
        let contiguous = adjacent && self.complete;
        let family = self.family_keys(writer)?;
        let mut complete = self.complete;
        let mut regroup = false;
        let mut land = false;
        let mut whole = false;
        let mut touched: Vec<u64> = Vec::new();
        let mut regional: Vec<u32> = Vec::new();
        if let Some(facts) = hour.facts {
            let facts::Facts {
                whole: all,
                lineage,
                values,
                world,
                rates,
                land: fresh,
                owned: counts,
                recount,
                regional: regions,
            } = *facts;
            whole = all;
            if let Some(rates) = rates {
                self.rates = rates;
            }
            if let Some(fresh) = fresh {
                land |= fresh != self.fresh;
                self.fresh = fresh;
            }
            match lineage {
                Some(Some(lineage)) => {
                    complete = true;
                    if self.lineage.as_deref() != Some(&*lineage) {
                        // A lineage read anew counts its ambiguous slots, as the row census did.
                        if recount.is_none() {
                            self.ambiguous += lineage.ambiguous as u64;
                            self.stale |= lineage.ambiguous != 0;
                        }
                        self.install(writer, &lineage, at)?;
                        self.lineage = Some(lineage);
                        regroup = true;
                    }
                }
                Some(None) => complete = false,
                None => {}
            }
            for (index, values) in values {
                let at = index as usize;
                if at >= self.values.len() {
                    return Err(dashboard_core::invalid("A territory holder has no tag"));
                }
                if self.values[at] != values {
                    self.values[at] = values;
                    touched.push(self.dense[at]);
                }
            }
            if let Some(world) = world {
                self.world_values = world;
            }
            if let Some(count) = recount {
                self.ambiguous += count as u64;
                self.stale = true;
            }
            if let Some(counts) = counts {
                self.owned(counts, &mut touched);
            }
            if let Some(regions) = regions {
                self.regional_facts(regions, &mut regional);
            }
        }
        if complete {
            self.complete_hour(
                writer,
                at,
                family,
                contiguous,
                (regroup || whole, land),
                touched,
                regional,
            )?;
        } else if self.complete || !adjacent {
            self.incomplete_hour(writer, at, family)?;
        }
        self.complete = complete;
        self.incomplete += u64::from(self.rates.incomplete);
        self.overflowed += self.rates.overflowed;
        self.stale |= self.rates.incomplete || self.rates.overflowed != 0;
        Ok(())
    }

    /// Takes up the owned-state counters, marking the tags whose holder's count changed.
    fn owned(&mut self, counters: facts::Counters, touched: &mut Vec<u64>) {
        let facts::Counters {
            complete,
            whole,
            slots,
        } = counters;
        if !complete || whole {
            self.counts = complete.then(HashMap::new);
            touched.extend(self.found.values().flatten().copied());
        }
        let Some(counts) = &mut self.counts else {
            return;
        };
        for (slot, count, held) in slots {
            if held {
                counts.insert(slot, count);
            } else {
                counts.remove(&slot);
            }
            if !whole {
                touched.extend(self.found.get(&slot).into_iter().flatten().copied());
            }
        }
    }

    /// Takes up the regional facts, marking the series whose sums changed.
    fn regional_facts(&mut self, regions: facts::Regional, touched: &mut Vec<u32>) {
        if regions.fresh {
            self.local.clear();
        }
        for (local, key) in regions.keys {
            let index = match self.index.get(&key) {
                Some(&index) => index,
                None => {
                    let index = self.series.len() as u32;
                    let tag = key
                        .tag
                        .as_ref()
                        .and_then(|tag| self.actors.find(tag.as_bytes()));
                    self.index.insert(key.clone(), index);
                    self.series.push(Series {
                        key,
                        tag,
                        values: None,
                        active: false,
                        run: None,
                        handle: None,
                    });
                    self.stale = true;
                    index
                }
            };
            let at = local as usize;
            if self.local.len() <= at {
                self.local.resize(at + 1, u32::MAX);
            }
            self.local[at] = index;
        }
        let mut listed = vec![false; if regions.whole { self.series.len() } else { 0 }];
        for (local, values) in regions.series {
            let Some(&index) = self.local.get(local as usize) else {
                continue;
            };
            let series = &mut self.series[index as usize];
            if regions.whole {
                listed[index as usize] = true;
            }
            if series.values != values {
                series.values = values;
                touched.push(index);
            }
        }
        if regions.whole {
            for (index, series) in self.series.iter_mut().enumerate() {
                if !listed[index] && series.values.take().is_some() {
                    touched.push(index as u32);
                }
            }
        }
        for id in regions.missing_states {
            self.stale |= self.missing_states.insert(id);
        }
        for id in regions.missing_provinces {
            self.stale |= self.missing_provinces.insert(id);
        }
    }

    /// The regional columns' availability.
    fn available(&self) -> [bool; REGIONAL_COLUMNS] {
        let [states, provinces] = self.fresh;
        let area = provinces && self.content.areas;
        [
            states, provinces, provinces, area, states, provinces, provinces, area,
        ]
    }

    #[allow(clippy::too_many_arguments)]
    fn complete_hour(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        family: FamilyKeys,
        contiguous: bool,
        (regroup, land): (bool, bool),
        mut touched: Vec<u64>,
        mut regional: Vec<u32>,
    ) -> io::Result<()> {
        let lineage = self
            .lineage
            .clone()
            .ok_or_else(|| dashboard_core::invalid("A complete territory hour has no countries"))?;
        let every = !contiguous || regroup;
        if every {
            touched = (0..self.countries.len() as u64).collect();
        }
        touched.sort_unstable();
        touched.dedup();
        if self.content.regions() {
            let available = self.available();
            for (column, known) in available.into_iter().enumerate() {
                self.set(
                    writer,
                    family.observed[column],
                    at,
                    known.then_some(1),
                    false,
                )?;
            }
            if every || land {
                regional = (0..self.series.len() as u32)
                    .filter(|&index| {
                        let series = &self.series[index as usize];
                        series.active || series.values.is_some()
                    })
                    .collect();
            } else {
                regional.extend(
                    self.series
                        .iter()
                        .enumerate()
                        .filter(|(_, series)| series.active && series.key.unknown)
                        .map(|(index, _)| index as u32),
                );
            }
            regional.sort_unstable();
            regional.dedup();
            for index in regional {
                self.region(writer, at, index, contiguous, &lineage, available)?;
            }
        }
        let mut ranked: Vec<Vec<(u64, Option<i64>)>> = vec![Vec::new(); columns::COUNT];
        for actor in touched {
            self.tag(writer, at, actor, contiguous, &lineage, &mut ranked)?;
        }
        for (column, moved) in ranked.into_iter().enumerate() {
            if !moved.is_empty() {
                self.rank(writer, at, column, moved)?;
            }
        }
        // The world's composite run.
        let world = self.world_values;
        let continues = contiguous && self.world_run.is_some_and(|(_, held)| held == world);
        if !continues {
            let fields = run_cells(writer, &world, !contiguous)?;
            Self::event(writer, family.world, at, kind::RUN, fields)?;
            self.set(writer, family.world, at, Some(i128::from(at)), !contiguous)?;
            self.world_run = Some((at, world));
        }
        self.set(writer, family.rosters, at, Some(1), false)
    }

    /// A tag at a complete hour.
    fn tag(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        actor: u64,
        contiguous: bool,
        lineage: &Lineage,
        ranked: &mut [Vec<(u64, Option<i64>)>],
    ) -> io::Result<()> {
        let index = actor as usize;
        let held = self.held[index].map(|at| &lineage.tags[at as usize]);
        let country = &self.countries[index];
        if held.is_none() && !country.present && country.run.is_none() && country.found.is_none() {
            return Ok(());
        }
        let keys = self.keys(writer, actor)?;
        let entry = held.and_then(|held| Some((held.identity?, held.entry?)));
        let found = held.and_then(|held| held.identity);
        let country = &self.countries[index];
        let continues = contiguous
            && entry.is_some()
            && country.entry.map(|(identity, _)| identity) == entry.map(|(identity, _)| identity);
        let values = entry
            .map(|(_, dense)| self.values[dense as usize])
            .filter(|values| values.iter().any(Option::is_some));
        let mut started = None;
        let run = match values {
            Some(values) => {
                if continues && country.run.is_some_and(|(_, held)| held == values) {
                    country.run
                } else {
                    let starts = !continues || country.run.is_none();
                    started = Some(starts);
                    Some((at, values))
                }
            }
            None => None,
        };
        let had_run = country.run.is_some();
        if let Some(starts) = started {
            let values = run.expect("started run").1;
            let fields = run_cells(writer, &values, starts)?;
            Self::event(writer, keys.facts, at, kind::RUN, fields)?;
            self.set(writer, keys.facts, at, Some(i128::from(at)), starts)?;
        } else if run.is_none() && had_run {
            self.set(writer, keys.facts, at, None, false)?;
        }
        let starts = started.unwrap_or(false);
        for column in 0..columns::COUNT {
            let amount = run.and_then(|(_, values)| values[column]);
            let key = keys.values[VALUE_CHANNELS[column]];
            if starts || self.carry.held(key) != amount.map(i128::from) {
                self.set(writer, key, at, amount.map(i128::from), starts)?;
            }
            let rank = self.rankings[column].amounts.get(&actor).copied();
            if rank != amount {
                ranked[column].push((actor, amount));
            }
        }
        self.set(writer, keys.present, at, held.is_some().then_some(1), false)?;
        // The native owned-state counter of its holder, ambiguous or not.
        let country = &self.countries[index];
        let streak = found.is_some() && !(contiguous && country.found == found);
        let amount = found.and_then(|(slot, _)| {
            self.counts
                .as_ref()
                .and_then(|counts| counts.get(&slot).copied().flatten())
        });
        let key = keys.values[NATIVE_OWNED_STATES];
        if streak || self.carry.held(key) != amount.map(i128::from) {
            self.set(writer, key, at, amount.map(i128::from), streak)?;
        }
        let country = &mut self.countries[index];
        country.present = held.is_some();
        country.entry = entry;
        country.found = found;
        country.run = run;
        Ok(())
    }

    /// A regional series at a complete hour.
    fn region(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        index: u32,
        contiguous: bool,
        lineage: &Lineage,
        available: [bool; REGIONAL_COLUMNS],
    ) -> io::Result<()> {
        let key = match self.series[index as usize].handle {
            Some(key) => key,
            None => {
                let key = self.key(
                    writer,
                    Place {
                        actor: FAMILY,
                        measure: REGIONS,
                        dimension: Some(u64::from(index)),
                    },
                )?;
                self.series[index as usize].handle = Some(key);
                key
            }
        };
        let series = &self.series[index as usize];
        let now = series
            .tag
            .and_then(|actor| self.held[actor as usize])
            .map(|at| &lineage.tags[at as usize]);
        // A series without a tag, an unknown holder's or the unassigned, stays open.
        let present = series.key.tag.is_none() || now.is_some_and(|held| held.linked);
        let rows = series.values.is_some();
        if !rows && !present {
            if series.active {
                let held = series.run.is_some();
                let series = &mut self.series[index as usize];
                series.active = false;
                series.run = None;
                if held {
                    self.set(writer, key, at, None, false)?;
                }
            }
            return Ok(());
        }
        if !rows && !series.active {
            return Ok(());
        }
        let values = series
            .values
            .clone()
            .unwrap_or_else(|| Arc::new(RegionValues::zero(available)));
        let same = series.key.tag.is_none()
            || series.tag.is_some_and(|actor| {
                let identity = now.and_then(|held| held.identity);
                identity.is_some() && self.countries[actor as usize].found == identity
            });
        let continues = !series.key.unknown && contiguous && same;
        if series.active
            && continues
            && series
                .run
                .as_ref()
                .is_some_and(|(_, held)| **held == *values)
        {
            return Ok(());
        }
        let starts = !continues || series.run.is_none();
        let fields = region_cells(writer, &values, starts)?;
        Self::event(writer, key, at, kind::REGION, fields)?;
        self.set(writer, key, at, Some(i128::from(at)), starts)?;
        let series = &mut self.series[index as usize];
        series.run = Some((at, values));
        series.active = true;
        Ok(())
    }

    /// The column's position key at `position`.
    fn position(
        &mut self,
        writer: &mut Writer<'_>,
        column: usize,
        position: usize,
    ) -> io::Result<KeyHandle> {
        if let Some(&key) = self.rankings[column].positions.get(position) {
            return Ok(key);
        }
        let key = self.key(
            writer,
            Place {
                actor: FAMILY,
                measure: POSITIONS + column as u64,
                dimension: Some(position as u64),
            },
        )?;
        self.rankings[column].positions.push(key);
        Ok(key)
    }

    /// Moves the `moved` tags in `column`'s ranking to their values' places; the positions and
    /// the ranks that changed go on from `at`.
    fn rank(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        column: usize,
        moved: Vec<(u64, Option<i64>)>,
    ) -> io::Result<()> {
        let (mut leaving, mut entering) = (Vec::new(), Vec::new());
        let ranking = &mut self.rankings[column];
        for (actor, amount) in moved {
            let held = ranking.amounts.get(&actor).copied();
            if held == amount {
                continue;
            }
            leaving.extend(held.map(|held| (actor, held)));
            entering.extend(amount.map(|amount| (actor, amount)));
            match amount {
                Some(amount) => ranking.amounts.insert(actor, amount),
                None => ranking.amounts.remove(&actor),
            };
        }
        if leaving.is_empty() && entering.is_empty() {
            return Ok(());
        }
        let labels = &self.labels;
        let order = |left: &(u64, i64), right: &(u64, i64)| before(labels, *left, *right);
        leaving.sort_unstable_by(order);
        entering.sort_unstable_by(order);
        let misplaced = || io::Error::other("The territory ranking lost its order");
        let mut low = ranking.order.len();
        if let Some(first) = leaving.first() {
            low = ranking
                .order
                .binary_search_by(|held| order(held, first))
                .map_err(|_| misplaced())?;
        }
        if let Some(first) = entering.first() {
            let Err(place) = ranking.order.binary_search_by(|held| order(held, first)) else {
                return Err(misplaced());
            };
            low = low.min(place);
        }
        let old = ranking.order.split_off(low);
        let (mut index, mut leaving_rest, mut entering_rest) = (0, &leaving[..], &entering[..]);
        while !leaving_rest.is_empty() || !entering_rest.is_empty() {
            let held = old.get(index);
            if held.is_some() && held == leaving_rest.first() {
                (index, leaving_rest) = (index + 1, &leaving_rest[1..]);
            } else if let Some(&entry) = entering_rest.first()
                && held.is_none_or(|held| order(held, &entry) == Ordering::Greater)
            {
                ranking.order.push(entry);
                entering_rest = &entering_rest[1..];
            } else {
                ranking.order.push(*held.ok_or_else(misplaced)?);
                index += 1;
            }
        }
        let merged = ranking.order.len();
        ranking.order.extend_from_slice(&old[index..]);
        // Past the merge, a tag keeps its place unless the ranking grew or shrank.
        let end = if ranking.order.len() == low + old.len() {
            merged
        } else {
            ranking.order.len().max(low + old.len())
        };
        let mut placed = Vec::new();
        for position in low..end {
            let was = old.get(position - low).map(|&(actor, _)| actor);
            let now = self.rankings[column]
                .order
                .get(position)
                .map(|&(actor, _)| actor);
            if was == now {
                continue;
            }
            let key = self.position(writer, column, position)?;
            if self.carry.held(key) != now.map(i128::from) {
                self.carry
                    .value(writer, key, at, now.map(i128::from), false)?;
            }
            if let Some(actor) = now {
                placed.push((actor, position));
            }
        }
        for (actor, position) in placed {
            let key = self.keys(writer, actor)?.ranks[column];
            let rank = Some(position as i128 + 1);
            if self.carry.held(key) != rank {
                self.carry.value(writer, key, at, rank, false)?;
            }
        }
        for &(actor, _) in &leaving {
            if self.rankings[column].amounts.contains_key(&actor) {
                continue;
            }
            let key = self.keys(writer, actor)?.ranks[column];
            if self.carry.held(key).is_some() {
                self.carry.value(writer, key, at, None, false)?;
            }
        }
        Ok(())
    }

    /// An hour whose countries are not complete: every tag, run and roster ends.
    fn incomplete_hour(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        family: FamilyKeys,
    ) -> io::Result<()> {
        for actor in 0..self.countries.len() as u64 {
            let country = &self.countries[actor as usize];
            if !country.present && country.run.is_none() && country.found.is_none() {
                continue;
            }
            let keys = self.keys(writer, actor)?;
            for key in keys
                .values
                .iter()
                .chain(&keys.ranks)
                .chain([&keys.facts, &keys.present])
            {
                if self.carry.held(*key).is_some() {
                    self.carry.value(writer, *key, at, None, false)?;
                }
            }
            let country = &mut self.countries[actor as usize];
            country.present = false;
            country.entry = None;
            country.found = None;
            country.run = None;
        }
        for column in 0..columns::COUNT {
            let positions = self.rankings[column].positions.clone();
            for key in positions {
                if self.carry.held(key).is_some() {
                    self.carry.value(writer, key, at, None, false)?;
                }
            }
            let ranking = &mut self.rankings[column];
            ranking.order.clear();
            ranking.amounts.clear();
        }
        for index in 0..self.series.len() {
            let series = &mut self.series[index];
            if series.run.take().is_some()
                && let Some(key) = series.handle
                && self.carry.held(key).is_some()
            {
                self.carry.value(writer, key, at, None, false)?;
            }
        }
        if self.content.regions() {
            for key in family.observed {
                self.set(writer, key, at, None, false)?;
            }
        }
        self.world_run = None;
        self.set(writer, family.world, at, None, false)?;
        self.set(writer, family.rosters, at, None, false)
    }

    /// Ends every run after `last`, the hour before one not walked.
    fn close(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.close(writer, last)?;
        for country in &mut self.countries {
            country.present = false;
            country.entry = None;
            country.found = None;
            country.run = None;
        }
        for ranking in &mut self.rankings {
            ranking.order.clear();
            ranking.amounts.clear();
        }
        for series in &mut self.series {
            series.run = None;
        }
        self.world_run = None;
        self.complete = false;
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, the hours since the last cut as known, and the scalars that changed.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)?;
        if let Some(first) = self.first.take() {
            writer.domain(
                DOMAIN,
                Evidence {
                    span: Span::new(first, last)?,
                    known: Knownness::Known,
                    supplying_read: None,
                    holder: None,
                    identity_first: None,
                },
            )?;
        }
        if self.stale {
            let meta = serde_json::to_vec(&self.content.meta)?;
            let regional = serde_json::to_vec(&self.content.regional)?;
            let scalars = [
                (META, Scalar::Text(meta.into_boxed_slice())),
                (REGIONAL, Scalar::Text(regional.into_boxed_slice())),
                (
                    COUNTS,
                    Scalar::List(
                        [
                            self.ambiguous,
                            self.overflowed,
                            self.incomplete,
                            self.missing_states.len() as u64,
                            self.missing_provinces.len() as u64,
                        ]
                        .map(Scalar::Unsigned)
                        .to_vec(),
                    ),
                ),
            ];
            for (id, scalar) in scalars {
                let cell = family::cell(writer, &scalar)?;
                writer.catalogue(id, cell)?;
            }
            for index in self.keyed..self.series.len() {
                let key = &self.series[index].key;
                let json = serde_json::to_vec(&(&key.tag, key.region, &key.missing, key.unknown))?;
                let cell = family::cell(writer, &Scalar::Text(json.into_boxed_slice()))?;
                writer.catalogue(REGION_KEYS + index as u64, cell)?;
            }
            self.keyed = self.series.len();
            self.stale = false;
        }
        Ok(())
    }

    /// What the emit goes on with after a cut.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let saved = Saved {
            countries: self.countries.clone(),
            lineage: self.lineage.as_deref().cloned(),
            complete: self.complete,
            values: self.values.clone(),
            counts: self.counts.clone(),
            world_values: self.world_values,
            world_run: self.world_run,
            fresh: self.fresh,
            rates: self.rates,
            series: self.series.clone(),
            local: self.local.clone(),
            missing_states: self.missing_states.clone(),
            missing_provinces: self.missing_provinces.clone(),
            ambiguous: self.ambiguous,
            overflowed: self.overflowed,
            incomplete: self.incomplete,
            last: self.last,
        };
        let json = serde_json::to_vec(&saved)?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(content: Arc<Content>, directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut emit = Self::new(content);
        emit.stale = false;
        emit.actors = Actors::resume(SCHEMA, directory)?;
        emit.labels = directory
            .actors
            .iter()
            .map(|actor| Arc::from(&actor.name[..]))
            .collect();
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        if saved.countries.len() > directory.actors.len() {
            return Err(invalid());
        }
        emit.countries = saved.countries;
        emit.countries
            .resize(directory.actors.len(), Country::default());
        emit.held = vec![None; directory.actors.len()];
        emit.complete = saved.complete;
        emit.counts = saved.counts;
        emit.world_values = saved.world_values;
        emit.world_run = saved.world_run;
        emit.fresh = saved.fresh;
        emit.rates = saved.rates;
        emit.series = saved.series;
        emit.index = emit
            .series
            .iter()
            .enumerate()
            .map(|(index, series)| (series.key.clone(), index as u32))
            .collect();
        emit.keyed = emit.series.len();
        emit.local = saved.local;
        emit.missing_states = saved.missing_states;
        emit.missing_provinces = saved.missing_provinces;
        emit.ambiguous = saved.ambiguous;
        emit.overflowed = saved.overflowed;
        emit.incomplete = saved.incomplete;
        emit.last = saved.last;
        for (index, series) in emit.series.iter_mut().enumerate() {
            series.handle = directory.key(FAMILY, REGIONS, Some(index as u64));
        }
        if let Some(lineage) = saved.lineage {
            for (index, held) in lineage.tags.iter().enumerate() {
                let actor = emit.actors.find(held.tag.as_bytes()).ok_or_else(invalid)?;
                emit.tag_actors.push(actor);
                emit.held[actor as usize] = Some(index as u32);
                if let Some((slot, _)) = held.identity {
                    emit.found.entry(slot).or_default().push(actor);
                }
                if let Some(entry) = held.entry {
                    let at = entry as usize;
                    if emit.dense.len() <= at {
                        emit.dense.resize(at + 1, u64::MAX);
                    }
                    emit.dense[at] = actor;
                }
            }
            emit.lineage = Some(Arc::new(lineage));
        }
        if saved.values.len() != emit.dense.len() {
            return Err(invalid());
        }
        emit.values = saved.values;
        for column in 0..columns::COUNT {
            emit.rankings[column].positions = directory
                .dimensions(FAMILY, POSITIONS + column as u64)
                .iter()
                .enumerate()
                .map(|(at, &(position, key))| {
                    (position == at as u64).then_some(key).ok_or_else(invalid)
                })
                .collect::<io::Result<_>>()?;
            // The ranking holds every tag with a known value, in rank order.
            let mut known: Vec<(u64, i64)> = emit
                .countries
                .iter()
                .enumerate()
                .filter_map(|(actor, country)| {
                    Some((
                        actor as u64,
                        country.run.and_then(|(_, values)| values[column])?,
                    ))
                })
                .collect();
            let labels = &emit.labels;
            known.sort_unstable_by(|&left, &right| before(labels, left, right));
            emit.rankings[column].amounts = known.iter().copied().collect();
            emit.rankings[column].order = known;
        }
        Ok(emit)
    }
}

#[derive(Serialize, Deserialize)]
struct Saved {
    countries: Vec<Country>,
    lineage: Option<Lineage>,
    complete: bool,
    values: Vec<Values>,
    counts: Option<HashMap<u32, Option<i64>>>,
    world_values: Values,
    world_run: Option<(i32, Values)>,
    fresh: [bool; 2],
    rates: Rates,
    series: Vec<Series>,
    local: Vec<u32>,
    missing_states: BTreeSet<u32>,
    missing_provinces: BTreeSet<u32>,
    ambiguous: u64,
    overflowed: u64,
    incomplete: u64,
    last: Option<i32>,
}
