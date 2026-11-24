use super::capture::{identity, name};
use super::*;
use engine_native_parts::{
    air_fuel_defines as defines,
    cair_wing_pool,
    creference_registry,
    modifier_accumulator,
};
use engine_native_air::{
    cair_base,
    cair_wing,
    ccountry_air_container as container,
    cstrategic_air,
    cstrategic_air_manager,
};
use engine_native::ccountry;
use engine_native_memory::cpdx_array::{self, Array};

pub(super) fn pointer(
    memory: &impl Memory,
    array: Array,
    ordinal: u32,
) -> Result<usize, &'static str> {
    array
        .element(ordinal, 8)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or("fuel demand graph pointer unreadable")
}
pub(super) fn array(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    Array::read(memory, at(pointer, offset)?, stride)
        .ok_or("fuel demand native array unreadable or invalid")
}
fn vtable(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    expected: Rva,
) -> Result<(), &'static str> {
    if pointer == 0 || field::<u64>(memory, pointer, 0)? != module_at(base, expected)? as u64 {
        return Err("fuel demand graph vtable mismatch");
    }
    Ok(())
}

pub(super) fn slot<M: Memory>(
    memory: &M,
    countries: &ccountry::Reader<'_, M>,
    raw: u32,
) -> Result<u32, &'static str> {
    if raw == 0 {
        return Ok(0);
    }
    if (raw as i32) < 0 {
        return Err("fuel demand negative native country-link address is not captured");
    }
    let translated = countries.arrays()[1]
        .element(raw, 4)
        .and_then(|at| read::<i32>(memory, at))
        .ok_or("fuel demand native country translation unreadable")?;
    u32::try_from(translated)
        .map_err(|_| "fuel demand negative native container address is not captured")
}

fn modifier(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    offset: usize,
    key: u32,
) -> Result<Modifier, &'static str> {
    modifier_accumulator::requested_operand_at(memory, base, at(pointer, offset)?, key)
}

fn module_flag(memory: &impl Memory, ship: usize) -> Result<bool, &'static str> {
    if field::<u8>(memory, ship, 0x802)? != 0 {
        return Ok(field::<u8>(memory, ship, 0x801)? != 0);
    }
    let variants = array(memory, ship, 0x60, 16)?;
    let mut found = false;
    for ordinal in 0..variants.count {
        let variant = variants
            .element(ordinal, 16)
            .and_then(|at| read::<u64>(memory, at))
            .ok_or("fuel demand host variant pointer unreadable")? as usize;
        let modules = array(memory, variant, 0xb8, 16)?;
        for ordinal in 0..modules.count {
            let module = modules
                .element(ordinal, 16)
                .and_then(|at| at.checked_add(8))
                .and_then(|at| read::<u64>(memory, at))
                .ok_or("fuel demand host module pointer unreadable")?
                as usize;
            if field::<u8>(memory, module, 0x2d0)? != 0 {
                found = true;
                break;
            }
        }
    }
    Ok(found)
}

pub(super) fn capacity(
    memory: &impl Memory,
    base: usize,
    wing: usize,
) -> Result<i32, &'static str> {
    let pool = field::<u64>(memory, wing, engine_native_parts::cair_wing_layout::POOL)? as usize;
    vtable(memory, base, pool, cair_wing_pool::VTABLE_RVA)?;
    let assigned = field::<u64>(memory, pool, cair_wing_pool::AIR_BASE_REFERENCE)?;
    let air_base = creference_registry::resolve(memory, base, assigned)?
        .ok_or("fuel demand capacity base reference unresolved")?;
    vtable(memory, base, air_base, engine_native_parts::cair_base_layout::VTABLE_RVA)?;
    let state = field::<u64>(memory, air_base, cair_base::STATE)?;
    let mut kind = 0;
    if state == 0 {
        let host = field::<u64>(memory, air_base, engine_native_parts::cair_base_layout::HOST_REFERENCE)?;
        let ship = creference_registry::resolve(memory, base, host)?
            .ok_or("fuel demand capacity host ship reference unresolved")?;
        vtable(memory, base, ship, engine_native::cship::VTABLE_RVA)?;
        kind = if field::<u8>(memory, ship, engine_native::cship::FORMATION_FLAGS)? & 0x80 != 0 {
            3
        } else {
            1 + usize::from(module_flag(memory, ship)?)
        };
    }
    let definition = field::<u64>(memory, pool, cair_wing_pool::DEFINITION)? as usize;
    vtable(
        memory,
        base,
        definition,
        engine_native_parts::csub_unit_definition::VTABLE_RVA,
    )?;
    if field::<u64>(memory, definition, 0x5a8)? & (1 << 33) != 0 {
        field(memory, air_base, cair_base::CAPACITY)
    } else {
        field(memory, definition, 0x600 + kind * 4)
    }
}

