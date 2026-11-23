
use crate::{
    Memory,
    air_fuel_defines as defines,
    air_mission,
    cace,
    cair_base,
    cair_wing,
    ccountry,
    cequipment_module,
    cequipment_variant::{self, pool as cequipment_variant_pool},
    cgame_state,
    cpdx_array::Array,
    cprovince,
    creference_registry::ScopedResolver,
    cship,
    cstrategic_air,
    cstrategic_region,
    csub_unit_definition,
    cweather_manager::{self, province_weather as sweather_per_province},
    memory::Raw,
    modifier_accumulator,
    read,
    read_slice,
    walk::Buffer,
};
use engine_sites::Rva;

fn ace_hash(reference: u64) -> usize {
    reference.wrapping_mul(0x9e37_79b9) as usize
}

fn forecast_hash(region: usize) -> usize {
    (region >> 4).wrapping_mul(0x9e37_79b9)
}

fn grow_memo<T: Default + Copy>(
    slots: &mut Buffer<'_, T>,
    required: usize,
    key: impl Fn(T) -> Option<usize>,
) -> Result<(), &'static str> {
    if !slots.is_growing() || required <= slots.len() {
        return Ok(());
    }
    let previous =
        crate::allocation::copy_vec(slots).map_err(|_| "capture buffer allocation failed")?;
    slots.ensure(required)?;
    slots.fill(T::default());
    for item in previous {
        let Some(hash) = key(item) else { continue };
        let mut index = hash % slots.len();
        while key(slots[index]).is_some() {
            index = (index + 1) % slots.len();
        }
        slots[index] = item;
    }
    Ok(())
}

const P: i64 = defines::FIXED_SCALE;
const SENTINEL: i64 = u32::MAX as i64;
const FUEL_KEY: u32 = 0x19e;
const ACE_KEY: u32 = crate::modifier_keys::ACE_EFFECTIVENESS_FACTOR;
const MAXIMUM_MISSIONS: usize = 18;
const MISSION_PLANES: usize = 0x170;
const SUBTYPE_CAPACITIES: usize = 0x600;
const SHIP_CARRIER_CACHE: usize = 0x801;
const MODULE_CARRIER_FLAG: usize = 0x2d0;
const FORECAST_COEFFICIENTS: usize = 0x350;
const FORECAST_HOURS: usize = 24;
const FORECAST_VECTOR_BYTES: usize = FORECAST_HOURS * 24;

type Value = Result<i64, &'static str>;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Packet {
    pub current: Option<i64>,
    pub maximum: Option<i64>,
    pub current_failure: Option<&'static str>,
    pub maximum_failure: Option<&'static str>,
}

#[derive(Clone, Copy, Debug)]
pub struct PoolInput {
    pub pointer: usize,
    pub owner: usize,
    pub base_reference: u64,
    pub definition: usize,
    pub wing_count: u32,
}

pub struct PoolRead {
    input: PoolInput,
    seen: u32,
    current: Value,
    maximum: Value,
    capacity: Option<Value>,
}

impl PoolRead {
    fn begin(input: PoolInput) -> Self {
        Self {
            input,
            seen: 0,
            current: Ok(0),
            maximum: Ok(0),
            capacity: None,
        }
    }

    fn refuse(&mut self, reason: &'static str) {
        if self.current.is_ok() {
            self.current = Err(reason);
        }
        if self.maximum.is_ok() {
            self.maximum = Err(reason);
        }
    }

    pub fn finish(mut self) -> Packet {
        if self.seen != self.input.wing_count {
            self.refuse("carrier fuel membership incomplete");
        }
        Packet {
            current: self.current.ok(),
            maximum: self.maximum.ok(),
            current_failure: self.current.err(),
            maximum_failure: self.maximum.err(),
        }
    }
}

#[derive(Default, Clone, Copy)]
pub struct CountryTerms {
    pointer: usize,
    fuel: Option<Value>,
    ace: Option<Value>,
}

#[derive(Default, Clone, Copy)]
pub struct AceSlot {
    reference: u64,
    value: Option<Value>,
}

#[derive(Default, Clone, Copy)]
pub struct ForecastSlot {
    region: usize,
    value: Option<Result<u8, &'static str>>,
}

