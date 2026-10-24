//! Cause-specific equipment destruction observations. Pool differences are
//! signed fixed5 net removals; a negative row is a gain. Logging, orders-group
//! and player predicates do not select these observations.
//!
//! Combat damage scales and rounds live division pools; aircraft loss removes
//! wing equipment. Resistance computes its pool difference before optional AI
//! logging. Deletion preserves ownership before expeditionary ownership clears,
//! and nested writers retain that country through the final combat flush.
//! Returned, merged and HQ-transferred pools produce deletion observations
//! without loss amounts. Runtime reset brackets queue and country cleanup;
//! parsed and failed-create cleanup supplies no live deletion observation.
//! Deception armies follow the census reveal-skip flag regardless of pool contents.
//!
//! Paradrop divides and floors the live pool. Combat flush precedes lifecycle
//! cleanup, and combat unlink finishes before the deleting destructor. The final
//! snapshot excludes earlier combat, attrition and immediate removals. Defeated
//! equipment capture credits winners from scaled copies without subtracting
//! the victim pool. Merge conserves selected main pools, canceled deliveries
//! and reserves across retained allocations and actual credits; transferred
//! bookkeeping is not counted twice at source disposal.
//!
//! Volunteer and exile transfers remove armies from rosters while retaining
//! full handles in transfer objects. Zero-amount transfer observations cover
//! that absence until reappearance or deletion. Exile scaling and integral
//! rounding are one observation because rounding can add equipment. Volunteer
//! return, capitulation escape and scripted fractions have separate exact taps,
//! including stockpiles. Signed damage, stochastic ceiling and signed entries
//! can increase pools; script factors remain unclamped.

use crate::{
    Memory, cair_wing, carmy, cequipment_variant_pool, cpdx_array::Array, equipment_entry, read,
    walk::Outcome,
};

pub struct Site {
    pub id: SiteId,
    pub kind: SiteKind,
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
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
    Call { target: usize },
}

