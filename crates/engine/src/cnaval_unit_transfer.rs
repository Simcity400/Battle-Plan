//! CNavalUnitTransfer resolved from combat participant handles; no event hook
//! is installed.

/// CNavalUnitTransfer vtable.
pub const VTABLE_RVA: usize = 0x0295_6cf8;
/// CNavalUnitTransfer packed reference handle u64.
pub const HANDLE: usize = 0x08;
/// Signed combat-reference count. Nonzero selects combat action nine
/// and prevents transfer; save and load retain the vector. Omission means
/// zero count.
pub const ACTION_STATUS: usize = 0x24;
/// CNavalUnitTransfer unit-handle array.
pub const UNITS: usize = 0x38;
/// CNavalUnitTransfer unit capacity u32.
pub const UNITS_CAPACITY: usize = 0x40;
/// CNavalUnitTransfer unit count u32.
pub const UNITS_COUNT: usize = 0x44;
/// CNavalUnitTransfer target province id u32.
pub const TARGET_PROVINCE: usize = 0x50;
/// Source province id u32.
pub const SOURCE_PROVINCE: usize = 0x54;
/// CNavalUnitTransfer constructor.
pub const CONSTRUCTOR_RVA: usize = 0x00e1_0de0;

use crate::{
    Memory, ccountry, ccountry_intel, cdiplomacy_status, cgame_state, country_intel_row,
    cpdx_array::Array, cstatic_modifier, cstrategic_navy, cstrategic_navy_manager,
    cstrategic_region, cweather_manager, modifier_accumulator, modifier_definition, read,
    sweather_per_region,
};
use std::hash::{Hash, Hasher};

/// Base transport speed i64.
pub const BASE_SPEED_RVA: usize = 0x0331_8a78;
/// Static modifier-set singleton.
const MODIFIER_SET_RVA: usize = 0x0331_7cc8;
/// Transport-penalty static-modifier pointer, gated before composition.
const TRANSPORT_MODIFIER: usize = 0xf8;
/// Maximum regional operand total i64.
const MAXIMUM_TOTAL_RVA: usize = 0x0331_e100;
/// Minimum intel advantage subtracted before surplus reduction.
const MINIMUM_INTEL_RVA: usize = 0x0331_d5b0;
/// Maximum intel advantage i64.
const MAXIMUM_INTEL_RVA: usize = 0x0331_d620;
/// Intel-advantage reduction scale; its product rounds before range division.
const INTEL_REDUCTION_RVA: usize = 0x0331_e550;
/// Naval transport speed token.
const SPEED_TOKEN: u32 = crate::modifier_keys::NAVAL_SPEED_FACTOR;
/// Country transport reduction token.
const REDUCTION_TOKEN: u32 = crate::modifier_keys::NAVAL_MINES_EFFECT_REDUCTION;
const P: i64 = 100_000;

#[derive(Clone, Copy)]
struct EnemyWeight {
    country: ccountry::Country,
    weight: i64,
    /// This enemy navy's bound-domain cells, once copied.
    operands: Option<usize>,
}

#[derive(Clone, Copy)]
struct OwnerEnemies {
    owner: ccountry::Country,
    start: usize,
    len: usize,
}

#[derive(Clone, Copy)]
struct NavyInputs {
    country: ccountry::Country,
    regions: Array,
    /// Start of this navy's cells for the bound source-region domain.
    operands: Option<usize>,
}

/// One enemy navy's regional access operand for one bound domain region.
#[derive(Clone, Copy)]
enum OperandCell {
    /// Not copied yet: the region's own element is read on first use.
    Unread,
    Value(i64),
    Unreadable,
}

/// Elements per guarded copy of a navy's consecutive region ids.
const OPERAND_BATCH: usize = 64;
const OPERAND_UNREADABLE: &str = "navy regional operand unreadable";

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum SharedInput {
    Countries,
    Regions,
    BaseSpeed,
    Weather,
    StaticDefinition,
    Navies,
    Maximum,
    MinimumIntel,
    MaximumIntel,
    IntelReduction,
    StaticValue,
    Category,
    Metadata,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Input {
    Shared(SharedInput),
    Country(u32),
    Region(usize),
    RegionKey(usize, u32),
    Weather(u32),
    Navy(ccountry::Country),
    NavyRegions(ccountry::Country),
    RegionalOperand(ccountry::Country, u32),
    Intel(ccountry::Country),
    Diplomacy(ccountry::Country),
    Enemies(ccountry::Country),
    Enemy(ccountry::Country, u32),
    Fog(ccountry::Country),
    Capitulated(ccountry::Country),
    Weight(ccountry::Country, ccountry::Country),
    Reduction(ccountry::Country),
}

impl Hash for Input {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        let country = |country: &ccountry::Country, state: &mut H| {
            (country.pointer, country.id, country.canonical_slot).hash(state);
        };
        match self {
            Self::Shared(input) => input.hash(state),
            Self::Country(raw) | Self::Weather(raw) => raw.hash(state),
            Self::Region(pointer) => pointer.hash(state),
            Self::RegionKey(pointer, id) => (pointer, id).hash(state),
            Self::Navy(value)
            | Self::NavyRegions(value)
            | Self::Intel(value)
            | Self::Diplomacy(value)
            | Self::Enemies(value)
            | Self::Fog(value)
            | Self::Capitulated(value)
            | Self::Reduction(value) => country(value, state),
            Self::RegionalOperand(value, region) | Self::Enemy(value, region) => {
                country(value, state);
                region.hash(state);
            }
            Self::Weight(owner, enemy) => {
                country(owner, state);
                country(enemy, state);
            }
        }
    }
}

/// Pass-local observation index hash. Probes compare complete keys, so the
/// hash only selects where a probe starts; it needs speed and spread, not
/// SipHash's flooding resistance.
#[derive(Default)]
struct ObservationHasher(u64);

