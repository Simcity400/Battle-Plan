
use viewer_layer_core::{layers::{Carried, Settling, Inputs, Layer, List, Num, Published, Scope, Scoped, Sight, View}, touched::Touched};
use super::land_markers::{bytes_of, held, recorded, u64s};
use viewer_layer_core::playhead_host::mirror::{Mirror, Mirrors};
use playhead::Step;
use replay_columnar::manifest::Status;
use replay_data::{
    Hour,
    rows::{Recorded, TableId},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use viewer_counters_naval::bases::{AirBaseCount, BaseFacts, BaseValues, RadarCircle, RadarOutcome, RegistrationFacts, WingCensus, base_count, controlled_of, merge_count, radar_notes_of, site_kind, wing_base_of, wing_country};
use viewer_foundation::perspective::{Choice, Vision};

fn whole(mirror: Option<&Mirror>) -> Option<&Mirror> {
    mirror.filter(|mirror| mirror.status() == Some(Status::Complete))
}

fn row(mirror: Option<&Mirror>, key: u128) -> Option<(&Mirror, u32)> {
    let mirror = held(mirror)?;
    Some((mirror, mirror.slot(key)?))
}

fn keyed(mirror: Option<&Mirror>) -> impl Iterator<Item = (&Mirror, u128, u32)> {
    held(mirror).into_iter().flat_map(|mirror| mirror.rows().filter_map(move |slot| Some((mirror, mirror.key(slot)?, slot))))
}

fn holds(status: Status) -> bool {
    matches!(status, Status::Complete | Status::Truncated { .. })
}

#[derive(Clone, Copy)]
struct Columns {
    controller: Num<u32>,
    base_state: Num<u32>,
    base_kind: Num<i32>,
    base_capacity: Num<i32>,
    demilitarized: Num<u8>,
    wing_owner: Num<u32>,
    country_override: Num<i32>,
    air_base: Num<u64>,
    base_override: Num<u64>,
    planes: Num<i32>,
    capacity_raw: Num<i64>,
    registered: List,
    radar_state: Num<u32>,
    faction: Num<u64>,
    subjects: List,
    overlord: Num<i32>,
    state_controller: Num<u32>,
    link_slot: Num<u32>,
}

const TABLES: [TableId; 10] = [
    TableId::Provinces,
    TableId::AirBases,
    TableId::StateFlags,
    TableId::AirWings,
    TableId::AirBaseRegistrations,
    TableId::RadarCoverage,
    TableId::FactionMembers,
    TableId::CountryAppearance,
    TableId::States,
    TableId::CountryLinks,
];

const PER_LOOK: usize = 8;

#[derive(Clone, Copy)]
struct WingRow {
    base: u64,
    country: u32,
    planes: i32,
}

#[derive(Default)]
struct Aggregate {
    standing: i64,
    located: Vec<(u32, u32)>,
    registered: Vec<Registered>,
}

#[derive(Clone, Copy)]
struct Registered {
    slot: u32,
    listed: u32,
    positive: u32,
    missing: u32,
}

fn take_from<K: std::hash::Hash + Eq + Ord, T: PartialEq>(map: &mut HashMap<K, Vec<T>>, key: K, item: &T) {
    if let std::collections::hash_map::Entry::Occupied(mut held) = map.entry(key) {
        if let Some(at) = held.get().iter().position(|held| held == item) {
            held.get_mut().swap_remove(at);
        }
        if held.get().is_empty() {
            held.remove();
        }
    }
}

#[derive(Default)]
struct Index {
    placed: HashMap<u64, (u32, i32)>,
    wings: HashMap<u64, WingRow>,
    registrations: HashMap<u128, Vec<u64>>,
    listing: HashMap<u64, Vec<u128>>,
    bases: HashMap<u64, Aggregate>,
    groups: HashMap<u128, (u32, u32)>,
    hosts: BTreeMap<u32, Vec<u32>>,
    stationed: HashMap<u32, Vec<u64>>,
    hosting: HashMap<u32, Vec<u32>>,
    controllers: HashMap<u32, u32>,
    controls: HashMap<u32, Vec<u32>>,
}

impl Index {
    fn build(mirrors: &Mirrors, columns: Columns) -> Self {
        let mut index = Self::default();
        let mut touched = BTreeSet::new();
        for (bases, key, slot) in keyed(mirrors.get(TableId::AirBases)) {
            if let Some(placed) = Self::placed_of(bases, slot, columns) {
                index.placed.insert(key as u64, placed);
            }
        }
        for (wings, key, slot) in keyed(mirrors.get(TableId::AirWings)) {
            index.set_wing(key as u64, Some(Self::wing_of(wings, slot, columns)), &mut touched);
        }
        for (rows, key, slot) in keyed(mirrors.get(TableId::AirBaseRegistrations)) {
            index.set_registration(key, Some(u64s(bytes_of(rows.list(columns.registered, slot)))), &mut touched);
        }
        let mut states = BTreeSet::new();
        for (groups, key, slot) in keyed(mirrors.get(TableId::RadarCoverage)) {
            index.set_group(key, groups.number(columns.radar_state, slot).value().map(|state| ((key >> 32) as u32, state)), &mut states);
        }
        index
    }

    fn placed_of(bases: &Mirror, slot: u32, columns: Columns) -> Option<(u32, i32)> {
        Some((bases.number(columns.base_state, slot).value()?, bases.number(columns.base_kind, slot).value().unwrap_or(0)))
    }

    fn wing_of(wings: &Mirror, slot: u32, columns: Columns) -> WingRow {
        WingRow {
            base: wing_base_of(wings.number(columns.air_base, slot).value().unwrap_or(0), wings.number(columns.base_override, slot).value().unwrap_or(0)),
            country: wing_country(wings.number(columns.wing_owner, slot).value().unwrap_or(0), wings.number(columns.country_override, slot).value().unwrap_or(0)),
            planes: wings.number(columns.planes, slot).value().unwrap_or(0),
        }
    }

    fn set_wing(&mut self, wing: u64, row: Option<WingRow>, touched: &mut BTreeSet<u64>) {
        let old = match row {
            Some(row) => self.wings.insert(wing, row),
            None => self.wings.remove(&wing),
        };
        if let Some(old) = old {
            self.bases.entry(old.base).or_default().standing -= i64::from(old.planes.max(0));
            self.locate(old.base, old.country, false);
            touched.insert(old.base);
        }
        if let Some(row) = row {
            self.bases.entry(row.base).or_default().standing += i64::from(row.planes.max(0));
            self.locate(row.base, row.country, true);
            touched.insert(row.base);
        }
        let positive = |row: Option<WingRow>| u32::from(row.is_some_and(|row| row.planes > 0));
        let missing = |row: Option<WingRow>| u32::from(row.is_none());
        if positive(old) == positive(row) && missing(old) == missing(row) {
            return;
        }
        for &key in self.listing.get(&wing).into_iter().flatten() {
            let base = (key >> 32) as u64;
            if let Some(at) = self.bases.get_mut(&base).and_then(|at| at.registered.iter_mut().find(|held| held.slot == key as u32)) {
                at.positive = at.positive + positive(row) - positive(old);
                at.missing = at.missing + missing(row) - missing(old);
            }
            touched.insert(base);
        }
    }

    fn locate(&mut self, base: u64, country: u32, up: bool) {
        let at = self.bases.entry(base).or_default();
        match at.located.iter().position(|&(held, _)| held == country) {
            Some(held) if up => at.located[held].1 += 1,
            Some(held) => {
                at.located[held].1 -= 1;
                if at.located[held].1 == 0 {
                    at.located.swap_remove(held);
                    take_from(&mut self.stationed, country, &base);
                }
            }
            None if up => {
                at.located.push((country, 1));
                self.stationed.entry(country).or_default().push(base);
            }
            None => {}
        }
    }

    fn set_controller(&mut self, state: u32, controller: Option<u32>) {
        let old = match controller {
            Some(controller) => self.controllers.insert(state, controller),
            None => self.controllers.remove(&state),
        };
        if old != controller {
            if let Some(old) = old {
                take_from(&mut self.controls, old, &state);
            }
            if let Some(controller) = controller {
                self.controls.entry(controller).or_default().push(state);
            }
        }
    }

    fn set_registration(&mut self, key: u128, wings: Option<Vec<u64>>, touched: &mut BTreeSet<u64>) {
        let (base, slot) = ((key >> 32) as u64, key as u32);
        for wing in self.registrations.remove(&key).unwrap_or_default() {
            take_from(&mut self.listing, wing, &key);
        }
        let registered = wings.as_ref().map(|wings| Registered {
            slot,
            listed: wings.len() as u32,
            positive: wings.iter().filter(|wing| self.wings.get(wing).is_some_and(|row| row.planes > 0)).count() as u32,
            missing: wings.iter().filter(|wing| !self.wings.contains_key(wing)).count() as u32,
        });
        let at = self.bases.entry(base).or_default();
        at.registered.retain(|held| held.slot != slot);
        at.registered.extend(registered);
        if let Some(wings) = wings {
            for &wing in &wings {
                self.listing.entry(wing).or_default().push(key);
            }
            self.registrations.insert(key, wings);
        }
        touched.insert(base);
    }

    fn set_group(&mut self, key: u128, group: Option<(u32, u32)>, states: &mut BTreeSet<u32>) {
        if let Some((country, state)) = self.groups.remove(&key) {
            take_from(&mut self.hosting, country, &state);
            if let Some(countries) = self.hosts.get_mut(&state) {
                if let Some(at) = countries.iter().position(|&held| held == country) {
                    countries.swap_remove(at);
                }
                if countries.is_empty() {
                    self.hosts.remove(&state);
                }
            }
            states.insert(state);
        }
        if let Some((country, state)) = group {
            self.groups.insert(key, (country, state));
            self.hosts.entry(state).or_default().push(country);
            self.hosting.entry(country).or_default().push(state);
            states.insert(state);
        }
    }

    fn bytes(&self) -> usize {
        self.placed.len() * 24
            + self.wings.len() * 32
            + self.registrations.values().map(|wings| wings.capacity() * 8 + 48).sum::<usize>()
            + self.listing.values().map(|keys| keys.capacity() * 16 + 40).sum::<usize>()
            + self.bases.values().map(|at| at.located.capacity() * 8 + at.registered.capacity() * 16 + 72).sum::<usize>()
            + self.groups.len() * 32
            + self.hosts.values().chain(self.hosting.values()).chain(self.controls.values()).map(|list| list.capacity() * 4 + 40).sum::<usize>()
            + self.stationed.values().map(|bases| bases.capacity() * 8 + 40).sum::<usize>()
            + self.controllers.len() * 16
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Census {
    flags: bool,
    wings: bool,
    registrations: bool,
    truncated: bool,
    bases: bool,
    members: bool,
    coverage: Result<bool, ()>,
}

impl Census {
    fn now(mirrors: &Mirrors) -> Self {
        let registrations = mirrors.get(TableId::AirBaseRegistrations);
        Self {
            flags: whole(mirrors.get(TableId::StateFlags)).is_some(),
            wings: whole(mirrors.get(TableId::AirWings)).is_some(),
            registrations: whole(registrations).is_some(),
            truncated: registrations.and_then(Mirror::status).is_some_and(|status| matches!(status, Status::Truncated { .. })),
            bases: whole(mirrors.get(TableId::AirBases)).is_some(),
            members: whole(mirrors.get(TableId::FactionMembers)).is_some(),
            coverage: match Self::unavailable(mirrors) {
                Some(_) => Err(()),
                None => Ok(whole(mirrors.get(TableId::RadarCoverage)).is_some()),
            },
        }
    }

    fn unavailable(mirrors: &Mirrors) -> Option<&str> {
        let coverage = mirrors.get(TableId::RadarCoverage)?;
        matches!(coverage.status(), Some(Status::Unavailable { .. }) | Some(Status::NotRecorded) | None).then(|| coverage.reason().unwrap_or("not recorded here"))
    }

    fn wings(self, ours: u32) -> WingCensus {
        WingCensus { flags_complete: self.flags, wings_complete: self.wings, registrations_complete: self.registrations, registrations_truncated: self.truncated, ours }
    }
}

pub struct BaseCounters {
    columns: Option<Columns>,
    refused: Option<String>,
    values: Option<Arc<BaseValues>>,
    index: Index,
    ours: Option<u32>,
    census: Option<Census>,
    circle: RadarCircle,
    outcomes: HashMap<u32, RadarOutcome>,
    withheld: usize,
    uncontrolled: usize,
    provinces: Vec<u32>,
    moved: HashMap<u64, Option<(u32, i32)>>,
    bases: BTreeSet<u64>,
    states: BTreeSet<u32>,
    sites: BTreeSet<(u32, usize)>,
    radar: BTreeSet<u32>,
    touched: Touched,
    sweep_sites: Option<(u32, i32)>,
    sweep_radar: Option<u32>,
    circle_stale: bool,
    notes_stale: bool,
    all: bool,
    pending: Option<Hour>,
    stepped: Option<Hour>,
    hidden: bool,
    since: Option<u64>,
    vision: Mutex<Option<(Choice, Arc<Vision>)>>,
    vision_new: AtomicBool,
    statuses: HashMap<usize, bool>,
    frame: u64,
    made: usize,
    work: u64,
    scoped: Scoped,
    #[cfg(any(test, feature = "test-support"))]
    applied_reads: u64,
}

const SWEEP_SITES: (u32, i32) = (0, i32::MIN);

impl BaseCounters {
    pub fn new() -> Self {
        Self {
            columns: None,
            refused: Some("not composed yet".into()),
            values: None,
            index: Index::default(),
            ours: None,
            census: None,
            circle: RadarCircle::default(),
            outcomes: HashMap::new(),
            withheld: 0,
            uncontrolled: 0,
            provinces: Vec::new(),
            moved: HashMap::new(),
            bases: BTreeSet::new(),
            states: BTreeSet::new(),
            sites: BTreeSet::new(),
            radar: BTreeSet::new(),
            touched: Touched::default(),
            sweep_sites: None,
            sweep_radar: None,
            circle_stale: false,
            notes_stale: false,
            all: true,
            pending: None,
            stepped: None,
            hidden: false,
            since: None,
            vision: Mutex::new(None),
            vision_new: AtomicBool::new(false),
            statuses: HashMap::new(),
            frame: 0,
            made: 0,
            work: 0,
            scoped: Scoped::default(),
            #[cfg(any(test, feature = "test-support"))]
            applied_reads: 0,
        }
    }

    pub fn fresh(&self) -> Self {
        let fresh = Self { columns: self.columns, hidden: self.hidden, ..Self::new() };
        if let Some((choice, vision)) = self.perspective() {
            fresh.set_vision(choice, vision);
        }
        fresh
    }

    pub fn set_vision(&self, choice: Choice, vision: Arc<Vision>) {
        let mut held = self.vision.lock().unwrap_or_else(|error| error.into_inner());
        if held.as_ref().is_none_or(|(_, held)| !Arc::ptr_eq(held, &vision) && **held != *vision) {
            self.vision_new.store(true, Ordering::Release);
        }
        *held = Some((choice, vision));
    }

    pub fn perspective(&self) -> Option<(Choice, Arc<Vision>)> {
        self.vision.lock().unwrap_or_else(|error| error.into_inner()).clone()
    }

    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    pub fn hidden(&self) -> bool {
        self.hidden
    }

    pub fn values(&self) -> Option<&BaseValues> {
        self.values.as_deref()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn published(&self) -> Option<&Arc<BaseValues>> {
        self.values.as_ref().filter(|_| self.refused.is_none() && !self.hidden)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn made(&self) -> usize {
        self.made
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn work(&self) -> u64 {
        self.work
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn applied_reads(&self) -> u64 {
        self.applied_reads
    }

    fn columns(&self) -> Columns {
        self.columns.expect("the base counters layer is declared before it composes")
    }

    fn release(&mut self) {
        let kept = std::mem::replace(self, Self::new());
        (self.columns, self.hidden, self.vision, self.vision_new, self.frame, self.made, self.work) = (kept.columns, kept.hidden, kept.vision, kept.vision_new, kept.frame, kept.made, kept.work);
        #[cfg(any(test, feature = "test-support"))]
        {
            self.applied_reads = kept.applied_reads;
        }
        self.all = false;
    }

    fn core(&self, mirrors: &Mirrors) -> Result<Arc<Vision>, String> {
        let (_, vision) = self.perspective().ok_or("the viewing country's vision is not set")?;
        let links = mirrors.get(TableId::CountryLinks).ok_or("CountryLinks is not read yet")?;
        if let Some(why) = links.not_recorded() {
            return Err(format!("{why}; the base counters need every country link"));
        }
        let provinces = mirrors.get(TableId::Provinces).ok_or("Provinces is not read yet")?;
        if provinces.status().is_none() {
            return Err("Provinces is not recorded here: a gap in the recording".into());
        }
        Ok(vision)
    }

    fn link<'a>(mirrors: &'a Mirrors, columns: Columns) -> impl Fn(u32) -> Option<u32> + 'a {
        let links = held(mirrors.get(TableId::CountryLinks));
        move |raw| {
            let links = links?;
            links.number(columns.link_slot, links.slot(u128::from(raw))?).value()
        }
    }

    fn edit(&mut self) -> &mut BaseValues {
        Arc::make_mut(self.values.get_or_insert_with(Default::default))
    }

    fn carries(&self) -> bool {
        self.all
            || self.circle_stale
            || self.notes_stale
            || self.sweep_sites.is_some()
            || self.sweep_radar.is_some()
            || !self.provinces.is_empty()
            || !self.moved.is_empty()
            || !self.bases.is_empty()
            || !self.states.is_empty()
            || !self.sites.is_empty()
            || !self.radar.is_empty()
            || !self.touched.is_empty()
    }

    fn base(&self, base: u64, state: u32, mirrors: &Mirrors, columns: Columns, census: &WingCensus, link: &dyn Fn(u32) -> Option<u32>) -> AirBaseCount {
        let capacity = row(mirrors.get(TableId::AirBases), u128::from(base)).and_then(|(bases, slot)| bases.number(columns.base_capacity, slot).value()).unwrap_or(0);
        let flag = row(mirrors.get(TableId::StateFlags), u128::from(state)).map(|(flags, slot)| flags.number(columns.demilitarized, slot).value().unwrap_or(0));
        let empty = Aggregate::default();
        let at = self.index.bases.get(&base).unwrap_or(&empty);
        let registrations = mirrors.get(TableId::AirBaseRegistrations);
        let facts: Vec<RegistrationFacts> = at
            .registered
            .iter()
            .map(|held| RegistrationFacts {
                slot: held.slot,
                capacity_raw: row(registrations, (u128::from(base) << 32) | u128::from(held.slot)).and_then(|(rows, slot)| rows.number(columns.capacity_raw, slot).value()).unwrap_or(0),
                listed: held.listed > 0,
                occupied: held.positive > 0,
                missing: held.missing > 0,
            })
            .collect();
        let located = |slot: u32| at.located.iter().any(|&(country, _)| link(country) == Some(slot));
        base_count(&BaseFacts { capacity, flag, registrations: &facts, located: &located, standing: at.standing }, census)
    }

    fn site(&mut self, site: (u32, usize), mirrors: &Mirrors, columns: Columns, census: &WingCensus, link: &dyn Fn(u32) -> Option<u32>) -> Option<AirBaseCount> {
        self.scoped.state(site.0);
        let values = self.values.as_deref()?;
        let bases: Vec<u64> = values.site_bases.keys_from((site.0, i32::MIN)).take_while(|&(state, _)| state == site.0).filter(|&(_, kind)| site_kind(kind) == site.1).flat_map(|key| values.site_bases[&key].iter().copied()).collect();
        let mut bases = bases.into_iter();
        let Some(first) = bases.next() else {
            return None;
        };
        let mut count = self.base(first, site.0, mirrors, columns, census, link);
        let mut worked = 1u64;
        for base in bases {
            merge_count(&mut count, self.base(base, site.0, mirrors, columns, census, link));
            worked += 1;
        }
        self.work += worked;
        Some(count)
    }

    fn make_site(&mut self, site: (u32, usize), mirrors: &Mirrors, columns: Columns, census: &WingCensus, link: &dyn Fn(u32) -> Option<u32>) {
        let count = self.site(site, mirrors, columns, census, link);
        if self.values.as_deref().and_then(|values| values.wings.get(&site)) != count.as_ref() {
            match count {
                Some(count) => self.edit().wings.insert(site, count),
                None => self.edit().wings.remove(&site),
            };
        }
        self.made += 1;
    }

    fn place(&mut self, base: u64, held: Option<(u32, i32)>) {
        match held.or_else(|| self.index.placed.get(&base).copied()) {
            Some((state, _)) => self.scoped.state(state),
            None => self.scoped.unplaced(),
        }
        let now = self.index.placed.get(&base).copied();
        self.work += 1;
        if now == held {
            return;
        }
        if let Some(key) = held {
            let mut bases = self.values.as_deref().and_then(|values| values.site_bases.get(&key)).cloned().unwrap_or_default();
            bases.retain(|&held| held != base);
            match bases.is_empty() {
                true => self.edit().site_bases.remove(&key),
                false => self.edit().site_bases.insert(key, bases),
            };
            self.sites.insert((key.0, site_kind(key.1)));
        }
        if let Some(key) = now {
            let mut bases = self.values.as_deref().and_then(|values| values.site_bases.get(&key)).cloned().unwrap_or_default();
            if let Err(at) = bases.binary_search(&base) {
                bases.insert(at, base);
            }
            self.edit().site_bases.insert(key, bases);
            self.sites.insert((key.0, site_kind(key.1)));
        }
    }

    fn circle(mirrors: &Mirrors, columns: Columns, ours: u32, link: &dyn Fn(u32) -> Option<u32>) -> RadarCircle {
        let mut members: Vec<(u32, u64)> = keyed(mirrors.get(TableId::FactionMembers)).map(|(rows, key, slot)| (key as u32, rows.number(columns.faction, slot).value().unwrap_or(0))).collect();
        members.sort_unstable();
        let appearance = keyed(mirrors.get(TableId::CountryAppearance)).filter(|&(_, key, _)| link(key as u32) == Some(ours)).min_by_key(|&(_, key, _)| key).map(|(rows, _, slot)| {
            let subjects: Recorded<Vec<i32>> = recorded(rows, columns.subjects, slot);
            (subjects, rows.number(columns.overlord, slot).value().unwrap_or(0))
        });
        RadarCircle::of(link, &members, whole(mirrors.get(TableId::FactionMembers)).is_some(), appearance.as_ref(), ours)
    }

    fn decide(&mut self, state: u32, mirrors: &Mirrors, columns: Columns, link: &dyn Fn(u32) -> Option<u32>) {
        self.scoped.state(state);
        let hosting: Vec<u32> = self.index.hosts.get(&state).into_iter().flatten().filter_map(|&country| link(country)).collect();
        let controller = row(mirrors.get(TableId::States), u128::from(state)).map(|(states, slot)| states.number(columns.state_controller, slot).value().unwrap_or(0));
        self.index.set_controller(state, controller.filter(|_| self.index.hosts.contains_key(&state)));
        let outcome = if hosting.is_empty() { RadarOutcome::Hidden } else { self.circle.outcome(hosting.iter().copied(), controller.and_then(link)) };
        self.work += 1 + hosting.len() as u64;
        self.made += 1;
        let old = self.outcomes.get(&state).copied().unwrap_or(RadarOutcome::Hidden);
        if old == outcome {
            return;
        }
        for (held, by) in [(old, -1), (outcome, 1)] {
            match held {
                RadarOutcome::Withheld => self.withheld = self.withheld.saturating_add_signed(by),
                RadarOutcome::Uncontrolled => self.uncontrolled = self.uncontrolled.saturating_add_signed(by),
                RadarOutcome::Shown | RadarOutcome::Hidden => {}
            }
        }
        match outcome {
            RadarOutcome::Hidden => self.outcomes.remove(&state),
            _ => self.outcomes.insert(state, outcome),
        };
        if (old == RadarOutcome::Shown) != (outcome == RadarOutcome::Shown) {
            match outcome == RadarOutcome::Shown {
                true => self.edit().radar.insert(state, ()),
                false => self.edit().radar.remove(&state),
            };
        }
        self.notes_stale = true;
    }

    fn make_whole(&mut self, mirrors: &Mirrors, columns: Columns, ours: u32, link: &dyn Fn(u32) -> Option<u32>) {
        self.statuses = TABLES.iter().filter_map(|&table| Some((table.index(), holds(mirrors.get(table)?.status()?)))).collect();
        self.index = Index::build(mirrors, columns);
        let census = Census::now(mirrors);
        (self.ours, self.census) = (Some(ours), Some(census));
        let controlled: BTreeMap<u32, bool> = keyed(mirrors.get(TableId::Provinces)).map(|(rows, key, slot)| (key as u32, controlled_of(rows.number(columns.controller, slot).value().unwrap_or(0)))).collect();
        controlled.keys().for_each(|&province| self.scoped.province(province));
        let mut site_bases: BTreeMap<(u32, i32), Vec<u64>> = BTreeMap::new();
        for (&base, &key) in &self.index.placed {
            site_bases.entry(key).or_default().push(base);
        }
        site_bases.values_mut().for_each(|bases| bases.sort_unstable());
        let sites: BTreeSet<(u32, usize)> = site_bases.keys().map(|&(state, kind)| (state, site_kind(kind))).collect();
        self.work += (self.index.wings.len() + self.index.registrations.len() + controlled.len() + site_bases.len()) as u64;
        self.edit_parts(|values| {
            values.controlled.assign(controlled);
            values.site_bases.assign(site_bases);
            values.complete = census.bases;
            values.aircraft_complete = census.wings;
        });
        let wing_census = census.wings(ours);
        let wings: BTreeMap<(u32, usize), AirBaseCount> = sites.into_iter().filter_map(|site| Some((site, self.site(site, mirrors, columns, &wing_census, link)?))).collect();
        self.edit_parts(|values| values.wings.assign(wings));
        self.circle = Self::circle(mirrors, columns, ours, link);
        (self.outcomes, self.withheld, self.uncontrolled) = (HashMap::new(), 0, 0);
        let states: Vec<u32> = self.index.hosts.keys().copied().collect();
        let mut shown = BTreeMap::new();
        for state in states {
            self.decide(state, mirrors, columns, link);
            if self.outcomes.get(&state) == Some(&RadarOutcome::Shown) {
                shown.insert(state, ());
            }
        }
        self.edit_parts(|values| values.radar.assign(shown));
        self.make_notes(mirrors);
        self.provinces.clear();
        self.touched.clear();
        (self.moved, self.bases, self.states, self.sites, self.radar) = Default::default();
        (self.sweep_sites, self.sweep_radar, self.circle_stale) = (None, None, false);
        self.made += 1;
    }

    fn edit_parts(&mut self, change: impl FnOnce(&mut BaseValues)) {
        let mut made = self.values.as_deref().cloned().unwrap_or_default();
        change(&mut made);
        if self.values.as_deref() != Some(&made) {
            *self.edit() = made;
        }
    }

    fn make_notes(&mut self, mirrors: &Mirrors) {
        let coverage = match Census::unavailable(mirrors) {
            Some(reason) => Err(reason),
            None => Ok(whole(mirrors.get(TableId::RadarCoverage)).is_some()),
        };
        let notes = radar_notes_of(coverage, self.withheld, self.uncontrolled, &self.circle.undecided);
        if self.values.as_deref().is_none_or(|values| *values.radar_notes != notes) {
            self.edit().radar_notes = Arc::new(notes);
        }
        self.notes_stale = false;
    }

    fn follow(&mut self, census: Census, ours: u32) {
        let (Some(held), Some(was)) = (self.census, self.ours) else { return };
        let marked = self.carries();
        if (held.flags, held.wings, held.registrations, held.truncated) != (census.flags, census.wings, census.registrations, census.truncated) || was != ours {
            self.sweep_sites = Some(SWEEP_SITES);
        }
        if held.members != census.members || was != ours {
            self.circle_stale = true;
        }
        if held.coverage != census.coverage {
            self.notes_stale = true;
        }
        if self.values.as_deref().is_some_and(|values| values.complete != census.bases || values.aircraft_complete != census.wings) {
            let values = self.edit();
            (values.complete, values.aircraft_complete) = (census.bases, census.wings);
        }
        (self.census, self.ours) = (Some(census), Some(ours));
        if !marked && self.carries() && self.pending.is_none() {
            self.pending = self.stepped;
        }
    }

    fn settle_dirty(&mut self, mirrors: &Mirrors) {
        self.frame += 1;
        let core = if self.hidden { Err("not drawn at this view".to_owned()) } else { self.core(mirrors) };
        let vision = match core {
            Ok(vision) => vision,
            Err(why) => {
                if self.refused.as_ref() != Some(&why) {
                    self.release();
                    self.refused = Some(why);
                }
                (self.all, self.since, self.pending) = (false, None, None);
                return;
            }
        };
        let ours = vision.ours();
        self.vision_new.swap(false, Ordering::AcqRel);
        if self.refused.take().is_some() {
            self.all = true;
        }
        if !self.all {
            self.follow(Census::now(mirrors), ours);
        }
        self.stepped = None;
        if !self.carries() {
            (self.since, self.pending) = (None, None);
            return;
        }
        let columns = self.columns();
        let link = Self::link(mirrors, columns);
        if std::mem::take(&mut self.all) {
            self.make_whole(mirrors, columns, ours, &link);
        }
        self.take_touched(mirrors, columns);
        let census = self.census.unwrap_or_else(|| Census::now(mirrors)).wings(ours);
        if self.circle_stale && true {
            let circle = Self::circle(mirrors, columns, ours, &link);
            self.work += 1;
            if circle != self.circle {
                self.circle = circle;
                self.sweep_radar = Some(0);
                self.notes_stale = true;
            }
            self.circle_stale = false;
        }
        while !self.moved.is_empty() && true {
            let moved: Vec<(u64, Option<(u32, i32)>)> = self.moved.iter().take(PER_LOOK).map(|(&base, &held)| (base, held)).collect();
            for (base, held) in moved {
                self.moved.remove(&base);
                self.place(base, held);
            }
        }
        let provinces = held(mirrors.get(TableId::Provinces));
        while !self.provinces.is_empty() && true {
            let at = self.provinces.len().saturating_sub(PER_LOOK * 8);
            for province in self.provinces.split_off(at) {
                self.scoped.province(province);
                let controlled = provinces.and_then(|provinces| Some(controlled_of(provinces.number(columns.controller, provinces.slot(u128::from(province))?).value().unwrap_or(0))));
                self.work += 1;
                if self.values.as_deref().and_then(|values| values.controlled.get(&province)) != controlled.as_ref() {
                    match controlled {
                        Some(controlled) => self.edit().controlled.insert(province, controlled),
                        None => self.edit().controlled.remove(&province),
                    };
                }
            }
        }
        if self.moved.is_empty() {
            for base in std::mem::take(&mut self.bases) {
                if let Some(&(state, kind)) = self.index.placed.get(&base) {
                    self.sites.insert((state, site_kind(kind)));
                }
            }
            if let Some(values) = self.values.as_deref() {
                for state in std::mem::take(&mut self.states) {
                    self.sites.extend(values.site_bases.keys_from((state, i32::MIN)).take_while(|&(of, _)| of == state).map(|(state, kind)| (state, site_kind(kind))));
                }
            }
            while !self.sites.is_empty() && true {
                for _ in 0..PER_LOOK {
                    let Some(site) = self.sites.pop_first() else { break };
                    self.make_site(site, mirrors, columns, &census, &link);
                }
            }
            while let Some(from) = self.sweep_sites.filter(|_| self.sites.is_empty())
                && true
            {
                let keys: Vec<(u32, i32)> = self.values.as_deref().map(|values| values.site_bases.keys_from(from).take(PER_LOOK + 1).collect()).unwrap_or_default();
                self.sweep_sites = keys.get(PER_LOOK).copied();
                for (state, kind) in keys.into_iter().take(PER_LOOK) {
                    self.make_site((state, site_kind(kind)), mirrors, columns, &census, &link);
                }
            }
        }
        if !self.circle_stale {
            while !self.radar.is_empty() && true {
                for _ in 0..PER_LOOK {
                    let Some(state) = self.radar.pop_first() else { break };
                    self.decide(state, mirrors, columns, &link);
                }
            }
            while let Some(from) = self.sweep_radar
                && true
            {
                let states: Vec<u32> = self.index.hosts.range(from..).take(PER_LOOK + 1).map(|(&state, _)| state).collect();
                self.sweep_radar = states.get(PER_LOOK).copied();
                for state in states.into_iter().take(PER_LOOK) {
                    self.decide(state, mirrors, columns, &link);
                }
            }
            if self.notes_stale && self.radar.is_empty() && self.sweep_radar.is_none() {
                self.make_notes(mirrors);
            }
        }
        if self.carries() {
            self.since.get_or_insert(self.frame);
        } else {
            (self.since, self.pending) = (None, None);
        }
    }

    fn groups(table: TableId, column: usize, columns: Columns) -> u8 {
        match table {
            TableId::Provinces => u8::from(column == columns.controller.column),
            TableId::AirBases => u8::from(column == columns.base_state.column || column == columns.base_kind.column) | (u8::from(column == columns.base_capacity.column) << 1),
            TableId::StateFlags => u8::from(column == columns.demilitarized.column),
            TableId::AirWings => u8::from([columns.wing_owner.column, columns.country_override.column, columns.air_base.column, columns.base_override.column, columns.planes.column].contains(&column)),
            TableId::AirBaseRegistrations => u8::from(column == columns.capacity_raw.column) | (u8::from(column == columns.registered.column) << 1),
            TableId::RadarCoverage => u8::from(column == columns.radar_state.column),
            TableId::States => u8::from(column == columns.state_controller.column),
            TableId::FactionMembers => u8::from(column == columns.faction.column),
            TableId::CountryAppearance => u8::from(column == columns.subjects.column || column == columns.overlord.column),
            TableId::CountryLinks => u8::from(column == columns.link_slot.column),
            _ => 0,
        }
    }

    fn touch(&mut self, table: TableId, groups: u8, key: u128, mirrors: &Mirrors, columns: Columns) {
        let mirror = mirrors.get(table);
        let (first, second) = (groups & 1 != 0, groups & 2 != 0);
        match table {
            TableId::Provinces => {
                self.work += 1;
                self.provinces.push(key as u32);
            }
            TableId::AirBases => {
                if first {
                    let base = key as u64;
                    let placed = row(mirror, key).and_then(|(bases, slot)| Index::placed_of(bases, slot, columns));
                    let held = match placed {
                        Some(placed) => self.index.placed.insert(base, placed),
                        None => self.index.placed.remove(&base),
                    };
                    self.moved.entry(base).or_insert(held);
                    self.work += 1;
                }
                if second {
                    self.work += 1;
                    self.bases.insert(key as u64);
                }
            }
            TableId::StateFlags => {
                self.work += 1;
                self.states.insert(key as u32);
            }
            TableId::AirWings => {
                let wing = row(mirror, key).map(|(wings, slot)| Index::wing_of(wings, slot, columns));
                self.work += 1 + self.index.listing.get(&(key as u64)).map_or(0, Vec::len) as u64;
                self.index.set_wing(key as u64, wing, &mut self.bases);
            }
            TableId::AirBaseRegistrations => {
                if first {
                    self.work += 1;
                    self.bases.insert((key >> 32) as u64);
                }
                if second {
                    let wings = row(mirror, key).map(|(rows, slot)| u64s(bytes_of(rows.list(columns.registered, slot))));
                    self.work += 1 + wings.as_ref().map_or(0, Vec::len) as u64 + self.index.registrations.get(&key).map_or(0, Vec::len) as u64;
                    self.index.set_registration(key, wings, &mut self.bases);
                }
            }
            TableId::RadarCoverage => {
                let group = row(mirror, key).and_then(|(groups, slot)| groups.number(columns.radar_state, slot).value()).map(|state| ((key >> 32) as u32, state));
                self.work += 1;
                self.index.set_group(key, group, &mut self.radar);
            }
            TableId::States => {
                self.work += 1;
                self.radar.insert(key as u32);
            }
            TableId::FactionMembers | TableId::CountryAppearance => self.circle_stale = true,
            TableId::CountryLinks => {
                let country = key as u32;
                let index = &self.index;
                self.bases.extend(index.stationed.get(&country).into_iter().flatten().copied());
                self.radar.extend(index.hosting.get(&country).into_iter().chain(index.controls.get(&country)).flatten().copied());
                self.circle_stale = true;
                self.work += 1;
            }
            _ => {}
        }
    }

    fn take_touched(&mut self, mirrors: &Mirrors, columns: Columns) {
        let mut touched = std::mem::take(&mut self.touched);
        touched.take(mirrors, |table, touch, key| self.touch(table, touch.groups(), key, mirrors, columns));
        self.touched = touched;
    }
}

impl Layer for BaseCounters {
    fn held_viewer(&self) -> Option<u32> {
        self.perspective().map(|(_, vision)| vision.ours())
    }

    fn name(&self) -> &'static str {
        "base_counters"
    }

    fn placed(&mut self, _view: &viewer_layer_core::layers::View) -> Option<u64> {
        Some(0)
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        if !view.overlay.bases {
            self.hidden = true;
            return Ok(());
        }
        for table in TABLES {
            inputs.table(table, true)?;
        }
        use TableId as T;
        self.columns = Some(Columns {
            controller: inputs.number(T::Provinces, "controller")?,
            base_state: inputs.number(T::AirBases, "state")?,
            base_kind: inputs.number(T::AirBases, "kind")?,
            base_capacity: inputs.number(T::AirBases, "capacity")?,
            demilitarized: inputs.number(T::StateFlags, "demilitarized")?,
            wing_owner: inputs.number(T::AirWings, "owner")?,
            country_override: inputs.number(T::AirWings, "country_override")?,
            air_base: inputs.number(T::AirWings, "air_base")?,
            base_override: inputs.number(T::AirWings, "base_override")?,
            planes: inputs.number(T::AirWings, "planes")?,
            capacity_raw: inputs.number(T::AirBaseRegistrations, "capacity_raw")?,
            registered: inputs.list(T::AirBaseRegistrations, "wings")?,
            radar_state: inputs.number(T::RadarCoverage, "state")?,
            faction: inputs.number(T::FactionMembers, "faction")?,
            subjects: inputs.list(T::CountryAppearance, "subjects")?,
            overlord: inputs.number(T::CountryAppearance, "overlord")?,
            state_controller: inputs.number(T::States, "controller")?,
            link_slot: inputs.number(T::CountryLinks, "canonical_slot")?,
        });
        if std::mem::take(&mut self.hidden) {
            self.all = true;
        }
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        let made = self.made;
        self.all = true;
        self.settle_dirty(mirrors);
        self.made = made;
        Ok(())
    }

    fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
        if step.gap() || self.columns.is_none() || self.hidden || self.refused.is_some() || self.all {
            return;
        }
        #[cfg(any(test, feature = "test-support"))]
        let reads = viewer_layer_core::playhead_host::mirror::probe::reads();
        self.stepped.get_or_insert(step.first());
        let columns = self.columns();
        let mut order = TABLES;
        order.sort_unstable_by_key(|table| table.index());
        for table in order {
            let Some(changes) = step.table(table.index()) else { continue };
            if changes.reset() || changes.status().is_some_and(|(status, _)| self.statuses.insert(changes.table(), holds(status)) != Some(holds(status))) {
                self.all = true;
                self.touched.clear();
                return;
            }
            self.touched.note(changes, self.frame, |column| Self::groups(table, column, columns));
        }
        if self.carries() && self.pending.is_none() {
            self.pending = Some(step.first());
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            self.applied_reads += viewer_layer_core::playhead_host::mirror::probe::reads() - reads;
        }
    }

    fn settle(&mut self, mirrors: &Mirrors, _settling: Settling) -> io::Result<()> {
        self.settle_dirty(mirrors);
        Ok(())
    }


    fn carried(&self) -> Carried {
        let flags = [self.all, self.circle_stale, self.notes_stale, self.sweep_sites.is_some(), self.sweep_radar.is_some()].into_iter().filter(|&flag| flag).count();
        let entities = flags + self.provinces.len() + self.moved.len() + self.bases.len() + self.states.len() + self.sites.len() + self.radar.len() + self.touched.len();
        Carried { entities }
    }

    fn published(&self) -> Published {
        if self.refused.is_some() || self.hidden {
            Published::Current
        } else if self.all {
            Published::Nothing
        } else {
            self.pending.map_or(Published::Current, |first| Published::At(first - 1))
        }
    }

    fn scope(&mut self, sight: &Sight<'_>) -> Option<Scope> {
        Some(self.scoped.take(sight, None))
    }

    fn held(&self) -> usize {
        self.values.as_ref().map_or(0, |values| {
            values.wings.len() * (16 + std::mem::size_of::<AirBaseCount>())
                + values.controlled.len() * 16
                + values.radar.len() * 8
                + (values.wings.shard_count() + values.controlled.shard_count() + values.radar.shard_count() + values.site_bases.shard_count()) * 64
                + values.site_bases.values().map(|bases| bases.capacity() * 8 + 40).sum::<usize>()
        }) + self.provinces.capacity() * 4
            + (self.moved.len() + self.bases.len() + self.states.len() + self.sites.len() + self.radar.len() + self.outcomes.len()) * 16
            + self.index.bytes()
            + self.touched.bytes()
            + self.scoped.bytes()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

pub mod fed;
pub use fed::BaseCountersFed;
#[cfg(test)]
mod tests;
