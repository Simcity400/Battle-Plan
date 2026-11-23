use super::*;
use engine_native::cgame_state;
use engine_native_memory::memory::SliceMemory;
use std::cell::Cell;

fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
    bytes[address - 0x1000..address - 0x1000 + value.len()].copy_from_slice(value);
}

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
        &(cweather_manager::VTABLE_RVA.0 as u64).to_le_bytes(),
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
        put(&mut bytes, address as usize, &(vtable.0 as u64).to_le_bytes());
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
    let words = fixture_words();
    for (offset, value) in [
        (sweather_per_province::DRIFT, words.drift),
        (sweather_per_province::STAGED_TEMPERATURE, words.staged),
        (sweather_per_province::WORKING_DRIFT, words.working),
    ] {
        put(&mut bytes, 0x4000 + offset, &value.to_le_bytes());
    }
    put(
        &mut bytes,
        0x2000 + cweather_manager::PROVINCE_CLOCKS,
        &0x4900_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x2000 + cweather_manager::PROVINCE_CLOCKS + 8,
        &3_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x2000 + cweather_manager::PROVINCE_CLOCKS + 12,
        &1_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x4900 + sweather_per_province::CLOCK_LOCAL_HOUR,
        &(-7_i32).to_le_bytes(),
    );
    put(
        &mut bytes,
        0x4900 + sweather_per_province::CLOCK_NIGHT_FACTOR,
        &(-9_007_199_254_740_993_i64).to_le_bytes(),
    );
    put(
        &mut bytes,
        0x2000 + cweather_manager::SUSPEND_FLAGS,
        &[3, 255],
    );
    bytes
}

fn fixture_words() -> replay_data::temperature::State {
    replay_data::temperature::State {
        temperature: -9_007_199_254_740_993,
        drift: 123_456,
        staged: 5,
        working: -654_321,
    }
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
    let ctx = WeatherCopyContext {
        base: 0,
        game_state: 0x1000,
    };
    let report = copy_weather_refresh(memory, ctx, &mut job);
    assert!(report.raw_ok);
    captured.raw_mode = true;
    assert_eq!(captured.finish_owned_capture(), [0; N]);
    captured.decode_weather_payload(0, &job, 0, &mut Default::default(), &mut [0; N]);
    (captured, job)
}

#[test]
fn air_forecast_derives_updated_direction_and_acquires_only_held_direction() {
    let mut bytes = fixture();
    put(
        &mut bytes,
        0x2000 + cweather_manager::SUSPEND_FLAGS,
        &[0, 0],
    );
    let capacities = Capacities {
        rows: [4, 4],
        ..Default::default()
    };
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let (updated, job) = split(&memory, &capacities);
    assert!(
        !job.log
            .spans
            .iter()
            .any(|span| span.addr == 0x2000 + cweather_manager::DAYLIGHT_DIRECTION)
    );
    assert_eq!(
        weather_display_row(&updated).daylight,
        rows::Recorded::Value(Some(rows::WeatherDaylight {
            updated: true,
            held: None,
            held_feather: None
        }))
    );
    put(
        &mut bytes,
        0x2000 + cweather_manager::SUSPEND_FLAGS,
        &[0, 1],
    );
    for (index, value) in [i64::MIN, -3, i64::MAX].into_iter().enumerate() {
        put(
            &mut bytes,
            0x2000 + cweather_manager::DAYLIGHT_DIRECTION + index * 8,
            &value.to_le_bytes(),
        );
    }
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let (held, job) = split(&memory, &capacities);
    assert_eq!(
        job.log
            .spans
            .iter()
            .filter(|span| span.addr == 0x2000 + cweather_manager::DAYLIGHT_DIRECTION)
            .count(),
        1
    );
    assert_eq!(
        weather_display_row(&held).daylight,
        rows::Recorded::Value(Some(rows::WeatherDaylight {
            updated: false,
            held: Some([i64::MIN, -3, i64::MAX]),
            held_feather: None
        }))
    );
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
    let captured = direct(
        &memory(&bytes),
        &Capacities {
            rows: [2; N],
            ..Default::default()
        },
    );
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
            temperature: rows::ProvinceTemperature::anchored(fixture_words()),
            raw_mud: replay_data::rows::Recorded::Value(7),
            clock: rows::DayNightClock::held(replay_data::day_night::Clock {
                local_hour: -7,
                night_factor: -9_007_199_254_740_993,
            }),
            display: rows::Recorded::Value(None),
            temperature_pass: rows::Recorded::Value(Some(rows::TemperaturePass {
                anchor: 0,
                cursor: 0,
                anchor_seed: None,
                hours: Vec::new(),
                witness: replay_data::rows::TemperatureWitness::new(
                    replay_data::temperature::witness([fixture_words().temperature])
                ),
            })),
            modifiers: rows::Recorded::Value(None),
            static_modifiers: rows::Recorded::Value(None),
            daylight: rows::Recorded::Value(Some(rows::WeatherDaylight {
                updated: false,
                held: Some([0, 0, 0]),
                held_feather: None,
            })),
        }]
    );
}

