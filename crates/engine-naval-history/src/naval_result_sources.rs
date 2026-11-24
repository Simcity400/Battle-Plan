
use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native::{cgame_state, cnaval_combat};
use crate::cnaval_combat_results;
use engine_native_parts::creference_registry;

pub struct Site {
    pub name: &'static str,
    pub rva: Rva,
    pub span: usize,
}

pub const NEW: Site = Site {
    name: "naval_result_new",
    rva: Rva(0x0166_1510),
    span: 16,
};

pub const MERGE: Site = Site {
    name: "naval_result_merge",
    rva: Rva(0x0166_1613),
    span: 13,
};

pub const TARGETS: [Rva; 2] = [
    cnaval_combat_results::FILL_FROM_COMBAT_RVA,
    cnaval_combat_results::MERGE_RVA,
];

pub const CONSTRUCTOR: Site = Site {
    name: "naval_combat_constructor",
    rva: Rva(0x0165_e980),
    span: 16,
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

pub const CONSTRUCTOR_CALLERS: [Rva; 8] = [
    Rva(0x00c0_b22d),
    Rva(0x00c0_b3ad),
    Rva(0x00c0_b91f),
    Rva(0x00c0_d813),
    Rva(0x00c0_bbb3),
    Rva(0x00c0_bdb9),
    Rva(0x00c0_bf4c),
    Rva(0x00c0_dc19),
];

pub fn constructor_caller(base: usize, return_address: usize) -> BirthKind {
    match engine_sites::which(base, return_address, &CONSTRUCTOR_CALLERS) {
        Some(Rva(0x00c0_b22d) | Rva(0x00c0_b3ad) | Rva(0x00c0_b91f) | Rva(0x00c0_d813)) => {
            BirthKind::Persistent
        }
        Some(Rva(0x00c0_bbb3) | Rva(0x00c0_bdb9) | Rva(0x00c0_bf4c)) => BirthKind::Transient,
        Some(Rva(0x00c0_dc19)) => BirthKind::Loaded,
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

pub fn creation_hour(memory: &impl Memory, base: usize) -> Result<i32, &'static str> {
    let state = global::<u64>(memory, base, cgame_state::SINGLETON_RVA)? as usize;
    if state == 0 {
        return Err("naval source game state is null");
    }
    field(memory, state, cgame_state::CURRENT_HOUR)
}

pub fn population(memory: &impl Memory, base: usize) -> Result<usize, &'static str> {
    let state = global::<u64>(memory, base, cgame_state::SINGLETON_RVA)? as usize;
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
            engine_native_parts::cair_wing_layout::TYPE,
        )?)
        .ok_or("naval source emitter population overflow")
}

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
                != engine_sites::at(base, vtable).ok_or("naval source vtable address overflow")?
                    as u64
        {
            return Err("naval source object identity mismatch");
        }
    }
    let invalid = global::<u64>(memory, base, creference_registry::INVALID_REFERENCE_RVA)?;
    let result_reference = field::<u64>(memory, result, cnaval_combat_results::HANDLE)?;
    if !boolean(memory, result, 0x10)? || result_reference == invalid {
        return Err("naval source result reference is unregistered");
    }
    let combat_reference = field::<u64>(memory, combat, cnaval_combat::HANDLE)?;
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

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    let at = object
        .checked_add(offset)
        .ok_or("naval source operand address overflow")?;
    read(memory, at).ok_or("naval source operand unreadable")
}

fn global<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    base: usize,
    key: Rva,
) -> Result<T, &'static str> {
    let at = engine_sites::at(base, key).ok_or("naval source operand address overflow")?;
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
    const INVALID: u64 = 0xfabc_0123_4567_89de;
    const RESULT_ID: u64 = 0xffff_8000_0000_1234;
    const COMBAT_ID: u64 = 0xff00_8000_0000_4321;
    fn fixture() -> Bytes {
        let mut bytes = Bytes::default();
        bytes.put(
            RESULT,
            &((BASE + cnaval_combat_results::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        bytes.put(
            COMBAT,
            &((BASE + cnaval_combat::VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        bytes.put(
            BASE + creference_registry::INVALID_REFERENCE_RVA.0,
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
            BASE + cgame_state::SINGLETON_RVA.0,
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
            BASE + creference_registry::LOW_ROOTS_RVA.0 + engine_native_parts::cair_wing_layout::TYPE as usize * 8,
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
                    * engine_native_parts::reference_bucket::STRIDE
                + engine_native_parts::reference_bucket::DISTANCE,
            &[255],
        );
        bytes
    }

    #[test]
    fn population_counts_wings_and_combats_without_roster_reads() {
        let mut bytes = population_fixture();
        assert_eq!(population(&bytes, BASE), Ok(10_000));
        bytes.put(
            BASE + creference_registry::LOW_ROOTS_RVA.0 + engine_native_parts::cair_wing_layout::TYPE as usize * 8,
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
        bytes.put(BASE + cgame_state::SINGLETON_RVA.0, &0u64.to_le_bytes());
        assert_eq!(
            population(&bytes, BASE),
            Err("naval source game state is null")
        );
        assert_eq!(
            population(&bytes, usize::MAX),
            Err("naval source operand address overflow")
        );
    }
}
