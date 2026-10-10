//! Cause-specific destruction in the hash-pinned a729 executable. No logging,
//! orders-group or player predicates are used. Pool differences are actual
//! signed native fixed5 net removals; a negative row is a native gain.
//!
//! `hoi4re asm/fn` evidence: active_damage 140C7BFC0 scales/rounds army+348
//! (140FFBCC0/140FFCCE0); KillAirplanes 140F4E400 removes wing+1D8 through
//! 140FF7A30; resistance 140FDFA80 scales/rounds occupation+90 then computes
//! its own pre-minus-post pool through 140FFD520, before the AI log gate.
//!
//! Deletion uses the shared DeleteUnit scope, its exact reserve-return CALL,
//! and CArmy's pre-free deleting destructor. The scope preserves ownership
//! before 140BF4E40 clears expeditionary ownership; nested army writers use
//! that captured country through the final combat flush. Returned, merged and
//! HQ-transferred pools have deletion witnesses without loss amounts.
//! ResetRuntimeState sets game-state+A3A before queue/country cleanup and
//! clears it afterwards; parser and failed-create cleanup have no witnesses.
//! Fake-intel armies use the recorded census's reveal_skip flag and are
//! excluded as deception objects, without assuming their parsed pools empty.
//! Paradrop's sole writer 140C7AE10 divides then floors the live army pool.
//! Combat flush CALL 1413CE6C1 precedes lifecycle CALL 1413CE9F0. DeleteUnit
//! also unlinks combat at 1401D69AB -> 140BF4C40 -> 1413CE250, invoking
//! OnCombatEnded before deleting-destructor CALL 1401D6DA3. The final snapshot
//! therefore excludes prior combat/attrition/immediate removals, including
//! an overrun's final combat unlink. CaptureEquipmentFromDefeatedArmies makes
//! scaled copies and credits winners; it never subtracts the victim's pool.
//! Whole Merge@14134ECE0 conserves selected main pools, canceled delivery
//! pools and touched reserves across retained allocation and actual credits.
//! Its source destructor witnesses never count transferred bookkeeping twice.
//! Volunteer/exile StartTransfer CALLs remove live armies from rosters but
//! retain their handles in serialized transfer objects: zero-amount transfer
//! witnesses cover that absence until reappearance or actual deletion.
//! SetExiled's scale then integral rounding are one pre/post observation;
//! rounding may add equipment, so observing the scale alone would overcount.
//! Volunteer return scaling, capitulation escape scaling and scripted
//! equipment fractions have exact call taps, including the script's reserves.
//! Land's negative damage, stochastic ceiling and signed pool entries can
//! increase pools (140C7C1AC/140C7C327/140FF7A30); 140C7C341 accounts for the
//! signed pre-minus-post difference. Script factors at 14049B2B3 are unclamped.

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
    rva: 0xc7bfc0,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x20, 0x44, 0x88, 0x44, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54,
        0x41,
    ],
};
/// Replace only CALL 140FFD5A0 at 140C5EAC4 (five bytes), with another CALL.
/// RCX is army+348 (MOV RCX,[RDI]; ADD RCX,348 at 140C5EAAA-B9), RDX
/// variant (R14), R8 positive fixed5 removal (RBX); void. Native RSP is
/// 16-byte aligned with home space. Next instruction is 140C5EAC9; no
/// branch targets or RIP-relative operands lie inside the five-byte span.
/// This is after the +438 <=0 return (140C5FE89-93), the +3B8/+234 gate
/// (140C5FE99-AC), and the helper's loss-roll <=0 return (140C5EAA2-A9).
/// The original CALL is PC-relative; invoke its verified target directly,
/// never call the mid-function continuation as a function trampoline.
pub const ATTRITION: Site = Site {
    id: SiteId::Attrition,
    kind: SiteKind::Call {
        target: ATTRITION_TARGET,
    },
    name: "attrition_removal_tap",
    rva: 0xc5eac4,
    sentinel: [
        0xe8, 0xd7, 0xea, 0x39, 0x00, 0x48, 0x8b, 0x47, 0x10, 0x48, 0x8b, 0x08, 0x48, 0x85, 0xc9,
        0x74,
    ],
};
pub const ATTRITION_TARGET: usize = 0xffd5a0;
/// Win64 RCX wing, EDX signed requested count, R8D source country; void.
pub const AIR: Site = Site {
    id: SiteId::Air,
    kind: SiteKind::Entry,
    name: "airplanes",
    rva: 0xf4e400,
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
    rva: 0xfdfa80,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54,
        0x41,
    ],
};
/// RCX game state, RDX unit, R8B return request; void. R9 is not consumed,
/// but the detour preserves its full slot. Entry/unwind: 1401D6210/1435EAA98.
pub const DELETE: Site = Site {
    id: SiteId::Delete,
    kind: SiteKind::Entry,
    name: "division_delete_scope",
    rva: 0x1d6210,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54,
        0x41,
    ],
};
/// Replace only CALL 1401D6718 with CALL. RCX reserve manager, RDX army+348;
/// void. This executes only after return was requested, the unit is not in a
/// naval transfer and HasHomeAreaOrSupplyPortConnection@140DE2430 succeeded.
/// No branch targets lie in the five-byte patch. Return PC is 1401D671D.
pub const RETURN: Site = Site {
    id: SiteId::Return,
    kind: SiteKind::Call {
        target: RETURN_TARGET,
    },
    name: "division_equipment_return_tap",
    rva: 0x1d6718,
    sentinel: [
        0xe8, 0x33, 0x5e, 0xc7, 0x00, 0x48, 0x8b, 0xd3, 0xeb, 0xc7, 0x48, 0x0f, 0xaf, 0x1d, 0xd6,
        0x3d,
    ],
};
pub const RETURN_TARGET: usize = 0xe4c550;
/// RCX primary army, EDX deleting flags; returns the original army pointer.
/// 140C5F08F calls ~CArmy; its 140C5E59B/5AB clear/free the live +348 pool.
/// The secondary -10/-338 thunks tail-jump here, preserving the caller PC.
/// Entry/unwind: 140C5F080/1436378A4.
pub const DIVISION_FREE: Site = Site {
    id: SiteId::DivisionFree,
    kind: SiteKind::Entry,
    name: "division_equipment_discard",
    rva: 0xc5f080,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x8b, 0xda, 0x48, 0x8b, 0xf9,
        0xe8,
    ],
};
/// RCX game state, RDX unit; void. Only remembers provenance: the actual
/// residual is copied at DIVISION_FREE, after any intervening writers.
/// Entry/unwind: 1401D6FE0/1435EAAA4.
pub const QUEUE: Site = Site {
    id: SiteId::Queue,
    kind: SiteKind::Entry,
    name: "division_deletion_queue_scope",
    rva: 0x1d6fe0,
    sentinel: [
        0x4c, 0x8b, 0xdc, 0x49, 0x89, 0x5b, 0x08, 0x49, 0x89, 0x6b, 0x18, 0x49, 0x89, 0x73, 0x20,
        0x57,
    ],
};
/// Five integer slots: army, source-country pointer, equipment divisor,
/// manpower divisor, organisation divisor. The fifth is at entry RSP+28.
/// Sole caller CALL 140C7B34B returns 140C7B350; 140FF8BB0 divides +348 then
/// 140FFD700 floors it before gated logging. Entry/unwind: 140C7AE10/143637FC4.
pub const PARADROP: Site = Site {
    id: SiteId::Paradrop,
    kind: SiteKind::Entry,
    name: "paradrop_disruption",
    rva: 0xc7ae10,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x56, 0x57, 0x41, 0x54, 0x41,
        0x56,
    ],
};
/// StartTransfer@1415108D0, CALL 141511023 -> RemoveUnitFromRoster@1407034B0.
/// RCX roster country, RDX primary army; returns the original integer result.
/// The transfer retains the full army handle; neither this call nor RemoveArmy
/// deletes the pool. Write@141511350 and ReadMember@14150FE60 serialize/read
/// the held divisions separately from the roster.
pub const VOLUNTEER_TRANSFER: Site = Site {
    id: SiteId::VolunteerTransfer,
    kind: SiteKind::Call {
        target: TRANSFER_TARGET,
    },
    name: "volunteer_roster_transfer_tap",
    rva: 0x1511023,
    sentinel: [
        0xe8, 0x88, 0x24, 0x1f, 0xff, 0x49, 0x8b, 0xcf, 0xe8, 0x10, 0x22, 0x1e, 0xff, 0x84, 0xc0,
        0x74,
    ],
};
/// Exile StartTransfer@1415014E0, CALL 1415015D3; same ABI as volunteer tap.
/// CreateExilesTransfer@1406D9C80 supplies live armies; ReadMember@141500AB0
/// reconstructs transfer-held armies separately from country roster units;
/// Write@141501660 writes each held army under the division token.
pub const EXILE_TRANSFER: Site = Site {
    id: SiteId::ExileTransfer,
    kind: SiteKind::Call {
        target: TRANSFER_TARGET,
    },
    name: "exile_roster_transfer_tap",
    rva: 0x15015d3,
    sentinel: [
        0xe8, 0xd8, 0x1e, 0x20, 0xff, 0x49, 0x8b, 0xce, 0xe8, 0x60, 0x1c, 0x1f, 0xff, 0x84, 0xc0,
        0x74,
    ],
};
pub const TRANSFER_TARGET: usize = 0x7034b0;
/// Win64 RCX primary army, EDX bool; void. Entry/unwind 140C7F020/143638180.
/// Scale CALL 140C7F085 -> 140FFBCC0 then rounding CALL 140C7F091 -> 140FFCC10
/// are the only pool mutations. SetMovementTarget clears +1E8; RefreshAbilities
/// rebuilds stats; CancelMovement flag=1 preserves active combat (140BEBF70).
/// Observe the entire method to include rounding gains in the exact net loss.
/// CArmy's primary vtable 142943AD0 has SetExiled in slot 50 (+190).
/// The fixed-displacement instruction/decompile census finds only the unit
/// access update 140BF5BB0: CALLs 140BF5C14 (true) and 140BF5BFE (false), from
/// army HourlyUpdate@140C78B6B or railway update@140E76804, never a loader.
/// Pointer copies/adjustments outside those indexed forms remain unverified;
/// this census does not prove a complete loader/reset caller exclusion.
pub const SET_EXILED: Site = Site {
    id: SiteId::SetExiled,
    kind: SiteKind::Entry,
    name: "exile_equipment",
    rva: 0xc7f020,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57, 0x48, 0x81, 0xec, 0x70,
        0x01,
    ],
};
/// All scale taps are void, RCX pool, RDX fixed5 factor. Only the five-byte
/// CALL is replaced: ScaleQuantities@140FFBCC0 does not call other writers.
pub const SCALE_TARGET: usize = 0xffbcc0;
/// Returning volunteers (+2C==0) scale live army+348 at 141510D60 using
/// VOLUNTEERS_RETURN_EQUIPMENT@14331C9C8. The removed difference is logged
/// as AddEquipmentLoss reason 1; no source reserve credit occurs. Update
/// @14150EAD0 returns the surviving army/pool through AddArmy, not the loss.
/// CreateVolunteersTransfer@1406DA640 forwards its fifth bool at 1406DA891
/// to the +2C setter 1415014D0. SendVolunteerAction supplies 1@1410F4AD6;
/// explicit/automatic recall supplies 0@1406CA2AA/1406CAAEC, proving return.
pub const VOLUNTEER_RETURN_LOSS: Site = Site {
    id: SiteId::VolunteerReturnLoss,
    kind: SiteKind::Call {
        target: SCALE_TARGET,
    },
    name: "volunteer_return_equipment_tap",
    rva: 0x1510d60,
    sentinel: [
        0xe8, 0x5b, 0xaf, 0xae, 0xff, 0x48, 0x8d, 0x96, 0x48, 0x03, 0x00, 0x00, 0x48, 0x8d, 0x4c,
        0x24,
    ],
};
/// Capitulation escape@1406D7270 scales selected live armies at 1406D7C58,
/// using GIE_ESCAPING_DIVISIONS_EQUIPMENT_RATIO@14331D5E0 and selection
/// rounding. The tap reads the actual pool result, never the define/factor.
pub const CAPITULATION_ESCAPE_LOSS: Site = Site {
    id: SiteId::CapitulationEscapeLoss,
    kind: SiteKind::Call {
        target: SCALE_TARGET,
    },
    name: "capitulation_escape_equipment_tap",
    rva: 0x6d7c58,
    sentinel: [
        0xe8, 0x63, 0x40, 0x92, 0x00, 0x48, 0x8d, 0x55, 0xf7, 0x48, 0x8d, 0x0d, 0xa8, 0x58, 0xc4,
        0x02,
    ],
};
/// CSetEquipmentFraction::vf13@14049B260 scales every roster army+348 in
/// the target country's array at CALL 14049B2F3. Evaluate@14049B2B3 is
/// unclamped: factors above one or below zero have signed native results.
pub const SCRIPTED_EQUIPMENT_FRACTION: Site = Site {
    id: SiteId::ScriptedEquipmentFraction,
    kind: SiteKind::Call {
        target: SCALE_TARGET,
    },
    name: "scripted_division_equipment_fraction_tap",
    rva: 0x49b2f3,
    sentinel: [
        0xe8, 0xc8, 0x09, 0xb6, 0x00, 0x48, 0xff, 0xc3, 0x48, 0x3b, 0xde, 0x7c, 0xe0, 0x48, 0x8b,
        0x5c,
    ],
};
/// The same script scales CProductionStatus+200 first at CALL 14049B2C0.
/// Owner comes through the authenticated manager's +290 country backlink.
/// This national pool includes aircraft and ships, so its cause has no fixed
/// land domain: resolve each design's equipment type through recorded content.
pub const SCRIPTED_RESERVE_EQUIPMENT_FRACTION: Site = Site {
    id: SiteId::ScriptedReserveEquipmentFraction,
    kind: SiteKind::Call {
        target: SCALE_TARGET,
    },
    name: "scripted_reserve_equipment_fraction_tap",
    rva: 0x49b2c0,
    sentinel: [
        0xe8, 0xfb, 0x09, 0xb6, 0x00, 0x48, 0x8b, 0xcf, 0xe8, 0xe3, 0x72, 0x22, 0x00, 0x48, 0x8b,
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

/// Merge@14134ECE0: RCX points to survivor followed by the candidate CPdxArray.
/// The complete operation clears signed pools and saturates retained allocations.
pub const MERGE: Site = Site {
    id: SiteId::Merge,
    kind: SiteKind::Entry,
    name: "division_merge_pools",
    rva: 0x134ece0,
    sentinel: [
        0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
        0x41,
    ],
};
/// clear_hq_provider@140BE4970 returns equipment at CALL140BE4C9C before
/// queuing the same complete army handle at140BE4DBF. Five-byte CALL span.
pub const HQ_RETURN: Site = Site {
    id: SiteId::HqReturn,
    kind: SiteKind::Call {
        target: RETURN_TARGET,
    },
    name: "hq_equipment_return_tap",
    rva: 0xbe4c9c,
    sentinel: [
        0xe8, 0xaf, 0x78, 0x26, 0x00, 0xba, 0xa0, 0x86, 0x01, 0x00, 0x48, 0x8d, 0x8b, 0xd0, 0x03,
        0x00,
    ],
};

/// Native selected pools: matching +1F0 location pointers at14134F01B.
/// Distinct physical pools are read once even if the input repeats a pointer.
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

/// Merge cancels survivor reinforcement deliveries at14134F973 ->140C7B9C0
/// ->1414F8330. Source destruction140C5E1BC ->1414F79C0 also returns upgrade
/// deliveries through1414F7610. Survivor upgrades remain live. Every status and
/// signed/zero row is passed to140E4C550; no progress getter is used.
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
    // CArmyRequests::vf0@1414F79C0, parent army at+8; child owning army+20.
    if field::<u64>(memory, requests, 0)?
        != base
            .checked_add(0x29bb490)
            .ok_or("merge requests vtable overflow")? as u64
        || pointer(memory, requests, 8)? != army
    {
        return Err("merge army requests identity mismatch");
    }
    for (offset, vtable, data, pool) in [
        (0x10, 0x29bb1d0, 0x28, 0x38),
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
/// surplus; bulk pool addition also reaches zero and negative source rows. The native bulk add/merge
/// searches these same reserve entries (140FF7830/14134F948); no getter runs.
/// Sources are sorted by callback-local pointer and retain full design handles.
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

/// Actual return PCs, including native tail jumps. Both transfer readers
/// tail-jump from ReadMember to DeleteUnit, preserving CPersistent::Read's
/// sole token-dispatch return (CALL 1424A9B02). Combat lifecycle similarly
/// preserves CALL 1413CE9F0's return through 141299E83 and 1413CC7A3.
pub const COMBAT_DELETION_RETURN: usize = 0x13ce9f6;
pub const QUEUED_DELETE_RETURN: usize = 0x1d5fc4;
pub const READ_CONTEXT_RETURN: usize = 0x24a9b05;
pub const DELETION_CAUSES: [(usize, Cause); 15] = [
    (COMBAT_DELETION_RETURN, Cause::DivisionDestroyed),
    // DeleteUnitEffect and DeleteUnitsEffect; their return flag is recorded
    // by the exact reserve-return tap, not interpreted as a loss predicate.
    (0x354980, Cause::DivisionScripted),
    (0x354ffe, Cause::DivisionScripted),
    // DestroyUnitEffect tail-jumps through ExecuteWithScopeValidation's
    // sole virtual Execute dispatch (CALL 14053130D).
    (0x531310, Cause::DivisionScripted),
    (0x700c06, Cause::DivisionTemplateRemoved),
    (0x1501127, Cause::DivisionTemplateRemoved),
    (0x1351072, Cause::DivisionDisbanded),
    (0x6d5094, Cause::DivisionCapitulation),
    // Annex/OOB replacement wipes; initial OOB loading passes wipe=false.
    (0x6dc460, Cause::DivisionDeleted),
    (0x6dc570, Cause::DivisionDeleted),
    // Country army validation and orphan exile/volunteer transfer cleanup.
    (0x706e08, Cause::DivisionDeleted),
    (0x14ff6e0, Cause::DivisionDeleted),
    (0x150e151, Cause::DivisionDeleted),
    // ResetCountryAppearance also serves gameplay dynamic-country reuse.
    // ResetRuntimeState is excluded by RESETTING, not these shared PCs.
    (0x6d68ee, Cause::DivisionDeleted),
    (0x6d6a01, Cause::DivisionDeleted),
];
/// Failed save-reader validation and rejected CreateUnitEffect setup.
/// CALL 1403527B9 deletes the parsed allocation before AddArmy@140352940:
/// no stockpile supplied it and no live division was attached.
pub const FAILED_CREATE_RETURN: usize = 0x3527be;
pub const IGNORED_DELETIONS: [usize; 4] = [
    0x6fa567,
    0x6fa81c,
    READ_CONTEXT_RETURN,
    FAILED_CREATE_RETURN,
];
/// Merge copies source pools into its aggregate before this DeleteUnit CALL
/// (14134FAC8). Source pools remain unchanged, so their disposal is a transfer.
pub const MERGED_DELETION_RETURN: usize = 0x134facd;

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

/// ScheduleUnitDeletion copies only a unique native handle, never the pool.
/// Preserve the first provenance until actual deletion. HQ provider cleanup
/// credits its full pool at 140BE4C9C before queueing, so it is a transfer.
pub const QUEUED_DELETION_CAUSES: [(usize, Cause); 7] = [
    (0xc7a306, Cause::DivisionOverrun),
    // Hourly attrition CALL 140C78C2E completes before the strength-threshold
    // comparison of runtime define 14331A2B8 and this scheduling CALL.
    (0xc78f76, Cause::DivisionDeleted),
    (0xde32d8, Cause::DivisionDeleted),
    (0x70b56d, Cause::DivisionDeleted),
    (0x150f0ad, Cause::DivisionDeleted),
    (0x9d08a4, Cause::DivisionDeleted),
    // TakeDamageImmediate's actual removal precedes this strength test.
    (0x342aa6, Cause::DivisionScripted),
];
pub const HQ_PROVIDER_RETURN: usize = 0xbe4dc4;
/// RevealFakeArmy@1412A5F40 CALL 1412A6246 returns here (1412A6250 is
/// the following MOV). Its GetDivisionRevealSkipFlag-selected armies are
/// deception objects: TryJoinUnit@1412A6C70 refuses them before combat.
pub const FAKE_INTEL_QUEUE_RETURN: usize = 0x12a624b;

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

/// ResetRuntimeState@1401D09B0 sets this at 1401D09F4, drains the queue and
/// clears countries, then clears it at 1401D1730. DeleteUnit samples it at
/// 1401D6340 and OnCombatEnded checks it before applying gameplay damage.
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

/// Class zero is CArmy (constructor 140C5DD30; DeleteUnit compares +8).
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

/// Native full handle is +18, whose high word (+1C) is the saved division ID.
/// Its low word can be dynamically assigned: no reference-type constant is
/// assumed. Authenticate both native CArmy vtables and the class discriminator.
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

/// Return PCs of the three direct active_damage callers, verified by asm.
/// OnCombatEnded@140C7B370 flushes accumulated combat losses and applies the
/// EQUIPMENT_COMBAT_LOSS_FACTOR before CALL 140C7B6B3.
pub const LAND_COMBAT_RETURN: usize = 0xc7b6b8;
/// navaltransfer.cpp 140E138F0 removes lost convoys and damages their carried
/// armies through CALL 140E13A6B, with the lost transport proportion.
pub const NAVAL_TRANSPORT_RETURN: usize = 0xe13a70;
/// TakeDamageImmediate@140C80020 applies the immediate strength/organisation
/// operands before CALL 140C80220.
pub const IMMEDIATE_RETURN: usize = 0xc80225;

pub fn land_cause(caller_rva: usize) -> Cause {
    match caller_rva {
        LAND_COMBAT_RETURN => Cause::LandCombat,
        NAVAL_TRANSPORT_RETURN => Cause::NavalTransport,
        IMMEDIATE_RETURN => Cause::Immediate,
        _ => Cause::Unknown,
    }
}

/// Build-specific return PC -> semantic cause map. `hoi4re fn` establishes
/// each enclosing context; `hoi4re asm` establishes the CALL and next PC.
/// The strategic and mission-AA groups preserve the coordinator's source
/// classifications, including mission loss application rather than guessing
/// a finer cause from the requested removal or source-country argument.
pub const AIR_CAUSES: [(usize, Cause); 11] = [
    // nuke.cpp 141084AA0 walks an airbase's wings and removes each wing's
    // aircraft count: CALL 141084BEE -> return 141084BF3.
    (0x1084bf3, Cause::AirNuclear),
    // 140F48C00 obtains the country loss roll from 1407086E0 and increments
    // wing accident losses (+84): CALL 140F48C6D -> return 140F48C72.
    (0xf48c72, Cause::AirAccident),
    // strategicair.cpp 140C4BC40 uses BASE_STRATEGIC_BOMBING_HIT_PLANE_*
    // chance/damage: CALL 140C4BDDC -> return 140C4BDE1.
    (0xc4bde1, Cause::AirStrategic),
    // strategicair.cpp 140C4FD80, called by StrategicAirManager::HourlyUpdate,
    // removes one aircraft in its wing pass: CALL 140C5047D -> return 140C50482.
    (0xc50482, Cause::AirStrategic),
    // airmission.cpp 140F60880 uses TRAIN_ANTI_AIR_HIT_* and
    // TRAIN_ANTI_AIR_ATTACK_TO_AMOUNT: CALL 140F61259 -> return 140F6125E.
    (0xf6125e, Cause::AirMissionAa),
    // airmission.cpp 140F6DB20 invokes mission AA helper 140F718C0 and
    // applies a mission-dependent wing loss: CALL 140F6E8E1 -> return 140F6E8E6.
    (0xf6e8e6, Cause::AirMissionAa),
    // airmission.cpp 140F718C0 uses ANTI_AIR_PLANE_DAMAGE_CHANCE/FACTOR:
    // CALL 140F71E5B -> return 140F71E60.
    (0xf71e60, Cause::AirMissionAa),
    // airmission.cpp 140F723D0 flushes the mission's accumulated losses
    // (+98), attributed to +A4: CALL 140F724F2 -> return 140F724F7.
    (0xf724f7, Cause::AirMissionAa),
    // 141295D10, called by LandCombatant::round, uses CAS_NIGHT_ATTACK_FACTOR
    // and land ANTI_AIR_*: CALL 14129667E -> return 141296683.
    (0x1296683, Cause::AirLandCombatCas),
    // 141454710 is called by DamageRaidUnitEffect::Execute@1404A29C0:
    // CALL 1414547D3 -> return 1414547D8.
    (0x14547d8, Cause::AirRaid),
    // naval_fire_exchange_air.cpp 141966B10 removes aircraft for the naval
    // AA callers 141967CD0/141969200: CALL 141966BD7 -> return 141966BDC.
    (0x1966bdc, Cause::AirNavalAa),
];

pub fn air_cause(caller_rva: usize) -> Cause {
    AIR_CAUSES
        .iter()
        .find_map(|&(pc, cause)| (pc == caller_rva).then_some(cause))
        .unwrap_or(Cause::Unknown)
}

/// +1DC when positive, otherwise +1D8: the country selected by both writers
/// at 140C600A9-C3 / active_damage 140C7BFC0 and for loss accounting. This
/// follows expeditionary ownership rather than the containing army roster.
pub fn army_owner(memory: &impl Memory, army: usize) -> Result<u32, &'static str> {
    let expeditionary = field::<i32>(memory, army, crate::cunit::EXPEDITIONARY_OWNER)?;
    if expeditionary > 0 {
        Ok(expeditionary as u32)
    } else {
        field(memory, army, crate::cunit::ROSTER_OWNER)
    }
}

/// Preserve the full raw expeditionary-owner word, including native nulls.
/// DeleteUnit's 140BF4E40 CALL at 1401D654E clears it before combat unlink.
pub fn army_expeditionary_owner(memory: &impl Memory, army: usize) -> Result<u32, &'static str> {
    field(memory, army, crate::cunit::EXPEDITIONARY_OWNER)
}

/// GetDivisionRevealSkipFlag@140C79AA0 selects deception armies through the
/// standard template's +234 byte. TryJoinUnit@1412A6C70 refuses them, and
/// TakeDamage@140C7FDC0/ApplyAttrition@140C5FE80 do not apply force losses.
/// Their parsed pools need not be empty; fake disposal is not an inventory loss.
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

/// KillAirplanes uses wing+9C4 when positive, else wing+38 -> +40 -> +90
/// (140F4E400). The source-country argument is the attacker, not the owner.
pub fn air_owner(memory: &impl Memory, wing: usize) -> Result<u32, &'static str> {
    let owner = field::<i32>(memory, wing, 0x9c4)?;
    if owner > 0 {
        return Ok(owner as u32);
    }
    let base = pointer(memory, wing, 0x38)?;
    let status = pointer(memory, base, 0x40)?;
    field(memory, status, 0x90)
}

