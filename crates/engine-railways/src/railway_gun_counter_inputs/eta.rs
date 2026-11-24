
use engine_sites::Rva;
use super::{EtaOperands, FreshEtaObservations, Row, at, key};
use engine_native_memory::{Memory, cpdx_array::Array, fixed5, read};
use engine_native::{
    carmy,
    cequipment_variant,
    cgame_state,
    cprovince::{self, definition as province_definition},
};
use crate::{cnaval_unit_transfer, crailway_manager, fixed_layout, railway_gun_speed};
use engine_native_parts::creference_registry::HeaderResolver;
use std::cell::OnceCell;

const VARIANT: usize = 0x338;
const SUPPLY_CAPACITY: usize = 0x400;
const SUPPLY_REMAINING: usize = 0x410;
const OUT_OF_SUPPLY_SPEED_RVA: Rva = Rva(0x033e_36d0);
const LAND_SCALE_RVA: Rva = Rva(0x033e_10c8);
const NAVAL_SCALE_RVA: Rva = Rva(0x033e_2cc0);
const RAIL_MAX_RVA: Rva = Rva(0x033e_0ae0);

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or("railway gun ETA input unreadable")
}

fn global<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    base: usize,
    key: Rva,
) -> Result<T, &'static str> {
    read(memory, super::key(base, key)?).ok_or("railway gun ETA input unreadable")
}

fn definition(memory: &impl Memory, pointer: usize, id: u32) -> Result<usize, &'static str> {
    let definition = usize::try_from(field::<u64>(memory, pointer, cprovince::DEFINITION)?)
        .map_err(|_| "railway gun ETA definition pointer overflow")?;
    if field::<u32>(memory, definition, province_definition::ID)? != id {
        return Err("railway gun ETA province definition identity mismatch");
    }
    Ok(definition)
}

fn variant(
    memory: &impl Memory,
    base: usize,
    gun: usize,
    registry: &mut HeaderResolver,
) -> Result<Option<usize>, &'static str> {
    let handle = field::<u64>(memory, gun, VARIANT)?;
    let Some(variant) = registry.resolve(memory, base, handle)? else {
        return Ok(None);
    };
    if handle as u32 != cequipment_variant::REFERENCE_TYPE
        || handle >> 32 == 0
        || field::<u64>(memory, variant, 0)? != key(base, engine_native_parts::equipment_variant_identity::VTABLE_RVA)? as u64
        || field::<u64>(memory, variant, cequipment_variant::TYPE)? != handle
    {
        return Err("railway gun ETA equipment variant identity mismatch");
    }
    Ok(Some(variant))
}

pub(super) fn named_demand(
    memory: &impl Memory,
    base: usize,
    gun: usize,
    registry: &mut HeaderResolver,
) -> Result<usize, &'static str> {
    variant(memory, base, gun, registry)?.map_or(Ok(0), |variant| {
        railway_gun_speed::named_bonus_demand(memory, variant)
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "the gun locates the speed, and the named bonuses, registry, layout and layout buffer are caller-owned"
)]
fn equipment_speed(
    memory: &impl Memory,
    base: usize,
    state: usize,
    gun: usize,
    named: &mut railway_gun_speed::NamedBonuses<'_>,
    registry: &mut HeaderResolver,
    layout: &OnceCell<Result<fixed_layout::Report, &'static str>>,
    layout_buffer: &mut [u8],
) -> Result<i64, &'static str> {
    let Some(variant) = variant(memory, base, gun, registry)? else {
        return Ok(0);
    };
    let checked = layout
        .get_or_init(|| fixed_layout::check(memory, base, layout_buffer))
        .as_ref()
        .map_err(|reason| *reason)?;
    if checked
        .failure(&[], &[railway_gun_speed::MAXIMUM_SPEED_TOKEN])
        .is_some()
    {
        return Err("railway gun ETA maximum_speed token layout mismatch");
    }
    railway_gun_speed::variant_speed(memory, base, state, variant, named)
}

fn supply_factor(
    memory: &impl Memory,
    base: usize,
    gun: usize,
    exiled: bool,
) -> Result<i64, &'static str> {
    if exiled {
        return Ok(fixed5::SCALE);
    }
    let capacity = field::<i64>(memory, gun, SUPPLY_CAPACITY)?;
    let supplied = if capacity < 1 {
        0
    } else {
        let remaining = field::<i64>(memory, gun, SUPPLY_REMAINING)?;
        if (remaining as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
            fixed5::SCALE
        } else {
            remaining
                .wrapping_mul(fixed5::SCALE)
                .checked_div(capacity)
                .ok_or("railway gun ETA supply division traps")?
                .clamp(0, fixed5::SCALE)
        }
    };
    if supplied >= fixed5::SCALE {
        return Ok(fixed5::SCALE);
    }
    let penalty = global::<i64>(memory, base, OUT_OF_SUPPLY_SPEED_RVA)?;
    Ok(fixed5::SCALE
        .wrapping_add(fixed5::SCALE.wrapping_sub(supplied).wrapping_mul(penalty) / fixed5::SCALE))
}