fn two_provinces(gates: [u8; 2]) -> Vec<u8> {
    let mut bytes = fixture();
    for offset in [
        cweather_manager::PROVINCES,
        cweather_manager::DISPLAY_SNOW,
        cweather_manager::DISPLAY_MUD,
        cweather_manager::PROVINCE_CLOCKS,
    ] {
        put(&mut bytes, 0x2000 + offset + 12, &2_u32.to_le_bytes());
    }
    put(
        &mut bytes,
        0x4180,
        &(sweather_per_province::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        0x4180 + sweather_per_province::ID,
        &1_u32.to_le_bytes(),
    );
    let clock = 0x4900 + sweather_per_province::CLOCK_STRIDE;
    put(
        &mut bytes,
        clock + sweather_per_province::CLOCK_LOCAL_HOUR,
        &19_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        clock + sweather_per_province::CLOCK_NIGHT_FACTOR,
        &61_234_i64.to_le_bytes(),
    );
    put(&mut bytes, 0x2000 + cweather_manager::SUSPEND_FLAGS, &gates);
    bytes
}

#[test]
fn clear_gates_record_updated_clocks_from_slot_one_and_hold_the_rest() {
    let capacities = Capacities {
        rows: [2; N],
        ..Default::default()
    };
    let held = |local_hour, night_factor| {
        rows::DayNightClock::held(replay_data::day_night::Clock {
            local_hour,
            night_factor,
        })
    };
    let slot_zero = held(-7, -9_007_199_254_740_993);
    for (gates, slot_one) in [
        ([0, 0], rows::DayNightClock::UPDATED),
        ([0, 1], held(19, 61_234)),
        ([1, 0], held(19, 61_234)),
        ([3, 255], held(19, 61_234)),
    ] {
        let bytes = two_provinces(gates);
        let direct = direct(&memory(&bytes), &capacities);
        let (split, _) = split(&memory(&bytes), &capacities);
        assert_matches(&split, &direct);
        for captured in [&direct, &split] {
            let tables = captured.tables();
            let TableView::Complete(provinces) = &tables[1] else {
                panic!("provinces")
            };
            let clocks: Vec<_> = provinces
                .rows::<tables::WeatherProvinces>()
                .unwrap()
                .iter()
                .map(|row| row.clock)
                .collect();
            assert_eq!(clocks, [slot_zero, slot_one], "{gates:?}");
        }
    }
}

#[test]
fn raw_copy_matches_the_whole_typed_walk_from_any_frame_capacity_and_under_faults() {
    let clean = fixture();
    let mut cases = vec![clean.clone()];
    for (address, value) in [
        (
            0x2000 + cweather_manager::REGIONS_COUNT,
            (-1_i32).to_le_bytes(),
        ),
        (0x4000 + sweather_per_province::ID, 7_i32.to_le_bytes()),
        (
            0x2000 + cweather_manager::DISPLAY_MUD + 12,
            2_i32.to_le_bytes(),
        ),
        (
            0x2000 + cweather_manager::PROVINCE_CLOCKS + 12,
            2_i32.to_le_bytes(),
        ),
        (
            0x2000 + cweather_manager::PROVINCE_CLOCKS + 12,
            (-1_i32).to_le_bytes(),
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
        let ample = Capacities {
            rows: [8, 8],
            ..Default::default()
        };
        for rows in [[0, 0], [1, 1], [8, 8]] {
            let capacities = Capacities {
                rows,
                ..Default::default()
            };
            let (split, _) = split(&memory(bytes), &capacities);
            assert_matches(&split, &direct(&memory(bytes), &ample));
        }
    }
}

#[test]
fn display_arrays_longer_than_a_read_chunk_decode_from_the_bulk_copy() {
    const PROVINCES: usize = 1500;
    let stride = sweather_per_province::STRIDE;
    let (array, snow, mud) = (
        0x10000,
        0x10000 + PROVINCES * stride,
        0x10000 + PROVINCES * stride + 0x1000,
    );
    let clocks = mud + PROVINCES;
    let mut bytes = fixture();
    bytes.resize(
        clocks + PROVINCES * sweather_per_province::CLOCK_STRIDE - 0x1000,
        0,
    );
    put(
        &mut bytes,
        0x2000 + cweather_manager::PROVINCES,
        &(array as u64).to_le_bytes(),
    );
    for (offset, data) in [
        (cweather_manager::PROVINCES, array),
        (cweather_manager::DISPLAY_SNOW, snow),
        (cweather_manager::DISPLAY_MUD, mud),
        (cweather_manager::PROVINCE_CLOCKS, clocks),
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
            &(sweather_per_province::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, row + 8, &(index as u32).to_le_bytes());
        put(&mut bytes, snow + index, &[index as u8]);
        put(&mut bytes, mud + index, &[(index % 2 * 255) as u8]);
        let clock = clocks + index * sweather_per_province::CLOCK_STRIDE;
        put(
            &mut bytes,
            clock + sweather_per_province::CLOCK_LOCAL_HOUR,
            &(index as i32).to_le_bytes(),
        );
        put(
            &mut bytes,
            clock + sweather_per_province::CLOCK_NIGHT_FACTOR,
            &(-(index as i64)).to_le_bytes(),
        );
    }
    let capacities = Capacities {
        rows: [8, PROVINCES],
        ..Default::default()
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
    assert_eq!(
        (
            split.provinces.rows[1499].local_hour,
            split.provinces.rows[1499].night_factor
        ),
        (1499, -1499)
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
        0x4000 + sweather_per_province::DRIFT + 7,
        0x4000 + sweather_per_province::STAGED_TEMPERATURE,
        0x4000 + sweather_per_province::WORKING_DRIFT + 4,
        0x4810,
        0x2000 + cweather_manager::PROVINCE_CLOCKS,
        0x4900 + sweather_per_province::CLOCK_NIGHT_FACTOR + 7,
        0x4900 + sweather_per_province::CLOCK_LOCAL_HOUR + 3,
    ] {
        let memory = FaultReads {
            memory: memory(&bytes),
            fault,
            failures: Cell::new(0),
        };
        let capacities = Capacities {
            rows: [2; N],
            ..Default::default()
        };
        let (split, job) = split(&memory, &capacities);
        assert_eq!(
            memory.failures.get(),
            1,
            "raw capture must not retry its failed source"
        );
        assert!(job.log.fault.iter().any(|&fault| fault));
        assert_matches(&split, &direct(&memory, &capacities));
        assert_eq!(memory.failures.get(), 2, "one independent direct capture");
        assert!(
            split
                .outcomes()
                .iter()
                .any(|outcome| matches!(outcome, Outcome::Unavailable { .. }))
        );
    }
}

#[test]
fn a_failed_snow_ceiling_keeps_the_province_modifier_entries() {
    let mut bytes = fixture();
    let entry = 0x4c00_usize;
    let header = 0x4000 + sweather_per_province::MODIFIERS;
    put(&mut bytes, header, &(entry as u64).to_le_bytes());
    put(&mut bytes, header + 8, &1_u32.to_le_bytes());
    put(&mut bytes, header + 12, &1_u32.to_le_bytes());
    put(
        &mut bytes,
        entry,
        &engine_executable::modifier_keys::ARMY_SPEED_FACTOR.to_le_bytes(),
    );
    put(&mut bytes, entry + 8, &(-20_000_i64).to_le_bytes());
    let capacities = Capacities {
        rows: [2; N],
        ..Default::default()
    };
    let modifiers = |captured: &Captured| {
        let tables = captured.tables();
        let TableView::Complete(provinces) = &tables[1] else {
            panic!("WeatherProvinces is complete without the snow ceiling")
        };
        provinces.rows::<tables::WeatherProvinces>().unwrap()[0]
            .modifiers
            .clone()
    };
    fn split(memory: &impl Memory, capacities: &Capacities) -> Captured {
        let mut captured = Captured::with_capacities(capacities);
        let mut job = WeatherJob::new();
        let ctx = WeatherCopyContext {
            base: 0,
            game_state: 0x1000,
        };
        copy_weather_refresh(memory, ctx, &mut job);
        captured.raw_mode = true;
        assert_eq!(captured.finish_owned_capture(), [0; N]);
        captured.decode_weather_payload(0, &job, 0, &mut Default::default(), &mut [0; N]);
        captured
    }
    let healthy = split(&memory(&bytes), &capacities);
    let recorded = modifiers(&healthy);
    assert!(
        matches!(&recorded, rows::Recorded::Value(Some(entries)) if entries.get(engine_executable::modifier_keys::ARMY_SPEED_FACTOR) == Some(-20_000)),
        "{recorded:?}"
    );
    let faulted = split(
        &FaultReads {
            memory: memory(&bytes),
            fault: 0x2000 + cweather_manager::MAX_SNOW + 3,
            failures: Cell::new(0),
        },
        &capacities,
    );
    assert!(matches!(faulted.outcomes()[0], Outcome::Unavailable { .. }));
    assert_eq!(modifiers(&faulted), recorded);
}

#[test]
fn an_ordinary_province_suffix_fault_fails_the_provinces_read_once() {
    let mut bytes = fixture();
    for offset in [
        cweather_manager::PROVINCES,
        cweather_manager::DISPLAY_SNOW,
        cweather_manager::DISPLAY_MUD,
        cweather_manager::PROVINCE_CLOCKS,
    ] {
        put(&mut bytes, 0x2000 + offset + 12, &2_u32.to_le_bytes());
    }
    let second = 0x4000 + sweather_per_province::STRIDE;
    put(
        &mut bytes,
        second,
        &(sweather_per_province::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        second + sweather_per_province::ID,
        &1_u32.to_le_bytes(),
    );
    let memory = FaultReads {
        memory: memory(&bytes),
        fault: second + sweather_per_province::SNOW,
        failures: Cell::new(0),
    };
    let (captured, job) = split(
        &memory,
        &Capacities {
            rows: [2, 1],
            ..Default::default()
        },
    );
    assert_eq!(memory.failures.get(), 1);
    assert!(job.log.fault[1]);
    assert_eq!(
        captured.provinces.outcome,
        Outcome::Unavailable {
            reason: "weather province fields unreadable"
        }
    );
    let full = direct(
        &memory,
        &Capacities {
            rows: [2, 2],
            ..Default::default()
        },
    );
    assert_matches(&captured, &full);
    assert_eq!(
        memory.failures.get(),
        2,
        "owned decoding never retries the failed native field"
    );
}

#[test]
fn a_short_clock_payload_grows_to_its_exact_demand_in_the_pass() {
    let bytes = fixture();
    let mut job = WeatherJob::new();
    job.clocks = DensePayload::new(sweather_per_province::CLOCK_RECORD_BYTES - 1);
    let context = WeatherCopyContext {
        base: 0,
        game_state: 0x1000,
    };
    assert!(copy_weather_refresh(&memory(&bytes), context, &mut job).raw_ok);
    assert_eq!(
        job.clocks.required,
        sweather_per_province::CLOCK_RECORD_BYTES
    );
    assert!(!job.grow_for_next());
    assert_eq!(
        job.clocks.copied_bytes,
        sweather_per_province::CLOCK_RECORD_BYTES
    );
    assert_eq!(
        job.clocks.copied_fields,
        sweather_per_province::CLOCK_FIELDS.len()
    );
    let mut captured = Captured::with_capacities(&Capacities {
        rows: [2; N],
        ..Default::default()
    });
    captured.decode_weather_payload(0, &job, 0, &mut Default::default(), &mut [0; N]);
    assert_matches(
        &captured,
        &direct(
            &memory(&bytes),
            &Capacities {
                rows: [2; N],
                ..Default::default()
            },
        ),
    );
    job.clear();
    assert_eq!(
        (
            job.clocks.required,
            job.clocks.copied_fields,
            job.clocks.copied_bytes
        ),
        (0, 0, 0)
    );
}


struct AttackWeatherMemory {
    bytes: Vec<u8>,
    metadata: std::collections::BTreeMap<usize, u8>,
    reads: std::cell::RefCell<Vec<(usize, usize)>>,
    fault_once: Cell<Option<usize>>,
}
impl AttackWeatherMemory {
    fn new(fault: Option<usize>) -> Self {
        use engine_native_parts::{
            modifier_definition as definition,
            modifier_entry as entry,
        };
        let mut bytes = fixture();
        let root = 0x3000 + sweather_per_region::MODIFIERS;
        const VALUES: usize = 0x4a00;
        put(&mut bytes, root, &(VALUES as u64).to_le_bytes());
        put(&mut bytes, root + 8, &7_u32.to_le_bytes());
        put(&mut bytes, root + 12, &7_u32.to_le_bytes());
        for (index, key, raw) in [
            (0, 1_u32, -12_345_i64),
            (
                1,
                engine_executable::modifier_keys::NAVAL_DETECTION,
                -99_999,
            ),
            (2, 0x2b, -25_000),
            (
                3,
                engine_executable::modifier_keys::AIR_ATTACK_FACTOR,
                i64::MIN,
            ),
            (
                4,
                engine_executable::modifier_keys::AIR_DEFENCE_FACTOR,
                i64::MAX,
            ),
            (5, 0xad, -9_007_199_254_740_993),
            (6, 0x1dd, 150_001),
        ] {
            let address = VALUES + index * entry::STRIDE;
            put(&mut bytes, address, &[0; entry::STRIDE]);
            put(&mut bytes, address + entry::KEY, &key.to_le_bytes());
            put(&mut bytes, address + entry::VALUE, &raw.to_le_bytes());
        }
        let root = 0x4000 + sweather_per_province::MODIFIERS;
        const PROVINCE_VALUES: usize = 0x4c00;
        put(&mut bytes, root, &(PROVINCE_VALUES as u64).to_le_bytes());
        put(&mut bytes, root + 8, &2_u32.to_le_bytes());
        put(&mut bytes, root + 12, &2_u32.to_le_bytes());
        for (index, key, raw) in [(0, 0x2b_u32, -40_000_i64), (1, 0x52, 25_000)] {
            let address = PROVINCE_VALUES + index * entry::STRIDE;
            put(&mut bytes, address, &[0; entry::STRIDE]);
            put(&mut bytes, address + entry::KEY, &key.to_le_bytes());
            put(&mut bytes, address + entry::VALUE, &raw.to_le_bytes());
        }
        for (index, flag) in [1_u8, 0, 255, 1, 2, 3].into_iter().enumerate() {
            put(
                &mut bytes,
                0x3000 + sweather_per_region::FLAGS + index * 8,
                &[flag],
            );
        }
        let mut metadata = std::collections::BTreeMap::new();
        const TABLE: usize = 0x10_0000;
        metadata.extend(
            (TABLE as u64)
                .to_le_bytes()
                .into_iter()
                .enumerate()
                .map(|(index, value)| (definition::TABLE_RVA.0 + index, value)),
        );
        for (key, flags) in [(0xad_u32, 0_u32), (0x1dd, definition::CLAMP_MAXIMUM)] {
            let row = TABLE + key as usize * definition::STRIDE;
            metadata.extend(
                0_u32
                    .to_le_bytes()
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| (row + definition::FEATURE_SELECTOR + index, value)),
            );
            metadata.extend(
                flags
                    .to_le_bytes()
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| (row + definition::FLAGS + index, value)),
            );
        }
        Self {
            bytes,
            metadata,
            reads: std::cell::RefCell::new(Vec::new()),
            fault_once: Cell::new(fault),
        }
    }
    fn attempts(&self, address: usize) -> usize {
        self.reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == address)
            .count()
    }
}
impl Memory for AttackWeatherMemory {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, destination.len()));
        if self.fault_once.get().is_some_and(|fault| {
            (address..address.saturating_add(destination.len())).contains(&fault)
        }) {
            self.fault_once.set(None);
            destination.fill(0xab);
            return false;
        }
        if memory(&self.bytes).copy(address, destination) {
            return true;
        }
        for (index, byte) in destination.iter_mut().enumerate() {
            let Some(&value) = self.metadata.get(&(address + index)) else {
                return false;
            };
            *byte = value;
        }
        true
    }
}