pub(super) fn context(
    memory: &impl Memory,
    base: usize,
    state: usize,
    region: usize,
    out: &mut Buffers<'_>,
) -> Result<(usize, Context), &'static str> {
    if out.context_slots.is_growing() {
        let required = out
            .contexts
            .len
            .checked_add(1)
            .and_then(|count| count.checked_mul(2))
            .and_then(usize::checked_next_power_of_two)
            .ok_or("fuel demand context index size overflow")?;
        let before = out.context_slots.len();
        out.context_slots.ensure(required)?;
        if out.context_slots.len() != before {
            out.context_slots.fill(0);
            let mask = out.context_slots.len() - 1;
            for (index, context) in out.contexts.rows[..out.contexts.len].iter().enumerate() {
                let mut bucket = (context.pointer >> 4).wrapping_mul(0x9e37_79b9) & mask;
                while out.context_slots[bucket] != 0 {
                    bucket = (bucket + 1) & mask;
                }
                out.context_slots[bucket] = index + 1;
            }
        }
    }
    let mut bucket = 0;
    if !out.context_slots.is_empty() {
        let mask = out.context_slots.len() - 1;
        bucket = (region >> 4).wrapping_mul(0x9e37_79b9) & mask;
        for _ in 0..out.context_slots.len() {
            let Some(index) = out.context_slots[bucket].checked_sub(1) else {
                break;
            };
            let owned = out
                .contexts
                .rows
                .get(index)
                .ok_or("fuel demand context index invalid")?;
            if owned.pointer == region {
                return owned.reason.map_or(Ok((index, *owned)), Err);
            }
            bucket = (bucket + 1) & mask;
        }
    }
    vtable(memory, base, region, engine_native_parts::cstrategic_region::VTABLE_RVA)?;
    let mut result = Context {
        region: field(memory, region, engine_native_parts::cstrategic_region::ID)?,
        pointer: region,
        ..Default::default()
    };
    let start = out.weather.len;
    let read = (|| {
        let manager = field::<u64>(memory, state, engine_native::cgame_state::WEATHER_MANAGER)? as usize;
        for slot in 0..24 {
            let count = field::<i32>(memory, region, engine_native_parts::cstrategic_region::PROVINCES_COUNT)?;
            result.counts[slot] = count;
            if count > 0 {
                vtable(memory, base, manager, engine_native::cweather_manager::VTABLE_RVA)?;
                let provinces = array(memory, region, engine_native_parts::cstrategic_region::PROVINCES, 8)?;
                if provinces.count != count as u32 {
                    return Err("fuel demand context province count changed within read");
                }
                out.weather.demand(provinces.count)?;
                for ordinal in 0..provinces.count {
                    let province = pointer(memory, provinces, ordinal)?;
                    vtable(memory, base, province, engine_native::cprovince::VTABLE_RVA)?;
                    let selector = field::<i32>(memory, province, 0xa4)?;
                    let grid = field::<u64>(memory, manager, 0x370)? as usize;
                    let offset = (i64::from(selector) * 0x22 + slot as i64) * 0x18;
                    let cell = grid
                        .checked_add_signed(
                            isize::try_from(offset)
                                .map_err(|_| "fuel demand weather index overflow")?,
                        )
                        .ok_or("fuel demand weather address overflow")?;
                    out.weather.push(Weather {
                        slot: slot as u8,
                        selector,
                        coefficients: [
                            field(memory, manager, 0x350)?,
                            field(memory, manager, 0x358)?,
                            field(memory, manager, 0x360)?,
                        ],
                        cell: {
                            let bytes = field::<[u8; 24]>(memory, cell, 0)?;
                            std::array::from_fn(|index| {
                                i64::from_le_bytes(std::array::from_fn(|byte| {
                                    bytes[index * 8 + byte]
                                }))
                            })
                        },
                        subtract: global(memory, base, defines::CONTEXT_SUBTRACT_RVA)?,
                        scale: global(memory, base, defines::CONTEXT_SCALE_RVA)?,
                    })?;
                }
            }
            result.thresholds[slot] = global(memory, base, defines::CONTEXT_THRESHOLD_RVA)?;
        }
        Ok(())
    })();
    result.weather = out.weather.span(start);
    result.reason = read.err();
    let index = out.contexts.len;
    out.contexts.push(result)?;
    if let Some(slot) = out.context_slots.get_mut(bucket) {
        *slot = index + 1;
    }
    result.reason.map_or(Ok((index, result)), Err)
}