/// Entry ABI: eight integer slots, RCX army; void. The four stack slots
/// and the full register slots must survive the caller-address wrapper.
pub const LAND: Site = Site {
    id: SiteId::Land,
    kind: SiteKind::Entry,
    name: "land_damage",
    rva: 0xc8b710,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x20, 0x44, 0x88, 0x44, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54,
        0x41,
    ],
};
/// Replace only the five-byte removal CALL with another CALL. RCX is the
/// division equipment pool, RDX the variant, R8 the positive fixed5 removal;
/// returns void. RSP is 16-byte aligned with home space. No branch targets or
/// RIP-relative operands lie in the patch span. Observation follows the
/// inactive-consumption, template and nonpositive-loss-roll gates. Invoke the
/// verified original PC-relative target directly; the continuation is not a function.
pub const ATTRITION: Site = Site {
    id: SiteId::Attrition,
    kind: SiteKind::Call {
        target: ATTRITION_TARGET,
    },
    name: "attrition_removal_tap",
    rva: 0xc6e204,
    sentinel: [
        0xe8, 0x47, 0x46, 0x3a, 0x00, 0x48, 0x8b, 0x47, 0x10, 0x48, 0x8b, 0x08, 0x48, 0x85, 0xc9,
        0x74,
    ],
};
pub const ATTRITION_TARGET: usize = 0x1012850;
/// Win64 RCX wing, EDX signed requested count, R8D source country; void.
pub const AIR: Site = Site {
    id: SiteId::Air,
    kind: SiteKind::Entry,
    name: "airplanes",
    rva: 0xf63580,
    sentinel: [
        0x48, 0x8b, 0xc4, 0x44, 0x89, 0x40, 0x18, 0x53, 0x41, 0x55, 0x41, 0x57, 0x48, 0x83, 0xec,
        0x70,
    ],
};
/// Win64 RCX occupation entry, RDX raw fixed-point damage, R8 source; void.
pub const GARRISON: Site = Site {
    id: SiteId::Garrison,
    kind: SiteKind::Entry,
    name: "garrison_resistance",
    rva: 0xff4d30,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54,
        0x41,
    ],
};
/// RCX game state, RDX unit, R8B return request; returns void. R9 is unused
/// but its full slot must be preserved.
pub const DELETE: Site = Site {
    id: SiteId::Delete,
    kind: SiteKind::Entry,
    name: "division_delete_scope",
    rva: 0x1d6690,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54,
        0x41,
    ],
};
/// Replace only the five-byte reserve-return CALL with another CALL. RCX is
/// reserve manager, RDX the division equipment pool; returns void. Reached only
/// for requested return outside naval transfer with a home-area or supply-port
/// connection. No branch targets lie within the patch span.
pub const RETURN: Site = Site {
    id: SiteId::Return,
    kind: SiteKind::Call {
        target: RETURN_TARGET,
    },
    name: "division_equipment_return_tap",
    rva: 0x1d6b98,
    sentinel: [
        0xe8, 0xb3, 0xa2, 0xc8, 0x00, 0x48, 0x8b, 0xd3, 0xeb, 0xc7, 0x48, 0x0f, 0xaf, 0x1d, 0xae,
        0xbf,
    ],
};
pub const RETURN_TARGET: usize = 0xe60e50;
/// RCX primary division, EDX deleting flags; returns the original pointer.
/// The destructor clears and frees the live equipment pool. Secondary-base
/// thunks tail-jump here and retain the original caller return address.
pub const DIVISION_FREE: Site = Site {
    id: SiteId::DivisionFree,
    kind: SiteKind::Entry,
    name: "division_equipment_discard",
    rva: 0xc6e7c0,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x8b, 0xda, 0x48, 0x8b, 0xf9,
        0xe8,
    ],
};
/// RCX game state, RDX unit; returns void. Records deletion cause only; the
/// residual is copied at DIVISION_FREE after intervening writers.
pub const QUEUE: Site = Site {
    id: SiteId::Queue,
    kind: SiteKind::Entry,
    name: "division_deletion_queue_scope",
    rva: 0x1d7460,
    sentinel: [
        0x4c, 0x8b, 0xdc, 0x49, 0x89, 0x5b, 0x08, 0x49, 0x89, 0x6b, 0x18, 0x49, 0x89, 0x73, 0x20,
        0x57,
    ],
};
/// Five integer slots: division, source-country pointer, equipment divisor, manpower divisor and
/// organization divisor. The fifth is in its stack argument slot. Division and flooring finish
/// before optional logging.
pub const PARADROP: Site = Site {
    id: SiteId::Paradrop,
    kind: SiteKind::Entry,
    name: "paradrop_disruption",
    rva: 0xc8a550,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x57, 0x41, 0x54, 0x41,
        0x56,
    ],
};
/// Volunteer roster-removal CALL. RCX roster country, RDX primary division;
/// retains the original integer result. The transfer keeps the complete handle
/// and serializes its held divisions separately; roster removal does not delete the pool.
pub const VOLUNTEER_TRANSFER: Site = Site {
    id: SiteId::VolunteerTransfer,
    kind: SiteKind::Call {
        target: TRANSFER_TARGET,
    },
    name: "volunteer_roster_transfer_tap",
    rva: 0x1523333,
    sentinel: [
        0xe8, 0x08, 0xce, 0x1e, 0xff, 0x49, 0x8b, 0xcf, 0xe8, 0x70, 0xca, 0x1d, 0xff, 0x84, 0xc0,
        0x74,
    ],
};
/// Exile roster-removal CALL; same ABI as the volunteer tap. Live armies and
/// load-restored transfer-held armies remain separate from the country roster.
/// Transfer serialization retains every held division.
pub const EXILE_TRANSFER: Site = Site {
    id: SiteId::ExileTransfer,
    kind: SiteKind::Call {
        target: TRANSFER_TARGET,
    },
    name: "exile_roster_transfer_tap",
    rva: 0x15138e3,
    sentinel: [
        0xe8, 0x58, 0xc8, 0x1f, 0xff, 0x49, 0x8b, 0xce, 0xe8, 0xc0, 0xc4, 0x1e, 0xff, 0x84, 0xc0,
        0x74,
    ],
};
pub const TRANSFER_TARGET: usize = 0x710140;
/// RCX primary division, EDX boolean; returns void. Observe scaling and
/// integral rounding together so rounding gains remain in the exact net loss.
/// Other work clears the movement target, rebuilds abilities and cancels
/// movement while preserving active combat. The operation is virtual; unit
/// access updates reach it from division and railway hourly updates. Those
/// observed routes do not establish a complete loader or reset exclusion.
pub const SET_EXILED: Site = Site {
    id: SiteId::SetExiled,
    kind: SiteKind::Entry,
    name: "exile_equipment",
    rva: 0xc8e770,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57, 0x48, 0x81, 0xec, 0x70,
        0x01,
    ],
};
/// Scale taps return void, with RCX pool and RDX fixed5 factor. Replace only
/// the five-byte CALL; scaling does not call other pool writers.
pub const SCALE_TARGET: usize = 0x1010f70;
/// Returning volunteers scale their live pool with the content-loaded return
/// ratio. The removed difference is logged as loss without a source-reserve
/// credit. Surviving armies and pools return to the roster. The stored transfer
/// direction distinguishes sends from explicit and automatic recalls.
pub const VOLUNTEER_RETURN_LOSS: Site = Site {
    id: SiteId::VolunteerReturnLoss,
    kind: SiteKind::Call {
        target: SCALE_TARGET,
    },
    name: "volunteer_return_equipment_tap",
    rva: 0x1523070,
    sentinel: [
        0xe8, 0xfb, 0xde, 0xae, 0xff, 0x48, 0x8d, 0x96, 0x48, 0x03, 0x00, 0x00, 0x48, 0x8d, 0x4c,
        0x24,
    ],
};
/// Capitulation escape scales selected live armies with a content-loaded ratio
/// and applies selection rounding. Observe the actual pool result rather than the factor.
pub const CAPITULATION_ESCAPE_LOSS: Site = Site {
    id: SiteId::CapitulationEscapeLoss,
    kind: SiteKind::Call {
        target: SCALE_TARGET,
    },
    name: "capitulation_escape_equipment_tap",
    rva: 0x6e4878,
    sentinel: [
        0xe8, 0xf3, 0xc6, 0x92, 0x00, 0x48, 0x8d, 0x55, 0xf7, 0x48, 0x8d, 0x0d, 0x28, 0x18, 0xc5,
        0x02,
    ],
};
/// Scripted equipment fractions scale every roster division. Factors remain
/// unclamped; values above one or below zero retain signed results.
pub const SCRIPTED_EQUIPMENT_FRACTION: Site = Site {
    id: SiteId::ScriptedEquipmentFraction,
    kind: SiteKind::Call {
        target: SCALE_TARGET,
    },
    name: "scripted_division_equipment_fraction_tap",
    rva: 0x49d023,
    sentinel: [
        0xe8, 0x48, 0x3f, 0xb7, 0x00, 0x48, 0xff, 0xc3, 0x48, 0x3b, 0xde, 0x7c, 0xe0, 0x48, 0x8b,
        0x5c,
    ],
};
/// Scripted fractions scale the national stockpile before division pools.
/// Authenticate the manager's country backlink. Aircraft and ships can occur
/// in this pool, so resolve each design's type through recorded content.
pub const SCRIPTED_RESERVE_EQUIPMENT_FRACTION: Site = Site {
    id: SiteId::ScriptedReserveEquipmentFraction,
    kind: SiteKind::Call {
        target: SCALE_TARGET,
    },
    name: "scripted_reserve_equipment_fraction_tap",
    rva: 0x49cff0,
    sentinel: [
        0xe8, 0x7b, 0x3f, 0xb7, 0x00, 0x48, 0x8b, 0xcf, 0xe8, 0xf3, 0x20, 0x23, 0x00, 0x48, 0x8b,
        0xf8,
    ],
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

/// Merge receiver points to the survivor followed by the candidate array.
/// The complete operation clears signed pools and saturates retained allocations.
pub const MERGE: Site = Site {
    id: SiteId::Merge,
    kind: SiteKind::Entry,
    name: "division_merge_pools",
    rva: 0x1363da0,
    sentinel: [
        0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
        0x41,
    ],
};
/// Five-byte HQ-provider reserve-return CALL, before the same full division
/// handle is queued for deletion.
pub const HQ_RETURN: Site = Site {
    id: SiteId::HqReturn,
    kind: SiteKind::Call {
        target: RETURN_TARGET,
    },
    name: "hq_equipment_return_tap",
    rva: 0xbf41ac,
    sentinel: [
        0xe8, 0x9f, 0xcc, 0x26, 0x00, 0xba, 0xa0, 0x86, 0x01, 0x00, 0x48, 0x8d, 0x8b, 0xd0, 0x03,
        0x00,
    ],
};

/// Selected pools have matching location pointers. Read each physical pool
/// once, even when the input repeats a pointer.
pub fn merge_armies(
    memory: &impl Memory,
    base: usize,
    argument: usize,
    output: &mut [usize],
) -> Result<(usize, usize), &'static str> {
    let survivor = pointer(memory, argument, 0)?;
    army_identity(memory, base, survivor)?;
    let location = pointer(memory, survivor, crate::cunit::LOCATION)?;
    let array_address = argument
        .checked_add(8)
        .ok_or("merge input address overflow")?;
    let array = Array::read(memory, array_address, 8).ok_or("merge input array unreadable")?;
    let first = output.first_mut().ok_or("merge army scratch too short")?;
    *first = survivor;
    let mut len = 1;
    for index in 0..array.count {
        let army = usize::try_from(
            crate::read::<u64>(
                memory,
                array
                    .element(index, 8)
                    .ok_or("merge input address overflow")?,
            )
            .ok_or("merge input army unreadable")?,
        )
        .map_err(|_| "merge input army pointer overflow")?;
        if pointer(memory, army, crate::cunit::LOCATION)? == location {
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
    let state = pointer(memory, base, crate::cgame_state::SINGLETON_RVA)?;
    let country = crate::ccountry::Reader::new(memory, base, state)?
        .resolve(merge_owner(memory, survivor)?)?;
    let manager = pointer(memory, country.pointer, crate::ccountry::PRODUCTION_STATUS)?;
    let pool = reserve_pool(manager)?;
    reserve_owner(memory, base, pool)?;
    Ok(pool)
}

/// Merge returns canceled survivor reinforcement deliveries and source upgrade
/// deliveries. Survivor upgrades remain live. All statuses and signed or zero
/// entries reach reserve credit; no progress getter is called.
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
    // Parent requests point to their division; children retain the owning division.
    if field::<u64>(memory, requests, 0)?
        != base
            .checked_add(0x29d2080)
            .ok_or("merge requests vtable overflow")? as u64
        || pointer(memory, requests, 8)? != army
    {
        return Err("merge army requests identity mismatch");
    }
    for (offset, vtable, data, pool) in [
        (0x10, 0x29d1dc0, 0x28, 0x38),
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
            != base
                .checked_add(vtable)
                .ok_or("merge requests vtable overflow")? as u64
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
                crate::read::<u64>(
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

/// Only variants the source operation can credit. Merge admits positive
/// surplus; bulk addition also reaches zero and negative rows. Both use the
/// selected reserve entries without calling getters. Sources sort by callback-local
/// pointer and retain complete design handles.
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
/// Copy only the destination rows selected by native source occurrences.
/// The caller's owned selectors can combine distinct reached credit routes.
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
            crate::read::<u64>(memory, address).ok_or("reserve variant unreadable")?,
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

/// Semantic native classification; replay owns the stable encoded values.
/// An unrecognized caller still has an exact removal and is recorded as
/// Unknown, rather than discarded or attributed to a guessed cause.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Cause {
    #[default]
    Unknown,
    LandCombat,
    NavalTransport,
    Immediate,
    Attrition,
    Garrison,
    AirNuclear,
    AirAccident,
    AirStrategic,
    AirMissionAa,
    AirLandCombatCas,
    AirRaid,
    AirNavalAa,
    DivisionDestroyed,
    DivisionOverrun,
    DivisionDeleted,
    DivisionDisbanded,
    DivisionCapitulation,
    DivisionScripted,
    DivisionTemplateRemoved,
    ParadropDisruption,
    DivisionMerged,
    ExileEquipment,
    VolunteerReturnEquipment,
    CapitulationEscapeEquipment,
    ScriptedEquipmentFraction,
    ScriptedStockpileFraction,
}

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

/// Actual return addresses include tail-jump routes. Transfer readers preserve
/// their original parser dispatch return through unit deletion; combat lifecycle
/// cleanup also retains the original caller return through its intervening dispatches.
pub const COMBAT_DELETION_RETURN: usize = 0x13e3d16;
pub const QUEUED_DELETE_RETURN: usize = 0x1d6444;
pub const READ_CONTEXT_RETURN: usize = 0x24bea65;
pub const DELETION_CAUSES: [(usize, Cause); 15] = [
    (COMBAT_DELETION_RETURN, Cause::DivisionDestroyed),
    // Unit-deletion effects retain their requested-return flag through the
    // exact reserve-return tap; it is not interpreted as a loss predicate.
    (0x355000, Cause::DivisionScripted),
    (0x35567e, Cause::DivisionScripted),
    // Destruction effects retain the return address through scope-validated virtual dispatch.
    (0x53dc20, Cause::DivisionScripted),
    (0x70d890, Cause::DivisionTemplateRemoved),
    (0x1513437, Cause::DivisionTemplateRemoved),
    (0x1366132, Cause::DivisionDisbanded),
    (0x6e1ca4, Cause::DivisionCapitulation),
    // Annex/OOB replacement wipes; initial OOB loading passes wipe=false.
    (0x6e9010, Cause::DivisionDeleted),
    (0x6e9120, Cause::DivisionDeleted),
    // Country army validation and orphan exile/volunteer transfer cleanup.
    (0x713a38, Cause::DivisionDeleted),
    (0x15119f0, Cause::DivisionDeleted),
    (0x1520461, Cause::DivisionDeleted),
    // Appearance reset also supports gameplay country reuse. Runtime reset
    // is excluded by RESETTING rather than by shared return addresses.
    (0x6e34fe, Cause::DivisionDeleted),
    (0x6e3611, Cause::DivisionDeleted),
];
/// Failed save-reader validation and rejected scripted unit creation delete
/// parsed allocations before roster attachment or stockpile supply.
pub const FAILED_CREATE_RETURN: usize = 0x352e3e;
pub const IGNORED_DELETIONS: [usize; 4] = [
    0x7070d7,
    0x70738c,
    READ_CONTEXT_RETURN,
    FAILED_CREATE_RETURN,
];
/// Merge copies source pools into its aggregate before disposal. Unchanged
/// source pools are therefore transfers rather than losses.
pub const MERGED_DELETION_RETURN: usize = 0x1364b8d;

pub fn deletion_context(caller_rva: usize) -> DeletionContext {
    if IGNORED_DELETIONS.contains(&caller_rva) {
        DeletionContext::Ignore
    } else if caller_rva == MERGED_DELETION_RETURN {
        DeletionContext::Transfer(Disposition::Merged)
    } else if caller_rva == QUEUED_DELETE_RETURN {
        DeletionContext::Queue
    } else {
        DeletionContext::Loss(
            DELETION_CAUSES
                .iter()
                .find_map(|&(pc, cause)| (pc == caller_rva).then_some(cause))
                .unwrap_or(Cause::Unknown),
        )
    }
}

/// Scheduling copies a unique full handle, never the pool. Retain the first
/// deletion cause until disposal. HQ cleanup credits its complete pool before
/// queueing, so that disposal is a transfer.
pub const QUEUED_DELETION_CAUSES: [(usize, Cause); 7] = [
    (0xc89a46, Cause::DivisionOverrun),
    // Hourly attrition completes before the content-loaded strength threshold
    // and deletion scheduling.
    (0xc886b6, Cause::DivisionDeleted),
    (0xdf7bd8, Cause::DivisionDeleted),
    (0x71819d, Cause::DivisionDeleted),
    (0x15213bd, Cause::DivisionDeleted),
    (0x9dd6f4, Cause::DivisionDeleted),
    // Immediate damage finishes its removal before this strength test.
    (0x343126, Cause::DivisionScripted),
];
pub const HQ_PROVIDER_RETURN: usize = 0xbf42d4;
/// Reveal-skip selects deception armies. Combat joining refuses those
/// objects, so their disposal is outside inventory loss.
pub const FAKE_INTEL_QUEUE_RETURN: usize = 0x12bb30b;

pub fn queued_context(caller_rva: usize) -> QueuedContext {
    if caller_rva == HQ_PROVIDER_RETURN {
        QueuedContext::Transfer(Disposition::HqTransferred)
    } else if caller_rva == FAKE_INTEL_QUEUE_RETURN {
        QueuedContext::Ignore
    } else {
        QueuedContext::Loss(
            QUEUED_DELETION_CAUSES
                .iter()
                .find_map(|&(pc, cause)| (pc == caller_rva).then_some(cause))
                .unwrap_or(Cause::Unknown),
        )
    }
}

/// Runtime reset sets this flag before draining queues and clearing countries,
/// then clears it. Deletion samples it and combat-end processing tests it
/// before applying gameplay damage.
pub const RESETTING: usize = 0xa3a;

pub fn cleanup(memory: &impl Memory, base: usize) -> Result<bool, &'static str> {
    let state = pointer(memory, base, crate::cgame_state::SINGLETON_RVA)?;
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
    if receiver == 0 || pointer(memory, base, crate::cgame_state::SINGLETON_RVA)? != receiver {
        return Err("destruction current game state identity mismatch");
    }
    Ok(())
}

/// Class zero identifies divisions for unit-deletion capture.
/// Other live unit classes are outside division-equipment deletion capture.
pub fn is_army(memory: &impl Memory, base: usize, unit: usize) -> Result<bool, &'static str> {
    if unit == 0 {
        return Err("destruction unit is null");
    }
    if field::<u32>(memory, unit, crate::cunit::CLASS)? != 0 {
        return Ok(false);
    }
    army_identity(memory, base, unit).map(|_| true)
}

/// Native full handle's high word is the saved division ID. Its low word can be dynamically
/// assigned: no reference-type constant is assumed. Authenticate both native CArmy vtables and the
/// class discriminator.
pub fn army_identity(memory: &impl Memory, base: usize, army: usize) -> Result<u64, &'static str> {
    let primary = base.checked_add(carmy::VTABLE_RVA);
    let secondary = base.checked_add(carmy::SECONDARY_VTABLE_RVA);
    if army == 0
        || field::<u32>(memory, army, crate::cunit::CLASS)? != 0
        || field::<u64>(memory, army, 0)? != primary.map(|p| p as u64).unwrap_or(0)
        || field::<u64>(memory, army, carmy::SECONDARY_BASE)?
            != secondary.map(|p| p as u64).unwrap_or(0)
    {
        return Err("destruction division identity mismatch");
    }
    let handle = field::<u64>(memory, army, crate::cunit::HANDLE)?;
    if handle >> 32 == 0 {
        return Err("destruction division handle invalid");
    }
    Ok(handle)
}

