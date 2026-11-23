
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x029f_0218);
pub const READ_EXTENT: usize = 0x18;
pub const MONTH: usize = 0x08;
pub const CONVOYS: usize = 0x0c;
pub const KILLER_COUNTRY: usize = 0x10;
pub const OWNER_COUNTRY: usize = 0x14;

use engine_native_memory::{Memory, cpdx_array::Array, read, walk::{Buffer, Outcome}};
use engine_native::cgame_state;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub month: i32,
    pub killer_country: u32,
    pub owner_country: u32,
    pub convoys: i32,
    pub ordinal: u32,
}

impl Row {
    pub fn key(self) -> u128 {
        (u128::from(self.month as u32) << 64)
            | (u128::from(self.killer_country) << 32)
            | u128::from(self.owner_country)
    }
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    walk_buffer(memory, base, game_state, Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(memory, base, game_state, Buffer::Growing(output))
}

fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: Buffer<'_, Row>,
) -> Outcome {
    match walk_inner(memory, base, game_state, output) {
        Ok(outcome) => outcome,
        Err(reason) => Outcome::Unavailable { reason },
    }
}

fn walk_inner(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: Buffer<'_, Row>,
) -> Result<Outcome, &'static str> {
    let array = game_state
        .checked_add(cgame_state::SUNK_CONVOY_HISTORY)
        .and_then(|address| Array::read(memory, address, 8))
        .ok_or("sunk convoy history vector unreadable or invalid")?;
    let vtable = engine_sites::at(base, VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    output.ensure(array.count as usize)?;
    let len = output.len().min(array.count as usize);
    for (ordinal, row) in output[..len].iter_mut().enumerate() {
        let pointer = array
            .element(ordinal as u32, 8)
            .and_then(|address| read::<u64>(memory, address))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|&pointer| pointer != 0)
            .ok_or("sunk convoy history pointer unreadable or null")?;
        let raw = read::<[u8; READ_EXTENT]>(memory, pointer)
            .ok_or("sunk convoy history record unreadable")?;
        if u64::from_le_bytes(raw[..8].try_into().expect("fixed span")) != vtable {
            return Err("sunk convoy history vtable mismatch");
        }
        *row = Row {
            month: i32::from_le_bytes(raw[MONTH..CONVOYS].try_into().expect("fixed span")),
            convoys: i32::from_le_bytes(
                raw[CONVOYS..KILLER_COUNTRY].try_into().expect("fixed span"),
            ),
            killer_country: u32::from_le_bytes(
                raw[KILLER_COUNTRY..OWNER_COUNTRY]
                    .try_into()
                    .expect("fixed span"),
            ),
            owner_country: u32::from_le_bytes(
                raw[OWNER_COUNTRY..READ_EXTENT]
                    .try_into()
                    .expect("fixed span"),
            ),
            ordinal: ordinal as u32,
        };
    }
    if len < array.count as usize {
        Ok(Outcome::Truncated {
            len,
            unread_rows: array.count - len as u32,
        })
    } else {
        Ok(Outcome::Rows { len })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x700];
        bytes[cgame_state::SUNK_CONVOY_HISTORY..cgame_state::SUNK_CONVOY_HISTORY + 8]
            .copy_from_slice(&0x1600_u64.to_le_bytes());
        bytes[cgame_state::SUNK_CONVOY_HISTORY_CAPACITY
            ..cgame_state::SUNK_CONVOY_HISTORY_CAPACITY + 4]
            .copy_from_slice(&2_u32.to_le_bytes());
        bytes[cgame_state::SUNK_CONVOY_HISTORY_COUNT..cgame_state::SUNK_CONVOY_HISTORY_COUNT + 4]
            .copy_from_slice(&2_u32.to_le_bytes());
        bytes[0x600..0x608].copy_from_slice(&0x1620_u64.to_le_bytes());
        bytes[0x608..0x610].copy_from_slice(&0x1640_u64.to_le_bytes());
        for (offset, month, killer, owner, count) in [
            (0x620, i32::MIN, 0_u32, u32::MAX, i32::MAX),
            (0x640, i32::MAX, u32::MAX, 1, -1),
        ] {
            bytes[offset..offset + 8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
            bytes[offset + MONTH..offset + CONVOYS].copy_from_slice(&month.to_le_bytes());
            bytes[offset + CONVOYS..offset + KILLER_COUNTRY].copy_from_slice(&count.to_le_bytes());
            bytes[offset + KILLER_COUNTRY..offset + OWNER_COUNTRY]
                .copy_from_slice(&killer.to_le_bytes());
            bytes[offset + OWNER_COUNTRY..offset + READ_EXTENT]
                .copy_from_slice(&owner.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn history_preserves_full_native_values_unknown_killer_and_order() {
        let bytes = fixture();
        let mut rows = vec![Row::default(); 2];
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000,
                &mut rows
            ),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(
            rows[0],
            Row {
                month: i32::MIN,
                killer_country: 0,
                owner_country: u32::MAX,
                convoys: i32::MAX,
                ordinal: 0
            }
        );
        assert_eq!(rows[1].convoys, -1);
        assert_eq!(rows[1].ordinal, 1);
        assert_ne!(rows[0].key(), rows[1].key());
        assert_eq!(
            rows[0].key(),
            (u128::from(0x8000_0000_u32) << 64) | u128::from(u32::MAX)
        );
    }

    #[test]
    fn history_count_fault_and_empty_are_distinct() {
        let mut bytes = fixture();
        let mut rows = vec![Row::default(); 1];
        assert_eq!(
            count(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000,
            ),
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
        bytes[0x600..0x608].fill(0);
        assert!(matches!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000,
                &mut rows
            ),
            Outcome::Unavailable { .. }
        ));
        bytes[cgame_state::SUNK_CONVOY_HISTORY_COUNT..cgame_state::SUNK_CONVOY_HISTORY_COUNT + 4]
            .fill(0);
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000,
                &mut rows
            ),
            Outcome::Rows { len: 0 }
        );
        bytes[cgame_state::SUNK_CONVOY_HISTORY_CAPACITY
            ..cgame_state::SUNK_CONVOY_HISTORY_CAPACITY + 4]
            .copy_from_slice(&(-1_i32).to_le_bytes());
        assert!(matches!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000,
                &mut rows
            ),
            Outcome::Unavailable { .. }
        ));
    }

}