#[derive(Clone, Copy)]
struct ForecastSource {
    clocks: Array,
    provinces: Array,
    coefficients: [i64; 3],
    subtract: i64,
    scale: i64,
}

pub struct Reader<'a, M: Memory> {
    memory: &'a M,
    base: usize,
    game_state: usize,
    countries: &'a ccountry::Reader<'a, M>,
    country_terms: Buffer<'a, CountryTerms>,
    aces: Buffer<'a, AceSlot>,
    forecasts: Buffer<'a, ForecastSlot>,
    wing_demand: usize,
    references: ScopedResolver,
    costs: Option<Result<Array, &'static str>>,
    maximum_cost: Option<Value>,
    weight: Option<Value>,
    thresholds: Option<Result<Array, &'static str>>,
    night_threshold: Option<Value>,
    ace_count: Option<Value>,
    ace_cap: Option<Value>,
    forecast_source: Option<Result<ForecastSource, &'static str>>,
    regions: Option<Result<Array, &'static str>>,
}

impl<'a, M: Memory> Reader<'a, M> {
    #[cfg(test)]
    pub fn new(
        memory: &'a M,
        base: usize,
        game_state: usize,
        countries: &'a ccountry::Reader<'a, M>,
        country_terms: &'a mut Vec<CountryTerms>,
        aces: &'a mut Vec<AceSlot>,
        forecasts: &'a mut Vec<ForecastSlot>,
    ) -> Self {
        Self::new_buffers(
            memory,
            base,
            game_state,
            countries,
            Buffer::Growing(country_terms),
            Buffer::Growing(aces),
            Buffer::Growing(forecasts),
        )
    }

    pub(crate) fn new_buffers(
        memory: &'a M,
        base: usize,
        game_state: usize,
        countries: &'a ccountry::Reader<'a, M>,
        mut country_terms: Buffer<'a, CountryTerms>,
        mut aces: Buffer<'a, AceSlot>,
        mut forecasts: Buffer<'a, ForecastSlot>,
    ) -> Self {
        country_terms.fill(CountryTerms::default());
        aces.fill(AceSlot::default());
        forecasts.fill(ForecastSlot::default());
        Self {
            memory,
            base,
            game_state,
            countries,
            country_terms,
            aces,
            forecasts,
            wing_demand: 0,
            references: ScopedResolver::new(base),
            costs: None,
            maximum_cost: None,
            weight: None,
            thresholds: None,
            night_threshold: None,
            ace_count: None,
            ace_cap: None,
            forecast_source: None,
            regions: None,
        }
    }

    pub fn begin_pool(&mut self, input: PoolInput) -> PoolRead {
        let mut pool = PoolRead::begin(input);
        if input.pointer == 0
            || input.owner == 0
            || input.definition == 0
            || input.wing_count > i32::MAX as u32
        {
            pool.refuse("carrier fuel pool certificate invalid");
            return pool;
        }
        if let Err(reason) = self.reserve_wings(input.wing_count as usize) {
            pool.refuse(reason);
            return pool;
        }
        if input.wing_count != 0
            && let Err(reason) = self.maximum_cost()
        {
            pool.maximum = Err(reason);
        }
        pool
    }

