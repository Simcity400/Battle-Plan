
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native_air::{cair_wing, cstrategic_air};
use engine_native_parts::{cair_wing_pool, cbuilding, cbuilding_template};
use engine_native::{
    cprovince::{buildings as cbuilding_status, self},
    cgame_state,
    cstate,
};
use engine_sites::Rva;

pub const APPLY_DAMAGE_RVA: Rva = Rva(0x0116_3cd0);

pub use game_model::building_damage::Cause;

pub const CALLERS: [(Rva, Cause); 10] = [
    (Rva(0x0100_02cb), Cause::AirMission),
    (Rva(0x01a0_f0db), Cause::AirNaval),
    (Rva(0x0133_be87), Cause::LandCombat),
    (Rva(0x0133_c01c), Cause::LandCombat),
    (Rva(0x0133_c0f8), Cause::LandCombat),
    (Rva(0x0133_c1f7), Cause::LandCombat),
    (Rva(0x0032_71ae), Cause::Scripted),
    (Rva(0x00a2_bb51), Cause::Weekly),
    (Rva(0x014d_f801), Cause::ProgramTeardown),
    (Rva(0x014d_e667), Cause::AbortDismantle),
];

pub const CALLER_KEYS: [Rva; CALLERS.len()] = {
    let mut keys = [CALLERS[0].0; CALLERS.len()];
    let mut index = 1;
    while index < CALLERS.len() {
        keys[index] = CALLERS[index].0;
        index += 1;
    }
    keys
};

pub const AIR_SOURCE_KIND: u32 = 1;
pub const SOURCE_WING: usize = 8;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct Snapshot {
    pub building: usize,
    pub health_raw: i64,
    pub attacker: Option<u32>,
    pub attribution_failed: bool,
    pub victim: u32,
    pub state: u32,
    pub province: Option<u32>,
    pub building_type: u32,
    pub native_cause: u32,
    pub cause: Cause,
    pub game_hour: i32,
    status: usize,
    game_state: usize,
    source: usize,
    template: usize,
    total_level: i16,
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("building damage address overflow")
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
    reason: &'static str,
) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or(reason)
}

fn pointer(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    reason: &'static str,
) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, object, offset, reason)?).map_err(|_| reason)
}

fn authenticate(
    memory: &impl Memory,
    base: usize,
    object: usize,
    vtable: Rva,
    reason: &'static str,
) -> Result<(), &'static str> {
    let expected = engine_sites::at(base, vtable).ok_or("module address overflow")?;
    if object == 0 || read::<u64>(memory, object) != Some(expected as u64) {
        return Err(reason);
    }
    Ok(())
}

fn array(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    stride: usize,
    reason: &'static str,
) -> Result<Array, &'static str> {
    Array::read(memory, at(object, offset)?, stride).ok_or(reason)
}

fn health(memory: &impl Memory, building: usize) -> Result<(i16, i64), &'static str> {
    let bytes = field::<[u8; 16]>(
        memory,
        building,
        cbuilding::LEVEL,
        "building damage health unreadable",
    )?;
    let total = i16::from_le_bytes(bytes[..2].try_into().unwrap());
    let healthy = i16::from_le_bytes(bytes[2..4].try_into().unwrap());
    let partial = i64::from_le_bytes(bytes[8..].try_into().unwrap());
    let scale = cbuilding::PARTIAL_HEALTH_PER_LEVEL;
    if total < 0 || healthy < 0 || healthy > total || partial < 0 {
        return Err("building damage health form unsupported");
    }
    let value = (i64::from(healthy) * scale)
        .checked_add(partial)
        .ok_or("building damage health overflow")?;
    Ok((total, value))
}

pub fn building_count(memory: &impl Memory, base: usize) -> Option<usize> {
    let game_state = pointer(
        memory,
        engine_sites::at(base, cgame_state::SINGLETON_RVA)?,
        0,
        "building damage census game state unreadable",
    )
    .ok()?;
    if game_state == 0 {
        return None;
    }
    let mut count = 0_usize;
    for (root, vtable, id_offset, status_offset, expected_scope) in [
        (
            cgame_state::STATES,
            cstate::VTABLE_RVA,
            cstate::ID,
            cstate::BUILDING_STATUS,
            cbuilding_status::STATE_FLAG,
        ),
        (
            cgame_state::PROVINCES,
            cprovince::VTABLE_RVA,
            cprovince::ID,
            cprovince::BUILDING_STATUS,
            cbuilding_status::PROVINCE_FLAG,
        ),
    ] {
        let locations = array(
            memory,
            game_state,
            root,
            8,
            "building damage census root invalid",
        )
        .ok()?;
        for id in 0..locations.count {
            let slot = locations.element(id, 8)?;
            let object = pointer(
                memory,
                slot,
                0,
                "building damage census location unreadable",
            )
            .ok()?;
            if object == 0 {
                continue;
            }
            authenticate(
                memory,
                base,
                object,
                vtable,
                "building damage census location mismatch",
            )
            .ok()?;
            if id == 0
                || field::<u32>(
                    memory,
                    object,
                    id_offset,
                    "building damage census id unreadable",
                )
                .ok()?
                    != id
            {
                return None;
            }
            let status = at(object, status_offset).ok()?;
            authenticate(
                memory,
                base,
                status,
                cbuilding_status::VTABLE_RVA,
                "building damage census status mismatch",
            )
            .ok()?;
            let fields = field::<[u8; 8]>(
                memory,
                status,
                cbuilding_status::FLAGS,
                "building damage census scope unreadable",
            )
            .ok()?;
            let scope = u32::from_le_bytes(fields[..4].try_into().ok()?)
                & (cbuilding_status::STATE_FLAG | cbuilding_status::PROVINCE_FLAG);
            if scope != expected_scope || u32::from_le_bytes(fields[4..].try_into().ok()?) != id {
                return None;
            }
            let buildings = array(
                memory,
                status,
                cbuilding_status::BUILDINGS,
                8,
                "building damage census vector invalid",
            )
            .ok()?;
            count = count.checked_add(buildings.count as usize)?;
        }
    }
    Some(count)
}

