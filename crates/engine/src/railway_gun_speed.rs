//! On-demand equipment speed for a railway gun's first ordinary route edge.
//!
//! `140E74B20` asks `140BC5E60` for stat 0x3E. That path initializes an
//! accumulator (`141929560`), gathers current inputs (`141927730`) and reduces
//! it (`140BC6A10`); it never reads the variant's cached stat at +0x2F0.
//! The variant's +0xD0 list already holds the equipment bonuses selected by
//! `140BD2040` / `140E4AE40`, so no research or country bonus census is inferred.
//! Every collection retains native order and first-match semantics. Reads are
//! callback-local and allocation-free; unreadable dependencies refuse the ETA.

use crate::{
    Memory, ccountry, cequipment_module, cequipment_type, cequipment_upgrade,
    cequipment_upgrades_instance, cequipment_variant, cpdx_array::Array, cproduction_status,
    equipment_module_entry, equipment_upgrade_entry, fixed5, modifier_entry, read,
};

/// `140632AC0` maps stat 0x3E to this token. Static registration `1400EA1E1`
/// reads 0x2EB2 at `142ACDFE0` and "maximum_speed" at `142ACBBB8`.
/// The caller must admit it through the existing fixed-layout token check.
pub const MAXIMUM_SPEED_TOKEN: u32 = 0x2eb2;
const SPEED_STAT: u32 = 0x3e;

// 141927755: the type's dense stat slots start at +148.
const TYPE_STATS: usize = 0x148;
// 1406E6DB0 returns variant+D0; 14192776E/771 walk its pointer array.
const DIRECT_BONUSES: usize = 0xd0;
// 140BC6620: variant+E8 handles, production+178 entries of strideC8, handle+C0.
const NAMED_HANDLES: usize = 0xe8;
const NAMED_BONUSES: usize = 0x178;
const NAMED_STRIDE: usize = 0xc8;
const NAMED_HANDLE: usize = 0xc0;
// Constructors 140624790/140E47420 embed CEquipmentBonus at +38/+40;
// its entry array starts at +8, as 141927790/7F0 read.
const SPECIFIC_VTABLE_RVA: usize = 0x0270_29e8;
const BONUS_VTABLE_RVA: usize = 0x0270_2990;
const NAMED_VTABLE_RVA: usize = 0x0295_9ac0;
// 140BC1190 constructs the MIO CTraitBonus at variant+490.
const MIO_BONUS: usize = 0x490;
const MIO_VTABLE_RVA: usize = 0x0270_2c38;
// 141525090 / 1415196D0 construct upgrade+40 / module+128 stat groups.
const GROUP_VTABLE_RVA: usize = 0x0292_17f8;
const STATS_VTABLE_RVA: usize = 0x0270_2938;

/// Reused caller-owned storage for native `140BC6620`'s selected pointers.
/// It collects every handle before `141927730` reads any selected stat array.
/// A shortage reports the live raw handle count; grow outside acquisition.
pub struct NamedBonuses<'a> {
    pub pointers: &'a mut [usize],
    pub required: usize,
}

/// Initial caller-arena demand from the authenticated variant's own header.
/// Admission uses the raw handle count, before native filtering or matching.
pub fn named_bonus_demand(memory: &impl Memory, variant: usize) -> Result<usize, &'static str> {
    Ok(array(memory, variant, NAMED_HANDLES, 4)?.count as usize)
}

#[derive(Default)]
struct Accumulator {
    add: i64,
    average: i64,
    average_count: i32,
    factor: i64,
}

impl Accumulator {
    // 140BC6A10: average first, then the multiplicative factor for stat3E.
    fn speed(self) -> Result<i64, &'static str> {
        let average = if self.average_count > 0 {
            fixed5::fraction(self.average, i64::from(self.average_count) * fixed5::SCALE)?
        } else {
            0
        };
        Ok(self
            .add
            .wrapping_add(average)
            .wrapping_mul(self.factor.wrapping_add(fixed5::SCALE))
            / fixed5::SCALE)
    }
}

fn at(object: usize, offset: usize) -> Result<usize, &'static str> {
    if object == 0 {
        return Err("railway speed dependency is null");
    }
    object
        .checked_add(offset)
        .ok_or("railway speed dependency address overflow")
}

fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at(object, offset)?).ok_or("railway speed dependency unreadable")
}

fn pointer(memory: &impl Memory, object: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, object, offset)?)
        .map_err(|_| "railway speed pointer outside address space")
}

