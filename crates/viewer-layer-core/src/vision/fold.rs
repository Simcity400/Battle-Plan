
use super::{Columns, Content, Global};
use crate::owned::{Delta as Changes, Table as Mirror, Tables as Mirrors, complete, opaque, owner_rows, u32s};
use crate::{
    layers::Dirty,
    playhead_host::mirror::Cell,
};
use replay_data::rows::{Recorded, ReservedCountryIntelSample, ReservedReverseVisibilityInputs, ReverseZeroVisibilityInputs, TableId, ZeroSubjectInputs};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use viewer_foundation::perspective::{
    active_war,
    intel_fraction,
    map_cutoff,
    map_intel_subject,
    network_term,
    saturate,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Order {
    foreign: bool,
    slot: u32,
    native: u32,
    phase: u8,
    a: u64,
    b: u64,
    sub: u32,
}

#[derive(Clone, Copy, Debug)]
enum Action {
    Set,
    Stamp { from: u32, base: i32 },
    Network { raw_country: u32, state: u32 },
    Radar { raw_country: u32 },
}

#[derive(Clone, Copy, Debug)]
struct Term {
    order: Order,
    action: Action,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Source {
    Controlled(u32),
    Areas,
    Hosts,
    Occupied(u32),
    Division(u64),
    Fleet(u64),
    Task(u64),
    Radar(u128),
    Stationed,
}

const BEFORE: u8 = 0;
const INPUTS: u8 = 1;
const WALK: u8 = 2;
const INVASION: u8 = 3;

type ErrorKey = (u8, Order);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Owner {
    Source(Source),
    Province(u32),
    Sea(u64),
    Tables,
}

mod group {
    pub const COUNTRIES: u8 = 0;
    pub const PROVINCES: u8 = 1;
    pub const DIVISIONS: u8 = 2;
    pub const TASK_FORCES: u8 = 3;
    pub const STATE_VISIBILITY: u8 = 4;
    pub const TOPOLOGY: u8 = 5;
    pub const CONTROLLED: u8 = 6;
    pub const STATES: u8 = 7;
    pub const OCCUPIED: u8 = 8;
    pub const ROSTERS: u8 = 9;
    pub const FLEETS_INPUT: u8 = 10;
    pub const FLEETS: u8 = 11;
    pub const THEATRES_INPUT: u8 = 12;
    pub const OWN_THEATRES: u8 = 13;
    pub const VOLUNTEERS_HERE: u8 = 14;
    pub const AREAS_INPUT: u8 = 15;
    pub const STATIONED_INPUT: u8 = 16;
    pub const HOSTS_INPUT: u8 = 17;
    pub const HOSTS: u8 = 18;
    pub const RADAR_INPUT: u8 = 19;
    pub const RADAR: u8 = 20;
    pub const VIEWER: u8 = 21;
}

const PHASE_CONTROLLED: u8 = 0;
const PHASE_AREAS: u8 = 1;
const PHASE_HOSTS: u8 = 2;
const PHASE_OCCUPIED: u8 = 3;
const PHASE_DIVISIONS: u8 = 4;
const PHASE_TASKS: u8 = 5;
const PHASE_RADAR: u8 = 6;
const PHASE_STATIONED: u8 = 7;

fn at(group: u8, key: u128) -> Order {
    Order { phase: group, a: (key >> 64) as u64, b: key as u64, ..Order::default() }
}

struct Reads<'a> {
    content: &'a Content,
    global: &'a Global,
    columns: &'a Columns,
    mirrors: &'a Mirrors,
    provinces: Option<&'a Mirror>,
}

impl Reads<'_> {
    fn slot(&self, raw: u32) -> Result<u32, &'static str> {
        if raw == 0 { Ok(0) } else { self.global.links.get(&raw).copied().ok_or("visibility country link") }
    }

    fn equal(&self, a: u32, b: u32) -> Result<bool, &'static str> {
        Ok(a == b || (a != 0 && b != 0 && self.slot(a)? == self.slot(b)?))
    }

    fn owned(&self, raw: u32) -> Result<Option<u32>, &'static str> {
        if raw == 0 { Ok(None) } else { self.global.links.get(&raw).copied().map(Some).ok_or("visibility country link") }
    }

    fn controller(&self, province: u32) -> Option<u32> {
        let rows = self.provinces?;
        rows.number(self.columns.controller, rows.slot(u128::from(province))?).value()
    }

    fn present(&self, province: u32) -> bool {
        self.provinces.is_some_and(|rows| rows.slot(u128::from(province)).is_some())
    }

    fn reserved(&self) -> Result<(&Mirror, u32), &'static str> {
        let rows = complete(self.mirrors, TableId::ReservedCountryNaming).ok_or("ReservedCountryNaming")?;
        let mut held = rows.rows();
        match (held.next(), held.next()) {
            (Some(slot), None) => Ok((rows, slot)),
            _ => Err("ReservedCountryNaming singleton"),
        }
    }

    fn reserved_faction(&self) -> Result<u64, &'static str> {
        let (rows, slot) = self.reserved()?;
        match rows.number(self.columns.reserved_faction, slot) {
            Cell::Value(faction) => Ok(faction),
            Cell::Missing => Err("ReservedCountryNaming.faction capture refused"),
            Cell::Unrecorded => Err("ReservedCountryNaming.faction not recorded"),
        }
    }

    fn faction_of(&self, target: u32) -> Result<Option<u64>, &'static str> {
        let rows = complete(self.mirrors, TableId::FactionMembers).ok_or("FactionMembers")?;
        let mut faction = None;
        for slot in rows.rows() {
            let Some(key) = rows.key(slot) else { continue };
            let country = self.slot(key as u32)?;
            let held = rows.number(self.columns.faction, slot).value().unwrap_or(0);
            if held == 0 {
                return Err("FactionMembers has null faction identity");
            }
            if country == target {
                if faction.is_some_and(|previous| previous != held) {
                    return Err("FactionMembers has inconsistent faction identity");
                }
                faction = Some(held);
            }
        }
        Ok(faction)
    }

    fn reverse_zero_shared(&self, target: u32, hour: i32) -> Result<bool, &'static str> {
        let (rows, slot) = self.reserved()?;
        let packet = match opaque::<Option<ReservedReverseVisibilityInputs>>(rows, self.columns.reverse_visibility, slot) {
            Ok(Recorded::Value(Some(packet))) => packet,
            Ok(Recorded::Value(None)) | Err(_) => return Err("ReservedCountryNaming.reverse_visibility capture refused"),
            Ok(Recorded::NotRecorded) => return Err("ReservedCountryNaming.reverse_visibility not recorded"),
        };
        if !packet.sample_hour.fresh_at(hour) {
            return Err("ReservedCountryNaming.reverse_visibility sample hour mismatch");
        }
        let cell = packet.cells.get(target as usize).and_then(Option::as_ref).ok_or("ReservedCountryNaming.reverse_visibility cell capture refused")?;
        if active_war(cell.war.ok_or("ReservedCountryNaming.reverse_visibility war capture refused")?)? {
            return Ok(false);
        }
        let faction = self.reserved_faction()?;
        if faction != 0 && (target == 0 || self.faction_of(target)? == Some(faction)) {
            return Ok(true);
        }
        let cell_target = cell.target.ok_or("ReservedCountryNaming.reverse_visibility target capture refused");
        let subject = cell.subject.as_ref().ok_or("ReservedCountryNaming.reverse_visibility subject capture refused")?;
        map_intel_subject(subject, &self.global.links, cell_target)
    }

    fn forward_zero_shared(&self, owner: u32) -> Result<bool, &'static str> {
        let rows = complete(self.mirrors, TableId::CountryCounterInputs).ok_or("CountryCounterInputs")?;
        let mut matching = rows.rows().filter(|&slot| rows.key(slot).is_some_and(|key| self.global.links.get(&(key as u32)) == Some(&owner)));
        let slot = matching.next().ok_or("CountryCounterInputs visibility owner missing")?;
        if matching.next().is_some() {
            return Err("CountryCounterInputs visibility owner duplicated");
        }
        let forward = opaque::<Option<ReverseZeroVisibilityInputs>>(rows, self.columns.forward_visibility, slot).unwrap_or(Recorded::Value(None));
        if !matches!(forward, Recorded::NotRecorded) {
            let cell = match forward {
                Recorded::Value(Some(cell)) => cell,
                _ => return Err("CountryCounterInputs.forward_visibility capture refused"),
            };
            if active_war(cell.war.ok_or("CountryCounterInputs.forward_visibility war capture refused")?)? {
                return Ok(false);
            }
            let faction = (|| -> Result<bool, &'static str> {
                let Some(faction) = self.faction_of(owner)? else { return Ok(false) };
                Ok(self.reserved_faction()? == faction)
            })();
            if faction == Ok(true) {
                return Ok(true);
            }
            let subject = (|| -> Result<bool, &'static str> {
                let target = cell.target.ok_or("CountryCounterInputs.forward_visibility target capture refused");
                let subject = cell.subject.as_ref().ok_or("CountryCounterInputs.forward_visibility subject capture refused")?;
                map_intel_subject(subject, &self.global.links, target)
            })();
            if subject == Ok(true) {
                return Ok(true);
            }
            faction?;
            return subject;
        }
        let war = match opaque::<Option<[u8; 2]>>(rows, self.columns.zero_war, slot) {
            Ok(Recorded::NotRecorded) => return Err("CountryCounterInputs.zero_war not recorded"),
            Ok(Recorded::Value(Some(war))) => war,
            _ => return Err("CountryCounterInputs.zero_war capture refused"),
        };
        if active_war(war)? {
            return Ok(false);
        }
        if let Some(faction) = self.faction_of(owner)?
            && self.reserved_faction()? == faction
        {
            return Ok(true);
        }
        let subject = match opaque::<Option<ZeroSubjectInputs>>(rows, self.columns.zero_subject, slot) {
            Ok(Recorded::NotRecorded) => return Err("CountryCounterInputs.zero_subject not recorded"),
            Ok(Recorded::Value(Some(subject))) => subject,
            _ => return Err("CountryCounterInputs.zero_subject capture refused"),
        };
        map_intel_subject(&subject, &self.global.links, Ok(0))
    }

    fn army(&self, owner: u32, target: u32) -> Result<i64, &'static str> {
        let rows = complete(self.mirrors, TableId::CountryIntel).ok_or("CountryIntel")?;
        Ok(self
            .global
            .raws
            .get(&owner)
            .into_iter()
            .flatten()
            .find_map(|&raw| rows.slot(u128::from(raw) << 32 | u128::from(target)).and_then(|slot| rows.number(self.columns.army, slot).value()))
            .unwrap_or(0))
    }

    fn zero_army(&self, raw_target: u32, target: u32, hour: i32, zero_rows: &[(u32, i64)]) -> Result<i64, &'static str> {
        if raw_target as i32 <= 0 {
            return Ok(0);
        }
        if target > i32::MAX as u32 {
            return Err("CountryIntel owner0 target outside native index range");
        }
        let (rows, slot) = self.reserved()?;
        let sample = match opaque::<Option<ReservedCountryIntelSample>>(rows, self.columns.intel_sample, slot) {
            Ok(Recorded::NotRecorded) => return Err("ReservedCountryNaming.country_intel_sample not recorded"),
            Ok(Recorded::Value(Some(sample))) => sample,
            _ => return Err("ReservedCountryNaming.country_intel_sample capture refused"),
        };
        if sample.sample_hour > hour {
            return Err("ReservedCountryNaming.country_intel_sample is later than frame");
        }
        if sample.vector_count > i32::MAX as u32 {
            return Err("ReservedCountryNaming.country_intel_sample count outside native range");
        }
        complete(self.mirrors, TableId::CountryIntel).ok_or("CountryIntel")?;
        if zero_rows.iter().any(|&(at, _)| at >= sample.vector_count) {
            return Err("CountryIntel owner0 keys outside admitted vector or duplicated");
        }
        if sample.vector_count > 0 && zero_rows.last().map(|&(at, _)| at) != Some(sample.vector_count - 1) {
            return Err("CountryIntel owner0 final sentinel missing");
        }
        if target >= sample.vector_count {
            return Ok(0);
        }
        Ok(zero_rows.binary_search_by_key(&target, |&(at, _)| at).map_or(0, |index| zero_rows[index].1))
    }

    fn hour(&self) -> i32 {
        self.provinces.and_then(Mirror::head).unwrap_or(0)
    }

    fn cutoff(&self, raw_a: u32, raw_b: u32, zero_rows: &[(u32, i64)]) -> Result<i32, &'static str> {
        let Ok(fog) = &self.content.fog else { return Err("fog content (topology or NIntel defines) failed to load") };
        let cutoffs = &fog.defines.map_cutoffs;
        if cutoffs.len() < 2 {
            return Ok(0);
        }
        let a = self.slot(raw_a)?;
        let b = self.slot(raw_b)?;
        let relations = self.global.fog_relations.as_ref().map_err(|reason| *reason)?;
        let shared = if a == 0 {
            raw_b == 0 || self.reverse_zero_shared(b, self.hour())?
        } else if b == 0 {
            self.forward_zero_shared(a)?
        } else {
            relations.shared(a, b)
        };
        let fraction = if shared {
            100_000
        } else {
            let army = if a == 0 { self.zero_army(raw_b, b, self.hour(), zero_rows)? } else { self.army(a, b)? };
            intel_fraction(army, fog.defines.intel_maximums[1])?
        };
        Ok(map_cutoff(cutoffs, fraction))
    }
}


