use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native::{ccountry, cgame_state, cprovince, cstate, cunit};
use engine_sites::Rva;

pub const SCRIPTED_REFRESH_CALL_RVA: Rva = Rva(0x0036_fe21);
pub const MAP_MODE_SINGLETON_RVA: Rva = Rva(0x033e_ad00);
pub const FIRST_SCRIPTED_MAP_MODE: i32 = 40;

pub fn scripted_map_mode(memory: &impl Memory, base: usize) -> Result<u32, &'static str> {
    let pointer = engine_sites::at(base, MAP_MODE_SINGLETON_RVA)
        .and_then(|address| read::<u64>(memory, address))
        .filter(|&pointer| pointer != 0)
        .ok_or("map-mode object is unavailable")?;
    let mode = read::<i32>(memory, pointer as usize).ok_or("map-mode selection is unreadable")?;
    mode.checked_sub(FIRST_SCRIPTED_MAP_MODE)
        .and_then(|index| u32::try_from(index).ok())
        .ok_or("forced map refresh names no scripted mode")
}

pub const FAKE_REVEAL_VISIBILITY_CALL_RVA: Rva = Rva(0x0134_af20);
pub const RAILWAY_OVERRUN_RVA: Rva = Rva(0x00ef_e970);
pub const CRITICAL_BOMBARDMENT_TAP_RVA: Rva = Rva(0x0133_bcda);
pub const GENERAL_CAPTURE_CALL_RVA: Rva = Rva(0x00c6_7ec0);
pub const PARADROP_LANDING_CALL_RVA: Rva = Rva(0x0100_8fcd);
pub const RESISTANCE_ACTIVITY_RVA: Rva = Rva(0x0101_c8d0);
pub const BUILDING_NOTIFICATION_RVA: Rva = Rva(0x0116_3760);

pub const FOG_OF_WAR_RVA: Rva = Rva(0x033d_daaa);

pub const INTEL_STRIDE: usize = 0x18;
pub const INTEL_LEVEL: usize = 0x10;

pub const UNIT_LEADER_COUNTRY: usize = 0x120;
pub const RESISTANCE_STATE: usize = 0x08;
pub const ACTIVITY_MAP_TEXT: usize = 0x370;
pub const STATE_BEST_PROVINCE: usize = 0x85c;
pub const COUNTRY_DIPLOMACY: usize = 0xf88;
pub const DIPLOMACY_OVERLORD: usize = 0x188;

pub const BUILDING_LEVEL: usize = 0x40;
pub const BUILDING_HEALTHY_LEVEL: usize = 0x42;
pub const BUILDING_PARTIAL_HEALTH: usize = 0x48;
pub const BUILDING_TYPE: usize = 0x08;
pub const BUILDING_STATUS: usize = 0x1d8;
pub const BUILDING_STATUS_FLAGS: usize = 0x68;
pub const BUILDING_STATUS_ID: usize = 0x6c;
pub const BUILDING_STATUS_STATE_FLAG: u32 = 2;
pub const LEVEL_HEALTH: i64 = 10_000_000;

fn at(object: usize, offset: usize) -> Result<usize, &'static str> {
    object
        .checked_add(offset)
        .ok_or("map event address overflow")
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    reason: &'static str,
) -> Result<T, &'static str> {
    if object == 0 {
        return Err(reason);
    }
    read(memory, at(object, offset)?).ok_or(reason)
}

fn pointer(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    reason: &'static str,
) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, object, offset, reason)?).map_err(|_| reason)
}

pub fn game_state(memory: &impl Memory, base: usize) -> Result<usize, &'static str> {
    const UNREADABLE: &str = "map event game state unreadable";
    let singleton = engine_sites::at(base, cgame_state::SINGLETON_RVA).ok_or(UNREADABLE)?;
    let state = pointer(memory, singleton, 0, UNREADABLE)?;
    if state == 0 {
        return Err("map event game state absent");
    }
    Ok(state)
}

pub fn hour(memory: &impl Memory, game_state: usize) -> Result<i32, &'static str> {
    field(
        memory,
        game_state,
        cgame_state::CURRENT_HOUR,
        "map event hour unreadable",
    )
}

pub fn fog(memory: &impl Memory, base: usize) -> Result<bool, &'static str> {
    const UNREADABLE: &str = "map event fog switch unreadable";
    let switch = engine_sites::at(base, FOG_OF_WAR_RVA).ok_or(UNREADABLE)?;
    field::<u8>(memory, switch, 0, UNREADABLE).map(|fog| fog != 0)
}

pub fn province_id(memory: &impl Memory, province: usize) -> Result<u32, &'static str> {
    field(
        memory,
        province,
        cprovince::ID,
        "map event province id unreadable",
    )
}

fn provinces(memory: &impl Memory, game_state: usize) -> Result<Array, &'static str> {
    Array::read(memory, at(game_state, cgame_state::PROVINCES)?, 8)
        .ok_or("map event province root unreadable")
}

