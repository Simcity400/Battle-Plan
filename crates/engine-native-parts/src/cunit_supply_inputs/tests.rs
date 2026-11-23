use super::*;
use engine_native_memory::memory::SliceMemory;
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
    let mut origins = vec![Origin::default(); 3];
    let row = capture_buffer(&memory, UNIT, engine_native_memory::walk::Buffer::Growing(&mut origins)).unwrap();
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
    let row = capture_buffer(&memory, UNIT, engine_native_memory::walk::Buffer::Growing(&mut origins)).unwrap();
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
        capture_buffer(&memory, UNIT, engine_native_memory::walk::Buffer::Count).unwrap().origins_len,
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
    assert_eq!(capture_buffer(&memory, UNIT, engine_native_memory::walk::Buffer::Count).unwrap().origins_len, Ok(0));
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
            capture_buffer(&memory, UNIT, engine_native_memory::walk::Buffer::Growing(&mut vec![Origin::default()]))
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
        let result = capture_buffer(&memory, UNIT, engine_native_memory::walk::Buffer::Growing(&mut vec![Origin::default(); 2]));
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
        capture_buffer(&memory, UNIT, engine_native_memory::walk::Buffer::Growing(&mut vec![Origin::default(); 2]))
            .unwrap()
            .origins_len,
        Err(Failure::OriginsChanged)
    );
}

#[test]
fn unrelated_consumer_bytes_and_origin_fields_are_not_read() {
    let bytes = fixture(&[Origin::default()]);
    for fail in [
        UNIT + 0x40,
        UNIT + DIVISION_COUNT + 4,
        DATA,
        DATA + 8,
        DATA + 0x20,
        DATA + 0x2c,
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
        assert!(capture_buffer(&memory, UNIT, engine_native_memory::walk::Buffer::Growing(&mut vec![Origin::default()])).is_ok());
    }
}

#[test]
fn worker_origins_grow_from_original_header_before_first_and_later_publication() {
    let mut output = Vec::new();
    for count in [300, 1103] {
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
            capture_buffer(&memory, UNIT, engine_native_memory::walk::Buffer::Count).unwrap().origins_len,
            Err(Failure::OriginsCapacity { required: count })
        );
        let inputs =
            capture_buffer(&memory, UNIT, engine_native_memory::walk::Buffer::Growing(&mut output)).unwrap();
        assert_eq!(inputs.origins_len, Ok(count));
        assert_eq!(&output[..count], &expected);
    }
}

fn units_fixture(origins: &[Vec<Origin>]) -> (Vec<u8>, Vec<usize>) {
    const STRIDE: usize = 0x100;
    let rows: usize = origins.iter().map(Vec::len).sum();
    let mut bytes = vec![0; origins.len() * STRIDE + rows * ORIGIN_STRIDE];
    let mut data = BASE + origins.len() * STRIDE;
    let mut pointers = Vec::new();
    for (index, rows) in origins.iter().enumerate() {
        let unit = BASE + index * STRIDE;
        pointers.push(unit);
        let expected = unit_inputs(index, Ok(rows.len()));
        for (offset, value) in [
            (ASKED, expected.asked),
            (RECEIVED, expected.received),
            (AIR_DROPS, expected.air_drops),
            (FROM_STATE, expected.from_state),
            (DIVISION_DEMAND, expected.division_demand.value),
            (AIRPLANE_DEMAND, expected.airplane_demand.value),
            (SHIP_DEMAND, expected.ship_demand.value),
            (DISRUPTION, expected.disruption),
        ] {
            write(&mut bytes, unit + offset, &value.to_le_bytes());
        }
        for (offset, value) in [
            (DIVISION_COUNT, expected.division_demand.count),
            (AIRPLANE_COUNT, expected.airplane_demand.count),
            (SHIP_COUNT, expected.ship_demand.count),
        ] {
            write(&mut bytes, unit + offset, &value.to_le_bytes());
        }
        write(&mut bytes, unit + ORIGINS, &(data as u64).to_le_bytes());
        let count = (rows.len() as u32).to_le_bytes();
        write(&mut bytes, unit + ORIGINS + 8, &count);
        write(&mut bytes, unit + ORIGINS + 12, &count);
        for origin in rows {
            write(
                &mut bytes,
                data + ORIGIN_PROVINCE,
                &origin.province.to_le_bytes(),
            );
            write(
                &mut bytes,
                data + ORIGIN_REQUESTED,
                &origin.requested.to_le_bytes(),
            );
            write(
                &mut bytes,
                data + ORIGIN_RECEIVED,
                &origin.received.to_le_bytes(),
            );
            data += ORIGIN_STRIDE;
        }
    }
    (bytes, pointers)
}

fn unit_inputs(index: usize, origins_len: Result<usize, Failure>) -> Inputs {
    let index = index as i64;
    Inputs {
        asked: index * 1_000 + 1,
        received: -index,
        from_state: index * 3,
        air_drops: index + 7,
        disruption: index - 3,
        division_demand: Demand {
            count: index as i32,
            value: index * 5,
        },
        airplane_demand: Demand {
            count: -1,
            value: i64::MIN,
        },
        ship_demand: Demand {
            count: i32::MAX,
            value: i64::MAX,
        },
        origins_len,
    }
}