#[expect(
    clippy::type_complexity,
    reason = "the inputs, the weather span, the optional context index and the optional raw owner and country override are returned together from one wing read"
)]
fn inputs<M: Memory>(
    memory: &M,
    base: usize,
    state: usize,
    countries: &ccountry::Reader<'_, M>,
    wing: usize,
    out: &mut Buffers<'_>,
) -> Result<(AirInputs, Span, Option<usize>, Option<(u32, i32)>), &'static str> {
    let mut result = AirInputs {
        mission_first: field(memory, wing, 0xa4)?,
        ..Default::default()
    };
    if result.mission_first == 0 {
        result.mask = field(memory, wing, 0xa0)?;
        if result.mask & engine_native_air::air_mission::TRAINING_MASK == 0 {
            return Ok((result, Span::default(), None, None));
        }
        result.veto = field(memory, wing, cair_wing::ELIGIBILITY_VETO)?;
        if result.veto != 0 {
            return Ok((result, Span::default(), None, None));
        }
        result.counter = field(memory, wing, cair_wing::ELIGIBILITY_COUNTER)?;
        result.limit = field(memory, wing, cair_wing::ELIGIBILITY_LIMIT)?;
        if result.counter > result.limit {
            return Ok((result, Span::default(), None, None));
        }
        result.override_reference = field(memory, wing, cair_wing::BASE_OVERRIDE)?;
        if result.override_reference != 0 {
            result.override_resolved =
                creference_registry::resolve(memory, base, result.override_reference)?.is_some();
            if result.override_resolved {
                return Ok((result, Span::default(), None, None));
            }
        }
        result.planes = field(memory, wing, cair_wing::PLANE_COUNT)?;
        if result.planes <= 0 {
            return Ok((result, Span::default(), None, None));
        }
        let class = field::<i32>(
            memory,
            wing,
            cair_wing::MISSION + engine_native_air::air_mission::CAPACITY_CLASS,
        )?;
        let thresholds = global::<u64>(memory, base, defines::CAPACITY_THRESHOLDS_RVA)? as usize;
        let selected = thresholds
            .checked_add_signed(class as isize * 8)
            .ok_or("fuel demand capacity threshold address overflow")?;
        result.capacity_threshold = field(memory, selected, 0)?;
        result.capacity_divisor = capacity(memory, base, wing)?;
        if !capacity_gate(
            result.planes,
            result.capacity_divisor,
            result.capacity_threshold,
        ) {
            return Ok((result, Span::default(), None, None));
        }
    }
    result.reached = true;
    result.mission_selected = field(memory, wing, 0xa4)?;
    let index = if result.mission_selected == 0 {
        result.planes = field(memory, wing, cair_wing::PLANE_COUNT)?;
        defines::TRAINING_COST_INDEX
    } else {
        if (result.mission_selected as i32) < 0 {
            return Err(
                "fuel demand native mission-bit loop does not terminate for a negative mask",
            );
        }
        result.active_count = field(memory, wing, 0x170)?;
        (31 - result.mission_selected.leading_zeros()) as usize
    };
    let costs = global::<u64>(memory, base, defines::MISSION_COSTS_RVA)? as usize;
    result.mission_cost = field(memory, costs, index * 8)?;
    result.base_fuel = field(memory, wing, cair_wing::BASE_FUEL)?;
    result.ace = field(memory, wing, cair_wing::ACE_REFERENCE)?;
    if result.ace != 0
        && let Some(ace) = creference_registry::resolve(memory, base, result.ace)?
    {
        vtable(memory, base, ace, engine_native_air::cace::VTABLE_RVA)?;
        result.ace_alive = field(memory, ace, engine_native_air::cace::ALIVE)?;
        if result.ace_alive != 0 {
            result.ace_modifier = modifier(memory, base, ace, engine_native_air::cace::MODIFIERS, 0x19e)?;
            result.ace_assigned_wing = field(memory, ace, engine_native_air::cace::ASSIGNED_WING_REFERENCE)?;
            if result.ace_assigned_wing != 0
                && let Some(assigned) =
                    creference_registry::resolve(memory, base, result.ace_assigned_wing)?
                && assigned != 0x10
            {
                let assigned = assigned
                    .checked_sub(engine_native_parts::cair_wing_layout::SECONDARY)
                    .ok_or("fuel demand ace wing address invalid")?;
                vtable(memory, base, assigned, engine_native_parts::cair_wing_layout::VTABLE_RVA)?;
                result.ace_assigned_planes = Some(field(memory, assigned, cair_wing::PLANE_COUNT)?);
                if result.ace_assigned_planes != Some(0) {
                    result.ace_count = global(memory, base, defines::ACE_COUNT_RVA)?;
                    result.ace_cap = global(memory, base, defines::ACE_CAP_RVA)?;
                }
            }
            let owner = field::<u64>(memory, ace, engine_native_air::cace::OWNER_COUNTRY)? as usize;
            if field::<u32>(memory, owner, ccountry::ID)? == 0 {
                let reserved =
                    engine_native_air::game_score_inputs::read_slot_zero(memory, base, countries.arrays()[0])?
                        .ok_or("fuel demand ace reserved country object is null")?;
                if reserved.pointer != owner {
                    return Err("fuel demand ace reserved country identity mismatch");
                }
            } else {
                countries.of_pointer(owner)?;
            }
            result.ace_country_modifier =
                modifier(memory, base, owner, engine_native_memory::country_identity::MODIFIER_ACCUMULATOR, 0x25a)?;
        }
    }
    let overridden = field::<i32>(memory, wing, cair_wing::COUNTRY_OVERRIDE)?;
    let raw = if overridden > 0 {
        overridden as u32
    } else {
        let pool = field::<u64>(memory, wing, engine_native_parts::cair_wing_layout::POOL)? as usize;
        vtable(memory, base, pool, cair_wing_pool::VTABLE_RVA)?;
        let owner = field::<u64>(memory, pool, cair_wing_pool::OWNER)? as usize;
        vtable(memory, base, owner, cstrategic_air::VTABLE_RVA)?;
        field(memory, owner, cstrategic_air::COUNTRY)?
    };
    let owner = if raw == 0 {
        engine_native_air::game_score_inputs::read_slot_zero(memory, base, countries.arrays()[0])?
            .ok_or("fuel demand reserved country object is null")?
    } else {
        countries.resolve(raw)?
    };
    result.country_modifier = modifier(
        memory,
        base,
        owner.pointer,
        engine_native_memory::country_identity::MODIFIER_ACCUMULATOR,
        0x19e,
    )?;
    result.period = field(memory, wing, 0xa8)?;
    result.air_weight = global(memory, base, defines::AIR_WEIGHT_RVA)?;
    let region = field::<u64>(
        memory,
        wing,
        cair_wing::MISSION + engine_native_air::air_mission::REGION,
    )? as usize;
    let mut weather = Span::default();
    let mut context_index = None;
    if region != 0 {
        vtable(memory, base, region, engine_native_parts::cstrategic_region::VTABLE_RVA)?;
        result.region = Some(field(memory, region, engine_native_parts::cstrategic_region::ID)?);
        if result.period != 2 {
            let (index, sampled) = context(memory, base, state, region, out)?;
            context_index = Some(index);
            result.context_counts = sampled.counts;
            result.context_thresholds = sampled.thresholds;
            weather = sampled.weather;
        }
    }
    Ok((result, weather, context_index, Some((raw, overridden))))
}

