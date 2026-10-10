//! CNavalUnitTransfer resolved from combat participant handles; no event hook is installed.

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
}

#[derive(Clone, Copy)]
struct Speed {
    owner: u32,
    region: usize,
    value: i64,
}

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
    } else if let Some(observation) = state.observation(input) {
        return observation.result.map(T::from_owned);
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
    } else if state.observations.len() < state.observations.capacity() {
        let index = state.observations.len();
        state.observations.push(observation);
        state.index_observation(input, index * 2 + 1);
    }
    result
}

/// Successful transport observations in one frozen acquisition. Enemy gates
/// and reciprocal intel depend on the owner, not the queried region. Preserve
/// their original interleaving with regional reads on the first query; only a
/// complete successful roster may serve a later region. Every clear expires
/// the owned inputs, including current naval array geometry and speed results.
pub(crate) struct MovementSpeeds {
    key: Option<(usize, usize)>,
    speeds: Vec<Option<Speed>>,
    owners: Vec<OwnerEnemies>,
    enemies: Vec<EnemyWeight>,
    navies: Vec<Option<NavyInputs>>,
    shared: [Option<Result<InputValue, &'static str>>; SHARED_INPUTS],
    observations: Vec<Observation>,
    failures: Vec<Observation>,
    observation_index: Vec<usize>,
    retain_failures: bool,
}

impl MovementSpeeds {
    pub fn vacant() -> Self {
        Self {
            key: None,
            speeds: Vec::new(),
            owners: Vec::new(),
            enemies: Vec::new(),
            navies: Vec::new(),
            shared: [None; SHARED_INPUTS],
            observations: Vec::new(),
            failures: Vec::new(),
            observation_index: Vec::new(),
            retain_failures: false,
        }
    }

    pub fn try_with_capacity(rows: usize) -> std::io::Result<Self> {
        let slots = if rows == 0 {
            0
        } else {
            rows.checked_next_power_of_two()
                .ok_or_else(crate::allocation::oom)?
        };
        let speed_slots = slots.checked_mul(2).ok_or_else(crate::allocation::oom)?;
        let enemy_slots = 0;
        Ok(Self {
            key: None,
            speeds: crate::allocation::filled(speed_slots, None)?,
            owners: crate::allocation::vec(slots)?,
            enemies: crate::allocation::vec(enemy_slots)?,
            navies: crate::allocation::filled(slots, None)?,
            shared: [None; SHARED_INPUTS],
            observations: crate::allocation::vec(enemy_slots)?,
            failures: crate::allocation::vec(speed_slots)?,
            observation_index: crate::allocation::filled(
                observation_slots(enemy_slots, speed_slots)?,
                0,
            )?,
            retain_failures: false,
        })
    }
    #[cfg(test)]
    pub fn new(rows: usize) -> Self {
        Self::try_with_capacity(rows).expect("movement speed test capacity")
    }
    pub fn clear(&mut self) {
        self.key = None;
        self.speeds.fill(None);
        self.owners.clear();
        self.enemies.clear();
        self.navies.fill(None);
        self.shared.fill(None);
        self.observations.clear();
        self.failures.clear();
        self.observation_index.fill(0);
    }
    pub fn reserve(&mut self, rows: usize) -> std::io::Result<()> {
        if self.owners.capacity() < rows {
            *self = Self::try_with_capacity(rows)?;
        }
        Ok(())
    }
    pub fn allocated_bytes(&self) -> usize {
        self.speeds.capacity() * size_of::<Option<Speed>>()
            + self.owners.capacity() * size_of::<OwnerEnemies>()
            + self.enemies.capacity() * size_of::<EnemyWeight>()
            + self.navies.capacity() * size_of::<Option<NavyInputs>>()
            + (self.observations.capacity() + self.failures.capacity()) * size_of::<Observation>()
            + self.observation_index.capacity() * size_of::<usize>()
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
        let mut hash = std::collections::hash_map::DefaultHasher::new();
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
    pub fn get(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        owner: u32,
        region: usize,
    ) -> Result<i64, &'static str> {
        if self.key.is_some_and(|key| key != (base, state)) {
            self.clear();
        }
        self.key = Some((base, state));
        let mut slot = None;
        if let Some(mask) = self.speeds.len().checked_sub(1) {
            let hash = (region
                ^ (usize::try_from(owner).unwrap_or(0)).wrapping_mul(0x9e37_79b9_7f4a_7c15))
            .wrapping_mul(0xbf58_476d_1ce4_e5b9);
            for probe in 0..self.speeds.len() {
                let index = ((hash ^ (hash >> 32)).wrapping_add(probe)) & mask;
                match self.speeds[index] {
                    Some(value) if (value.owner, value.region) == (owner, region) => {
                        return Ok(value.value);
                    }
                    None => {
                        slot = Some(index);
                        break;
                    }
                    _ => {}
                }
            }
        }
        let enemy_start = self.enemies.len();
        let owner_start = self.owners.len();
        let result = match movement_speed_shared(memory, base, state, owner, region, Some(self)) {
            Ok(result) => result,
            Err(reason) => {
                // A failed query may have staged an incomplete enemy roster.
                // Never retain that prefix as a successful observation.
                self.enemies.truncate(enemy_start);
                self.owners.truncate(owner_start);
                return Err(reason);
            }
        };
        if let Some(slot) = slot {
            self.speeds[slot] = Some(Speed {
                owner,
                region,
                value: result,
            });
        }
        Ok(result)
    }

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
        if self.key.is_some_and(|key| key != (base, state)) {
            self.clear();
        }
        self.key = Some((base, state));
        observe(&mut Some(&mut *self), Input::RegionKey(region, id), |_| {
            let observed = cstrategic_region::id(memory, base, region)?;
            (observed == Some(id))
                .then_some(i64::from(id))
                .ok_or("country movement source-region key mismatch")
        })?;
        self.get(memory, base, state, owner, region)
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
        let regions = match found {
            Some(entry) => entry.regions,
            None => {
                let mut memo = Some(&mut *self);
                let navy = selected_navy(memory, base, state, country, countries, &mut memo)?;
                observe(&mut memo, Input::NavyRegions(country), |_| {
                    cstrategic_navy::regional_array(memory, navy)
                })?
            }
        };
        let at = regions
            .element(region, 8)
            .ok_or("navy regional operand outside array")?;
        let value = observe(
            &mut Some(&mut *self),
            Input::RegionalOperand(country, region),
            |_| read::<i64>(memory, at).ok_or("navy regional operand unreadable"),
        )?;
        if let Some(slot) = slot {
            self.navies[slot] = Some(NavyInputs { country, regions });
        }
        Ok(value)
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
        && let Some(owner) = memo
            .owners
            .iter()
            .find(|entry| same_country(entry.owner, owner))
            .copied()
    {
        let mut total = 0_i64;
        let mut weighted = 0_i64;
        for index in owner.start..owner.start + owner.len {
            let enemy = memo.enemies[index];
            let operand =
                memo.regional_operand(memory, base, state, enemy.country, region, countries)?;
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
        .filter(|memo| memo.retain_failures || !memo.speeds.is_empty())
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
                memo.enemies.push(EnemyWeight { country, weight });
            } else {
                complete = false;
            }
        }
    }
    if let Some(memo) = memo {
        if complete && memo.owners.len() < memo.owners.capacity() {
            memo.owners.push(OwnerEnemies {
                owner,
                start,
                len: memo.enemies.len() - start,
            });
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
mod tests;