/// Resistance writer 140FDFA80 attributes the occupation entry's pool to
/// its state (+50), using that state's controller (+CC) for its loss record.
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
    }; // 140FDFA80
    receiver
        .checked_add(offset)
        .ok_or("destruction pool address overflow")
}

/// AddEquipmentPoolToCountryReserves@140E4C550 and the item wrapper
/// 140E4C530 add to the reserve manager's embedded pool at +200.
pub fn reserve_pool(manager: usize) -> Result<usize, &'static str> {
    manager
        .checked_add(0x200)
        .ok_or("destruction reserve pool address overflow")
}

/// Scripted fractions reach the stockpile through country+F68 then the folded
/// +200 accessor@140BEE1A0. CProductionStatus ctor@140E476A0 stores +290 owner;
/// authenticate its vtable, the country's vtable and the reverse F68 link.
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

/// Merge returns use the survivor's roster country (14134F8F3), which can
/// differ from ordinary combat's expeditionary owner. The merge validator
/// 141354570 rejects expeditionary armies and active combat, so this is the
/// equipment owner on the admitted native route. Keep the native raw tag.
pub fn merge_owner(memory: &impl Memory, army: usize) -> Result<u32, &'static str> {
    field(memory, army, crate::cunit::ROSTER_OWNER)
}

// AddVariantQuantity@140FF7A30 saturates, but ScaleQuantities@140FFBCC0
// stores signed i64 results past that bound. Preserve the stored bytes; only
// actual duplicate aggregation or net-difference overflow is refused below.

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
/// Preserve the original pool+20/+2C header (140FFDB00) demand even when
/// allocation-free callback storage cannot retain its payload.
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

/// Attribute one variant's actual change, not helper 140C5E9C0's requested
/// amount. 140FFD5A0 subtracts before the optional Combat Log write. Duplicate
/// occurrences, if supplied, are summed rather than assumed unique.
pub fn variant_amount(
    memory: &impl Memory,
    base: usize,
    pool: usize,
    variant: usize,
    scratch: &mut [u8],
) -> Result<i64, &'static str> {
    variant_amount_with_demand(memory, base, pool, variant, scratch, |_| ())
}
/// 140FFD5A0 consumes the same pool header; report its row count before the
/// scratch check without changing the original payload/consistency reads.
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

/// Off-thread sizing only: GetDivisionArray@1406C25B0 returns country+290;
/// sum its +29C counts and division+348 pool rows once per canonical country.
/// 140FFDB00 walks pool+20/+2C; the stockpile accessor 140BEE1A0 returns +200.
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