fn authenticate(
    memory: &impl Memory,
    base: usize,
    object: usize,
    vtable: usize,
) -> Result<(), &'static str> {
    let expected = base
        .checked_add(vtable)
        .ok_or("railway speed module address overflow")?;
    if field::<u64>(memory, object, 0)? != expected as u64 {
        return Err("railway speed dependency vtable mismatch");
    }
    Ok(())
}

fn array(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    Array::read(memory, at(object, offset)?, stride)
        .ok_or("railway speed array unreadable or invalid")
}

fn element(source: Array, ordinal: u32, stride: usize) -> Result<usize, &'static str> {
    source
        .element(ordinal, stride)
        .ok_or("railway speed array element outside bounds")
}

// These native equipment-stat and MIO scans are linear, not modifier
// accumulator binary searches. A matching zero remains present.
fn first_value(memory: &impl Memory, source: Array, key: u32) -> Result<Option<i64>, &'static str> {
    for ordinal in 0..source.count {
        let entry = element(source, ordinal, modifier_entry::STRIDE)?;
        if field::<u32>(memory, entry, modifier_entry::KEY)? == key {
            return field(memory, entry, modifier_entry::VALUE).map(Some);
        }
    }
    Ok(None)
}

fn bonus_speed(
    memory: &impl Memory,
    base: usize,
    bonus: usize,
    named: bool,
) -> Result<i64, &'static str> {
    let embedded = if named {
        authenticate(memory, base, bonus, NAMED_VTABLE_RVA)?;
        at(bonus, 0x40)?
    } else {
        authenticate(memory, base, bonus, SPECIFIC_VTABLE_RVA)?;
        at(bonus, 0x38)?
    };
    authenticate(memory, base, embedded, BONUS_VTABLE_RVA)?;
    Ok(first_value(memory, array(memory, embedded, 8, 16)?, SPEED_STAT)?.unwrap_or(0))
}

fn country(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    raw: u32,
) -> Result<usize, &'static str> {
    if raw != 0 {
        return countries.resolve(raw).map(|country| country.pointer);
    }
    // 140BA56A0 chooses slot zero for a zero reference, without reading a link.
    let pointer = pointer(memory, element(countries.arrays()[0], 0, 8)?, 0)?;
    authenticate(memory, base, pointer, ccountry::VTABLE_RVA)?;
    if field::<u32>(memory, pointer, ccountry::ID)? != 0 {
        return Err("railway speed reserved country identity mismatch");
    }
    Ok(pointer)
}

fn collect_named(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    variant: usize,
    scratch: &mut NamedBonuses<'_>,
) -> Result<usize, &'static str> {
    let handles = array(memory, variant, NAMED_HANDLES, 4)?;
    scratch.required = scratch.required.max(handles.count as usize);
    if handles.count == 0 {
        return Ok(0);
    }
    if handles.count as usize > scratch.pointers.len() {
        return Err("railway speed named bonus capacity insufficient");
    }
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    // 140BC666D resolves the creator before the optional origin override.
    let mut owner = country(
        memory,
        base,
        &countries,
        field(memory, variant, cequipment_variant::CREATOR_LINK)?,
    )?;
    let origin = field::<i32>(memory, variant, cequipment_variant::ORIGIN_LINK)?;
    if origin > 0 {
        owner = country(memory, base, &countries, origin as u32)?;
    }
    let production = pointer(memory, owner, ccountry::PRODUCTION_STATUS)?;
    authenticate(memory, base, production, cproduction_status::VTABLE_RVA)?;
    let bonuses = array(memory, production, NAMED_BONUSES, NAMED_STRIDE)?;
    let mut len = 0;
    for ordinal in 0..handles.count {
        let handle = field::<i32>(memory, element(handles, ordinal, 4)?, 0)?;
        if handle == -1 {
            continue;
        }
        for candidate in 0..bonuses.count {
            let bonus = element(bonuses, candidate, NAMED_STRIDE)?;
            if field::<i32>(memory, bonus, NAMED_HANDLE)? == handle {
                scratch.pointers[len] = bonus;
                len += 1;
                break;
            }
        }
    }
    Ok(len)
}

