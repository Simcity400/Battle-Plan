//! Combat/result operands at retained-result fill and merge calls.
//!
//! Creation registers a new result or chooses an existing province result before
//! the call. Both receive RCX=result and RDX=complete combat; results do not retain
//! the source reference afterward. Temporary strikes keep an invalid reference.
//! Already-emitted combats, ineligible sides and sides without qualifying outcomes
//! skip both calls, so the observer appends no outcome event. Identity assignment
//! requires the main thread; fill and merge check the game-state thread guard.
//! Ordinary hourly finalization and coordinated strikes can both reach these sites.

use crate::{
    Memory, cair_wing, cgame_state, cnaval_combat, cnaval_combat_results, cpdx_array::Array,
    creference_registry, read,
};

pub struct Site {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
    pub sentinel_len: usize,
}

/// Five-byte fill call after result identity assignment. No branch target
/// lies inside the patched call; preserve the original frame.
pub const NEW: Site = Site {
    name: "naval_result_new",
    rva: 0x0166_10e0,
    sentinel_len: 16,
    sentinel: [
        0xe8, 0x0b, 0x4b, 0x6e, 0xff, 0x80, 0x3d, 0x25, 0xe5, 0x02, 0x02, 0x00, 0x0f, 0x84, 0xc7,
        0x00,
    ],
};

/// Five-byte merge call with the existing result in RCX, returning to the
/// original epilogue. Verify only the owned 13-byte sentinel.
pub const MERGE: Site = Site {
    name: "naval_result_merge",
    rva: 0x0166_11e3,
    sentinel_len: 13,
    sentinel: [
        0xe8, 0x48, 0x3c, 0x6e, 0xff, 0xeb, 0xde, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0x48, 0x89,
        0x5c,
    ],
};

/// Original CALL targets; never invoke a relocated mid-function continuation.
pub const TARGETS: [usize; 2] = [
    cnaval_combat_results::FILL_FROM_COMBAT_RVA,
    cnaval_combat_results::MERGE_RVA,
];

/// Combat constructor entry: RCX is the complete object, RAX the same object.
/// Its first relocated instruction spills RBX to caller home space. An entry
/// observer can pass the return PC in RDX while preserving RCX.
pub const CONSTRUCTOR: Site = Site {
    name: "naval_combat_constructor",
    rva: 0x0165_e550,
    sentinel_len: 16,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18,
        0x57,
    ],
};

pub const SITES: [&Site; 3] = [&NEW, &MERGE, &CONSTRUCTOR];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BirthKind {
    #[default]
    Unknown,
    Persistent,
    Transient,
    Loaded,
}

/// Direct constructor callers. Fresh persistent combats register after
/// construction; save loading restores a reference rather than creating a birth.
/// Temporary unit, convoy and port strikes resolve, finalize and destroy in one
/// call without registering the complete object's reference subobject.
pub fn constructor_caller(return_rva: usize) -> BirthKind {
    match return_rva {
        0x00c0_b1bd | 0x00c0_b33d | 0x00c0_b8af | 0x00c0_d7a3 => BirthKind::Persistent,

        0x00c0_bb43 | 0x00c0_bd49 | 0x00c0_bedc => BirthKind::Transient,
        0x00c0_dba9 => BirthKind::Loaded,
        _ => BirthKind::Unknown,
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Inputs {
    pub game_hour: i32,
    pub result: u64,
    pub combat: u64,
    pub registered: bool,
    pub port_strike: bool,
    pub naval_strike: bool,
}

/// Raw game hour copied into retained results. Combat day and duration fields
/// are counters, distinct from absolute birth, so construction uses the live clock.
pub fn creation_hour(memory: &impl Memory, base: usize) -> Result<i32, &'static str> {
    let state = field::<u64>(memory, base, cgame_state::SINGLETON_RVA)? as usize;
    if state == 0 {
        return Err("naval source game state is null");
    }
    field(memory, state, cgame_state::CURRENT_HOUR)
}

/// Storage demand from active combat slots and every registered wing,
/// without walking a roster or buckets. Including all wings covers mission
/// changes and coordinated port strikes. Population is distinct from event count:
/// one wing can emit several targets. Observed demand and live headroom cover bursts.
pub fn population(memory: &impl Memory, base: usize) -> Result<usize, &'static str> {
    let state = field::<u64>(memory, base, cgame_state::SINGLETON_RVA)? as usize;
    if state == 0 {
        return Err("naval source game state is null");
    }
    let combat_array = state
        .checked_add(cgame_state::COMBATS + 8)
        .ok_or("naval source combat registry address overflow")?;
    let combats = Array::read(memory, combat_array, 8)
        .ok_or("naval source combat registry unreadable or invalid")?;
    (combats.count as usize)
        .checked_add(creference_registry::entry_count(
            memory,
            base,
            cair_wing::TYPE,
        )?)
        .ok_or("naval source emitter population overflow")
}

