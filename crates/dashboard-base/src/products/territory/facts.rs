//! Territory facts, projected hour by hour from a columnar walk ([`replay::segments`]): each
//! held tag's native holder, the controlled, owned and occupied totals of every holder whose
//! sums the hour changed (every holder's after a whole hour), the world's totals, the owned-state
//! counters by canonical slot and each regional series' sums. States and provinces are kept from
//! the hour's changes and their weights move by difference, so an hour reads only the rows it
//! changed. What a tag held before the segment is the ordered emit's ([`super::emit`]).

use super::columns;
use crate::{products::territory_breakdown::Regions, protocol::MAX_EXACT_JS_INTEGER};
use dashboard_world::{Needs, Roster, Tagged, World};
use installed_content_province_area::province_area::ProvinceAreas;
use replay::{
    columnar_frames::metrics,
    columns as walked,
    segments::{self, Item, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{CountryCounterInputRow, ProvinceRow, StateRow, TableId},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    mem::size_of,
    sync::Arc,
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::States,
    TableId::Provinces,
    TableId::CountryCounterInputs,
];

/// What a projection reads of its walk's world: the countries.
pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

/// A column's values, in the wire order ([`columns`]).
pub type Values = [Option<i64>; columns::COUNT];

/// A held tag of the hour: every UTF-8 tag a country holds but the empty one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Held {
    pub tag: Arc<str>,
    /// Its first holder by id the links resolve, as (canonical slot, id).
    pub identity: Option<(u32, u32)>,
    /// Its holder's dense index, where no other tag names the same slot.
    pub entry: Option<u32>,
    /// Whether a link names its unambiguous slot: regional series of it stay open.
    pub linked: bool,
}

/// The countries the hour's complete Countries and CountryLinks resolve.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lineage {
    /// Ascending by tag bytes.
    pub tags: Vec<Held>,
    /// A non-UTF-8 tag, an unresolved tag or a slot two tags name: world totals are unknown.
    pub incomplete: bool,
    /// The slots two tags name.
    pub ambiguous: usize,
}

impl Lineage {
    fn heap(&self) -> usize {
        self.tags.capacity() * size_of::<Held>()
            + self.tags.iter().map(|held| held.tag.len()).sum::<usize>()
    }
}

/// A regional series' key: its holder's tag, its strategic region and why either is missing.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RegionKey {
    pub tag: Option<Arc<str>>,
    pub region: Option<u32>,
    pub missing: Option<Arc<str>>,
    /// A non-null holder no tag names: its runs restart every hour.
    pub unknown: bool,
}

/// A regional series' sums: provinces, victory points and area, controlled then owned, and
/// each rule's state shares by denominator; none where the read is not available.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionValues {
    pub values: [Option<i64>; 6],
    pub states: [Option<BTreeMap<u32, u64>>; 2],
}

impl RegionValues {
    /// Zero in each available measure.
    pub fn zero(available: [bool; 8]) -> Self {
        Self {
            values: [1, 2, 3, 5, 6, 7].map(|column| available[column].then_some(0)),
            states: [0, 4].map(|column| available[column].then(BTreeMap::new)),
        }
    }

    fn heap(&self) -> usize {
        self.states
            .iter()
            .flatten()
            .map(|terms| terms.len() * 32)
            .sum()
    }
}

/// The regional facts the hour changed.
pub struct Regional {
    /// Every series with rows is listed; the others hold none.
    pub whole: bool,
    /// The keys new to this segment's numbering, and whether its numbering starts anew.
    pub fresh: bool,
    pub keys: Vec<(u32, RegionKey)>,
    /// Each listed series' sums, none where it holds no row.
    pub series: Vec<(u32, Option<Arc<RegionValues>>)>,
    /// State and province ids first seen this segment without a regional attribution.
    pub missing_states: Vec<u32>,
    pub missing_provinces: Vec<u32>,
}

/// The owned-state counters by canonical slot: every slot's where `whole`, else the slots
/// whose counters changed. A slot two rows name holds no count; one no row names is not held.
#[derive(Default)]
pub struct Counters {
    /// CountryCounterInputs and the countries are complete.
    pub complete: bool,
    pub whole: bool,
    /// Each slot's count, and whether a row names it.
    pub slots: Vec<(u32, Option<i64>, bool)>,
}

/// What the hour counts towards the family's warnings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rates {
    /// Totals past the exact integer range.
    pub overflowed: u64,
    /// A read incomplete or unavailable.
    pub incomplete: bool,
}

/// What the ordered emit reads of an hour that changed what territory reads.
#[derive(Default)]
pub struct Facts {
    /// Everything is listed anew: the segment's first hour.
    pub whole: bool,
    /// The countries where they changed: none while they are not complete.
    pub lineage: Option<Option<Arc<Lineage>>>,
    /// Each dense holder's values where they changed; every holder's where `whole` or the
    /// lineage changed.
    pub values: Vec<(u32, Values)>,
    pub world: Option<Values>,
    pub rates: Option<Rates>,
    /// Whether States and Provinces were freshly read and complete, where it changed.
    pub land: Option<[bool; 2]>,
    /// The owned-state counters where they changed.
    pub owned: Option<Counters>,
    /// The ambiguous slots of a lineage the hour's country or link changes read anew, which
    /// the row census counts at each such read.
    pub recount: Option<usize>,
    pub regional: Option<Regional>,
}