impl Hasher for ObservationHasher {
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.write_u64(u64::from_le_bytes(word));
        }
    }
    fn write_u32(&mut self, value: u32) {
        self.write_u64(u64::from(value));
    }
    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(26) ^ value).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }
    fn finish(&self) -> u64 {
        // SplitMix64 finalizer: masked low bits depend on every input word.
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }
}

fn observation_slots(observations: usize, failures: usize) -> std::io::Result<usize> {
    let count = observations
        .checked_add(failures)
        .and_then(|count| count.checked_mul(2))
        .ok_or_else(crate::allocation::oom)?;
    if count == 0 {
        Ok(0)
    } else {
        count
            .checked_next_power_of_two()
            .ok_or_else(crate::allocation::oom)
    }
}

#[derive(Clone, Copy)]
enum InputValue {
    Arrays([Array; 2]),
    Array(Array),
    Country(ccountry::Country),
    Scalar(i64),
    Pointer(usize),
    Modifier(modifier_accumulator::Value),
    Metadata(modifier_definition::Operands),
}

trait ObservedValue: Copy {
    fn owned(self) -> InputValue;
    fn from_owned(value: InputValue) -> Self;
}

macro_rules! observed_value {
    ($($type:ty => $variant:ident),+ $(,)?) => {$ (
        impl ObservedValue for $type {
            fn owned(self) -> InputValue { InputValue::$variant(self) }
            fn from_owned(value: InputValue) -> Self {
                let InputValue::$variant(value) = value else {
                    unreachable!("transport input kind differs from its selecting key")
                };
                value
            }
        }
    )+};
}

observed_value! {
    [Array; 2] => Arrays,
    Array => Array,
    ccountry::Country => Country,
    i64 => Scalar,
    usize => Pointer,
    modifier_accumulator::Value => Modifier,
    modifier_definition::Operands => Metadata,
}

#[derive(Clone, Copy)]
struct Observation {
    input: Input,
    result: Result<InputValue, &'static str>,
}

const SHARED_INPUTS: usize = SharedInput::Metadata as usize + 1;
pub(crate) const REFUSAL_CAPACITY: &str = "transport refusal arena capacity exhausted";

fn observe<T: ObservedValue>(
    memo: &mut Option<&mut MovementSpeeds>,
    input: Input,
    acquire: impl FnOnce(&mut Option<&mut MovementSpeeds>) -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    let Some(state) = memo.as_deref() else {
        return acquire(memo);
    };
    if let Input::Shared(input) = input {
        if let Some(result) = state.shared[input as usize] {
            return result.map(T::from_owned);
        }
    } else if let Some(value) = state.recent(input) {
        return Ok(T::from_owned(value));
    } else if let Some(observation) = state.observation(input) {
        let result = observation.result;
        if let Ok(value) = result {
            memo.as_deref_mut()
                .expect("transport observation memo retained")
                .remember(input, value);
        }
        return result.map(T::from_owned);
    } else if state.retain_failures && state.failures.len() == state.failures.capacity() {
        // Each entered query can introduce at most one refused input. Reserve
        // that live query demand before acquisition, including cache saturation.
        return Err(REFUSAL_CAPACITY);
    }
    let failed = state.failures.len();
    let result = acquire(memo);
    let state = memo
        .as_deref_mut()
        .expect("transport observation memo retained");
    let observation = Observation {
        input,
        result: result.map(ObservedValue::owned),
    };
    if let Input::Shared(input) = input {
        state.shared[input as usize] = Some(observation.result);
    } else if result.is_err() {
        // Nested controls own their failure. Keep the selecting result too
        // only if another query slot remains; it can otherwise replay that
        // original nested refusal without another native attempt.
        if state.failures.len() == failed && state.failures.len() < state.failures.capacity() {
            let index = state.failures.len();
            state.failures.push(observation);
            state.index_observation(input, index * 2 + 2);
        }
    } else {
        if let Ok(value) = observation.result {
            state.remember(input, value);
        }
        if state.observations.len() < state.observations.capacity() {
            let index = state.observations.len();
            state.observations.push(observation);
            state.index_observation(input, index * 2 + 1);
        }
    }
    result
}

/// Successful per-owner inputs of the most recent query. A domain walk asks
/// every region for one owner in turn.
#[derive(Clone, Copy, Default)]
struct RecentOwner {
    country: Option<(u32, ccountry::Country)>,
    navy: Option<(ccountry::Country, usize)>,
    reduction: Option<(ccountry::Country, i64)>,
}

/// Successful per-region inputs at one bound domain position.
#[derive(Clone, Copy, Default)]
struct RegionCell {
    key: Option<(usize, u32)>,
    region: Option<(usize, i64)>,
    weather: Option<(u32, i64)>,
}

/// Successful transport observations in one frozen acquisition. Enemy gates
/// and reciprocal intel depend on the owner, not the queried region. Preserve
/// their original interleaving with regional reads on the first query; only a
/// complete successful roster may serve a later region. Every clear expires
/// the owned inputs, including current naval array geometry.
pub(crate) struct MovementSpeeds {
    key: Option<(usize, usize)>,
    owners: Vec<OwnerEnemies>,
    /// `owners` position + 1 by masked canonical slot; 0 was never pushed.
    owner_index: Vec<usize>,
    enemies: Vec<EnemyWeight>,
    navies: Vec<Option<NavyInputs>>,
    shared: [Option<Result<InputValue, &'static str>>; SHARED_INPUTS],
    observations: Vec<Observation>,
    failures: Vec<Observation>,
    observation_index: Vec<usize>,
    retain_failures: bool,
    /// The caller's complete source-region key domain, in its key order, and
    /// the position of the region being queried (`get_in_domain`).
    domain: Vec<u32>,
    position: Option<usize>,
    /// One cell per bound domain region for each enemy navy reached.
    operands: Vec<OperandCell>,
    /// Successful observations repeated by consecutive or same-position
    /// queries, served before the hashed index with identical values.
    recent: RecentOwner,
    regions: Vec<RegionCell>,
}

impl MovementSpeeds {
    pub fn vacant() -> Self {
        Self {
            key: None,
            owners: Vec::new(),
            owner_index: Vec::new(),
            enemies: Vec::new(),
            navies: Vec::new(),
            shared: [None; SHARED_INPUTS],
            observations: Vec::new(),
            failures: Vec::new(),
            observation_index: Vec::new(),
            retain_failures: false,
            domain: Vec::new(),
            position: None,
            operands: Vec::new(),
            recent: RecentOwner::default(),
            regions: Vec::new(),
        }
    }

