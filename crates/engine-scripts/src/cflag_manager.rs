
use engine_native_memory::{Memory, memory::RecordSources, read, walk::Outcome};
use crate::cscript_flag;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x029d_9d98);
pub const VTABLE: usize = 0x00;
pub const ROWS: usize = 0x08;
pub const SLOT_COUNT: usize = 0x10;
pub const ENTRY_COUNT: usize = 0x14;
pub const ALLOCATOR: usize = 0x18;

pub fn walk(
    memory: &impl Memory,
    base: usize,
    manager: usize,
    output: &mut Vec<cscript_flag::Row>,
) -> Outcome {
    walk_buffer(memory, base, manager, engine_native_memory::walk::Buffer::Growing(output))
}

pub const HEADER: usize = 24;
const ROW_BATCH: usize = 64;

pub fn daily_expiry_writes(memory: &impl Memory, manager: usize) -> bool {
    let (Some(rows), Some(count)) = (
        manager
            .checked_add(ROWS)
            .and_then(|address| read::<u64>(memory, address)),
        manager
            .checked_add(ENTRY_COUNT)
            .and_then(|address| read::<i32>(memory, address)),
    ) else {
        return true;
    };
    (0..count.max(0) as usize).any(|index| {
        index
            .checked_mul(cscript_flag::STRIDE)
            .and_then(|offset| offset.checked_add(cscript_flag::REMAINING_DAYS))
            .and_then(|offset| (rows as usize).checked_add(offset))
            .and_then(|address| read::<i16>(memory, address))
            .is_none_or(|days| days > 0)
    })
}

pub fn read_header(memory: &impl Memory, manager: usize) -> Option<[u8; HEADER]> {
    read::<[u8; HEADER]>(memory, manager)
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    manager: usize,
    output: engine_native_memory::walk::Buffer<'_, cscript_flag::Row>,
) -> Outcome {
    walk_header_buffer(memory, base, read_header(memory, manager), output)
}