fn location(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    id: u32,
    province_scope: bool,
    status: usize,
) -> Result<(u32, Option<u32>, u32), &'static str> {
    let (root, vtable, id_offset, status_offset, controller) = if province_scope {
        (
            cgame_state::PROVINCES,
            cprovince::VTABLE_RVA,
            cprovince::ID,
            cprovince::BUILDING_STATUS,
            cprovince::CONTROLLER,
        )
    } else {
        (
            cgame_state::STATES,
            cstate::VTABLE_RVA,
            cstate::ID,
            cstate::BUILDING_STATUS,
            cstate::CONTROLLER,
        )
    };
    let locations = array(
        memory,
        game_state,
        root,
        8,
        "building damage location root invalid",
    )?;
    let slot = locations
        .element(id, 8)
        .filter(|_| id != 0)
        .ok_or("building damage location outside root")?;
    let object = pointer(
        memory,
        slot,
        0,
        "building damage location pointer unreadable",
    )?;
    authenticate(
        memory,
        base,
        object,
        vtable,
        "building damage location vtable mismatch",
    )?;
    if field::<u32>(
        memory,
        object,
        id_offset,
        "building damage location id unreadable",
    )? != id
        || at(object, status_offset)? != status
    {
        return Err("building damage location identity mismatch");
    }
    let victim = field(
        memory,
        object,
        controller,
        "building damage controller unreadable",
    )?;
    if !province_scope {
        return Ok((id, None, victim));
    }
    let state = pointer(
        memory,
        object,
        cprovince::STATE,
        "building damage province state unreadable",
    )?;
    if state == 0 {
        return Ok((0, Some(id), victim));
    }
    authenticate(
        memory,
        base,
        state,
        cstate::VTABLE_RVA,
        "building damage province state vtable mismatch",
    )?;
    let state_id = field::<u32>(
        memory,
        state,
        cstate::ID,
        "building damage province state id unreadable",
    )?;
    let states = array(
        memory,
        game_state,
        cgame_state::STATES,
        8,
        "building damage state root invalid",
    )?;
    let slot = states
        .element(state_id, 8)
        .filter(|_| state_id != 0)
        .ok_or("building damage province state outside root")?;
    if pointer(memory, slot, 0, "building damage state slot unreadable")? != state {
        return Err("building damage province state identity mismatch");
    }
    Ok((state_id, Some(id), victim))
}

fn attacker(
    memory: &impl Memory,
    base: usize,
    source: usize,
    kind: u32,
) -> Result<Option<u32>, &'static str> {
    if kind != AIR_SOURCE_KIND {
        return Ok(None);
    }
    let wing = pointer(
        memory,
        source,
        SOURCE_WING,
        "building damage wing pointer unreadable",
    )?;
    if wing == 0 {
        return Ok(None);
    }
    authenticate(
        memory,
        base,
        wing,
        engine_native_parts::cair_wing_layout::VTABLE_RVA,
        "building damage wing vtable mismatch",
    )?;
    if field::<u32>(
        memory,
        wing,
        cair_wing::ID,
        "building damage wing id unreadable",
    )? == 0
    {
        return Err("building damage wing id is zero");
    }
    let override_country = field::<i32>(
        memory,
        wing,
        cair_wing::COUNTRY_OVERRIDE,
        "building damage wing country unreadable",
    )?;
    if override_country > 0 {
        return Ok(Some(override_country as u32));
    }
    let pool = pointer(
        memory,
        wing,
        engine_native_parts::cair_wing_layout::POOL,
        "building damage wing pool unreadable",
    )?;
    authenticate(
        memory,
        base,
        pool,
        cair_wing_pool::VTABLE_RVA,
        "building damage wing pool vtable mismatch",
    )?;
    let owner = pointer(
        memory,
        pool,
        cair_wing_pool::OWNER,
        "building damage air owner unreadable",
    )?;
    authenticate(
        memory,
        base,
        owner,
        cstrategic_air::VTABLE_RVA,
        "building damage air owner vtable mismatch",
    )?;
    let country = field::<u32>(
        memory,
        owner,
        cstrategic_air::COUNTRY,
        "building damage air owner country unreadable",
    )?;
    Ok((country != 0).then_some(country))
}