#[derive(Default)]
pub(super) struct Fold {
    terms: Vec<Vec<Term>>,
    bytes: Vec<u8>,
    present: BTreeSet<u32>,
    held: Vec<bool>,
    sources: HashMap<Source, Vec<(u32, Order)>>,
    source_dirty: Vec<Source>,
    source_marked: HashSet<Source>,
    province_dirty: Dirty,
    controller_of: HashMap<u32, Option<u32>>,
    state_of: HashMap<u32, u32>,
    controlled: BTreeMap<u32, BTreeSet<u32>>,
    states: BTreeMap<u32, BTreeSet<u32>>,
    fleet_tasks: HashMap<u64, BTreeSet<u64>>,
    task_fleet: HashMap<u64, u64>,
    host_slots: BTreeSet<u32>,
    stationed_units: HashSet<u64>,
    cutoffs: HashMap<(u32, u32), (Result<i32, &'static str>, HashSet<u32>)>,
    pairs: HashMap<u32, Vec<(u32, u32)>>,
    cutoffs_stale: bool,
    zero_stale: bool,
    spent: [u64; 4],
    intel_changed: Vec<(u32, u32)>,
    zero_rows: Vec<(u32, i64)>,
    sea: BTreeMap<u64, Result<Option<u32>, &'static str>>,
    sea_dirty: BTreeSet<u64>,
    errors: BTreeMap<ErrorKey, &'static str>,
    owned: HashMap<Owner, Vec<ErrorKey>>,
    rebuild: bool,
    since: Option<u64>,
    marked: u64,
    pub(super) changed: crate::layers::Changed,
}

impl Fold {
    pub(super) fn mark_all(&mut self, frame: u64) {
        self.rebuild = true;
        self.since.get_or_insert(frame);
    }