fn split_attack_at(source: &AttackWeatherMemory) -> (Captured, WeatherJob) {
    let capacity = Capacities {
        rows: [2; N],
        ..Default::default()
    };
    let mut captured = Captured::with_capacities(&capacity);
    let mut job = WeatherJob::new();
    let context = WeatherCopyContext {
        base: 0,
        game_state: 0x1000,
    };
    assert!(copy_weather_refresh(source, context, &mut job).raw_ok);
    let reads = source.reads.borrow().len();
    captured.raw_mode = true;
    assert_eq!(captured.finish_owned_capture(), [0; N]);
    assert!(captured.raw_mode);
    captured.decode_weather_payload(0, &job, 0, &mut Default::default(), &mut [0; N]);
    assert_eq!(
        source.reads.borrow().len(),
        reads,
        "owned decode must not follow native pointers"
    );
    (captured, job)
}

#[test]
fn missing_weather_acquisition_refuses_previously_complete_typed_rows() {
    let source = AttackWeatherMemory::new(None);
    let mut captured = Captured::with_capacities(&Capacities {
        rows: [2; N],
        ..Default::default()
    });
    captured.capture(&source, 0, 0x1000);
    assert_eq!(captured.outcomes(), [Outcome::Rows { len: 1 }; N]);
    let reads = source.reads.borrow().len();
    let reason = "weather acquisition not supplied";
    assert_eq!(captured.finish_owned_capture(), [0; N]);
    assert!(!captured.raw_mode);
    assert_eq!(captured.outcomes(), [Outcome::Unavailable { reason }; N]);
    assert_eq!(
        captured.tables(),
        std::array::from_fn(|_| TableView::Unavailable {
            reason: reason.to_owned(),
        }),
    );
    assert_eq!(source.reads.borrow().len(), reads);
}