pub fn snapshot(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    building: usize,
    source: usize,
    return_address: usize,
    game_hour: i32,
) -> Result<Snapshot, &'static str> {
    authenticate(
        memory,
        base,
        building,
        cbuilding::VTABLE_RVA,
        "building damage vtable mismatch",
    )?;
    let building_type = field(
        memory,
        building,
        cbuilding::TYPE_ID,
        "building damage type unreadable",
    )?;
    let status = pointer(
        memory,
        building,
        cbuilding::BUILDING_STATUS,
        "building damage status unreadable",
    )?;
    let template = pointer(
        memory,
        building,
        cbuilding::TEMPLATE,
        "building damage template unreadable",
    )?;
    let (total_level, health_raw) = health(memory, building)?;
    let native_cause = if source == 0 || health_raw == 0 {
        0
    } else {
        field(memory, source, 0, "building damage source kind unreadable")?
    };
    let cause = engine_sites::which(base, return_address, &CALLER_KEYS)
        .and_then(|key| CALLERS.iter().find(|(caller, _)| *caller == key))
        .map_or(Cause::Unknown, |(_, cause)| *cause);
    Ok(Snapshot {
        building,
        building_type,
        status,
        template,
        total_level,
        health_raw,
        game_state,
        source,
        native_cause,
        cause,
        game_hour,
        ..Default::default()
    })
}

pub fn attribution(
    memory: &impl Memory,
    base: usize,
    before: &Snapshot,
) -> Result<Snapshot, &'static str> {
    let status = before.status;
    let template = before.template;
    authenticate(
        memory,
        base,
        status,
        cbuilding_status::VTABLE_RVA,
        "building damage status vtable mismatch",
    )?;
    let identity = field::<[u8; 8]>(
        memory,
        status,
        cbuilding_status::FLAGS,
        "building damage status identity unreadable",
    )?;
    let flags = u32::from_le_bytes(identity[..4].try_into().unwrap());
    let id = u32::from_le_bytes(identity[4..].try_into().unwrap());
    let scope = flags & (cbuilding_status::PROVINCE_FLAG | cbuilding_status::STATE_FLAG);
    if scope != cbuilding_status::PROVINCE_FLAG && scope != cbuilding_status::STATE_FLAG {
        return Err("building damage status scope unsupported");
    }
    let (state, province, victim) = location(
        memory,
        base,
        before.game_state,
        id,
        scope == cbuilding_status::PROVINCE_FLAG,
        status,
    )?;
    if template == 0
        || field::<u32>(
            memory,
            template,
            cbuilding_template::TYPE_ID,
            "building damage template type unreadable",
        )? != before.building_type
    {
        return Err("building damage template type mismatch");
    }
    let ordinal = field::<u32>(
        memory,
        template,
        cbuilding_template::DATABASE_ORDINAL,
        "building damage template ordinal unreadable",
    )?;
    let indices = array(
        memory,
        status,
        cbuilding_status::BUILDING_INDICES,
        4,
        "building damage indices invalid",
    )?;
    let index_at = indices
        .element(ordinal, 4)
        .ok_or("building damage template outside indices")?;
    let index = field::<i32>(memory, index_at, 0, "building damage index unreadable")?;
    let buildings = array(
        memory,
        status,
        cbuilding_status::BUILDINGS,
        8,
        "building damage vector invalid",
    )?;
    let slot = u32::try_from(index)
        .ok()
        .and_then(|index| buildings.element(index, 8))
        .ok_or("building damage index outside vector")?;
    if pointer(memory, slot, 0, "building damage instance slot unreadable")? != before.building {
        return Err("building damage instance differs from status slot");
    }
    let (attacker, attribution_failed) =
        match attacker(memory, base, before.source, before.native_cause) {
            Ok(attacker) => (attacker, false),
            Err(_) => (None, true),
        };
    Ok(Snapshot {
        state,
        province,
        victim,
        attacker,
        attribution_failed,
        ..*before
    })
}

pub fn applied_damage(
    memory: &impl Memory,
    base: usize,
    before: &Snapshot,
) -> Result<i64, &'static str> {
    authenticate(
        memory,
        base,
        before.building,
        cbuilding::VTABLE_RVA,
        "building damage post vtable mismatch",
    )?;
    if field::<u32>(
        memory,
        before.building,
        cbuilding::TYPE_ID,
        "building damage post type unreadable",
    )? != before.building_type
        || pointer(
            memory,
            before.building,
            cbuilding::BUILDING_STATUS,
            "building damage post status unreadable",
        )? != before.status
        || pointer(
            memory,
            before.building,
            cbuilding::TEMPLATE,
            "building damage post template unreadable",
        )? != before.template
    {
        return Err("building damage identity changed during call");
    }
    let (level, after) = health(memory, before.building)?;
    if level != before.total_level {
        return Err("building damage total level changed during call");
    }
    before
        .health_raw
        .checked_sub(after)
        .ok_or("building damage health delta overflow")
}

#[cfg(test)]
mod tests;