    pub(super) fn global_changed(&mut self, frame: u64, relations: bool) {
        self.cutoffs_stale = true;
        if relations {
            self.sea_dirty.extend(self.sea.keys().copied());
            self.mark_source(Source::Stationed);
        }
        self.since.get_or_insert(frame);
    }

    pub(super) fn marked(&self) -> u64 {
        self.marked
    }

    pub(super) fn carried(&self) -> usize {
        self.source_dirty.len() + self.province_dirty.len() + self.sea_dirty.len() + self.intel_changed.len() + usize::from(self.rebuild) + usize::from(self.cutoffs_stale) + usize::from(self.zero_stale)
    }


    pub(super) fn byte_of(&self, province: u32) -> u8 {
        self.bytes.get(province as usize).copied().unwrap_or(0)
    }

    pub(super) fn touch_all(&mut self) {
        self.changed.everything();
    }


    pub(super) fn gap(&self, content: &Content) -> Option<&'static str> {
        if content.fog.is_err() {
            return Some("fog content (topology or NIntel defines) failed to load");
        }
        self.errors.values().next().copied()
    }

    pub(super) fn province_bytes(&self) -> BTreeMap<u32, u8> {
        self.present.iter().map(|&id| (id, self.bytes.get(id as usize).copied().unwrap_or(0))).collect()
    }

    pub(super) fn sight(&self) -> BTreeMap<u64, u32> {
        self.sea.iter().filter_map(|(&division, sight)| sight.ok().flatten().map(|province| (division, province))).collect()
    }

    fn mark_source(&mut self, source: Source) -> u64 {
        if self.source_marked.insert(source) {
            self.source_dirty.push(source);
            1
        } else {
            0
        }
    }

    fn mark_province(&mut self, province: u32, frame: u64) {
        if self.province_dirty.mark(province, frame) {
            self.marked += 1;
        }
    }

