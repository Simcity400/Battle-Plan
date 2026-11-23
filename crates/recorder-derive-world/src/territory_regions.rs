
use recorder_derive_framework::framework::{Columns, Definition, Hour, Output, Read, Rows, Uses, Value, Watch, Writer};
use replay_columnar::{
    leaf::Writer as Bytes,
    state::Kind,
    store::{TableView, Value as Held},
};
use replay_data::rows::{TableId, TerritoryRegionTotalRow};
use std::{
    collections::{HashMap, HashSet},
    io,
};

#[cfg(test)]
mod tests;

const STATE_OWNER: usize = 0;
const STATE_CONTROLLER: usize = 1;
const PROVINCE_STATE: usize = 0;
const PROVINCE_OWNER: usize = 1;
const PROVINCE_CONTROLLER: usize = 2;
const PROVINCE_POINTS: usize = 3;
const STATE_COLUMNS: &[usize] = &[STATE_OWNER, STATE_CONTROLLER];
const PROVINCE_COLUMNS: &[usize] = &[PROVINCE_STATE, PROVINCE_OWNER, PROVINCE_CONTROLLER, PROVINCE_POINTS];

const READS: &[Read] = &[
    Read { table: TableId::States, columns: Columns::These(STATE_COLUMNS), watch: Watch::Changed, required: true },
    Read { table: TableId::Provinces, columns: Columns::These(PROVINCE_COLUMNS), watch: Watch::Changed, required: true },
    Read { table: TableId::CountryLinks, columns: Columns::These(&[0]), watch: Watch::Changed, required: true },
];

const USES: &[Uses] = &[
    Uses { definition: Definition::StrategicRegions, required: true },
    Uses { definition: Definition::ProvinceAreas, required: false },
];

const NO_COUNTRY: u32 = TerritoryRegionTotalRow::NO_COUNTRY;
const NO_REGION: u32 = TerritoryRegionTotalRow::NO_REGION;
const RULES: usize = 2;

type Key = (u32, u32);

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Province {
    state: u32,
    owner: u32,
    controller: u32,
    points: i64,
    region: u32,
    area: Option<i64>,
}

