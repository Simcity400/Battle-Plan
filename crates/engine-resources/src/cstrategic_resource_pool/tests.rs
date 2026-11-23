use super::*;
use engine_native_memory::memory::SliceMemory;

#[test]
fn faction_resource_pool_uses_live_mod_resource_count_and_retains_zero_and_negative_amounts() {
    let pool = 0x100;
    let data = 0x1000;
    let count = 5001;
    let mut bytes = vec![0; data + count * VALUE_STRIDE];
    bytes[pool..pool + 8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[pool + 8..pool + 16].copy_from_slice(&(data as u64).to_le_bytes());
    bytes[pool + 16..pool + 20].copy_from_slice(&(count as u32).to_le_bytes());
    bytes[pool + 20..pool + 24].copy_from_slice(&(count as u32).to_le_bytes());
    for ordinal in 0..count {
        let address = data + ordinal * VALUE_STRIDE;
        bytes[address..address + 8].copy_from_slice(&(ordinal as i64 - 2500).to_le_bytes());
        bytes[address + TOKEN..address + TOKEN + 4]
            .copy_from_slice(&(40000_u32 + ordinal as u32).to_le_bytes());
    }
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut output = Vec::new();
    read_into(&memory, 0, pool, &mut output).unwrap();
    assert_eq!(output.len(), 5001);
    assert_eq!(
        output[0],
        StoredAmount {
            token: 40000,
            raw: -2500
        }
    );
    assert_eq!(
        output[2500],
        StoredAmount {
            token: 42500,
            raw: 0
        }
    );
    assert_eq!(
        output[5000],
        StoredAmount {
            token: 45000,
            raw: 2500
        }
    );
    let pointer = output.as_ptr();
    read_into(&memory, 0, pool, &mut output).unwrap();
    assert_eq!(output.as_ptr(), pointer);
}

#[test]
fn faction_resource_pool_refuses_bad_counts_and_incomplete_payloads() {
    let pool = 0x100;
    let data = 0x400;
    let mut bytes = vec![0; data + VALUE_STRIDE + 11];
    bytes[pool..pool + 8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[pool + 8..pool + 16].copy_from_slice(&(data as u64).to_le_bytes());
    bytes[pool + 16..pool + 20].copy_from_slice(&2_u32.to_le_bytes());
    bytes[pool + 20..pool + 24].copy_from_slice(&2_u32.to_le_bytes());
    let mut output = Vec::new();
    assert!(
        read_into(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0,
            pool,
            &mut output
        )
        .is_err()
    );
    bytes[pool + 20..pool + 24].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(
        read_into(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0,
            pool,
            &mut output
        )
        .is_err()
    );
}