// 1419293C0 visits additive, multiplicative, then averaged arrays. Each
// selected value is scaled before the wrapping merge into the accumulator.
fn group(
    memory: &impl Memory,
    base: usize,
    source: usize,
    level: i64,
    sum: &mut Accumulator,
) -> Result<(), &'static str> {
    authenticate(memory, base, source, GROUP_VTABLE_RVA)?;
    for (offset, mode) in [(0x08, 0), (0x48, 1), (0x28, 2)] {
        let stats = at(source, offset)?;
        authenticate(memory, base, stats, STATS_VTABLE_RVA)?;
        if let Some(value) = first_value(memory, array(memory, stats, 8, 16)?, SPEED_STAT)? {
            let value = value.wrapping_mul(level) / fixed5::SCALE;
            match mode {
                0 => sum.add = sum.add.wrapping_add(value),
                1 => sum.factor = sum.factor.wrapping_add(value),
                _ => {
                    sum.average = sum.average.wrapping_add(value);
                    sum.average_count = sum.average_count.wrapping_add(1);
                }
            }
        }
    }
    Ok(())
}

/// Rebuild only stat0x3E from the authenticated live variant and its current
/// selected inputs. The caller has resolved the exact railway-gun reference
/// and authenticated this variant's full reference and primary vtable,
/// and must have verified [`MAXIMUM_SPEED_TOKEN`] in its fixed-layout report.
/// The named-pointer arena must be grown off-thread using `required`.
pub fn variant_speed(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    variant: usize,
    named: &mut NamedBonuses<'_>,
) -> Result<i64, &'static str> {
    let definition = pointer(memory, variant, cequipment_variant::DEFINITION)?;
    authenticate(memory, base, definition, cequipment_type::VTABLE_RVA)?;
    if field::<u32>(memory, definition, cequipment_type::TOKEN)?
        != field::<u32>(memory, variant, cequipment_variant::EQUIPMENT_TOKEN)?
    {
        return Err("railway speed equipment definition token mismatch");
    }
    let mut sum = Accumulator {
        add: field(memory, definition, TYPE_STATS + SPEED_STAT as usize * 8)?,
        ..Accumulator::default()
    };
    let direct = array(memory, variant, DIRECT_BONUSES, 8)?;
    crate::cpdx_array::pointers(
        memory,
        direct,
        "railway speed bonus pointer unreadable",
        |_, bonus| {
            sum.factor = sum
                .factor
                .wrapping_add(bonus_speed(memory, base, bonus, false)?);
            Ok(())
        },
    )?;
    let count = collect_named(memory, base, game_state, variant, named)?;
    for &bonus in &named.pointers[..count] {
        sum.factor = sum
            .factor
            .wrapping_add(bonus_speed(memory, base, bonus, true)?);
    }
    let mio = at(variant, MIO_BONUS)?;
    authenticate(memory, base, mio, MIO_VTABLE_RVA)?;
    sum.factor = sum.factor.wrapping_add(
        first_value(memory, array(memory, mio, 8, 16)?, MAXIMUM_SPEED_TOKEN)?.unwrap_or(0),
    );
    authenticate(
        memory,
        base,
        at(variant, cequipment_variant::UPGRADES)?,
        cequipment_upgrades_instance::VTABLE_RVA,
    )?;
    let upgrades = array(
        memory,
        variant,
        cequipment_variant::UPGRADES + cequipment_upgrades_instance::ENTRIES,
        equipment_upgrade_entry::STRIDE,
    )?;
    for ordinal in 0..upgrades.count {
        let entry = element(upgrades, ordinal, equipment_upgrade_entry::STRIDE)?;
        let level = field::<u8>(memory, entry, equipment_upgrade_entry::LEVEL_STORAGE)?;
        let upgrade = pointer(memory, entry, equipment_upgrade_entry::DEFINITION)?;
        authenticate(memory, base, upgrade, cequipment_upgrade::VTABLE_RVA)?;
        group(
            memory,
            base,
            at(upgrade, 0x40)?,
            i64::from(level) * fixed5::SCALE,
            &mut sum,
        )?;
    }
    let modules = array(
        memory,
        variant,
        cequipment_variant::MODULES + 8,
        equipment_module_entry::STRIDE,
    )?;
    for ordinal in 0..modules.count {
        let entry = element(modules, ordinal, equipment_module_entry::STRIDE)?;
        let module = pointer(memory, entry, equipment_module_entry::DEFINITION)?;
        authenticate(memory, base, module, cequipment_module::VTABLE_RVA)?;
        group(memory, base, at(module, 0x128)?, fixed5::SCALE, &mut sum)?;
    }
    sum.speed()
}

#[cfg(test)]
mod tests;