/// Direct combat-damage caller return addresses. Combat end flushes accumulated
/// losses and applies the content-loaded combat-loss factor before damage.
pub const LAND_COMBAT_RETURN: usize = 0xc8adf8;
/// Lost transports remove convoys and damage carried divisions in proportion
/// to the lost transport amount.
pub const NAVAL_TRANSPORT_RETURN: usize = 0xe28370;
/// Immediate damage applies strength and organization operands before
/// equipment damage.
pub const IMMEDIATE_RETURN: usize = 0xc8f975;

pub fn land_cause(caller_rva: usize) -> Cause {
    match caller_rva {
        LAND_COMBAT_RETURN => Cause::LandCombat,
        NAVAL_TRANSPORT_RETURN => Cause::NavalTransport,
        IMMEDIATE_RETURN => Cause::Immediate,
        _ => Cause::Unknown,
    }
}

/// Supported-build return-address to cause map. Strategic and mission-AA
/// channels retain the coordinator's classification. Requested removal and
/// source-country arguments do not establish a finer cause.
pub const AIR_CAUSES: [(usize, Cause); 11] = [
    // Nuclear airbase damage removes aircraft from every wing.
    (0x1099ea3, Cause::AirNuclear),
    // Accident losses apply the country loss roll and update the wing accumulator.
    (0xf5ddf2, Cause::AirAccident),
    // Strategic bombing applies content-loaded aircraft hit and damage factors.
    (0xc5b521, Cause::AirStrategic),
    // Strategic hourly processing removes aircraft during its wing pass.
    (0xc5fbc2, Cause::AirStrategic),
    // Train anti-air losses use the content-loaded hit and amount factors.
    (0xf763de, Cause::AirMissionAa),
    // Mission anti-air applies the mission-dependent wing loss.
    (0xf83a66, Cause::AirMissionAa),
    // Mission anti-air damage uses content-loaded chance and damage factors.
    (0xf86fe0, Cause::AirMissionAa),
    // Mission processing flushes its accumulated losses with their stored attribution.
    (0xf87677, Cause::AirMissionAa),
    // Land anti-air applies the night-attack and anti-air content factors.
    (0x12ab743, Cause::AirLandCombatCas),
    // Scripted raid-unit damage reaches this aircraft-removal channel.
    (0x1466ae8, Cause::AirRaid),
    // Naval anti-air callers remove aircraft through their shared exchange channel.
    (0x197a28c, Cause::AirNavalAa),
];

