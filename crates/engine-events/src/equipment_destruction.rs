
use engine_native_memory::{Memory, cpdx_array::Array, read, walk::Outcome};
use engine_native_air::cair_wing;
use engine_native::{carmy, cequipment_variant::pool as cequipment_variant_pool};
use engine_native_parts::equipment_entry;
use engine_sites::Rva;

pub struct Site {
    pub id: SiteId,
    pub kind: SiteKind,
    pub name: &'static str,
    pub rva: Rva,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum SiteId {
    Land,
    Attrition,
    Air,
    Garrison,
    Delete,
    Return,
    DivisionFree,
    Queue,
    Paradrop,
    VolunteerTransfer,
    ExileTransfer,
    SetExiled,
    VolunteerReturnLoss,
    CapitulationEscapeLoss,
    ScriptedEquipmentFraction,
    ScriptedReserveEquipmentFraction,
    Merge,
    HqReturn,
}

impl SiteId {
    pub const fn is_supplemental(self) -> bool {
        !matches!(
            self,
            Self::Land | Self::Attrition | Self::Air | Self::Garrison
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SiteKind {
    Entry,
    Call { target: Rva },
}

pub const LAND: Site = Site {
    id: SiteId::Land,
    kind: SiteKind::Entry,
    name: "land_damage",
    rva: Rva(0xce84d0),
};
pub const ATTRITION: Site = Site {
    id: SiteId::Attrition,
    kind: SiteKind::Call {
        target: ATTRITION_TARGET,
    },
    name: "attrition_removal_tap",
    rva: Rva(0xccaf44),
};
pub const ATTRITION_TARGET: Rva = Rva(0x1098700);
pub const AIR: Site = Site {
    id: SiteId::Air,
    kind: SiteKind::Entry,
    name: "airplanes",
    rva: Rva(0xfe7a80),
};
pub const GARRISON: Site = Site {
    id: SiteId::Garrison,
    kind: SiteKind::Entry,
    name: "garrison_resistance",
    rva: Rva(0x107ad10),
};
pub const DELETE: Site = Site {
    id: SiteId::Delete,
    kind: SiteKind::Entry,
    name: "division_delete_scope",
    rva: Rva(0x1ecbe0),
};
pub const RETURN: Site = Site {
    id: SiteId::Return,
    kind: SiteKind::Call {
        target: RETURN_TARGET,
    },
    name: "division_equipment_return_tap",
    rva: Rva(0x1ed0e8),
};
pub const RETURN_TARGET: Rva = Rva(0xed1b60);
pub const DIVISION_FREE: Site = Site {
    id: SiteId::DivisionFree,
    kind: SiteKind::Entry,
    name: "division_equipment_discard",
    rva: Rva(0xccb4f0),
};
pub const QUEUE: Site = Site {
    id: SiteId::Queue,
    kind: SiteKind::Entry,
    name: "division_deletion_queue_scope",
    rva: Rva(0x1ed9b0),
};
pub const PARADROP: Site = Site {
    id: SiteId::Paradrop,
    kind: SiteKind::Entry,
    name: "paradrop_disruption",
    rva: Rva(0xce7310),
};
pub const VOLUNTEER_TRANSFER: Site = Site {
    id: SiteId::VolunteerTransfer,
    kind: SiteKind::Call {
        target: TRANSFER_TARGET,
    },
    name: "volunteer_roster_transfer_tap",
    rva: Rva(0x15be8c3),
};
pub const EXILE_TRANSFER: Site = Site {
    id: SiteId::ExileTransfer,
    kind: SiteKind::Call {
        target: TRANSFER_TARGET,
    },
    name: "exile_roster_transfer_tap",
    rva: Rva(0x15aeb53),
};
pub const TRANSFER_TARGET: Rva = Rva(0x7582c0);
pub const SET_EXILED: Site = Site {
    id: SiteId::SetExiled,
    kind: SiteKind::Entry,
    name: "exile_equipment",
    rva: Rva(0xceb540),
};
pub const SCALE_TARGET: Rva = Rva(0x1096e40);
pub const VOLUNTEER_RETURN_LOSS: Site = Site {
    id: SiteId::VolunteerReturnLoss,
    kind: SiteKind::Call {
        target: SCALE_TARGET,
    },
    name: "volunteer_return_equipment_tap",
    rva: Rva(0x15be600),
};
pub const CAPITULATION_ESCAPE_LOSS: Site = Site {
    id: SiteId::CapitulationEscapeLoss,
    kind: SiteKind::Call {
        target: SCALE_TARGET,
    },
    name: "capitulation_escape_equipment_tap",
    rva: Rva(0x72c3f8),
};
pub const SCRIPTED_EQUIPMENT_FRACTION: Site = Site {
    id: SiteId::ScriptedEquipmentFraction,
    kind: SiteKind::Call {
        target: SCALE_TARGET,
    },
    name: "scripted_division_equipment_fraction_tap",
    rva: Rva(0x4bade3),
};
pub const SCRIPTED_RESERVE_EQUIPMENT_FRACTION: Site = Site {
    id: SiteId::ScriptedReserveEquipmentFraction,
    kind: SiteKind::Call {
        target: SCALE_TARGET,
    },
    name: "scripted_reserve_equipment_fraction_tap",
    rva: Rva(0x4badb0),
};
pub const SITES: [&Site; 18] = [
    &LAND,
    &ATTRITION,
    &AIR,
    &GARRISON,
    &DELETE,
    &RETURN,
    &DIVISION_FREE,
    &QUEUE,
    &PARADROP,
    &VOLUNTEER_TRANSFER,
    &EXILE_TRANSFER,
    &SET_EXILED,
    &VOLUNTEER_RETURN_LOSS,
    &CAPITULATION_ESCAPE_LOSS,
    &SCRIPTED_EQUIPMENT_FRACTION,
    &SCRIPTED_RESERVE_EQUIPMENT_FRACTION,
    &MERGE,
    &HQ_RETURN,
];

pub const MERGE: Site = Site {
    id: SiteId::Merge,
    kind: SiteKind::Entry,
    name: "division_merge_pools",
    rva: Rva(0x13f9fb0),
};
pub const HQ_RETURN: Site = Site {
    id: SiteId::HqReturn,
    kind: SiteKind::Call {
        target: RETURN_TARGET,
    },
    name: "hq_equipment_return_tap",
    rva: Rva(0xc49bec),
};

fn merge_inputs(memory: &impl Memory, argument: usize) -> Result<(usize, Array), &'static str> {
    let at = argument
        .checked_add(8)
        .ok_or("merge input address overflow")?;
    Ok((
        at,
        Array::read(memory, at, 8).ok_or("merge input array unreadable")?,
    ))
}

pub fn merge_input_count(memory: &impl Memory, argument: usize) -> Result<usize, &'static str> {
    merge_inputs(memory, argument).map(|(_, array)| array.count as usize)
}

pub fn merge_armies(
    memory: &impl Memory,
    base: usize,
    argument: usize,
    output: &mut [usize],
) -> Result<(usize, usize), &'static str> {
    let survivor = pointer(memory, argument, 0)?;
    army_identity(memory, base, survivor)?;
    let location = pointer(memory, survivor, engine_native::cunit::LOCATION)?;
    let (array_address, array) = merge_inputs(memory, argument)?;
    let first = output.first_mut().ok_or("merge army scratch too short")?;
    *first = survivor;
    let mut len = 1;
    for index in 0..array.count {
        let army = usize::try_from(
            engine_native_memory::read::<u64>(
                memory,
                array
                    .element(index, 8)
                    .ok_or("merge input address overflow")?,
            )
            .ok_or("merge input army unreadable")?,
        )
        .map_err(|_| "merge input army pointer overflow")?;
        if pointer(memory, army, engine_native::cunit::LOCATION)? == location {
            army_identity(memory, base, army)?;
            *output.get_mut(len).ok_or("merge army scratch too short")? = army;
            len += 1;
        }
    }
    if Array::read(memory, array_address, 8) != Some(array)
        || pointer(memory, argument, 0)? != survivor
    {
        return Err("merge input changed during read");
    }
    output[..len].sort_unstable();
    let mut unique = 0;
    for index in 0..len {
        if unique == 0 || output[index] != output[unique - 1] {
            output[unique] = output[index];
            unique += 1;
        }
    }
    Ok((survivor, unique))
}

pub fn merge_reserves(
    memory: &impl Memory,
    base: usize,
    survivor: usize,
) -> Result<usize, &'static str> {
    let state = game_state(memory, base)?;
    let country = engine_native::ccountry::Reader::new(memory, base, state)?
        .resolve(merge_owner(memory, survivor)?)?;
    let manager = pointer(memory, country.pointer, engine_native::ccountry::PRODUCTION_STATUS)?;
    let pool = reserve_pool(manager)?;
    reserve_owner(memory, base, pool)?;
    Ok(pool)
}

