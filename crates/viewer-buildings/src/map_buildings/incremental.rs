use super::*;
use map_display_data::copy_index::CopyIndex;
use replay_data::rows::RowCodec;
use replay_frame::frame::ViewIdentity;
use std::sync::atomic::{AtomicU64, Ordering};
use viewer_foundation::perspective::MAP_OBJECT_VISIBILITY;

const CHUNK_SHIFT: u32 = 6;
pub(super) const CHUNK_SIZE: usize = 1usize << CHUNK_SHIFT;

use map_display_data::{retained::Retained, value_index::ValueIndex};

pub(super) struct ChunkBuilder<T: PartialEq> {
    directory: ValueIndex<usize, Retained<Arc<[T]>>>,
    pending: Vec<T>,
    len: usize,
}

impl<T: PartialEq> ChunkBuilder<T> {
    pub(super) fn new() -> Self {
        Self {
            directory: ValueIndex::default(),
            pending: Vec::new(),
            len: 0,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.len
    }

    pub(super) fn push(&mut self, value: T) -> io::Result<()> {
        if self.pending.capacity() == 0 {
            self.pending
                .try_reserve_exact(CHUNK_SIZE)
                .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        }
        self.pending.push(value);
        self.len += 1;
        if self.pending.len() == CHUNK_SIZE {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let pending = std::mem::take(&mut self.pending);
        let chunk = Retained::new(Arc::<[T]>::from(pending));
        self.directory = self.directory.inserted(self.directory.len(), chunk)?;
        Ok(())
    }

    pub(super) fn finish(mut self) -> io::Result<Chunked<T>> {
        self.flush()?;
        Chunked::from_directory(self.directory)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Chunked<T: PartialEq> {
    shift: u32,
    pub(super) chunks: Arc<ChunkDirectory<T>>,
}
#[derive(Clone, Debug)]
pub(super) struct ChunkDirectory<T> {
    values: ValueIndex<usize, Retained<Arc<[T]>>>,
}
impl<T: PartialEq> PartialEq for ChunkDirectory<T> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}
impl<T: PartialEq> ChunkDirectory<T> {
    fn new(values: ValueIndex<usize, Retained<Arc<[T]>>>) -> Self {
        Self { values }
    }
    pub(super) fn len(&self) -> usize {
        self.values.len()
    }
    pub(super) fn get(&self, index: usize) -> Option<&Arc<[T]>> {
        self.values.get(&index).map(|value| &**value)
    }
    pub(super) fn last(&self) -> Option<&Arc<[T]>> {
        self.len().checked_sub(1).and_then(|index| self.get(index))
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = &Arc<[T]>> {
        (0..self.len()).map(|index| self.get(index).unwrap())
    }
    pub(super) fn changed_pages(&self, previous: &Self) -> io::Result<CopyIndex<usize, ()>> {
        let mut changed = CopyIndex::default();
        let mut error = None;
        self.values.visit_changes(&previous.values, |index, _, _| {
            if error.is_none() {
                match changed.inserted(*index, ()) {
                    Ok(next) => changed = next,
                    Err(failed) => error = Some(failed),
                }
            }
        });
        match error {
            Some(error) => Err(error),
            None => Ok(changed),
        }
    }
}
impl<T: PartialEq> Chunked<T> {
    pub(super) fn shared(chunks: Arc<ChunkDirectory<T>>) -> Self {
        Self {
            shift: CHUNK_SHIFT,
            chunks,
        }
    }
    pub(super) fn from_directory(
        values: ValueIndex<usize, Retained<Arc<[T]>>>,
    ) -> io::Result<Self> {
        Ok(Self {
            shift: CHUNK_SHIFT,
            chunks: Arc::new(ChunkDirectory::new(values)),
        })
    }
    pub(super) fn changed_chunks(&self, previous: &Self) -> io::Result<CopyIndex<usize, ()>> {
        self.chunks.changed_pages(&previous.chunks)
    }

    pub(super) fn directory(&self) -> ValueIndex<usize, Retained<Arc<[T]>>> {
        self.chunks.values.clone()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn from_values(values: Vec<T>) -> io::Result<Self> {
        let mut directory = ValueIndex::default();
        let mut values = values.into_iter();
        let mut index = 0;
        while !values.as_slice().is_empty() {
            let count = values.as_slice().len().min(CHUNK_SIZE);
            let chunk = Retained::new(values.by_ref().take(count).collect::<Arc<[T]>>());
            directory = directory.inserted(index, chunk)?;
            index += 1;
        }
        Ok(Self {
            shift: CHUNK_SHIFT,
            chunks: Arc::new(ChunkDirectory::new(directory)),
        })
    }
    pub(super) fn get(&self, id: usize) -> Option<&T> {
        self.chunks
            .get(id >> self.shift)?
            .get(id & ((1usize << self.shift) - 1))
    }
    pub(super) fn len(&self) -> usize {
        self.chunks.last().map_or(0, |tail| {
            ((self.chunks.len() - 1) << self.shift) + tail.len()
        })
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
impl<T: Clone + PartialEq> Chunked<T> {
    pub(super) fn patched(
        &self,
        changed: &[(u32, T)],
        tail_from: usize,
        tail: &[T],
        same: impl Fn(&T, &T) -> bool,
    ) -> io::Result<Option<Self>> {
        if self.len() != tail_from + tail.len() {
            return Ok(None);
        }
        if changed.is_empty()
            && tail.iter().enumerate().all(|(index, value)| {
                self.get(tail_from + index)
                    .is_some_and(|old| same(old, value))
            })
        {
            return Ok(Some(self.clone()));
        }
        let mut touched = CopyIndex::default();
        for (id, _) in changed {
            touched = touched.inserted((*id as usize) >> self.shift, ())?;
        }
        if !tail.is_empty() {
            for page in (tail_from >> self.shift)..self.chunks.len() {
                touched = touched.inserted(page, ())?;
            }
        }
        let mut values = self.chunks.values.clone();
        let mut edits = Vec::with_capacity(touched.len());
        for (&index, _) in touched.iter() {
            let base = self.chunks.get(index).expect("checked chunk index");
            let start = index << self.shift;
            let chunk = Retained::new(
                base.iter()
                    .enumerate()
                    .map(|(offset, old)| {
                        let id = start + offset;
                        if id >= tail_from {
                            tail[id - tail_from].clone()
                        } else {
                            changed
                                .binary_search_by_key(&(id as u32), |entry| entry.0)
                                .ok()
                                .map_or_else(|| old.clone(), |at| changed[at].1.clone())
                        }
                    })
                    .collect::<Arc<[T]>>(),
            );
            edits.push((index, Some(chunk)));
        }
        values = values.patched(&edits)?;
        Ok(Some(Self {
            shift: self.shift,
            chunks: Arc::new(ChunkDirectory::new(values)),
        }))
    }
}
impl<T: PartialEq> std::ops::Index<usize> for Chunked<T> {
    type Output = T;
    fn index(&self, id: usize) -> &T {
        self.get(id).expect("slot id within the hour's slots")
    }
}
#[cfg(any(test, feature = "test-support"))]
impl<T: PartialEq> From<Vec<T>> for Chunked<T> {
    fn from(values: Vec<T>) -> Self {
        Self::from_values(values).unwrap()
    }
}
#[cfg(any(test, feature = "test-support"))]
impl<T: PartialEq> FromIterator<T> for Chunked<T> {
    fn from_iter<I: IntoIterator<Item = T>>(values: I) -> Self {
        values.into_iter().collect::<Vec<_>>().into()
    }
}
#[cfg(any(test, feature = "test-support"))]
impl<T: Clone + PartialEq> Chunked<T> {
    #[cfg(test)]
    pub(super) fn iter(&self) -> impl Iterator<Item = &T> {
        self.chunks.iter().flat_map(|chunk| chunk.iter())
    }

    pub(super) fn edit(&mut self, id: usize, edit: impl FnOnce(&mut T)) {
        let index = id >> self.shift;
        let offset = id & ((1usize << self.shift) - 1);
        let mut chunk = self.chunks.get(index).unwrap().to_vec();
        edit(&mut chunk[offset]);
        let values = self
            .chunks
            .values
            .inserted(index, Retained::new(chunk.into()))
            .unwrap();
        self.chunks = Arc::new(ChunkDirectory::new(values));
    }

    pub(super) fn edit_all(&mut self, mut edit: impl FnMut(&mut T)) {
        let edits: Vec<_> = self
            .chunks
            .values
            .iter()
            .map(|(&index, chunk)| {
                let mut chunk = chunk.to_vec();
                chunk.iter_mut().for_each(&mut edit);
                (index, Some(Retained::new(chunk.into())))
            })
            .collect();
        let values = self.chunks.values.patched(&edits).unwrap();
        self.chunks = Arc::new(ChunkDirectory::new(values));
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

    pub(super) fn reads(self, flag: u16) -> bool {
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
    unresolved: CopyIndex<usize, usize>,
}

impl SlotSummary {
    pub(super) fn add(&mut self, tally: Tally, _types: &[BuildingType]) -> io::Result<()> {
        self.fogged += tally.has(Tally::FOGGED);
        self.hubs_left += tally.has(Tally::HUB_LEFT);
        self.unknown_hub += tally.has(Tally::HUB_UNKNOWN);
        self.undetected += tally.has(Tally::UNDETECTED);
        self.unknown_detection += tally.has(Tally::DETECTION_UNKNOWN);
        self.placed += tally.has(Tally::PLACED);
        if let Some(index) = tally.unresolved {
            self.unresolved = self
                .unresolved
                .inserted(index, self.unresolved.get(&index).copied().unwrap_or(0) + 1)?;
        }
        Ok(())
    }

    fn remove(&mut self, tally: Tally, _types: &[BuildingType]) -> io::Result<()> {
        self.fogged -= tally.has(Tally::FOGGED);
        self.hubs_left -= tally.has(Tally::HUB_LEFT);
        self.unknown_hub -= tally.has(Tally::HUB_UNKNOWN);
        self.undetected -= tally.has(Tally::UNDETECTED);
        self.unknown_detection -= tally.has(Tally::DETECTION_UNKNOWN);
        self.placed -= tally.has(Tally::PLACED);
        if let Some(index) = tally.unresolved {
            if let Some(&count) = self.unresolved.get(&index) {
                self.unresolved = if count == 1 {
                    self.unresolved.removed(&index)?
                } else {
                    self.unresolved.inserted(index, count - 1)?
                };
            }
        }
        Ok(())
    }

    pub(super) fn apply(&self, summary: &mut MapBuildingSummary, types: &[BuildingType]) {
        summary.fogged += self.fogged;
        summary.hubs_left_to_another_layer += self.hubs_left;
        summary.unknown_hub_membership += self.unknown_hub;
        summary.undetected += self.undetected;
        summary.unknown_detection += self.unknown_detection;
        summary.placed += self.placed;
        for (&index, count) in self.unresolved.iter() {
            *summary
                .unresolved_by_type
                .entry(types[index].name.clone())
                .or_default() += count;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildPath {
    Full(&'static str),
    Patched { slots: usize },
}

#[cfg(test)]
#[derive(Debug, PartialEq)]
pub(super) struct GateInputs {
    pub(super) ports: CopyIndex<u32, ()>,
    pub(super) provinces: CopyIndex<u32, u32>,
    pub(super) states: CopyIndex<u32, u32>,
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
            ports: {
                let mut ports = CopyIndex::default();
                for row in rows::<tables::NavalBases>(frame)?.rows() {
                    ports = ports.inserted(row.province, ())?;
                }
                ports
            },
            provinces: detection.provinces,
            states: detection.states,
        })
    }
}

struct VisionOperands {
    ours: u32,
    fog: bool,
    visibility_gap: bool,
    provinces: Option<Arc<viewer_foundation::perspective::values::Values<u32, u8>>>,
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

pub(super) fn fog_changes(
    base: Option<&BuildingsBase>,
    vision: &Vision,
) -> io::Result<Option<CopyIndex<u32, ()>>> {
    let Some(base) = base else { return Ok(None) };
    if base.vision.ours != vision.ours()
        || base.vision.fog != vision.fog()
        || base.vision.visibility_gap
        || vision.visibility_gap().is_some()
    {
        return Ok(None);
    };
    let mut changes = CopyIndex::default();
    match (&base.vision.provinces, vision.shared_province_fog()) {
        (None, None) => {}
        (Some(old), Some(new)) if Arc::ptr_eq(old, new) => {}
        (Some(old), Some(new)) => {
            let mut result = Ok(());
            new.changed(old, |province| {
                if result.is_ok()
                    && old.get(&province).map(|byte| fog_decisions(*byte))
                        != new.get(&province).map(|byte| fog_decisions(*byte))
                {
                    result = changes.inserted(province, ()).map(|next| changes = next);
                }
            });
            result?;
        }
        _ => return Ok(None),
    }
    Ok(Some(changes))
}

pub(super) struct CityBank {
    values: std::sync::Weak<Vec<bool>>,
    width: u32,
    height: u32,
}
impl CityBank {
    pub(super) fn new(city: &CityTerrain<'_>, values: &Arc<Vec<bool>>) -> io::Result<Self> {
        if !std::ptr::eq(city.spawn_city, values.as_slice()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "city terrain does not belong to its immutable bank",
            ));
        }
        Ok(Self {
            values: Arc::downgrade(values),
            width: city.width,
            height: city.height,
        })
    }
    pub(super) fn same(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.values.ptr_eq(&other.values)
    }
}
impl BuildingsBase {
    pub(super) fn available_for(&self, assets: u64) -> Option<&super::available::AvailableRows> {
        (self.assets == assets).then_some(&self.available)
    }
    pub(super) fn floating_for(&self, assets: u64) -> Option<&FloatingInputs> {
        (self.assets == assets).then_some(&self.floating)
    }
    pub(super) fn city_for(&self, bank: &CityBank, assets: u64) -> Option<&Retained<Vec<bool>>> {
        if self.assets != assets {
            return None;
        }
        self.city_bank
            .as_ref()
            .filter(|old| old.same(bank))
            .map(|_| &self.city)
    }
}

pub struct BuildingsBase {
    assets: u64,
    defines: Defines,
    facts: Arc<viewer_foundation::building_facts::BuildingFacts>,
    city: Retained<Vec<bool>>,
    pub(super) city_bank: Option<CityBank>,
    damage_refused: bool,
    available: super::available::AvailableRows,
    pub(super) floating: FloatingInputs,
    pub(super) floating_output: super::floating::Output,
    pub(super) observation: Observation,
    vision: VisionOperands,
    tokens: Retained<TokenTypes>,
    #[cfg(test)]
    gates: Arc<GateInputs>,
    pub(super) slots: Chunked<PreparedSlot>,
    pub(super) display: Option<Arc<map_display_data::buildings::Buildings>>,
    tallies: Chunked<Tally>,
    totals: SlotSummary,
    path: BuildPath,
}

impl BuildingsBase {
    pub fn set_display(&mut self, display: Arc<map_display_data::buildings::Buildings>) {
        self.display = Some(display);
    }
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
    pub(super) available: Option<super::available::AvailableRows>,
    pub(super) floating: Option<FloatingInputs>,
    pub(super) floating_output: Option<super::floating::Output>,
    pub(super) tokens: Retained<TokenTypes>,
    #[cfg(test)]
    pub(super) gates: Arc<GateInputs>,
    pub(super) city: Option<Retained<Vec<bool>>>,
    pub(super) observation: Option<Observation>,
}

impl Built {
    pub(super) fn base(
        &mut self,
        assets: &MapBuildingAssets,
        vision: &Vision,
        city: Retained<Vec<bool>>,
    ) -> BuildingsBase {
        BuildingsBase {
            assets: assets.id,
            defines: assets.defines,
            facts: Arc::clone(&assets.facts),
            city,
            city_bank: None,
            damage_refused: self.damage_refused,
            available: self
                .available
                .take()
                .expect("one available-row continuation per extraction"),
            floating: self
                .floating
                .take()
                .expect("one floating continuation per extraction"),
            floating_output: self
                .floating_output
                .take()
                .expect("one floating output per extraction"),
            observation: self
                .observation
                .take()
                .expect("one continuation per extraction"),
            vision: VisionOperands::read(vision),
            tokens: self.tokens.clone(),
            #[cfg(test)]
            gates: Arc::clone(&self.gates),
            slots: self.prepared.slots.clone(),
            display: None,
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

#[cfg(test)]
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
        types.remove(&token).ok()?;
        if let Ok(at) = after.binary_search_by_key(&token, |row| row.token) {
            let row = TokenNames::from_rows(std::slice::from_ref(&after[at]), true);
            for (token, value) in named_types(&row, keys, index).ok()?.0.iter() {
                types.0 = types.0.inserted(*token, value.clone()).ok()?;
            }
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

    pub(super) fn slots_of(&self, key: (bool, u32, usize)) -> impl Iterator<Item = u32> + '_ {
        Self::reached(&self.by_key, key)
    }

    pub(super) fn provinces(&self) -> impl Iterator<Item = u32> + '_ {
        let mut last = None;
        self.by_province.iter().filter_map(move |&(province, _)| (last.replace(province) != Some(province)).then_some(province))
    }

    pub(super) fn slots_in(&self, province: u32) -> impl Iterator<Item = u32> + '_ {
        Self::reached(&self.by_province, province)
    }

    pub(super) fn at_location(&self, location: u32) -> impl Iterator<Item = u32> + '_ {
        Self::reached(&self.by_location, (false, location))
            .chain(Self::reached(&self.by_location, (true, location)))
    }
}

#[cfg(test)]
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

fn fog_decisions(byte: u8) -> (bool, bool) {
    (
        byte >= MAP_OBJECT_VISIBILITY,
        byte >= FLOATING_HARBOR_VISIBILITY,
    )
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

fn retyped_tokens(
    base: &Retained<TokenTypes>,
    hour: &Retained<TokenTypes>,
) -> io::Result<CopyIndex<u32, ()>> {
    let mut changed = CopyIndex::default();
    let mut error = None;
    (**hour).0.visit_changes(&(**base).0, |token, _, _| {
        if error.is_none() && named(base, *token) != named(hour, *token) {
            match changed.inserted(*token, ()) {
                Ok(next) => changed = next,
                Err(why) => error = Some(why),
            }
        }
    });
    match error {
        Some(error) => Err(error),
        None => Ok(changed),
    }
}

fn row_key(row: &BuildingRow) -> (u32, u32) {
    (row.location, row.building_type)
}

type SlotRead = (i16, (bool, bool, bool), u32, u32);

fn slot_fields(table: TableId) -> u64 {
    [
        "level",
        "healthy_level",
        "partial_health",
        "status_flags",
        "country",
    ]
    .into_iter()
    .fold(0, |mask, name| {
        mask | (0..table.schema().len())
            .find(|&bit| table.field_name(bit) == Some(name))
            .map_or(table.full_mask(), |bit| 1 << bit)
    })
}

fn slot_read(row: &BuildingRow, defines: &Defines) -> SlotRead {
    (
        row.level,
        health_state(row.level, row.healthy_level, row.partial_health, defines),
        row.status_flags,
        row.country,
    )
}

fn read_by(base: &BuildingsBase, flag: u16, slot: u32) -> bool {
    base.tallies[slot as usize].reads(flag)
}

pub(super) struct Affected {
    pub(super) slots: Retained<Vec<u32>>,
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

struct ScalarFold<V> {
    table: ViewIdentity,
    values: CopyIndex<u32, V>,
    changes: Vec<u32>,
}
fn fields(table: TableId, names: &[&str]) -> u64 {
    names.iter().fold(0, |mask, name| {
        mask | (0..table.schema().len())
            .find(|&bit| table.field_name(bit) == Some(*name))
            .map_or(table.full_mask(), |bit| 1 << bit)
    })
}
impl<V: Copy + PartialEq> ScalarFold<V> {
    fn read<R: RowCodec>(
        frame: &DecodedFrame,
        table: TableId,
        rows: &[R],
        mask: u64,
        previous: Option<&Self>,
        value: impl Fn(&R) -> V,
    ) -> io::Result<Self> {
        let view = &frame.tables[table.index()];
        let changes = replay_frame::frame::HourInput::new(frame).changes().since(
            table,
            previous.map(|old| &old.table),
            mask,
        );
        if let replay_frame::frame::TableChanges::Unchanged = changes {
            let old = previous.expect("certified scalar predecessor");
            return Ok(Self {
                table: view.identity(),
                values: old.values.clone(),
                changes: Vec::new(),
            });
        }
        let facts = match changes {
            replay_frame::frame::TableChanges::Rows(facts) => Some(facts),
            replay_frame::frame::TableChanges::Discontinuity(_) => None,
            replay_frame::frame::TableChanges::Unchanged => unreachable!(),
        };
        let count = facts
            .map_or_else(
                || {
                    rows.len()
                        .checked_add(previous.map_or(0, |old| old.values.len()))
                },
                |facts| facts.changed().len().checked_add(facts.deleted().len()),
            )
            .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?;
        let mut changes = Vec::with_capacity(count);
        let mut values = previous.map_or_else(CopyIndex::default, |old| old.values.clone());
        if let Some(facts) = facts {
            for &key in facts.deleted() {
                let key = u32::try_from(key).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "scalar change key exceeds u32")
                })?;
                if values.get(&key).is_some() {
                    values = values.removed(&key)?;
                    changes.push(key);
                }
            }
            for row in facts
                .changed()
                .iter()
                .filter(|row| row.inserted || row.fields & mask != 0)
            {
                let at = rows
                    .binary_search_by_key(&row.key, RowCodec::key)
                    .map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "scalar change fact has no current row",
                        )
                    })?;
                let key = u32::try_from(row.key).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "scalar change key exceeds u32")
                })?;
                let next = value(&rows[at]);
                if values.get(&key) != Some(&next) {
                    values = values.inserted(key, next)?;
                    changes.push(key);
                }
            }
        } else {
            values = CopyIndex::default();
            for row in rows {
                let key = u32::try_from(row.key()).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "scalar row key exceeds u32")
                })?;
                values = values.inserted(key, value(row))?;
            }
            if let Some(old) = previous {
                for (key, value) in old.values.iter() {
                    if values.get(key) != Some(value) {
                        changes.push(*key);
                    }
                }
                for (key, _) in values.iter() {
                    if old.values.get(key).is_none() {
                        changes.push(*key);
                    }
                }
            } else {
                changes.extend(values.iter().map(|(key, _)| *key));
            }
        }
        Ok(Self {
            table: view.identity(),
            values,
            changes,
        })
    }
}
struct TargetFold {
    rows: ScalarFold<(Scope, bool)>,
    by_scope_ordinal: CopyIndex<(Scope, u32), bool>,
    winners: CopyIndex<Scope, bool>,
    changes: Vec<Scope>,
}
impl TargetFold {
    fn read(frame: &DecodedFrame, ours: u32, previous: Option<&Self>) -> io::Result<Option<Self>> {
        let Ok(rows) = frame.complete::<tables::RaidTargetStatuses>() else {
            return Ok(None);
        };
        let mask = fields(
            TableId::RaidTargetStatuses,
            &[
                "scope.building_type",
                "scope.building_location",
                "scope.building_flags",
                "scope.building_country",
                "detected",
            ],
        );
        let rows = ScalarFold::read(
            frame,
            TableId::RaidTargetStatuses,
            rows,
            mask,
            previous.map(|old| &old.rows),
            |row| {
                let scope = &row.scope;
                let slot = ours as usize;
                (
                    (
                        scope.building_type,
                        scope.building_location,
                        scope.building_flags,
                        scope.building_country as u32,
                    ),
                    row.detected
                        .get(slot / 8)
                        .is_some_and(|byte| byte & (1 << (slot % 8)) != 0),
                )
            },
        )?;
        let count = rows
            .changes
            .len()
            .checked_mul(2)
            .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?;
        let mut by_scope_ordinal =
            previous.map_or_else(CopyIndex::default, |old| old.by_scope_ordinal.clone());
        let mut winners = previous.map_or_else(CopyIndex::default, |old| old.winners.clone());
        let mut dirty = Vec::with_capacity(count);
        for &ordinal in &rows.changes {
            if let Some(&(scope, _)) = previous
                .and_then(|old| old.rows.values.get(&ordinal))
                .filter(|(scope, _)| scope.0 != 0)
            {
                by_scope_ordinal = by_scope_ordinal.removed(&(scope, ordinal))?;
                dirty.push(scope);
            }
            if let Some(&(scope, detected)) =
                rows.values.get(&ordinal).filter(|(scope, _)| scope.0 != 0)
            {
                by_scope_ordinal = by_scope_ordinal.inserted((scope, ordinal), detected)?;
                dirty.push(scope);
            }
        }
        dirty.sort_unstable();
        dirty.dedup();
        let mut changes = Vec::with_capacity(dirty.len());
        for scope in dirty {
            let value = by_scope_ordinal
                .last_at_or_before(&(scope, u32::MAX))
                .filter(|((key, _), _)| *key == scope)
                .map(|(_, value)| *value);
            if winners.get(&scope).copied() != value {
                winners = match value {
                    Some(value) => winners.inserted(scope, value)?,
                    None => winners.removed(&scope)?,
                };
                changes.push(scope);
            }
        }
        Ok(Some(Self {
            rows,
            by_scope_ordinal,
            winners,
            changes,
        }))
    }
}