pub fn air_cause(caller_rva: usize) -> Cause {
    AIR_CAUSES
        .iter()
        .find_map(|&(pc, cause)| (pc == caller_rva).then_some(cause))
        .unwrap_or(Cause::Unknown)
}

/// Positive expeditionary ownership selects the equipment country; otherwise
/// use the roster owner. Retain the raw reference independently of the enclosing roster.
pub fn army_owner(memory: &impl Memory, army: usize) -> Result<u32, &'static str> {
    let expeditionary = field::<i32>(memory, army, crate::cunit::EXPEDITIONARY_OWNER)?;
    if expeditionary > 0 {
        Ok(expeditionary as u32)
    } else {
        field(memory, army, crate::cunit::ROSTER_OWNER)
    }
}

/// Preserve the full raw expeditionary-owner word, including nulls. Deletion
/// clears it before combat unlink.
pub fn army_expeditionary_owner(memory: &impl Memory, army: usize) -> Result<u32, &'static str> {
    field(memory, army, crate::cunit::EXPEDITIONARY_OWNER)
}

/// The standard template's reveal-skip byte selects deception armies. Combat
/// joining refuses them and damage or attrition does not apply force losses.
/// Their parsed pools need not be empty; disposal is not an inventory loss.
pub fn fake_intel_division(memory: &impl Memory, army: usize) -> Result<bool, &'static str> {
    let template = pointer(memory, army, carmy::STANDARD_TEMPLATE)?;
    if template == 0 {
        Ok(false)
    } else {
        field::<u8>(
            memory,
            template,
            crate::creferenced_division_template::REVEAL_SKIP,
        )
        .map(|flag| flag != 0)
    }
}