pub fn canceled_delivery_pools(
    memory: &impl Memory,
    base: usize,
    army: usize,
    include_upgrades: bool,
    mut visit: impl FnMut(usize) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let requests = pointer(memory, army, carmy::REQUESTS)?;
    if requests == 0 {
        return Ok(());
    }
    if field::<u64>(memory, requests, 0)?
        != engine_sites::at(base, Rva(0x2a4bab0)).ok_or("merge requests vtable overflow")? as u64
        || pointer(memory, requests, 8)? != army
    {
        return Err("merge army requests identity mismatch");
    }
    for (offset, vtable, data, pool) in [
        (0x10, Rva(0x2a4b7f0), 0x28, 0x38),
        (
            crate::carmy_upgrades_requests::IN_ARMY_REQUESTS,
            crate::carmy_upgrades_requests::VTABLE_RVA,
            crate::carmy_upgrades_requests::DELIVERIES_DATA,
            crate::cupgrade_delivery::PRODUCED_EQUIPMENT_POOL,
        ),
    ] {
        if offset == crate::carmy_upgrades_requests::IN_ARMY_REQUESTS && !include_upgrades {
            continue;
        }
        let child = requests
            .checked_add(offset)
            .ok_or("merge requests child overflow")?;
        if field::<u64>(memory, child, 0)?
            != engine_sites::at(base, vtable).ok_or("merge requests vtable overflow")? as u64
            || pointer(memory, child, 0x20)? != army
        {
            return Err("merge delivery group identity mismatch");
        }
        let at = child
            .checked_add(data)
            .ok_or("merge deliveries address overflow")?;
        let array = Array::read(memory, at, 8).ok_or("merge deliveries array unreadable")?;
        for index in 0..array.count {
            let delivery = usize::try_from(
                engine_native_memory::read::<u64>(
                    memory,
                    array
                        .element(index, 8)
                        .ok_or("merge delivery address overflow")?,
                )
                .filter(|&value| value != 0)
                .ok_or("merge delivery pointer unreadable or null")?,
            )
            .map_err(|_| "merge delivery pointer overflow")?;
            visit(
                delivery
                    .checked_add(pool)
                    .ok_or("merge delivery pool overflow")?,
            )?;
        }
        if Array::read(memory, at, 8) != Some(array) {
            return Err("merge deliveries changed during read");
        }
    }
    if pointer(memory, requests, 8)? != army {
        return Err("merge requests changed army");
    }
    Ok(())
}