    fn reserve_wings(&mut self, count: usize) -> Result<(), &'static str> {
        self.wing_demand = self
            .wing_demand
            .checked_add(count)
            .ok_or("carrier fuel memo demand overflow")?;
        let required = self
            .wing_demand
            .checked_mul(2)
            .ok_or("carrier fuel memo demand overflow")?;
        grow_memo(&mut self.aces, required, |slot| {
            slot.value.map(|_| ace_hash(slot.reference))
        })?;
        grow_memo(&mut self.forecasts, required, |slot| {
            slot.value.map(|_| forecast_hash(slot.region))
        })?;
        Ok(())
    }

    pub fn wing(&mut self, pool: &mut PoolRead, wing: usize) {
        let _ = self.wing_observation(pool, wing, false);
    }

    pub fn wing_with_planes(
        &mut self,
        pool: &mut PoolRead,
        wing: usize,
    ) -> Result<i32, &'static str> {
        self.wing_observation(pool, wing, true)
            .ok_or("carrier original wing planes observation unreached")?
    }

    fn wing_observation(
        &mut self,
        pool: &mut PoolRead,
        wing: usize,
        demand_planes: bool,
    ) -> Option<Result<i32, &'static str>> {
        let Some(seen) = pool.seen.checked_add(1) else {
            pool.refuse("carrier fuel wing count overflow");
            return demand_planes.then_some(Err("carrier fuel wing count overflow"));
        };
        pool.seen = seen;
        if seen > pool.input.wing_count || wing == 0 {
            pool.refuse("carrier fuel unexpected wing occurrence");
            return demand_planes.then_some(Err("carrier fuel unexpected wing occurrence"));
        }
        let mut planes = None;
        let current = if pool.current.is_ok() {
            self.current_cost(pool, wing, &mut planes)
        } else {
            Ok(None)
        };
        let needs_current = current.as_ref().is_ok_and(Option::is_some);
        let modified = if pool.maximum.is_ok() || needs_current {
            self.modified_fuel(pool.input, wing)
        } else {
            Ok(0)
        };
        if let Ok(sum) = pool.maximum {
            let term = (|| {
                let modified = modified?;
                let planes =
                    *planes.get_or_insert_with(|| field(self.memory, wing, cair_wing::PLANE_COUNT));
                Ok(maximum_term(
                    planes?,
                    modified,
                    self.weight()?,
                    self.maximum_cost()?,
                ))
            })();
            pool.maximum = term.map(|term| sum.wrapping_add(term));
        }
        if let Ok(sum) = pool.current {
            let term = (|| match current? {
                None => Ok(0),
                Some(cost) => {
                    let amount = current_term(modified?, self.weight()?, cost);
                    let mode: u8 = field(
                        self.memory,
                        wing,
                        cair_wing::MISSION + air_mission::CONTEXT_MODE,
                    )?;
                    let region =
                        pointer(self.memory, wing, cair_wing::MISSION + air_mission::REGION)?;
                    if region == 0 || mode == 2 {
                        Ok(amount)
                    } else {
                        Ok(period_term(amount, mode, self.forecast(region)?))
                    }
                }
            })();
            pool.current = term.map(|term| sum.wrapping_add(term));
        }
        if demand_planes {
            Some(*planes.get_or_insert_with(|| field(self.memory, wing, cair_wing::PLANE_COUNT)))
        } else {
            planes
        }
    }

    fn current_cost(
        &mut self,
        pool: &mut PoolRead,
        wing: usize,
        planes: &mut Option<Result<i32, &'static str>>,
    ) -> Result<Option<i64>, &'static str> {
        object(
            self.memory,
            self.base,
            wing.checked_add(cair_wing::MISSION)
                .ok_or("carrier mission address overflow")?,
            air_mission::VTABLE_RVA,
        )?;
        let current: i32 = field(
            self.memory,
            wing,
            cair_wing::MISSION + air_mission::CURRENT_TYPE,
        )?;
        let (count, index) = if current != 0 {
            if current < 0 || !(current as u32).is_power_of_two() {
                return Err("carrier current mission is not one positive bit");
            }
            (
                field::<i32>(self.memory, wing, MISSION_PLANES)?,
                (current as u32).trailing_zeros() as usize,
            )
        } else {
            let mask: u32 = field(
                self.memory,
                wing,
                cair_wing::MISSION + air_mission::ENABLED_MASK,
            )?;
            if mask & air_mission::TRAINING_MASK == 0 {
                return Ok(None);
            }
            if field::<u8>(self.memory, wing, cair_wing::ELIGIBILITY_VETO)? != 0 {
                return Ok(None);
            }
            let counter: i32 = field(self.memory, wing, cair_wing::ELIGIBILITY_COUNTER)?;
            if counter > field::<i32>(self.memory, wing, cair_wing::ELIGIBILITY_LIMIT)? {
                return Ok(None);
            }
            let base_reference = field(self.memory, wing, cair_wing::BASE_OVERRIDE)?;
            if self
                .references
                .resolve(self.memory, base_reference)?
                .is_some()
            {
                return Ok(None);
            }
            let count =
                *planes.get_or_insert_with(|| field(self.memory, wing, cair_wing::PLANE_COUNT));
            let count = count?;
            if count <= 0 {
                return Ok(None);
            }
            if pointer(self.memory, wing, cair_wing::MISSION + air_mission::WING)? != wing {
                return Err("carrier training mission wing backlink mismatch");
            }
            let class: i32 = field(
                self.memory,
                wing,
                cair_wing::MISSION + air_mission::CAPACITY_CLASS,
            )?;
            let threshold = table_value(self.memory, self.thresholds()?, class)?;
            let capacity = *pool
                .capacity
                .get_or_insert_with(|| self.capacity(pool.input));
            if !capacity_gate(count, capacity? as i32, threshold) {
                return Ok(None);
            }
            (count, defines::TRAINING_COST_INDEX)
        };
        let cost = table_value(
            self.memory,
            self.costs()?,
            i32::try_from(index).map_err(|_| "carrier mission index overflow")?,
        )?;
        Ok(Some(i64::from(count).wrapping_mul(cost)))
    }

    fn modified_fuel(&mut self, pool: PoolInput, wing: usize) -> Value {
        let fuel: i64 = field(self.memory, wing, cair_wing::BASE_FUEL)?;
        let reference = field(self.memory, wing, cair_wing::ACE_REFERENCE)?;
        let ace = self.ace(reference)?;
        let raw: i32 = field(self.memory, wing, cair_wing::COUNTRY_OVERRIDE)?;
        let country = if raw > 0 {
            self.countries.resolve(raw as u32)?
        } else {
            let raw: u32 = field(self.memory, pool.owner, cstrategic_air::COUNTRY)?;
            if raw == 0 {
                self.countries.reserved()?
            } else {
                self.countries.resolve(raw)?
            }
        };
        Ok(modified_base(fuel, ace, self.country_term(country, false)?))
    }

    fn country_term(&mut self, country: ccountry::Country, ace: bool) -> Value {
        self.country_terms
            .ensure(country.canonical_slot as usize + 1)?;
        let slot = self
            .country_terms
            .get_mut(country.canonical_slot as usize)
            .ok_or("carrier fuel country memo capacity exceeded")?;
        if slot.pointer != 0 && slot.pointer != country.pointer {
            return Err("carrier fuel country identity changed in pass");
        }
        slot.pointer = country.pointer;
        let term = if ace { &mut slot.ace } else { &mut slot.fuel };
        *term.get_or_insert_with(|| {
            let address = country
                .pointer
                .checked_add(ccountry::MODIFIER_ACCUMULATOR)
                .ok_or("carrier country modifier address overflow")?;
            modifier_accumulator::requested_effective_at(
                self.memory,
                self.base,
                address,
                if ace { ACE_KEY } else { FUEL_KEY },
            )
        })
    }

    fn ace(&mut self, reference: u64) -> Value {
        if reference == 0 {
            return Ok(0);
        }
        let start = ace_hash(reference) % self.aces.len().max(1);
        for offset in 0..self.aces.len() {
            let index = (start + offset) % self.aces.len();
            let slot = self.aces[index];
            if let Some(value) = slot.value
                && slot.reference == reference
            {
                return value;
            }
            if slot.value.is_none() {
                let value = self.read_ace(reference);
                self.aces[index] = AceSlot {
                    reference,
                    value: Some(value),
                };
                return value;
            }
        }
        Err("carrier fuel ace memo capacity exceeded")
    }

    fn read_ace(&mut self, reference: u64) -> Value {
        let Some(ace) = self.references.resolve(self.memory, reference)? else {
            return Ok(0);
        };
        object(self.memory, self.base, ace, cace::VTABLE_RVA)?;
        if field::<u64>(self.memory, ace, cace::REFERENCE)? != reference {
            return Err("carrier ace full reference mismatch");
        }
        if field::<u8>(self.memory, ace, cace::ALIVE)? == 0 {
            return Ok(0);
        }
        let accumulator = ace
            .checked_add(cace::MODIFIERS)
            .ok_or("carrier ace modifier address overflow")?;
        let raw = modifier_accumulator::requested_effective_at(
            self.memory,
            self.base,
            accumulator,
            FUEL_KEY,
        )?;
        let assigned = field(self.memory, ace, cace::ASSIGNED_WING_REFERENCE)?;
        let count = if let Some(secondary) = self.references.resolve(self.memory, assigned)? {
            if secondary == cair_wing::SECONDARY {
                None
            } else {
                let wing = secondary
                    .checked_sub(cair_wing::SECONDARY)
                    .ok_or("carrier ace wing address invalid")?;
                if assigned as u32 != cair_wing::TYPE {
                    return Err("carrier ace assigned wing reference type mismatch");
                }
                object(
                    self.memory,
                    self.base,
                    secondary,
                    cair_wing::SECONDARY_VTABLE_RVA,
                )?;
                object(self.memory, self.base, wing, cair_wing::VTABLE_RVA)?;
                if field::<u64>(self.memory, wing, cair_wing::REFERENCE_TYPE)? != assigned {
                    return Err("carrier ace assigned wing full reference mismatch");
                }
                Some(field::<i32>(self.memory, wing, cair_wing::PLANE_COUNT)?)
            }
        } else {
            None
        };
        let count_factor = if let Some(count) = count.filter(|count| *count != 0) {
            let cap = cached_scalar(
                self.memory,
                self.base,
                defines::ACE_CAP_RVA,
                &mut self.ace_cap,
            )?;
            let numerator = cached_scalar(
                self.memory,
                self.base,
                defines::ACE_COUNT_RVA,
                &mut self.ace_count,
            )?;
            ace_count_factor(count, numerator, cap)
        } else {
            P
        };
        let owner = pointer(self.memory, ace, cace::OWNER_COUNTRY)?;
        let raw_owner: u32 = field(self.memory, owner, ccountry::ID)?;
        let country = if raw_owner == 0 {
            let country = self.countries.reserved()?;
            if country.pointer != owner {
                return Err("carrier ace reserved owner mismatch");
            }
            country
        } else {
            self.countries.of_pointer(owner)?
        };
        let effectiveness = self.country_term(country, true)?;
        Ok(ace_term(raw, count_factor, effectiveness))
    }

    fn capacity(&mut self, pool: PoolInput) -> Value {
        let base = self
            .references
            .resolve(self.memory, pool.base_reference)?
            .ok_or("carrier training base unresolved")?;
        object(self.memory, self.base, base, cair_base::VTABLE_RVA)?;
        if field::<u64>(self.memory, base, cair_base::HANDLE)? != pool.base_reference {
            return Err("carrier training base full reference mismatch");
        }
        let kind = if pointer(self.memory, base, cair_base::STATE)? != 0 {
            0
        } else {
            let reference = field(self.memory, base, cair_base::HOST_REFERENCE)?;
            let host = self
                .references
                .resolve(self.memory, reference)?
                .ok_or("carrier training host unresolved")?;
            object(self.memory, self.base, host, cship::VTABLE_RVA)?;
            if field::<u64>(self.memory, host, cship::HANDLE)? != reference {
                return Err("carrier training host full reference mismatch");
            }
            if field::<u8>(self.memory, host, cship::FORMATION_FLAGS)? & 0x80 != 0 {
                3
            } else {
                let valid: u8 = field(self.memory, host, SHIP_CARRIER_CACHE + 1)?;
                let special = if valid != 0 {
                    field::<u8>(self.memory, host, SHIP_CARRIER_CACHE)?
                } else {
                    u8::from(self.host_has_carrier_module(host)?)
                };
                if special > 1 {
                    return Err("carrier training host capacity type unsupported");
                }
                usize::from(special) + 1
            }
        };
        let flags: u64 = field(
            self.memory,
            pool.definition,
            csub_unit_definition::AIR_FLAGS,
        )?;
        if flags & (1_u64 << 33) != 0 {
            Ok(i64::from(field::<i32>(
                self.memory,
                base,
                cair_base::CAPACITY,
            )?))
        } else {
            if flags & 0x1f_0037_fc00 == 0 {
                return Err("carrier training subtype is not air");
            }
            Ok(i64::from(field::<i32>(
                self.memory,
                pool.definition,
                SUBTYPE_CAPACITIES + kind * 4,
            )?))
        }
    }

    fn host_has_carrier_module(&self, host: usize) -> Result<bool, &'static str> {
        let pool = host
            .checked_add(cship::EQUIPMENT_POOL)
            .ok_or("carrier host equipment pool address overflow")?;
        let variants = cequipment_variant_pool::entries(self.memory, self.base, pool)?;
        let mut found = false;
        for index in 0..variants.count {
            let entry = variants
                .element(index, 16)
                .ok_or("carrier host variant address overflow")?;
            let variant = pointer(self.memory, entry, 0)?;
            cequipment_variant::identity(self.memory, self.base, variant)?;
            let modules = array(self.memory, variant, cequipment_variant::MODULES + 8, 16)?;
            for index in 0..modules.count {
                let entry = modules
                    .element(index, 16)
                    .ok_or("carrier host module address overflow")?;
                let module = pointer(self.memory, entry, 8)?;
                object(
                    self.memory,
                    self.base,
                    module,
                    cequipment_module::VTABLE_RVA,
                )?;
                if field::<u8>(self.memory, module, MODULE_CARRIER_FLAG)? != 0 {
                    found = true;
                    break;
                }
            }
        }
        Ok(found)
    }

    fn costs(&mut self) -> Result<Array, &'static str> {
        *self.costs.get_or_insert_with(|| {
            global_array(self.memory, self.base, defines::MISSION_COSTS_RVA, 8)
        })
    }
    fn thresholds(&mut self) -> Result<Array, &'static str> {
        *self.thresholds.get_or_insert_with(|| {
            global_array(self.memory, self.base, defines::CAPACITY_THRESHOLDS_RVA, 8)
        })
    }
    fn maximum_cost(&mut self) -> Value {
        if let Some(value) = self.maximum_cost {
            return value;
        }
        let value = (|| {
            let costs = self.costs()?;
            if costs.count < MAXIMUM_MISSIONS as u32 {
                return Err("carrier maximum mission costs incomplete");
            }
            let mut bytes = [0; MAXIMUM_MISSIONS * 8];
            if !read_slice(self.memory, costs.data, &mut bytes) {
                return Err("carrier maximum mission costs unreadable");
            }
            Ok(bytes
                .chunks_exact(8)
                .map(|bytes| i64::from_le_bytes(bytes.try_into().unwrap()))
                .fold(0, i64::max))
        })();
        self.maximum_cost = Some(value);
        value
    }
    fn weight(&mut self) -> Value {
        cached_scalar(
            self.memory,
            self.base,
            defines::AIR_WEIGHT_RVA,
            &mut self.weight,
        )
    }
    fn night_threshold(&mut self) -> Value {
        cached_scalar(
            self.memory,
            self.base,
            defines::CONTEXT_THRESHOLD_RVA,
            &mut self.night_threshold,
        )
    }

    fn forecast(&mut self, region: usize) -> Result<u8, &'static str> {
        let start = forecast_hash(region) % self.forecasts.len().max(1);
        for offset in 0..self.forecasts.len() {
            let index = (start + offset) % self.forecasts.len();
            let slot = self.forecasts[index];
            if let Some(value) = slot.value
                && slot.region == region
            {
                return value;
            }
            if slot.value.is_none() {
                let value = (|| {
                    let hours = self.read_forecast(region)?;
                    let threshold = self.night_threshold()?;
                    Ok(hours.iter().filter(|value| **value > threshold).count() as u8)
                })();
                self.forecasts[index] = ForecastSlot {
                    region,
                    value: Some(value),
                };
                return value;
            }
        }
        Err("carrier fuel forecast memo capacity exceeded")
    }

    fn read_forecast(&mut self, region: usize) -> Result<[i64; FORECAST_HOURS], &'static str> {
        object(
            self.memory,
            self.base,
            region,
            cstrategic_region::VTABLE_RVA,
        )?;
        let secondary = region
            .checked_add(cstrategic_region::SECONDARY)
            .ok_or("carrier forecast region address overflow")?;
        object(
            self.memory,
            self.base,
            secondary,
            cstrategic_region::SECONDARY_VTABLE_RVA,
        )?;
        let regions = *self.regions.get_or_insert_with(|| {
            array(
                self.memory,
                self.game_state,
                cgame_state::STRATEGIC_REGIONS,
                8,
            )
        });
        let slot: u32 = field(self.memory, region, cstrategic_region::RUNTIME_SLOT)?;
        let at = regions?
            .element(slot, 8)
            .ok_or("carrier forecast region outside census")?;
        if read::<u64>(self.memory, at) != Some(region as u64) {
            return Err("carrier forecast region census mismatch");
        }
        let provinces = array(self.memory, region, cstrategic_region::PROVINCES, 8)?;
        if provinces.count == 0 {
            return Ok([SENTINEL; FORECAST_HOURS]);
        }
        let source = if let Some(source) = self.forecast_source {
            source?
        } else {
            let source = self.read_forecast_source();
            self.forecast_source = Some(source);
            source?
        };
        let mut sums = [0_i64; FORECAST_HOURS];
        for index in 0..provinces.count {
            let entry = provinces
                .element(index, 8)
                .ok_or("carrier forecast province address overflow")?;
            let province = pointer(self.memory, entry, 0)?;
            object(self.memory, self.base, province, cprovince::VTABLE_RVA)?;
            let id: i32 = field(self.memory, province, cprovince::ID)?;
            let id = u32::try_from(id).map_err(|_| "carrier forecast province id negative")?;
            let slot = source
                .provinces
                .element(id, 8)
                .ok_or("carrier forecast province outside census")?;
            if read::<u64>(self.memory, slot) != Some(province as u64) {
                return Err("carrier forecast province census mismatch");
            }
            let clock = source
                .clocks
                .element(id, sweather_per_province::CLOCK_STRIDE)
                .ok_or("carrier forecast province outside clock array")?;
            let mut bytes = [0; FORECAST_VECTOR_BYTES];
            if !read_slice(self.memory, clock, &mut bytes) {
                return Err("carrier forecast province vectors unreadable");
            }
            for (sum, bytes) in sums.iter_mut().zip(bytes.chunks_exact(24)) {
                let vector = std::array::from_fn(|index| {
                    i64::from_le_bytes(bytes[index * 8..index * 8 + 8].try_into().unwrap())
                });
                *sum = sum.wrapping_add(province_forecast(
                    vector,
                    source.coefficients,
                    source.subtract,
                    source.scale,
                ));
            }
        }
        Ok(sums.map(|sum| region_average(sum, provinces.count)))
    }

    fn read_forecast_source(&self) -> Result<ForecastSource, &'static str> {
        let manager = pointer(self.memory, self.game_state, cgame_state::WEATHER_MANAGER)?;
        object(
            self.memory,
            self.base,
            manager,
            cweather_manager::VTABLE_RVA,
        )?;
        let coefficients: [u8; 24] = field(self.memory, manager, FORECAST_COEFFICIENTS)?;
        Ok(ForecastSource {
            clocks: array(
                self.memory,
                manager,
                cweather_manager::PROVINCE_CLOCKS,
                sweather_per_province::CLOCK_STRIDE,
            )?,
            provinces: array(self.memory, self.game_state, cgame_state::PROVINCES, 8)?,
            coefficients: std::array::from_fn(|index| {
                i64::from_le_bytes(coefficients[index * 8..index * 8 + 8].try_into().unwrap())
            }),
            subtract: global(self.memory, self.base, defines::CONTEXT_SUBTRACT_RVA)?,
            scale: global(self.memory, self.base, defines::CONTEXT_SCALE_RVA)?,
        })
    }
}