/// Aircraft removal prefers a positive pool-country reference, otherwise
/// the host owner. The source-country argument identifies the attacker.
pub fn air_owner(memory: &impl Memory, wing: usize) -> Result<u32, &'static str> {
    let owner = field::<i32>(memory, wing, 0x9c4)?;
    if owner > 0 {
        return Ok(owner as u32);
    }
    let base = pointer(memory, wing, 0x38)?;
    let status = pointer(memory, base, 0x40)?;
    field(memory, status, 0x90)
}

/// Resistance attributes the occupation-entry pool through its state to
/// that state's controller.
pub fn garrison_owner(memory: &impl Memory, occupation: usize) -> Result<u32, &'static str> {
    let state = pointer(memory, occupation, 0x50)?;
    field(memory, state, 0xcc)
}

fn field<T: crate::memory::Raw>(
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

/// Bulk equipment reserve credit reaches the reserve manager's embedded pool.
pub fn reserve_pool(manager: usize) -> Result<usize, &'static str> {
    manager
        .checked_add(0x200)
        .ok_or("destruction reserve pool address overflow")
}

/// Scripted stockpile fractions follow the country's production manager to
/// its embedded pool. Authenticate manager and country vtables, the manager
/// owner backlink and the country's reverse manager link.
pub fn reserve_owner(memory: &impl Memory, base: usize, pool: usize) -> Result<u32, &'static str> {
    use crate::{ccountry, cproduction_status};
    let manager = pool
        .checked_sub(cproduction_status::STOCKPILE_POOL)
        .filter(|&manager| manager != 0)
        .ok_or("destruction reserve manager address invalid")?;
    let expected_manager = base
        .checked_add(cproduction_status::VTABLE_RVA)
        .ok_or("destruction reserve manager vtable address overflow")?;
    if field::<u64>(memory, manager, 0)? != expected_manager as u64 {
        return Err("destruction reserve manager identity mismatch");
    }
    let country = pointer(memory, manager, cproduction_status::COUNTRY)?;
    let expected_country = base
        .checked_add(ccountry::VTABLE_RVA)
        .ok_or("destruction reserve country vtable address overflow")?;
    if country == 0
        || field::<u64>(memory, country, 0)? != expected_country as u64
        || pointer(memory, country, ccountry::PRODUCTION_STATUS)? != manager
    {
        return Err("destruction reserve country identity mismatch");
    }
    field(memory, country, ccountry::ID)
}

