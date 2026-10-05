use super::*;
use engine::{cgame_state, memory::SliceMemory};
use std::cell::Cell;

fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
    bytes[address - 0x1000..address - 0x1000 + value.len()].copy_from_slice(value);
}

/// Manager 0x2000, one region at 0x3000, one province at 0x4000, display
/// snow at 0x4800 and mud at 0x4810.
fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x4000];
    put(
        &mut bytes,
        0x1000 + cgame_state::WEATHER_MANAGER,
        &0x2000_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x2000,
        &(cweather_manager::VTABLE_RVA as u64).to_le_bytes(),
    );
    for (offset, address, vtable) in [
        (
            cweather_manager::REGIONS,
            0x3000_u64,
            sweather_per_region::VTABLE_RVA,
        ),
        (
            cweather_manager::PROVINCES,
            0x4000_u64,
            sweather_per_province::VTABLE_RVA,
        ),
    ] {
        put(&mut bytes, 0x2000 + offset, &address.to_le_bytes());
        put(&mut bytes, 0x2000 + offset + 8, &3_u32.to_le_bytes());
        put(&mut bytes, 0x2000 + offset + 12, &1_u32.to_le_bytes());
        put(&mut bytes, address as usize, &(vtable as u64).to_le_bytes());
    }
    for (offset, address, value) in [
        (cweather_manager::DISPLAY_SNOW, 0x4800_u64, 200),
        (cweather_manager::DISPLAY_MUD, 0x4810, 255),
    ] {
        put(&mut bytes, 0x2000 + offset, &address.to_le_bytes());
        put(&mut bytes, 0x2000 + offset + 8, &4_u32.to_le_bytes());
        put(&mut bytes, 0x2000 + offset + 12, &1_u32.to_le_bytes());
        put(&mut bytes, address as usize, &[value]);
    }
    put(&mut bytes, 0x3000 + sweather_per_region::SNOW, &[255]);
    put(
        &mut bytes,
        0x2000 + cweather_manager::MAX_SNOW,
        &8_000_000_i64.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x4000 + sweather_per_province::PERMANENT_SNOW,
        &40_000_i64.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x4000 + sweather_per_province::SNOW,
        &((1_i64 << 40) + 3).to_le_bytes(),
    );
    put(
        &mut bytes,
        0x4000 + sweather_per_province::TEMPERATURE,
        &(-9_007_199_254_740_993_i64).to_le_bytes(),
    );
    put(&mut bytes, 0x4000 + sweather_per_province::RAW_MUD, &[7]);
    bytes
}

fn memory(bytes: &[u8]) -> SliceMemory<'_> {
    SliceMemory {
        base: 0x1000,
        bytes,
    }
}

fn direct(memory: &impl Memory, capacities: &Capacities) -> Captured {
    let mut captured = Captured::with_capacities(capacities);
    let _ticks = captured.capture(memory, 0, 0x1000);
    captured
}

fn split(memory: &impl Memory, capacities: &Capacities) -> (Captured, WeatherJob) {
    let mut captured = Captured::with_capacities(capacities);
    let mut job = WeatherJob::new();
    let ctx = WeatherCopyContext { game_state: 0x1000 };
    let report = copy_weather_refresh(memory, ctx, &mut job);
    assert!(report.raw_ok);
    captured.raw_mode = true;
    assert_eq!(captured.capture(memory, 0, 0x1000), [0; N]);
    captured.decode_weather_payload(0, &job, &mut [0; N]);
    (captured, job)
}

fn assert_matches(split: &Captured, direct: &Captured) {
    assert_eq!(split.outcomes(), direct.outcomes());
    let len = split.regions.outcome.len();
    assert_eq!(split.regions.rows[..len], direct.regions.rows[..len]);
    let len = split.provinces.outcome.len();
    assert_eq!(split.provinces.rows[..len], direct.provinces.rows[..len]);
}

#[test]
fn acquisition_keeps_native_snow_display_bytes_and_raw_flags() {
    let bytes = fixture();
    let captured = direct(&memory(&bytes), &Capacities { rows: [2; N] });
    let tables = captured.tables();
    let TableView::Complete(regions) = &tables[0] else {
        panic!("regions")
    };
    let region = &regions.rows::<tables::WeatherRegions>().unwrap()[0];
    assert_eq!((region.snow, region.max_snow), (255, 8_000_000));
    let TableView::Complete(provinces) = &tables[1] else {
        panic!("provinces")
    };
    assert_eq!(
        provinces.rows::<tables::WeatherProvinces>().unwrap(),
        &[rows::WeatherProvinceRow {
            id: 0,
            snow: (1_i64 << 40) + 3,
            permanent_snow: 40_000,
            display_snow: 200,
            display_mud: 255,
            temperature: -9_007_199_254_740_993,
            raw_mud: 7,
        }]
    );
}