fn province_at(memory: &impl Memory, provinces: Array, id: u32) -> Result<usize, &'static str> {
    let slot = provinces
        .element(id, 8)
        .ok_or("map event province outside root")?;
    pointer(memory, slot, 0, "map event province slot unreadable")
}

pub fn intel(memory: &impl Memory, unit: usize) -> Result<Array, &'static str> {
    Array::read(memory, at(unit, cunit::VIEWER_DATA)?, INTEL_STRIDE)
        .ok_or("map event intel array unreadable")
}

pub type IntelEntry = (u32, i8);

pub fn intel_entries(
    memory: &impl Memory,
    intel: Array,
    start: usize,
    output: &mut [IntelEntry],
) -> Result<(), &'static str> {
    const BATCH: usize = 32;
    let mut bytes = [0_u8; BATCH * INTEL_STRIDE];
    for (batch, out) in output.chunks_mut(BATCH).enumerate() {
        let first = start + batch * BATCH;
        let address = first
            .checked_mul(INTEL_STRIDE)
            .and_then(|offset| intel.data.checked_add(offset))
            .ok_or("map event intel address overflow")?;
        let bytes = &mut bytes[..out.len() * INTEL_STRIDE];
        if !engine_native_memory::read_slice(memory, address, bytes) {
            return Err("map event intel entries unreadable");
        }
        for (entry, raw) in out.iter_mut().zip(bytes.chunks_exact(INTEL_STRIDE)) {
            *entry = (
                u32::from_le_bytes(raw[..4].try_into().unwrap()),
                raw[INTEL_LEVEL] as i8,
            );
        }
    }
    Ok(())
}

pub fn unit_province(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    unit: usize,
) -> Result<u32, &'static str> {
    let current = pointer(
        memory,
        unit,
        cunit::LOCATION,
        "map event unit province unreadable",
    )?;
    let current_id = province_id(memory, current)?;
    let route = field::<i32>(
        memory,
        unit,
        cunit::ORDINARY_ROUTE_COUNT,
        "map event unit route unreadable",
    )?;
    if route <= 0
        || field::<u32>(
            memory,
            unit,
            cunit::CLASS,
            "map event unit class unreadable",
        )? != 0
    {
        return Ok(current_id);
    }
    let path = pointer(
        memory,
        unit,
        cunit::ORDINARY_ROUTE_DATA,
        "map event unit route data unreadable",
    )?;
    let first = field::<u32>(memory, path, 0, "map event unit route unreadable")?;
    let target = province_at(memory, provinces(memory, game_state)?, first)?;
    if target == 0 {
        return Ok(current_id);
    }
    let flags = |province: usize| -> Result<u8, &'static str> {
        let definition = pointer(
            memory,
            province,
            cprovince::DEFINITION,
            "map event province definition unreadable",
        )?;
        field(
            memory,
            definition,
            engine_native::cprovince::definition::TERRAIN_FLAGS,
            "map event province terrain unreadable",
        )
    };
    if flags(current)? & 3 != 0 || flags(target)? & 1 == 0 {
        return Ok(current_id);
    }
    let controller = field::<u32>(
        memory,
        target,
        cprovince::CONTROLLER,
        "map event route controller unreadable",
    )?;
    let owner = field::<u32>(
        memory,
        unit,
        cunit::LOGICAL_OWNER,
        "map event unit logical owner unreadable",
    )?;
    let enemy = if controller == 0 {
        let reserved = ccountry::Reader::new(memory, base, game_state)?.reserved()?;
        if (reserved.id as i32) > 0 {
            return Err("map event reserved route controller has a positive id");
        }
        false
    } else {
        ccountry::movement_enemy(memory, base, game_state, controller, owner)?
    };
    if enemy {
        province_id(memory, target)
    } else {
        Ok(current_id)
    }
}

pub fn unit_owner(memory: &impl Memory, unit: usize) -> Result<u32, &'static str> {
    field(
        memory,
        unit,
        cunit::ROSTER_OWNER,
        "map event unit owner unreadable",
    )
}

pub fn general_owner(memory: &impl Memory, leader: usize) -> Result<u32, &'static str> {
    field(
        memory,
        leader,
        UNIT_LEADER_COUNTRY,
        "map event general country unreadable",
    )
}

pub fn reference(memory: &impl Memory, reference: usize) -> Result<u32, &'static str> {
    field(
        memory,
        reference,
        0,
        "map event country reference unreadable",
    )
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Resistance {
    pub province: u32,
    pub controller: u32,
    pub overlord: u32,
}