/// Merge return uses the survivor's roster country, which can differ from
/// combat's expeditionary owner. The merge route rejects expeditionary armies
/// and active combat. Preserve the admitted equipment owner's raw reference.
pub fn merge_owner(memory: &impl Memory, army: usize) -> Result<u32, &'static str> {
    field(memory, army, crate::cunit::ROSTER_OWNER)
}

// Quantity addition saturates, while scaling can store signed i64 values
// beyond that bound. Preserve stored bytes; refuse only overflow in actual
// duplicate aggregation or net differences below.

/// Owned copies, bounded by the live design census. A short copy or growing
/// pool fails the event; nothing reads game pointers after the detour returns.
pub fn snapshot(
    memory: &impl Memory,
    base: usize,
    pool: usize,
    rows: &mut [equipment_entry::Row],
) -> Result<usize, &'static str> {
    snapshot_with_demand(memory, base, pool, rows, |_| ())
}
/// Retain original pool-header demand even when allocation-free callback
/// storage cannot retain its payload.
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

/// Attribute the variant's actual change rather than the requested amount.
/// Subtraction precedes optional combat logging. Sum repeated occurrences
/// instead of assuming uniqueness.
pub fn variant_amount(
    memory: &impl Memory,
    base: usize,
    pool: usize,
    variant: usize,
    scratch: &mut [u8],
) -> Result<i64, &'static str> {
    variant_amount_with_demand(memory, base, pool, variant, scratch, |_| ())
}
/// Report the original pool row count before the scratch check, preserving
/// payload and consistency-read order.
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