pub fn touched_reserves(
    memory: &impl Memory,
    base: usize,
    pool: usize,
    sources: &[equipment_entry::Row],
    positive_only: bool,
    output: &mut [equipment_entry::Row],
) -> Result<usize, &'static str> {
    touched_reserves_if(
        memory,
        base,
        pool,
        |variant| {
            let start = sources.partition_point(|row| row.variant_pointer < variant);
            sources[start..]
                .iter()
                .take_while(|row| row.variant_pointer == variant)
                .any(|row| !positive_only || row.amount_raw > 0)
        },
        output,
    )
}
pub fn touched_reserves_if(
    memory: &impl Memory,
    base: usize,
    pool: usize,
    selected: impl Fn(usize) -> bool,
    output: &mut [equipment_entry::Row],
) -> Result<usize, &'static str> {
    let array = cequipment_variant_pool::entries(memory, base, pool)?;
    let mut len = 0;
    for index in 0..array.count {
        let address = array
            .element(index, equipment_entry::STRIDE)
            .ok_or("reserve entry address overflow")?;
        let variant = usize::try_from(
            engine_native_memory::read::<u64>(memory, address).ok_or("reserve variant unreadable")?,
        )
        .map_err(|_| "reserve variant pointer overflow")?;
        if selected(variant) {
            let row = equipment_entry::read(memory, base, address, index)?;
            *output
                .get_mut(len)
                .ok_or("touched reserve scratch too short")? = row;
            len += 1;
        }
    }
    if cequipment_variant_pool::entries(memory, base, pool)? != array {
        return Err("reserve entries changed during read");
    }
    Ok(len)
}

