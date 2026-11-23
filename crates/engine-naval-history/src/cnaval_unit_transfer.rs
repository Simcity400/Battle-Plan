
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x029e_a3a0);
pub const HANDLE: usize = 0x08;
pub const ACTION_STATUS: usize = 0x24;
pub const UNITS: usize = 0x38;
pub const TARGET_PROVINCE: usize = 0x50;
#[cfg(test)]
pub const SOURCE_PROVINCE: usize = 0x54;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x00e9_3e70);

use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native::{
    ccountry,
    cgame_state,
    cprovince,
    cweather_manager::{self, region_weather as sweather_per_region},
};
use engine_native_intelligence::{ccountry_intel, country_intel_row};
use engine_native_politics::cdiplomacy_status;
use engine_native_parts::{
    cstatic_modifier,
    cstrategic_region,
    modifier_accumulator,
    modifier_definition,
};
use engine_native_naval::{cstrategic_navy, cstrategic_navy_manager};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transfer {
    Absent,
    Present {
        handle: u64,
        status: Result<i32, &'static str>,
    },
}

pub fn transfer(
    memory: &impl Memory,
    base: usize,
    pointer: Result<usize, &'static str>,
    mut resolve: impl FnMut(u64) -> Result<Option<usize>, &'static str>,
) -> Result<Transfer, &'static str> {
    let pointer = pointer?;
    if pointer == 0 {
        return Ok(Transfer::Absent);
    }
    let expected = engine_sites::at(base, VTABLE_RVA)
        .ok_or("selected transport transfer module overflow")? as u64;
    if read::<u64>(memory, pointer) != Some(expected) {
        return Err("selected transport transfer vtable mismatch");
    }
    let handle = read::<u64>(memory, transfer_field(pointer, HANDLE)?)
        .filter(|handle| *handle >> 32 != 0)
        .ok_or("selected transport transfer full handle invalid")?;
    if resolve(handle)? != Some(pointer) {
        return Err("selected transport transfer registry mismatch");
    }
    let status = transfer_field(pointer, ACTION_STATUS).and_then(|address| {
        read(memory, address).ok_or("selected transport transfer status unreadable")
    });
    Ok(Transfer::Present { handle, status })
}

fn transfer_field(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .ok_or("selected transport original pointer null or overflow")
}

pub const BASE_SPEED_RVA: Rva = Rva(0x033e_3150);
const MODIFIER_SET_RVA: Rva = Rva(0x033d_e590);
const TRANSPORT_MODIFIER: usize = 0xf8;
const MAXIMUM_TOTAL_RVA: Rva = Rva(0x033e_1d68);
const MINIMUM_INTEL_RVA: Rva = Rva(0x033e_1000);
const MAXIMUM_INTEL_RVA: Rva = Rva(0x033e_1088);
const INTEL_REDUCTION_RVA: Rva = Rva(0x033e_2200);
const SPEED_TOKEN: u32 = crate::modifier_keys::NAVAL_SPEED_FACTOR;
const REDUCTION_TOKEN: u32 = crate::modifier_keys::NAVAL_MINES_EFFECT_REDUCTION;
const P: i64 = 100_000;

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("transport address overflow")
}
fn value<T: engine_native_memory::memory::Raw>(
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
fn key(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("transport address overflow")
}
fn global<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    base: usize,
    key: Rva,
) -> Result<T, &'static str> {
    value(memory, self::key(base, key)?, 0)
}
fn global_pointer(memory: &impl Memory, base: usize, key: Rva) -> Result<usize, &'static str> {
    pointer(memory, self::key(base, key)?, 0)
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
    country: engine_native_memory::country_identity::Country,
) -> Result<usize, &'static str> {
    let p = pointer(memory, country.pointer, ccountry::DIPLOMACY_STATUS)?;
    if value::<u64>(memory, p, 0)? != key(base, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA)? as u64
        || value::<u64>(memory, p, engine_native_parts::cdiplomacy_status_layout::COUNTRY)? != country.pointer as u64
    {
        return Err("transport diplomacy identity mismatch");
    }
    Ok(p)
}

fn intel(
    memory: &impl Memory,
    base: usize,
    country: engine_native_memory::country_identity::Country,
) -> Result<usize, &'static str> {
    let p = pointer(memory, country.pointer, ccountry::INTEL_TABLE)?;
    if value::<u64>(memory, p, 0)? != key(base, ccountry_intel::VTABLE_RVA)? as u64 {
        return Err("transport intel identity mismatch");
    }
    Ok(p)
}