    pub fn try_with_capacity(rows: usize) -> std::io::Result<Self> {
        let slots = if rows == 0 {
            0
        } else {
            rows.checked_next_power_of_two()
                .ok_or_else(crate::allocation::oom)?
        };
        let query_slots = slots.checked_mul(2).ok_or_else(crate::allocation::oom)?;
        let enemy_slots = 0;
        Ok(Self {
            key: None,
            owners: crate::allocation::vec(slots)?,
            owner_index: crate::allocation::filled(slots, 0)?,
            enemies: crate::allocation::vec(enemy_slots)?,
            navies: crate::allocation::filled(slots, None)?,
            shared: [None; SHARED_INPUTS],
            observations: crate::allocation::vec(enemy_slots)?,
            failures: crate::allocation::vec(query_slots)?,
            observation_index: crate::allocation::filled(
                observation_slots(enemy_slots, query_slots)?,
                0,
            )?,
            retain_failures: false,
            domain: Vec::new(),
            position: None,
            operands: Vec::new(),
            recent: RecentOwner::default(),
            regions: Vec::new(),
        })
    }
    #[cfg(test)]
    pub fn new(rows: usize) -> Self {
        Self::try_with_capacity(rows).expect("movement speed test capacity")
    }
    pub fn clear(&mut self) {
        self.key = None;
        self.owners.clear();
        self.owner_index.fill(0);
        self.enemies.clear();
        self.navies.fill(None);
        self.shared.fill(None);
        self.observations.clear();
        self.failures.clear();
        self.observation_index.fill(0);
        self.domain.clear();
        self.position = None;
        self.operands.clear();
        self.recent = RecentOwner::default();
        self.regions.clear();
    }
    pub fn reserve(&mut self, rows: usize) -> std::io::Result<()> {
        if self.owners.capacity() < rows {
            *self = Self::try_with_capacity(rows)?;
        }
        Ok(())
    }
    pub fn allocated_bytes(&self) -> usize {
        self.owners.capacity() * size_of::<OwnerEnemies>()
            + self.owner_index.capacity() * size_of::<usize>()
            + self.enemies.capacity() * size_of::<EnemyWeight>()
            + self.navies.capacity() * size_of::<Option<NavyInputs>>()
            + (self.observations.capacity() + self.failures.capacity()) * size_of::<Observation>()
            + self.observation_index.capacity() * size_of::<usize>()
            + self.domain.capacity() * size_of::<u32>()
            + self.operands.capacity() * size_of::<OperandCell>()
            + self.regions.capacity() * size_of::<RegionCell>()
    }

    pub fn reserve_queries(&mut self, queries: usize, regions: usize) -> std::io::Result<()> {
        let observations = queries
            .checked_add(regions)
            .ok_or_else(crate::allocation::oom)?;
        let failures = queries.saturating_sub(self.failures.len());
        let observation_demand = observations;
        let observations = observations.saturating_sub(self.observations.len());
        crate::allocation::reserve(&mut self.failures, failures)?;
        crate::allocation::reserve(&mut self.observations, observations)?;
        let domain = regions.saturating_sub(self.domain.len());
        crate::allocation::reserve(&mut self.domain, domain)?;
        let cells = regions.saturating_sub(self.regions.len());
        crate::allocation::reserve(&mut self.regions, cells)?;
        let slots = observation_slots(
            observation_demand.max(self.observations.len()),
            queries.max(self.failures.len()),
        )?;
        if self.observation_index.len() < slots {
            self.observation_index = crate::allocation::filled(slots, 0)?;
            for index in 0..self.observations.len() {
                self.index_observation(self.observations[index].input, index * 2 + 1);
            }
            for index in 0..self.failures.len() {
                self.index_observation(self.failures[index].input, index * 2 + 2);
            }
        }
        self.retain_failures = true;
        Ok(())
    }

    fn observation_slot(&self, input: Input) -> Option<usize> {
        let mask = self.observation_index.len().checked_sub(1)?;
        let mut hash = ObservationHasher::default();
        input.hash(&mut hash);
        let mut slot = hash.finish() as usize & mask;
        loop {
            let index = self.observation_index[slot];
            if index == 0 || self.indexed_observation(index).input == input {
                return Some(slot);
            }
            slot = slot.wrapping_add(1) & mask;
        }
    }

    fn indexed_observation(&self, index: usize) -> &Observation {
        if index.is_multiple_of(2) {
            &self.failures[index / 2 - 1]
        } else {
            &self.observations[index / 2]
        }
    }

    fn observation(&self, input: Input) -> Option<&Observation> {
        let index = self.observation_index[self.observation_slot(input)?];
        (index != 0).then(|| self.indexed_observation(index))
    }

    fn index_observation(&mut self, input: Input, index: usize) {
        let slot = self
            .observation_slot(input)
            .expect("prepared transport observation index");
        self.observation_index[slot] = index;
    }
    fn select_pass(&mut self, base: usize, state: usize) {
        if self.key.is_some_and(|key| key != (base, state)) {
            self.clear();
        }
        self.key = Some((base, state));
    }