#[test]
fn raw_copy_matches_the_typed_walk_under_truncation_and_faults() {
    let clean = fixture();
    let mut cases = vec![clean.clone()];
    for (address, value) in [
        // Region header invalid: provinces stay available.
        (
            0x2000 + cweather_manager::REGIONS_COUNT,
            (-1_i32).to_le_bytes(),
        ),
        // Province identity mismatch.
        (0x4000 + sweather_per_province::ID, 7_i32.to_le_bytes()),
        // Display array sized for another province count.
        (
            0x2000 + cweather_manager::DISPLAY_MUD + 12,
            2_i32.to_le_bytes(),
        ),
    ] {
        let mut bytes = clean.clone();
        put(&mut bytes, address, &value);
        cases.push(bytes);
    }
    let mut absent = clean.clone();
    put(
        &mut absent,
        0x1000 + cgame_state::WEATHER_MANAGER,
        &0_u64.to_le_bytes(),
    );
    cases.push(absent);
    for bytes in &cases {
        for rows in [[0, 0], [1, 1], [8, 8]] {
            let capacities = Capacities { rows };
            let (split, _) = split(&memory(bytes), &capacities);
            assert_matches(&split, &direct(&memory(bytes), &capacities));
        }
    }
}

/// More provinces than one display-array read chunk: the decode worker
/// serves chunked reads from the recorded bulk spans.
#[test]
fn display_arrays_longer_than_a_read_chunk_decode_from_the_bulk_copy() {
    const PROVINCES: usize = 1500;
    let stride = sweather_per_province::STRIDE;
    let (array, snow, mud) = (
        0x10000,
        0x10000 + PROVINCES * stride,
        0x10000 + PROVINCES * stride + 0x1000,
    );
    let mut bytes = fixture();
    bytes.resize(mud + PROVINCES - 0x1000, 0);
    put(
        &mut bytes,
        0x2000 + cweather_manager::PROVINCES,
        &(array as u64).to_le_bytes(),
    );
    for (offset, data) in [
        (cweather_manager::PROVINCES, array),
        (cweather_manager::DISPLAY_SNOW, snow),
        (cweather_manager::DISPLAY_MUD, mud),
    ] {
        put(&mut bytes, 0x2000 + offset, &(data as u64).to_le_bytes());
        put(
            &mut bytes,
            0x2000 + offset + 8,
            &(PROVINCES as u32).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x2000 + offset + 12,
            &(PROVINCES as u32).to_le_bytes(),
        );
    }
    for index in 0..PROVINCES {
        let row = array + index * stride;
        put(
            &mut bytes,
            row,
            &(sweather_per_province::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, row + 8, &(index as u32).to_le_bytes());
        put(&mut bytes, snow + index, &[index as u8]);
        put(&mut bytes, mud + index, &[(index % 2 * 255) as u8]);
    }
    let capacities = Capacities {
        rows: [8, PROVINCES],
    };
    let (split, _) = split(&memory(&bytes), &capacities);
    assert_eq!(split.provinces.outcome, Outcome::Rows { len: PROVINCES });
    assert_eq!(
        (
            split.provinces.rows[1499].display_snow,
            split.provinces.rows[1499].display_mud
        ),
        (1499_u32 as u8, 255)
    );
    assert_matches(&split, &direct(&memory(&bytes), &capacities));
}

struct FaultReads<'a> {
    memory: SliceMemory<'a>,
    fault: usize,
    failures: Cell<usize>,
}
impl Memory for FaultReads<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if self.fault >= address && self.fault < address.saturating_add(output.len()) {
            self.failures.set(self.failures.get() + 1);
            output.fill(0xab);
            return false;
        }
        self.memory.copy(address, output)
    }
}

#[test]
fn a_faulted_field_or_display_array_is_unavailable_in_both_paths() {
    let bytes = fixture();
    for fault in [
        0x3000 + sweather_per_region::SNOW,
        0x2000 + cweather_manager::MAX_SNOW + 3,
        0x4000 + sweather_per_province::SNOW + 3,
        0x4000 + sweather_per_province::TEMPERATURE + 7,
        0x4000 + sweather_per_province::RAW_MUD,
        0x4810,
    ] {
        let memory = FaultReads {
            memory: memory(&bytes),
            fault,
            failures: Cell::new(0),
        };
        let capacities = Capacities { rows: [2; N] };
        let (split, job) = split(&memory, &capacities);
        assert!(job.log.fault.iter().any(|&fault| fault));
        assert_matches(&split, &direct(&memory, &capacities));
        assert!(
            split
                .outcomes()
                .iter()
                .any(|outcome| matches!(outcome, Outcome::Unavailable { .. }))
        );
    }
}

#[test]
fn staging_overflow_falls_back_to_the_typed_walk_and_grows() {
    let bytes = fixture();
    let ctx = WeatherCopyContext { game_state: 0x1000 };
    let mut tiny = WeatherJob {
        log: RawLog::new(8, 8),
        ..Default::default()
    };
    let report = copy_weather_refresh(&memory(&bytes), ctx, &mut tiny);
    assert!(!report.raw_ok);
    // Growth converges within a few hours; each overflowed hour takes the
    // typed walk.
    let mut hours = 0;
    while tiny.grow_for_next() {
        tiny.grow();
        hours += 1;
        assert!(hours < 16);
        copy_weather_refresh(&memory(&bytes), ctx, &mut tiny);
    }
    assert!(copy_weather_refresh(&memory(&bytes), ctx, &mut tiny).raw_ok);
}