pub use game_model::equipment_destruction::Cause;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Disposition {
    #[default]
    Loss,
    Returned,
    Merged,
    HqTransferred,
    VolunteerTransferred,
    ExileTransferred,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeletionContext {
    Ignore,
    Queue,
    Loss(Cause),
    Transfer(Disposition),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueuedContext {
    Ignore,
    Loss(Cause),
    Transfer(Disposition),
}

pub const COMBAT_DELETION_RETURN: Rva = Rva(0x147fa46);
pub const QUEUED_DELETE_RETURN: Rva = Rva(0x1ec994);
pub const READ_CONTEXT_RETURN: Rva = Rva(0x25414d5);
pub const DELETION_CAUSES: [(Rva, Cause); 15] = [
    (COMBAT_DELETION_RETURN, Cause::DivisionDestroyed),
    (Rva(0x36d5e0), Cause::DivisionScripted),
    (Rva(0x36dc5e), Cause::DivisionScripted),
    (Rva(0x584fd0), Cause::DivisionScripted),
    (Rva(0x755a10), Cause::DivisionTemplateRemoved),
    (Rva(0x15ae6a7), Cause::DivisionTemplateRemoved),
    (Rva(0x13fc152), Cause::DivisionDisbanded),
    (Rva(0x729854), Cause::DivisionCapitulation),
    (Rva(0x730be0), Cause::DivisionDeleted),
    (Rva(0x730cf0), Cause::DivisionDeleted),
    (Rva(0x75bbb8), Cause::DivisionDeleted),
    (Rva(0x15acc70), Cause::DivisionDeleted),
    (Rva(0x15bb8c1), Cause::DivisionDeleted),
    (Rva(0x72b0ae), Cause::DivisionDeleted),
    (Rva(0x72b1c1), Cause::DivisionDeleted),
];
pub const FAILED_CREATE_RETURN: Rva = Rva(0x36b420);
pub const IGNORED_DELETIONS: [Rva; 4] = [
    Rva(0x74f24a),
    Rva(0x74f4fc),
    READ_CONTEXT_RETURN,
    FAILED_CREATE_RETURN,
];
pub const MERGED_DELETION_RETURN: Rva = Rva(0x13fab3c);

const DELETION_KEY_COUNT: usize = 2 + IGNORED_DELETIONS.len() + DELETION_CAUSES.len();
const DELETION_KEYS: [Rva; DELETION_KEY_COUNT] = {
    let mut keys = [MERGED_DELETION_RETURN; DELETION_KEY_COUNT];
    keys[1] = QUEUED_DELETE_RETURN;
    let mut index = 0;
    while index < IGNORED_DELETIONS.len() {
        keys[2 + index] = IGNORED_DELETIONS[index];
        index += 1;
    }
    let mut index = 0;
    while index < DELETION_CAUSES.len() {
        keys[2 + IGNORED_DELETIONS.len() + index] = DELETION_CAUSES[index].0;
        index += 1;
    }
    keys
};

pub fn deletion_context(base: usize, return_address: usize) -> DeletionContext {
    match engine_sites::which(base, return_address, &DELETION_KEYS) {
        Some(key) if IGNORED_DELETIONS.contains(&key) => DeletionContext::Ignore,
        Some(MERGED_DELETION_RETURN) => DeletionContext::Transfer(Disposition::Merged),
        Some(QUEUED_DELETE_RETURN) => DeletionContext::Queue,
        key => DeletionContext::Loss(cause_of(&DELETION_CAUSES, key)),
    }
}

fn cause_of(causes: &[(Rva, Cause)], key: Option<Rva>) -> Cause {
    key.and_then(|key| {
        causes
            .iter()
            .find_map(|&(pc, cause)| (pc == key).then_some(cause))
    })
    .unwrap_or(Cause::Unknown)
}

pub const QUEUED_DELETION_CAUSES: [(Rva, Cause); 7] = [
    (Rva(0xce6806), Cause::DivisionOverrun),
    (Rva(0xce54a6), Cause::DivisionDeleted),
    (Rva(0xe65af8), Cause::DivisionDeleted),
    (Rva(0x7602ed), Cause::DivisionDeleted),
    (Rva(0x15bc7dd), Cause::DivisionDeleted),
    (Rva(0xa28424), Cause::DivisionDeleted),
    (Rva(0x35b6f6), Cause::DivisionScripted),
];
pub const HQ_PROVIDER_RETURN: Rva = Rva(0xc49d14);
pub const FAKE_INTEL_QUEUE_RETURN: Rva = Rva(0x134b10b);

const QUEUED_KEYS: [Rva; 2 + QUEUED_DELETION_CAUSES.len()] = {
    let mut keys = [HQ_PROVIDER_RETURN; 2 + QUEUED_DELETION_CAUSES.len()];
    keys[1] = FAKE_INTEL_QUEUE_RETURN;
    let mut index = 0;
    while index < QUEUED_DELETION_CAUSES.len() {
        keys[2 + index] = QUEUED_DELETION_CAUSES[index].0;
        index += 1;
    }
    keys
};

pub fn queued_context(base: usize, return_address: usize) -> QueuedContext {
    match engine_sites::which(base, return_address, &QUEUED_KEYS) {
        Some(HQ_PROVIDER_RETURN) => QueuedContext::Transfer(Disposition::HqTransferred),
        Some(FAKE_INTEL_QUEUE_RETURN) => QueuedContext::Ignore,
        key => QueuedContext::Loss(cause_of(&QUEUED_DELETION_CAUSES, key)),
    }
}

pub const RESETTING: usize = 0xa3a;

pub fn cleanup(memory: &impl Memory, base: usize) -> Result<bool, &'static str> {
    let state = game_state(memory, base)?;
    if state == 0 {
        return Err("destruction current game state is null");
    }
    match field::<u8>(memory, state, RESETTING)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err("unsupported destruction reset flag"),
    }
}