pub(super) struct FloatingInputs {
    branch: u64,
    supply: viewer_scene_base::projection_fold::RowFold<(u16, u8)>,
    deployments: viewer_scene_base::projection_fold::RowFold<(i32, u8)>,
    by_country_province_ordinal: CopyIndex<(u32, i32, u32), u8>,
    supply_complete: bool,
    deployments_complete: bool,
}
impl FloatingInputs {
    pub(super) fn read(frame: &DecodedFrame, previous: Option<&Self>) -> io::Result<Self> {
        use viewer_scene_base::projection_fold::{RowFold, fields};
        let previous = previous.filter(|old| old.branch == frame.branch);
        let supply = rows::<tables::ProvinceSupply>(frame)?;
        let deployments = rows::<tables::FloatingHarborDeployments>(frame)?;
        let supply_complete = supply.is_complete();
        let deployments_complete = deployments.is_complete();
        let supply = RowFold::read(
            frame,
            TableId::ProvinceSupply,
            supply.rows(),
            fields(TableId::ProvinceSupply, &["controller_index", "flags"]),
            previous.map(|old| &old.supply),
            |row| Ok((row.controller_index, row.flags & 1)),
        )?;
        let deployments = RowFold::read(
            frame,
            TableId::FloatingHarborDeployments,
            deployments.rows(),
            fields(TableId::FloatingHarborDeployments, &["province", "status"]),
            previous.map(|old| &old.deployments),
            |row| Ok((row.province, row.status)),
        )?;
        let mut by_country_province_ordinal = previous.map_or_else(CopyIndex::default, |old| {
            old.by_country_province_ordinal.clone()
        });
        for &key in deployments.edits.iter() {
            let country = (key >> 32) as u32;
            let ordinal = key as u32;
            if let Some(&(province, _)) = previous.and_then(|old| old.deployments.values.get(&key))
            {
                by_country_province_ordinal =
                    by_country_province_ordinal.removed(&(country, province, ordinal))?;
            }
            if let Some(&(province, status)) = deployments.values.get(&key) {
                by_country_province_ordinal =
                    by_country_province_ordinal.inserted((country, province, ordinal), status)?;
            }
        }
        Ok(Self {
            branch: frame.branch,
            supply,
            deployments,
            by_country_province_ordinal,
            supply_complete,
            deployments_complete,
        })
    }
    pub(super) fn changed_provinces(
        &self,
        old: Option<&Self>,
    ) -> io::Result<Option<CopyIndex<u32, ()>>> {
        let Some(old) = old.filter(|old| {
            old.branch == self.branch
                && old.supply_complete == self.supply_complete
                && old.deployments_complete == self.deployments_complete
        }) else {
            return Ok(None);
        };
        let mut changed = CopyIndex::default();
        for &key in self.supply.edits.iter() {
            changed = changed.inserted(key as u32, ())?;
        }
        for &key in self.deployments.edits.iter() {
            for &(province, _) in self
                .deployments
                .values
                .get(&key)
                .into_iter()
                .chain(old.deployments.values.get(&key))
            {
                changed = changed.inserted(province as u32, ())?;
            }
        }
        Ok(Some(changed))
    }
    pub(super) fn deployed(&self, province: u32) -> Option<bool> {
        let Some(&(country, flags)) = self.supply.values.get(&u128::from(province)) else {
            return self.supply_complete.then_some(false);
        };
        if flags == 0 || country == 0 {
            return Some(false);
        }
        if !self.deployments_complete {
            return None;
        }
        let key = (u32::from(country), province as i32, 0);
        Some(
            self.by_country_province_ordinal
                .iter_from(&key)
                .next()
                .filter(|(key, _)| key.0 == u32::from(country) && key.1 == province as i32)
                .is_some_and(|(_, status)| *status != 0),
        )
    }
}

