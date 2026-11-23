
use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, fixed5, read, walk::Buffer};
use engine_native::{ccountry, cequipment_variant, cproduction_status};
use engine_native_parts::{
    cequipment_module,
    cequipment_type,
    cequipment_upgrade,
    modifier_entry,
};
use crate::cequipment_upgrades_instance;
use engine_native_politics::{equipment_module_entry, equipment_upgrade_entry};

pub use engine_native_memory::cpdx_lexer::MAXIMUM_SPEED_TOKEN;
const SPEED_STAT: u32 = 0x3e;

const TYPE_STATS: usize = 0x148;
const DIRECT_BONUSES: usize = 0xd0;
const NAMED_HANDLES: usize = 0xe8;
const NAMED_BONUSES: usize = 0x178;
const NAMED_STRIDE: usize = 0xc8;
const NAMED_HANDLE: usize = 0xc0;
const SPECIFIC_VTABLE_RVA: Rva = Rva(0x0279_69f0);
const BONUS_VTABLE_RVA: Rva = Rva(0x0279_6998);
const NAMED_VTABLE_RVA: Rva = Rva(0x029e_cff8);
const MIO_BONUS: usize = 0x490;
const MIO_VTABLE_RVA: Rva = Rva(0x0279_6c00);
const GROUP_VTABLE_RVA: Rva = Rva(0x029b_5d78);
const STATS_VTABLE_RVA: Rva = Rva(0x0279_6940);

pub struct NamedBonuses<'a> {
    pub pointers: Buffer<'a, usize>,
    pub required: usize,
}

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

fn field<T: engine_native_memory::memory::Raw>(
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
    vtable: Rva,
) -> Result<(), &'static str> {
    let expected = engine_sites::at(base, vtable)
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
    scratch.pointers.ensure(handles.count as usize)?;
    if handles.count == 0 {
        return Ok(0);
    }
    if handles.count as usize > scratch.pointers.len() {
        return Err("railway speed named bonus capacity insufficient");
    }
    let countries = ccountry::Reader::new(memory, base, game_state)?;
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
    engine_native_memory::cpdx_array::pointers(
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