    fn query(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        owner: u32,
        region: usize,
    ) -> Result<i64, &'static str> {
        let enemy_start = self.enemies.len();
        let owner_start = self.owners.len();
        let result = movement_speed_shared(memory, base, state, owner, region, Some(self));
        if result.is_err() {
            // A failed query may have staged an incomplete enemy roster.
            // Never retain that prefix as a successful observation.
            self.enemies.truncate(enemy_start);
            self.owners.truncate(owner_start);
        }
        result
    }

    fn region_key(
        &mut self,
        memory: &impl Memory,
        base: usize,
        region: usize,
        id: u32,
    ) -> Result<(), &'static str> {
        observe(&mut Some(&mut *self), Input::RegionKey(region, id), |_| {
            let observed = cstrategic_region::id(memory, base, region)?;
            (observed == Some(id))
                .then_some(i64::from(id))
                .ok_or("country movement source-region key mismatch")
        })
        .map(|_| ())
    }

    #[cfg(test)]
    pub fn get_for_region(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        owner: u32,
        region: usize,
        id: u32,
    ) -> Result<i64, &'static str> {
        self.retain_failures = true;
        self.select_pass(base, state);
        self.position = None;
        self.region_key(memory, base, region, id)?;
        self.query(memory, base, state, owner, region)
    }

    /// Binds the caller's complete source-region key domain for this pass.
    /// `get_in_domain` then serves each enemy navy's regional access operands from one
    /// copy of its consecutive region ids. False keeps per-operand reads.
    pub fn bind_domain(
        &mut self,
        base: usize,
        state: usize,
        ids: impl ExactSizeIterator<Item = u32> + Clone,
    ) -> bool {
        self.select_pass(base, state);
        if self.domain.len() == ids.len() && self.domain.iter().copied().eq(ids.clone()) {
            return true;
        }
        self.domain.clear();
        self.operands.clear();
        self.regions.clear();
        for navy in self.navies.iter_mut().flatten() {
            navy.operands = None;
        }
        for enemy in &mut self.enemies {
            enemy.operands = None;
        }
        if ids.len() > self.domain.capacity() || ids.len() > self.regions.capacity() {
            return false;
        }
        self.regions.resize(ids.len(), RegionCell::default());
        self.domain.extend(ids);
        true
    }

    /// Transport speed for one owner and one region of the bound domain. A
    /// caller queries each (owner, region) key once per pass, so no speed
    /// result is retained; every native input keeps its first observation.
    #[allow(clippy::too_many_arguments)]
    pub fn get_in_domain(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        owner: u32,
        region: usize,
        id: u32,
        position: usize,
    ) -> Result<i64, &'static str> {
        self.retain_failures = true;
        self.select_pass(base, state);
        self.position = (self.domain.get(position) == Some(&id)).then_some(position);
        let result = self
            .region_key(memory, base, region, id)
            .and_then(|()| self.query(memory, base, state, owner, region));
        self.position = None;
        result
    }

    fn recent(&self, input: Input) -> Option<InputValue> {
        let cell = || self.regions.get(self.position?);
        match input {
            Input::Country(raw) => self
                .recent
                .country
                .filter(|(key, _)| *key == raw)
                .map(|(_, country)| InputValue::Country(country)),
            Input::Navy(country) => self
                .recent
                .navy
                .filter(|(key, _)| same_country(*key, country))
                .map(|(_, navy)| InputValue::Pointer(navy)),
            Input::Reduction(country) => self
                .recent
                .reduction
                .filter(|(key, _)| same_country(*key, country))
                .map(|(_, value)| InputValue::Scalar(value)),
            Input::RegionKey(pointer, id) => cell()?
                .key
                .filter(|key| *key == (pointer, id))
                .map(|_| InputValue::Scalar(i64::from(id))),
            Input::Region(pointer) => cell()?
                .region
                .filter(|(key, _)| *key == pointer)
                .map(|(_, value)| InputValue::Scalar(value)),
            Input::Weather(id) => cell()?
                .weather
                .filter(|(key, _)| *key == id)
                .map(|(_, value)| InputValue::Scalar(value)),
            _ => None,
        }
    }

    fn remember(&mut self, input: Input, value: InputValue) {
        let cell = self
            .position
            .and_then(|position| self.regions.get_mut(position));
        match (input, value) {
            (Input::Country(raw), InputValue::Country(country)) => {
                self.recent.country = Some((raw, country));
            }
            (Input::Navy(country), InputValue::Pointer(navy)) => {
                self.recent.navy = Some((country, navy));
            }
            (Input::Reduction(country), InputValue::Scalar(value)) => {
                self.recent.reduction = Some((country, value));
            }
            (Input::RegionKey(pointer, id), _) => {
                if let Some(cell) = cell {
                    cell.key = Some((pointer, id));
                }
            }
            (Input::Region(pointer), InputValue::Scalar(value)) => {
                if let Some(cell) = cell {
                    cell.region = Some((pointer, value));
                }
            }
            (Input::Weather(id), InputValue::Scalar(value)) => {
                if let Some(cell) = cell {
                    cell.weather = Some((id, value));
                }
            }
            _ => {}
        }
    }

    /// The bound domain position of the queried region, if any.
    fn bound_position(&self, region: u32) -> Option<usize> {
        self.position
            .filter(|&position| self.domain.get(position) == Some(&region))
    }

    /// A navy's copied cells for the bound domain (own unmasked slot only).
    fn navy_operands(&self, country: ccountry::Country) -> Option<usize> {
        self.navies
            .get(country.canonical_slot as usize)
            .copied()
            .flatten()
            .filter(|entry| same_country(entry.country, country))
            .and_then(|entry| entry.operands)
    }

    fn owner_roster(&self, owner: ccountry::Country) -> Option<OwnerEnemies> {
        let mask = self.owner_index.len().checked_sub(1)?;
        let index = self.owner_index[owner.canonical_slot as usize & mask];
        if index == 0 {
            return None;
        }
        match self.owners.get(index - 1) {
            Some(entry) if same_country(entry.owner, owner) => Some(*entry),
            // A shared masked slot or a truncated failed roster.
            _ => self
                .owners
                .iter()
                .find(|entry| same_country(entry.owner, owner))
                .copied(),
        }
    }

    fn push_owner(&mut self, entry: OwnerEnemies) {
        self.owners.push(entry);
        if let Some(mask) = self.owner_index.len().checked_sub(1) {
            self.owner_index[entry.owner.canonical_slot as usize & mask] = self.owners.len();
        }
    }

    fn regional_operand<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        state: usize,
        country: ccountry::Country,
        region: u32,
        countries: &ccountry::Reader<'_, M>,
    ) -> Result<i64, &'static str> {
        let slot = self
            .navies
            .len()
            .checked_sub(1)
            .map(|mask| country.canonical_slot as usize & mask);
        let found = slot
            .and_then(|i| self.navies[i])
            .filter(|entry| same_country(entry.country, country));
        let mut entry = match found {
            Some(entry) => entry,
            None => {
                let mut memo = Some(&mut *self);
                let navy = selected_navy(memory, base, state, country, countries, &mut memo)?;
                let regions = observe(&mut memo, Input::NavyRegions(country), |_| {
                    cstrategic_navy::regional_array(memory, navy)
                })?;
                NavyInputs {
                    country,
                    regions,
                    operands: None,
                }
            }
        };
        // Regional access lookup indexes the navy's +C8 array by region id.
        let at = entry
            .regions
            .element(region, 8)
            .ok_or("navy regional operand outside array")?;
        // Domain cells need this navy's own unmasked slot, so a navy keeps one
        // span per bound domain.
        let position = self
            .bound_position(region)
            .filter(|_| (country.canonical_slot as usize) < self.navies.len());
        if let Some(position) = position {
            if entry.operands.is_none() {
                entry.operands = self.copy_operands(memory, entry.regions);
            }
            if let Some(start) = entry.operands {
                let value = self.operand_cell(memory, start + position, at);
                self.navies[country.canonical_slot as usize] = Some(entry);
                return value;
            }
        }
        let value = observe(
            &mut Some(&mut *self),
            Input::RegionalOperand(country, region),
            |_| read::<i64>(memory, at).ok_or(OPERAND_UNREADABLE),
        )?;
        if let Some(slot) = slot {
            self.navies[slot] = Some(entry);
        }
        Ok(value)
    }

    /// Copies one enemy navy's elements for every bound domain region inside
    /// its array, a run of consecutive ids per guarded copy. A failed run
    /// leaves its cells unread, so each region later reads only its own
    /// element. None (optional storage unavailable) keeps per-operand reads.
    fn copy_operands(&mut self, memory: &impl Memory, regions: Array) -> Option<usize> {
        let len = self.domain.len();
        let start = self.operands.len();
        crate::allocation::advisory(|| crate::allocation::reserve(&mut self.operands, len)).ok()?;
        self.operands.resize(start + len, OperandCell::Unread);
        let mut first = 0;
        while first < len {
            let Some(at) = regions.element(self.domain[first], 8) else {
                // Outside the native array: refused before any cell is read.
                first += 1;
                continue;
            };
            let mut end = first + 1;
            while end < len
                && end - first < OPERAND_BATCH
                && self.domain[end] == self.domain[end - 1].wrapping_add(1)
                && self.domain[end] < regions.count
            {
                end += 1;
            }
            let mut bytes = [0; OPERAND_BATCH * 8];
            let bytes = &mut bytes[..(end - first) * 8];
            if crate::memory::read_slice(memory, at, bytes) {
                for (cell, value) in self.operands[start + first..start + end]
                    .iter_mut()
                    .zip(bytes.chunks_exact(8))
                {
                    *cell = OperandCell::Value(i64::from_le_bytes(
                        value.try_into().expect("eight-byte operand"),
                    ));
                }
            }
            first = end;
        }
        Some(start)
    }

    fn operand_cell(
        &mut self,
        memory: &impl Memory,
        index: usize,
        at: usize,
    ) -> Result<i64, &'static str> {
        match self.operands[index] {
            OperandCell::Value(value) => Ok(value),
            OperandCell::Unreadable => Err(OPERAND_UNREADABLE),
            OperandCell::Unread => {
                // Only this region's own element, attempted once per pass.
                let value = read::<i64>(memory, at);
                self.operands[index] = value.map_or(OperandCell::Unreadable, OperandCell::Value);
                value.ok_or(OPERAND_UNREADABLE)
            }
        }
    }
}

