
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    read_slice,
    walk::{Buffer, Outcome},
};
use engine_native::{cgame_state, ccountry_supply_system::world as csupply_system};

pub const STRIDE: usize = 0x20;
pub const CONTROLLER_INDEX: usize = 0x10;
pub const CAPITAL_COUNTRY_INDEX: usize = 0x12;
pub const SUPPLY_NODE_LEVEL: usize = 0x14;
pub const NAVAL_BASE_LEVEL: usize = 0x15;
pub const FLAGS: usize = 0x18;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub province: u32,
    pub controller_index: u16,
    pub capital_country_index: u16,
    pub supply_node_level: u8,
    pub naval_base_level: u8,
    pub flags: u8,
}

fn decode(province: u32, bytes: &[u8]) -> Row {
    Row {
        province,
        controller_index: u16::from_le_bytes(
            bytes[CONTROLLER_INDEX..CONTROLLER_INDEX + 2]
                .try_into()
                .unwrap(),
        ),
        capital_country_index: u16::from_le_bytes(
            bytes[CAPITAL_COUNTRY_INDEX..CAPITAL_COUNTRY_INDEX + 2]
                .try_into()
                .unwrap(),
        ),
        supply_node_level: bytes[SUPPLY_NODE_LEVEL],
        naval_base_level: bytes[NAVAL_BASE_LEVEL],
        flags: bytes[FLAGS],
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
    mut output: Buffer<'_, Row>,
) -> Outcome {
    let result = (|| {
        let root = csupply_system::read_root(memory, base, game_state)?;
        let provinces = game_state
            .checked_add(cgame_state::PROVINCES)
            .and_then(|p| Array::read(memory, p, 8))
            .ok_or("province array unreadable or invalid")?;
        let supply = root
            .checked_add(csupply_system::PROVINCE_DATA)
            .and_then(|p| Array::read(memory, p, STRIDE))
            .ok_or("province supply array unreadable or invalid")?;
        if supply.count < 2 || supply.count != provinces.count {
            return Err("province supply slot count differs from province array");
        }
        let available = (supply.count - 1) as usize;
        output.ensure(available)?;
        let len = available.min(output.len());
        const BATCH: usize = 64;
        let mut bytes = [0_u8; STRIDE * BATCH];
        for start in (0..len).step_by(BATCH) {
            let count = BATCH.min(len - start);
            let address = supply
                .element((start + 1) as u32, STRIDE)
                .ok_or("province supply address overflow")?;
            let used = (count - 1) * STRIDE + FLAGS + 1;
            if !read_slice(memory, address, &mut bytes[..used]) {
                return Err("province supply rows unreadable");
            }
            for index in 0..count {
                output[start + index] =
                    decode((start + index + 1) as u32, &bytes[index * STRIDE..]);
            }
        }
        Ok(if len == available {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: (available - len) as u32,
            }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }
    fn fixture(count: u32) -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x2000 + count as usize * STRIDE];
        put(
            &mut bytes,
            cgame_state::SUPPLY_SYSTEM,
            &0x1800_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x800,
            &(csupply_system::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x808,
            &(csupply_system::SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        for (at, pointer) in [
            (cgame_state::PROVINCES, 0x4000_u64),
            (0x800 + csupply_system::PROVINCE_DATA, 0x3000_u64),
        ] {
            put(&mut bytes, at, &pointer.to_le_bytes());
            put(&mut bytes, at + 8, &(count + 7).to_le_bytes());
            put(&mut bytes, at + 12, &count.to_le_bytes());
        }
        for slot in 1..count {
            let at = 0x2000 + slot as usize * STRIDE;
            put(&mut bytes, at, &(i64::MIN + i64::from(slot)).to_le_bytes());
            put(
                &mut bytes,
                at + 8,
                &(i64::MAX - i64::from(slot)).to_le_bytes(),
            );
            put(&mut bytes, at + 0x10, &u16::MAX.to_le_bytes());
            put(&mut bytes, at + 0x12, &0x8000_u16.to_le_bytes());
            put(&mut bytes, at + 0x14, &[1, 2, 3, 4, 0xff]);
            bytes[at + 0x19..at + STRIDE].fill(0xa5);
        }
        bytes
    }

    #[test]
    fn worker_rows_follow_the_original_native_count_without_a_short_hour() {
        let bytes = fixture(20_152);
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut rows = vec![Row::default(); 1];
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows),
            Outcome::Rows { len: 20_151 }
        );
        assert_eq!(rows[20_150].province, 20_151);
        assert_eq!(rows[20_150].flags, 0xff);
    }

    #[test]
    fn retained_operands_survive_batch_boundary_and_unrecorded_bytes() {
        let bytes = fixture(67);
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut rows = vec![Row::default(); 66];
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows),
            Outcome::Rows { len: 66 }
        );
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(row.province, index as u32 + 1);
            assert_eq!(
                (row.controller_index, row.capital_country_index),
                (u16::MAX, 0x8000)
            );
            assert_eq!(
                (row.supply_node_level, row.naval_base_level, row.flags),
                (1, 2, 0xff)
            );
        }
    }

    #[test]
    fn the_count_probe_counts_rows_without_reading_their_bytes() {
        let mut bytes = fixture(4);
        bytes.truncate(0x2000 + STRIDE + FLAGS + 1);
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            count(&memory, 0, 0x1000),
            Outcome::Truncated {
                len: 0,
                unread_rows: 3
            }
        );
        let mut rows = vec![Row::default(); 3];
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows),
            Outcome::Unavailable {
                reason: "province supply rows unreadable"
            }
        );
    }

    #[test]
    fn malformed_counts_and_root_are_unavailable() {
        let mut bytes = fixture(3);
        put(
            &mut bytes,
            0x800 + csupply_system::PROVINCE_DATA + 12,
            &4_u32.to_le_bytes(),
        );
        let mut rows = vec![Row::default(); 3];
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
        put(&mut bytes, 0x800, &0_u64.to_le_bytes());
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
            Outcome::Unavailable {
                reason: "supply system vtable mismatch"
            }
        );
    }
}