/// One hour of a segment.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// None when the hour changes nothing territory reads.
    pub facts: Option<Box<Facts>>,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        let Some(facts) = &self.facts else {
            return 0;
        };
        size_of::<Facts>()
            + match &facts.lineage {
                Some(Some(lineage)) => size_of::<Lineage>() + lineage.heap(),
                _ => 0,
            }
            + facts.values.capacity() * size_of::<(u32, Values)>()
            + facts.owned.as_ref().map_or(0, |owned| {
                owned.slots.capacity() * size_of::<(u32, Option<i64>, bool)>()
            })
            + facts.regional.as_ref().map_or(0, |regional| {
                regional.keys.capacity() * size_of::<(u32, RegionKey)>()
                    + regional.series.capacity() * size_of::<(u32, Option<Arc<RegionValues>>)>()
                    + regional
                        .series
                        .iter()
                        .flat_map(|(_, values)| values)
                        .map(|values| size_of::<RegionValues>() + values.heap())
                        .sum::<usize>()
                    + (regional.missing_states.capacity() + regional.missing_provinces.capacity())
                        * 4
            })
    }
}

/// A raw holder reference: null holds nothing; one no unambiguous tag names is unknown.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Holder {
    Unassigned,
    Unknown,
    Country(u32),
}

/// Occupied territory: a controller other than the owner, both known.
fn occupied(controller: Holder, owner: Holder) -> Holder {
    match (controller, owner) {
        (Holder::Unknown, _) | (_, Holder::Unknown) => Holder::Unknown,
        (Holder::Country(controller), Holder::Country(owner)) if controller != owner => {
            Holder::Country(controller)
        }
        _ => Holder::Unassigned,
    }
}

#[derive(Clone, Copy)]
struct StateCell {
    id: u32,
    controller: u32,
    owner: u32,
}

#[derive(Clone, Copy)]
struct ProvinceCell {
    id: u32,
    controller: u32,
    owner: u32,
    victory_points: i32,
    /// None where the map holds no area for it.
    area: Option<i64>,
}

/// A holder's or the world's sums: each column's total, and each rule's provinces without an
/// area.
#[derive(Clone, Copy, Default)]
struct Sums {
    totals: [i128; columns::COUNT],
    missing: [u32; 3],
}

/// A regional series' running sums.
#[derive(Clone, Default)]
struct RegionSums {
    provinces: [i128; 6],
    missing: [u32; 2],
    province_rows: u32,
    states: [BTreeMap<u32, u64>; 2],
    state_rows: u32,
}

impl RegionSums {
    fn heap(&self) -> usize {
        self.states.iter().map(|terms| terms.len() * 32).sum()
    }
}

/// The fields a projection reads.
struct Fields {
    state_controller: usize,
    state_owner: usize,
    province_controller: usize,
    province_owner: usize,
    province_points: usize,
    counter_owned: usize,
}

impl Fields {
    fn new() -> io::Result<Self> {
        Ok(Self {
            state_controller: walked::field(TableId::States, "controller")?,
            state_owner: walked::field(TableId::States, "owner")?,
            province_controller: walked::field(TableId::Provinces, "controller")?,
            province_owner: walked::field(TableId::Provinces, "owner")?,
            province_points: walked::field(TableId::Provinces, "victory_points")?,
            counter_owned: walked::field(TableId::CountryCounterInputs, "owned_state_count")?,
        })
    }

    fn states(&self) -> u64 {
        1 << self.state_controller | 1 << self.state_owner
    }

    fn provinces(&self) -> u64 {
        1 << self.province_controller | 1 << self.province_owner | 1 << self.province_points
    }

