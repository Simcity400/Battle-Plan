use super::*;
use crate::memory::SliceMemory;
use std::cell::Cell;

const BASE: usize = 0x1000;
const UNIT: usize = BASE + 0x100;
const DATA: usize = BASE + 0x300;

fn write(bytes: &mut [u8], address: usize, value: &[u8]) {
    let at = address - BASE;
    bytes[at..at + value.len()].copy_from_slice(value);
}

fn fixture(origins: &[Origin]) -> Vec<u8> {
    let mut bytes = vec![0; DATA - BASE + origins.len() * ORIGIN_STRIDE];
    for (offset, value) in [
        (ASKED, i64::MIN),
        (RECEIVED, i64::MAX),
        (FROM_STATE, -123),
        (AIR_DROPS, 456),
        (DISRUPTION, -789),
        (DIVISION_DEMAND, 987),
        (AIRPLANE_DEMAND, -654),
        (SHIP_DEMAND, 321),
    ] {
        write(&mut bytes, UNIT + offset, &value.to_le_bytes());
    }
    for (offset, value) in [
        (DIVISION_COUNT, i32::MIN),
        (AIRPLANE_COUNT, i32::MAX),
        (SHIP_COUNT, -1),
    ] {
        write(&mut bytes, UNIT + offset, &value.to_le_bytes());
    }
    write(&mut bytes, UNIT + ORIGINS, &(DATA as u64).to_le_bytes());
    write(
        &mut bytes,
        UNIT + ORIGINS + 8,
        &(origins.len() as u32).to_le_bytes(),
    );
    write(
        &mut bytes,
        UNIT + ORIGINS + 12,
        &(origins.len() as u32).to_le_bytes(),
    );
    for (index, origin) in origins.iter().enumerate() {
        let at = DATA + index * ORIGIN_STRIDE;
        write(
            &mut bytes,
            at + ORIGIN_PROVINCE,
            &origin.province.to_le_bytes(),
        );
        write(
            &mut bytes,
            at + ORIGIN_REQUESTED,
            &origin.requested.to_le_bytes(),
        );
        write(
            &mut bytes,
            at + ORIGIN_RECEIVED,
            &origin.received.to_le_bytes(),
        );
    }
    bytes
}

#[test]
fn complete_inputs_preserve_signed_words_and_native_origin_order() {
    let expected = [
        Origin {
            province: -42,
            requested: i64::MIN,
            received: 0,
        },
        Origin {
            province: 999,
            requested: -7,
            received: -8,
        },
        Origin {
            province: -42,
            requested: i64::MAX,
            received: 5_000,
        },
    ];
    let bytes = fixture(&expected);
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let mut origins = [Origin::default(); 3];
    let row = capture(&memory, UNIT, &mut origins).unwrap();
    assert_eq!(origins, expected);
    assert_eq!((row.asked, row.received), (i64::MIN, i64::MAX));
    assert_eq!(
        (row.from_state, row.air_drops, row.disruption),
        (-123, 456, -789)
    );
    assert_eq!(
        row.division_demand,
        Demand {
            count: i32::MIN,
            value: 987
        }
    );
    assert_eq!(
        row.airplane_demand,
        Demand {
            count: i32::MAX,
            value: -654
        }
    );
    assert_eq!(
        row.ship_demand,
        Demand {
            count: -1,
            value: 321
        }
    );
    assert_eq!(row.origins_len, Ok(3));
}

#[test]
fn live_extent_crosses_batches_without_a_content_count_limit() {
    let expected: Vec<_> = (0..193)
        .map(|index| Origin {
            province: index % 7,
            requested: i64::from(index) - 100,
            received: -i64::from(index),
        })
        .collect();
    let bytes = fixture(&expected);
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let mut origins = vec![Origin::default(); expected.len()];
    let row = capture(&memory, UNIT, &mut origins).unwrap();
    assert_eq!(row.origins_len, Ok(expected.len()));
    assert_eq!(origins, expected);
}

struct Fault<'a> {
    memory: SliceMemory<'a>,
    fail: usize,
    change_header: bool,
    header_reads: Cell<u8>,
}

impl Memory for Fault<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if address <= self.fail && self.fail < address.saturating_add(destination.len()) {
            // A failed guarded copy may modify a prefix. Its bytes must not
            // become a successful scalar or shortened origin vector.
            destination.fill(0xa5);
            return false;
        }
        let copied = self.memory.copy(address, destination);
        if copied && self.change_header && address == UNIT + ORIGINS {
            let reads = self.header_reads.get() + 1;
            self.header_reads.set(reads);
            if reads == 2 {
                destination[12] ^= 1;
            }
        }
        copied
    }
}

