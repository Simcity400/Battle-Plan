
use engine_native_memory::{Memory, cpdx_array::Array, read as read_memory, walk::Outcome};
use crate::cequipment_type;

pub const STRIDE: usize = 0x18;
pub const ARCHETYPE: usize = 0x00;
pub const AMOUNT: usize = 0x10;
pub const FIXED_POINT_SCALE: i64 = 100_000;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub source_ordinal: u32,
    pub archetype_id: i32,
    pub amount_raw: i64,
    pub archetype_pointer: usize,
}

pub fn read(
    memory: &impl Memory,
    module_base: usize,
    address: usize,
    source_ordinal: u32,
) -> Result<Row, &'static str> {
    let bytes =
        read_memory::<[u8; STRIDE]>(memory, address).ok_or("equipment allocation unreadable")?;
    let archetype_pointer = usize::try_from(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
        .map_err(|_| "equipment archetype pointer overflow")?;
    let expected = engine_sites::at(module_base, cequipment_type::VTABLE_RVA)
        .ok_or("equipment archetype vtable address overflow")?;
    let vtable = read_memory::<u64>(memory, archetype_pointer)
        .ok_or("equipment archetype vtable unreadable")?;
    if vtable != expected as u64 {
        return Err("equipment archetype vtable mismatch");
    }
    let id_address = archetype_pointer
        .checked_add(cequipment_type::ARCHETYPE_ID)
        .ok_or("equipment archetype id address overflow")?;
    let archetype_id =
        read_memory::<i32>(memory, id_address).ok_or("equipment archetype id unreadable")?;
    Ok(Row {
        source_ordinal,
        archetype_id,
        amount_raw: i64::from_le_bytes(bytes[AMOUNT..STRIDE].try_into().unwrap()),
        archetype_pointer,
    })
}

pub fn walk(memory: &impl Memory, module_base: usize, array: Array, output: &mut [Row]) -> Outcome {
    let len = output.len().min(array.count as usize);
    for (ordinal, destination) in output[..len].iter_mut().enumerate() {
        let Some(address) = array.element(ordinal as u32, STRIDE) else {
            return Outcome::Unavailable {
                reason: "equipment allocation array address overflow",
            };
        };
        match read(memory, module_base, address, ordinal as u32) {
            Ok(row) => *destination = row,
            Err(reason) => return Outcome::Unavailable { reason },
        }
    }
    let unread_rows = array.count - len as u32;
    if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    #[test]
    fn allocations_preserve_duplicates_and_signed_extrema_and_authenticate_archetypes() {
        let mut bytes = vec![0_u8; 0x600];
        bytes[0x100..0x108].copy_from_slice(&(cequipment_type::VTABLE_RVA.0 as u64).to_le_bytes());
        let id = 0x100 + cequipment_type::ARCHETYPE_ID;
        bytes[id..id + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        for (ordinal, amount) in [i64::MIN, 0, i64::MAX].into_iter().enumerate() {
            let offset = ordinal * STRIDE;
            bytes[offset..offset + 8].copy_from_slice(&0x1100_u64.to_le_bytes());
            bytes[offset + AMOUNT..offset + STRIDE].copy_from_slice(&amount.to_le_bytes());
        }
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut rows = [Row::default(); 3];
        let array = Array {
            data: 0x1000,
            count: 3,
        };
        assert_eq!(walk(&memory, 0, array, &mut rows), Outcome::Rows { len: 3 });
        for (ordinal, amount) in [i64::MIN, 0, i64::MAX].into_iter().enumerate() {
            assert_eq!(
                rows[ordinal],
                Row {
                    source_ordinal: ordinal as u32,
                    archetype_id: -1,
                    amount_raw: amount,
                    archetype_pointer: 0x1100
                }
            );
        }
        assert_eq!(
            walk(&memory, 0, array, &mut rows[..1]),
            Outcome::Truncated {
                len: 1,
                unread_rows: 2
            }
        );
        assert_eq!(
            walk(&memory, 1, array, &mut rows),
            Outcome::Unavailable {
                reason: "equipment archetype vtable mismatch"
            }
        );
        assert!(read(&memory, 0, 0x15f0, 0).is_err());
    }
}