fn projected_weather_regions(captured: &Captured) -> Vec<rows::WeatherRegionRow> {
    let views = captured.tables();
    let TableView::Complete(table) = &views[0] else {
        panic!("weather region prefix must remain complete")
    };
    table.rows::<tables::WeatherRegions>().unwrap().to_vec()
}

#[test]
fn regional_map_terms_select_the_same_complete_accumulator_bytes() {
    let source = AttackWeatherMemory::new(None);
    let address = 0x3000 + sweather_per_region::MODIFIERS;
    let former = [
        1,
        engine_executable::modifier_keys::ARMY_SPEED_FACTOR,
        engine_executable::modifier_keys::ARMY_ATTACK_FACTOR,
        engine_executable::modifier_keys::BREAKTHROUGH_FACTOR,
        engine_executable::modifier_keys::ARMY_ATTACK_SPEED_FACTOR,
    ];
    let mut former_values = [engine_native_memory::modifier_cache::Value::default(); 5];
    engine_native_parts::modifier_accumulator::lookup_many_at(
        &source,
        address,
        &former,
        &mut former_values,
    )
    .unwrap();
    let former_reads = source.reads.take();
    let mut values = [engine_native_memory::modifier_cache::Value::default(); COMBAT_KEYS.len()];
    engine_native_parts::modifier_accumulator::lookup_many_at(
        &source,
        address,
        &COMBAT_KEYS,
        &mut values,
    )
    .unwrap();
    assert_eq!(source.reads.take(), former_reads);
    assert_eq!(values[1].raw, -99_999);
    assert_eq!(values[3].raw, i64::MIN);
    assert_eq!(values[4].raw, i64::MAX);
}

#[test]
fn regional_map_terms_distinguish_known_empty_from_refused_accumulator() {
    let mut source = AttackWeatherMemory::new(None);
    let address = 0x3000 + sweather_per_region::MODIFIERS;
    put(&mut source.bytes, address + 12, &0_u32.to_le_bytes());
    let mut job = WeatherJob::new();
    let context = WeatherCopyContext {
        base: 0,
        game_state: 0x1000,
    };
    assert!(copy_weather_refresh(&source, context, &mut job).raw_ok);
    let mut captured = Captured::with_capacities(&Capacities {
        rows: [2; N],
        ..Default::default()
    });
    captured.decode_weather_payload(0, &job, 0, &mut Default::default(), &mut [0; N]);
    let known = projected_weather_regions(&captured).remove(0);
    assert_eq!(
        known.map_modifiers_known,
        replay_data::rows::Recorded::Value(true)
    );
    assert_eq!(
        (known.naval_detection, known.air_attack, known.air_defence),
        (None.into(), None.into(), None.into())
    );

    source.fault_once.set(Some(address));
    assert!(copy_weather_refresh(&source, context, &mut job).raw_ok);
    captured.decode_weather_payload(0, &job, 0, &mut Default::default(), &mut [0; N]);
    let refused = projected_weather_regions(&captured).remove(0);
    assert_eq!(
        refused.map_modifiers_known,
        replay_data::rows::Recorded::Value(false)
    );
    assert_eq!(
        (
            refused.naval_detection,
            refused.air_attack,
            refused.air_defence
        ),
        (None.into(), None.into(), None.into())
    );
}

fn give_fixture_accumulators(typed: &mut Captured) {
    use engine_executable::modifier_keys as key;
    typed.combat = vec![Some([
        Some(-12_345),
        Some(-99_999),
        Some(-25_000),
        Some(i64::MIN),
        Some(i64::MAX),
        Some(-9_007_199_254_740_993),
        Some(150_001),
        None,
    ])];
    typed.air_efficiency = vec![Some(std::sync::Arc::new(vec![
        rows::ModifierBreakdownNode {
            children: 0,
            value: 0,
            name: Vec::new(),
        },
    ]))];
    typed.province_modifiers.clear();
    assert!(typed.province_modifiers.push(Some(&[
        (key::ARMY_SPEED_FACTOR, -40_000),
        (key::HEAT_ATTRITION, 25_000),
    ])));
    typed.custom_sea.clear();
    assert!(typed.custom_sea.push(Some(&[])));
    let mut statics = ProvinceStatics::default();
    assert!(statics.push(Some(&[])));
    typed.province_statics = Some(engine_allocation::shared(statics).unwrap());
}

#[test]
fn weather_regions_record_custom_sea_values_and_refuse_them_alone() {
    let mut source = AttackWeatherMemory::new(None);
    let header = 0x3000 + sweather_per_region::CUSTOM_SEA;
    put(&mut source.bytes, header, &0x4f80_u64.to_le_bytes());
    put(&mut source.bytes, header + 8, &4_u32.to_le_bytes());
    put(&mut source.bytes, header + 12, &3_u32.to_le_bytes());
    put(&mut source.bytes, 0x4f80, &[1, 0, 0, 0, 0xff, 0xff]);
    let region = |source: &AttackWeatherMemory| {
        let mut job = WeatherJob::new();
        let context = WeatherCopyContext {
            base: 0,
            game_state: 0x1000,
        };
        assert!(copy_weather_refresh(source, context, &mut job).raw_ok);
        let mut captured = Captured::with_capacities(&Capacities {
            rows: [2; N],
            ..Default::default()
        });
        captured.decode_weather_payload(0, &job, 0, &mut Default::default(), &mut [0; N]);
        projected_weather_regions(&captured).remove(0)
    };
    let read = region(&source);
    assert_eq!(read.custom_sea, rows::Recorded::Value(Some(vec![1, 0, -1])));
    source.fault_once.set(Some(0x4f82));
    let faulted = region(&source);
    assert_eq!(faulted.custom_sea, rows::Recorded::Value(None));
    assert_eq!((faulted.snow, &faulted.combat), (read.snow, &read.combat));
    put(&mut source.bytes, header + 12, &5_u32.to_le_bytes());
    let invalid = region(&source);
    assert_eq!(invalid.custom_sea, rows::Recorded::Value(None));
    assert_eq!((invalid.snow, &invalid.combat), (read.snow, &read.combat));
}

#[test]
fn weather_regions_record_raw_combat_entries_and_no_witness() {
    let source = AttackWeatherMemory::new(None);
    let (copied, job) = split_attack_at(&source);
    assert_eq!(job.log.fault, [false; N]);
    let root = 0x3000 + sweather_per_region::MODIFIERS;
    assert_eq!(source.attempts(root), 2);
    assert_eq!(
        source
            .reads
            .borrow()
            .iter()
            .filter(|&&read| read == (root, 16))
            .count(),
        2
    );
    assert_eq!(
        source.attempts(engine_native_parts::modifier_definition::TABLE_RVA.0),
        0
    );
    let typed_source = AttackWeatherMemory::new(None);
    let mut typed = Captured::with_capacities(&Capacities {
        rows: [2; N],
        ..Default::default()
    });
    typed.capture(&typed_source, 0, 0x1000);
    assert_eq!(typed_source.attempts(root), 0);
    assert_matches(&copied, &typed);
    give_fixture_accumulators(&mut typed);
    let copied_rows = projected_weather_regions(&copied);
    assert_eq!(copied_rows, projected_weather_regions(&typed));
    let region = &copied_rows[0];
    assert_eq!(region.attack, rows::Recorded::Value(rows::Witnessed(None)));
    assert_eq!(
        (
            region.light_rain,
            region.heavy_rain,
            region.snow,
            region.blizzard,
            region.sandstorm,
            region.arctic_water
        ),
        (1, 0, 255, 1, 2, 3)
    );
    assert_eq!(
        region.combat,
        rows::Recorded::Value(Some(rows::RegionCombatWeather {
            army_attack_factor: Some(-9_007_199_254_740_993),
            breakthrough_factor: Some(150_001),
            army_speed_factor: Some(-25_000),
            army_attack_speed_factor: None,
        }))
    );
    assert_eq!(
        region.air_detection,
        rows::Recorded::Value(Some(Some(-12_345)))
    );
    assert_eq!(
        region.map_modifiers_known,
        replay_data::rows::Recorded::Value(true)
    );
    assert_eq!(region.naval_detection, Some(-99_999).into());
    assert_eq!(region.air_attack, Some(i64::MIN).into());
    assert_eq!(region.air_defence, Some(i64::MAX).into());
    let views = copied.tables();
    let TableView::Complete(provinces) = &views[1] else {
        panic!("weather provinces must stay complete")
    };
    assert_eq!(
        provinces.rows::<tables::WeatherProvinces>().unwrap()[0].modifiers,
        rows::Recorded::Value(Some(rows::ProvinceWeatherModifiers {
            entries: vec![
                rows::ProvinceModifierEntry {
                    key: engine_executable::modifier_keys::ARMY_SPEED_FACTOR,
                    value: -40_000,
                },
                rows::ProvinceModifierEntry {
                    key: engine_executable::modifier_keys::HEAT_ATTRITION,
                    value: 25_000,
                },
            ],
        }))
    );
}