#[test]
fn capacity_shortage_reports_exact_demand_before_origin_payload_reads() {
    let bytes = fixture(&[Origin::default(); 3]);
    let memory = Fault {
        memory: SliceMemory {
            base: BASE,
            bytes: &bytes,
        },
        fail: DATA + ORIGIN_REQUESTED,
        change_header: false,
        header_reads: Cell::new(0),
    };
    assert_eq!(
        capture(&memory, UNIT, &mut []).unwrap().origins_len,
        Err(Failure::OriginsCapacity { required: 3 })
    );
}

#[test]
fn malformed_empty_null_and_overflow_headers_are_distinct_from_absent_origins() {
    let mut bytes = fixture(&[]);
    write(&mut bytes, UNIT + ORIGINS, &0_u64.to_le_bytes());
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    assert_eq!(capture(&memory, UNIT, &mut []).unwrap().origins_len, Ok(0));
    for (data, capacity, count) in [
        (DATA as u64, 0_u32, 1_u32),
        (0, 1, 1),
        (DATA as u64, u32::MAX, 0),
        (u64::MAX, 1, 1),
    ] {
        write(&mut bytes, UNIT + ORIGINS, &data.to_le_bytes());
        write(&mut bytes, UNIT + ORIGINS + 8, &capacity.to_le_bytes());
        write(&mut bytes, UNIT + ORIGINS + 12, &count.to_le_bytes());
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        assert_eq!(
            capture(&memory, UNIT, &mut [Origin::default()])
                .unwrap()
                .origins_len,
            Err(Failure::InvalidOrigins)
        );
    }
}

#[test]
fn field_and_origin_faults_never_certify_a_partial_pack_and_header_drift_is_refused() {
    let bytes = fixture(&[Origin::default(); 2]);
    for fail in [
        UNIT + ASKED,
        UNIT + SHIP_DEMAND,
        DATA + ORIGIN_REQUESTED,
        DATA + ORIGIN_STRIDE + ORIGIN_PROVINCE,
        UNIT + ORIGINS + 12,
    ] {
        let memory = Fault {
            memory: SliceMemory {
                base: BASE,
                bytes: &bytes,
            },
            fail,
            change_header: false,
            header_reads: Cell::new(0),
        };
        let result = capture(&memory, UNIT, &mut [Origin::default(); 2]);
        if fail < UNIT + ORIGINS || fail == UNIT + SHIP_DEMAND {
            assert_eq!(result, Err(Failure::Unreadable));
        } else {
            let inputs = result.unwrap();
            assert_eq!(inputs.origins_len, Err(Failure::Unreadable));
            assert_eq!((inputs.asked, inputs.received), (i64::MIN, i64::MAX));
        }
    }
    let memory = Fault {
        memory: SliceMemory {
            base: BASE,
            bytes: &bytes,
        },
        fail: usize::MAX,
        change_header: true,
        header_reads: Cell::new(0),
    };
    assert_eq!(
        capture(&memory, UNIT, &mut [Origin::default(); 2])
            .unwrap()
            .origins_len,
        Err(Failure::OriginsChanged)
    );
}

#[test]
fn unrelated_consumer_bytes_and_origin_fields_are_not_read() {
    let bytes = fixture(&[Origin::default()]);
    for fail in [
        UNIT + 0x40,               // stored ratio
        UNIT + DIVISION_COUNT + 4, // padding
        DATA,                      // native origin sort key
        DATA + 8,                  // native consumer pointer
        DATA + 0x20,               // third amount not used by the footer
        DATA + 0x2c,               // source country not used by the footer
    ] {
        let memory = Fault {
            memory: SliceMemory {
                base: BASE,
                bytes: &bytes,
            },
            fail,
            change_header: false,
            header_reads: Cell::new(0),
        };
        assert!(capture(&memory, UNIT, &mut [Origin::default()]).is_ok());
    }
}

#[test]
fn worker_origins_grow_from_original_header_before_first_and_later_publication() {
    let mut output = Vec::new();
    for count in [5000, 11003] {
        let expected = (0..count)
            .map(|index| Origin {
                province: (index % 257) as i32,
                requested: index as i64 - 7,
                received: -(index as i64),
            })
            .collect::<Vec<_>>();
        let bytes = fixture(&expected);
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        assert_eq!(
            capture(&memory, UNIT, &mut []).unwrap().origins_len,
            Err(Failure::OriginsCapacity { required: count })
        );
        let inputs =
            capture_buffer(&memory, UNIT, crate::walk::Buffer::Growing(&mut output)).unwrap();
        assert_eq!(inputs.origins_len, Ok(count));
        assert_eq!(&output[..count], &expected);
    }
}