fn same_country(a: ccountry::Country, b: ccountry::Country) -> bool {
    (a.pointer, a.id, a.canonical_slot) == (b.pointer, b.id, b.canonical_slot)
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("transport address overflow")
}
fn value<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or("transport input unreadable")
}
fn pointer(memory: &impl Memory, parent: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(value::<u64>(memory, parent, offset)?)
        .ok()
        .filter(|&p| p != 0)
        .ok_or("transport pointer null or invalid")
}
fn multiply(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / P
}
fn fraction(numerator: i64, denominator: i64) -> Result<i64, &'static str> {
    if denominator == 0
        || (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd
    {
        return Ok(0xffff_ffff);
    }
    numerator
        .wrapping_mul(P)
        .checked_div(denominator)
        .ok_or("transport native division overflows")
}

fn diplomacy(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
) -> Result<usize, &'static str> {
    let p = pointer(memory, country.pointer, ccountry::DIPLOMACY_STATUS)?;
    if value::<u64>(memory, p, 0)? != at(base, cdiplomacy_status::VTABLE_RVA)? as u64
        || value::<u64>(memory, p, cdiplomacy_status::COUNTRY)? != country.pointer as u64
    {
        return Err("transport diplomacy identity mismatch");
    }
    Ok(p)
}

fn intel(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
) -> Result<usize, &'static str> {
    let p = pointer(memory, country.pointer, ccountry::INTEL_TABLE)?;
    if value::<u64>(memory, p, 0)? != at(base, ccountry_intel::VTABLE_RVA)? as u64 {
        return Err("transport intel identity mismatch");
    }
    Ok(p)
}