    fn counters(&self) -> u64 {
        1 << self.counter_owned
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

/// The reason a regional series' holder or region is missing.
const UNKNOWN_HOLDER: &str = "Country holder not recorded or ambiguous";
const PROVINCE_REGION: &str = "Region membership missing or conflicting";
const STATE_REGION: &str =
    "State province roster absent or empty, or region membership missing or conflicting";

/// A column's rule (controlled, owned, occupied) and whether it reads States.
fn group(column: usize) -> usize {
    column / 4 * 2 + usize::from(column % 4 != 0)
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"territory projector: cold";

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    areas: Option<Arc<ProvinceAreas>>,
    regions: Option<Arc<Regions>>,
    fields: Fields,
    cold: bool,
    last: Option<i32>,
    /// Whether Countries and CountryLinks were complete.
    complete: bool,
    lineage: Option<Arc<Lineage>>,
    /// Each raw link's dense holder, and each dense holder's tag index.
    holders: HashMap<u32, u32>,
    dense: Vec<u32>,
    /// The rows of complete States and Provinces, by slot; none while not complete.
    states: Option<Vec<Option<StateCell>>>,
    provinces: Option<Vec<Option<ProvinceCell>>>,
    statuses: [Option<walked::Status>; 2],
    /// Whether each table was freshly read and complete.
    fresh: [bool; 2],
    /// The sums hold every row under the current lineage.
    summed: bool,
    sums: Vec<Sums>,
    world: Sums,
    /// Rows whose holder is unknown, by rule and table ([`group`]).
    unknown: [u32; 6],
    /// Provinces without an area.
    missing: u32,
    dirty: Vec<bool>,
    dirty_list: Vec<u32>,
    /// The slots a table's changes wrote, kept between hours.
    written: Vec<u32>,
    /// What each dense holder's values were last sent as, and the world's.
    sent: Vec<Values>,
    sent_world: Option<Values>,
    sent_rates: Option<Rates>,
    /// Each dense holder's columns past the exact range or unknown.
    over: Vec<u16>,
    unknowns: Vec<u16>,
    over_count: usize,
    over_columns: [u32; columns::COUNT],
    unknown_count: [u32; columns::COUNT],
    /// Each counter row's canonical slot (none where no link names its country) and count,
    /// by table slot, the rows naming each canonical slot, and whether they are held.
    counter_rows: Vec<Option<(Option<u32>, i32)>>,
    counter_slots: HashMap<u32, Vec<u32>>,
    counted: bool,
    counter_changed: Vec<u32>,
    counters: Option<walked::Status>,
    /// Regional numbering of this segment, its sums and the series changed this hour.
    region_ids: HashMap<RegionKey, u32>,
    region_keys: Vec<(u32, RegionKey)>,
    region_memo: HashMap<(u32, Option<u32>, bool), u32>,
    region_sums: Vec<RegionSums>,
    region_dirty: Vec<bool>,
    region_list: Vec<u32>,
    region_fresh: bool,
    seen_states: HashSet<u32>,
    seen_provinces: HashSet<u32>,
    missing_states: Vec<u32>,
    missing_provinces: Vec<u32>,
}

impl Projector {
    pub fn new(
        format: FormatVersion,
        areas: Option<Arc<ProvinceAreas>>,
        regions: Option<Arc<Regions>>,
    ) -> io::Result<Self> {
        Ok(Self {
            format,
            areas,
            regions,
            fields: Fields::new()?,
            cold: true,
            last: None,
            complete: false,
            lineage: None,
            holders: HashMap::new(),
            dense: Vec::new(),
            states: None,
            provinces: None,
            statuses: [None; 2],
            fresh: [false; 2],
            summed: false,
            sums: Vec::new(),
            world: Sums::default(),
            unknown: [0; 6],
            missing: 0,
            dirty: Vec::new(),
            dirty_list: Vec::new(),
            written: Vec::new(),
            sent: Vec::new(),
            sent_world: None,
            sent_rates: None,
            over: Vec::new(),
            unknowns: Vec::new(),
            over_count: 0,
            over_columns: [0; columns::COUNT],
            unknown_count: [0; columns::COUNT],
            counter_rows: Vec::new(),
            counter_slots: HashMap::new(),
            counted: false,
            counter_changed: Vec::new(),
            counters: None,
            region_ids: HashMap::new(),
            region_keys: Vec::new(),
            region_memo: HashMap::new(),
            region_sums: Vec::new(),
            region_dirty: Vec::new(),
            region_list: Vec::new(),
            region_fresh: true,
            seen_states: HashSet::new(),
            seen_provinces: HashSet::new(),
            missing_states: Vec::new(),
            missing_provinces: Vec::new(),
        })
    }

    /// The lineage the roster resolves, with each raw link's dense holder.
    fn lineage(roster: &Roster) -> (Lineage, HashMap<u32, u32>, Vec<u32>) {
        let mut tags = Vec::new();
        let mut incomplete = false;
        for tag in roster.held() {
            let bytes = roster.tag(tag);
            let Ok(text) = std::str::from_utf8(bytes) else {
                incomplete = true;
                continue;
            };
            if text.is_empty() {
                continue;
            }
            let identity = match roster.tagged(tag) {
                Tagged::Found(country) => Some((country.slot, country.id)),
                _ => {
                    incomplete = true;
                    None
                }
            };
            tags.push(Held {
                tag: Arc::from(text),
                identity,
                entry: None,
                linked: false,
            });
        }
        tags.sort_unstable_by(|left, right| left.tag.as_bytes().cmp(right.tag.as_bytes()));
        let mut named: HashMap<u32, (usize, bool)> = HashMap::new();
        for (at, held) in tags.iter().enumerate() {
            if let Some((slot, _)) = held.identity {
                named
                    .entry(slot)
                    .and_modify(|seen| seen.1 = true)
                    .or_insert((at, false));
            }
        }
        let ambiguous = named.values().filter(|(_, twice)| *twice).count();
        incomplete |= ambiguous != 0;
        // Dense indexes follow the tags' order, one per unambiguous slot.
        let mut dense = Vec::new();
        let mut slots = HashMap::new();
        for (at, held) in tags.iter_mut().enumerate() {
            let Some((slot, _)) = held.identity else {
                continue;
            };
            if named.get(&slot).is_some_and(|&(_, twice)| !twice) {
                held.entry = Some(dense.len() as u32);
                slots.insert(slot, dense.len() as u32);
                dense.push(at as u32);
            }
        }
        let mut holders = HashMap::new();
        for link in roster.links() {
            if let Some(&index) = slots.get(&link.canonical_slot) {
                holders.insert(link.id, index);
                tags[dense[index as usize] as usize].linked = true;
            }
        }
        (
            Lineage {
                tags,
                incomplete,
                ambiguous,
            },
            holders,
            dense,
        )
    }

    fn holder(&self, raw: u32) -> Holder {
        if raw == 0 {
            return Holder::Unassigned;
        }
        self.holders
            .get(&raw)
            .copied()
            .map_or(Holder::Unknown, Holder::Country)
    }

    fn mark(&mut self, index: u32) {
        let at = index as usize;
        if !self.dirty[at] {
            self.dirty[at] = true;
            self.dirty_list.push(index);
        }
    }

    /// Adds `sign` times `values` at `column` to the world and `holder`.
    fn add(
        &mut self,
        holder: Holder,
        column: usize,
        values: &[i128],
        area: Option<bool>,
        sign: i128,
    ) {
        let rule = column / 4;
        for (offset, value) in values.iter().enumerate() {
            self.world.totals[column + offset] += sign * value;
        }
        let lacks = area == Some(false);
        if lacks {
            self.world.missing[rule] = self.world.missing[rule].wrapping_add_signed(sign as i32);
        }
        match holder {
            Holder::Unassigned => {}
            Holder::Unknown => {
                let unknown = &mut self.unknown[group(column)];
                *unknown = unknown.wrapping_add_signed(sign as i32);
            }
            Holder::Country(index) => {
                let sums = &mut self.sums[index as usize];
                for (offset, value) in values.iter().enumerate() {
                    sums.totals[column + offset] += sign * value;
                }
                if lacks {
                    sums.missing[rule] = sums.missing[rule].wrapping_add_signed(sign as i32);
                }
                self.mark(index);
            }
        }
    }

    fn state_weight(&mut self, cell: StateCell, sign: i128) {
        let controller = self.holder(cell.controller);
        let owner = self.holder(cell.owner);
        self.add(controller, columns::CONTROLLED_STATES, &[1], None, sign);
        self.add(owner, columns::OWNED_STATES, &[1], None, sign);
        match occupied(controller, owner) {
            Holder::Unassigned => {}
            holder => self.add(holder, columns::OCCUPIED_STATES, &[1], None, sign),
        }
        self.state_regions(cell, sign);
    }

    fn province_weight(&mut self, cell: ProvinceCell, sign: i128) {
        let controller = self.holder(cell.controller);
        let owner = self.holder(cell.owner);
        let area = self.areas.is_some().then_some(cell.area.is_some());
        let values = [
            1,
            i128::from(cell.victory_points),
            i128::from(cell.area.unwrap_or(0)),
        ];
        if area == Some(false) {
            self.missing = self.missing.wrapping_add_signed(sign as i32);
        }
        self.add(
            controller,
            columns::CONTROLLED_PROVINCES,
            &values,
            area,
            sign,
        );
        self.add(owner, columns::OWNED_PROVINCES, &values, area, sign);
        match occupied(controller, owner) {
            Holder::Unassigned => {}
            holder => self.add(holder, columns::OCCUPIED_PROVINCES, &values, area, sign),
        }
        self.province_regions(cell, area, sign);
    }

    /// The regional series of raw holder `raw` in `region`, numbered in this segment.
    fn series(&mut self, raw: u32, region: Option<u32>, state: bool) -> u32 {
        let memo = (raw, region, state);
        if let Some(&local) = self.region_memo.get(&memo) {
            return local;
        }
        let tag = match self.holder(raw) {
            Holder::Country(index) => self
                .lineage
                .as_ref()
                .map(|lineage| Arc::clone(&lineage.tags[self.dense[index as usize] as usize].tag)),
            _ => None,
        };
        let unknown = raw != 0 && tag.is_none();
        let region_reason = state
            .then(|| {
                self.regions
                    .as_ref()
                    .and_then(|regions| regions.state_reason())
                    .unwrap_or(STATE_REGION)
            })
            .unwrap_or(PROVINCE_REGION);
        let reasons: Vec<&str> = [
            unknown.then_some(UNKNOWN_HOLDER),
            region.is_none().then_some(region_reason),
        ]
        .into_iter()
        .flatten()
        .collect();
        let key = RegionKey {
            tag,
            region,
            missing: (!reasons.is_empty()).then(|| Arc::from(reasons.join("; "))),
            unknown,
        };
        let local = match self.region_ids.get(&key) {
            Some(&local) => local,
            None => {
                let local = self.region_sums.len() as u32;
                self.region_ids.insert(key.clone(), local);
                self.region_keys.push((local, key));
                self.region_sums.push(RegionSums::default());
                self.region_dirty.push(false);
                local
            }
        };
        self.region_memo.insert(memo, local);
        local
    }

    fn region_mark(&mut self, local: u32) {
        let at = local as usize;
        if !self.region_dirty[at] {
            self.region_dirty[at] = true;
            self.region_list.push(local);
        }
    }

    fn state_regions(&mut self, cell: StateCell, sign: i128) {
        let Some(regions) = self.regions.clone() else {
            return;
        };
        let parts = regions.state_parts(cell.id);
        if parts.is_none() && sign > 0 && self.seen_states.insert(cell.id) {
            self.missing_states.push(cell.id);
        }
        let fallback = [(None, 1, 1)];
        let parts: Vec<(Option<u32>, u64, u32)> = match parts {
            Some(parts) => parts.collect(),
            None => fallback.to_vec(),
        };
        for (region, numerator, denominator) in parts {
            for (raw, rule) in [(cell.controller, 0), (cell.owner, 1)] {
                let local = self.series(raw, region, true);
                let sums = &mut self.region_sums[local as usize];
                let terms = &mut sums.states[rule];
                let total = terms.entry(denominator).or_default();
                *total = total.wrapping_add_signed((sign as i64) * numerator as i64);
                if *total == 0 {
                    terms.remove(&denominator);
                }
                sums.state_rows = sums.state_rows.wrapping_add_signed(sign as i32);
                self.region_mark(local);
            }
        }
    }

    fn province_regions(&mut self, cell: ProvinceCell, area: Option<bool>, sign: i128) {
        let Some(regions) = self.regions.clone() else {
            return;
        };
        let region = regions.province_region(cell.id);
        if region.is_none() && sign > 0 && self.seen_provinces.insert(cell.id) {
            self.missing_provinces.push(cell.id);
        }
        let values = [
            1,
            i128::from(cell.victory_points),
            i128::from(cell.area.unwrap_or(0)),
        ];
        for (raw, rule) in [(cell.controller, 0), (cell.owner, 1)] {
            let local = self.series(raw, region, false);
            let sums = &mut self.region_sums[local as usize];
            for (offset, value) in values.iter().enumerate() {
                sums.provinces[rule * 3 + offset] += sign * value;
            }
            if area == Some(false) {
                sums.missing[rule] = sums.missing[rule].wrapping_add_signed(sign as i32);
            }
            sums.province_rows = sums.province_rows.wrapping_add_signed(sign as i32);
            self.region_mark(local);
        }
    }

    fn state_cell(&self, state: &State, slot: u32) -> io::Result<StateCell> {
        let held = walked::table(state, TableId::States)?;
        let id = held
            .key(slot)
            .ok_or_else(|| invalid("A States slot holds no row"))? as u32;
        let fields = [self.fields.state_controller, self.fields.state_owner];
        if fields.iter().any(|&field| !held.recorded(field, slot)) {
            let row: StateRow = walked::row(state, TableId::States, slot, self.format.version())?;
            return Ok(StateCell {
                id,
                controller: row.controller,
                owner: row.owner,
            });
        }
        let cell = |field| -> io::Result<u32> {
            walked::plain::<u32>(held, field)?
                .get(slot as usize)
                .copied()
                .ok_or_else(|| invalid("A States slot holds no cell"))
        };
        Ok(StateCell {
            id,
            controller: cell(self.fields.state_controller)?,
            owner: cell(self.fields.state_owner)?,
        })
    }

    fn province_cell(&self, state: &State, slot: u32) -> io::Result<ProvinceCell> {
        let held = walked::table(state, TableId::Provinces)?;
        let id = held
            .key(slot)
            .ok_or_else(|| invalid("A Provinces slot holds no row"))? as u32;
        let area = |id: u32| {
            self.areas
                .as_ref()
                .and_then(|areas| areas.land.get(id as usize).copied())
        };
        let fields = [
            self.fields.province_controller,
            self.fields.province_owner,
            self.fields.province_points,
        ];
        if fields.iter().any(|&field| !held.recorded(field, slot)) {
            let row: ProvinceRow =
                walked::row(state, TableId::Provinces, slot, self.format.version())?;
            return Ok(ProvinceCell {
                id,
                controller: row.controller,
                owner: row.owner,
                victory_points: row.victory_points,
                area: area(id),
            });
        }
        let cell = |field| -> io::Result<u32> {
            walked::plain::<u32>(held, field)?
                .get(slot as usize)
                .copied()
                .ok_or_else(|| invalid("A Provinces slot holds no cell"))
        };
        Ok(ProvinceCell {
            id,
            controller: cell(self.fields.province_controller)?,
            owner: cell(self.fields.province_owner)?,
            victory_points: walked::plain::<i32>(held, self.fields.province_points)?
                .get(slot as usize)
                .copied()
                .ok_or_else(|| invalid("A Provinces slot holds no cell"))?,
            area: area(id),
        })
    }

    /// Every live row of `table`, read whole.
    fn whole<T>(
        &self,
        state: &State,
        table: TableId,
        read: impl Fn(&Self, &State, u32) -> io::Result<T>,
    ) -> io::Result<Vec<Option<T>>> {
        let held = walked::table(state, table)?;
        let mut rows = Vec::new();
        for slot in held.live().ones() {
            if rows.len() <= slot as usize {
                rows.resize_with(slot as usize + 1, || None);
            }
            rows[slot as usize] = Some(read(self, state, slot)?);
        }
        Ok(rows)
    }

    /// Into `written`, the slots the hour inserted or wrote any of `mask` at, ascending.
    fn edits(state: &State, table: TableId, mask: u64, written: &mut Vec<u32>) -> io::Result<()> {
        let changes = walked::changes(state, table)?;
        written.clear();
        written.extend(
            changes
                .changed
                .iter()
                .zip(&changes.masks)
                .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
                .filter(|&(_, &bits)| bits & mask != 0)
                .map(|(&slot, _)| slot)
                .chain(changes.inserted.iter().copied()),
        );
        written.sort_unstable();
        written.dedup();
        Ok(())
    }

    /// States and Provinces from the hour: read whole where they open or became complete,
    /// else moved by their changed rows, their weights with them while the sums hold.
    fn land(&mut self, state: &State, opens: bool) -> io::Result<()> {
        for (index, table) in [TableId::States, TableId::Provinces]
            .into_iter()
            .enumerate()
        {
            let status = walked::status(state, table);
            let complete = status == walked::Status::Complete;
            let shaped = opens || self.statuses[index] != Some(status);
            self.statuses[index] = Some(status);
            let held = if index == 0 {
                self.states.is_some()
            } else {
                self.provinces.is_some()
            };
            if !complete {
                if held {
                    self.summed = false;
                }
                if index == 0 {
                    self.states = None;
                } else {
                    self.provinces = None;
                }
                continue;
            }
            if shaped || !held {
                self.summed = false;
                if index == 0 {
                    self.states = Some(self.whole(state, table, Self::state_cell)?);
                } else {
                    self.provinces = Some(self.whole(state, table, Self::province_cell)?);
                }
                continue;
            }
            let mask = if index == 0 {
                self.fields.states()
            } else {
                self.fields.provinces()
            };
            let mut written = std::mem::take(&mut self.written);
            Self::edits(state, table, mask, &mut written)?;
            for &slot in &walked::changes(state, table)?.deleted {
                if index == 0 {
                    let old = self
                        .states
                        .as_mut()
                        .and_then(|rows| rows.get_mut(slot as usize))
                        .and_then(Option::take);
                    if let Some(old) = old.filter(|_| self.summed) {
                        self.state_weight(old, -1);
                    }
                } else {
                    let old = self
                        .provinces
                        .as_mut()
                        .and_then(|rows| rows.get_mut(slot as usize))
                        .and_then(Option::take);
                    if let Some(old) = old.filter(|_| self.summed) {
                        self.province_weight(old, -1);
                    }
                }
            }
            for &slot in &written {
                let at = slot as usize;
                if index == 0 {
                    let cell = self.state_cell(state, slot)?;
                    let rows = self.states.as_mut().expect("held States");
                    if rows.len() <= at {
                        rows.resize_with(at + 1, || None);
                    }
                    let old = rows[at].replace(cell);
                    if self.summed {
                        if let Some(old) = old {
                            self.state_weight(old, -1);
                        }
                        self.state_weight(cell, 1);
                    }
                } else {
                    let cell = self.province_cell(state, slot)?;
                    let rows = self.provinces.as_mut().expect("held Provinces");
                    if rows.len() <= at {
                        rows.resize_with(at + 1, || None);
                    }
                    let old = rows[at].replace(cell);
                    if self.summed {
                        if let Some(old) = old {
                            self.province_weight(old, -1);
                        }
                        self.province_weight(cell, 1);
                    }
                }
            }
            self.written = written;
        }
        Ok(())
    }

    /// Every sum from zero under the current lineage: every holder and series changes.
    fn resum(&mut self) {
        let count = self.dense.len();
        self.sums = vec![Sums::default(); count];
        self.world = Sums::default();
        self.unknown = [0; 6];
        self.missing = 0;
        self.dirty = vec![false; count];
        self.dirty_list.clear();
        for sums in &mut self.region_sums {
            *sums = RegionSums::default();
        }
        let states = self.states.take();
        for cell in states.iter().flatten().flatten() {
            self.state_weight(*cell, 1);
        }
        self.states = states;
        let provinces = self.provinces.take();
        for cell in provinces.iter().flatten().flatten() {
            self.province_weight(*cell, 1);
        }
        self.provinces = provinces;
        self.summed = true;
    }

    /// Whether `column` is read: its table fresh and complete, and the map's areas for an area.
    fn available(&self, column: usize) -> bool {
        self.fresh[usize::from(!columns::is_state(column))]
            && (!columns::is_area(column) || self.areas.is_some())
    }

    /// A dense holder's values, with the columns past the exact range.
    fn values(&self, index: usize) -> (Values, u16) {
        let sums = &self.sums[index];
        let mut over = 0u16;
        let values = std::array::from_fn(|column| {
            if !self.available(column) || self.unknown[group(column)] != 0 {
                return None;
            }
            if columns::is_area(column) && sums.missing[column / 4] != 0 {
                return None;
            }
            let total = sums.totals[column];
            if total.unsigned_abs() > u128::from(MAX_EXACT_JS_INTEGER) {
                over |= 1 << column;
                return None;
            }
            Some(total as i64)
        });
        (values, over)
    }

    /// The world's values, with the columns past the exact range. A holder past it leaves the
    /// world's column unknown.
    fn world_values(&self) -> (Values, usize) {
        let mut overflowed = 0;
        let values = std::array::from_fn(|column| {
            if !self.available(column) || self.unknown[group(column)] != 0 {
                return None;
            }
            if columns::is_area(column) && self.world.missing[column / 4] != 0 {
                return None;
            }
            let total = self.world.totals[column];
            if total.unsigned_abs() > u128::from(MAX_EXACT_JS_INTEGER) {
                overflowed += 1;
                return None;
            }
            (self.over_columns[column] == 0).then_some(total as i64)
        });
        (values, overflowed)
    }

    /// Each dense holder's values the hour changed: every holder's when `all`.
    fn holder_values(&mut self, all: bool) -> Vec<(u32, Values)> {
        let indexes: Vec<u32> = if all {
            self.dirty_list.clear();
            self.dirty.iter_mut().for_each(|dirty| *dirty = false);
            let count = self.dense.len();
            self.sent = vec![[None; columns::COUNT]; count];
            self.over = vec![0; count];
            self.unknowns = vec![0; count];
            self.over_count = 0;
            self.over_columns = [0; columns::COUNT];
            self.unknown_count = [0; columns::COUNT];
            (0..count as u32).collect()
        } else {
            let mut list = std::mem::take(&mut self.dirty_list);
            list.sort_unstable();
            for &index in &list {
                self.dirty[index as usize] = false;
            }
            list
        };
        let mut changed = Vec::new();
        for &index in &indexes {
            let at = index as usize;
            let (values, over) = self.values(at);
            let unknown = (0..columns::COUNT)
                .filter(|&column| values[column].is_none())
                .fold(0u16, |bits, column| bits | 1 << column);
            for column in 0..columns::COUNT {
                let bit = 1 << column;
                if self.over[at] & bit != 0 {
                    self.over_columns[column] -= 1;
                    self.over_count -= 1;
                }
                if over & bit != 0 {
                    self.over_columns[column] += 1;
                    self.over_count += 1;
                }
                if self.unknowns[at] & bit != 0 {
                    self.unknown_count[column] -= 1;
                }
                if unknown & bit != 0 {
                    self.unknown_count[column] += 1;
                }
            }
            self.over[at] = over;
            self.unknowns[at] = unknown;
            if all || self.sent[at] != values {
                self.sent[at] = values;
                changed.push((index, values));
            }
        }
        if !all {
            let mut indexes = indexes;
            indexes.clear();
            self.dirty_list = indexes;
        }
        changed
    }

    /// The hour's rates, as the row census counts every hour it observes.
    fn rates(&self, world_overflowed: usize) -> Rates {
        let lineage = self
            .lineage
            .as_ref()
            .is_some_and(|lineage| lineage.incomplete);
        let measured = (0..columns::COUNT).any(|column| {
            self.unknown_count[column] != 0 && (!columns::is_area(column) || self.areas.is_some())
        });
        let missing = self.fresh[1] && self.areas.is_some() && self.missing != 0;
        Rates {
            overflowed: (self.over_count + world_overflowed) as u64,
            incomplete: lineage || measured || missing,
        }
    }

    /// The country and count of the counter row at `slot`.
    fn counter_cell(&self, state: &State, slot: u32) -> io::Result<(u32, i32)> {
        let table = TableId::CountryCounterInputs;
        let held = walked::table(state, table)?;
        // The country is the counter row's key, not one of its fields.
        let country = held
            .key(slot)
            .and_then(|key| u32::try_from(key).ok())
            .ok_or_else(|| invalid("A counter slot holds no country"))?;
        if held.recorded(self.fields.counter_owned, slot) {
            return Ok((
                country,
                *walked::plain::<i32>(held, self.fields.counter_owned)?
                    .get(slot as usize)
                    .ok_or_else(|| invalid("A counter slot holds no cell"))?,
            ));
        }
        let row: CountryCounterInputRow = walked::row(state, table, slot, self.format.version())?;
        Ok((row.country, row.owned_state_count))
    }

    /// Forgets the counter row at `slot`.
    fn counter_remove(&mut self, slot: u32) {
        let Some(Some((canonical, _))) = self.counter_rows.get_mut(slot as usize).map(Option::take)
        else {
            return;
        };
        let Some(canonical) = canonical else {
            return;
        };
        if let Some(rows) = self.counter_slots.get_mut(&canonical) {
            rows.retain(|&held| held != slot);
            if rows.is_empty() {
                self.counter_slots.remove(&canonical);
            }
        }
        self.counter_changed.push(canonical);
    }

    /// Keeps the counter row at `slot`, its country resolved through `roster`'s links.
    fn counter_insert(&mut self, slot: u32, (country, owned): (u32, i32), roster: &Roster) {
        let canonical = roster.slot_of(country);
        let at = slot as usize;
        if self.counter_rows.len() <= at {
            self.counter_rows.resize(at + 1, None);
        }
        self.counter_rows[at] = Some((canonical, owned));
        if let Some(canonical) = canonical {
            self.counter_slots.entry(canonical).or_default().push(slot);
            self.counter_changed.push(canonical);
        }
    }

    /// The count of canonical slot `canonical`, as the identity history joins the rows: none
    /// where two rows name it; and whether a row names it.
    fn counter(&self, canonical: u32) -> (Option<i64>, bool) {
        match self.counter_slots.get(&canonical).map(Vec::as_slice) {
            Some(&[slot]) => (
                self.counter_rows[slot as usize].map(|(_, owned)| i64::from(owned)),
                true,
            ),
            Some(rows) if !rows.is_empty() => (None, true),
            _ => (None, false),
        }
    }

    /// The owned-state counters from the hour: read whole where the table or the links
    /// changed shape, else moved by the rows whose country or count changed.
    fn counters(
        &mut self,
        state: &State,
        roster: &Roster,
        anew: bool,
        complete: bool,
    ) -> io::Result<Option<Counters>> {
        let table = TableId::CountryCounterInputs;
        let status = walked::status(state, table);
        let shaped = anew || self.counters != Some(status);
        self.counters = Some(status);
        if status != walked::Status::Complete || !complete {
            let was = std::mem::take(&mut self.counted);
            self.counter_rows.clear();
            self.counter_slots.clear();
            return Ok((shaped || was).then(Counters::default));
        }
        self.counter_changed.clear();
        if shaped || !self.counted || roster.rebuilt() || !roster.changed_links().is_empty() {
            self.counter_rows.clear();
            self.counter_slots.clear();
            let live: Vec<u32> = walked::table(state, table)?.live().ones().collect();
            for slot in live {
                let cell = self.counter_cell(state, slot)?;
                self.counter_insert(slot, cell, roster);
            }
            self.counted = true;
            self.counter_changed.clear();
            let mut slots: Vec<(u32, Option<i64>, bool)> = self
                .counter_slots
                .keys()
                .map(|&canonical| {
                    let (count, held) = self.counter(canonical);
                    (canonical, count, held)
                })
                .collect();
            slots.sort_unstable_by_key(|&(canonical, _, _)| canonical);
            return Ok(Some(Counters {
                complete: true,
                whole: true,
                slots,
            }));
        }
        if !touched(state, table, self.fields.counters())? {
            return Ok(None);
        }
        let mut written = std::mem::take(&mut self.written);
        Self::edits(state, table, self.fields.counters(), &mut written)?;
        for &slot in &walked::changes(state, table)?.deleted {
            self.counter_remove(slot);
        }
        for &slot in &written {
            let cell = self.counter_cell(state, slot)?;
            self.counter_remove(slot);
            self.counter_insert(slot, cell, roster);
        }
        self.written = written;
        let mut changed = std::mem::take(&mut self.counter_changed);
        changed.sort_unstable();
        changed.dedup();
        let slots = changed
            .iter()
            .map(|&canonical| {
                let (count, held) = self.counter(canonical);
                (canonical, count, held)
            })
            .collect();
        changed.clear();
        self.counter_changed = changed;
        Ok(Some(Counters {
            complete: true,
            whole: false,
            slots,
        }))
    }

    /// Regional sums of every series that changed, or of every series with rows when `whole`.
    fn regional(&mut self, whole: bool) -> Option<Regional> {
        self.regions.as_ref()?;
        let available = self.land_columns();
        let mut dirty = std::mem::take(&mut self.region_list);
        for &local in &dirty {
            self.region_dirty[local as usize] = false;
        }
        dirty.sort_unstable();
        let every: Vec<u32>;
        let list = if whole {
            every = (0..self.region_sums.len() as u32).collect();
            &every
        } else {
            &dirty
        };
        let mut series = Vec::new();
        for &local in list {
            let sums = &self.region_sums[local as usize];
            let state_rows = self.fresh[0] && sums.state_rows != 0;
            let province_rows = self.fresh[1] && sums.province_rows != 0;
            if !state_rows && !province_rows {
                if !whole {
                    series.push((local, None));
                }
                continue;
            }
            let mut values = RegionValues::zero(available);
            if province_rows {
                for (offset, column) in [1, 2, 3, 5, 6, 7].into_iter().enumerate() {
                    let rule = offset / 3;
                    values.values[offset] = (available[column]
                        && !(columns::is_area(column) && sums.missing[rule] != 0))
                        .then(|| sums.provinces[offset] as i64);
                }
            }
            if state_rows {
                for rule in 0..2 {
                    if available[rule * 4] {
                        values.states[rule] = Some(sums.states[rule].clone());
                    }
                }
            }
            series.push((local, Some(Arc::new(values))));
        }
        dirty.clear();
        self.region_list = dirty;
        let keys = std::mem::take(&mut self.region_keys);
        let missing_states = std::mem::take(&mut self.missing_states);
        let missing_provinces = std::mem::take(&mut self.missing_provinces);
        if !whole
            && series.is_empty()
            && keys.is_empty()
            && missing_states.is_empty()
            && missing_provinces.is_empty()
        {
            return None;
        }
        let fresh = std::mem::take(&mut self.region_fresh);
        Some(Regional {
            whole,
            fresh,
            keys,
            series,
            missing_states,
            missing_provinces,
        })
    }

    /// The regional columns' availability: controlled then owned states, provinces, victory
    /// points and area.
    fn land_columns(&self) -> [bool; 8] {
        std::array::from_fn(|column| self.available(column))
    }

    fn heap(&self) -> usize {
        self.lineage.as_ref().map_or(0, |lineage| lineage.heap())
            + self.holders.capacity() * 12
            + self.dense.capacity() * 4
            + self
                .states
                .as_ref()
                .map_or(0, |rows| rows.capacity() * size_of::<Option<StateCell>>())
            + self.provinces.as_ref().map_or(0, |rows| {
                rows.capacity() * size_of::<Option<ProvinceCell>>()
            })
            + self.sums.capacity() * size_of::<Sums>()
            + self.sent.capacity() * size_of::<Values>()
            + self.dirty.capacity()
            + self.dirty_list.capacity() * 4
            + (self.over.capacity() + self.unknowns.capacity()) * 2
            + self.counter_rows.capacity() * size_of::<Option<(Option<u32>, i32)>>()
            + self.counter_slots.capacity() * 40
            + self.region_ids.capacity() * (size_of::<RegionKey>() + 4)
            + self.region_memo.capacity() * 16
            + self.region_sums.capacity() * size_of::<RegionSums>()
            + self.region_sums.iter().map(RegionSums::heap).sum::<usize>()
            + self.region_dirty.capacity()
            + (self.seen_states.capacity() + self.seen_provinces.capacity()) * 8
    }
}

/// Whether the hour inserted, deleted or wrote any of `mask` in `table`, or read it in part.
fn touched(state: &State, table: TableId, mask: u64) -> io::Result<bool> {
    let changes = walked::changes(state, table)?;
    Ok(changes.truncated
        || !changes.inserted.is_empty()
        || !changes.deleted.is_empty()
        || changes
            .masks
            .iter()
            .chain(&changes.fill_masks)
            .any(|bits| bits & mask != 0))
}

// Every segment starts cold: its first hour reads every country, state and province again;
// what came before is the emit's.
impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, _: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.cold = true;
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("territory.columnar", Some("territory"));
        let state = hour.state;
        let at = hour.hour;
        let roster = hour.world.countries()?;
        let opens = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        if self.cold {
            // The segment's regional numbering starts anew.
            self.region_ids.clear();
            self.region_keys.clear();
            self.region_memo.clear();
            self.region_sums.clear();
            self.region_dirty.clear();
            self.region_list.clear();
            self.region_fresh = true;
            self.seen_states.clear();
            self.seen_provinces.clear();
            self.missing_states.clear();
            self.missing_provinces.clear();
            self.sent_world = None;
            self.sent_rates = None;
        }
        self.cold = false;
        self.last = Some(at);
        let mut facts = Facts {
            whole: opens,
            ..Facts::default()
        };
        let complete = roster.complete();
        let became = opens || complete != self.complete;
        self.complete = complete;
        let mut regroup = false;
        if !complete {
            if became || self.lineage.is_some() {
                facts.lineage = Some(None);
            }
            if self.lineage.take().is_some() {
                self.summed = false;
            }
            self.holders.clear();
            self.dense.clear();
        } else if became || roster.changed() {
            let (lineage, holders, dense) = Self::lineage(roster);
            // The row census reads the lineage anew at each change of a country or link.
            let reread = !roster.rebuilt()
                && (!roster.changed_ids().is_empty() || !roster.changed_links().is_empty());
            if reread && lineage.ambiguous != 0 {
                facts.recount = Some(lineage.ambiguous);
            }
            if opens || self.lineage.as_deref() != Some(&lineage) || self.holders != holders {
                let lineage = Arc::new(lineage);
                facts.lineage = Some(Some(Arc::clone(&lineage)));
                self.lineage = Some(lineage);
                self.holders = holders;
                self.dense = dense;
                self.region_memo.clear();
                self.summed = false;
                regroup = true;
            }
        }
        let unknown = self.unknown.map(|count| count != 0);
        self.land(state, opens)?;
        let flags = metrics(hour.manifest);
        let slot_read = |table| dashboard_text::metadata::slot_read(self.format, &flags, table, at);
        let fresh = [
            self.states.is_some() && slot_read(TableId::States),
            self.provinces.is_some() && slot_read(TableId::Provinces),
        ];
        let refreshed = opens || fresh != self.fresh;
        if refreshed {
            facts.land = Some(fresh);
        }
        self.fresh = fresh;
        facts.owned = self.counters(state, roster, opens || became, complete)?;
        if complete {
            let mut whole = opens || regroup || !self.summed || refreshed;
            if !self.summed {
                self.resum();
            }
            // An unknown holder hides its column of every holder.
            whole |= self.unknown.map(|count| count != 0) != unknown;
            facts.values = self.holder_values(whole);
            let (world, overflowed) = self.world_values();
            let world = if self
                .lineage
                .as_ref()
                .is_some_and(|lineage| lineage.incomplete)
            {
                [None; columns::COUNT]
            } else {
                world
            };
            if opens || self.sent_world != Some(world) {
                self.sent_world = Some(world);
                facts.world = Some(world);
            }
            let rates = self.rates(overflowed);
            if opens || became || self.sent_rates != Some(rates) {
                self.sent_rates = Some(rates);
                facts.rates = Some(rates);
            }
            facts.regional = self.regional(whole);
        } else {
            let rates = Rates {
                overflowed: 0,
                incomplete: true,
            };
            if opens || self.sent_rates != Some(rates) {
                self.sent_rates = Some(rates);
                facts.rates = Some(rates);
            }
            self.sent_world = None;
        }
        let quiet = !facts.whole
            && facts.lineage.is_none()
            && facts.values.is_empty()
            && facts.world.is_none()
            && facts.rates.is_none()
            && facts.land.is_none()
            && facts.owned.is_none()
            && facts.recount.is_none()
            && facts.regional.is_none();
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            facts: (!quiet).then(|| Box::new(facts)),
        })
    }

    fn carried(&self) -> Option<usize> {
        Some(self.heap())
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a saved territory projector",
            ));
        }
        *self = Self::new(self.format, self.areas.clone(), self.regions.clone())?;
        Ok(())
    }
}
