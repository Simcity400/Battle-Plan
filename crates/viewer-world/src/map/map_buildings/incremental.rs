//! Patch changed building slots and share every untouched chunk.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use viewer_foundation::map::{memo::project, perspective::MAP_OBJECT_VISIBILITY};

// Chunk size bounds copying, not the number of building slots.
const CHUNK_SHIFT: u32 = 6;
pub(super) const CHUNK_SIZE: usize = 1usize << CHUNK_SHIFT;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Chunked<T> {
    shift: u32,
    chunks: Arc<[Arc<[T]>]>,
}

impl<T> Chunked<T> {
    pub(super) fn get(&self, id: usize) -> Option<&T> {
        self.chunks
            .get(id >> self.shift)?
            .get(id & ((1usize << self.shift) - 1))
    }

    pub(super) fn len(&self) -> usize {
        self.chunks.iter().map(|chunk| chunk.len()).sum()
    }

    #[cfg(test)]
    pub(super) fn shared_chunks(&self, other: &Self) -> usize {
        self.chunks
            .iter()
            .zip(other.chunks.iter())
            .filter(|(a, b)| Arc::ptr_eq(a, b))
            .count()
    }

    #[cfg(test)]
    pub(super) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.chunks, &other.chunks)
    }
}

impl<T: Clone> Chunked<T> {
    pub(super) fn patched(
        &self,
        changed: &[(u32, T)],
        tail_from: usize,
        tail: &[T],
    ) -> Option<Self> {
        let len = tail_from + tail.len();
        if self.len() != len {
            return None;
        }
        let mut next = 0;
        let mut chunks = Vec::with_capacity(self.chunks.len());
        for (index, base) in self.chunks.iter().enumerate() {
            let start = index << self.shift;
            let range = start..start + base.len();
            let touched = range.end > tail_from
                || changed
                    .get(next)
                    .is_some_and(|(id, _)| (*id as usize) < range.end);
            if !touched {
                chunks.push(Arc::clone(base));
                continue;
            }
            let value = |id: usize| {
                if id >= tail_from {
                    tail[id - tail_from].clone()
                } else if changed.get(next).is_some_and(|(at, _)| *at as usize == id) {
                    next += 1;
                    changed[next - 1].1.clone()
                } else {
                    base[id - start].clone()
                }
            };
            chunks.push(range.map(value).collect::<Arc<[T]>>());
        }
        Some(Self {
            shift: self.shift,
            chunks: chunks.into(),
        })
    }
}

impl<T> From<Vec<T>> for Chunked<T> {
    fn from(values: Vec<T>) -> Self {
        let size = CHUNK_SIZE;
        let mut chunks = Vec::with_capacity(values.len().div_ceil(size));
        let mut values = values.into_iter();
        while !values.as_slice().is_empty() {
            chunks.push(values.by_ref().take(size).collect::<Arc<[T]>>());
        }
        Self {
            shift: CHUNK_SHIFT,
            chunks: chunks.into(),
        }
    }
}

impl<T> std::ops::Index<usize> for Chunked<T> {
    type Output = T;

    fn index(&self, id: usize) -> &T {
        self.get(id).expect("slot id within the hour's slots")
    }
}

impl<T> FromIterator<T> for Chunked<T> {
    fn from_iter<I: IntoIterator<Item = T>>(values: I) -> Self {
        values.into_iter().collect::<Vec<_>>().into()
    }
}

#[cfg(test)]
impl<T: Clone> Chunked<T> {
    pub(super) fn iter(&self) -> impl Iterator<Item = &T> {
        self.chunks.iter().flat_map(|chunk| chunk.iter())
    }

    pub(super) fn get_mut(&mut self, id: usize) -> &mut T {
        let mask = (1usize << self.shift) - 1;
        let chunk = &mut Arc::make_mut(&mut self.chunks)[id >> self.shift];
        &mut Arc::make_mut(chunk)[id & mask]
    }

    pub(super) fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        Arc::make_mut(&mut self.chunks)
            .iter_mut()
            .flat_map(|chunk| Arc::make_mut(chunk).iter_mut())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Tally {
    flags: u16,
    unresolved: Option<usize>,
}

impl Tally {
    pub(super) const FOGGED: u16 = 1;
    pub(super) const HUB_LEFT: u16 = 2;
    pub(super) const HUB_UNKNOWN: u16 = 4;
    pub(super) const UNDETECTED: u16 = 8;
    pub(super) const DETECTION_UNKNOWN: u16 = 16;
    pub(super) const PLACED: u16 = 32;
    pub(super) const READ_FOG: u16 = 64;
    pub(super) const READ_HUB: u16 = 128;
    pub(super) const READ_DETECTION: u16 = 256;