/// Third reciprocal-difference lane. A missing target lane contributes
/// zero; unreadable or out-of-bounds positive country references fail.
fn intel_lane(
    memory: &impl Memory,
    countries: &ccountry::Reader<'_, impl Memory>,
    source: usize,
    target: usize,
    target_country: ccountry::Country,
) -> Result<i64, &'static str> {
    let raw = value::<i32>(memory, target, ccountry_intel::OWNER)?;
    if raw <= 0 {
        return Ok(0);
    }
    let resolved = countries.resolve(raw as u32)?;
    if resolved.pointer != target_country.pointer {
        return Err("transport intel owner mismatch");
    }
    let root = at(source, ccountry_intel::DATA)?;
    let array = Array::read(memory, root, country_intel_row::STRIDE)
        .ok_or("transport intel array invalid")?;
    let result = match array.element(resolved.canonical_slot, country_intel_row::STRIDE) {
        Some(row) => value::<i64>(memory, row, country_intel_row::NAVY)?,
        None => 0,
    };
    Ok(result)
}

fn selected_navy<M: Memory>(
    memory: &M,
    base: usize,
    state: usize,
    country: ccountry::Country,
    countries: &ccountry::Reader<'_, M>,
    memo: &mut Option<&mut MovementSpeeds>,
) -> Result<usize, &'static str> {
    observe(memo, Input::Navy(country), |memo| {
        // Select navies by canonical country from the shared manager.
        let navies = observe(memo, Input::Shared(SharedInput::Navies), |_| {
            cstrategic_navy_manager::navies(memory, base, state)
        })?;
        let slot = navies
            .element(country.canonical_slot, 8)
            .ok_or("navy manager country outside array")?;
        let navy = read::<u64>(memory, slot)
            .filter(|&p| p != 0)
            .ok_or("country navy absent or unreadable")? as usize;
        // Resolve the owner backlink after DLC/static-definition gates.
        cstrategic_navy::authenticate_owner_with_countries(
            memory, base, state, navy, country, countries,
        )?;
        Ok(navy)
    })
}

fn totals<M: Memory>(
    memory: &M,
    base: usize,
    state: usize,
    countries: &ccountry::Reader<'_, M>,
    owner: ccountry::Country,
    region: u32,
    mut memo: Option<&mut MovementSpeeds>,
) -> Result<(i64, i64), &'static str> {
    if let Some(memo) = memo.as_deref_mut()
        && let Some(owner) = memo.owner_roster(owner)
    {
        let mut total = 0_i64;
        let mut weighted = 0_i64;
        let position = memo.bound_position(region);
        for index in owner.start..owner.start + owner.len {
            let enemy = memo.enemies[index];
            // A copied in-array value; every other cell takes the full path.
            let copied = position.zip(enemy.operands).and_then(|(position, start)| {
                match memo.operands[start + position] {
                    OperandCell::Value(value) => Some(value),
                    _ => None,
                }
            });
            let operand = match copied {
                Some(value) => value,
                None => {
                    let value = memo.regional_operand(
                        memory,
                        base,
                        state,
                        enemy.country,
                        region,
                        countries,
                    )?;
                    if position.is_some() {
                        memo.enemies[index].operands = memo.navy_operands(enemy.country);
                    }
                    value
                }
            };
            total = total.wrapping_add(operand);
            weighted = weighted.wrapping_add(multiply(operand, enemy.weight));
        }
        return Ok((
            total,
            if total == 0 {
                0
            } else {
                fraction(weighted, total)?
            },
        ));
    }
    // Acquire the owner's intel object before visiting enemies.
    let owner_intel = observe(&mut memo, Input::Intel(owner), |_| {
        intel(memory, base, owner)
    })?;
    let relations = observe(&mut memo, Input::Diplomacy(owner), |_| {
        diplomacy(memory, base, owner)
    })?;
    let root = at(relations, cdiplomacy_status::ENEMY_COUNTRIES)?;
    let array = observe(&mut memo, Input::Enemies(owner), |_| {
        Array::read(memory, root, 4).ok_or("transport enemy roster invalid")
    })?;
    let mut total = 0_i64;
    let mut weighted = 0_i64;
    let start = memo.as_ref().map_or(0, |memo| memo.enemies.len());
    let mut complete = true;
    if let Some(memo) = memo
        .as_deref_mut()
        .filter(|memo| memo.retain_failures || !memo.owner_index.is_empty())
    {
        // Consume the original owner's enemy vector. Reserve reached
        // dependencies before payloads without a per-country ratio.
        crate::allocation::reserve(&mut memo.enemies, array.count as usize)
            .map_err(|_| "transport enemy cache allocation failed")?;
        crate::allocation::reserve(&mut memo.owners, 1)
            .map_err(|_| "transport owner cache allocation failed")?;
    }
    for index in 0..array.count {
        let country = observe(&mut memo, Input::Enemy(owner, index), |_| {
            let raw = read::<u32>(
                memory,
                array
                    .element(index, 4)
                    .ok_or("transport enemy index overflow")?,
            )
            .ok_or("transport enemy reference unreadable")?;
            countries.resolve(raw)
        })?;
        if observe(&mut memo, Input::Fog(country), |_| {
            value::<i32>(memory, country.pointer, ccountry::FOG_NETWORK_TABLE_COUNT).map(i64::from)
        })? <= 0
        {
            continue;
        }
        let enemy_relations = observe(&mut memo, Input::Diplomacy(country), |_| {
            diplomacy(memory, base, country)
        })?;
        if observe(&mut memo, Input::Capitulated(country), |_| {
            value::<u8>(memory, enemy_relations, cdiplomacy_status::CAPITULATED).map(i64::from)
        })? != 0
        {
            continue;
        }
        let enemy_intel = observe(&mut memo, Input::Intel(country), |_| {
            intel(memory, base, country)
        })?;
        let weight = observe(&mut memo, Input::Weight(owner, country), |_| {
            Ok(
                intel_lane(memory, countries, owner_intel, enemy_intel, country)?.wrapping_sub(
                    intel_lane(memory, countries, enemy_intel, owner_intel, owner)?,
                ),
            )
        })?;
        let operand = match memo.as_deref_mut() {
            Some(memo) => memo.regional_operand(memory, base, state, country, region, countries)?,
            None => {
                let navy = selected_navy(memory, base, state, country, countries, &mut None)?;
                let regions = cstrategic_navy::regional_array(memory, navy)?;
                let at = regions
                    .element(region, 8)
                    .ok_or("navy regional operand outside array")?;
                read::<i64>(memory, at).ok_or("navy regional operand unreadable")?
            }
        };
        total = total.wrapping_add(operand);
        weighted = weighted.wrapping_add(multiply(operand, weight));
        if let Some(memo) = memo.as_deref_mut() {
            if memo.enemies.len() < memo.enemies.capacity() {
                memo.enemies.push(EnemyWeight {
                    country,
                    weight,
                    operands: None,
                });
            } else {
                complete = false;
            }
        }
    }
    if let Some(memo) = memo {
        if complete && memo.owners.len() < memo.owners.capacity() {
            let len = memo.enemies.len() - start;
            memo.push_owner(OwnerEnemies { owner, start, len });
        } else {
            memo.enemies.truncate(start);
        }
    }
    Ok((
        total,
        if total == 0 {
            0
        } else {
            fraction(weighted, total)?
        },
    ))
}

