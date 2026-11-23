
use engine_native_memory::{Memory, cpdx_array::Array, read as read_memory, walk::Outcome};

pub const STRIDE: usize = 0x10;
pub const VARIANT: usize = 0x00;
pub const AMOUNT: usize = 0x08;
pub const FIXED_POINT_SCALE: i64 = 100_000;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub source_ordinal: u32,
    pub variant_handle: u64,
    pub amount_raw: i64,
    pub variant_pointer: usize,
}

pub fn read(
    memory: &impl Memory,
    module_base: usize,
    address: usize,
    source_ordinal: u32,
) -> Result<Row, &'static str> {
    let bytes = read_memory::<[u8; STRIDE]>(memory, address).ok_or("equipment entry unreadable")?;
    let variant_pointer = usize::try_from(u64::from_le_bytes(
        bytes[VARIANT..AMOUNT].try_into().unwrap(),
    ))
    .map_err(|_| "equipment variant pointer overflow")?;
    let amount_raw = i64::from_le_bytes(bytes[AMOUNT..STRIDE].try_into().unwrap());
    let variant_handle = crate::equipment_variant_identity::identity(memory, module_base, variant_pointer)?;
    Ok(Row {
        source_ordinal,
        variant_handle,
        amount_raw,
        variant_pointer,
    })
}

pub fn walk(memory: &impl Memory, module_base: usize, array: Array, output: &mut [Row]) -> Outcome {
    let len = output.len().min(array.count as usize);
    for (ordinal, destination) in output[..len].iter_mut().enumerate() {
        let Some(address) = array.element(ordinal as u32, STRIDE) else {
            return Outcome::Unavailable {
                reason: "equipment array address overflow",
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

    const BASE: usize = 0x1000;
    const MODULE: usize = 0x140000000;
    const VARIANT_ADDRESS: usize = BASE + 0x100;

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x180];
        bytes[0x100..0x108]
            .copy_from_slice(&((MODULE + crate::equipment_variant_identity::VTABLE_RVA.0) as u64).to_le_bytes());
        bytes[0x108..0x110].copy_from_slice(&(0xfedc_ba98_0000_0046_u64).to_le_bytes());
        for (index, amount) in [0_i64, i64::MIN, i64::MAX].into_iter().enumerate() {
            let offset = index * STRIDE;
            bytes[offset..offset + AMOUNT].copy_from_slice(&(VARIANT_ADDRESS as u64).to_le_bytes());
            bytes[offset + AMOUNT..offset + STRIDE].copy_from_slice(&amount.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn signed_amounts_full_handles_duplicates_and_order_are_preserved() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let mut output = [Row::default(); 3];
        assert_eq!(
            walk(
                &memory,
                MODULE,
                Array {
                    data: BASE,
                    count: 3
                },
                &mut output
            ),
            Outcome::Rows { len: 3 }
        );
        for (ordinal, amount) in [0_i64, i64::MIN, i64::MAX].into_iter().enumerate() {
            assert_eq!(
                output[ordinal],
                Row {
                    source_ordinal: ordinal as u32,
                    variant_handle: 0xfedc_ba98_0000_0046,
                    amount_raw: amount,
                    variant_pointer: VARIANT_ADDRESS,
                }
            );
        }
    }

    #[test]
    fn unread_entries_beyond_capacity_are_truncated_not_falsely_authenticated() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let mut output = [Row::default(); 1];
        assert_eq!(
            walk(
                &memory,
                MODULE,
                Array {
                    data: BASE,
                    count: 500
                },
                &mut output
            ),
            Outcome::Truncated {
                len: 1,
                unread_rows: 499
            }
        );
        assert_eq!(
            walk(
                &memory,
                MODULE,
                Array {
                    data: usize::MAX,
                    count: 3
                },
                &mut []
            ),
            Outcome::Truncated {
                len: 0,
                unread_rows: 3
            }
        );
        assert_eq!(
            walk(&memory, MODULE, Array { data: 0, count: 0 }, &mut output),
            Outcome::Rows { len: 0 }
        );
    }

    #[test]
    fn invalid_variant_or_partial_row_makes_the_whole_table_unavailable() {
        let mut bytes = fixture();
        bytes[STRIDE..STRIDE + AMOUNT].fill(0);
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let mut output = [Row::default(); 3];
        assert!(matches!(
            walk(
                &memory,
                MODULE,
                Array {
                    data: BASE,
                    count: 3
                },
                &mut output
            ),
            Outcome::Unavailable { .. }
        ));
        let bytes = fixture();
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        assert_eq!(
            read(&memory, MODULE, BASE + bytes.len() - 15, 0),
            Err("equipment entry unreadable")
        );
    }
}
