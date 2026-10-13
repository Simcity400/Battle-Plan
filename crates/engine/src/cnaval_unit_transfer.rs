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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Transfer {
    Absent,
    Present {
        handle: u64,
        status: Result<i32, &'static str>,
    },
}

/// Transport status uses the original unit +0x2F8 selector, then the signed
/// combat reference count at +0x24. The count is saved with token 0x2916.
/// The railway counter consumes only presence and status. Refusal retains identity;
/// never retry an original selector failure.
pub(crate) fn transfer(
    memory: &impl Memory,
    base: usize,
    pointer: Result<usize, &'static str>,
    mut resolve: impl FnMut(u64) -> Result<Option<usize>, &'static str>,
) -> Result<Transfer, &'static str> {
    let pointer = pointer?;
    if pointer == 0 {
        return Ok(Transfer::Absent);
    }
    let expected = base
        .checked_add(VTABLE_RVA)
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
) -> Result<usize, &'static str> {
    // Select navies by canonical country from the shared manager.
    let navies = cstrategic_navy_manager::navies(memory, base, state)?;
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
}

fn totals<M: Memory>(
    memory: &M,
    base: usize,
    state: usize,
    countries: &ccountry::Reader<'_, M>,
    owner: ccountry::Country,
    region: u32,
) -> Result<(i64, i64), &'static str> {
    // Acquire the owner's intel object before visiting enemies.
    let owner_intel = intel(memory, base, owner)?;
    let relations = diplomacy(memory, base, owner)?;
    let root = at(relations, cdiplomacy_status::ENEMY_COUNTRIES)?;
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
        let navy = selected_navy(memory, base, state, country, countries)?;
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
) -> Result<i64, &'static str> {
    let flags = value::<u32>(memory, base, modifier_definition::TRADE_FEATURE_FLAGS_RVA)?;
    // Feature 0x13 tests exactly DLC bit 4.
    if flags & (1 << 4) == 0 {
        return Ok(0);
    }
    let definitions = pointer(memory, base, MODIFIER_SET_RVA)?;
    let definition = value::<u64>(memory, definitions, TRANSPORT_MODIFIER)?;
    if definition == 0 {
        return Ok(0);
    }
    let definition =
        usize::try_from(definition).map_err(|_| "transport modifier pointer invalid")?;
    if value::<u64>(memory, definition, 0)? != at(base, cstatic_modifier::VTABLE_RVA)? as u64 {
        return Err("transport modifier identity mismatch");
    }
    // Resolve the owner's navy and +0x10 backlink after DLC/static-definition
    // gates, even with no enemies. The owner's regional operand is not reached.
    selected_navy(memory, base, state, owner, countries)?;
    let (total, mean) = totals(memory, base, state, countries, owner, region)?;
    let maximum = value::<i64>(memory, base, MAXIMUM_TOTAL_RVA)?;
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
            at(owner.pointer, ccountry::MODIFIER_ACCUMULATOR)?,
            REDUCTION_TOKEN,
        )?
        .effective
    } else {
        0
    };
    // The intel defines participate only after the initial nonzero fraction.
    let (minimum, upper, scale) = if first_fraction != 0 {
        let minimum = value::<i64>(memory, base, MINIMUM_INTEL_RVA)?;
        if mean.wrapping_sub(minimum) > 0 {
            (
                minimum,
                value::<i64>(memory, base, MAXIMUM_INTEL_RVA)?,
                value::<i64>(memory, base, INTEL_REDUCTION_RVA)?,
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
            let metadata = pointer(memory, base, modifier_definition::TABLE_RVA)?;
            let category_at = at(
                at(metadata, SPEED_TOKEN as usize * modifier_definition::STRIDE)?,
                modifier_definition::CATEGORY_MASK,
            )?;
            let category = read::<u32>(memory, category_at)
                .map(i64::from)
                .ok_or("transport modifier category unreadable")?;
            // Both category-filter stages use mask 0xffffffff, initialized by the
            // temporary accumulator constructor.
            if category == 0 {
                0
            } else {
                let scaled = multiply(raw.raw, fraction);
                modifier_definition::operands(memory, base, SPEED_TOKEN)?.effective(scaled)
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
) -> Result<i64, &'static str> {
    let manager = pointer(memory, state, cgame_state::WEATHER_MANAGER)?;
    if value::<u64>(memory, manager, 0)? != at(base, cweather_manager::VTABLE_RVA)? as u64 {
        return Err("transport weather manager identity mismatch");
    }
    let root = at(manager, cweather_manager::REGIONS)?;
    let array = Array::read(memory, root, sweather_per_region::STRIDE)
        .ok_or("transport weather array invalid")?;
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
    let vtable = at(base, cstrategic_region::VTABLE_RVA)? as u64;
    if region == 0
        || region_id > i32::MAX as u32
        || read::<u64>(memory, entry) != Some(region as u64)
        || read::<u64>(memory, region) != Some(vtable)
    {
        return Err("transport region identity mismatch");
    }
    // Read base speed before both additive factors.
    let speed = value::<i64>(memory, base, BASE_SPEED_RVA)?;
    let weather = weather_modifier(memory, base, state, region_id)?;
    let composed = composed_modifier(memory, base, state, country, &countries, region_id)?;
    Ok(multiply(
        P.wrapping_add(weather).wrapping_add(composed),
        speed,
    ))
}

#[cfg(test)]
mod tests;
