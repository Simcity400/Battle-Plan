//! Dense scripted-flag storage shared by country, state and global scopes.

use crate::{Memory, cscript_flag, read, walk::Outcome};

/// CFlagManager primary vtable.
pub const VTABLE_RVA: usize = 0x0294_6bf8;
/// CFlagManager primary vtable pointer.
pub const VTABLE: usize = 0x00;
/// CFlagManager dense flag-row array pointer.
pub const ROWS: usize = 0x08;
/// CFlagManager packed u64 slot/entry counts.
pub const PACKED_SLOT_ENTRY_COUNT: usize = 0x10;
/// CFlagManager signed capacity.
pub const SLOT_COUNT: usize = 0x10;
/// CFlagManager signed live count.
pub const ENTRY_COUNT: usize = 0x14;
/// CFlagManager allocator pointer.
pub const ALLOCATOR: usize = 0x18;

/// Reads one caller-supplied scope without allocation, retries or engine calls.
/// The scope and owner form the replay key together with the native token.
/// Capacity stops before the next row and reports the exact omitted count.
/// Output is sorted by token and rejects duplicate identities in the read prefix.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    manager: usize,
    output: &mut [cscript_flag::Row],
) -> Outcome {
    walk_buffer(memory, base, manager, crate::walk::Buffer::Fixed(output))
}

pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    manager: usize,
    output: &mut Vec<cscript_flag::Row>,
) -> Outcome {
    walk_buffer(memory, base, manager, crate::walk::Buffer::Growing(output))
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    manager: usize,
    mut output: crate::walk::Buffer<'_, cscript_flag::Row>,
) -> Outcome {
    let result = (|| {
        let expected = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")?;
        let head = read::<[u8; 24]>(memory, manager).ok_or("flag manager header unreadable")?;
        if u64::from_le_bytes(head[0..8].try_into().unwrap()) != expected as u64 {
            return Err("flag manager vtable mismatch");
        }
        let rows = usize::try_from(u64::from_le_bytes(head[8..16].try_into().unwrap()))
            .map_err(|_| "flag row pointer overflow")?;
        let capacity = i32::from_le_bytes(head[16..20].try_into().unwrap());
        let count = i32::from_le_bytes(head[20..24].try_into().unwrap());
        if capacity < 0 || count < 0 || count > capacity || (count != 0 && rows == 0) {
            return Err("flag manager header invalid");
        }
        let count = count as usize;
        output.ensure(count)?;
        let len = count.min(output.len());
        for (index, target) in output[..len].iter_mut().enumerate() {
            let address = index
                .checked_mul(cscript_flag::STRIDE)
                .and_then(|offset| rows.checked_add(offset))
                .ok_or("flag row address overflow")?;
            *target = cscript_flag::read_row(memory, base, address)?;
        }
        output[..len].sort_unstable_by_key(|row| row.token);
        if output[..len]
            .windows(2)
            .any(|pair| pair[0].token == pair[1].token)
        {
            return Err("duplicate flag token");
        }
        Ok(if len == count {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: (count - len) as u32,
            }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    const BASE: usize = 0x1000_0000;
    const MANAGER: usize = 0x2000;
    const DATA: usize = MANAGER + 0x40;

    fn fixture(tokens: &[u32]) -> Vec<u8> {
        let mut bytes = vec![0; 0x40 + tokens.len() * cscript_flag::STRIDE];
        bytes[0..8].copy_from_slice(&((BASE + VTABLE_RVA) as u64).to_le_bytes());
        bytes[8..16].copy_from_slice(&(DATA as u64).to_le_bytes());
        bytes[16..20].copy_from_slice(&(tokens.len() as i32).to_le_bytes());
        bytes[20..24].copy_from_slice(&(tokens.len() as i32).to_le_bytes());
        for (index, token) in tokens.iter().enumerate() {
            let start = 0x40 + index * cscript_flag::STRIDE;
            bytes[start..start + 8]
                .copy_from_slice(&((BASE + cscript_flag::VTABLE_RVA) as u64).to_le_bytes());
            bytes[start + 8..start + 12].copy_from_slice(&token.to_le_bytes());
            bytes[start + 16..start + 24]
                .copy_from_slice(&((BASE + cscript_flag::DATE_VTABLE_RVA) as u64).to_le_bytes());
            bytes[start + 24..start + 28].copy_from_slice(&i32::MIN.to_le_bytes());
            bytes[start + 32..start + 40].copy_from_slice(
                &((BASE + cscript_flag::DATE_SECONDARY_VTABLE_RVA) as u64).to_le_bytes(),
            );
            bytes[start + 40..start + 42].copy_from_slice(&i16::MIN.to_le_bytes());
            bytes[start + 42..start + 44].copy_from_slice(&(-1_i16).to_le_bytes());
        }
        bytes
    }

    fn sample(bytes: &[u8], output: &mut [cscript_flag::Row]) -> Outcome {
        walk(
            &SliceMemory {
                base: MANAGER,
                bytes,
            },
            BASE,
            MANAGER,
            output,
        )
    }

    #[test]
    fn growing_flags_capture_empty_storage_then_a_larger_native_roster() {
        let mut rows = Vec::new();
        for count in [5_000_u32, 10_000] {
            let tokens: Vec<_> = (0..count).rev().collect();
            let bytes = fixture(&tokens);
            let memory = SliceMemory {
                base: MANAGER,
                bytes: &bytes,
            };
            let mut expected = vec![cscript_flag::Row::default(); count as usize];
            assert_eq!(
                sample(&bytes, &mut expected),
                Outcome::Rows {
                    len: count as usize
                }
            );
            assert_eq!(
                walk_growing(&memory, BASE, MANAGER, &mut rows),
                Outcome::Rows {
                    len: count as usize
                }
            );
            assert_eq!(&rows[..count as usize], expected.as_slice());
        }
    }

    #[test]
    fn preserves_independent_signed_fields_and_full_token_identity() {
        let bytes = fixture(&[u32::MAX, 0]);
        let mut output = [cscript_flag::Row::default(); 2];
        assert_eq!(sample(&bytes, &mut output), Outcome::Rows { len: 2 });
        assert_eq!(
            output[0],
            cscript_flag::Row {
                token: 0,
                set_hour: i32::MIN,
                value: i16::MIN,
                remaining_days: -1,
            }
        );
        assert_eq!(output[1].token, u32::MAX);
    }

    #[test]
    fn truncation_does_not_touch_unreadable_omitted_rows() {
        let bytes = fixture(&[3, 4, 5]);
        let mut output = [cscript_flag::Row::default(); 1];
        assert_eq!(
            sample(&bytes[..0x70], &mut output),
            Outcome::Truncated {
                len: 1,
                unread_rows: 2
            }
        );
        assert_eq!(output[0].token, 3);
    }

    #[test]
    fn rejects_duplicate_tokens_and_wrong_row_types() {
        let mut bytes = fixture(&[7, 7]);
        let mut output = [cscript_flag::Row::default(); 2];
        assert_eq!(
            sample(&bytes, &mut output),
            Outcome::Unavailable {
                reason: "duplicate flag token"
            }
        );
        bytes[0x40..0x48].fill(0);
        assert_eq!(
            sample(&bytes, &mut output),
            Outcome::Unavailable {
                reason: "script flag vtable mismatch"
            }
        );
    }

    #[test]
    fn distinguishes_empty_invalid_and_unreadable_managers() {
        let mut bytes = fixture(&[]);
        bytes[8..16].fill(0);
        assert_eq!(sample(&bytes, &mut []), Outcome::Rows { len: 0 });
        bytes[20..24].copy_from_slice(&(-1_i32).to_le_bytes());
        assert_eq!(
            sample(&bytes, &mut []),
            Outcome::Unavailable {
                reason: "flag manager header invalid"
            }
        );
        assert_eq!(
            sample(&bytes[..23], &mut []),
            Outcome::Unavailable {
                reason: "flag manager header unreadable"
            }
        );
    }
}