fn intel_lane(
    memory: &impl Memory,
    countries: &ccountry::Reader<'_, impl Memory>,
    source: usize,
    target: usize,
    target_country: engine_native_memory::country_identity::Country,
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

fn totals<M: Memory>(
    memory: &M,
    base: usize,
    state: usize,
    countries: &ccountry::Reader<'_, M>,
    owner: engine_native_memory::country_identity::Country,
    region: u32,
) -> Result<(i64, i64), &'static str> {
    let owner_intel = intel(memory, base, owner)?;
    let relations = diplomacy(memory, base, owner)?;
    let root = at(relations, engine_native_parts::cdiplomacy_status_layout::ENEMY_COUNTRIES)?;
    let array = Array::read(memory, root, 4).ok_or("transport enemy roster invalid")?;
    let mut total = 0_i64;
    let mut weighted = 0_i64;
    for index in 0..array.count {
        let raw = read::<u32>(
            memory,
            array
                .element(index, 4)
                .ok_or("transport enemy index overflow")?,
        )
        .ok_or("transport enemy reference unreadable")?;
        let country = countries.resolve(raw)?;
        if value::<i32>(memory, country.pointer, ccountry::FOG_NETWORK_TABLE_COUNT)? <= 0 {
            continue;
        }
        let enemy_relations = diplomacy(memory, base, country)?;
        if value::<u8>(memory, enemy_relations, cdiplomacy_status::CAPITULATED)? != 0 {
            continue;
        }
        let enemy_intel = intel(memory, base, country)?;
        let weight =
            intel_lane(memory, countries, owner_intel, enemy_intel, country)?.wrapping_sub(
                intel_lane(memory, countries, enemy_intel, owner_intel, owner)?,
            );
        let navy = cstrategic_navy_manager::navy(memory, base, state, country)?;
        let regions = cstrategic_navy::regional_array(memory, navy)?;
        let at = regions
            .element(region, 8)
            .ok_or("navy regional operand outside array")?;
        let operand = read::<i64>(memory, at).ok_or("navy regional operand unreadable")?;
        total = total.wrapping_add(operand);
        weighted = weighted.wrapping_add(multiply(operand, weight));
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

fn composed_inputs<M: Memory>(
    memory: &M,
    base: usize,
    state: usize,
    owner: engine_native_memory::country_identity::Country,
    countries: &ccountry::Reader<'_, M>,
    region: u32,
) -> Result<ComposedInputs, &'static str> {
    let flags = global::<u32>(memory, base, modifier_definition::TRADE_FEATURE_FLAGS_RVA)?;
    if flags & (1 << 4) == 0 {
        return Ok(ComposedInputs::Zero);
    }
    let definitions = global_pointer(memory, base, MODIFIER_SET_RVA)?;
    let definition = value::<u64>(memory, definitions, TRANSPORT_MODIFIER)?;
    if definition == 0 {
        return Ok(ComposedInputs::Zero);
    }
    let definition =
        usize::try_from(definition).map_err(|_| "transport modifier pointer invalid")?;
    if value::<u64>(memory, definition, 0)? != key(base, cstatic_modifier::VTABLE_RVA)? as u64 {
        return Err("transport modifier identity mismatch");
    }
    cstrategic_navy_manager::navy(memory, base, state, owner)?;
    let (total, mean) = totals(memory, base, state, countries, owner, region)?;
    let maximum = global::<i64>(memory, base, MAXIMUM_TOTAL_RVA)?;
    let bounded = if total < 0 {
        0
    } else if total > maximum {
        maximum
    } else {
        total
    };
    let first_fraction = fraction(bounded, maximum)?;
    let reduction = if first_fraction != 0 {
        modifier_accumulator::lookup_effective_at(
            memory,
            base,
            at(owner.pointer, engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)?,
            REDUCTION_TOKEN,
        )?
        .effective
    } else {
        0
    };
    let (minimum, upper, scale) = if first_fraction != 0 {
        let minimum = global::<i64>(memory, base, MINIMUM_INTEL_RVA)?;
        if mean.wrapping_sub(minimum) > 0 {
            (
                minimum,
                global::<i64>(memory, base, MAXIMUM_INTEL_RVA)?,
                global::<i64>(memory, base, INTEL_REDUCTION_RVA)?,
            )
        } else {
            (minimum, minimum, 0)
        }
    } else {
        (0, 0, 0)
    };
    let fraction = reduction_fraction(total, mean, maximum, reduction, minimum, upper, scale)?;
    let result = if fraction > 0 {
        let raw = modifier_accumulator::lookup_at(
            memory,
            at(definition, cstatic_modifier::ACCUMULATOR)?,
            SPEED_TOKEN,
        )?;
        if raw.present {
            let metadata = global_pointer(memory, base, modifier_definition::TABLE_RVA)?;
            let category_at = at(
                at(metadata, SPEED_TOKEN as usize * modifier_definition::STRIDE)?,
                modifier_definition::CATEGORY_MASK,
            )?;
            let category = read::<u32>(memory, category_at)
                .map(i64::from)
                .ok_or("transport modifier category unreadable")?;
            if category == 0 {
                ComposedInputs::Zero
            } else {
                ComposedInputs::Modifier {
                    raw: raw.raw,
                    fraction,
                    operands: modifier_definition::operands(memory, base, SPEED_TOKEN)?,
                }
            }
        } else {
            ComposedInputs::Zero
        }
    } else {
        ComposedInputs::Zero
    };
    Ok(result)
}

fn weather_modifier(
    memory: &impl Memory,
    base: usize,
    state: usize,
    region: u32,
) -> Result<i64, &'static str> {
    let manager = pointer(memory, state, cgame_state::WEATHER_MANAGER)?;
    if value::<u64>(memory, manager, 0)? != key(base, cweather_manager::VTABLE_RVA)? as u64 {
        return Err("transport weather manager identity mismatch");
    }
    let root = at(manager, cweather_manager::REGIONS)?;
    let array = Array::read(memory, root, sweather_per_region::STRIDE)
        .ok_or("transport weather array invalid")?;
    let result = if let Some(row) = array.element(region, sweather_per_region::STRIDE) {
        if value::<u64>(memory, row, 0)? != key(base, sweather_per_region::VTABLE_RVA)? as u64
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
    };
    Ok(result)
}

