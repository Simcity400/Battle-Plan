//! Applied building damage for hash-pinned a729, without calling the game.
//!
//! `CBuilding::ApplyDamage@1410C6F60` is the sole caller of the health writer
//! `ApplyDamageStep@1410C7000`. The instruction index finds ten calls in seven
//! functions, listed in [`CALLERS`]. Three air-mission paths (logistics train
//! F60880, logistics building F612F0 and strategic F6F8D0) reach the helper
//! F663C0; naval air/port damage 1419681E0 reaches ApplyDamage independently.
//! Land collateral, weekly sabotage, script damage and facility dismantling
//! use the same writer. This ledger retains these causes with null attackers.
//! This is indexed direct-call coverage, not a claim that the call graph alone
//! proves absence of every indirect call; unmatched callers stay Unknown.
//! CBuilding constructor 1410C5800 stores status/template at +1D8/+1E0 and
//! copies template+8 into its own +8 lexer token. Native lookup 141160CD0
//! uses template+2E0 as the index into status+20, then status+38's instance
//! vector. Both live mappings authenticate the token; no building names,
//! template ordinals, state counts or province counts are assumed.
//!
//! F663C0 constructs source kind 1 and its wing at +8; 1419681E0 constructs
//! the same source. Land 141296CC0 uses kind 3, weekly 1409D3B60 kind 2, and
//! script 14030E5B0 / facility 141430D60 / abort 14142FC10 kind 0. Only kind
//! 1's payload is followed. Unknown callers are classified Unknown; unknown
//! kinds remain raw with a null attacker. A null wing also remains a null
//! attacker. Attribution-only faults retain null attackers; location/health faults refuse.
//!
//! Air credit's victim is the location controller: F663C0 writes province+188
//! or state+CC to its victim output after applying damage. Status+70's explicit
//! country selects repair/notification routing (141162010), not this credit.
//! A controller word of zero is retained: F663C0 explicitly handles its zero
//! case, and country resolvers 140BA5140 / 140BA56A0 map it to reserved slot 0.
//! Consumers must treat it as known absence of a country, without a phantom
//! CountryLinks join or omission of the damage event.
//! GetEffectiveCountryTag F45950 selects a positive wing+9C4, otherwise
//! wing+38 -> pool+40 -> strategic-air+90. References remain opaque raw words.
//!
//! True applied damage is the before/after difference of healthy*10,000,000
//! plus partial health, as get_level_health 1410C64D0 establishes. Step
//! 1410C7000 can multiply an air request by a building modifier and round it
//! before reducing health; ApplyDamage's returned budget consumption can
//! therefore differ. Consumers must retain it separately from this difference.
//! Raw health divided by 10,000,000 displays full building-level equivalents;
//! divided by 100,000 it displays the native fixed5 health amount.
//! No total-level write occurs in either damage function. Serializer 1410C7960
//! saves total/healthy/partial health; it saves no event attribution or ledger.
//! Normal hourly damage runs after the air TBB joins (140C485F0 / 140C48E42).
//! Tick command 140EF1A60 calls 1401DCEF0 synchronously; session 14223DDC0
//! executes command slot10 serially and refuses recursive drains. Scorch
//! 1411487C0 uses that same slot. Daily repair follows hourly work through
//! 140E527A0 -> 140F55590 -> 1410C5C90; nuclear 1410831B0 -> 141083430 ->
//! 141082C90 / 1410C6D20 runs in serial pending refresh 1406F8AC0. Nuclear
//! and scorch bypass ApplyDamage, so this ledger is not all health reductions.
//! Damage's repair queue 140E4B5D0 -> 140E4AFC0 passes bRepair=1, excluding
//! instant set_level. Its status notifier 141163C10 dispatches the indexed
//! building listeners 140E56010 (dirty flag), 140C4BFE0 (aircraft losses) and
//! 140E95660 (ship losses), with no direct health/controller write. Naval
//! listener 140E95660 routes by province controller+188; unlike F663C0,
//! naval caller 1419681E0 has no separate building victim-credit output.
//! These are normal-path scheduling facts. Complete exclusion of console
//! repair, arbitrary script/setter mutations and transitive ship callbacks
//! is unproved. Same-building damage claims cannot detect unhooked setters;
//! changed attribution/total levels are refused, but those
//! guards alone do not establish health isolation. Debug/init paths and
//! recovered native exceptions remain outside the live-validated evidence.
//! Synthetic tests exercise layouts and refusals; live cost is unverified.

