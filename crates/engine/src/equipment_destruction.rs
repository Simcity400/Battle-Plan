//! Cause-specific destruction in the hash-pinned a729 executable. No logging,
//! orders-group or player predicates are used. Pool differences are actual
//! native fixed5 removals, never requested damage or inventory transfers.
//!
//! `hoi4re asm/fn` evidence: active_damage 140C7BFC0 scales/rounds army+348
//! (140FFBCC0/140FFCCE0); KillAirplanes 140F4E400 removes wing+1D8 through
//! 140FF7A30; resistance 140FDFA80 scales/rounds occupation+90 then computes
//! its own pre-minus-post pool through 140FFD520, before the AI log gate.

use crate::{
    Memory, cair_wing, carmy, cequipment_variant_pool, cpdx_array::Array, equipment_entry, read,
    walk::Outcome,
};

pub struct Site {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
}

/// Entry ABI: eight integer slots, RCX army; void. The four stack slots
/// and the full register slots must survive the caller-address wrapper.
pub const LAND: Site = Site {
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
    name: "airplanes",
    rva: 0xf4e400,
    sentinel: [
        0x48, 0x8b, 0xc4, 0x44, 0x89, 0x40, 0x18, 0x53, 0x41, 0x55, 0x41, 0x57, 0x48, 0x83, 0xec,
        0x70,
    ],
};
/// Win64 RCX occupation entry, RDX raw fixed-point damage, R8 source; void.
pub const GARRISON: Site = Site {
    name: "garrison_resistance",
    rva: 0xfdfa80,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54,
        0x41,
    ],
};
pub const SITES: [&Site; 4] = [&LAND, &ATTRITION, &AIR, &GARRISON];

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

pub fn pool(receiver: usize, site: usize) -> Result<usize, &'static str> {
    let offset = match site {
        0 => carmy::EQUIPMENT_POOL,
        2 => cair_wing::EQUIPMENT_POOL,
        3 => 0x90,
        _ => return Err("unsupported destruction writer"),
    }; // 140FDFA80
    receiver
        .checked_add(offset)
        .ok_or("destruction pool address overflow")
}

/// Owned copies, bounded by the live design census. A short copy or growing
/// pool fails the event; nothing reads game pointers after the detour returns.
pub fn snapshot(
    memory: &impl Memory,
    base: usize,
    pool: usize,
    rows: &mut [equipment_entry::Row],
) -> Result<usize, &'static str> {
    let array = cequipment_variant_pool::entries(memory, base, pool)?;
    if array.count as usize > rows.len() {
        return Err("destruction pool exceeds live design capacity");
    }
    match equipment_entry::walk(memory, base, array, rows) {
        Outcome::Rows { len } => {
            if cequipment_variant_pool::entries(memory, base, pool)? != array {
                return Err("destruction pool changed during snapshot");
            }
            if rows[..len].iter().any(|r| r.amount_raw < 0) {
                return Err("unsupported negative destruction pool quantity");
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
    let array = cequipment_variant_pool::entries(memory, base, pool)?;
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
            if quantity < 0 {
                return Err("unsupported negative destruction pool quantity");
            }
            amount = amount
                .checked_add(quantity)
                .ok_or("destruction variant quantity overflow")?;
        }
    }
    Ok(amount)
}

