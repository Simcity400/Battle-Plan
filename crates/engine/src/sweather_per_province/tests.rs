use super::*;
use crate::memory::SliceMemory;

fn bytes(slot: u32) -> [u8; STRIDE] {
    let mut bytes = [0; STRIDE];
    bytes[..8].copy_from_slice(&(VTABLE_RVA as u64).to_le_bytes());
    bytes[ID..ID + 4].copy_from_slice(&slot.to_le_bytes());
    bytes[SNOW..SNOW + 8].copy_from_slice(&((1_i64 << 41) + 7).to_le_bytes());
    bytes[PERMANENT_SNOW..PERMANENT_SNOW + 8].copy_from_slice(&(-5_i64).to_le_bytes());
    bytes[TEMPERATURE..TEMPERATURE + 8]
        .copy_from_slice(&(-9_007_199_254_740_993_i64).to_le_bytes());
    bytes[RAW_MUD] = 7;
    bytes
}

#[test]
fn snow_keeps_its_native_word_and_read_faults_are_named() {
    let bytes = bytes(0);
    let array = Array {
        data: 0x1000,
        count: 1,
    };
    let mut rows = [Row::default(); 1];
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(walk(&memory, 0, array, &mut rows), Outcome::Rows { len: 1 });
    assert_eq!(
        (rows[0].snow, rows[0].permanent_snow),
        ((1_i64 << 41) + 7, -5)
    );
    assert_eq!(
        (rows[0].temperature, rows[0].raw_mud),
        (-9_007_199_254_740_993, 7)
    );
    let short = SliceMemory {
        base: 0x1000,
        bytes: &bytes[..SNOW + 7],
    };
    assert_eq!(
        walk(&short, 0, array, &mut rows),
        Outcome::Unavailable {
            reason: "weather province fields unreadable"
        }
    );
    assert_eq!(
        walk(&memory, 0, array, &mut []),
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
}

#[test]
fn temperature_and_raw_mud_failures_withhold_the_row_without_narrowing() {
    let bytes = bytes(0);
    for end in [TEMPERATURE + 7, RAW_MUD] {
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes[..end],
        };
        assert_eq!(
            walk(
                &memory,
                0,
                Array {
                    data: 0x1000,
                    count: 1
                },
                &mut [Row::default()]
            ),
            Outcome::Unavailable {
                reason: "weather province fields unreadable"
            }
        );
    }
    for raw in [i64::MIN, -1, 0, i64::MAX] {
        let mut record = [0; RECORD_BYTES];
        record[..8].copy_from_slice(&(VTABLE_RVA as u64).to_le_bytes());
        record[28..36].copy_from_slice(&raw.to_le_bytes());
        record[36] = 255;
        let row = decode_record(0, &record, FIELDS.len(), VTABLE_RVA).unwrap();
        assert_eq!((row.temperature, row.raw_mud), (raw, 255));
        assert!(decode_record(0, &record, FIELDS.len() - 1, VTABLE_RVA).is_err());
    }
}

/// A batched gather crosses the guarded boundary once per 64 rows and
/// copies only the declared spans.
#[test]
fn a_dense_census_costs_one_guarded_gather_per_batch_not_one_per_row() {
    use crate::read_census::ReadCensus;
    const ROWS: usize = 130;
    let mut storage = vec![0_u8; ROWS * STRIDE];
    for slot in 0..ROWS {
        storage[slot * STRIDE..(slot + 1) * STRIDE].copy_from_slice(&bytes(slot as u32));
    }
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &storage,
    };
    let census = ReadCensus::new(&memory);
    let mut rows = vec![Row::default(); ROWS];
    let array = Array {
        data: 0x1000,
        count: ROWS as u32,
    };
    assert_eq!(
        walk(&census, 0, array, &mut rows),
        Outcome::Rows { len: ROWS }
    );
    assert_eq!(rows[129].id, 129);
    assert_eq!(census.calls(), ROWS.div_ceil(64));
    assert_eq!(census.spans(), ROWS * FIELDS.len());
    assert_eq!(census.bytes(), ROWS * RECORD_BYTES);
}

#[test]
fn display_bytes_join_by_slot_and_a_foreign_count_refuses() {
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &[7, 8, 9, 0, 255, 0],
    };
    let arrays = [
        Array {
            data: 0x1000,
            count: 3,
        },
        Array {
            data: 0x1003,
            count: 3,
        },
    ];
    let mut rows = [Row::default(); 2];
    fill_display(&memory, arrays, 3, &mut rows).unwrap();
    assert_eq!(
        rows.map(|row| (row.display_snow, row.display_mud)),
        [(7, 0), (8, 255)]
    );
    assert!(fill_display(&memory, arrays, 4, &mut rows).is_err());
    let unreadable = [
        arrays[0],
        Array {
            data: 0x2000,
            count: 3,
        },
    ];
    assert!(fill_display(&memory, unreadable, 3, &mut rows).is_err());
}