fn reduction_fraction(
    total: i64,
    mean: i64,
    maximum: i64,
    country_reduction: i64,
    minimum_intel: i64,
    maximum_intel: i64,
    intel_reduction: i64,
) -> Result<i64, &'static str> {
    // Ordered native clamp also preserves the behavior of a negative define.
    let bounded = if total < 0 {
        0
    } else if total > maximum {
        maximum
    } else {
        total
    };
    let mut result = fraction(bounded, maximum)?;
    if result != 0 {
        let mut reduction = country_reduction;
        let surplus = mean.wrapping_sub(minimum_intel);
        if surplus > 0 {
            let range = maximum_intel.wrapping_sub(minimum_intel);
            let scaled = multiply(surplus.min(range), intel_reduction);
            reduction = reduction.wrapping_add(if range == 0 {
                0
            } else {
                fraction(scaled, range)?
            });
        }
        if reduction != 0 {
            result = multiply(P.wrapping_sub(reduction), result).clamp(0, P);
        }
    }
    Ok(result)
}

fn composed_modifier<M: Memory>(
    memory: &M,
    base: usize,
    state: usize,
    owner: ccountry::Country,
    countries: &ccountry::Reader<'_, M>,
    region: u32,
    mut memo: Option<&mut MovementSpeeds>,
) -> Result<i64, &'static str> {
    let definition = observe(
        &mut memo,
        Input::Shared(SharedInput::StaticDefinition),
        |_| {
            let flags = value::<u32>(memory, base, modifier_definition::TRADE_FEATURE_FLAGS_RVA)?;
            // Feature 0x13 tests exactly DLC bit 4.
            if flags & (1 << 4) == 0 {
                return Ok(0_usize);
            }
            let definitions = pointer(memory, base, MODIFIER_SET_RVA)?;
            let definition = value::<u64>(memory, definitions, TRANSPORT_MODIFIER)?;
            if definition == 0 {
                return Ok(0);
            }
            let definition =
                usize::try_from(definition).map_err(|_| "transport modifier pointer invalid")?;
            if value::<u64>(memory, definition, 0)?
                != at(base, cstatic_modifier::VTABLE_RVA)? as u64
            {
                return Err("transport modifier identity mismatch");
            }
            Ok(definition)
        },
    )?;
    if definition == 0 {
        return Ok(0);
    }
    // Resolve the owner's navy and +0x10 backlink after DLC/static-definition
    // gates, even with no enemies. The owner's regional operand is not reached.
    selected_navy(memory, base, state, owner, countries, &mut memo)?;
    let (total, mean) = totals(
        memory,
        base,
        state,
        countries,
        owner,
        region,
        memo.as_deref_mut(),
    )?;
    let maximum = observe(&mut memo, Input::Shared(SharedInput::Maximum), |_| {
        value::<i64>(memory, base, MAXIMUM_TOTAL_RVA)
    })?;
    let bounded = if total < 0 {
        0
    } else if total > maximum {
        maximum
    } else {
        total
    };
    let first_fraction = fraction(bounded, maximum)?;
    let reduction = if first_fraction != 0 {
        observe(&mut memo, Input::Reduction(owner), |_| {
            Ok(modifier_accumulator::lookup_effective_at(
                memory,
                base,
                at(owner.pointer, ccountry::MODIFIER_ACCUMULATOR)?,
                REDUCTION_TOKEN,
            )?
            .effective)
        })?
    } else {
        0
    };
    // The intel defines participate only after the initial nonzero fraction.
    let (minimum, upper, scale) = if first_fraction != 0 {
        let minimum = observe(&mut memo, Input::Shared(SharedInput::MinimumIntel), |_| {
            value::<i64>(memory, base, MINIMUM_INTEL_RVA)
        })?;
        if mean.wrapping_sub(minimum) > 0 {
            (
                minimum,
                observe(&mut memo, Input::Shared(SharedInput::MaximumIntel), |_| {
                    value::<i64>(memory, base, MAXIMUM_INTEL_RVA)
                })?,
                observe(
                    &mut memo,
                    Input::Shared(SharedInput::IntelReduction),
                    |_| value::<i64>(memory, base, INTEL_REDUCTION_RVA),
                )?,
            )
        } else {
            (minimum, minimum, 0)
        }
    } else {
        (0, 0, 0)
    };
    let fraction = reduction_fraction(total, mean, maximum, reduction, minimum, upper, scale)?;
    let result = if fraction > 0 {
        let raw = observe(&mut memo, Input::Shared(SharedInput::StaticValue), |_| {
            modifier_accumulator::lookup_at(
                memory,
                at(definition, cstatic_modifier::ACCUMULATOR)?,
                SPEED_TOKEN,
            )
        })?;
        if raw.present {
            let category = observe(&mut memo, Input::Shared(SharedInput::Category), |_| {
                let metadata = pointer(memory, base, modifier_definition::TABLE_RVA)?;
                let category_at = at(
                    at(metadata, SPEED_TOKEN as usize * modifier_definition::STRIDE)?,
                    modifier_definition::CATEGORY_MASK,
                )?;
                read::<u32>(memory, category_at)
                    .map(i64::from)
                    .ok_or("transport modifier category unreadable")
            })?;
            // Both category-filter stages use mask 0xffffffff, initialized by the
            // temporary accumulator constructor.
            if category == 0 {
                0
            } else {
                let scaled = multiply(raw.raw, fraction);
                observe(&mut memo, Input::Shared(SharedInput::Metadata), |_| {
                    modifier_definition::operands(memory, base, SPEED_TOKEN)
                })?
                .effective(scaled)
            }
        } else {
            0
        }
    } else {
        0
    };
    Ok(result)
}