pub fn current_state(
    memory: &impl Memory,
    base: usize,
    receiver: usize,
) -> Result<(), &'static str> {
    if receiver == 0 || game_state(memory, base)? != receiver {
        return Err("destruction current game state identity mismatch");
    }
    Ok(())
}

pub fn is_army(memory: &impl Memory, base: usize, unit: usize) -> Result<bool, &'static str> {
    if unit == 0 {
        return Err("destruction unit is null");
    }
    if field::<u32>(memory, unit, engine_native::cunit::CLASS)? != 0 {
        return Ok(false);
    }
    army_identity(memory, base, unit).map(|_| true)
}

pub fn army_identity(memory: &impl Memory, base: usize, army: usize) -> Result<u64, &'static str> {
    let (Some(primary), Some(secondary)) = (
        engine_sites::at(base, carmy::VTABLE_RVA),
        engine_sites::at(base, carmy::SECONDARY_VTABLE_RVA),
    ) else {
        return Err("destruction division identity mismatch");
    };
    if army == 0
        || field::<u32>(memory, army, engine_native::cunit::CLASS)? != 0
        || field::<u64>(memory, army, 0)? != primary as u64
        || field::<u64>(memory, army, carmy::SECONDARY_BASE)? != secondary as u64
    {
        return Err("destruction division identity mismatch");
    }
    let handle = field::<u64>(memory, army, engine_native::cunit::HANDLE)?;
    if handle >> 32 == 0 {
        return Err("destruction division handle invalid");
    }
    Ok(handle)
}