pub(super) struct Observation {
    defines: Defines,
    branch: u64,
    hour: i32,
    ours: u32,
    links: ScalarFold<u32>,
    controllers: [ScalarFold<u32>; 2],
    by_controller_raw: CopyIndex<(u32, u8, u32), ()>,
    ports: ScalarFold<()>,
    completeness: BTreeMap<TableId, (u8, u32, String)>,
    state_buildings: CopyIndex<(u32, u32), SlotRead>,
    province_buildings: CopyIndex<(u32, u32), SlotRead>,
    building_tables: [ViewIdentity; 2],
    building_tokens: [CopyIndex<(u32, u32), ()>; 2],
    building_changes: [Vec<(u32, u32)>; 2],
    targets: Option<TargetFold>,
    ordered: bool,
}
impl Observation {
    pub(super) fn port_membership(&self) -> CopyIndex<u32, ()> {
        self.ports.values.clone()
    }
    #[cfg(test)]
    pub(super) fn read(
        frame: &DecodedFrame,
        defines: &Defines,
        ours: u32,
        previous: Option<&Self>,
    ) -> io::Result<Self> {
        Self::read_input(
            replay_frame::frame::HourInput::new(frame),
            defines,
            ours,
            previous,
        )
    }

    pub(super) fn read_input(
        input: replay_frame::frame::HourInput<'_>,
        defines: &Defines,
        ours: u32,
        previous: Option<&Self>,
    ) -> io::Result<Self> {
        let frame = input.frame();
        let previous = previous.filter(|old| {
            old.defines == *defines && old.ours == ours && old.branch == frame.branch
        });
        let state_rows = rows::<tables::StateBuildings>(frame)?;
        let province_rows = rows::<tables::ProvinceBuildings>(frame)?;
        let operands = [
            (
                TableId::StateBuildings,
                state_rows.rows(),
                previous.map(|old| &old.state_buildings),
            ),
            (
                TableId::ProvinceBuildings,
                province_rows.rows(),
                previous.map(|old| &old.province_buildings),
            ),
        ];
        let links = core::<tables::CountryLinks>(frame)?;
        let mut built = [None, None];
        let mut building_tokens: [CopyIndex<(u32, u32), ()>; 2] =
            previous.map_or_else(Default::default, |old| old.building_tokens.clone());
        let mut ordered = true;
        for (at, (table, rows, old)) in operands.into_iter().enumerate() {
            let view = &frame.tables[table.index()];
            let before = previous.map(|old| &old.building_tables[at]);
            let fields = slot_fields(table);
            if before.is_some_and(|before| before.fields_unchanged(view, fields)) {
                built[at] = Some((old.expect("same observation").clone(), Vec::new()));
                ordered &= previous.is_none_or(|old| old.ordered);
                continue;
            }
            if let replay_frame::frame::TableChanges::Rows(facts) =
                input.changes().since(table, before, fields)
            {
                let mut values = old.expect("predecessor observation").clone();
                let mut changes = Vec::with_capacity(facts.changed().len() + facts.deleted().len());
                for &key in facts.deleted() {
                    let key = ((key >> 32) as u32, key as u32);
                    if values.get(&key).is_some() {
                        values = values.removed(&key)?;
                        building_tokens[at] = building_tokens[at].removed(&(key.1, key.0))?;
                        changes.push(key);
                    }
                }
                for change in facts
                    .changed()
                    .iter()
                    .filter(|row| row.inserted || row.fields & fields != 0)
                {
                    let row_at = rows
                        .binary_search_by_key(&change.key, RowCodec::key)
                        .map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                "building change fact has no current row",
                            )
                        })?;
                    let row = &rows[row_at];
                    let key = row_key(row);
                    let value = slot_read(row, defines);
                    if values.get(&key) != Some(&value) {
                        if values.get(&key).is_none() {
                            building_tokens[at] =
                                building_tokens[at].inserted((key.1, key.0), ())?;
                        }
                        values = values.inserted(key, value)?;
                        changes.push(key);
                    }
                }
                built[at] = Some((values, changes));
                ordered &= previous.is_none_or(|old| old.ordered);
            } else {
                let mut values = CopyIndex::default();
                building_tokens[at] = CopyIndex::default();
                for row in rows {
                    values = values.inserted(row_key(row), slot_read(row, defines))?;
                    building_tokens[at] =
                        building_tokens[at].inserted((row.building_type, row.location), ())?;
                }
                let mut changes = Vec::with_capacity(rows.len() + old.map_or(0, CopyIndex::len));
                if let Some(old) = old {
                    let (mut a, mut b) = (old.iter().peekable(), values.iter().peekable());
                    loop {
                        match (a.peek(), b.peek()) {
                            (Some((x, _)), Some((y, _))) if x < y => {
                                changes.push(*a.next().unwrap().0);
                            }
                            (Some((x, _)), Some((y, _))) if x > y => {
                                changes.push(*b.next().unwrap().0);
                            }
                            (Some(_), Some(_)) => {
                                let (key, value) = a.next().unwrap();
                                if value != b.next().unwrap().1 {
                                    changes.push(*key);
                                }
                            }
                            (Some(_), None) => {
                                changes.push(*a.next().unwrap().0);
                            }
                            (None, Some(_)) => {
                                changes.push(*b.next().unwrap().0);
                            }
                            (None, None) => break,
                        }
                    }
                }
                ordered &= rows
                    .windows(2)
                    .all(|pair| row_key(&pair[0]) < row_key(&pair[1]));
                built[at] = Some((values, changes));
            }
        }
        let (province_buildings, province_changes) = built[1].take().expect("province observation");
        let (state_buildings, state_changes) = built[0].take().expect("state observation");
        let controllers = [
            ScalarFold::read(
                frame,
                TableId::Provinces,
                rows::<tables::Provinces>(frame)?.rows(),
                fields(TableId::Provinces, &["controller"]),
                previous.map(|old| &old.controllers[0]),
                |row| row.controller,
            )?,
            ScalarFold::read(
                frame,
                TableId::States,
                rows::<tables::States>(frame)?.rows(),
                fields(TableId::States, &["controller"]),
                previous.map(|old| &old.controllers[1]),
                |row| row.controller,
            )?,
        ];
        let mut by_controller_raw =
            previous.map_or_else(CopyIndex::default, |old| old.by_controller_raw.clone());
        for (at, rows) in controllers.iter().enumerate() {
            for &location in &rows.changes {
                if let Some(&raw) =
                    previous.and_then(|old| old.controllers[at].values.get(&location))
                {
                    by_controller_raw = by_controller_raw.removed(&(raw, at as u8, location))?;
                }
                if let Some(&raw) = rows.values.get(&location) {
                    by_controller_raw =
                        by_controller_raw.inserted((raw, at as u8, location), ())?;
                }
            }
        }
        Ok(Self {
            defines: *defines,
            branch: frame.branch,
            hour: frame.hour,
            ours,
            links: ScalarFold::read(
                frame,
                TableId::CountryLinks,
                links,
                fields(TableId::CountryLinks, &["canonical_slot"]),
                previous.map(|old| &old.links),
                |row| row.canonical_slot,
            )?,
            ports: ScalarFold::read(
                frame,
                TableId::NavalBases,
                rows::<tables::NavalBases>(frame)?.rows(),
                0,
                previous.map(|old| &old.ports),
                |_| (),
            )?,
            controllers,
            by_controller_raw,
            completeness: COMPLETENESS
                .iter()
                .map(|(table, _)| {
                    let (kind, missing, reason) = completeness(frame, *table);
                    (*table, (kind, missing, reason.to_owned()))
                })
                .collect(),
            state_buildings,
            province_buildings,
            building_tables: [TableId::StateBuildings, TableId::ProvinceBuildings]
                .map(|table| frame.tables[table.index()].identity()),
            building_changes: [state_changes, province_changes],
            building_tokens,
            ordered,
            targets: TargetFold::read(frame, ours, previous.and_then(|old| old.targets.as_ref()))?,
        })
    }
}