/// Sort owned snapshots and visit every nonzero signed pre-minus-post net.
/// Negative rows are native gains, including an added design or signed entry.
/// Duplicate native entries retain their total; arithmetic overflow is refused.
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

/// Live counts only; these are capacity observations, never recorded facts.
pub fn design_count(memory: &impl Memory, base: usize) -> Option<usize> {
    let state = usize::try_from(read::<u64>(
        memory,
        base.checked_add(crate::cgame_state::SINGLETON_RVA)?,
    )?)
    .ok()?;
    if state == 0 {
        return Some(0);
    }
    let header = read(
        memory,
        state.checked_add(crate::cgame_state::EQUIPMENT_VARIANTS)?,
    )?;
    Some(Array::from_header(header, 8)?.count as usize)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Population {
    pub divisions: usize,
    pub division_pool_entries: usize,
    pub stockpile_entries: usize,
    pub delivery_pool_entries: usize,
}

/// Off-thread sizing only: sum live division-roster counts and equipment-pool
/// rows once per canonical country, retaining stockpile demand separately.
pub fn population(memory: &impl Memory, base: usize) -> Option<Population> {
    let state = usize::try_from(read::<u64>(
        memory,
        base.checked_add(crate::cgame_state::SINGLETON_RVA)?,
    )?)
    .ok()?;
    if state == 0 {
        return Some(Population::default());
    }
    let countries = Array::read(memory, state.checked_add(crate::cgame_state::COUNTRIES)?, 8)?;
    if countries.count < 2 {
        return Some(Population::default());
    }
    let reader = crate::ccountry::Reader::new(memory, base, state).ok()?;
    let mut demand = Population::default();
    for slot in 1..reader.count() {
        if let Some(country) = reader.at(slot).ok()? {
            let roster = Array::read(
                memory,
                country.pointer.checked_add(crate::ccountry::DIVISIONS)?,
                8,
            )?;
            demand.divisions = demand.divisions.checked_add(roster.count as usize)?;
            for index in 0..roster.count {
                let army = usize::try_from(read::<u64>(memory, roster.element(index, 8)?)?).ok()?;
                if army != 0 {
                    let pool = army.checked_add(carmy::EQUIPMENT_POOL)?;
                    let entries = cequipment_variant_pool::entries(memory, base, pool).ok()?;
                    canceled_delivery_pools(memory, base, army, true, |pool| {
                        let entries = cequipment_variant_pool::entries(memory, base, pool)?;
                        demand.delivery_pool_entries = demand
                            .delivery_pool_entries
                            .checked_add(entries.count as usize)
                            .ok_or("merge delivery census overflow")?;
                        Ok(())
                    })
                    .ok()?;
                    demand.division_pool_entries = demand
                        .division_pool_entries
                        .checked_add(entries.count as usize)?;
                }
            }
            let manager = usize::try_from(read::<u64>(
                memory,
                country
                    .pointer
                    .checked_add(crate::ccountry::PRODUCTION_STATUS)?,
            )?)
            .ok()?;
            if manager != 0 {
                let pool = manager.checked_add(crate::cproduction_status::STOCKPILE_POOL)?;
                let entries = cequipment_variant_pool::entries(memory, base, pool).ok()?;
                demand.stockpile_entries = demand
                    .stockpile_entries
                    .checked_add(entries.count as usize)?;
            }
        }
    }
    Some(demand)
}

#[cfg(test)]
mod tests;
