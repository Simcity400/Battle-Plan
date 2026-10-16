//! The simulation edges behind the game's floating map text, for the
//! supported executable, and the operands each one needs.
//!
//! Each site is where the game decides an event, before the game's own
//! local-viewpoint filter, so one recording can show what any country would
//! have seen:
//!
//! - [`FAKE_REVEAL_VISIBILITY_CALL_RVA`]: a fake division is revealed. The site
//!   is the call that asks the local viewer's intel level, reached only after
//!   the game has refused a division already queued for deletion.
//! - [`RAILWAY_OVERRUN_RVA`]: a railway gun is overrun, at its entry, before
//!   the local intel test, the capture or the deletion.
//! - [`CRITICAL_BOMBARDMENT_TAP_RVA`]: a shore bombardment hits critically, on
//!   the rare critical branch after its random draw. Ordinary bombardment
//!   never reaches it.
//! - [`GENERAL_CAPTURE_CALL_RVA`]: a general is captured, at the one call that
//!   emits the capture notification.
//! - [`PARADROP_LANDING_CALL_RVA`]: a paradrop deployment completes, at the
//!   call that notifies the landing, before its friendship and fog tests.
//! - [`RESISTANCE_ACTIVITY_RVA`]: a selected resistance activity executes, at
//!   its entry, before its effects and its local test.
//! - [`BUILDING_NOTIFICATION_RVA`]: a building's health is notified, at the one
//!   notifier every damage, repair and destruction path shares, before its
//!   text and client tests.
//!
//! Readers copy plain values through guarded reads only; they never call the
//! game. A refused read refuses the whole event.
use crate::{Memory, ccountry, cgame_state, cpdx_array::Array, cprovince, cstate, cunit, read};

/// The call asking the local viewer's intel level of a revealed fake division
/// (`RCX`, the division), after the queued-for-deletion refusal.
pub const FAKE_REVEAL_VISIBILITY_CALL_RVA: usize = 0x012a_6060;
pub const FAKE_REVEAL_VISIBILITY_CALL_SENTINEL: [u8; 16] = [
    0xe8, 0x5b, 0x08, 0x36, 0x00, 0x3c, 0x96, 0x0f, 0x82, 0xf3, 0x00, 0x00, 0x00, 0x33, 0xd2, 0x48,
];
/// The overrun entry: `RCX` the gun, `RDX` its province, `R8` the province
/// it is overrun into, where the text stands.
pub const RAILWAY_OVERRUN_RVA: usize = 0x00e7_6e90;
pub const RAILWAY_OVERRUN_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x48, 0x89, 0x7c, 0x24, 0x18, 0x55,
];
/// The first instruction after the critical branch's random draw returns:
/// it loads the bombarded province's building list from `R13`, the province.
pub const CRITICAL_BOMBARDMENT_TAP_RVA: usize = 0x0129_6e2a;
pub const CRITICAL_BOMBARDMENT_TAP_SENTINEL: [u8; 16] = [
    0x49, 0x8b, 0x85, 0xe0, 0x01, 0x00, 0x00, 0x48, 0x8b, 0x34, 0xc8, 0x48, 0x8b, 0x0d, 0xac, 0x04,
];
/// The capture notification call: `RCX` the general's unit leader, `RDX` the
/// captor's country reference, `R8` the province (null shows no text).
pub const GENERAL_CAPTURE_CALL_RVA: usize = 0x00c0_2dd0;
pub const GENERAL_CAPTURE_CALL_SENTINEL: [u8; 16] = [
    0xe8, 0x1b, 0x11, 0x01, 0x00, 0x4c, 0x8b, 0xac, 0x24, 0x38, 0x01, 0x00, 0x00, 0x48, 0x81, 0xc4,
];
/// The landing notification call: `RCX` the province, `RDX` the dropping
/// country's reference, `R8B` whether the landing took losses.
pub const PARADROP_LANDING_CALL_RVA: usize = 0x00f6_f87d;
pub const PARADROP_LANDING_CALL_SENTINEL: [u8; 16] = [
    0xe8, 0xee, 0xac, 0xef, 0xff, 0x49, 0x8b, 0x4f, 0x28, 0xe8, 0x05, 0x1f, 0xfe, 0xff, 0x49, 0x8b,
];
/// The activity execution entry: `RCX` the state's resistance, `RDX` the
/// activity definition.
pub const RESISTANCE_ACTIVITY_RVA: usize = 0x00f8_31c0;
pub const RESISTANCE_ACTIVITY_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x55,
];
/// The building health notifier: `RCX` the building, `RDX` the change's
/// source (its first dword is the reason), `R8W` the level, `R9` that level's
/// health before the change, then a byte that enables the client branch and a
/// byte that enables its text.
pub const BUILDING_NOTIFICATION_RVA: usize = 0x010c_69f0;
pub const BUILDING_NOTIFICATION_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48,
];

/// The game's fog of war switch, a byte: zero shows every province.
pub const FOG_OF_WAR_RVA: usize = 0x0331_726a;

/// A unit's intel array (the unit viewer entries): 24-byte entries holding
/// a canonical country ordinal and, at [`INTEL_LEVEL`], a signed level.
pub const INTEL_STRIDE: usize = 0x18;
pub const INTEL_LEVEL: usize = 0x10;