impl Detection {
    pub(super) fn observed(observation: &Observation) -> Self {
        Self {
            links: observation.links.values.clone(),
            ours: observation.ours,
            provinces: observation.controllers[0].values.clone(),
            states: observation.controllers[1].values.clone(),
            targets: observation
                .targets
                .as_ref()
                .map(|targets| targets.winners.clone()),
        }
    }
}

pub(super) struct Inputs<'a> {
    pub(super) frame: &'a DecodedFrame,
    pub(super) vision: &'a Vision,
    pub(super) tokens: &'a Retained<TokenTypes>,
    pub(super) city: &'a Retained<Vec<bool>>,
    pub(super) damage_refused: bool,
    pub(super) observation: &'a Observation,
    pub(super) available: &'a super::available::AvailableRows,
    pub(super) fog_changes: Option<&'a CopyIndex<u32, ()>>,
}

impl MapBuildingAssets {
    pub(super) fn sites(&self) -> &Sites {
        self.sites.get_or_init(|| Sites::read(self))
    }

    #[cfg(test)]
    pub(super) fn token_map(&self, frame: &DecodedFrame) -> io::Result<Retained<TokenTypes>> {
        self.token_map_input(replay_frame::frame::HourInput::new(frame))
    }

    pub(super) fn token_map_input(
        &self,
        input: replay_frame::frame::HourInput<'_>,
    ) -> io::Result<Retained<TokenTypes>> {
        let frame = input.frame();
        let view = &frame.tables[TableId::LexicalTokens.index()];
        let mut kept = self
            .tokens
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((identity, types)) = kept.as_ref().filter(|(identity, _)| identity.is(view)) {
            let _ = identity;
            let _ = &**view;
            return Ok(types.clone());
        }
        let types = if let Some((identity, old)) = kept.as_ref() {
            if let replay_frame::frame::TableChanges::Rows(facts) = input.changes().since(
                TableId::LexicalTokens,
                Some(identity),
                TableId::LexicalTokens.full_mask(),
            ) {
                let table = rows::<tables::LexicalTokens>(frame)?;
                let mut next = (**old).clone();
                for key in facts.deleted() {
                    next.remove(&(*key as u32))?;
                }
                for edit in facts.changed() {
                    let token = u32::try_from(edit.key).map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidData, "lexical token key exceeds u32")
                    })?;
                    next.remove(&token)?;
                    let at = table
                        .rows()
                        .binary_search_by_key(&token, |row| row.token)
                        .map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                "token change has no current row",
                            )
                        })?;
                    let names =
                        TokenNames::from_rows(&table.rows()[at..at + 1], table.is_complete());
                    if let Some((_, spelling)) = names.iter().next() {
                        if let Some(value) =
                            named_type(spelling, &self.facts.keys, &self.facts.index)?
                        {
                            next.0 = next.0.inserted(token, value)?;
                        }
                    }
                }
                next
            } else {
                token_types(frame, &self.facts.keys, &self.facts.index)?
            }
        } else {
            token_types(frame, &self.facts.keys, &self.facts.index)?
        };
        let types = Retained::new(types);
        *kept = Some((view.identity(), types.clone()));
        Ok(types)
    }

    pub(super) fn city_bits(&self, city: &CityTerrain<'_>) -> Retained<Vec<bool>> {
        Retained::new(
            self.sites()
                .hub_points
                .iter()
                .map(|&(x, z)| city.at(x, z))
                .collect(),
        )
    }
    pub(super) fn ordinary_slot_count(&self) -> usize {
        self.sites().slots.len()
    }

    pub(super) fn affected(
        &self,
        base: &BuildingsBase,
        hour: &Inputs<'_>,
    ) -> io::Result<Result<Affected, &'static str>> {
        let observed = hour.observation;
        if base.assets != self.id {
            return Ok(Err("the base belongs to other content"));
        }
        if base.observation.branch != hour.frame.branch {
            return Ok(Err("the base is on another branch"));
        }
        if base.observation.hour >= hour.frame.hour {
            return Ok(Err("the base is not an earlier hour"));
        }
        if base.defines != self.defines || !Arc::ptr_eq(&base.facts, &self.facts) {
            return Ok(Err("the building content operands changed"));
        }
        if base.vision.ours != hour.vision.ours() || base.vision.fog != hour.vision.fog() {
            return Ok(Err("the viewing slot or the fog switch changed"));
        }
        if base.vision.visibility_gap || hour.vision.visibility_gap().is_some() {
            return Ok(Err("a fog rebuild gap leaves the fog bytes undecided"));
        }
        for (table, reason) in COMPLETENESS {
            if base.observation.completeness[&table] != observed.completeness[&table] {
                return Ok(Err(reason));
            }
        }
        if base.damage_refused {
            return Ok(Err("the base's city damage was refused"));
        }
        if hour.damage_refused {
            return Ok(Err("this hour's city damage is refused"));
        }
        if !base.observation.ordered || !observed.ordered {
            return Ok(Err("building rows are not in strict key order"));
        }
        let sites = self.sites();
        if base.slots.len() != sites.slots.len() + self.floating.len()
            || base.tallies.len() != sites.slots.len()
            || base.city.len() != sites.hubs.len()
            || hour.city.len() != sites.hubs.len()
        {
            return Ok(Err("the base holds another number of slots"));
        }

        let targets = match (&base.observation.targets, &observed.targets) {
            (Some(_), Some(new)) => new.changes.as_slice(),
            (None, None) => &[],
            _ => return Ok(Err("the RaidTargetStatuses completeness changed")),
        };
        fn add(
            index: &mut CopyIndex<u32, ()>,
            slots: impl IntoIterator<Item = u32>,
        ) -> io::Result<()> {
            for slot in slots {
                *index = index.inserted(slot, ())?;
            }
            Ok(())
        }
        let mut affected = CopyIndex::default();
        let retyped = retyped_tokens(&base.tokens, hour.tokens)?;
        for (at, provincial) in [false, true].into_iter().enumerate() {
            for &(location, token) in &observed.building_changes[at] {
                for index in type_indices(&base.tokens, hour.tokens, token)
                    .into_iter()
                    .flatten()
                {
                    add(
                        &mut affected,
                        Sites::reached(&sites.by_key, (provincial, location, index)),
                    )?;
                }
            }
            for (&token, _) in retyped.iter() {
                let locations = base.observation.building_tokens[at]
                    .iter_from(&(token, 0))
                    .take_while(|(key, _)| key.0 == token)
                    .chain(
                        observed.building_tokens[at]
                            .iter_from(&(token, 0))
                            .take_while(|(key, _)| key.0 == token),
                    );
                for (&(_, location), _) in locations {
                    for index in type_indices(&base.tokens, hour.tokens, token)
                        .into_iter()
                        .flatten()
                    {
                        add(
                            &mut affected,
                            Sites::reached(&sites.by_key, (provincial, location, index)),
                        )?;
                    }
                }
            }
        }
        if hour.fog_changes.is_none() {
            return Ok(Err("the fog byte availability changed"));
        }
        if let Some(changes) = hour.fog_changes {
            for (&province, _) in changes.iter() {
                if base.vision.sees_province_object(province)
                    != hour.vision.sees_province_object(province, false)
                {
                    add(
                        &mut affected,
                        Sites::reached(&sites.by_province, province)
                            .filter(|&slot| read_by(base, Tally::READ_FOG, slot)),
                    )?;
                }
            }
        }
        if !base.city.ptr_eq(hour.city) {
            for (index, &slot) in sites.hubs.iter().enumerate() {
                if base.city[index] != hour.city[index] && read_by(base, Tally::READ_HUB, slot) {
                    affected = affected.inserted(slot, ())?;
                }
            }
        }
        for province in &observed.ports.changes {
            add(
                &mut affected,
                Sites::reached(&sites.by_province, *province)
                    .filter(|&slot| read_by(base, Tally::READ_HUB, slot)),
            )?;
        }
        for &raw in &observed.links.changes {
            for values in [&base.available, hour.available] {
                for (provincial, location, index) in values.country_keys(raw) {
                    add(
                        &mut affected,
                        Sites::reached(&sites.by_key, (provincial, location, index))
                            .filter(|&slot| read_by(base, Tally::READ_DETECTION, slot)),
                    )?;
                }
            }
            for values in [
                &base.observation.by_controller_raw,
                &observed.by_controller_raw,
            ] {
                for (&(_, _, location), _) in values
                    .iter_from(&(raw, 0, 0))
                    .take_while(|(key, _)| key.0 == raw)
                {
                    add(
                        &mut affected,
                        sites
                            .at_location(location)
                            .filter(|&slot| read_by(base, Tally::READ_DETECTION, slot)),
                    )?;
                }
            }
        }
        for location in observed.controllers[0]
            .changes
            .iter()
            .copied()
            .chain(observed.controllers[1].changes.iter().copied())
            .chain(targets.iter().map(|scope| scope.1))
        {
            add(
                &mut affected,
                sites
                    .at_location(location)
                    .filter(|&slot| read_by(base, Tally::READ_DETECTION, slot)),
            )?;
        }
        let slots = Retained::new(affected.iter().map(|(key, _)| *key).collect());
        Ok(Ok(Affected { slots }))
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
    mut evaluate: impl FnMut(u32) -> io::Result<(PreparedSlot, Tally)>,
) -> io::Result<Patch> {
    let mut totals = base.totals.clone();
    let mut slots = Vec::with_capacity(affected.len());
    let mut tallies = Vec::with_capacity(affected.len());
    for &id in affected {
        let (slot, tally) = evaluate(id)?;
        if let Some(old) = base.tallies.get(id as usize) {
            totals.remove(*old, types)?;
        }
        totals.add(tally, types)?;
        slots.push((id, slot));
        tallies.push((id, tally));
    }
    Ok((slots, tallies, totals))
}