fn modified_base(base: i64, ace: i64, country: i64) -> i64 {
    P.wrapping_add(ace)
        .wrapping_add(country)
        .max(0)
        .wrapping_mul(base)
        / P
}
fn ace_count_factor(count: i32, numerator: i64, cap: i64) -> i64 {
    let denominator = i64::from(count).wrapping_mul(P);
    if denominator != 0
        && (numerator as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
    {
        (numerator.wrapping_mul(P) / denominator).min(cap)
    } else {
        SENTINEL.min(cap)
    }
}
fn ace_term(raw: i64, count: i64, owner: i64) -> i64 {
    (raw.wrapping_mul(count) / P).wrapping_mul(P.wrapping_add(owner)) / P
}
fn current_term(modified: i64, weight: i64, mission_cost: i64) -> i64 {
    (modified.wrapping_mul(weight) / P).wrapping_mul(mission_cost) / P
}
fn maximum_term(planes: i32, modified: i64, weight: i64, cost: i64) -> i64 {
    let amount = i64::from(planes).wrapping_mul(modified).wrapping_mul(P) / P;
    (amount.wrapping_mul(weight) / P).wrapping_mul(cost) / P
}
fn capacity_gate(planes: i32, capacity: i32, threshold: i64) -> bool {
    let denominator = i64::from(capacity).wrapping_mul(P);
    if denominator == 0 {
        threshold < P + 1
    } else {
        threshold <= i64::from(planes).wrapping_mul(10_000_000_000) / denominator
    }
}
fn province_forecast(vector: [i64; 3], coefficients: [i64; 3], subtract: i64, scale: i64) -> i64 {
    let value = (coefficients[0].wrapping_mul(vector[0]) / P)
        .wrapping_add(coefficients[2].wrapping_mul(vector[2]) / P)
        .wrapping_add(coefficients[1].wrapping_mul(vector[1]) / P);
    (value.wrapping_sub(subtract).wrapping_mul(scale) / P).clamp(0, P)
}
fn region_average(sum: i64, count: u32) -> i64 {
    let denominator = i64::from(count).wrapping_mul(P);
    if denominator != 0 && (sum as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd
    {
        sum.wrapping_mul(P) / denominator
    } else {
        SENTINEL
    }
}
fn period_term(amount: i64, mode: u8, nights: u8) -> i64 {
    let nights = i64::from(nights);
    let count = if mode == 0 {
        FORECAST_HOURS as i64 - nights
    } else {
        nights
    };
    let fraction = count.wrapping_mul(10_000_000_000) / 2_400_000;
    fraction.wrapping_mul(amount) / P
}

fn cached_scalar(memory: &impl Memory, base: usize, key: Rva, value: &mut Option<Value>) -> Value {
    *value.get_or_insert_with(|| global(memory, base, key))
}
fn global<T: Raw>(memory: &impl Memory, base: usize, key: Rva) -> Result<T, &'static str> {
    engine_sites::at(base, key)
        .and_then(|address| read(memory, address))
        .ok_or("carrier fuel field unreadable or address overflow")
}
fn global_array(
    memory: &impl Memory,
    base: usize,
    key: Rva,
    stride: usize,
) -> Result<Array, &'static str> {
    engine_sites::at(base, key)
        .and_then(|address| Array::read(memory, address, stride))
        .ok_or("carrier fuel array unreadable or invalid")
}
fn table_value(memory: &impl Memory, table: Array, index: i32) -> Value {
    let address = u32::try_from(index)
        .ok()
        .and_then(|index| table.element(index, 8))
        .ok_or("carrier fuel define index outside table")?;
    read(memory, address).ok_or("carrier fuel define value unreadable")
}
fn field<T: Raw>(memory: &impl Memory, pointer: usize, offset: usize) -> Result<T, &'static str> {
    if pointer == 0 {
        return Err("carrier fuel source pointer is null");
    }
    pointer
        .checked_add(offset)
        .and_then(|address| read(memory, address))
        .ok_or("carrier fuel field unreadable or address overflow")
}
fn pointer(memory: &impl Memory, object: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, object, offset)?)
        .map_err(|_| "carrier fuel pointer address overflow")
}
fn array(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    if object == 0 {
        return Err("carrier fuel array source pointer is null");
    }
    object
        .checked_add(offset)
        .and_then(|address| Array::read(memory, address, stride))
        .ok_or("carrier fuel array unreadable or invalid")
}
fn object(memory: &impl Memory, base: usize, pointer: usize, rva: Rva) -> Result<(), &'static str> {
    let expected =
        engine_sites::at(base, rva).ok_or("carrier fuel module address overflow")? as u64;
    if pointer == 0 || read::<u64>(memory, pointer) != Some(expected) {
        Err("carrier fuel object identity mismatch")
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