fn weather_modifier(
    memory: &impl Memory,
    base: usize,
    state: usize,
    region: u32,
    memo: &mut Option<&mut MovementSpeeds>,
) -> Result<i64, &'static str> {
    let array = observe(memo, Input::Shared(SharedInput::Weather), |_| {
        let manager = pointer(memory, state, cgame_state::WEATHER_MANAGER)?;
        if value::<u64>(memory, manager, 0)? != at(base, cweather_manager::VTABLE_RVA)? as u64 {
            return Err("transport weather manager identity mismatch");
        }
        let root = at(manager, cweather_manager::REGIONS)?;
        Array::read(memory, root, sweather_per_region::STRIDE)
            .ok_or("transport weather array invalid")
    })?;
    observe(memo, Input::Weather(region), |_| {
        let result = if let Some(row) = array.element(region, sweather_per_region::STRIDE) {
            if value::<u64>(memory, row, 0)? != at(base, sweather_per_region::VTABLE_RVA)? as u64
                || value::<u32>(memory, row, sweather_per_region::ID)? != region
            {
                return Err("transport weather region identity mismatch");
            }
            modifier_accumulator::lookup_effective_at(
                memory,
                base,
                at(row, sweather_per_region::MODIFIERS)?,
                SPEED_TOKEN,
            )?
            .effective
        } else {
            0
        }; // Out-of-range regions yield no weather modifier.
        Ok(result)
    })
}

/// Passive transport-speed operands for a country and strategic region.
/// Return raw fixed5 speed before the caller multiplier. Only current
/// callback inputs are read: no getter calls, allocation or cache refresh.
/// Optional task-force and diagnostic contexts remain null.
pub fn movement_speed(
    memory: &impl Memory,
    base: usize,
    state: usize,
    owner: u32,
    region: usize,
) -> Result<i64, &'static str> {
    movement_speed_shared(memory, base, state, owner, region, None)
}

fn movement_speed_shared(
    memory: &impl Memory,
    base: usize,
    state: usize,
    owner: u32,
    region: usize,
    mut memo: Option<&mut MovementSpeeds>,
) -> Result<i64, &'static str> {
    let arrays = observe(&mut memo, Input::Shared(SharedInput::Countries), |_| {
        ccountry::Reader::new(memory, base, state).map(|countries| countries.arrays())
    })?;
    let countries = ccountry::Reader::from_arrays(memory, base, state, arrays)?;
    let country = observe(&mut memo, Input::Country(owner), |_| {
        countries.resolve(owner)
    })?;
    let region_id = observe(&mut memo, Input::Region(region), |memo| {
        let region_id = value::<u32>(memory, region, cstrategic_region::ID)?;
        let slot = value::<u32>(memory, region, cstrategic_region::RUNTIME_SLOT)?;
        let root = at(state, cgame_state::STRATEGIC_REGIONS)?;
        let regions = observe(memo, Input::Shared(SharedInput::Regions), |_| {
            Array::read(memory, root, 8).ok_or("transport region census invalid")
        })?;
        let entry = regions
            .element(slot, 8)
            .ok_or("transport region slot outside census")?;
        let vtable = at(base, cstrategic_region::VTABLE_RVA)? as u64;
        if region == 0
            || region_id > i32::MAX as u32
            || read::<u64>(memory, entry) != Some(region as u64)
            || read::<u64>(memory, region) != Some(vtable)
        {
            return Err("transport region identity mismatch");
        }
        Ok(i64::from(region_id))
    })? as u32;
    // Read base speed before both additive factors.
    let speed = observe(&mut memo, Input::Shared(SharedInput::BaseSpeed), |_| {
        value::<i64>(memory, base, BASE_SPEED_RVA)
    })?;
    let weather = weather_modifier(memory, base, state, region_id, &mut memo)?;
    let composed = composed_modifier(memory, base, state, country, &countries, region_id, memo)?;
    Ok(multiply(
        P.wrapping_add(weather).wrapping_add(composed),
        speed,
    ))
}

#[cfg(test)]
mod scale_tests;
#[cfg(test)]
mod tests;