pub const LAND_COMBAT_RETURN: Rva = Rva(0xce7bb8);
pub const NAVAL_TRANSPORT_RETURN: Rva = Rva(0xe96b00);
pub const IMMEDIATE_RETURN: Rva = Rva(0xcec745);

pub fn classification_sites(base: usize) -> Result<(), &'static str> {
    engine_sites::require(base, &DELETION_KEYS)?;
    engine_sites::require(base, &QUEUED_KEYS)?;
    engine_sites::require(base, &[LAND_COMBAT_RETURN, NAVAL_TRANSPORT_RETURN, IMMEDIATE_RETURN])?;
    engine_sites::require(base, &AIR_KEYS)
}

pub fn land_cause(base: usize, return_address: usize) -> Cause {
    match engine_sites::which(
        base,
        return_address,
        &[LAND_COMBAT_RETURN, NAVAL_TRANSPORT_RETURN, IMMEDIATE_RETURN],
    ) {
        Some(LAND_COMBAT_RETURN) => Cause::LandCombat,
        Some(NAVAL_TRANSPORT_RETURN) => Cause::NavalTransport,
        Some(IMMEDIATE_RETURN) => Cause::Immediate,
        _ => Cause::Unknown,
    }
}

pub const AIR_CAUSES: [(Rva, Cause); 11] = [
    (Rva(0x11215b3), Cause::AirNuclear),
    (Rva(0xfe23c2), Cause::AirAccident),
    (Rva(0xcb8301), Cause::AirStrategic),
    (Rva(0xcbc912), Cause::AirStrategic),
    (Rva(0xffaaa3), Cause::AirMissionAa),
    (Rva(0x1008036), Cause::AirMissionAa),
    (Rva(0x100b5b0), Cause::AirMissionAa),
    (Rva(0x100bc47), Cause::AirMissionAa),
    (Rva(0x133b533), Cause::AirLandCombatCas),
    (Rva(0x1502ff4), Cause::AirRaid),
    (Rva(0x1a0cd3c), Cause::AirNavalAa),
];

const AIR_KEYS: [Rva; AIR_CAUSES.len()] = {
    let mut keys = [AIR_CAUSES[0].0; AIR_CAUSES.len()];
    let mut index = 1;
    while index < AIR_CAUSES.len() {
        keys[index] = AIR_CAUSES[index].0;
        index += 1;
    }
    keys
};

pub fn air_cause(base: usize, return_address: usize) -> Cause {
    cause_of(
        &AIR_CAUSES,
        engine_sites::which(base, return_address, &AIR_KEYS),
    )
}