impl Province {
    fn raws(&self) -> [u32; RULES] {
        [self.controller, self.owner]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct State {
    owner: u32,
    controller: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Holding {
    provinces: i64,
    points: i64,
    area: i64,
    unmeasured: i64,
    states: Vec<(u32, i64)>,
}

impl Holding {
    fn holds(&self) -> bool {
        self.provinces != 0 || !self.states.is_empty()
    }

    fn add_share(&mut self, numerator: i64, denominator: u32) {
        match self.states.binary_search_by_key(&denominator, |&(held, _)| held) {
            Ok(at) => {
                self.states[at].1 += numerator;
                if self.states[at].1 == 0 {
                    self.states.remove(at);
                }
            }
            Err(at) => self.states.insert(at, (denominator, numerator)),
        }
    }
}

fn gcd(mut a: i128, mut b: i128) -> i128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.abs()
}

fn states_sum(parts: &[(u32, i64)]) -> Option<(i64, i64)> {
    let (mut numerator, mut denominator) = (0_i128, 1_i128);
    for &(over, part) in parts {
        let (over, part) = (i128::from(over), i128::from(part));
        let common = over / gcd(denominator, over);
        numerator = numerator.checked_mul(common)?.checked_add(part.checked_mul(denominator / gcd(denominator, over))?)?;
        denominator = denominator.checked_mul(common)?;
        let shared = gcd(numerator, denominator).max(1);
        (numerator, denominator) = (numerator / shared, denominator / shared);
    }
    Some((i64::try_from(numerator).ok()?, i64::try_from(denominator).ok()?))
}

fn cell(table: &TableView<'_>, column: usize, slot: u32) -> Option<i64> {
    match (table.kinds().get(column)?, table.value(column, slot)) {
        (Kind::U32 | Kind::I32, Held::Number(bits)) => Some(bits as i64),
        _ => None,
    }
}

#[derive(Default)]
pub struct TerritoryRegions {
    rebuild: bool,
    links: HashMap<u32, u32>,
    provinces: HashMap<u32, Province>,
    states: HashMap<u32, State>,
    rosters: HashMap<u32, Vec<(u32, u32)>>,
    holdings: HashMap<Key, [Holding; RULES]>,
    unresolved: [i64; RULES],
    held: HashSet<Key>,
    unmapped: Option<String>,
}

impl TerritoryRegions {
    fn holder(&self, raw: u32) -> Result<u32, ()> {
        match raw {
            0 => Ok(NO_COUNTRY),
            raw => self.links.get(&raw).copied().ok_or(()),
        }
    }

    fn credit(&mut self, raws: [u32; RULES], region: u32, sign: i64, dirty: &mut HashSet<Key>, add: impl Fn(&mut Holding)) {
        for (rule, raw) in raws.into_iter().enumerate() {
            match self.holder(raw) {
                Ok(holder) => {
                    add(&mut self.holdings.entry((holder, region)).or_default()[rule]);
                    dirty.insert((holder, region));
                }
                Err(()) => self.unresolved[rule] += sign,
            }
        }
    }

    fn count_province(&mut self, province: Province, sign: i64, dirty: &mut HashSet<Key>) {
        self.credit(province.raws(), province.region, sign, dirty, |holding| {
            holding.provinces += sign;
            holding.points += sign * province.points;
            match province.area {
                Some(area) => holding.area += sign * area,
                None => holding.unmeasured += sign,
            }
        });
    }

    fn count_state(&mut self, id: u32, state: State, sign: i64, dirty: &mut HashSet<Key>) {
        let parts = match self.rosters.get(&id) {
            Some(parts) => parts.clone(),
            None => vec![(NO_REGION, 1)],
        };
        let denominator: u32 = parts.iter().map(|&(_, count)| count).sum();
        let raws = [state.controller, state.owner];
        let resolved: [Result<u32, ()>; RULES] = raws.map(|raw| self.holder(raw));
        for (rule, holder) in resolved.into_iter().enumerate() {
            let Ok(holder) = holder else {
                self.unresolved[rule] += sign;
                continue;
            };
            for &(region, count) in &parts {
                self.holdings.entry((holder, region)).or_default()[rule].add_share(sign * i64::from(count), denominator);
                dirty.insert((holder, region));
            }
        }
    }

    fn province(table: &TableView<'_>, slot: u32, id: u32, hour: &Hour<'_>) -> Province {
        let definitions = hour.definitions();
        Province {
            state: cell(table, PROVINCE_STATE, slot).unwrap_or(0) as u32,
            owner: cell(table, PROVINCE_OWNER, slot).unwrap_or(0) as u32,
            controller: cell(table, PROVINCE_CONTROLLER, slot).unwrap_or(0) as u32,
            points: cell(table, PROVINCE_POINTS, slot).unwrap_or(0),
            region: definitions.regions().ok().and_then(|regions| regions.of(id)).unwrap_or(NO_REGION),
            area: definitions.areas().ok().and_then(|areas| areas.get(id as usize).copied()),
        }
    }

    fn state(table: &TableView<'_>, slot: u32) -> State {
        State { owner: cell(table, STATE_OWNER, slot).unwrap_or(0) as u32, controller: cell(table, STATE_CONTROLLER, slot).unwrap_or(0) as u32 }
    }

    fn rebuild(&mut self, hour: &Hour<'_>, out: &Rows<'_>) {
        self.unmapped = hour.definitions().areas().err().map(str::to_owned);
        self.links.clear();
        let links = hour.table(TableId::CountryLinks);
        for slot in links.rows() {
            let (Some(key), Some(canonical)) = (links.key(slot), cell(&links, 0, slot)) else { continue };
            self.links.insert(key as u32, canonical as u32);
        }
        let provinces = hour.table(TableId::Provinces);
        self.provinces = provinces.rows().filter_map(|slot| provinces.key(slot).map(|key| (key as u32, Self::province(&provinces, slot, key as u32, hour)))).collect();
        let mut rosters: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
        for province in self.provinces.values().filter(|province| province.state != 0) {
            let parts = rosters.entry(province.state).or_default();
            match parts.binary_search_by_key(&province.region, |&(region, _)| region) {
                Ok(at) => parts[at].1 += 1,
                Err(at) => parts.insert(at, (province.region, 1)),
            }
        }
        self.rosters = rosters;
        let states = hour.table(TableId::States);
        self.states = states.rows().filter_map(|slot| states.key(slot).map(|key| (key as u32, Self::state(&states, slot)))).collect();
        self.holdings.clear();
        self.unresolved = [0; RULES];
        let mut ignored = HashSet::new();
        let places: Vec<Province> = self.provinces.values().copied().collect();
        for province in places {
            self.count_province(province, 1, &mut ignored);
        }
        let states: Vec<(u32, State)> = self.states.iter().map(|(&id, &state)| (id, state)).collect();
        for (id, state) in states {
            self.count_state(id, state, 1, &mut ignored);
        }
        let derived = out.table(TableId::TerritoryRegionTotals);
        self.held = derived.rows().filter_map(|slot| derived.key(slot)).map(|key| ((key >> 32) as u32, key as u32)).collect();
    }

    fn apply(&mut self, hour: &Hour<'_>, dirty: &mut HashSet<Key>) -> bool {
        if hour.deleted_keys(TableId::Provinces).next().is_some() {
            return false;
        }
        let provinces = hour.table(TableId::Provinces);
        let mut moved: Vec<(u32, Province, Province)> = Vec::new();
        for slot in hour.written_rows(TableId::Provinces, Columns::These(PROVINCE_COLUMNS)) {
            let Some(key) = provinces.key(slot) else { continue };
            let (id, now) = (key as u32, Self::province(&provinces, slot, key as u32, hour));
            match self.provinces.get(&id) {
                Some(before) if before.state == now.state => {
                    if *before != now {
                        moved.push((id, *before, now));
                    }
                }
                _ => return false,
            }
        }
        for (id, before, now) in moved {
            self.count_province(before, -1, dirty);
            self.count_province(now, 1, dirty);
            self.provinces.insert(id, now);
        }
        let states = hour.table(TableId::States);
        let mut edited: HashMap<u32, Option<State>> = hour.deleted_keys(TableId::States).map(|key| (key as u32, None)).collect();
        for slot in hour.written_rows(TableId::States, Columns::These(STATE_COLUMNS)) {
            if let Some(key) = states.key(slot) {
                edited.insert(key as u32, Some(Self::state(&states, slot)));
            }
        }
        for (id, now) in edited {
            let before = self.states.get(&id).copied();
            if before == now {
                continue;
            }
            if let Some(before) = before {
                self.count_state(id, before, -1, dirty);
            }
            match now {
                Some(now) => {
                    self.count_state(id, now, 1, dirty);
                    self.states.insert(id, now);
                }
                None => {
                    self.states.remove(&id);
                }
            }
        }
        true
    }
}

impl TerritoryRegions {
    fn cells(&self, key: Key) -> Vec<Value> {
        let held = self.holdings.get(&key);
        let value = |known: bool, value: i64| if known { Value::I64(value) } else { Value::Unrecorded };
        let mut cells = Vec::with_capacity(RULES * 5);
        for rule in 0..RULES {
            let empty = Holding::default();
            let holding = held.map_or(&empty, |held| &held[rule]);
            let known = self.unresolved[rule] == 0;
            cells.push(value(known, holding.provinces));
            cells.push(value(known, holding.points));
            cells.push(value(known && self.unmapped.is_none() && holding.unmeasured == 0, holding.area));
            match states_sum(&holding.states).filter(|_| known) {
                Some((numerator, denominator)) => cells.extend([Value::I64(numerator), Value::I64(denominator)]),
                None => cells.extend([Value::Unrecorded, Value::Unrecorded]),
            }
        }
        cells
    }

    fn kept(&self, key: Key) -> bool {
        self.holdings.get(&key).is_some_and(|held| held.iter().any(Holding::holds))
    }

    fn write(&mut self, dirty: HashSet<Key>, out: &mut Rows) {
        for key in dirty {
            let row = TerritoryRegionTotalRow::key_of(key.0, key.1);
            if self.kept(key) {
                out.set(TableId::TerritoryRegionTotals, row, self.cells(key));
                self.held.insert(key);
            } else {
                self.holdings.remove(&key);
                if self.held.remove(&key) {
                    out.delete(TableId::TerritoryRegionTotals, row);
                }
            }
        }
    }
}

impl Writer for TerritoryRegions {
    fn name(&self) -> &'static str {
        "TerritoryRegions"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::TerritoryRegionTotals)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn definitions(&self) -> &'static [Uses] {
        USES
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let unresolved = self.unresolved;
        let mut dirty = HashSet::new();
        let whole = hour.fresh || self.rebuild || hour.wrote(TableId::CountryLinks, Columns::These(&[0]));
        if whole || !self.apply(hour, &mut dirty) {
            self.rebuild(hour, out);
            self.rebuild = false;
            dirty = self.holdings.keys().copied().chain(self.held.iter().copied()).collect();
        } else if self.unresolved != unresolved {
            dirty.extend(self.held.iter().copied());
        }
        self.write(dirty, out);
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("TerritoryRegions' state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}
