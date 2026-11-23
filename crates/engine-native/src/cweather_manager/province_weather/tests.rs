use super::*;
use engine_native_memory::memory::SliceMemory;

fn bytes(slot: u32) -> [u8; STRIDE] {
    let mut bytes = [0; STRIDE];
    bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[ID..ID + 4].copy_from_slice(&slot.to_le_bytes());
    bytes[SNOW..SNOW + 8].copy_from_slice(&((1_i64 << 41) + 7).to_le_bytes());
    bytes[PERMANENT_SNOW..PERMANENT_SNOW + 8].copy_from_slice(&(-5_i64).to_le_bytes());
    bytes[TEMPERATURE..TEMPERATURE + 8]
        .copy_from_slice(&(-9_007_199_254_740_993_i64).to_le_bytes());
    bytes[RAW_MUD] = 7;
    for (offset, value) in [
        (DRIFT, i64::MIN + 3),
        (STAGED_TEMPERATURE, -9_007_199_254_740_993_i64),
        (WORKING_DRIFT, 600_000),
    ] {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
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
    assert_eq!(
        (
            rows[0].drift,
            rows[0].staged_temperature,
            rows[0].working_drift
        ),
        (i64::MIN + 3, -9_007_199_254_740_993, 600_000)
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
    for end in [
        TEMPERATURE + 7,
        RAW_MUD,
        DRIFT + 7,
        STAGED_TEMPERATURE,
        WORKING_DRIFT + 1,
    ] {
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
        record[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        record[28..36].copy_from_slice(&raw.to_le_bytes());
        record[36] = 255;
        let row = decode_record(0, &record, FIELDS.len(), VTABLE_RVA.0).unwrap();
        assert_eq!((row.temperature, row.raw_mud), (raw, 255));
        assert!(decode_record(0, &record, FIELDS.len() - 1, VTABLE_RVA.0).is_err());
    }
}

#[test]
fn a_dense_census_costs_one_guarded_gather_per_batch_not_one_per_row() {
    use engine_native_memory::read_census::ReadCensus;
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
fn clock_records_preserve_signed_words_and_never_commit_a_failed_pair() {
    for (hour, night) in [(i32::MIN, i64::MIN), (0, 0), (i32::MAX, i64::MAX)] {
        let mut record = [0; CLOCK_RECORD_BYTES];
        record[..8].copy_from_slice(&night.to_le_bytes());
        record[8..].copy_from_slice(&hour.to_le_bytes());
        let mut row = Row {
            local_hour: 7,
            night_factor: 19,
            snow: 31,
            ..Default::default()
        };
        let before = row;
        assert_eq!(
            fill_clock_record(&record, 1, &mut row),
            Err("weather clock fields unreadable")
        );
        assert_eq!(row, before);
        assert!(fill_clock_record(&record[..11], 2, &mut row).is_err());
        assert_eq!(row, before);
        fill_clock_record(&record, CLOCK_FIELDS.len(), &mut row).unwrap();
        assert_eq!(
            (row.local_hour, row.night_factor, row.snow),
            (hour, night, 31)
        );
    }
}

#[test]
fn clock_join_copies_only_twelve_bytes_per_admitted_slot_and_checks_live_counts() {
    use engine_native_memory::read_census::ReadCensus;
    const ROWS: usize = 130;
    let mut bytes = vec![0; ROWS * CLOCK_STRIDE];
    for index in 0..ROWS {
        let start = index * CLOCK_STRIDE;
        bytes[start + CLOCK_LOCAL_HOUR..start + CLOCK_LOCAL_HOUR + 4]
            .copy_from_slice(&(index as i32 - 17).to_le_bytes());
        bytes[start + CLOCK_NIGHT_FACTOR..start + CLOCK_NIGHT_FACTOR + 8]
            .copy_from_slice(&(-(index as i64) - 11).to_le_bytes());
    }
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let census = ReadCensus::new(&memory);
    let array = Array {
        data: 0x1000,
        count: ROWS as u32,
    };
    let mut rows = [Row::default(); 65];
    assert_eq!(
        fill_clocks(&census, array, ROWS as u32 - 1, &mut rows),
        Err("weather clock array differs from the province array")
    );
    assert_eq!(census.calls(), 0);
    fill_clocks(&census, array, ROWS as u32, &mut rows).unwrap();
    assert_eq!(
        (
            rows[0].local_hour,
            rows[64].local_hour,
            rows[64].night_factor
        ),
        (-17, 47, -75)
    );
    assert_eq!(census.calls(), 2);
    assert_eq!(census.spans(), rows.len() * CLOCK_FIELDS.len());
    assert_eq!(census.bytes(), rows.len() * CLOCK_RECORD_BYTES);
    let short = SliceMemory {
        base: 0x1000,
        bytes: &bytes[..CLOCK_LOCAL_HOUR + 3],
    };
    assert_eq!(
        fill_clocks(&short, array, ROWS as u32, &mut rows[..1]),
        Err("weather clock fields unreadable")
    );
    assert_eq!(
        fill_clocks(&census, Array { data: 0, count: 0 }, 0, &mut []),
        Ok(())
    );
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

#[test]
fn static_modifier_lists_keep_order_days_and_names() {
    const VTABLE: u64 = 0x7777;
    const LIST: usize = 0x100;
    const FIRST: usize = 0x200;
    const HEAP: usize = 0x800;
    let definition_bytes = engine_native_parts::cstatic_modifier::NAME + engine_native_memory::msvc_string::STRIDE;
    let mut bytes = vec![0_u8; 0x900];
    let mut put = |at: usize, value: &[u8]| bytes[at..at + value.len()].copy_from_slice(value);
    let long = b"a_mod_static_modifier_name";
    for (index, (name, days)) in [(&b"the_great_wall"[..], -1_i32), (&long[..], 30)]
        .into_iter()
        .enumerate()
    {
        let definition = FIRST + index * definition_bytes;
        put(definition, &VTABLE.to_le_bytes());
        let string = definition + engine_native_parts::cstatic_modifier::NAME;
        if name.len() <= 15 {
            put(string, name);
            put(string + engine_native_memory::msvc_string::CAPACITY, &15_u64.to_le_bytes());
        } else {
            put(HEAP, name);
            put(string, &((0x10_000 + HEAP) as u64).to_le_bytes());
            put(string + engine_native_memory::msvc_string::CAPACITY, &31_u64.to_le_bytes());
        }
        put(
            string + engine_native_memory::msvc_string::LENGTH,
            &(name.len() as u64).to_le_bytes(),
        );
        let entry = LIST + index * STATIC_MODIFIER_STRIDE;
        put(entry, &((0x10_000 + definition) as u64).to_le_bytes());
        put(entry + STATIC_MODIFIER_DAYS, &days.to_le_bytes());
    }
    let memory = SliceMemory {
        base: 0x10_000,
        bytes: &bytes,
    };
    let mut header = [0_u8; 16];
    header[..8].copy_from_slice(&((0x10_000 + LIST) as u64).to_le_bytes());
    header[8..12].copy_from_slice(&4_u32.to_le_bytes());
    header[12..].copy_from_slice(&2_u32.to_le_bytes());
    let list = static_modifier_list(header).unwrap();
    let mut out = Vec::new();
    read_static_modifiers(&memory, list, VTABLE as usize, &mut out).unwrap();
    let mut expected = Vec::new();
    for (name, days) in [(&b"the_great_wall"[..], -1_i32), (&long[..], 30)] {
        expected.extend_from_slice(&days.to_le_bytes());
        expected.extend_from_slice(&(name.len() as u32).to_le_bytes());
        expected.extend_from_slice(name);
    }
    assert_eq!(out, expected);
    let empty = Array { data: 0, count: 0 };
    out.clear();
    read_static_modifiers(&memory, empty, VTABLE as usize, &mut out).unwrap();
    assert!(out.is_empty());
    assert_eq!(
        read_static_modifiers(&memory, list, VTABLE as usize + 8, &mut Vec::new()),
        Err("static modifier definition vtable mismatch")
    );
    assert_eq!(
        read_static_modifiers(
            &memory,
            Array {
                data: 0x10_000 + 0x8f8,
                count: 2
            },
            VTABLE as usize,
            &mut Vec::new()
        ),
        Err("static modifier list unreadable")
    );
    header[8..12].copy_from_slice(&1_u32.to_le_bytes());
    assert_eq!(static_modifier_list(header), None);
}