pub fn army_owner(memory: &impl Memory, army: usize) -> Result<u32, &'static str> {
    let expeditionary = field::<i32>(memory, army, engine_native::cunit::EXPEDITIONARY_OWNER)?;
    if expeditionary > 0 {
        Ok(expeditionary as u32)
    } else {
        field(memory, army, engine_native::cunit::ROSTER_OWNER)
    }
}

pub fn army_expeditionary_owner(memory: &impl Memory, army: usize) -> Result<u32, &'static str> {
    field(memory, army, engine_native::cunit::EXPEDITIONARY_OWNER)
}

pub fn fake_intel_division(memory: &impl Memory, army: usize) -> Result<bool, &'static str> {
    let template = pointer(memory, army, carmy::STANDARD_TEMPLATE)?;
    if template == 0 {
        Ok(false)
    } else {
        field::<u8>(
            memory,
            template,
            engine_native::creferenced_division_template::REVEAL_SKIP,
        )
        .map(|flag| flag != 0)
    }
}

pub fn air_owner(memory: &impl Memory, wing: usize) -> Result<u32, &'static str> {
    let owner = field::<i32>(memory, wing, 0x9c4)?;
    if owner > 0 {
        return Ok(owner as u32);
    }
    let base = pointer(memory, wing, 0x38)?;
    let status = pointer(memory, base, 0x40)?;
    field(memory, status, 0x90)
}

pub fn garrison_owner(memory: &impl Memory, occupation: usize) -> Result<u32, &'static str> {
    let state = pointer(memory, occupation, 0x50)?;
    field(memory, state, 0xcc)
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    let at = object
        .checked_add(offset)
        .ok_or("destruction operand address overflow")?;
    read(memory, at).ok_or("destruction owner/context unreadable")
}

fn pointer(memory: &impl Memory, object: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, object, offset)?)
        .map_err(|_| "destruction owner pointer overflow")
}

fn game_state(memory: &impl Memory, base: usize) -> Result<usize, &'static str> {
    let singleton = engine_sites::at(base, engine_native::cgame_state::SINGLETON_RVA)
        .ok_or("destruction operand address overflow")?;
    pointer(memory, singleton, 0)
}

pub fn pool(receiver: usize, site: SiteId) -> Result<usize, &'static str> {
    let offset = match site {
        SiteId::Air => cair_wing::EQUIPMENT_POOL,
        SiteId::Garrison => 0x90,
        _ => return Err("unsupported destruction writer"),
    };
    receiver
        .checked_add(offset)
        .ok_or("destruction pool address overflow")
}

pub fn reserve_pool(manager: usize) -> Result<usize, &'static str> {
    manager
        .checked_add(0x200)
        .ok_or("destruction reserve pool address overflow")
}

pub fn reserve_owner(memory: &impl Memory, base: usize, pool: usize) -> Result<u32, &'static str> {
    use engine_native::{ccountry, cproduction_status};
    let manager = pool
        .checked_sub(cproduction_status::STOCKPILE_POOL)
        .filter(|&manager| manager != 0)
        .ok_or("destruction reserve manager address invalid")?;
    let expected_manager = engine_sites::at(base, cproduction_status::VTABLE_RVA)
        .ok_or("destruction reserve manager vtable address overflow")?;
    if field::<u64>(memory, manager, 0)? != expected_manager as u64 {
        return Err("destruction reserve manager identity mismatch");
    }
    let country = pointer(memory, manager, cproduction_status::COUNTRY)?;
    let expected_country = engine_sites::at(base, ccountry::VTABLE_RVA)
        .ok_or("destruction reserve country vtable address overflow")?;
    if country == 0
        || field::<u64>(memory, country, 0)? != expected_country as u64
        || pointer(memory, country, ccountry::PRODUCTION_STATUS)? != manager
    {
        return Err("destruction reserve country identity mismatch");
    }
    field(memory, country, ccountry::ID)
}

pub fn merge_owner(memory: &impl Memory, army: usize) -> Result<u32, &'static str> {
    field(memory, army, engine_native::cunit::ROSTER_OWNER)
}