/// Read after the original fill/merge returns. All operands are scalar,
/// guarded and allocation-free; no engine function is called.
pub fn read_inputs(
    memory: &impl Memory,
    base: usize,
    result: usize,
    combat: usize,
) -> Result<Inputs, &'static str> {
    for (object, vtable) in [
        (result, cnaval_combat_results::VTABLE_RVA),
        (combat, cnaval_combat::VTABLE_RVA),
    ] {
        if object == 0
            || field::<u64>(memory, object, 0)?
                != base
                    .checked_add(vtable)
                    .ok_or("naval source vtable address overflow")? as u64
        {
            return Err("naval source object identity mismatch");
        }
    }
    let invalid = field::<u64>(memory, base, creference_registry::INVALID_REFERENCE_RVA)?;
    let result_reference = field::<u64>(memory, result, cnaval_combat_results::HANDLE)?;
    // Identity registration writes the complete result's reference field.
    if !boolean(memory, result, 0x10)? || result_reference == invalid {
        return Err("naval source result reference is unregistered");
    }
    let combat_reference = field::<u64>(memory, combat, cnaval_combat::HANDLE)?;
    // The same identity writer selects the complete combat's reference subobject.
    let registered = boolean(memory, combat, 0x20)?;
    if registered == (combat_reference == invalid) {
        return Err("naval source combat reference registration mismatch");
    }
    let port_strike = boolean(memory, combat, cnaval_combat::RESULT_FLAG_A)?;
    let naval_strike = boolean(memory, combat, cnaval_combat::RESULT_FLAG_B)?;
    if !registered && !port_strike && !naval_strike {
        return Err("naval source unregistered non-strike combat");
    }
    let game_hour = creation_hour(memory, base)?;
    if field::<i32>(memory, result, cnaval_combat_results::RESULT_HOUR)? != game_hour {
        return Err("naval source result hour differs from native hour");
    }
    Ok(Inputs {
        game_hour,
        result: result_reference,
        combat: combat_reference,
        registered,
        port_strike,
        naval_strike,
    })
}

fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    let at = object
        .checked_add(offset)
        .ok_or("naval source operand address overflow")?;
    read(memory, at).ok_or("naval source operand unreadable")
}