/// Sort owned snapshots and visit positive, exact per-design differences.
/// An increase/new design is unsupported in these destruction-only writers.
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
                    if row.amount_raw < 0 {
                        return Err("unsupported negative destruction pool quantity");
                    }
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
        if loss < 0 {
            return Err("destruction writer increased equipment pool");
        }
        if loss > 0 {
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

#[cfg(test)]
mod tests {
    use super::*;
    fn row(design: u64, amount: i64) -> equipment_entry::Row {
        equipment_entry::Row {
            variant_handle: design,
            amount_raw: amount,
            ..Default::default()
        }
    }
    #[test]
    fn actual_removals_keep_designs_rounding_duplicates_and_noops() {
        let mut before = [row(70, 450000), row(0xabc00000046, 300000), row(70, 50000)];
        let mut after = [row(70, 370000), row(0xabc00000046, 300000), row(70, 30000)];
        let mut events = Vec::new();
        removals(&mut before, &mut after, |d, a| events.push((d, a))).unwrap();
        assert_eq!(events, [(70, 100000)]);
        events.clear();
        removals(&mut after.clone(), &mut after, |d, a| events.push((d, a))).unwrap();
        assert!(events.is_empty());
        assert!(removals(&mut [row(70, 1)], &mut [row(70, 2)], |_, _| {}).is_err());
        assert!(removals(&mut [row(70, -1)], &mut [], |_, _| {}).is_err());
        assert!(removals(&mut [row(70, i64::MAX), row(70, 1)], &mut [], |_, _| {}).is_err());
    }
    #[test]
    fn callers_are_classified_semantically_and_unknown_is_preserved() {
        assert_eq!(land_cause(LAND_COMBAT_RETURN), Cause::LandCombat);
        assert_eq!(land_cause(NAVAL_TRANSPORT_RETURN), Cause::NavalTransport);
        assert_eq!(land_cause(IMMEDIATE_RETURN), Cause::Immediate);
        assert_eq!(land_cause(LAND_COMBAT_RETURN + 1), Cause::Unknown);
        for (source, expected) in AIR_CAUSES {
            assert_eq!(air_cause(source), expected);
            assert_eq!(air_cause(source + 1), Cause::Unknown);
        }
        assert_eq!(air_cause(0), Cause::Unknown);
        assert_eq!(Cause::default(), Cause::Unknown);
    }

    #[test]
    fn bounded_variant_copy_sums_duplicates_and_refuses_short_or_negative() {
        use crate::memory::SliceMemory;
        let mut bytes = vec![0u8; 0xc0];
        bytes[..8].copy_from_slice(&(cequipment_variant_pool::VTABLE_RVA as u64).to_le_bytes());
        bytes[0x20..0x28].copy_from_slice(&0x1080u64.to_le_bytes());
        bytes[0x28..0x2c].copy_from_slice(&3i32.to_le_bytes());
        bytes[0x2c..0x30].copy_from_slice(&3i32.to_le_bytes());
        for (i, (design, amount)) in [(0x2000u64, 450001i64), (0x3000, 7), (0x2000, 50000)]
            .into_iter()
            .enumerate()
        {
            bytes[0x80 + i * 16..0x88 + i * 16].copy_from_slice(&design.to_le_bytes());
            bytes[0x88 + i * 16..0x90 + i * 16].copy_from_slice(&amount.to_le_bytes());
        }
        let mem = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            variant_amount(&mem, 0, 0x1000, 0x2000, &mut [0; 48]),
            Ok(500001)
        );
        assert_eq!(variant_amount(&mem, 0, 0x1000, 0x4000, &mut [0; 48]), Ok(0));
        assert!(variant_amount(&mem, 0, 0x1000, 0x2000, &mut [0; 47]).is_err());
        assert!(
            variant_amount(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes[..0xaf]
                },
                0,
                0x1000,
                0x2000,
                &mut [0; 48]
            )
            .is_err()
        );
        bytes[0x88..0x90].copy_from_slice(&(-1i64).to_le_bytes());
        assert!(
            variant_amount(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000,
                0x2000,
                &mut [0; 48]
            )
            .is_err()
        );
    }

    #[test]
    fn native_owners_follow_expeditionary_roster_wing_and_controller() {
        use crate::memory::SliceMemory;
        let mut bytes = vec![0u8; 0xc00];
        let write32 = |bytes: &mut [u8], at: usize, value: i32| {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes())
        };
        write32(&mut bytes, crate::cunit::ROSTER_OWNER, 4999);
        write32(&mut bytes, crate::cunit::LOGICAL_OWNER, 1234);
        for expeditionary in [-1, 0, 1500] {
            write32(&mut bytes, crate::cunit::EXPEDITIONARY_OWNER, expeditionary);
            assert_eq!(
                army_owner(
                    &SliceMemory {
                        base: 0x1000,
                        bytes: &bytes
                    },
                    0x1000
                )
                .unwrap(),
                if expeditionary > 0 { 1500 } else { 4999 }
            );
        }
        bytes[0x38..0x40].copy_from_slice(&0x1100u64.to_le_bytes());
        bytes[0x140..0x148].copy_from_slice(&0x1200u64.to_le_bytes());
        write32(&mut bytes, 0x290, 5000);
        for direct in [0, 1500] {
            write32(&mut bytes, 0x9c4, direct);
            assert_eq!(
                air_owner(
                    &SliceMemory {
                        base: 0x1000,
                        bytes: &bytes
                    },
                    0x1000
                )
                .unwrap(),
                if direct > 0 { 1500 } else { 5000 }
            );
        }
        bytes[0x50..0x58].copy_from_slice(&0x1300u64.to_le_bytes());
        write32(&mut bytes, 0x3cc, 4999);
        assert_eq!(
            garrison_owner(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0x1000
            ),
            Ok(4999)
        );
        assert!(
            garrison_owner(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes[..0x3cf]
                },
                0x1000
            )
            .is_err()
        );
    }
}