#[expect(
    clippy::too_many_arguments,
    reason = "the state, country reader, country and pointer locate one site, and the row and buffers are caller-owned"
)]
fn site<M: Memory>(
    memory: &M,
    base: usize,
    state: usize,
    countries: &ccountry::Reader<'_, M>,
    country: engine_native_memory::country_identity::Country,
    pointer: usize,
    row: &mut Site,
    out: &mut Buffers<'_>,
) -> Result<(), &'static str> {
    row.base = identity(
        memory,
        base,
        pointer,
        engine_native_parts::cair_base_layout::VTABLE_RVA,
        engine_native_parts::cair_base_layout::HANDLE,
        0,
    )?;
    let containers = array(memory, pointer, cair_base::COUNTRY_CONTAINERS, 8)?;
    let entry = self::pointer(memory, containers, slot(memory, countries, country.id)?)?;
    if entry == 0 {
        return Ok(());
    }
    vtable(memory, base, entry, container::VTABLE_RVA)?;
    let owner = field(memory, entry, container::COUNTRY)?;
    row.container_owner = Some(owner);
    let witness = self::pointer(memory, containers, slot(memory, countries, owner)?)?;
    if witness == 0 {
        return Ok(());
    }
    vtable(memory, base, witness, container::VTABLE_RVA)?;
    let capacity = field(memory, witness, container::CAPACITY_RAW)?;
    row.capacity = Some(capacity);
    if capacity <= 0 {
        return Ok(());
    }
    let zone = field::<u64>(memory, pointer, cair_base::STATE)? as usize;
    row.demilitarized = Some(if zone == 0 {
        0
    } else {
        vtable(memory, base, zone, engine_native::cstate::VTABLE_RVA)?;
        field(memory, zone, engine_native::cstate::DEMILITARIZED)?
    });
    if row.demilitarized != Some(0) {
        return Ok(());
    }
    if field::<i32>(memory, entry, container::POOLS_COUNT)? <= 0 {
        return Ok(());
    }
    let pools = array(memory, entry, container::POOLS, 8)?;
    for pool_ordinal in 0..pools.count {
        let pool = self::pointer(memory, pools, pool_ordinal)?;
        vtable(memory, base, pool, cair_wing_pool::VTABLE_RVA)?;
        let wings = array(memory, pool, cair_wing_pool::WINGS, 8)?;
        out.wings.demand(wings.count)?;
        cpdx_array::pointers(
            memory,
            wings,
            "fuel demand pool wing pointer unreadable",
            |wing_ordinal, wing| {
                let reference = identity(
                    memory,
                    base,
                    wing,
                    engine_native_parts::cair_wing_layout::VTABLE_RVA,
                    engine_native_parts::cair_wing_layout::REFERENCE_TYPE,
                    engine_native_parts::cair_wing_layout::SECONDARY,
                )?;
                let value = inputs(memory, base, state, countries, wing, out);
                let stored = name(memory, at(wing, cair_wing::NAME), out);
                let (inputs, weather, context, owner, reason) = match value {
                    Ok((inputs, weather, context, owner)) => {
                        (Some(inputs), weather, context, owner, None)
                    }
                    Err(reason) => (None, Span::default(), None, None, Some(reason)),
                };
                let owner = owner.map(Ok).unwrap_or_else(|| {
                    let override_country = field::<i32>(memory, wing, cair_wing::COUNTRY_OVERRIDE)?;
                    if override_country > 0 {
                        return Ok((override_country as u32, override_country));
                    }
                    let current_pool = field::<u64>(memory, wing, engine_native_parts::cair_wing_layout::POOL)? as usize;
                    vtable(memory, base, current_pool, cair_wing_pool::VTABLE_RVA)?;
                    let current_owner =
                        field::<u64>(memory, current_pool, cair_wing_pool::OWNER)? as usize;
                    vtable(memory, base, current_owner, cstrategic_air::VTABLE_RVA)?;
                    field(memory, current_owner, cstrategic_air::COUNTRY)
                        .map(|owner| (owner, override_country))
                });
                out.wings.push(Wing {
                    reference,
                    pool_ordinal,
                    wing_ordinal,
                    owner: owner.as_ref().ok().map(|value| value.0),
                    country_override: owner.as_ref().ok().map(|value| value.1),
                    owner_reason: owner.err(),
                    name: stored,
                    inputs,
                    weather,
                    context,
                    reason,
                })
            },
        )?;
    }
    Ok(())
}