/// A unit leader's raw country reference.
pub const UNIT_LEADER_COUNTRY: usize = 0x120;
/// A resistance object's state.
pub const RESISTANCE_STATE: usize = 0x08;
/// An activity definition's authored map text key, a native string.
pub const ACTIVITY_MAP_TEXT: usize = 0x370;
/// A state's best victory-point province index; negative selects the first.
pub const STATE_BEST_PROVINCE: usize = 0x85c;
/// A country's diplomacy block, holding its overlord's raw reference.
pub const COUNTRY_DIPLOMACY: usize = 0xf88;
pub const DIPLOMACY_OVERLORD: usize = 0x188;

/// A building's health per level and the notifier's reason source.
pub const BUILDING_LEVEL: usize = 0x40;
pub const BUILDING_HEALTHY_LEVEL: usize = 0x42;
pub const BUILDING_PARTIAL_HEALTH: usize = 0x48;
pub const BUILDING_TYPE: usize = 0x08;
pub const BUILDING_STATUS: usize = 0x1d8;
pub const BUILDING_STATUS_FLAGS: usize = 0x68;
pub const BUILDING_STATUS_ID: usize = 0x6c;
pub const BUILDING_STATUS_STATE_FLAG: u32 = 2;
/// The health of one full building level.
pub const LEVEL_HEALTH: i64 = 10_000_000;

fn at(object: usize, offset: usize) -> Result<usize, &'static str> {
    object
        .checked_add(offset)
        .ok_or("map event address overflow")
}

fn field<T: crate::memory::Raw>(
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

/// The game state singleton.
pub fn game_state(memory: &impl Memory, base: usize) -> Result<usize, &'static str> {
    let state = pointer(
        memory,
        base,
        cgame_state::SINGLETON_RVA,
        "map event game state unreadable",
    )?;
    if state == 0 {
        return Err("map event game state absent");
    }
    Ok(state)
}

/// The game hour the event happens at.
pub fn hour(memory: &impl Memory, game_state: usize) -> Result<i32, &'static str> {
    field(
        memory,
        game_state,
        cgame_state::CURRENT_HOUR,
        "map event hour unreadable",
    )
}

/// Whether fog of war is on.
pub fn fog(memory: &impl Memory, base: usize) -> Result<bool, &'static str> {
    field::<u8>(
        memory,
        base,
        FOG_OF_WAR_RVA,
        "map event fog switch unreadable",
    )
    .map(|fog| fog != 0)
}

/// A province's id.
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

/// A unit's intel array.
pub fn intel(memory: &impl Memory, unit: usize) -> Result<Array, &'static str> {
    Array::read(memory, at(unit, cunit::VIEWER_DATA)?, INTEL_STRIDE)
        .ok_or("map event intel array unreadable")
}

/// One intel entry: (canonical country ordinal, signed level).
pub type IntelEntry = (u32, i8);

/// Entries `start..start + output.len()` of `intel`, in native order.
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
        if !crate::read_slice(memory, address, bytes) {
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

/// The province the game reads a viewer's visibility of a unit at, and the
/// province a fake division's text stands on: the first province of its
/// route when the division is embarking from land onto a sea province that
/// an enemy of its owner controls, otherwise its current province. Only a
/// division (class 0) can take the route province.
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
            engine_native::province_definition::TERRAIN_FLAGS,
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
    // A zero controller reaches the reserved country, which is at war with
    // nobody while its id is not positive.
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

/// A unit's raw owner reference.
pub fn unit_owner(memory: &impl Memory, unit: usize) -> Result<u32, &'static str> {
    field(
        memory,
        unit,
        cunit::ROSTER_OWNER,
        "map event unit owner unreadable",
    )
}

/// A general's raw country reference.
pub fn general_owner(memory: &impl Memory, leader: usize) -> Result<u32, &'static str> {
    field(
        memory,
        leader,
        UNIT_LEADER_COUNTRY,
        "map event general country unreadable",
    )
}

/// A country reference's raw word.
pub fn reference(memory: &impl Memory, reference: usize) -> Result<u32, &'static str> {
    field(
        memory,
        reference,
        0,
        "map event country reference unreadable",
    )
}

/// What a resistance activity's text needs from its state.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Resistance {
    pub province: u32,
    pub controller: u32,
    pub overlord: u32,
}

/// The state's best victory-point province, its controller and its owner's
/// overlord.
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

/// A native string's length and bytes address, for copying an authored key.
pub fn string(memory: &impl Memory, at_: usize) -> Result<(usize, usize), &'static str> {
    use crate::msvc_string::{CAPACITY, INLINE_CAPACITY, LENGTH, STRIDE};
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

/// The activity's authored map text key.
pub fn activity_key(memory: &impl Memory, activity: usize) -> Result<(usize, usize), &'static str> {
    if activity == 0 {
        return Err("map event activity absent");
    }
    string(memory, at(activity, ACTIVITY_MAP_TEXT)?)
}

/// One building level's health, as the game computes it: a full level below
/// the healthy level, the partial health at the next level, none above.
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

/// [`level_health`]'s arithmetic on the copied fields.
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

/// What a building notification's text needs.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Building {
    pub building_type: u32,
    pub province: u32,
    pub controller: u32,
}

/// The building's type token and the province its text stands on with that
/// province's controller: a state building's state's first province,
/// otherwise its own province.
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