pub fn movement_speed(
    memory: &impl Memory,
    base: usize,
    state: usize,
    owner: u32,
    region: usize,
) -> Result<i64, &'static str> {
    movement_speed_inputs(memory, base, state, owner, region).map(SpeedInputs::evaluate)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpeedInputs {
    speed: i64,
    weather: i64,
    composed: ComposedInputs,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComposedInputs {
    Zero,
    Modifier {
        raw: i64,
        fraction: i64,
        operands: modifier_definition::Operands,
    },
}
impl ComposedInputs {
    fn evaluate(self) -> i64 {
        match self {
            Self::Zero => 0,
            Self::Modifier { raw, fraction, operands } => {
                operands.effective(multiply(raw, fraction))
            }
        }
    }
}
impl SpeedInputs {
    pub fn evaluate(self) -> i64 {
        multiply(
            P.wrapping_add(self.weather).wrapping_add(self.composed.evaluate()),
            self.speed,
        )
    }
}

pub fn movement_speed_inputs(
    memory: &impl Memory,
    base: usize,
    state: usize,
    owner: u32,
    region: usize,
) -> Result<SpeedInputs, &'static str> {
    let arrays = ccountry::Reader::new(memory, base, state)?.arrays();
    let countries = ccountry::Reader::from_arrays(memory, base, state, arrays)?;
    let country = countries.resolve(owner)?;
    let region_id = value::<u32>(memory, region, cstrategic_region::ID)?;
    let slot = value::<u32>(memory, region, cstrategic_region::RUNTIME_SLOT)?;
    let root = at(state, cgame_state::STRATEGIC_REGIONS)?;
    let regions = Array::read(memory, root, 8).ok_or("transport region census invalid")?;
    let entry = regions
        .element(slot, 8)
        .ok_or("transport region slot outside census")?;
    let vtable = key(base, cstrategic_region::VTABLE_RVA)? as u64;
    if region == 0
        || region_id > i32::MAX as u32
        || read::<u64>(memory, entry) != Some(region as u64)
        || read::<u64>(memory, region) != Some(vtable)
    {
        return Err("transport region identity mismatch");
    }
    let speed = global::<i64>(memory, base, BASE_SPEED_RVA)?;
    let weather = weather_modifier(memory, base, state, region_id)?;
    let composed = composed_inputs(memory, base, state, country, &countries, region_id)?;
    Ok(SpeedInputs { speed, weather, composed })
}

mod speeds;
pub use speeds::{
    SpeedCache, TransportRegion, movement_speeds, movement_speeds_inputs, province_region,
};

#[cfg(test)]
mod tests;