    fn set_errors(&mut self, owner: Owner, errors: Vec<(ErrorKey, &'static str)>) {
        for key in self.owned.remove(&owner).unwrap_or_default() {
            self.errors.remove(&key);
        }
        if !errors.is_empty() {
            let keys = errors.iter().map(|(key, _)| *key).collect();
            self.errors.extend(errors);
            self.owned.insert(owner, keys);
        }
    }

    pub(super) fn status_changed(&mut self, table: TableId, mirror: &Mirror, frame: u64) -> u64 {
        if self.rebuild {
            return 0;
        }
        self.cutoffs_stale = true;
        self.since.get_or_insert(frame);
        let keys: Vec<u128> = mirror.rows().filter_map(|slot| mirror.key(slot)).collect();
        let mut marked = 0;
        match table {
            TableId::Provinces => {
                self.mark_all(frame);
                return 1;
            }
            TableId::States | TableId::StateVisibility => {
                let states: Vec<u32> = self.states.keys().copied().chain(keys.iter().map(|&key| key as u32)).collect();
                for state in states {
                    marked += self.mark_source(Source::Occupied(state));
                }
            }
            TableId::Divisions => {
                for &key in &keys {
                    marked += self.mark_source(Source::Division(key as u64));
                    self.sea_dirty.insert(key as u64);
                }
                let held: Vec<Source> = self.sources.keys().filter(|source| matches!(source, Source::Division(_))).copied().collect();
                for source in held {
                    marked += self.mark_source(source);
                }
                let sea: Vec<u64> = self.sea.keys().copied().collect();
                self.sea_dirty.extend(sea);
                marked += self.mark_source(Source::Stationed);
            }
            TableId::DivisionMovement => {
                let sea: Vec<u64> = self.sea.keys().copied().collect();
                self.sea_dirty.extend(sea);
            }
            TableId::TaskForces | TableId::Fleets => {
                let tasks: Vec<u64> = self.task_fleet.keys().copied().collect();
                for task in tasks {
                    marked += self.mark_source(Source::Task(task));
                }
                if table == TableId::TaskForces {
                    for &key in &keys {
                        marked += self.mark_source(Source::Task(key as u64));
                    }
                } else {
                    for &key in &keys {
                        marked += self.mark_source(Source::Fleet(key as u64));
                    }
                }
                marked += self.mark_source(Source::Stationed);
            }
            TableId::LandTheatres | TableId::TheatreAreas | TableId::StationedUnits => {
                marked += self.mark_source(Source::Areas) + self.mark_source(Source::Stationed);
            }
            TableId::VolunteerHosts => marked += self.mark_source(Source::Hosts),
            TableId::RadarCoverage => {
                let held: Vec<Source> = self.sources.keys().filter(|source| matches!(source, Source::Radar(_))).copied().collect();
                for source in held.into_iter().chain(keys.iter().map(|&key| Source::Radar(key))) {
                    marked += self.mark_source(source);
                }
            }
            _ => {}
        }
        marked
    }

    pub(super) fn apply(&mut self, table: TableId, changes: &Changes, mirror: &Mirror, columns: &Columns, frame: u64) -> u64 {
        if self.rebuild {
            return 0;
        }
        let keys = |changes: &Changes, wanted: &[usize]| -> Vec<u128> {
            let mut keys: Vec<u128> = changes.deleted().keys().unwrap_or_default().iter().chain(changes.inserted().keys().unwrap_or_default()).copied().collect();
            for written in changes.columns().iter().filter(|written| wanted.contains(&written.column())) {
                keys.extend(written.slots().iter().filter_map(|&slot| mirror.key(slot)));
            }
            keys
        };
        let mut marked = 0;
        match table {
            TableId::Provinces => {
                if !changes.deleted().is_empty() || !changes.inserted().is_empty() {
                    self.mark_all(frame);
                    return 1;
                }
                let controllers = changes.columns().iter().any(|written| written.column() == columns.controller.column && !written.is_empty());
                for key in keys(changes, &[columns.controller.column, columns.province_state.column]) {
                    marked += self.mark_source(Source::Controlled(key as u32));
                }
                if controllers {
                    self.sea_dirty.extend(self.sea.keys().copied());
                }
            }
            TableId::States | TableId::StateVisibility => {
                let wanted = [columns.state_owner.column, columns.state_controller.column, columns.network_modifier.column];
                for key in keys(changes, &wanted) {
                    marked += self.mark_source(Source::Occupied(key as u32));
                }
            }
            TableId::Divisions => {
                let wanted = [columns.roster_owner.column, columns.division_owner.column, columns.division_province.column, columns.reveal_skip.column];
                for key in keys(changes, &wanted) {
                    let division = key as u64;
                    marked += self.mark_source(Source::Division(division));
                    if self.stationed_units.contains(&division) {
                        marked += self.mark_source(Source::Stationed);
                    }
                    self.sea_dirty.insert(division);
                }
            }
            TableId::DivisionMovement => {
                for &key in changes.deleted().keys().unwrap_or_default().iter().chain(changes.inserted().keys().unwrap_or_default()) {
                    self.sea_dirty.insert(key as u64);
                }
                for key in keys(changes, &[columns.path.column]) {
                    if self.sea.contains_key(&(key as u64)) {
                        self.sea_dirty.insert(key as u64);
                    }
                }
            }
            TableId::TaskForces => {
                for key in keys(changes, &[columns.task_fleet.column, columns.task_province.column, columns.task_owner.column]) {
                    marked += self.mark_source(Source::Task(key as u64));
                    if self.stationed_units.contains(&(key as u64)) {
                        marked += self.mark_source(Source::Stationed);
                    }
                }
            }
            TableId::Fleets => {
                for key in keys(changes, &[columns.fleet_owner.column]) {
                    marked += self.mark_source(Source::Fleet(key as u64));
                }
            }
            TableId::LandTheatres | TableId::TheatreAreas => {
                marked += self.mark_source(Source::Areas) + self.mark_source(Source::Stationed);
            }
            TableId::StationedUnits => marked += self.mark_source(Source::Stationed),
            TableId::VolunteerHosts => marked += self.mark_source(Source::Hosts),
            TableId::RadarCoverage => {
                for key in keys(changes, &[columns.radar_provinces.column]) {
                    marked += self.mark_source(Source::Radar(key));
                }
            }
            TableId::CountryIntel => {
                for key in keys(changes, &[columns.army.column]) {
                    let (owner, target) = ((key >> 32) as u32, key as u32);
                    if owner == 0 {
                        self.zero_stale = true;
                    } else {
                        self.intel_changed.push((owner, target));
                    }
                }
            }
            TableId::ReservedCountryNaming | TableId::CountryCounterInputs | TableId::FactionMembers => self.zero_stale = true,
            _ => {}
        }
        if marked > 0 {
            self.since.get_or_insert(frame);
        }
        marked
    }

    fn start_over(&mut self, reads: &Reads<'_>, frame: u64) {
        let (kept_marked, changed, spent) = (self.marked, std::mem::take(&mut self.changed), self.spent);
        *self = Self { marked: kept_marked, changed, spent, ..Self::default() };
        self.touch_all();
        if !reads.global.fog || reads.global.failed.is_some() {
            return;
        }
        let mirrors = reads.mirrors;
        let keys = |table: TableId| -> Vec<u128> { mirrors.get(table).map_or_else(Vec::new, |rows| rows.rows().filter_map(|slot| rows.key(slot)).collect()) };
        if let Some(rows) = reads.provinces {
            self.present = rows.rows().filter_map(|slot| rows.key(slot)).map(|key| key as u32).collect();
            self.held = vec![false; self.present.last().map_or(0, |&id| id as usize + 1)];
            self.present.iter().for_each(|&id| self.held[id as usize] = true);
        }
        let present: Vec<u32> = self.present.iter().copied().collect();
        for &id in &present {
            self.mark_source(Source::Controlled(id));
            self.mark_province(id, frame);
        }
        for key in keys(TableId::States) {
            self.mark_source(Source::Occupied(key as u32));
        }
        for key in keys(TableId::Divisions) {
            self.mark_source(Source::Division(key as u64));
            self.sea_dirty.insert(key as u64);
        }
        for key in keys(TableId::Fleets) {
            self.mark_source(Source::Fleet(key as u64));
        }
        for key in keys(TableId::TaskForces) {
            self.mark_source(Source::Task(key as u64));
        }
        for key in keys(TableId::RadarCoverage) {
            self.mark_source(Source::Radar(key));
        }
        for source in [Source::Areas, Source::Hosts, Source::Stationed] {
            self.mark_source(source);
        }
        self.cutoffs_stale = true;
        self.tables_errors(reads);
    }

    fn tables_errors(&mut self, reads: &Reads<'_>) {
        let mut errors = Vec::new();
        for (group, table, name) in [
            (group::COUNTRIES, TableId::Countries, "Countries"),
            (group::PROVINCES, TableId::Provinces, "Provinces"),
            (group::DIVISIONS, TableId::Divisions, "Divisions"),
            (group::TASK_FORCES, TableId::TaskForces, "TaskForces"),
            (group::STATE_VISIBILITY, TableId::StateVisibility, "StateVisibility"),
            (group::STATES, TableId::States, "States"),
            (group::FLEETS_INPUT, TableId::Fleets, "Fleets"),
            (group::THEATRES_INPUT, TableId::LandTheatres, "LandTheatres"),
            (group::AREAS_INPUT, TableId::TheatreAreas, "TheatreAreas"),
            (group::STATIONED_INPUT, TableId::StationedUnits, "StationedUnits"),
            (group::HOSTS_INPUT, TableId::VolunteerHosts, "VolunteerHosts"),
            (group::RADAR_INPUT, TableId::RadarCoverage, "RadarCoverage"),
        ] {
            if complete(reads.mirrors, table).is_none() {
                errors.push(((INPUTS, at(group, 0)), name));
            }
        }
        if reads.global.viewer.is_none() {
            errors.push(((INPUTS, at(group::VIEWER, 0)), "visibility viewing country"));
        }
        let global = reads.global;
        if let Err(reason) = &global.relations {
            errors.push(((BEFORE, Order::default()), *reason));
        } else if let Err(reason) = &global.fog_relations {
            errors.push(((BEFORE, Order { sub: 1, ..Order::default() }), *reason));
        }
        if complete(reads.mirrors, TableId::Divisions).is_none() {
            errors.push(((INVASION, Order::default()), "Divisions"));
        } else if complete(reads.mirrors, TableId::DivisionMovement).is_none() {
            errors.push(((INVASION, Order { sub: 1, ..Order::default() }), "DivisionMovement"));
        }
        self.set_errors(Owner::Tables, errors);
    }

    fn walked(global: &Global, country: u32) -> Vec<(Order, u32)> {
        let Ok(relations) = &global.fog_relations else { return Vec::new() };
        if country == global.ours {
            return global.viewer.map(|native| (Order { foreign: false, slot: country, native, ..Order::default() }, native)).into_iter().collect();
        }
        if !relations.allied_for_fog(country, global.ours) {
            return Vec::new();
        }
        global.natives.get(&country).into_iter().flatten().map(|&native| (Order { foreign: true, slot: country, native, ..Order::default() }, native)).collect()
    }

    fn reveal(content: &Content, province: u32, base: i32, order: Order, out: &mut Vec<(u32, Order, Action)>) {
        out.push((province, order, Action::Set));
        Self::neighbors(content, province, base, Order { sub: order.sub + 1, ..order }, out);
    }

    fn neighbors(content: &Content, province: u32, base: i32, order: Order, out: &mut Vec<(u32, Order, Action)>) {
        for (index, &next) in content.neighbors(province).into_iter().flatten().enumerate() {
            if content.land(next) {
                out.push((next, Order { sub: order.sub + index as u32, ..order }, Action::Stamp { from: province, base }));
            }
        }
    }

    fn generate(&mut self, source: Source, reads: &Reads<'_>, frame: u64) -> (Vec<(u32, Order, Action)>, Vec<(ErrorKey, &'static str)>) {
        let (content, global, columns, mirrors) = (reads.content, reads.global, reads.columns, reads.mirrors);
        let mut terms = Vec::new();
        let mut errors = Vec::new();
        match source {
            Source::Controlled(id) => {
                let row = reads.provinces.and_then(|rows| rows.slot(u128::from(id)).map(|slot| (rows, slot)));
                let (raw, state) = row.map_or((0, 0), |(rows, slot)| (rows.number(columns.controller, slot).value().unwrap_or(0), rows.number(columns.province_state, slot).value().unwrap_or(0)));
                if row.is_some() && content.neighbors(id).is_none() {
                    errors.push(((INPUTS, at(group::TOPOLOGY, u128::from(id))), "visibility province topology"));
                }
                let owned = if row.is_some() { reads.owned(raw) } else { Ok(None) };
                if let Err(reason) = owned {
                    errors.push(((INPUTS, at(group::CONTROLLED, u128::from(id))), reason));
                }
                let slot = owned.ok().flatten();
                let old = self.controller_of.insert(id, slot).flatten();
                if old != slot {
                    if let Some(old) = old {
                        self.controlled.entry(old).or_default().remove(&id);
                    }
                    if let Some(new) = slot {
                        self.controlled.entry(new).or_default().insert(id);
                    }
                    if old.is_some_and(|old| self.host_slots.contains(&old)) || slot.is_some_and(|new| self.host_slots.contains(&new)) {
                        self.mark_source(Source::Hosts);
                    }
                    self.mark_province(id, frame);
                    for &next in content.neighbors(id).into_iter().flatten() {
                        self.mark_province(next, frame);
                    }
                }
                let old_state = if row.is_some() { self.state_of.insert(id, state) } else { self.state_of.remove(&id) };
                if old_state != row.map(|_| state) {
                    if let Some(old_state) = old_state.filter(|&state| state != 0) {
                        self.states.entry(old_state).or_default().remove(&id);
                        self.mark_source(Source::Occupied(old_state));
                    }
                    if row.is_some() && state != 0 {
                        self.states.entry(state).or_default().insert(id);
                        self.mark_source(Source::Occupied(state));
                    }
                }
                for (order, _) in slot.map(|slot| Self::walked(global, slot)).unwrap_or_default() {
                    Self::reveal(content, id, 150, Order { phase: PHASE_CONTROLLED, a: u64::from(id), ..order }, &mut terms);
                }
            }
            Source::Occupied(state) => {
                if let Some(rows) = complete(mirrors, TableId::States)
                    && let Some(slot) = rows.slot(u128::from(state))
                {
                    let owner = rows.number(columns.state_owner, slot).value().unwrap_or(0);
                    let controller = rows.number(columns.state_controller, slot).value().unwrap_or(0);
                    let canonical = |raw| reads.slot(raw);
                    let decided = (|| -> Result<Option<u32>, &'static str> {
                        let differs = canonical(owner)? != canonical(controller)?;
                        let owned = reads.owned(owner)?;
                        Ok(owned.filter(|_| differs))
                    })();
                    match decided {
                        Err(reason) => errors.push(((INPUTS, at(group::OCCUPIED, u128::from(state))), reason)),
                        Ok(Some(country)) => {
                            let network = complete(mirrors, TableId::StateVisibility).and_then(|networks| networks.slot(u128::from(state)));
                            if network.is_some() {
                                let provinces: Vec<u32> = self.states.get(&state).into_iter().flatten().copied().collect();
                                for (order, native) in Self::walked(global, country) {
                                    for &province in &provinces {
                                        terms.push((province, Order { phase: PHASE_OCCUPIED, a: u64::from(state), b: u64::from(province), ..order }, Action::Network { raw_country: native, state }));
                                    }
                                }
                            }
                        }
                        Ok(None) => {}
                    }
                }
            }
            Source::Division(division) => {
                if let Some(rows) = complete(mirrors, TableId::Divisions)
                    && let Some(slot) = rows.slot(u128::from(division))
                {
                    let roster = rows.number(columns.roster_owner, slot).value().unwrap_or(0);
                    match reads.owned(roster) {
                        Err(reason) => errors.push(((INPUTS, at(group::ROSTERS, u128::from(division))), reason)),
                        Ok(Some(country)) => {
                            let skip = rows.number(columns.reveal_skip, slot).value().unwrap_or(0);
                            let province = rows.number(columns.division_province, slot).value().unwrap_or(0);
                            if skip == 0 && province != 0 {
                                for (order, _) in Self::walked(global, country) {
                                    Self::reveal(content, province, 250, Order { phase: PHASE_DIVISIONS, a: division, ..order }, &mut terms);
                                }
                            }
                        }
                        Ok(None) => {}
                    }
                }
            }
            Source::Fleet(fleet) => {
                if let Some(rows) = complete(mirrors, TableId::Fleets)
                    && let Some(slot) = rows.slot(u128::from(fleet))
                    && let Err(reason) = reads.owned(rows.number(columns.fleet_owner, slot).value().unwrap_or(0))
                {
                    errors.push(((INPUTS, at(group::FLEETS, u128::from(fleet))), reason));
                }
                let tasks: Vec<u64> = self.fleet_tasks.get(&fleet).into_iter().flatten().copied().collect();
                tasks.into_iter().for_each(|task| {
                    self.mark_source(Source::Task(task));
                });
            }
            Source::Task(task) => {
                let row = complete(mirrors, TableId::TaskForces).and_then(|rows| rows.slot(u128::from(task)).map(|slot| (rows, slot)));
                let fleet = row.map(|(rows, slot)| (rows.number(columns.task_fleet, slot).value().unwrap_or(0), rows.number(columns.task_province, slot).value().unwrap_or(0)));
                let old = match fleet {
                    Some((fleet, _)) => self.task_fleet.insert(task, fleet),
                    None => self.task_fleet.remove(&task),
                };
                if old != fleet.map(|(fleet, _)| fleet) {
                    if let Some(old) = old {
                        self.fleet_tasks.entry(old).or_default().remove(&task);
                    }
                    if let Some((new, _)) = fleet {
                        self.fleet_tasks.entry(new).or_default().insert(task);
                    }
                }
                if let Some((fleet, province)) = fleet.filter(|&(_, province)| province != 0) {
                    let owner = complete(mirrors, TableId::Fleets).and_then(|rows| rows.slot(u128::from(fleet)).and_then(|slot| rows.number(columns.fleet_owner, slot).value()));
                    if let Some(Ok(Some(country))) = owner.map(|raw| reads.owned(raw)) {
                        for (order, _) in Self::walked(global, country) {
                            let order = Order { phase: PHASE_TASKS, a: fleet, b: task, ..order };
                            if !reads.present(province) {
                                errors.push(((WALK, order), "visibility source province"));
                            } else {
                                Self::neighbors(content, province, 250, order, &mut terms);
                            }
                        }
                    }
                }
            }
            Source::Radar(key) => {
                if let Some(rows) = complete(mirrors, TableId::RadarCoverage)
                    && let Some(slot) = rows.slot(key)
                {
                    let raw = (key >> 32) as u32;
                    match reads.owned(raw) {
                        Err(reason) => errors.push(((INPUTS, at(group::RADAR, key)), reason)),
                        Ok(Some(country)) => {
                            let provinces: Vec<u32> = u32s(rows, columns.radar_provinces, slot).collect();
                            for (order, native) in Self::walked(global, country) {
                                for (index, &province) in provinces.iter().enumerate() {
                                    terms.push((province, Order { phase: PHASE_RADAR, a: u64::from(key as u32), b: key as u64 >> 32, sub: index as u32, ..order }, Action::Radar { raw_country: native }));
                                }
                            }
                        }
                        Ok(None) => {}
                    }
                }
            }
            Source::Areas | Source::Stationed => self.theatres(source, reads, &mut terms, &mut errors),
            Source::Hosts => {
                self.host_slots.clear();
                if let Some(rows) = complete(mirrors, TableId::VolunteerHosts) {
                    let mut keys: Vec<(u128, u32)> = rows.rows().filter_map(|slot| rows.key(slot).map(|key| (key, slot))).collect();
                    keys.sort_unstable();
                    for (key, slot) in keys {
                        match reads.owned((key >> 32) as u32) {
                            Err(reason) => errors.push(((INPUTS, at(group::HOSTS, key)), reason)),
                            Ok(Some(country)) if country == global.ours => {
                                let order = Order { foreign: false, slot: global.ours, native: global.viewer.unwrap_or(0), phase: PHASE_HOSTS, a: (key >> 64) as u64, b: key as u64, ..Order::default() };
                                match reads.slot(rows.number(columns.host, slot).value().unwrap_or(0)) {
                                    Err(reason) => errors.push(((WALK, order), reason)),
                                    Ok(host) => {
                                        self.host_slots.insert(host);
                                        if global.viewer.is_some() && global.fog_relations.is_ok() {
                                            let provinces: Vec<u32> = self.controlled.get(&host).into_iter().flatten().copied().collect();
                                            for province in provinces {
                                                Self::reveal(content, province, 150, Order { sub: province * 64, ..order }, &mut terms);
                                            }
                                        }
                                    }
                                }
                            }
                            Ok(_) => {}
                        }
                    }
                }
            }
        }
        (terms, errors)
    }

    fn theatres(&mut self, source: Source, reads: &Reads<'_>, terms: &mut Vec<(u32, Order, Action)>, errors: &mut Vec<(ErrorKey, &'static str)>) {
        let (content, global, columns, mirrors) = (reads.content, reads.global, reads.columns, reads.mirrors);
        let Some(theatres) = complete(mirrors, TableId::LandTheatres) else { return };
        let mut rows: Vec<(u64, u32, u32, u32, i32)> = theatres
            .rows()
            .filter_map(|slot| {
                let key = theatres.key(slot)? as u64;
                Some((key, slot, theatres.number(columns.theatre_owner, slot).value().unwrap_or(0), theatres.number(columns.theatre_ordinal, slot).value().unwrap_or(0), theatres.number(columns.volunteers, slot).value().unwrap_or(0)))
            })
            .collect();
        rows.sort_unstable();
        let mut owners = BTreeMap::new();
        for &(id, _, owner, _, volunteers) in &rows {
            match reads.owned(owner) {
                Err(reason) => {
                    errors.push(((INPUTS, at(group::OWN_THEATRES, u128::from(id))), reason));
                    continue;
                }
                Ok(slot) => {
                    owners.insert(id, slot);
                }
            }
            if volunteers > 0
                && let Err(reason) = reads.slot(volunteers as u32)
            {
                errors.push(((INPUTS, at(group::VOLUNTEERS_HERE, u128::from(id))), reason));
            }
        }
        if source == Source::Areas {
            let Some(viewer) = global.viewer.filter(|_| global.fog_relations.is_ok()) else { return };
            let Some(areas) = complete(mirrors, TableId::TheatreAreas) else { return };
            let mut by_theatre: BTreeMap<u64, Vec<(u32, u32)>> = BTreeMap::new();
            for slot in areas.rows() {
                if let Some(key) = areas.key(slot) {
                    by_theatre.entry((key >> 32) as u64).or_default().push((key as u32, slot));
                }
            }
            for &(id, _, _, _, volunteers) in &rows {
                if volunteers <= 0 || owners.get(&id) != Some(&Some(global.ours)) {
                    continue;
                }
                let mut held = by_theatre.remove(&id).unwrap_or_default();
                held.sort_unstable();
                for (ordinal, slot) in held {
                    let provinces: Vec<u32> = u32s(areas, columns.area_provinces, slot).collect();
                    if provinces.is_empty() {
                        break;
                    }
                    for (index, province) in provinces.into_iter().enumerate() {
                        let order = Order { foreign: false, slot: global.ours, native: viewer, phase: PHASE_AREAS, a: id, b: u64::from(ordinal), sub: index as u32 * 64 };
                        Self::reveal(content, province, 150, order, terms);
                    }
                }
            }
            return;
        }
        self.stationed_units.clear();
        let Ok(relations) = &global.fog_relations else { return };
        let Some(stationed) = complete(mirrors, TableId::StationedUnits) else { return };
        let mut by_theatre: BTreeMap<u64, Vec<(u32, u32)>> = BTreeMap::new();
        for slot in stationed.rows() {
            if let Some(key) = stationed.key(slot) {
                by_theatre.entry((key >> 32) as u64).or_default().push((key as u32, slot));
            }
        }
        let mut chosen: BTreeMap<u32, (u32, u64)> = BTreeMap::new();
        for &(id, _, _, ordinal, volunteers) in &rows {
            let Some(Some(country)) = owners.get(&id) else { continue };
            if volunteers <= 0 || reads.slot(volunteers as u32) != Ok(global.ours) {
                continue;
            }
            let held = chosen.entry(*country).or_insert((ordinal, id));
            if ordinal < held.0 {
                *held = (ordinal, id);
            }
        }
        for (country, (_, theatre)) in chosen {
            if country == global.ours || relations.allied_for_fog(country, global.ours) {
                continue;
            }
            for &native in global.natives.get(&country).into_iter().flatten() {
                let place = Order { foreign: true, slot: country, native, phase: PHASE_STATIONED, ..Order::default() };
                let mut held = by_theatre.get(&theatre).cloned().unwrap_or_default();
                held.sort_unstable();
                for (ordinal, slot) in held {
                    let order = Order { a: u64::from(ordinal), ..place };
                    let unit = stationed.number(columns.stationed_unit, slot).value().unwrap_or(0);
                    self.stationed_units.insert(unit);
                    let found = complete(mirrors, TableId::Divisions)
                        .and_then(|rows| rows.slot(u128::from(unit)).map(|slot| (rows.number(columns.division_owner, slot).value().unwrap_or(0), rows.number(columns.division_province, slot).value().unwrap_or(0))))
                        .or_else(|| {
                            complete(mirrors, TableId::TaskForces).and_then(|rows| rows.slot(u128::from(unit)).map(|slot| (rows.number(columns.task_owner, slot).value().unwrap_or(0), rows.number(columns.task_province, slot).value().unwrap_or(0))))
                        });
                    let Some((owner, province)) = found else {
                        errors.push(((WALK, order), "StationedUnits unit"));
                        break;
                    };
                    match reads.slot(owner) {
                        Err(reason) => {
                            errors.push(((WALK, order), reason));
                            break;
                        }
                        Ok(slot) if slot == global.ours && province != 0 => Self::reveal(content, province, 250, Order { sub: 0, ..order }, terms),
                        Ok(_) => {}
                    }
                }
            }
        }
    }

    fn regenerate(&mut self, source: Source, reads: &Reads<'_>, frame: u64) {
        let (terms, errors) = self.generate(source, reads, frame);
        for (province, order) in self.sources.remove(&source).unwrap_or_default() {
            if let Some(held) = self.terms.get_mut(province as usize)
                && let Ok(index) = held.binary_search_by(|term| term.order.cmp(&order))
            {
                held.remove(index);
            }
            self.mark_province(province, frame);
        }
        let mut placed = Vec::with_capacity(terms.len());
        for (province, order, action) in terms {
            let at = province as usize;
            if at >= self.terms.len() {
                self.terms.resize_with(at + 1, Vec::new);
                self.bytes.resize(at + 1, 0);
            }
            let held = &mut self.terms[at];
            let index = held.binary_search_by(|term| term.order.cmp(&order)).unwrap_or_else(|index| index);
            held.insert(index, Term { order, action });
            placed.push((province, order));
            self.mark_province(province, frame);
        }
        if !placed.is_empty() {
            self.sources.insert(source, placed);
        }
        self.set_errors(Owner::Source(source), errors);
    }

    fn refold(&mut self, province: u32, reads: &Reads<'_>) {
        for pair in self.pairs.remove(&province).unwrap_or_default() {
            if let Some((_, readers)) = self.cutoffs.get_mut(&pair) {
                readers.remove(&province);
            }
        }
        let terms = std::mem::take(self.terms.get_mut(province as usize).unwrap_or(&mut Vec::new()));
        let present = reads.present(province);
        let land = reads.content.land(province);
        let mut byte = 0u8;
        let mut refusal = None;
        let mut read = Vec::new();
        for term in &terms {
            let evaluated = (|| -> Result<(), &'static str> {
                let mut cutoff = |raw_a: u32, raw_b: u32| -> Result<i32, &'static str> {
                    read.push((raw_a, raw_b));
                    self.cutoff(raw_a, raw_b, reads)
                };
                match term.action {
                    Action::Set => {
                        if !present {
                            return Err("visibility revealed province");
                        }
                        byte = 255;
                    }
                    Action::Stamp { from, base } => {
                        let source = reads.controller(from).ok_or("visibility source province")?;
                        let other = reads.controller(province).ok_or("visibility neighbour controller")?;
                        if reads.equal(other, source)? {
                            return Ok(());
                        }
                        let value = base.wrapping_add(cutoff(source, other)?).clamp(0, 255) as u8;
                        byte = byte.max(value);
                    }
                    Action::Network { raw_country, state } => {
                        let states = complete(reads.mirrors, TableId::States).ok_or("States")?;
                        let Some(slot) = states.slot(u128::from(state)) else { return Ok(()) };
                        let controller = states.number(reads.columns.state_controller, slot).value().unwrap_or(0);
                        let networks = complete(reads.mirrors, TableId::StateVisibility).ok_or("StateVisibility")?;
                        let Some(network) = networks.slot(u128::from(state)) else { return Ok(()) };
                        let modifier = networks.number(reads.columns.network_modifier, network).value().unwrap_or(0);
                        let term = network_term(cutoff(raw_country, controller)?, modifier);
                        if !present {
                            return Err("visibility additive province");
                        }
                        byte = if land { saturate(byte, term) } else { 0 };
                    }
                    Action::Radar { raw_country } => {
                        let controller = reads.controller(province).ok_or("RadarCoverage province")?;
                        if reads.equal(controller, raw_country)? {
                            return Ok(());
                        }
                        let Ok(fog) = &reads.content.fog else { return Err("fog content (topology or NIntel defines) failed to load") };
                        let term = fog.defines.radar_effect.wrapping_add(cutoff(raw_country, controller)?);
                        byte = if land { saturate(byte, term) } else { 0 };
                    }
                }
                Ok(())
            })();
            if let Err(reason) = evaluated {
                refusal = Some(((WALK, term.order), reason));
                break;
            }
        }
        let at = province as usize;
        if at < self.terms.len() {
            self.terms[at] = terms;
        } else if present {
            self.terms.resize_with(at + 1, Vec::new);
            self.bytes.resize(at + 1, 0);
        }
        if at < self.bytes.len() && self.bytes[at] != byte {
            self.bytes[at] = byte;
            self.changed.mark("province", u64::from(province));
        }
        read.sort_unstable();
        read.dedup();
        for &pair in &read {
            if let Some((_, readers)) = self.cutoffs.get_mut(&pair) {
                readers.insert(province);
            }
        }
        if !read.is_empty() {
            self.pairs.insert(province, read);
        }
        self.set_errors(Owner::Province(province), refusal.into_iter().collect());
    }

    fn cutoff(&mut self, raw_a: u32, raw_b: u32, reads: &Reads<'_>) -> Result<i32, &'static str> {
        if let Some((value, _)) = self.cutoffs.get(&(raw_a, raw_b)) {
            return *value;
        }
        let value = reads.cutoff(raw_a, raw_b, &self.zero_rows);
        self.cutoffs.insert((raw_a, raw_b), (value, HashSet::new()));
        value
    }

    fn reread_cutoffs(&mut self, reads: &Reads<'_>, frame: u64, which: impl Fn(u32, u32) -> bool) {
        if which(0, 0) {
            self.zero_rows = complete(reads.mirrors, TableId::CountryIntel)
                .map(|rows| owner_rows(rows, 0).into_iter().map(|(target, slot)| (target, rows.number(reads.columns.army, slot).value().unwrap_or(0))).collect())
                .unwrap_or_default();
        }
        let pairs: Vec<(u32, u32)> = self.cutoffs.keys().copied().filter(|&(a, b)| which(a, b)).collect();
        let mut stale = Vec::new();
        for pair in pairs {
            let value = reads.cutoff(pair.0, pair.1, &self.zero_rows);
            let held = self.cutoffs.get_mut(&pair).expect("held");
            if held.0 != value {
                held.0 = value;
                stale.extend(held.1.drain());
            }
        }
        for province in stale {
            self.mark_province(province, frame);
        }
    }

    fn settle_sea(&mut self, division: u64, reads: &Reads<'_>) {
        let columns = reads.columns;
        let (Some(divisions), Some(moves)) = (complete(reads.mirrors, TableId::Divisions), complete(reads.mirrors, TableId::DivisionMovement)) else {
            self.sea.remove(&division);
            self.set_errors(Owner::Sea(division), Vec::new());
            return;
        };
        let Some(slot) = divisions.slot(u128::from(division)) else {
            self.sea.remove(&division);
            self.set_errors(Owner::Sea(division), Vec::new());
            return;
        };
        let province = divisions.number(columns.division_province, slot).value().unwrap_or(0);
        if !reads.content.sea(province) {
            self.sea.remove(&division);
            self.set_errors(Owner::Sea(division), Vec::new());
            return;
        }
        let sight = (|| -> Result<Option<u32>, &'static str> {
            let moving = moves.slot(u128::from(division)).ok_or("DivisionMovement division")?;
            let Some(next) = u32s(moves, columns.path, moving).next() else { return Ok(None) };
            if !reads.content.land(next) {
                return Ok(None);
            }
            let controller = complete(reads.mirrors, TableId::Provinces)
                .and_then(|rows| rows.slot(u128::from(next)).and_then(|slot| rows.number(columns.controller, slot).value()))
                .and_then(|raw| reads.global.links.get(&raw).copied())
                .ok_or("invasion target controller")?;
            let owner = *reads.global.links.get(&divisions.number(columns.division_owner, slot).value().unwrap_or(0)).ok_or("invasion division owner")?;
            let relations = reads.global.fog_relations.as_ref().map_err(|reason| *reason)?;
            Ok(relations.at_war(controller, owner).then_some(next))
        })();
        let errors = match sight {
            Err(reason) => vec![((INVASION, Order { a: division, sub: 2, ..Order::default() }), reason)],
            Ok(_) => Vec::new(),
        };
        self.sea.insert(division, sight);
        self.set_errors(Owner::Sea(division), errors);
    }

    pub(super) fn settle(&mut self, content: &Content, global: &Global, columns: &Columns, mirrors: &Mirrors, frame: u64) {
        let reads = Reads { content, global, columns, mirrors, provinces: mirrors.get(TableId::Provinces) };
        if self.rebuild {
            self.start_over(&reads, frame);
        }
        if !global.fog || global.failed.is_some() {
            return;
        }
        let started = std::time::Instant::now();
        if self.cutoffs_stale {
            self.cutoffs_stale = false;
            self.zero_stale = true;
            self.intel_changed.clear();
            self.tables_errors(&reads);
            self.reread_cutoffs(&reads, frame, |_, _| true);
        }
        if self.zero_stale || !self.intel_changed.is_empty() {
            let zero = std::mem::take(&mut self.zero_stale);
            let mut changed: Vec<(u32, u32)> = std::mem::take(&mut self.intel_changed).into_iter().filter_map(|(owner, target)| Some((*global.links.get(&owner)?, target))).collect();
            changed.sort_unstable();
            changed.dedup();
            let links = &global.links;
            let slot = |raw: u32| if raw == 0 { Some(0) } else { links.get(&raw).copied() };
            self.reread_cutoffs(&reads, frame, |a, b| (zero && (a == 0 || b == 0)) || (slot(a).zip(slot(b)).is_some_and(|pair| changed.binary_search(&pair).is_ok())));
        }
        let cutoffs = std::time::Instant::now();
        self.spent[0] += (cutoffs - started).as_nanos() as u64;
        let mut done = 0;
        while done < self.source_dirty.len() {
            let source = self.source_dirty[done];
            self.source_marked.remove(&source);
            done += 1;
            self.regenerate(source, &reads, frame);
        }
        self.source_dirty.drain(..done);
        let sources = std::time::Instant::now();
        self.spent[1] += (sources - cutoffs).as_nanos() as u64;
        let mut dirty = std::mem::take(&mut self.province_dirty);
        let _ = dirty.settle(|province| {
            self.refold(province, &reads);
            Ok(())
        });
        self.province_dirty = dirty;
        let provinces = std::time::Instant::now();
        self.spent[2] += (provinces - sources).as_nanos() as u64;
        while let Some(&division) = self.sea_dirty.first() {
            self.sea_dirty.remove(&division);
            self.settle_sea(division, &reads);
        }
        self.spent[3] += provinces.elapsed().as_nanos() as u64;
        if self.carried() == 0 {
            self.since = None;
        }
    }

    pub(super) fn spent(&self) -> [u64; 4] {
        self.spent
    }

    pub(super) fn held(&self) -> usize {
        self.terms.capacity() * 24
            + self.terms.iter().map(|terms| terms.capacity() * std::mem::size_of::<Term>()).sum::<usize>()
            + self.bytes.capacity()
            + self.present.len() * 8
            + self.sources.capacity() * 48
            + self.sources.values().map(|placed| placed.capacity() * std::mem::size_of::<(u32, Order)>()).sum::<usize>()
            + self.province_dirty.bytes()
            + (self.controller_of.capacity() + self.state_of.capacity()) * 16
            + self.controlled.values().chain(self.states.values()).map(|set| set.len() * 8).sum::<usize>()
            + self.cutoffs.capacity() * 64
            + self.cutoffs.values().map(|(_, readers)| readers.capacity() * 8).sum::<usize>()
            + self.pairs.values().map(|pairs| pairs.capacity() * 8 + 32).sum::<usize>()
            + self.sea.len() * 32
            + self.errors.len() * 64
    }
}