pub(super) fn country<M: Memory>(
    memory: &M,
    base: usize,
    state: usize,
    countries: &ccountry::Reader<'_, M>,
    country: engine_native_memory::country_identity::Country,
    out: &mut Buffers<'_>,
) -> Result<(), &'static str> {
    let manager = field::<u64>(memory, state, engine_native::cgame_state::STRATEGIC_AIR_MANAGER)? as usize;
    vtable(memory, base, manager, cstrategic_air_manager::VTABLE_RVA)?;
    let owners = array(memory, manager, cstrategic_air_manager::COUNTRY_AIR, 8)?;
    let air = pointer(memory, owners, country.canonical_slot)?;
    vtable(memory, base, air, cstrategic_air::VTABLE_RVA)?;
    let raw = field(memory, air, cstrategic_air::COUNTRY)?;
    if slot(memory, countries, raw)? != country.canonical_slot {
        return Err("fuel demand strategic-air owner mismatch");
    }
    for (list, offset) in [
        cstrategic_air::FIRST_AIR_SITES,
        cstrategic_air::SECOND_AIR_SITES,
    ]
    .into_iter()
    .enumerate()
    {
        if field::<i32>(memory, air, offset + 12)? <= 0 {
            continue;
        }
        let sites = array(memory, air, offset, 8)?;
        out.sites.demand(sites.count)?;
        for ordinal in 0..sites.count {
            let start = out.wings.len;
            let mut row = Site {
                list: list as u8,
                ordinal,
                ..Default::default()
            };
            let site_pointer = pointer(memory, sites, ordinal);
            row.reason = site_pointer
                .and_then(|pointer| {
                    site(
                        memory, base, state, countries, country, pointer, &mut row, out,
                    )
                })
                .err();
            row.wings = out.wings.span(start);
            out.sites.push(row)?;
        }
    }
    Ok(())
}