    pub(super) fn mark(&mut self, flag: u16) {
        self.flags |= flag;
    }

    pub(super) fn unresolved(&mut self, index: usize) {
        self.unresolved = Some(index);
    }

    fn has(self, flag: u16) -> usize {
        usize::from(self.flags & flag != 0)
    }

    fn reads(self, flag: u16) -> bool {
        self.flags & flag != 0
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct SlotSummary {
    fogged: usize,
    hubs_left: usize,
    unknown_hub: usize,
    undetected: usize,
    unknown_detection: usize,
    placed: usize,
    unresolved: BTreeMap<String, usize>,
}

impl SlotSummary {
    pub(super) fn add(&mut self, tally: Tally, types: &[BuildingType]) {
        self.fogged += tally.has(Tally::FOGGED);
        self.hubs_left += tally.has(Tally::HUB_LEFT);
        self.unknown_hub += tally.has(Tally::HUB_UNKNOWN);
        self.undetected += tally.has(Tally::UNDETECTED);
        self.unknown_detection += tally.has(Tally::DETECTION_UNKNOWN);
        self.placed += tally.has(Tally::PLACED);
        if let Some(index) = tally.unresolved {
            *self
                .unresolved
                .entry(types[index].name.clone())
                .or_default() += 1;
        }
    }

    fn remove(&mut self, tally: Tally, types: &[BuildingType]) {
        self.fogged -= tally.has(Tally::FOGGED);
        self.hubs_left -= tally.has(Tally::HUB_LEFT);
        self.unknown_hub -= tally.has(Tally::HUB_UNKNOWN);
        self.undetected -= tally.has(Tally::UNDETECTED);
        self.unknown_detection -= tally.has(Tally::DETECTION_UNKNOWN);
        self.placed -= tally.has(Tally::PLACED);
        if let Some(index) = tally.unresolved {
            let name = &types[index].name;
            if let Some(count) = self.unresolved.get_mut(name) {
                *count -= 1;
                if *count == 0 {
                    self.unresolved.remove(name);
                }
            }
        }
    }

    pub(super) fn apply(&self, summary: &mut MapBuildingSummary) {
        summary.fogged += self.fogged;
        summary.hubs_left_to_another_layer += self.hubs_left;
        summary.unknown_hub_membership += self.unknown_hub;
        summary.undetected += self.undetected;
        summary.unknown_detection += self.unknown_detection;
        summary.placed += self.placed;
        for (name, count) in &self.unresolved {
            *summary.unresolved_by_type.entry(name.clone()).or_default() += count;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildPath {
    Full(&'static str),
    Patched { slots: usize },
}

#[derive(Debug, PartialEq)]
pub(super) struct GateInputs {
    pub(super) ports: BTreeSet<u32>,
    pub(super) provinces: BTreeMap<u32, u32>,
    pub(super) states: BTreeMap<u32, u32>,
}

#[cfg(test)]
impl GateInputs {
    pub(super) fn reread(
        frame: &DecodedFrame,
        links: &BTreeMap<u32, u32>,
        ours: u32,
    ) -> io::Result<Self> {
        let detection = Detection::read(frame, links, ours)?;
        Ok(Self {
            ports: rows::<tables::NavalBases>(frame)?
                .rows()
                .iter()
                .map(|row| row.province)
                .collect(),
            provinces: detection.provinces,
            states: detection.states,
        })
    }
}

const BASE_TABLES: [TableId; 5] = [
    TableId::StateBuildings,
    TableId::ProvinceBuildings,
    TableId::CountryLinks,
    TableId::NavalBases,
    TableId::RaidTargetStatuses,
];

struct VisionOperands {
    ours: u32,
    fog: bool,
    visibility_gap: bool,
    provinces: Option<Arc<BTreeMap<u32, u8>>>,
}

impl VisionOperands {
    fn read(vision: &Vision) -> Self {
        Self {
            ours: vision.ours(),
            fog: vision.fog(),
            visibility_gap: vision.visibility_gap().is_some(),
            provinces: vision.shared_province_fog().cloned(),
        }
    }

    fn sees_province_object(&self, province: u32) -> Option<bool> {
        if !self.fog {
            return Some(true);
        }
        self.provinces
            .as_ref()?
            .get(&province)
            .map(|&byte| byte >= MAP_OBJECT_VISIBILITY)
    }
}

// A base holds its own inputs, never an earlier base.
pub struct BuildingsBase {
    assets: u64,
    defines: Defines,
    facts: Arc<viewer_foundation::map::building_facts::BuildingFacts>,
    city: Vec<bool>,
    damage_refused: bool,
    frame: DecodedFrame,
    vision: VisionOperands,
    tokens: Arc<TokenTypes>,
    gates: Arc<GateInputs>,
    slots: Chunked<PreparedSlot>,
    tallies: Chunked<Tally>,
    totals: SlotSummary,
    path: BuildPath,
}

impl BuildingsBase {
    pub fn path(&self) -> &BuildPath {
        &self.path
    }
}

pub(super) struct Built {
    pub(super) prepared: PreparedBuildings,
    pub(super) summary: MapBuildingSummary,
    pub(super) tallies: Chunked<Tally>,
    pub(super) totals: SlotSummary,
    pub(super) path: BuildPath,
    pub(super) damage_refused: bool,
    pub(super) tokens: Arc<TokenTypes>,
    pub(super) gates: Arc<GateInputs>,
    pub(super) city: Option<Vec<bool>>,
}

impl Built {
    pub(super) fn base(
        &self,
        assets: &MapBuildingAssets,
        frame: &DecodedFrame,
        vision: &Vision,
        city: Vec<bool>,
    ) -> BuildingsBase {
        BuildingsBase {
            assets: assets.id,
            defines: assets.defines,
            facts: Arc::clone(&assets.facts),
            city,
            damage_refused: self.damage_refused,
            frame: project(frame, &BASE_TABLES),
            vision: VisionOperands::read(vision),
            tokens: Arc::clone(&self.tokens),
            gates: Arc::clone(&self.gates),
            slots: self.prepared.slots.clone(),
            tallies: self.tallies.clone(),
            totals: self.totals.clone(),
            path: self.path.clone(),
        }
    }
}

pub(super) fn next_assets() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

// Unordered dictionaries defer to the full reader's named refusal.
fn token_types_from(
    old_frame: &DecodedFrame,
    old: &TokenTypes,
    frame: &DecodedFrame,
    keys: &TokenKeys,
    index: &BTreeMap<String, usize>,
) -> Option<TokenTypes> {
    let before = rows::<tables::LexicalTokens>(old_frame).ok()?.rows();
    let after = rows::<tables::LexicalTokens>(frame).ok()?.rows();
    let changed = changed_keys(before, after, |row| row.token)?;
    let mut types = old.clone();
    for token in changed {
        types.remove(&token);
        if let Ok(at) = after.binary_search_by_key(&token, |row| row.token) {
            let row = TokenNames::from_rows(std::slice::from_ref(&after[at]), true);
            types.extend(named_types(&row, keys, index));
        }
    }
    Some(types)
}

pub(super) struct Sites {
    slots: Vec<(u32, u32)>,
    groups: Vec<SlotKey>,
    by_key: Vec<((bool, u32, usize), u32)>,
    by_location: Vec<((bool, u32), u32)>,
    by_province: Vec<(u32, u32)>,
    hubs: Vec<u32>,
    hub_points: Vec<(f32, f32)>,
}

impl Sites {
    fn read(assets: &MapBuildingAssets) -> Self {
        let mut sites = Self {
            slots: Vec::new(),
            groups: Vec::with_capacity(assets.slots.len()),
            by_key: Vec::new(),
            by_location: Vec::new(),
            by_province: Vec::new(),
            hubs: Vec::new(),
            hub_points: Vec::new(),
        };
        for (group, (key, positions)) in assets.slots.iter().enumerate() {
            sites.groups.push(key.clone());
            let candidates = assets.candidates.get(&key.1).map_or(&[][..], Vec::as_slice);
            for (ordinal, (position, province)) in positions.iter().enumerate() {
                let id = sites.slots.len() as u32;
                sites.slots.push((group as u32, ordinal as u32));
                if let Some(province) = province {
                    sites.by_province.push((*province, id));
                }
                if candidates
                    .iter()
                    .any(|&index| assets.facts.ordered[index].supply_node)
                {
                    sites.hubs.push(id);
                    sites.hub_points.push((position.x, position.z));
                }
                for &index in candidates {
                    let provincial = assets.facts.ordered[index].provincial();
                    let location = if provincial {
                        let Some(province) = province else {
                            continue;
                        };
                        *province
                    } else {
                        position.state
                    };
                    sites.by_key.push(((provincial, location, index), id));
                    sites.by_location.push(((provincial, location), id));
                }
            }
        }
        sites.by_key.sort_unstable();
        sites.by_key.dedup();
        sites.by_location.sort_unstable();
        sites.by_location.dedup();
        sites.by_province.sort_unstable();
        sites
    }

    fn reached<K: Ord>(index: &[(K, u32)], key: K) -> impl Iterator<Item = u32> + '_ {
        let start = index.partition_point(|(at, _)| *at < key);
        index[start..]
            .iter()
            .take_while(move |(at, _)| *at == key)
            .map(|(_, id)| *id)
    }

    fn at_location(&self, location: u32) -> impl Iterator<Item = u32> + '_ {
        Self::reached(&self.by_location, (false, location))
            .chain(Self::reached(&self.by_location, (true, location)))
    }
}

pub(super) fn changed_keys<R: PartialEq, K: Ord + Copy>(
    old: &[R],
    new: &[R],
    key: impl Fn(&R) -> K,
) -> Option<Vec<K>> {
    let ascending = |rows: &[R]| rows.windows(2).all(|pair| key(&pair[0]) < key(&pair[1]));
    if !ascending(old) || !ascending(new) {
        return None;
    }
    let (mut a, mut b) = (0, 0);
    let mut out = Vec::new();
    while a < old.len() || b < new.len() {
        match (old.get(a), new.get(b)) {
            (Some(x), Some(y)) => match key(x).cmp(&key(y)) {
                std::cmp::Ordering::Less => {
                    out.push(key(x));
                    a += 1;
                }
                std::cmp::Ordering::Greater => {
                    out.push(key(y));
                    b += 1;
                }
                std::cmp::Ordering::Equal => {
                    if x != y {
                        out.push(key(x));
                    }
                    a += 1;
                    b += 1;
                }
            },
            (Some(x), None) => {
                out.push(key(x));
                a += 1;
            }
            (None, Some(y)) => {
                out.push(key(y));
                b += 1;
            }
            (None, None) => break,
        }
    }
    Some(out)
}

fn changed_entries<K: Ord + Copy, V: PartialEq>(
    old: &BTreeMap<K, V>,
    new: &BTreeMap<K, V>,
) -> Vec<K> {
    let mut out = Vec::new();
    let (mut olds, mut news) = (old.iter(), new.iter());
    let (mut a, mut b) = (olds.next(), news.next());
    loop {
        match (a, b) {
            (Some((x, old)), Some((y, new))) => match x.cmp(y) {
                std::cmp::Ordering::Less => {
                    out.push(*x);
                    a = olds.next();
                }
                std::cmp::Ordering::Greater => {
                    out.push(*y);
                    b = news.next();
                }
                std::cmp::Ordering::Equal => {
                    if old != new {
                        out.push(*x);
                    }
                    a = olds.next();
                    b = news.next();
                }
            },
            (Some((x, _)), None) => {
                out.push(*x);
                a = olds.next();
            }
            (None, Some((y, _))) => {
                out.push(*y);
                b = news.next();
            }
            (None, None) => return out,
        }
    }
}

fn named(types: &TokenTypes, token: u32) -> Option<usize> {
    types
        .get(&token)
        .and_then(|named| named.as_ref().ok())
        .copied()
}

fn type_indices(base: &TokenTypes, hour: &TokenTypes, token: u32) -> [Option<usize>; 2] {
    let old = named(base, token);
    [old, named(hour, token).filter(|new| Some(*new) != old)]
}

fn retyped_tokens(base: &Arc<TokenTypes>, hour: &Arc<TokenTypes>) -> BTreeSet<u32> {
    if Arc::ptr_eq(base, hour) {
        return BTreeSet::new();
    }
    base.keys()
        .chain(hour.keys())
        .filter(|&&token| named(base, token) != named(hour, token))
        .copied()
        .collect()
}

fn row_key(row: &BuildingRow) -> (u32, u32) {
    (row.location, row.building_type)
}

// Slots read health classifications, not raw health, ordinal or active flags.
type SlotRead = (i16, (bool, bool, bool), u32, u32);

fn slot_read(row: &BuildingRow, defines: &Defines) -> SlotRead {
    (
        row.level,
        health_state(row.level, row.healthy_level, row.partial_health, defines),
        row.status_flags,
        row.country,
    )
}

fn same_slot_read(
    old: &[BuildingRow],
    new: &[BuildingRow],
    key: (u32, u32),
    defines: &Defines,
) -> bool {
    let read = |rows: &[BuildingRow]| {
        rows.binary_search_by_key(&key, row_key)
            .ok()
            .map(|at| slot_read(&rows[at], defines))
    };
    read(old) == read(new)
}

// Unchanged row operands preserve the base's gate dependencies.
fn read_by(base: &BuildingsBase, flag: u16, slot: u32) -> bool {
    base.tallies[slot as usize].reads(flag)
}

pub(super) struct Affected {
    pub(super) slots: Vec<u32>,
}

fn completeness(frame: &DecodedFrame, table: TableId) -> (u8, u32, &str) {
    match &*frame.tables[table.index()] {
        replay_frame::frame::TableView::Complete(_) => (0, 0, ""),
        replay_frame::frame::TableView::Truncated { missing, .. } => (1, *missing, ""),
        replay_frame::frame::TableView::Unavailable { reason } => (2, 0, reason.as_str()),
        replay_frame::frame::TableView::NotRecorded { mode } => (3, 0, mode.name()),
    }
}

const COMPLETENESS: [(TableId, &str); 4] = [
    (
        TableId::StateBuildings,
        "the StateBuildings completeness changed",
    ),
    (
        TableId::ProvinceBuildings,
        "the ProvinceBuildings completeness changed",
    ),
    (TableId::NavalBases, "the NavalBases completeness changed"),
    (
        TableId::RaidTargetStatuses,
        "the RaidTargetStatuses completeness changed",
    ),
];

pub(super) struct Inputs<'a> {
    pub(super) frame: &'a DecodedFrame,
    pub(super) vision: &'a Vision,
    pub(super) detection: &'a Detection<'a>,
    pub(super) naval_bases: &'a BTreeSet<u32>,
    pub(super) tokens: &'a Arc<TokenTypes>,
    pub(super) city: &'a [bool],
    pub(super) damage_refused: bool,
}

impl MapBuildingAssets {
    pub(super) fn sites(&self) -> &Sites {
        self.sites.get_or_init(|| Sites::read(self))
    }

    pub(super) fn token_map(&self, frame: &DecodedFrame) -> io::Result<Arc<TokenTypes>> {
        let (keys, index) = (&self.facts.keys, &self.facts.index);
        let last = self.last_tokens.lock().ok().and_then(|last| last.clone());
        let diffed = last
            .and_then(|(old_frame, old)| token_types_from(&old_frame, &old, frame, keys, index));
        let types = Arc::new(match diffed {
            Some(types) => types,
            None => token_types(frame, keys, index)?,
        });
        if let Ok(mut last) = self.last_tokens.lock() {
            *last = Some((frame.clone(), Arc::clone(&types)));
        }
        Ok(types)
    }

    pub(super) fn city_bits(&self, city: &CityTerrain<'_>) -> Vec<bool> {
        self.sites()
            .hub_points
            .iter()
            .map(|&(x, z)| city.at(x, z))
            .collect()
    }

    pub(super) fn ordinary_slot_count(&self) -> usize {
        self.sites().slots.len()
    }

    pub(super) fn affected(
        &self,
        base: &BuildingsBase,
        hour: &Inputs<'_>,
    ) -> Result<Affected, &'static str> {
        let same =
            |table: TableId| base.frame.tables[table.index()] == hour.frame.tables[table.index()];
        if base.assets != self.id {
            return Err("the base belongs to other content");
        }
        if base.frame.branch != hour.frame.branch {
            return Err("the base is on another branch");
        }
        if base.frame.hour >= hour.frame.hour {
            return Err("the base is not an earlier hour");
        }
        if base.defines != self.defines || !Arc::ptr_eq(&base.facts, &self.facts) {
            return Err("the building content operands changed");
        }
        if !same(TableId::CountryLinks) {
            return Err("the country links changed");
        }
        if base.vision.ours != hour.vision.ours() || base.vision.fog != hour.vision.fog() {
            return Err("the viewing slot or the fog switch changed");
        }
        if base.vision.visibility_gap || hour.vision.visibility_gap().is_some() {
            return Err("a fog rebuild gap leaves the fog bytes undecided");
        }
        for (table, reason) in COMPLETENESS {
            if completeness(&base.frame, table) != completeness(hour.frame, table) {
                return Err(reason);
            }
        }
        if base.damage_refused {
            return Err("the base's city damage was refused");
        }
        if hour.damage_refused {
            return Err("this hour's city damage is refused");
        }
        let sites = self.sites();
        if base.slots.len() != sites.slots.len() + self.floating.len()
            || base.tallies.len() != sites.slots.len()
            || base.city.len() != sites.hubs.len()
            || hour.city.len() != sites.hubs.len()
        {
            return Err("the base holds another number of slots");
        }
        let unreadable = |_: io::Error| "a base table is unreadable";
        let gates = &base.gates;
        let targets = match (raid_targets(&base.frame), &hour.detection.targets) {
            (Some(old), Some(new)) => changed_entries(&old, new),
            (None, None) => Vec::new(),
            _ => return Err("the RaidTargetStatuses completeness changed"),
        };
        let mut affected = Vec::new();
        let building_rows = [
            (
                false,
                rows::<tables::StateBuildings>(&base.frame)
                    .map_err(unreadable)?
                    .rows(),
                rows::<tables::StateBuildings>(hour.frame)
                    .map_err(unreadable)?
                    .rows(),
            ),
            (
                true,
                rows::<tables::ProvinceBuildings>(&base.frame)
                    .map_err(unreadable)?
                    .rows(),
                rows::<tables::ProvinceBuildings>(hour.frame)
                    .map_err(unreadable)?
                    .rows(),
            ),
        ];
        let mut changed = Vec::with_capacity(building_rows.len());
        for (_, old, new) in building_rows {
            changed.push(
                changed_keys(old, new, row_key)
                    .ok_or("building rows are not in strict key order")?,
            );
        }
        for ((provincial, old, new), changed) in building_rows.iter().zip(&changed) {
            for &(location, token) in changed {
                if same_slot_read(old, new, (location, token), &self.defines) {
                    continue;
                }
                for index in type_indices(&base.tokens, hour.tokens, token)
                    .into_iter()
                    .flatten()
                {
                    affected.extend(Sites::reached(
                        &sites.by_key,
                        (*provincial, location, index),
                    ));
                }
            }
        }
        // Retyping also reaches unchanged rows under both the old and new type.
        let retyped = retyped_tokens(&base.tokens, hour.tokens);
        if !retyped.is_empty() {
            for (provincial, old, new) in building_rows {
                for row in old
                    .iter()
                    .chain(new)
                    .filter(|row| retyped.contains(&row.building_type))
                {
                    for index in type_indices(&base.tokens, hour.tokens, row.building_type)
                        .into_iter()
                        .flatten()
                    {
                        affected.extend(Sites::reached(
                            &sites.by_key,
                            (provincial, row.location, index),
                        ));
                    }
                }
            }
        }
        if let (Some(old), Some(new)) = (base.vision.provinces.as_ref(), hour.vision.province_fog())
        {
            for province in changed_entries(old, new) {
                if base.vision.sees_province_object(province)
                    != hour.vision.sees_province_object(province, false)
                {
                    affected.extend(
                        Sites::reached(&sites.by_province, province)
                            .filter(|&slot| read_by(base, Tally::READ_FOG, slot)),
                    );
                }
            }
        }
        for (index, &slot) in sites.hubs.iter().enumerate() {
            if base.city[index] != hour.city[index] && read_by(base, Tally::READ_HUB, slot) {
                affected.push(slot);
            }
        }
        for province in gates.ports.symmetric_difference(hour.naval_bases) {
            affected.extend(
                Sites::reached(&sites.by_province, *province)
                    .filter(|&slot| read_by(base, Tally::READ_HUB, slot)),
            );
        }
        let provinces = changed_entries(&gates.provinces, &hour.detection.provinces);
        let states = changed_entries(&gates.states, &hour.detection.states);
        for locations in [
            provinces,
            states,
            targets.into_iter().map(|scope| scope.1).collect(),
        ] {
            for location in locations {
                affected.extend(
                    sites
                        .at_location(location)
                        .filter(|&slot| read_by(base, Tally::READ_DETECTION, slot)),
                );
            }
        }
        affected.sort_unstable();
        affected.dedup();
        Ok(Affected { slots: affected })
    }

    pub(super) fn site(&self, id: u32) -> (&SlotKey, usize, &BuildingPosition, Option<u32>) {
        let sites = self.sites();
        let (group, ordinal) = sites.slots[id as usize];
        let key = &sites.groups[group as usize];
        let (position, province) = &self.slots[key][ordinal as usize];
        (key, ordinal as usize, position, *province)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl PreparedBuildings {
    // Float bits distinguish signed zeros and NaN payloads.
    pub fn bit_eq(&self, other: &Self) -> bool {
        let float = |a: f32, b: f32| a.to_bits() == b.to_bits();
        let entity = |a: &PreparedEntity, b: &PreparedEntity| {
            (a.healthy, a.ruined, a.destroyed, a.uncertain, a.damaged)
                == (b.healthy, b.ruined, b.destroyed, b.uncertain, b.damaged)
                && a.disable_grow_animation == b.disable_grow_animation
                && bytemuck::bytes_of(&a.instance) == bytemuck::bytes_of(&b.instance)
        };
        let slot = |a: &PreparedSlot, b: &PreparedSlot| {
            (a.id, a.drawn_at_distance) == (b.id, b.drawn_at_distance)
                && match (&a.entity, &b.entity) {
                    (Some(a), Some(b)) => entity(a, b),
                    (None, None) => true,
                    _ => false,
                }
        };
        let group = |a: &ProvinceSlots, b: &ProvinceSlots| {
            a.province == b.province
                && a.slots == b.slots
                && a.bounds.iter().zip(&b.bounds).all(|(a, b)| float(*a, *b))
        };
        (self.branch, self.hour) == (other.branch, other.hour)
            && float(self.shrink_distance, other.shrink_distance)
            && float(self.popup_speed, other.popup_speed)
            && self.slots.len() == other.slots.len()
            && (0..self.slots.len()).all(|id| slot(&self.slots[id], &other.slots[id]))
            && self.groups.len() == other.groups.len()
            && self
                .groups
                .iter()
                .zip(other.groups.iter())
                .all(|(a, b)| group(a, b))
            && self.city_damage == other.city_damage
    }
}

pub(super) type Patch = (Vec<(u32, PreparedSlot)>, Vec<(u32, Tally)>, SlotSummary);

pub(super) fn patch(
    base: &BuildingsBase,
    affected: &[u32],
    types: &[BuildingType],
    mut evaluate: impl FnMut(u32) -> (PreparedSlot, Tally),
) -> Patch {
    let mut totals = base.totals.clone();
    let mut slots = Vec::with_capacity(affected.len());
    let mut tallies = Vec::with_capacity(affected.len());
    for &id in affected {
        let (slot, tally) = evaluate(id);
        if let Some(old) = base.tallies.get(id as usize) {
            totals.remove(*old, types);
        }
        totals.add(tally, types);
        slots.push((id, slot));
        tallies.push((id, tally));
    }
    (slots, tallies, totals)
}

pub(super) enum Ordinary<'a> {
    Full {
        slots: Vec<PreparedSlot>,
        tallies: Vec<Tally>,
        totals: SlotSummary,
        reason: &'static str,
    },
    Patched {
        base: &'a BuildingsBase,
        slots: Vec<(u32, PreparedSlot)>,
        tallies: Vec<(u32, Tally)>,
        totals: SlotSummary,
    },
}

impl Ordinary<'_> {
    pub(super) fn finish(
        self,
        harbors: Vec<PreparedSlot>,
        ordinary: usize,
    ) -> (
        Chunked<PreparedSlot>,
        Chunked<Tally>,
        SlotSummary,
        BuildPath,
    ) {
        match self {
            Self::Full {
                mut slots,
                tallies,
                totals,
                reason,
            } => {
                slots.extend(harbors);
                (
                    slots.into(),
                    tallies.into(),
                    totals,
                    BuildPath::Full(reason),
                )
            }
            Self::Patched {
                base,
                slots,
                tallies,
                totals,
            } => {
                let evaluated = slots.len();
                let patched = base
                    .slots
                    .patched(&slots, ordinary, &harbors)
                    .zip(base.tallies.patched(&tallies, ordinary, &[]));
                let (slots, tallies) =
                    patched.expect("the plan checked the base's slot and tally counts");
                (
                    slots,
                    tallies,
                    totals,
                    BuildPath::Patched { slots: evaluated },
                )
            }
        }
    }
}

#[cfg(test)]
mod tests;