fn weather_display_memory(fault: Option<usize>) -> AttackWeatherMemory {
    let mut source = AttackWeatherMemory::new(fault);
    source.metadata.extend(
        0x2800_u64
            .to_le_bytes()
            .into_iter()
            .enumerate()
            .map(|(index, byte)| (engine_native_parts::cmap::SINGLETON_RVA.0 + index, byte)),
    );
    put(
        &mut source.bytes,
        0x2800 + engine_native_parts::cmap::WIDTH,
        &12_i32.to_le_bytes(),
    );
    put(
        &mut source.bytes,
        0x2800 + engine_native_parts::cmap::HEIGHT,
        &12_i32.to_le_bytes(),
    );
    put(
        &mut source.bytes,
        0x2000 + cweather_manager::DISPLAY_REFRESH_INTERVAL,
        &25_000_i64.to_le_bytes(),
    );
    put(
        &mut source.bytes,
        0x2000 + cweather_manager::DISPLAY_REFRESH_IDENTITY,
        &7.0_f64.to_bits().to_le_bytes(),
    );
    put(
        &mut source.bytes,
        0x2000 + cweather_manager::DISPLAY_WORK_COUNT,
        &0_i32.to_le_bytes(),
    );
    for (offset, pointer, start) in [
        (cweather_manager::DISPLAY_GRID_SNOW, 0x4d00_u64, 0_u8),
        (cweather_manager::DISPLAY_GRID_MUD, 0x4e00_u64, 20_u8),
    ] {
        put(&mut source.bytes, 0x2000 + offset, &pointer.to_le_bytes());
        put(
            &mut source.bytes,
            pointer as usize,
            &(start..start + 9).collect::<Vec<_>>(),
        );
    }
    put(
        &mut source.bytes,
        0x2000 + cweather_manager::DISPLAY_WORK_QUEUE,
        &0x4f00_u64.to_le_bytes(),
    );
    put(&mut source.bytes, 0x4f00, &[0; 4 * 9]);
    put(
        &mut source.bytes,
        0x2000 + cweather_manager::DISPLAY_WORK_CAPACITY,
        &9_i32.to_le_bytes(),
    );
    source
}

fn queue_whole_grid_mark(source: &mut AttackWeatherMemory) {
    for cell in 0..9_i32 {
        put(
            &mut source.bytes,
            0x4f00 + cell as usize * 4,
            &cell.to_le_bytes(),
        );
    }
}

#[test]
fn weather_display_hint_failure_keeps_capacity_after_healthy_capture() {
    let _guard = crate::country_scale::hint_guard();
    let source = weather_display_memory(None);
    let before = display::bytes_hint();
    assert_eq!(display::observe_capacity(&source, 0), Ok(10));
    let retained = before.max(10);
    assert_eq!(display::bytes_hint(), retained);
    source.reads.borrow_mut().clear();
    source
        .fault_once
        .set(Some(0x2800 + engine_native_parts::cmap::WIDTH));
    assert_eq!(
        display::observe_capacity(&source, 0),
        Err("weather display dimensions unreadable")
    );
    assert_eq!(source.attempts(0x2800 + engine_native_parts::cmap::WIDTH), 1);
    assert_eq!(display::bytes_hint(), retained);
    let (captured, _) = weather_display_split(&source, 10);
    assert!(matches!(captured.outcomes()[1], Outcome::Rows { len: 1 }));
    assert_eq!(display::observe_capacity(&source, 0), Ok(10));
    assert_eq!(display::bytes_hint(), retained);
}

fn weather_display_split(source: &impl Memory, capacity: usize) -> (Captured, WeatherJob) {
    let capacities = Capacities {
        rows: [2; N],
        display_bytes: capacity,
    };
    let mut job = WeatherJob::new();
    job.display = DisplayCapture::new(capacity);
    let context = WeatherCopyContext {
        base: 0,
        game_state: 0x1000,
    };
    assert!(copy_weather_refresh(source, context, &mut job).raw_ok);
    let mut captured = Captured::with_capacities(&capacities);
    captured.decode_weather_payload(0, &job, 0, &mut Default::default(), &mut [0; N]);
    (captured, job)
}

fn weather_display_row(captured: &Captured) -> rows::WeatherProvinceRow {
    let views = captured.tables();
    let TableView::Complete(table) = &views[1] else {
        panic!("province prefix unavailable")
    };
    table.rows::<tables::WeatherProvinces>().unwrap()[0].clone()
}

#[test]
fn weather_display_raw_and_typed_attach_dirty_and_full_rebuild_snapshots_match() {
    let mut source = weather_display_memory(None);
    let capacity = Capacities {
        rows: [2; N],
        display_bytes: 10,
    };
    for (case, rebuilt) in [false, false, true].into_iter().enumerate() {
        if rebuilt {
            put(&mut source.bytes, 0x4d00, &[99; 9]);
            put(&mut source.bytes, 0x4e00, &[88; 9]);
            put(
                &mut source.bytes,
                0x2000 + cweather_manager::DISPLAY_REFRESH_IDENTITY,
                &8.0_f64.to_bits().to_le_bytes(),
            );
        }
        let (mut copied, job) = weather_display_split(&source, 10);
        let native_reads = source.reads.borrow().len();
        copied.decode_weather_payload(0, &job, 0, &mut Default::default(), &mut [0; N]);
        assert_eq!(
            source.reads.borrow().len(),
            native_reads,
            "owned decode never follows live pointers"
        );
        let mut typed = Captured::with_capacities(&capacity);
        typed.capture(&source, 0, 0x1000);
        give_fixture_accumulators(&mut typed);
        assert_eq!(copied.tables(), typed.tables());
        let row = weather_display_row(&copied);
        let rows::Recorded::Value(Some(display)) = row.display else {
            panic!("live boundary missing")
        };
        assert_eq!([display.width, display.height], [3, 3]);
        assert_eq!(
            display.edges,
            if rebuilt {
                [vec![99; 5], vec![88; 5]].concat()
            } else {
                vec![2, 5, 6, 7, 8, 22, 25, 26, 27, 28]
            }
        );
        assert_eq!(row.display_snow, if case == 0 { 200 } else { 210 });
        put(&mut source.bytes, 0x4800, &[210]);
        put(&mut source.bytes, 0x4d00, &[200]);
    }
    put(
        &mut source.bytes,
        0x2000 + cweather_manager::DISPLAY_WORK_COUNT,
        &8_i32.to_le_bytes(),
    );
    let (queued, _) = weather_display_split(&source, 10);
    let rows::Recorded::Value(Some(display)) = weather_display_row(&queued).display else {
        panic!("boundary refused while interior cells were queued")
    };
    assert_eq!(display.edges, [vec![99; 5], vec![88; 5]].concat());
    assert_eq!(queued.display_failure(), None);
    let mut typed = Captured::with_capacities(&capacity);
    typed.capture(&source, 0, 0x1000);
    give_fixture_accumulators(&mut typed);
    assert_eq!(queued.tables(), typed.tables());
}