use crate::{
    Memory, cair_wing, cair_wing_pool, cbuilding, cbuilding_status, cbuilding_template,
    cgame_state, cpdx_array::Array, cprovince, cstate, cstrategic_air, read,
};

/// ApplyDamage entry, a729. The five-byte relocated instruction only stores
/// RBX in caller home space; it changes neither RSP nor nonvolatile registers.
pub const APPLY_DAMAGE_RVA: usize = 0x010c_6f60;
pub const APPLY_DAMAGE_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x20, 0x56, 0x57, 0x41, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0xf9,
];

/// Attribution of the pinned native caller, independently of source kind.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Cause {
    #[default]
    Unknown = 0,
    AirMission = 1,
    AirNaval = 2,
    LandCombat = 3,
    Scripted = 4,
    Weekly = 5,
    ProgramTeardown = 6,
    AbortDismantle = 7,
}

/// Exact return RVAs after every indexed direct CALL to ApplyDamage. The
/// original wrapper must retain its incoming return address before calling
/// the trampoline; the trampoline's own return address is not a caller.
pub const CALLERS: [(usize, Cause); 10] = [
    (0x00f6_6bbb, Cause::AirMission),
    (0x0196_8fab, Cause::AirNaval),
    (0x0129_6fd7, Cause::LandCombat),
    (0x0129_717c, Cause::LandCombat),
    (0x0129_7244, Cause::LandCombat),
    (0x0129_7347, Cause::LandCombat),
    (0x0030_e62e, Cause::Scripted),
    (0x009d_3fd1, Cause::Weekly),
    (0x0143_0f11, Cause::ProgramTeardown),
    (0x0142_fd37, Cause::AbortDismantle),
];

/// Native source kind; F663C0@140F66B86 and naval@141968F5B store 1.
pub const AIR_SOURCE_KIND: u32 = 1;
/// Wing payload offset: F663C0@140F66B91 and naval@141968F66 store here.
pub const SOURCE_WING: usize = 8;

/// Callback-local owned values. Internal pointers authenticate the post-read;
/// they must never leave the native call's lifetime or become replay fields.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct Snapshot {
    pub building: usize,
    pub health_raw: i64,
    pub attacker: Option<u32>,
    /// A failed wing read is distinct from native no-wing/source-kind absence.
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

fn field<T: crate::memory::Raw>(
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
    vtable: usize,
    reason: &'static str,
) -> Result<(), &'static str> {
    let expected = base.checked_add(vtable).ok_or("module address overflow")?;
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
    // The native signed-short levels and qword partial span +40..+50 exactly.
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
    // ApplyDamageStep@1410C7000 does not clamp a negative modified request:
    // partial health can exceed a full level, including the total level health.
    let value = (i64::from(healthy) * scale)
        .checked_add(partial)
        .ok_or("building damage health overflow")?;
    Ok((total, value))
}

/// Cold capacity census over the current state's and province's live building
/// headers, without following each instance or allocating. The locations,
/// embedded statuses and scope/id backlinks must all authenticate. A missing
/// root or failed read is None, never zero. This measures current population,
/// not a bound on event writes; a collector still counts queue overflow.
pub fn building_count(memory: &impl Memory, base: usize) -> Option<usize> {
    let game_state = pointer(
        memory,
        base,
        cgame_state::SINGLETON_RVA,
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
        cair_wing::VTABLE_RVA,
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
        cair_wing::POOL,
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

/// Pre reads only building identity, health, and source kind. Location,
/// content-index authentication and country attribution run only after damage.
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
    // ApplyDamage's exhausted-health gate precedes source reads (`1410C6FA0..AF`).
    let native_cause = if source == 0 || health_raw == 0 {
        0
    } else {
        field(memory, source, 0, "building damage source kind unreadable")?
    };
    let cause = return_address
        .checked_sub(base)
        .and_then(|rva| CALLERS.iter().find(|(caller, _)| *caller == rva))
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

/// Air credit reads the controller after ApplyDamage (`140F66BE2..BF7`).
/// Attribution failures retain a null attacker; identity/location failures refuse.
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

/// Health and identity are always checked; callers defer attribution until
/// this returns a nonzero amount. A no-op follows no location/wing pointers.
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
