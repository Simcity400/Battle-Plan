
use engine_native_memory::{Memory, cpdx_array::Array, walk::{self, Outcome}};
use engine_native::{cgame_state, cstate};
use engine_sites::Rva;

pub mod changes;
pub mod hover;
pub mod map_icons;

pub const VTABLE_RVA: Rva = Rva(0x029f_aa20);
pub const STATE: usize = 0x08;
pub const LEVEL: usize = 0x10;
pub const SPEED: usize = 0x18;
pub const COMPLIANCE: usize = 0x40;
pub const COMPLIANCE_SPEED: usize = 0x48;
pub const COUNTRY: usize = 0x50;
pub const COMPLIANCE_MODIFIER: usize = 0x58;
pub const RESISTANCE_MODIFIER: usize = 0x118;
pub const ACTIVE: usize = 0x208;
pub const FIXED_POINT_SCALE: i64 = 100_000;
pub const MAX_LEVEL: i64 = 10_000_000;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x0101_5520);
pub const UPDATE_RVA: Rva = Rva(0x0101_c5e0);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub state: u32,
    pub country: u32,
    pub level: i64,
    pub speed: i64,
    pub compliance: i64,
    pub compliance_speed: i64,
    pub active: u8,
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    let Some(array) = game_state
        .checked_add(cgame_state::STATES)
        .and_then(|address| Array::read(memory, address, 8))
    else {
        return Outcome::Unavailable {
            reason: "resistance state array unreadable or invalid",
        };
    };
    if array.count < 2 {
        return Outcome::Unavailable {
            reason: "resistance state table has no state slots",
        };
    }
    let outcome = walk::pointers(memory, array, output, |id, state| {
        read_state(memory, base, state, id)
    });
    if outcome == (Outcome::Rows { len: 0 }) {
        Outcome::Unavailable {
            reason: "resistance state table has no readable states",
        }
    } else {
        outcome
    }
}