#[test]
fn weather_display_refusals_and_growth_preserve_independent_province_fields() {
    for fault in [
        0x2000 + cweather_manager::DISPLAY_REFRESH_INTERVAL,
        0x2000 + cweather_manager::DISPLAY_WORK_COUNT,
        0x4d02,
        0x4e07,
    ] {
        let source = weather_display_memory(Some(fault));
        let (copied, _) = weather_display_split(&source, 10);
        let row = weather_display_row(&copied);
        assert_eq!(row.display, rows::Recorded::Value(None));
        assert_eq!(
            row.temperature.record(),
            Ok(rows::TemperatureRecord::Anchored(fixture_words()))
        );
        assert_eq!(
            row.clock.state(row.id),
            Ok(rows::ClockState::Held(replay_data::day_night::Clock {
                local_hour: -7,
                night_factor: -9_007_199_254_740_993
            }))
        );
        assert!(copied.display_failure().is_some());
        assert_eq!(
            source
                .reads
                .borrow()
                .iter()
                .filter(|&&(address, len)| (address..address + len).contains(&fault))
                .count(),
            1
        );
        let typed_source = weather_display_memory(Some(fault));
        let mut typed = Captured::with_capacities(&Capacities {
            rows: [2; N],
            display_bytes: 10,
        });
        typed.capture(&typed_source, 0, 0x1000);
        give_fixture_accumulators(&mut typed);
        assert_eq!(copied.tables(), typed.tables());
    }
    let source = weather_display_memory(None);
    let (captured, job) = weather_display_split(&source, 0);
    assert_eq!(job.display.required, 10);
    assert!(matches!(
        weather_display_row(&captured).display,
        rows::Recorded::Value(Some(_))
    ));
    assert_eq!(captured.display_failure(), None);
}

#[test]
fn weather_display_refresh_crossing_the_copy_refuses_only_the_boundary() {
    struct Refreshing {
        source: AttackWeatherMemory,
        refreshed: Cell<bool>,
    }
    impl Memory for Refreshing {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if !self.source.copy(address, destination) {
                return false;
            }
            if address == 0x2000 + cweather_manager::DISPLAY_REFRESH_IDENTITY
                && self.refreshed.get()
            {
                destination.copy_from_slice(&8.0_f64.to_bits().to_le_bytes());
            }
            if (0x4d00..0x4d09).contains(&address) {
                self.refreshed.set(true);
            }
            true
        }
    }
    for raw in [false, true] {
        let source = Refreshing {
            source: weather_display_memory(None),
            refreshed: Cell::new(false),
        };
        let captured = if raw {
            weather_display_split(&source, 10).0
        } else {
            let mut captured = Captured::with_capacities(&Capacities {
                rows: [2; N],
                display_bytes: 10,
            });
            captured.capture(&source, 0, 0x1000);
            captured
        };
        let row = weather_display_row(&captured);
        assert_eq!(row.display, rows::Recorded::Value(None));
        assert_eq!(
            row.temperature.record(),
            Ok(rows::TemperatureRecord::Anchored(fixture_words()))
        );
        assert_eq!(
            captured.display_failure(),
            Some("weather display refreshed during boundary copy")
        );
        assert_eq!(
            source.source.attempts(0x4d02),
            1,
            "no retry after a refresh crossing"
        );
    }
}

#[test]
fn weather_display_whole_grid_rebuild_with_equal_token_refuses_only_the_boundary() {
    struct Refreshing {
        source: AttackWeatherMemory,
        completed: Cell<bool>,
    }
    impl Memory for Refreshing {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if !self.source.copy(address, destination) {
                return false;
            }
            if address == 0x2000 + cweather_manager::DISPLAY_WORK_COUNT {
                let work = if self.completed.get() { 0_i32 } else { 9 };
                destination.copy_from_slice(&work.to_le_bytes());
            }
            if (0x4d00..0x4d09).contains(&address) {
                destination.fill(if self.completed.get() { 255 } else { 0 });
                self.completed.set(true);
            }
            if (0x4e00..0x4e09).contains(&address) {
                destination.fill(if self.completed.get() { 255 } else { 0 });
            }
            true
        }
    }
    for raw in [false, true] {
        let mut marked = weather_display_memory(None);
        queue_whole_grid_mark(&mut marked);
        let source = Refreshing {
            source: marked,
            completed: Cell::new(false),
        };
        let mut snow = [0];
        let mut mud = [0];
        assert!(source.copy(0x4d02, &mut snow));
        assert!(source.copy(0x4e02, &mut mud));
        assert_eq!([snow[0], mud[0]], [0, 255]);
        source.completed.set(false);
        source.source.reads.borrow_mut().clear();
        let captured = if raw {
            weather_display_split(&source, 10).0
        } else {
            let mut captured = Captured::with_capacities(&Capacities {
                rows: [2; N],
                display_bytes: 10,
            });
            captured.capture(&source, 0, 0x1000);
            captured
        };
        let row = weather_display_row(&captured);
        assert_eq!(row.display, rows::Recorded::Value(None));
        assert_eq!(
            row.temperature.record(),
            Ok(rows::TemperatureRecord::Anchored(fixture_words()))
        );
        assert_eq!(row.display_snow, 200);
        assert_eq!(
            captured.display_failure(),
            Some("weather display whole-grid rebuild queued")
        );
        assert_eq!(
            source.source.attempts(0x4d02),
            0,
            "refuse before selecting either plane"
        );
        assert_eq!(
            source
                .source
                .attempts(0x2000 + cweather_manager::DISPLAY_WORK_COUNT),
            1,
            "no waiting or retry for an in-progress refresh"
        );
        source.completed.set(true);
        let recovered = if raw {
            weather_display_split(&source, 10).0
        } else {
            let mut captured = Captured::with_capacities(&Capacities {
                rows: [2; N],
                display_bytes: 10,
            });
            captured.capture(&source, 0, 0x1000);
            captured
        };
        let rows::Recorded::Value(Some(display)) = weather_display_row(&recovered).display else {
            panic!("completed boundary did not recover");
        };
        assert_eq!(display.refresh_identity, 7.0_f64.to_bits());
        assert_eq!(display.edges, vec![255; 10]);
        assert_eq!(recovered.display_failure(), None);
    }
}