fn boolean(memory: &impl Memory, object: usize, offset: usize) -> Result<bool, &'static str> {
    match field::<u8>(memory, object, offset)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err("naval source boolean is invalid"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Bytes(BTreeMap<usize, u8>);
    impl Bytes {
        fn put(&mut self, at: usize, bytes: &[u8]) {
            for (offset, byte) in bytes.iter().copied().enumerate() {
                self.0.insert(at + offset, byte);
            }
        }
    }
    impl Memory for Bytes {
        fn copy(&self, at: usize, output: &mut [u8]) -> bool {
            for (offset, destination) in output.iter_mut().enumerate() {
                let Some(byte) = self.0.get(&(at + offset)) else {
                    return false;
                };
                *destination = *byte;
            }
            true
        }
    }
    const BASE: usize = 0x1_4000_0000;
    const RESULT: usize = 0x1000;
    const COMBAT: usize = 0x2000;
    const STATE: usize = 0x3000;
    // Synthetic nonzero sentinel and references, never sampled native values.
    const INVALID: u64 = 0xfabc_0123_4567_89de;
    const RESULT_ID: u64 = 0xffff_8000_0000_1234;
    const COMBAT_ID: u64 = 0xff00_8000_0000_4321;
    fn fixture() -> Bytes {
        let mut bytes = Bytes::default();
        bytes.put(
            RESULT,
            &((BASE + cnaval_combat_results::VTABLE_RVA) as u64).to_le_bytes(),
        );
        bytes.put(
            COMBAT,
            &((BASE + cnaval_combat::VTABLE_RVA) as u64).to_le_bytes(),
        );
        bytes.put(
            BASE + creference_registry::INVALID_REFERENCE_RVA,
            &INVALID.to_le_bytes(),
        );
        bytes.put(
            RESULT + cnaval_combat_results::HANDLE,
            &RESULT_ID.to_le_bytes(),
        );
        bytes.put(RESULT + 0x10, &[1]);
        bytes.put(COMBAT + cnaval_combat::HANDLE, &COMBAT_ID.to_le_bytes());
        bytes.put(COMBAT + 0x20, &[1]);
        bytes.put(COMBAT + cnaval_combat::RESULT_FLAG_A, &[0, 0]);
        bytes.put(
            BASE + cgame_state::SINGLETON_RVA,
            &(STATE as u64).to_le_bytes(),
        );
        bytes.put(STATE + cgame_state::CURRENT_HOUR, &(-123i32).to_le_bytes());
        bytes.put(
            RESULT + cnaval_combat_results::RESULT_HOUR,
            &(-123i32).to_le_bytes(),
        );
        bytes
    }

    #[test]
    fn new_and_merge_keep_full_native_references_and_signed_hour() {
        let bytes = fixture();
        assert_eq!(
            read_inputs(&bytes, BASE, RESULT, COMBAT),
            Ok(Inputs {
                game_hour: -123,
                result: RESULT_ID,
                combat: COMBAT_ID,
                registered: true,
                port_strike: false,
                naval_strike: false,
            })
        );
        assert_eq!(creation_hour(&bytes, BASE), Ok(-123));
    }

    #[test]
    fn transient_strikes_keep_the_live_invalid_reference_explicitly() {
        let mut bytes = fixture();
        bytes.put(COMBAT + cnaval_combat::HANDLE, &INVALID.to_le_bytes());
        bytes.put(COMBAT + 0x20, &[0]);
        for flags in [[1, 0], [0, 1], [1, 1]] {
            bytes.put(COMBAT + cnaval_combat::RESULT_FLAG_A, &flags);
            let inputs = read_inputs(&bytes, BASE, RESULT, COMBAT).unwrap();
            assert_eq!(inputs.combat, INVALID);
            assert!(!inputs.registered);
            assert_eq!(inputs.port_strike, flags[0] != 0);
            assert_eq!(inputs.naval_strike, flags[1] != 0);
        }
        bytes.put(COMBAT + cnaval_combat::RESULT_FLAG_A, &[0, 0]);
        assert_eq!(
            read_inputs(&bytes, BASE, RESULT, COMBAT),
            Err("naval source unregistered non-strike combat")
        );
    }

    #[test]
    fn source_reads_refuse_torn_identity_hour_flags_and_short_reads() {
        let mut bytes = fixture();
        bytes.put(COMBAT + 0x20, &[0]);
        assert_eq!(
            read_inputs(&bytes, BASE, RESULT, COMBAT),
            Err("naval source combat reference registration mismatch")
        );
        bytes = fixture();
        bytes.put(RESULT + 0x10, &[0]);
        assert_eq!(
            read_inputs(&bytes, BASE, RESULT, COMBAT),
            Err("naval source result reference is unregistered")
        );
        bytes = fixture();
        bytes.put(RESULT, &0u64.to_le_bytes());
        assert_eq!(
            read_inputs(&bytes, BASE, RESULT, COMBAT),
            Err("naval source object identity mismatch")
        );
        bytes = fixture();
        bytes.put(COMBAT + cnaval_combat::RESULT_FLAG_A, &[2]);
        assert_eq!(
            read_inputs(&bytes, BASE, RESULT, COMBAT),
            Err("naval source boolean is invalid")
        );
        bytes = fixture();
        bytes.put(
            RESULT + cnaval_combat_results::RESULT_HOUR,
            &(-124i32).to_le_bytes(),
        );
        assert_eq!(
            read_inputs(&bytes, BASE, RESULT, COMBAT),
            Err("naval source result hour differs from native hour")
        );
        bytes = fixture();
        bytes.0.remove(&(COMBAT + cnaval_combat::HANDLE + 7));
        assert_eq!(
            read_inputs(&bytes, BASE, RESULT, COMBAT),
            Err("naval source operand unreadable")
        );
        assert_eq!(
            creation_hour(&bytes, usize::MAX),
            Err("naval source operand address overflow")
        );
    }

    #[test]
    fn constructor_callers_cover_fresh_temporary_and_load_without_guessing() {
        for caller in [0x00c0_b1bd, 0x00c0_b33d, 0x00c0_b8af, 0x00c0_d7a3] {
            assert_eq!(constructor_caller(caller), BirthKind::Persistent);
            assert_eq!(constructor_caller(caller + 1), BirthKind::Unknown);
        }
        for caller in [0x00c0_bb43, 0x00c0_bd49, 0x00c0_bedc] {
            assert_eq!(constructor_caller(caller), BirthKind::Transient);
        }
        assert_eq!(constructor_caller(0x00c0_dba9), BirthKind::Loaded);
        assert_eq!(constructor_caller(0), BirthKind::Unknown);
    }

    const WING_TABLE: usize = 0x4000;
    const WING_ENTRIES: usize = 0x5000;
    const WING_CAPACITY: u32 = 8192;
    const WING_OVERFLOW: u8 = 15;
    fn population_fixture() -> Bytes {
        let mut bytes = fixture();
        let combat_array = STATE + cgame_state::COMBATS + 8;
        bytes.put(combat_array, &0x40000u64.to_le_bytes());
        bytes.put(combat_array + 8, &5000u32.to_le_bytes());
        bytes.put(combat_array + 12, &5000u32.to_le_bytes());
        bytes.put(
            BASE + creference_registry::LOW_ROOTS_RVA + cair_wing::TYPE as usize * 8,
            &(WING_TABLE as u64).to_le_bytes(),
        );
        bytes.put(
            WING_TABLE + creference_registry::ENTRIES,
            &(WING_ENTRIES as u64).to_le_bytes(),
        );
        bytes.put(
            WING_TABLE + creference_registry::COUNT,
            &5000u32.to_le_bytes(),
        );
        bytes.put(
            WING_TABLE + creference_registry::MASK,
            &(WING_CAPACITY - 1).to_le_bytes(),
        );
        bytes.put(
            WING_TABLE + creference_registry::OVERFLOW_DISTANCE,
            &[WING_OVERFLOW],
        );
        bytes.put(
            WING_ENTRIES
                + (WING_CAPACITY as usize + WING_OVERFLOW as usize)
                    * crate::reference_bucket::STRIDE
                + crate::reference_bucket::DISTANCE,
            &[255],
        );
        bytes
    }

    #[test]
    fn population_counts_wings_and_combats_without_roster_reads() {
        let mut bytes = population_fixture();
        assert_eq!(population(&bytes, BASE), Ok(10_000));
        // Every wing counts, even without reading its mission or plane count.
        // A null type-specific registry is the native successful empty case.
        bytes.put(
            BASE + creference_registry::LOW_ROOTS_RVA + cair_wing::TYPE as usize * 8,
            &0u64.to_le_bytes(),
        );
        assert_eq!(population(&bytes, BASE), Ok(5000));
        let combat_array = STATE + cgame_state::COMBATS + 8;
        bytes.put(combat_array, &0u64.to_le_bytes());
        bytes.put(combat_array + 8, &0u64.to_le_bytes());
        assert_eq!(population(&bytes, BASE), Ok(0));
    }

    #[test]
    fn population_refuses_short_or_invalid_native_headers() {
        let mut bytes = population_fixture();
        bytes.put(
            WING_TABLE + creference_registry::COUNT,
            &(WING_CAPACITY + 1).to_le_bytes(),
        );
        assert_eq!(population(&bytes, BASE), Err("reference header invalid"));
        bytes = population_fixture();
        bytes
            .0
            .remove(&(WING_TABLE + creference_registry::OVERFLOW_DISTANCE));
        assert_eq!(population(&bytes, BASE), Err("reference header unreadable"));
        bytes = population_fixture();
        bytes.put(
            STATE + cgame_state::COMBATS + 8 + 12,
            &5001u32.to_le_bytes(),
        );
        assert_eq!(
            population(&bytes, BASE),
            Err("naval source combat registry unreadable or invalid")
        );
        bytes = population_fixture();
        bytes.0.remove(&(STATE + cgame_state::COMBATS + 8 + 15));
        assert_eq!(
            population(&bytes, BASE),
            Err("naval source combat registry unreadable or invalid")
        );
        bytes = population_fixture();
        bytes.put(BASE + cgame_state::SINGLETON_RVA, &0u64.to_le_bytes());
        assert_eq!(
            population(&bytes, BASE),
            Err("naval source game state is null")
        );
        assert_eq!(
            population(&bytes, usize::MAX),
            Err("naval source operand address overflow")
        );
    }

    #[test]
    fn call_sentinels_decode_to_verified_native_targets() {
        assert_eq!(SITES.len(), 3);
        for (site, target) in SITES[..2].iter().copied().zip(TARGETS) {
            assert_eq!(site.sentinel[0], 0xe8);
            let relative = i32::from_le_bytes(site.sentinel[1..5].try_into().unwrap());
            assert_eq!(site.rva as i64 + 5 + relative as i64, target as i64);
        }
        assert_eq!(&CONSTRUCTOR.sentinel[..5], &[0x48, 0x89, 0x5c, 0x24, 0x08]);
        assert_eq!(NEW.sentinel_len, 16);
        assert_eq!(CONSTRUCTOR.sentinel_len, 16);
        assert_eq!(MERGE.sentinel_len, 13);
        assert_eq!(MERGE.rva + MERGE.sentinel_len, 0x0166_11f0);
    }
}