#[expect(
    clippy::too_many_arguments,
    reason = "the gun locates the speed, and the named bonuses, registry, layout, layout buffer and observations are caller-owned"
)]
pub(super) fn maximum_speed(
    memory: &impl Memory,
    base: usize,
    state: usize,
    gun: usize,
    named: &mut railway_gun_speed::NamedBonuses<'_>,
    registry: &mut HeaderResolver,
    layout: &OnceCell<Result<fixed_layout::Report, &'static str>>,
    layout_buffer: &mut [u8],
    observations: &mut FreshEtaObservations,
) -> Result<i64, &'static str> {
    *observations.maximum_speed.get_or_insert_with(|| {
        equipment_speed(
            memory,
            base,
            state,
            gun,
            named,
            registry,
            layout,
            layout_buffer,
        )
    })
}

pub(super) fn land_supply_factor(
    memory: &impl Memory,
    base: usize,
    gun: usize,
    exiled: bool,
    observations: &mut FreshEtaObservations,
) -> Result<i64, &'static str> {
    *observations
        .land_supply_factor
        .get_or_insert_with(|| supply_factor(memory, base, gun, exiled))
}

#[expect(
    clippy::too_many_arguments,
    reason = "the source, row and next hop identify one estimate, and the named bonuses, registry, layout, buffer and observations are caller-owned"
)]
pub(super) fn capture(
    memory: &impl Memory,
    base: usize,
    state: usize,
    source: (usize, usize),
    row: &Row,
    next: u32,
    named: &mut railway_gun_speed::NamedBonuses<'_>,
    registry: &mut HeaderResolver,
    layout: &OnceCell<Result<fixed_layout::Report, &'static str>>,
    layout_buffer: &mut [u8],
    observations: &mut FreshEtaObservations,
) -> Result<EtaOperands, &'static str> {
    let (gun, from) = source;
    let numerator = field::<i64>(memory, gun, carmy::MOVEMENT_NUMERATOR)?;
    if from == 0 {
        return Err("railway gun ETA departure province is null");
    }
    let provinces = Array::read(memory, at(state, cgame_state::PROVINCES)?, 8)
        .ok_or("railway gun ETA province census invalid")?;
    let to = provinces
        .element(next, 8)
        .and_then(|slot| super::pointer_at(memory, slot))
        .filter(|pointer| *pointer != 0)
        .ok_or("railway gun ETA arrival province absent")?;
    if engine_native::cunit::registered_province(memory, base, provinces, from)? != row.province
        || engine_native::cunit::registered_province(memory, base, provinces, to)? != next
    {
        return Err("railway gun ETA endpoint identity mismatch");
    }
    let from_definition = definition(memory, from, row.province)?;
    let from_land =
        field::<u8>(memory, from_definition, province_definition::TERRAIN_FLAGS)? & 1 != 0;
    let to_land = from_land
        && field::<u8>(
            memory,
            definition(memory, to, next)?,
            province_definition::TERRAIN_FLAGS,
        )? & 1
            != 0;
    let current_speed = if row.naval_transfer || !from_land || !to_land {
        let region = usize::try_from(field::<u64>(memory, from, cprovince::STRATEGIC_REGION)?)
            .map_err(|_| "railway gun ETA region pointer overflow")?;
        cnaval_unit_transfer::movement_speed(memory, base, state, row.logical_owner, region)?
            .wrapping_mul(global::<i64>(memory, base, NAVAL_SCALE_RVA)?)
            / fixed5::SCALE
    } else {
        let base_speed = maximum_speed(
            memory,
            base,
            state,
            gun,
            named,
            registry,
            layout,
            layout_buffer,
            observations,
        )?;
        let rail = crailway_manager::railway_percentage(memory, base, state, from, to)?;
        let speed = base_speed.wrapping_add(
            rail.wrapping_mul(global::<i64>(memory, base, RAIL_MAX_RVA)?) / fixed5::SCALE,
        );
        let scaled =
            speed.wrapping_mul(global::<i64>(memory, base, LAND_SCALE_RVA)?) / fixed5::SCALE;
        scaled.wrapping_mul(land_supply_factor(
            memory,
            base,
            gun,
            row.exiled,
            observations,
        )?) / fixed5::SCALE
    };
    Ok(EtaOperands {
        numerator,
        current_speed,
    })
}