#[test]
fn weather_display_completion_guard_refuses_pending_or_faulted_work_after_edges() {
    struct Refreshing {
        source: AttackWeatherMemory,
        copied_snow: Cell<bool>,
        fault_after_copy: bool,
    }
    impl Memory for Refreshing {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == 0x2000 + cweather_manager::DISPLAY_WORK_COUNT && self.copied_snow.get() {
                if self.fault_after_copy {
                    self.source
                        .reads
                        .borrow_mut()
                        .push((address, destination.len()));
                    return false;
                }
                if !self.source.copy(address, destination) {
                    return false;
                }
                destination.copy_from_slice(&9_i32.to_le_bytes());
                return true;
            }
            if !self.source.copy(address, destination) {
                return false;
            }
            if (0x4d00..0x4d09).contains(&address) {
                self.copied_snow.set(true);
            }
            true
        }
    }
    for raw in [false, true] {
        for fault_after_copy in [false, true] {
            let mut marked = weather_display_memory(None);
            queue_whole_grid_mark(&mut marked);
            let source = Refreshing {
                source: marked,
                copied_snow: Cell::new(false),
                fault_after_copy,
            };
            let captured = if raw {
                weather_display_split(&source, 10).0
            } else {
                let mut captured = Captured::with_capacities(&Capacities {
                    rows: [2; N],
                    display_bytes: 10,
                });
                captured.capture(&source, 0, 0x1000);
                captured
            };
            let row = weather_display_row(&captured);
            assert_eq!(row.display, rows::Recorded::Value(None));
            assert_eq!(
                row.clock.state(row.id),
                Ok(rows::ClockState::Held(replay_data::day_night::Clock {
                    local_hour: -7,
                    night_factor: -9_007_199_254_740_993
                }))
            );
            assert_eq!(
                captured.display_failure(),
                Some(if fault_after_copy {
                    "weather display work count unreadable"
                } else {
                    "weather display whole-grid rebuild queued"
                })
            );
            assert_eq!(source.source.attempts(0x4d02), 1, "copy once; no retry");
            assert_eq!(
                source
                    .source
                    .attempts(0x2000 + cweather_manager::DISPLAY_WORK_COUNT),
                2
            );
            assert_eq!(
                source
                    .source
                    .attempts(0x2000 + cweather_manager::DISPLAY_REFRESH_IDENTITY),
                1,
                "final work check precedes the final token check"
            );
        }
    }
}