#[test]
fn batched_units_keep_each_footer_and_native_origin_order_across_batches() {
    let origins: Vec<Vec<Origin>> = (0..150)
        .map(|unit: usize| {
            (0..unit % 4)
                .map(|row| Origin {
                    province: ((unit * 10 + row) % 7) as i32,
                    requested: unit as i64 - row as i64,
                    received: -(row as i64),
                })
                .collect()
        })
        .collect();
    let (bytes, pointers) = units_fixture(&origins);
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let mut units = vec![UnitCapture::default(); pointers.len()];
    let mut arena = Vec::new();
    let used = capture_units(&memory, |index| pointers[index], &mut units, &mut arena);
    let total: usize = origins.iter().map(Vec::len).sum();
    assert_eq!(
        used,
        OriginsUse {
            len: total,
            required: total
        }
    );
    let mut start = 0;
    for (index, (unit, expected)) in units.iter().zip(&origins).enumerate() {
        assert_eq!(unit.inputs, Ok(unit_inputs(index, Ok(expected.len()))));
        assert_eq!(unit.origins_start, start, "{index}");
        assert_eq!(&arena[start..start + expected.len()], expected.as_slice());
        start += expected.len();
        let mut single = vec![Origin::default(); expected.len()];
        assert_eq!(capture_buffer(&memory, pointers[index], engine_native_memory::walk::Buffer::Growing(&mut single)), unit.inputs);
        assert_eq!(&single, expected);
    }
}

struct Faults<'a> {
    memory: SliceMemory<'a>,
    fail: Vec<usize>,
    changed: Option<usize>,
    header_reads: Cell<u8>,
}

impl Memory for Faults<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if self
            .fail
            .iter()
            .any(|&fail| address <= fail && fail < address.saturating_add(destination.len()))
        {
            destination.fill(0xa5);
            return false;
        }
        let copied = self.memory.copy(address, destination);
        if copied && Some(address) == self.changed && destination.len() == 16 {
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
fn a_fault_fails_only_its_own_unit_or_origin_fragment() {
    let origins: Vec<Vec<Origin>> = [1_usize, 2, 1, 2, 1]
        .into_iter()
        .enumerate()
        .map(|(unit, rows)| {
            (0..rows)
                .map(|row| Origin {
                    province: (unit * 10 + row) as i32,
                    requested: 100 + row as i64,
                    received: 50 + unit as i64,
                })
                .collect()
        })
        .collect();
    let (bytes, pointers) = units_fixture(&origins);
    let unit_3_second_row = BASE + origins.len() * 0x100 + 5 * ORIGIN_STRIDE;
    let memory = Faults {
        memory: SliceMemory {
            base: BASE,
            bytes: &bytes,
        },
        fail: vec![pointers[1] + RECEIVED, unit_3_second_row + ORIGIN_RECEIVED],
        changed: Some(pointers[2] + ORIGINS),
        header_reads: Cell::new(0),
    };
    let mut units = vec![UnitCapture::default(); pointers.len()];
    let mut arena = Vec::new();
    let used = capture_units(&memory, |index| pointers[index], &mut units, &mut arena);
    assert_eq!(
        used,
        OriginsUse {
            len: 5,
            required: 5
        }
    );
    assert_eq!(units[0].inputs, Ok(unit_inputs(0, Ok(1))));
    assert_eq!(&arena[..1], origins[0].as_slice());
    assert_eq!(units[1].inputs, Err(Failure::Unreadable));
    assert_eq!(
        units[2].inputs,
        Ok(unit_inputs(2, Err(Failure::OriginsChanged)))
    );
    assert_eq!(
        units[3].inputs,
        Ok(unit_inputs(3, Err(Failure::Unreadable)))
    );
    assert_eq!(units[4].inputs, Ok(unit_inputs(4, Ok(1))));
    assert_eq!(units[4].origins_start, 4);
    assert_eq!(&arena[4..5], origins[4].as_slice());
}

#[test]
fn null_roots_are_unreadable() {
    let memory = SliceMemory {
        base: BASE,
        bytes: &[],
    };
    let mut units = [UnitCapture::default(); 2];
    let mut arena = Vec::new();
    assert_eq!(
        capture_units(&memory, |_| 0, &mut units, &mut arena),
        OriginsUse::default()
    );
    assert_eq!(units.map(|unit| unit.inputs), [Err(Failure::Unreadable); 2]);
    assert!(arena.is_empty());
}

#[test]
fn batched_units_read_no_unrelated_consumer_bytes() {
    let origins = vec![vec![Origin::default()]; 3];
    let (bytes, pointers) = units_fixture(&origins);
    let data = BASE + origins.len() * 0x100;
    for fail in [
        pointers[1] + 0x40,
        pointers[1] + DIVISION_COUNT + 4,
        data + ORIGIN_STRIDE,
        data + ORIGIN_STRIDE + 8,
        data + ORIGIN_STRIDE + 0x20,
        data + ORIGIN_STRIDE + 0x2c,
    ] {
        let memory = Faults {
            memory: SliceMemory {
                base: BASE,
                bytes: &bytes,
            },
            fail: vec![fail],
            changed: None,
            header_reads: Cell::new(0),
        };
        let mut units = vec![UnitCapture::default(); 3];
        capture_units(&memory, |index| pointers[index], &mut units, &mut Vec::new());
        for (index, unit) in units.iter().enumerate() {
            assert_eq!(unit.inputs, Ok(unit_inputs(index, Ok(1))), "{fail:#x}");
        }
    }
}