pub fn walk_header_buffer(
    memory: &impl Memory,
    base: usize,
    head: Option<[u8; HEADER]>,
    mut output: engine_native_memory::walk::Buffer<'_, cscript_flag::Row>,
) -> Outcome {
    let result = (|| {
        let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
        let head = head.ok_or("flag manager header unreadable")?;
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
        read_rows(memory, base, rows, &mut output[..len])?;
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

fn read_rows(
    memory: &impl Memory,
    base: usize,
    rows: usize,
    output: &mut [cscript_flag::Row],
) -> Result<(), &'static str> {
    let end = output
        .len()
        .checked_mul(cscript_flag::STRIDE)
        .and_then(|span| rows.checked_add(span));
    if end.is_none() {
        for (index, target) in output.iter_mut().enumerate() {
            let address = index
                .checked_mul(cscript_flag::STRIDE)
                .and_then(|offset| rows.checked_add(offset))
                .ok_or("flag row address overflow")?;
            *target = cscript_flag::read_row(memory, base, address)?;
        }
        return Ok(());
    }
    let mut fields = [0_u8; ROW_BATCH * cscript_flag::FIELD_BYTES];
    for (batch, targets) in output.chunks_mut(ROW_BATCH).enumerate() {
        let gathered = &mut fields[..targets.len() * cscript_flag::FIELD_BYTES];
        let completed = memory.copy_records(
            RecordSources::Strided {
                base: rows + batch * ROW_BATCH * cscript_flag::STRIDE,
                stride: cscript_flag::STRIDE,
                count: targets.len(),
            },
            &cscript_flag::LAYOUT,
            gathered,
        );
        let copied = completed / cscript_flag::LAYOUT.field_count();
        for (target, row) in targets
            .iter_mut()
            .zip(gathered.chunks_exact(cscript_flag::FIELD_BYTES))
            .take(copied)
        {
            *target = cscript_flag::decode_fields(base, row)?;
        }
        if copied != targets.len() {
            return Err("script flag fields unreadable");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    const BASE: usize = 0x1000_0000;
    const MANAGER: usize = 0x2000;
    const DATA: usize = MANAGER + 0x40;

    fn fixture(tokens: &[u32]) -> Vec<u8> {
        let mut bytes = vec![0; 0x40 + tokens.len() * cscript_flag::STRIDE];
        bytes[0..8].copy_from_slice(&((BASE + VTABLE_RVA.0) as u64).to_le_bytes());
        bytes[8..16].copy_from_slice(&(DATA as u64).to_le_bytes());
        bytes[16..20].copy_from_slice(&(tokens.len() as i32).to_le_bytes());
        bytes[20..24].copy_from_slice(&(tokens.len() as i32).to_le_bytes());
        for (index, token) in tokens.iter().enumerate() {
            let start = 0x40 + index * cscript_flag::STRIDE;
            bytes[start..start + 8]
                .copy_from_slice(&((BASE + cscript_flag::VTABLE_RVA.0) as u64).to_le_bytes());
            bytes[start + 8..start + 12].copy_from_slice(&token.to_le_bytes());
            bytes[start + 16..start + 24]
                .copy_from_slice(&((BASE + cscript_flag::DATE_VTABLE_RVA.0) as u64).to_le_bytes());
            bytes[start + 24..start + 28].copy_from_slice(&i32::MIN.to_le_bytes());
            bytes[start + 32..start + 40].copy_from_slice(
                &((BASE + cscript_flag::DATE_SECONDARY_VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            bytes[start + 40..start + 42].copy_from_slice(&i16::MIN.to_le_bytes());
            bytes[start + 42..start + 44].copy_from_slice(&(-1_i16).to_le_bytes());
        }
        bytes
    }

    fn sample(bytes: &[u8], output: &mut Vec<cscript_flag::Row>) -> Outcome {
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
    fn daily_expiry_writes_only_a_manager_with_a_positive_duration() {
        let mut bytes = fixture(&[3, 4, 5]);
        let writes = |bytes: &[u8]| {
            daily_expiry_writes(
                &SliceMemory {
                    base: MANAGER,
                    bytes,
                },
                MANAGER,
            )
        };
        assert!(!writes(&bytes));
        let last = 0x40 + 2 * cscript_flag::STRIDE + cscript_flag::REMAINING_DAYS;
        bytes[last..last + 2].copy_from_slice(&0_i16.to_le_bytes());
        assert!(!writes(&bytes));
        bytes[last..last + 2].copy_from_slice(&1_i16.to_le_bytes());
        assert!(writes(&bytes));
        bytes[20..24].copy_from_slice(&2_i32.to_le_bytes());
        assert!(!writes(&bytes));
        bytes[20..24].copy_from_slice(&0_i32.to_le_bytes());
        assert!(!writes(&bytes));
        bytes[20..24].copy_from_slice(&3_i32.to_le_bytes());
        assert!(writes(&bytes[..0x40 + 2 * cscript_flag::STRIDE]));
        assert!(writes(&bytes[..0x10]));
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
                walk(&memory, BASE, MANAGER, &mut rows),
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
        let mut output = vec![cscript_flag::Row::default(); 2];
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
    fn rejects_duplicate_tokens_and_wrong_row_types() {
        let mut bytes = fixture(&[7, 7]);
        let mut output = vec![cscript_flag::Row::default(); 2];
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

    fn row_at_a_time(memory: &impl Memory, output: &mut [cscript_flag::Row]) -> Outcome {
        let result = (|| {
            for (index, target) in output.iter_mut().enumerate() {
                *target =
                    cscript_flag::read_row(memory, BASE, DATA + index * cscript_flag::STRIDE)?;
            }
            output.sort_unstable_by_key(|row| row.token);
            Ok(Outcome::Rows { len: output.len() })
        })();
        result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
    }

    #[test]
    fn rows_are_gathered_in_batches_with_the_row_at_a_time_verdicts() {
        struct Denied<'a> {
            bytes: &'a [u8],
            denied: Option<usize>,
        }
        impl Memory for Denied<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                !self
                    .denied
                    .is_some_and(|at| (address..address + output.len()).contains(&at))
                    && SliceMemory {
                        base: MANAGER,
                        bytes: self.bytes,
                    }
                    .copy(address, output)
            }
        }
        const ROWS: usize = 130;
        let tokens: Vec<u32> = (0..ROWS as u32).rev().collect();
        let mut bytes = fixture(&tokens);
        let source = SliceMemory {
            base: MANAGER,
            bytes: &bytes,
        };
        let census = engine_native_memory::read_census::ReadCensus::new(&source);
        let mut rows = vec![cscript_flag::Row::default(); ROWS];
        assert_eq!(
            walk(&census, BASE, MANAGER, &mut rows),
            Outcome::Rows { len: ROWS }
        );
        assert_eq!(census.calls(), 1 + ROWS.div_ceil(64));
        let mut expected = [cscript_flag::Row::default(); ROWS];
        row_at_a_time(
            &Denied {
                bytes: &bytes,
                denied: None,
            },
            &mut expected,
        );
        assert_eq!(rows, expected);
        bytes[0x40 + 70 * cscript_flag::STRIDE] ^= 1;
        for row in [0, 63, 64, 70, 100, 129] {
            for offset in [0, 11, 12, cscript_flag::DATE, 0x27, 0x2c, 0x2f] {
                let memory = Denied {
                    bytes: &bytes,
                    denied: Some(DATA + row * cscript_flag::STRIDE + offset),
                };
                let mut reference = [cscript_flag::Row::default(); ROWS];
                assert_eq!(
                    walk(&memory, BASE, MANAGER, &mut rows),
                    row_at_a_time(&memory, &mut reference),
                    "row {row} offset {offset:#x}"
                );
            }
        }
    }

    #[test]
    fn distinguishes_empty_invalid_and_unreadable_managers() {
        let mut bytes = fixture(&[]);
        bytes[8..16].fill(0);
        assert_eq!(sample(&bytes, &mut Vec::new()), Outcome::Rows { len: 0 });
        bytes[20..24].copy_from_slice(&(-1_i32).to_le_bytes());
        assert_eq!(
            sample(&bytes, &mut Vec::new()),
            Outcome::Unavailable {
                reason: "flag manager header invalid"
            }
        );
        assert_eq!(
            sample(&bytes[..23], &mut Vec::new()),
            Outcome::Unavailable {
                reason: "flag manager header unreadable"
            }
        );
    }
}