pub(super) enum Ordinary<'a> {
    Full {
        slots: ChunkBuilder<PreparedSlot>,
        tallies: ChunkBuilder<Tally>,
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
        harbors: &super::floating::Output,
        ordinary: usize,
    ) -> io::Result<(
        Chunked<PreparedSlot>,
        Chunked<Tally>,
        SlotSummary,
        BuildPath,
    )> {
        match self {
            Self::Full {
                mut slots,
                tallies,
                totals,
                reason,
            } => {
                for slot in harbors.iter().cloned() {
                    slots.push(slot)?;
                }
                Ok((
                    slots.finish()?,
                    tallies.finish()?,
                    totals,
                    BuildPath::Full(reason),
                ))
            }
            Self::Patched {
                base,
                slots,
                tallies,
                totals,
            } => {
                let evaluated = slots.len();
                let count = slots
                    .len()
                    .checked_add(harbors.changes.len())
                    .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?;
                let mut combined = Vec::with_capacity(count);
                combined.extend(slots);
                combined.extend(harbors.changes.iter().cloned());
                let patched = base
                    .slots
                    .patched(&combined, base.slots.len(), &[], PreparedSlot::same_bits)?
                    .zip(
                        base.tallies
                            .patched(&tallies, ordinary, &[], |a, b| a == b)?,
                    );
                let (slots, tallies) =
                    patched.expect("the plan checked the base's slot and tally counts");
                Ok((
                    slots,
                    tallies,
                    totals,
                    BuildPath::Patched { slots: evaluated },
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests;
