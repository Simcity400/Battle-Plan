//! Exact combat/result operands at the two native retained-result calls.
//!
//! `create_result@1415B2790` independently registers a new result before
//! CALL 1415B29C0, or chooses an existing province result before CALL
//! 1415B2AC3. Both calls receive RCX=result, RDX=complete CNavalCombat.
//! No result stores the source reference afterward (140CD6600/140CD5840).
//! Stack-local strikes retain the live invalid reference, rather than a
//! unique identity (1413CB400; 140BA8DB0/140BA8F30/140BA9130).
//! Creation can be skipped: result-emitted +114, neither eligible side
//! (1413CD470), or neither side with qualifying evidence (14160CF80). None
//! reaches either CALL, so a source observer appends no outcome event.
//! Native identity minting asserts main-thread access (14220A450), while
//! fill/merge check the native gamestate thread-forbid operand. Ordinary
//! finalization follows the hourly manager (140BA9890 -> 140BAAF60 ->
//! 1415B2210 -> 1413CD9F0 vslot+B8); temporary strikes also follow the
//! coordinated-strike effect (140355BB0 -> 140F6ED50 -> 140F6DB20).

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

/// Five-byte CALL to fill; the result reference is already final from
/// RegisterIdentity@1422097C0, called at 1415B2999. Return PC 1415B29C5.
/// No branch target lies inside the patched CALL; preserve the native frame.
pub const NEW: Site = Site {
    name: "naval_result_new",
    rva: 0x015b_29c0,
    sentinel_len: 16,
    sentinel: [
        0xe8, 0x3b, 0x3c, 0x72, 0xff, 0x80, 0x3d, 0x26, 0x62, 0x01, 0x02, 0x00, 0x0f, 0x84, 0xc7,
        0x00,
    ],
};

/// Five-byte CALL to merge; RCX is the existing result selected by
/// 1415B2930-55. Return PC 1415B2AC8 jumps to the native epilogue.
/// Verify only the owned 13 bytes: 1415B2AD0 starts the next function.
pub const MERGE: Site = Site {
    name: "naval_result_merge",
    rva: 0x015b_2ac3,
    sentinel_len: 13,
    sentinel: [
        0xe8, 0x78, 0x2d, 0x72, 0xff, 0xeb, 0xde, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc, 0x48, 0x89,
        0x5c,
    ],
};

/// Original CALL targets; never invoke a relocated mid-function continuation.
pub const TARGETS: [usize; 2] = [
    cnaval_combat_results::FILL_FROM_COMBAT_RVA,
    cnaval_combat_results::MERGE_RVA,
];

/// CNavalCombat constructor 1415AFEB0: RCX=complete object, RAX=same object.
/// Its first relocated instruction only writes RBX to caller home space.
/// A naked entry can pass the return PC in RDX without changing native RCX.
pub const CONSTRUCTOR: Site = Site {
    name: "naval_combat_constructor",
    rva: 0x015a_feb0,
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

/// Every direct constructor caller in the pinned export, verified by asm.
/// Fresh persistent callers mint/register after construction; the load
/// factory 140BAAE20 restores the saved reference and is not a native birth.
/// The three temporary strike functions resolve/finalize/destroy in one call
/// without registering the complete object's CReferenceObject at +0x10.
pub fn constructor_caller(return_rva: usize) -> BirthKind {
    match return_rva {
        // 140BA8410, 140BA8590, 140BA8B00, 140BAA780.
        0x00ba_845d | 0x00ba_85dd | 0x00ba_8b4f | 0x00ba_aad3 => BirthKind::Persistent,
        // Naval unit, convoy and port strike: 140BA8DB0/8F30/9130.
        0x00ba_8de3 | 0x00ba_8fe9 | 0x00ba_917c => BirthKind::Transient,
        0x00ba_aed9 => BirthKind::Loaded,
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

/// Native raw hour, also written into result+118 by fill/merge. The combat's
/// +40/+44 day/duration are counters, not an absolute birth (1413CB400,
/// 1415B1210, 1415B2240), so a constructor observer uses this live clock.
pub fn creation_hour(memory: &impl Memory, base: usize) -> Result<i32, &'static str> {
    let state = field::<u64>(memory, base, cgame_state::SINGLETON_RVA)? as usize;
    if state == 0 {
        return Err("naval source game state is null");
    }
    field(memory, state, cgame_state::CURRENT_HOUR)
}

/// Storage demand from live possible emitters, without a roster or bucket
/// walk: active combat slots (140BA9890/140BAAF60) plus all registered wings
/// (140C40EB0 uses type 69, 140F445E0 registers it). Include every wing so
/// mission changes and coordinated port strikes (140F6ED50, enabled mask)
/// cannot evade the census. This is population, not a maximum event count:
/// 140F6CF90/140F6ED50 invoke 140F6DB20 once per admitted target, so one wing
/// can emit several strikes. Observed demand and live headroom certify bursts.
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
    // RegisterIdentity@1422097C0 writes reference+10, here complete result+10.
    if !boolean(memory, result, 0x10)? || result_reference == invalid {
        return Err("naval source result reference is unregistered");
    }
    let combat_reference = field::<u64>(memory, combat, cnaval_combat::HANDLE)?;
    // Same native writer, with receiver complete combat+10: complete+20.
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
        for caller in [0x00ba_845d, 0x00ba_85dd, 0x00ba_8b4f, 0x00ba_aad3] {
            assert_eq!(constructor_caller(caller), BirthKind::Persistent);
            assert_eq!(constructor_caller(caller + 1), BirthKind::Unknown);
        }
        for caller in [0x00ba_8de3, 0x00ba_8fe9, 0x00ba_917c] {
            assert_eq!(constructor_caller(caller), BirthKind::Transient);
        }
        assert_eq!(constructor_caller(0x00ba_aed9), BirthKind::Loaded);
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
    fn population_counts_five_thousand_wings_and_combats_without_roster_reads() {
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
        assert_eq!(MERGE.rva + MERGE.sentinel_len, 0x015b_2ad0);
    }
}