pub fn read_state(
    memory: &impl Memory,
    base: usize,
    state: usize,
    id: u32,
) -> Result<Row, &'static str> {
    let state_vtable =
        engine_sites::at(base, cstate::VTABLE_RVA).ok_or("module address overflow")?;
    let resistance_vtable = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    let id_address = state
        .checked_add(cstate::ID)
        .ok_or("resistance state address overflow")?;
    let mut vtable_bytes = [0; 8];
    let mut id_bytes = [0; 4];
    let completed =
        memory.copy_scattered([state, id_address], &mut [&mut vtable_bytes, &mut id_bytes]);
    if completed < 1 || u64::from_le_bytes(vtable_bytes) != state_vtable as u64 {
        return Err("resistance state vtable mismatch or unreadable");
    }
    if completed < 2 {
        return Err("resistance state id unreadable");
    }
    if u32::from_le_bytes(id_bytes) != id {
        return Err("resistance state id differs from slot");
    }
    let resistance = state
        .checked_add(cstate::RESISTANCE)
        .ok_or("resistance address overflow")?;
    let compliance_address = resistance
        .checked_add(COMPLIANCE)
        .ok_or("resistance address overflow")?;
    let active_address = resistance
        .checked_add(ACTIVE)
        .ok_or("resistance address overflow")?;
    let mut header = [0; 32];
    let mut compliance = [0; 20];
    let mut active = [0; 1];
    let completed = memory.copy_scattered(
        [resistance, compliance_address, active_address],
        &mut [&mut header, &mut compliance, &mut active],
    );
    if completed < 1 {
        return Err("resistance header unreadable");
    }
    if u64::from_le_bytes(header[0..8].try_into().unwrap()) != resistance_vtable as u64 {
        return Err("resistance vtable mismatch");
    }
    if u64::from_le_bytes(header[STATE..STATE + 8].try_into().unwrap()) != state as u64 {
        return Err("resistance state backlink mismatch");
    }
    if completed < 2 {
        return Err("resistance compliance or country unreadable");
    }
    if completed < 3 {
        return Err("resistance active byte unreadable");
    }
    Ok(Row {
        state: id,
        country: u32::from_le_bytes(compliance[16..20].try_into().unwrap()),
        level: i64::from_le_bytes(header[LEVEL..LEVEL + 8].try_into().unwrap()),
        speed: i64::from_le_bytes(header[SPEED..SPEED + 8].try_into().unwrap()),
        compliance: i64::from_le_bytes(compliance[0..8].try_into().unwrap()),
        compliance_speed: i64::from_le_bytes(compliance[8..16].try_into().unwrap()),
        active: active[0],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::{cpdx_array, memory::SliceMemory};

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        let index = address - 0x1000;
        bytes[index..index + value.len()].copy_from_slice(value);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x2000];
        put(
            &mut bytes,
            0x1000 + cgame_state::STATES,
            &0x1300_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x1000 + cgame_state::STATES + cpdx_array::CAPACITY,
            &4_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x1000 + cgame_state::STATES + cpdx_array::COUNT,
            &4_u32.to_le_bytes(),
        );
        put(&mut bytes, 0x1308, &0x1500_u64.to_le_bytes());
        put(&mut bytes, 0x1318, &0x2000_u64.to_le_bytes());
        for (state, id) in [(0x1500, 1_u32), (0x2000, 3)] {
            put(
                &mut bytes,
                state,
                &(cstate::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(&mut bytes, state + cstate::ID, &id.to_le_bytes());
            let resistance = state + cstate::RESISTANCE;
            put(&mut bytes, resistance, &(VTABLE_RVA.0 as u64).to_le_bytes());
            put(
                &mut bytes,
                resistance + STATE,
                &(state as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                resistance + COUNTRY,
                &0x8000_0001_u32.to_le_bytes(),
            );
            put(
                &mut bytes,
                resistance + LEVEL,
                &(i64::from(i32::MAX) + 9).to_le_bytes(),
            );
            put(
                &mut bytes,
                resistance + SPEED,
                &(-8_765_432_100_i64).to_le_bytes(),
            );
            put(
                &mut bytes,
                resistance + COMPLIANCE,
                &1_234_567_i64.to_le_bytes(),
            );
            put(
                &mut bytes,
                resistance + COMPLIANCE_SPEED,
                &(-123_456_i64).to_le_bytes(),
            );
            put(&mut bytes, resistance + ACTIVE, &[id as u8]);
        }
        bytes
    }

    #[test]
    fn occupation_hover_keeps_inactive_levels_without_reading_active_only_sources() {
        let mut bytes = fixture();
        let state = 0x1500;
        put(&mut bytes, state + cstate::RESISTANCE + ACTIVE, &[0]);
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let row = hover::read_inputs(
            &memory,
            0,
            0x1000,
            cstate::Source {
                pointer: state,
                id: 1,
            },
        )
        .unwrap();
        assert_eq!(row.resistance, read_state(&memory, 0, state, 1).unwrap());
        assert_eq!(row.sources, [0; 4]);
        assert!(row.effects.iter().all(Vec::is_empty));
        assert!(row.added.is_empty());
    }

    #[test]
    fn resistance_preserves_full_qwords_and_counts_capacity_after_null_holes() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut output = vec![Row::default(); 3];
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut output),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(
            output[0],
            Row {
                state: 1,
                country: 0x8000_0001,
                level: i64::from(i32::MAX) + 9,
                speed: -8_765_432_100,
                compliance: 1_234_567,
                compliance_speed: -123_456,
                active: 1,
            }
        );
        assert_eq!(output[1].state, 3);
        assert_eq!(output[1].active, 3);
        assert_eq!(
            walk_buffer(&memory, 0, 0x1000, engine_native_memory::walk::Buffer::Count),
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
    }

    #[test]
    fn resistance_authentication_and_faults_never_publish_a_prefix() {
        for (address, value, reason) in [
            (0x2000, 0, "resistance state vtable mismatch or unreadable"),
            (0x2000 + cstate::RESISTANCE, 0, "resistance vtable mismatch"),
            (
                0x2000 + cstate::RESISTANCE + STATE,
                0x1500,
                "resistance state backlink mismatch",
            ),
        ] {
            let mut bytes = fixture();
            put(&mut bytes, address, &u64::to_le_bytes(value));
            let mut output = vec![Row::default(); 3];
            assert_eq!(
                walk(
                    &SliceMemory {
                        base: 0x1000,
                        bytes: &bytes
                    },
                    0,
                    0x1000,
                    &mut output
                ),
                Outcome::Unavailable { reason }
            );
        }
        let bytes = fixture();
        let end = 0x2000 + cstate::RESISTANCE + ACTIVE - 0x1000;
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes[..end]
                },
                0,
                0x1000,
                &mut vec![Row::default(); 3]
            ),
            Outcome::Unavailable {
                reason: "resistance active byte unreadable"
            }
        );
    }
}