pub fn resistance(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    resistance: usize,
) -> Result<Resistance, &'static str> {
    let state = pointer(
        memory,
        resistance,
        RESISTANCE_STATE,
        "map event resistance state unreadable",
    )?;
    let index = field::<i32>(
        memory,
        state,
        STATE_BEST_PROVINCE,
        "map event state best province unreadable",
    )?;
    let provinces = pointer(
        memory,
        state,
        cstate::PROVINCES,
        "map event state provinces unreadable",
    )?;
    let slot = usize::try_from(index.max(0))
        .ok()
        .and_then(|index| index.checked_mul(8))
        .ok_or("map event state best province overflow")?;
    let province = pointer(
        memory,
        provinces,
        slot,
        "map event state province unreadable",
    )?;
    let controller = field(
        memory,
        state,
        cstate::CONTROLLER,
        "map event state controller unreadable",
    )?;
    let owner = field::<u32>(
        memory,
        state,
        cstate::OWNER,
        "map event state owner unreadable",
    )?;
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let owner = if owner == 0 {
        countries.reserved()?
    } else {
        countries.resolve(owner)?
    };
    let diplomacy = pointer(
        memory,
        owner.pointer,
        COUNTRY_DIPLOMACY,
        "map event owner diplomacy unreadable",
    )?;
    Ok(Resistance {
        province: province_id(memory, province)?,
        controller,
        overlord: field(
            memory,
            diplomacy,
            DIPLOMACY_OVERLORD,
            "map event owner overlord unreadable",
        )?,
    })
}

pub fn string(memory: &impl Memory, at_: usize) -> Result<(usize, usize), &'static str> {
    use engine_native_memory::msvc_string::{CAPACITY, INLINE_CAPACITY, LENGTH, STRIDE};
    let header = read::<[u8; STRIDE]>(memory, at_).ok_or("map event string unreadable")?;
    let len = u64::from_le_bytes(header[LENGTH..CAPACITY].try_into().unwrap());
    let capacity = u64::from_le_bytes(header[CAPACITY..STRIDE].try_into().unwrap());
    if len > capacity || capacity < INLINE_CAPACITY {
        return Err("map event string header invalid");
    }
    let len = usize::try_from(len).map_err(|_| "map event string too long")?;
    let source = if capacity == INLINE_CAPACITY {
        at_
    } else {
        usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .map_err(|_| "map event string pointer invalid")?
    };
    if source == 0 {
        return Err("map event string pointer null");
    }
    Ok((source, len))
}

pub fn activity_key(memory: &impl Memory, activity: usize) -> Result<(usize, usize), &'static str> {
    if activity == 0 {
        return Err("map event activity absent");
    }
    string(memory, at(activity, ACTIVITY_MAP_TEXT)?)
}

pub fn level_health(
    memory: &impl Memory,
    building: usize,
    level: i16,
) -> Result<i64, &'static str> {
    let bytes = field::<[u8; 16]>(
        memory,
        building,
        BUILDING_LEVEL,
        "map event building health unreadable",
    )?;
    let total = u16::from_le_bytes(bytes[..2].try_into().unwrap());
    let healthy = i16::from_le_bytes(bytes[2..4].try_into().unwrap());
    let partial = i64::from_le_bytes(bytes[8..].try_into().unwrap());
    Ok(health_of(total, healthy, partial, level))
}

pub fn health_of(total: u16, healthy: i16, partial: i64, level: i16) -> i64 {
    if total == 0 {
        0
    } else if level <= healthy {
        LEVEL_HEALTH
    } else if i32::from(level) > i32::from(healthy) + 1 {
        0
    } else {
        partial
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Building {
    pub building_type: u32,
    pub province: u32,
    pub controller: u32,
}

pub fn building(
    memory: &impl Memory,
    game_state: usize,
    building: usize,
) -> Result<Building, &'static str> {
    let building_type = field(
        memory,
        building,
        BUILDING_TYPE,
        "map event building type unreadable",
    )?;
    let status = pointer(
        memory,
        building,
        BUILDING_STATUS,
        "map event building status unreadable",
    )?;
    let identity = field::<[u8; 8]>(
        memory,
        status,
        BUILDING_STATUS_FLAGS,
        "map event building location unreadable",
    )?;
    let flags = u32::from_le_bytes(identity[..4].try_into().unwrap());
    let id = u32::from_le_bytes(identity[4..].try_into().unwrap());
    let province = if flags & BUILDING_STATUS_STATE_FLAG != 0 {
        let states = Array::read(memory, at(game_state, cgame_state::STATES)?, 8)
            .ok_or("map event state root unreadable")?;
        let slot = states
            .element(id, 8)
            .filter(|_| id != 0)
            .ok_or("map event building state outside root")?;
        let state = pointer(memory, slot, 0, "map event building state unreadable")?;
        let provinces = pointer(
            memory,
            state,
            cstate::PROVINCES,
            "map event building state provinces unreadable",
        )?;
        pointer(
            memory,
            provinces,
            0,
            "map event building state province unreadable",
        )?
    } else {
        province_at(memory, provinces(memory, game_state)?, id)?
    };
    Ok(Building {
        building_type,
        province: province_id(memory, province)?,
        controller: field(
            memory,
            province,
            cprovince::CONTROLLER,
            "map event building controller unreadable",
        )?,
    })
}

#[cfg(test)]
#[path = "map_event_sites/tests.rs"]
mod tests;