pub fn snapshot(
    memory: &impl Memory,
    base: usize,
    pool: usize,
    rows: &mut [equipment_entry::Row],
) -> Result<usize, &'static str> {
    snapshot_with_demand(memory, base, pool, rows, |_| ())
}
pub fn snapshot_with_demand(
    memory: &impl Memory,
    base: usize,
    pool: usize,
    rows: &mut [equipment_entry::Row],
    demand: impl FnOnce(usize),
) -> Result<usize, &'static str> {
    let array = cequipment_variant_pool::entries(memory, base, pool)?;
    demand(array.count as usize);
    if array.count as usize > rows.len() {
        return Err("destruction pool exceeds live design capacity");
    }
    match equipment_entry::walk(memory, base, array, rows) {
        Outcome::Rows { len } => {
            if cequipment_variant_pool::entries(memory, base, pool)? != array {
                return Err("destruction pool changed during snapshot");
            }
            Ok(len)
        }
        Outcome::Unavailable { reason } => Err(reason),
        _ => Err("destruction snapshot incomplete"),
    }
}

pub fn variant_amount(
    memory: &impl Memory,
    base: usize,
    pool: usize,
    variant: usize,
    scratch: &mut [u8],
) -> Result<i64, &'static str> {
    variant_amount_with_demand(memory, base, pool, variant, scratch, |_| ())
}
pub fn variant_amount_with_demand(
    memory: &impl Memory,
    base: usize,
    pool: usize,
    variant: usize,
    scratch: &mut [u8],
    demand: impl FnOnce(usize),
) -> Result<i64, &'static str> {
    let array = cequipment_variant_pool::entries(memory, base, pool)?;
    demand(array.count as usize);
    let bytes = (array.count as usize)
        .checked_mul(equipment_entry::STRIDE)
        .ok_or("destruction variant snapshot size overflow")?;
    let output = scratch
        .get_mut(..bytes)
        .ok_or("destruction pool exceeds live design capacity")?;
    if !output.is_empty() && !memory.copy(array.data, output) {
        return Err("destruction variant snapshot unreadable");
    }
    if cequipment_variant_pool::entries(memory, base, pool)? != array {
        return Err("destruction pool changed during variant read");
    }
    let mut amount = 0i64;
    for bytes in output.chunks_exact(equipment_entry::STRIDE) {
        if u64::from_le_bytes(bytes[..8].try_into().unwrap()) == variant as u64 {
            let quantity = i64::from_le_bytes(bytes[8..].try_into().unwrap());
            amount = amount
                .checked_add(quantity)
                .ok_or("destruction variant quantity overflow")?;
        }
    }
    Ok(amount)
}

pub fn removals(
    before: &mut [equipment_entry::Row],
    after: &mut [equipment_entry::Row],
    mut removed: impl FnMut(u64, i64),
) -> Result<(), &'static str> {
    before.sort_unstable_by_key(|r| r.variant_handle);
    after.sort_unstable_by_key(|r| r.variant_handle);
    let (mut i, mut j) = (0, 0);
    while i < before.len() || j < after.len() {
        let design = before
            .get(i)
            .map(|r| r.variant_handle)
            .unwrap_or(u64::MAX)
            .min(after.get(j).map(|r| r.variant_handle).unwrap_or(u64::MAX));
        let total =
            |rows: &[equipment_entry::Row], index: &mut usize| -> Result<i64, &'static str> {
                let mut value = 0i64;
                while let Some(row) = rows.get(*index).filter(|r| r.variant_handle == design) {
                    value = value
                        .checked_add(row.amount_raw)
                        .ok_or("destruction design quantity overflow")?;
                    *index += 1;
                }
                Ok(value)
            };
        let loss = total(before, &mut i)?
            .checked_sub(total(after, &mut j)?)
            .ok_or("destruction difference overflow")?;
        if loss != 0 {
            removed(design, loss);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