fn large_fixture(regions: usize, provinces: usize) -> Vec<u8> {
    let region_data = 0x3000;
    let province_data = region_data + regions * sweather_per_region::STRIDE;
    let snow = province_data + provinces * sweather_per_province::STRIDE;
    let mud = snow + provinces;
    let clocks = mud + provinces;
    let mut bytes = vec![0; clocks + provinces * sweather_per_province::CLOCK_STRIDE - 0x1000];
    put(
        &mut bytes,
        0x1000 + cgame_state::WEATHER_MANAGER,
        &0x2000_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x2000,
        &(cweather_manager::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    for (offset, data, count) in [
        (cweather_manager::REGIONS, region_data, regions),
        (cweather_manager::PROVINCES, province_data, provinces),
        (cweather_manager::DISPLAY_SNOW, snow, provinces),
        (cweather_manager::DISPLAY_MUD, mud, provinces),
        (cweather_manager::PROVINCE_CLOCKS, clocks, provinces),
    ] {
        put(&mut bytes, 0x2000 + offset, &(data as u64).to_le_bytes());
        put(
            &mut bytes,
            0x2000 + offset + 8,
            &(count as u32).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x2000 + offset + 12,
            &(count as u32).to_le_bytes(),
        );
    }
    for slot in 0..regions {
        let at = region_data + slot * sweather_per_region::STRIDE;
        put(
            &mut bytes,
            at,
            &(sweather_per_region::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            at + sweather_per_region::ID,
            &(slot as u32).to_le_bytes(),
        );
        put(&mut bytes, at + sweather_per_region::SNOW, &[255]);
    }
    for slot in 0..provinces {
        let at = province_data + slot * sweather_per_province::STRIDE;
        put(
            &mut bytes,
            at,
            &(sweather_per_province::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            at + sweather_per_province::ID,
            &(slot as u32).to_le_bytes(),
        );
        put(
            &mut bytes,
            at + sweather_per_province::SNOW,
            &(slot as i64).to_le_bytes(),
        );
        put(&mut bytes, snow + slot, &[200]);
        put(&mut bytes, mud + slot, &[255]);
        put(
            &mut bytes,
            clocks
                + slot * sweather_per_province::CLOCK_STRIDE
                + sweather_per_province::CLOCK_LOCAL_HOUR,
            &(-7_i32).to_le_bytes(),
        );
    }
    bytes
}

#[test]
fn live_weather_hints_and_original_worker_headers_cover_large_mod_maps() {
    struct HeaderCounts<'a> {
        memory: SliceMemory<'a>,
        counts: [Cell<usize>; 6],
        calls: Cell<usize>,
    }
    impl Memory for HeaderCounts<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.calls.set(self.calls.get() + 1);
            for (index, control) in [
                0x1000 + cgame_state::WEATHER_MANAGER,
                0x2000 + cweather_manager::REGIONS,
                0x2000 + cweather_manager::PROVINCES,
                0x2000 + cweather_manager::DISPLAY_SNOW,
                0x2000 + cweather_manager::DISPLAY_MUD,
                0x2000 + cweather_manager::PROVINCE_CLOCKS,
            ]
            .into_iter()
            .enumerate()
            {
                if address == control {
                    self.counts[index].set(self.counts[index].get() + 1);
                }
            }
            self.memory.copy(address, destination)
        }
    }
    let mut job = WeatherJob::default();
    let mut captured = Captured::with_capacities(&Capacities {
        rows: [1; N],
        ..Default::default()
    });
    for (regions, provinces) in [(5_000, 25_000), (6_000, 50_000)] {
        let bytes = large_fixture(regions, provinces);
        let mut capacity = Capacities::default();
        capacity.presize(&memory(&bytes), 0, 0x1000).unwrap();
        assert!(capacity.rows[0] >= regions && capacity.rows[1] >= provinces);
        let source = HeaderCounts {
            memory: memory(&bytes),
            counts: std::array::from_fn(|_| Cell::new(0)),
            calls: Cell::new(0),
        };
        let report = copy_weather_refresh(
            &source,
            WeatherCopyContext {
                base: 0,
                game_state: 0x1000,
            },
            &mut job,
        );
        assert!(report.raw_ok);
        assert!(!job.log.overflowed);
        assert_eq!(
            job.regions.copied_fields,
            regions * sweather_per_region::FIELDS.len()
        );
        assert_eq!(
            job.provinces.copied_fields,
            provinces * sweather_per_province::FIELDS.len()
        );
        assert_eq!(
            job.clocks.copied_fields,
            provinces * sweather_per_province::CLOCK_FIELDS.len()
        );
        assert!(
            source.counts.iter().all(|count| count.get() == 1),
            "growth retains each original header"
        );
        let calls = source.calls.get();
        captured.decode_weather_payload(0, &job, 0, &mut Default::default(), &mut [0; N]);
        assert_eq!(source.calls.get(), calls, "decode reads only owned data");
        assert_eq!(
            captured.outcomes(),
            [
                Outcome::Rows { len: regions },
                Outcome::Rows { len: provinces }
            ]
        );
        assert_eq!(captured.regions.rows[regions - 1].snow, 255);
        let row = captured.provinces.rows[provinces - 1];
        assert_eq!(
            (
                row.id,
                row.snow,
                row.display_snow,
                row.display_mud,
                row.local_hour
            ),
            ((provinces - 1) as u32, (provinces - 1) as i64, 200, 255, -7)
        );
    }
}

fn tracked(
    memory: &impl Memory,
    hour: i32,
    tracker: &mut temperature::Tracker,
) -> rows::WeatherProvinceRow {
    let mut captured = Captured::with_capacities(&Capacities {
        rows: [2; N],
        ..Default::default()
    });
    let mut job = WeatherJob::new();
    let ctx = WeatherCopyContext {
        base: 0,
        game_state: 0x1000,
    };
    assert!(copy_weather_refresh(memory, ctx, &mut job).raw_ok);
    captured.raw_mode = true;
    captured.finish_owned_capture();
    captured.decode_weather_payload(0, &job, hour, tracker, &mut [0; N]);
    let tables = captured.tables();
    let TableView::Complete(provinces) = &tables[1] else {
        panic!("provinces")
    };
    provinces.rows::<tables::WeatherProvinces>().unwrap()[0].clone()
}

#[test]
fn the_temperature_pass_records_proven_hours_against_the_anchored_words() {
    let mut bytes = fixture();
    let manager = |bytes: &mut Vec<u8>, cursor: i32, window: i32, seed: u32, gates: [u8; 2]| {
        put(
            bytes,
            0x2000 + cweather_manager::UPDATE_CURSOR,
            &cursor.to_le_bytes(),
        );
        put(
            bytes,
            0x2000 + cweather_manager::PROVINCES_PER_UPDATE,
            &window.to_le_bytes(),
        );
        put(bytes, 0x2000 + cweather_manager::SEED, &seed.to_le_bytes());
        put(bytes, 0x2000 + cweather_manager::SUSPEND_FLAGS, &gates);
    };
    let mut tracker = temperature::Tracker::default();
    manager(&mut bytes, 0, 584, 77, [0, 0]);
    let first = tracked(&memory(&bytes), 500, &mut tracker);
    assert_eq!(
        first.temperature.record(),
        Ok(rows::TemperatureRecord::Anchored(fixture_words()))
    );
    let pass = |row: &rows::WeatherProvinceRow| match &row.temperature_pass {
        rows::Recorded::Value(Some(pass)) => pass.clone(),
        other => panic!("pass {other:?}"),
    };
    assert_eq!(
        pass(&first),
        rows::TemperaturePass {
            anchor: 500,
            cursor: 0,
            anchor_seed: None,
            hours: Vec::new(),
            witness: replay_data::rows::TemperatureWitness::new(replay_data::temperature::witness(
                [fixture_words().temperature]
            )),
        }
    );
    put(
        &mut bytes,
        0x4000 + sweather_per_province::TEMPERATURE,
        &42_i64.to_le_bytes(),
    );
    manager(&mut bytes, 0, 584, 78, [0, 0]);
    let second = tracked(&memory(&bytes), 501, &mut tracker);
    assert_eq!(second.temperature, first.temperature);
    let recorded = pass(&second);
    assert_eq!(
        (recorded.anchor, recorded.hours.clone()),
        (500, vec![Some(78)])
    );
    assert_eq!(
        recorded.witness,
        replay_data::rows::TemperatureWitness::new(replay_data::temperature::witness([42]))
    );
    let unread = FaultReads {
        memory: memory(&bytes),
        fault: 0x2000 + cweather_manager::SEED + 2,
        failures: Cell::new(0),
    };
    let refused = tracked(&unread, 502, &mut tracker);
    assert_eq!(refused.temperature_pass, rows::Recorded::Value(None));
    assert_eq!(refused.raw_mud, rows::Recorded::Value(7));
    let third = tracked(&memory(&bytes), 503, &mut tracker);
    assert_eq!(pass(&third).anchor, 503);
}

#[test]
fn province_projection_compares_all_owned_fields_without_allocating() {
    let source = weather_display_memory(None);
    let (mut captured, _) = weather_display_split(&source, 10);
    captured.province_modifiers.clear();
    assert!(captured.province_modifiers.push(Some(&[(1, 17), (2, -19)])));
    captured.temperature_pass = Some(rows::TemperaturePass {
        anchor: 10,
        cursor: 2,
        anchor_seed: Some(7),
        hours: vec![Some(8)],
        witness: replay_data::rows::TemperatureWitness::new(13),
    });
    let native = captured.provinces.rows[0];
    let previous = captured.province_row(&native);
    assert!(collector_test_support::with_allocations_denied(|| {
        captured.province_equal(&native, &previous)
    }));
    let changes: &[fn(&mut rows::WeatherProvinceRow)] = &[
        |row| row.id += 1,
        |row| row.snow += 1,
        |row| row.permanent_snow += 1,
        |row| row.display_snow ^= 1,
        |row| row.display_mud ^= 1,
        |row| row.raw_mud = rows::Recorded::NotRecorded,
        |row| row.temperature = rows::ProvinceTemperature::anchored(Default::default()),
        |row| row.clock = rows::DayNightClock::UPDATED,
        |row| row.daylight = rows::Recorded::Value(None),
        |row| row.modifiers = rows::Recorded::Value(None),
        |row| {
            row.modifiers
                .require_mut("modifiers")
                .unwrap()
                .as_mut()
                .unwrap()
                .entries[0]
                .value += 1
        },
        |row| {
            row.temperature_pass
                .require_mut("pass")
                .unwrap()
                .as_mut()
                .unwrap()
                .hours[0] = None
        },
        |row| {
            row.display
                .require_mut("display")
                .unwrap()
                .as_mut()
                .unwrap()
                .edges[0] ^= 1
        },
        |row| {
            row.display
                .require_mut("display")
                .unwrap()
                .as_mut()
                .unwrap()
                .refresh_identity ^= 1
        },
    ];
    for (index, change) in changes.iter().enumerate() {
        let mut changed = previous.clone();
        change(&mut changed);
        assert_ne!(changed, previous, "fixture change {index}");
        assert!(
            !collector_test_support::with_allocations_denied(|| {
                captured.province_equal(&native, &changed)
            }),
            "field {index}"
        );
    }
    let mut other = native;
    other.id = 1;
    captured.clocks_updated = true;
    let previous = captured.province_row(&other);
    assert!(collector_test_support::with_allocations_denied(|| {
        captured.province_equal(&other, &previous)
    }));
}

#[test]
fn province_projection_matches_complete_frames_across_changes_and_keyframes() {
    use collector_projection::projection::{Projection, ProjectionScratch};
    use replay_frame::frame::{FrameEncoder, FrameInput, Metrics, TableInput};
    let source = weather_display_memory(None);
    let (mut captured, _) = weather_display_split(&source, 10);
    let mut complete = FrameEncoder::new();
    let mut projected = FrameEncoder::new();
    let scratch = ProjectionScratch::default();
    for hour in 0..8 {
        if hour == 2 {
            captured.provinces.rows[0].snow += 1;
        }
        if hour == 3 {
            captured.province_modifiers.clear();
        }
        if hour == 4 {
            captured.clocks_updated = !captured.clocks_updated;
        }
        if hour == 5 {
            captured.provinces.outcome = Outcome::Truncated {
                len: 1,
                unread_rows: 1,
            };
        }
        if hour == 6 {
            captured.provinces.outcome = Outcome::Rows { len: 1 };
        }
        let force = hour == 7;
        let frame = |values: [TableInput; N]| {
            let mut tables = std::array::from_fn(|_| {
                crate::table_conversion::unavailable("fixture table").into()
            });
            for (table, value) in TABLES.into_iter().zip(values) {
                tables[table.index()] = value;
            }
            FrameInput {
                branch: 1,
                hour,
                tables,
                metrics: Metrics::default(),
            }
        };
        let expected = complete
            .encode_input(frame(captured.tables().map(Into::into)), force)
            .unwrap();
        let mut projection = Projection::new(&projected, 1, hour, force, &scratch);
        let inputs = captured.table_inputs(Some(&mut projection));
        if hour == 1 {
            assert!(matches!(&inputs[1], TableInput::ComparedEqual(_)));
        }
        if hour == 2 {
            assert!(matches!(&inputs[1], TableInput::SliceUpdate { update, .. }
                if update.keys.as_slice() == [0]));
        }
        assert_eq!(
            projected.encode_input(frame(inputs), force).unwrap(),
            expected,
            "hour {hour}"
        );
    }
}
