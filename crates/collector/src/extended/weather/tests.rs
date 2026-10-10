use super::*;
use engine::{cgame_state, memory::SliceMemory};
use std::cell::Cell;

fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
    bytes[address - 0x1000..address - 0x1000 + value.len()].copy_from_slice(value);
}

/// Manager 0x2000, one region at 0x3000, one province at 0x4000, display
/// snow at 0x4800, mud at 0x4810 and stored clock at 0x4900.
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

fn fixed_job() -> WeatherJob {
    WeatherJob {
        log: RawLog::new(16, 48 * 1024),
        worker_growth: false,
        regions: DensePayload::new(512 * sweather_per_region::RECORD_BYTES),
        provinces: DensePayload::new(20_000 * sweather_per_province::RECORD_BYTES),
        clocks: DensePayload::new(20_000 * sweather_per_province::CLOCK_RECORD_BYTES),
        attack: vec![Default::default(); 512],
        attack_metadata: engine::selected_attack::ScalarMetadata::try_new(512).unwrap(),
        ..Default::default()
    }
}

fn split(memory: &impl Memory, capacities: &Capacities) -> (Captured, WeatherJob) {
    let mut captured = Captured::with_capacities(capacities);
    let mut job = fixed_job();
    let ctx = WeatherCopyContext {
        base: 0,
        hour: 0,
        game_state: 0x1000,
        attack_guard: Ok(()),
    };
    let report = copy_weather_refresh(memory, ctx, &mut job);
    assert!(report.raw_ok);
    captured.raw_mode = true;
    assert_eq!(
        captured.finish_owned_capture(Err("selected attack modifier layout unverified")),
        [0; N]
    );
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
            temperature: replay::rows::Recorded::Value(-9_007_199_254_740_993),
            raw_mud: replay::rows::Recorded::Value(7),
            local_hour: replay::rows::Recorded::Value(-7),
            night_factor: replay::rows::Recorded::Value(-9_007_199_254_740_993),
            display: rows::Recorded::Value(None),
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
        for rows in [[0, 0], [1, 1], [8, 8]] {
            let capacities = Capacities {
                rows,
                ..Default::default()
            };
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
            &(sweather_per_province::VTABLE_RVA as u64).to_le_bytes(),
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
fn an_ordinary_province_suffix_fault_preserves_the_admitted_display_and_clock_prefix() {
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
        &(sweather_per_province::VTABLE_RVA as u64).to_le_bytes(),
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
    let capacities = Capacities {
        rows: [2, 1],
        ..Default::default()
    };
    let (captured, job) = split(&memory, &capacities);
    assert_eq!(memory.failures.get(), 1);
    assert!(job.log.fault[1]);
    assert_eq!(
        captured.provinces.outcome,
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    assert_eq!(
        (
            captured.provinces.rows[0].display_snow,
            captured.provinces.rows[0].display_mud,
            captured.provinces.rows[0].local_hour,
            captured.provinces.rows[0].night_factor
        ),
        (200, 255, -7, -9_007_199_254_740_993)
    );
    assert_matches(&captured, &direct(&memory, &capacities));
    assert_eq!(
        memory.failures.get(),
        1,
        "the independent direct prefix never reads the suffix"
    );
    let full = direct(
        &memory,
        &Capacities {
            rows: [2, 2],
            ..Default::default()
        },
    );
    assert_eq!(
        full.provinces.outcome,
        Outcome::Unavailable {
            reason: "weather province fields unreadable"
        }
    );
    let mut raw_full = Captured::with_capacities(&Capacities {
        rows: [2, 2],
        ..Default::default()
    });
    raw_full.decode_weather_payload(0, &job, &mut [0; N]);
    assert_matches(&raw_full, &full);
    assert_eq!(
        memory.failures.get(),
        2,
        "owned decoding never retries the failed native field"
    );
}

#[test]
fn clock_payload_shortage_keeps_exact_demand_and_recovers_after_worker_growth() {
    let bytes = fixture();
    let mut job = fixed_job();
    job.clocks = DensePayload::new(sweather_per_province::CLOCK_RECORD_BYTES - 1);
    let context = WeatherCopyContext {
        base: 0,
        hour: 0,
        game_state: 0x1000,
        attack_guard: Ok(()),
    };
    assert!(!copy_weather_refresh(&memory(&bytes), context, &mut job).raw_ok);
    assert_eq!(
        job.clocks.required,
        sweather_per_province::CLOCK_RECORD_BYTES
    );
    assert_eq!(job.clocks.copied_bytes, 0);
    assert!(job.grow_for_next());
    job.grow();
    assert!(copy_weather_refresh(&memory(&bytes), context, &mut job).raw_ok);
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
    captured.decode_weather_payload(0, &job, &mut [0; N]);
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

#[test]
fn staging_overflow_falls_back_to_the_typed_walk_and_grows() {
    let bytes = fixture();
    let ctx = WeatherCopyContext {
        base: 0,
        hour: 0,
        game_state: 0x1000,
        attack_guard: Ok(()),
    };
    let mut tiny = WeatherJob {
        log: RawLog::new(8, 8),
        ..fixed_job()
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

// PRIVATE UNRUN R6 append for crates/collector/src/extended/weather/tests.rs.
// Reuses fixture(), memory(), put(), and assert_matches(). The engine registry
// Fixture is cfg(test)/pub(crate), so collector cannot import that helper.

struct AttackWeatherMemory {
    bytes: Vec<u8>,
    metadata: std::collections::BTreeMap<usize, u8>,
    reads: std::cell::RefCell<Vec<(usize, usize)>>,
    fault_once: Cell<Option<usize>>,
}
impl AttackWeatherMemory {
    fn new(fault: Option<usize>) -> Self {
        use engine::{modifier_definition as definition, modifier_entry as entry};
        let mut bytes = fixture();
        let root = 0x3000 + sweather_per_region::MODIFIERS;
        const VALUES: usize = 0x4a00;
        put(&mut bytes, root, &(VALUES as u64).to_le_bytes());
        put(&mut bytes, root + 8, &2_u32.to_le_bytes());
        put(&mut bytes, root + 12, &2_u32.to_le_bytes());
        for (index, key, raw) in [
            (0, 0xad_u32, -9_007_199_254_740_993_i64),
            (1, 0x1dd, 150_001),
        ] {
            let address = VALUES + index * entry::STRIDE;
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
        // Use the numeric getter's real passive metadata path, not a Memory
        // modifier-value override. Eligibility selector0 needs no callback.
        let mut metadata = std::collections::BTreeMap::new();
        const TABLE: usize = 0x10_0000;
        metadata.extend(
            (TABLE as u64)
                .to_le_bytes()
                .into_iter()
                .enumerate()
                .map(|(index, value)| (definition::TABLE_RVA + index, value)),
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

fn split_attack_at(source: &AttackWeatherMemory, hour: i32) -> (Captured, WeatherJob) {
    let capacity = Capacities {
        rows: [2; N],
        ..Default::default()
    };
    let mut captured = Captured::with_capacities(&capacity);
    let mut job = fixed_job();
    let context = WeatherCopyContext {
        base: 0,
        hour,
        game_state: 0x1000,
        attack_guard: Ok(()),
    };
    assert!(copy_weather_refresh(source, context, &mut job).raw_ok);
    let reads = source.reads.borrow().len();
    captured.raw_mode = true;
    assert_eq!(captured.finish_owned_capture(Ok(())), [0; N]);
    assert!(captured.raw_mode);
    captured.decode_weather_payload(0, &job, &mut [0; N]);
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
    captured.capture_at(&source, 0, 0x1000, 83, Ok(()));
    assert_eq!(captured.outcomes(), [Outcome::Rows { len: 1 }; N]);
    let reads = source.reads.borrow().len();
    let reason = "weather acquisition not supplied";
    assert_eq!(captured.finish_owned_capture(Ok(())), [0; N]);
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
fn real_weather_raw_and_typed_scalar_paths_match_with_unchanged_flags() {
    let source = AttackWeatherMemory::new(None);
    let (copied, job) = split_attack_at(&source, -23);
    assert_eq!(job.log.fault, [false; N]);
    let mut typed = Captured::with_capacities(&Capacities {
        rows: [2; N],
        ..Default::default()
    });
    typed.capture_at(&source, 0, 0x1000, -23, Ok(()));
    assert_matches(&copied, &typed);
    let copied_rows = projected_weather_regions(&copied);
    assert_eq!(copied_rows, projected_weather_regions(&typed));
    assert_eq!(
        copied_rows[0].attack,
        rows::Recorded::Value(rows::WeatherRegionAttackInputs {
            sample_hour: -23,
            attack: Some(-9_007_199_254_740_993),
            breakthrough: Some(100_000),
        })
    );
    assert_eq!(
        (
            copied_rows[0].light_rain,
            copied_rows[0].heavy_rain,
            copied_rows[0].snow,
            copied_rows[0].blizzard,
            copied_rows[0].sandstorm,
            copied_rows[0].arctic_water
        ),
        (1, 0, 255, 1, 2, 3)
    );
    assert_eq!(copied.attack_refusals(), [0, 0]);
    assert_eq!(
        source.attempts(0x3000 + sweather_per_region::MODIFIERS),
        2,
        "one raw accumulator observation per independent acquisition"
    );
}

#[test]
fn real_weather_scalar_faults_are_independent_and_do_not_change_prefix_outcomes() {
    use engine::modifier_definition as definition;
    let clean = AttackWeatherMemory::new(None);
    let clean_prefix = direct(
        &clean,
        &Capacities {
            rows: [2; N],
            ..Default::default()
        },
    );
    let attack_flags = 0x10_0000 + 0xad * definition::STRIDE + definition::FLAGS;
    for (fault, expected_attack, expected_breakthrough, refusals) in [
        (attack_flags, None, Some(100_000), [1_u64, 0]),
        (0x3000 + sweather_per_region::MODIFIERS, None, None, [1, 1]),
        (0x4a00 + engine::modifier_entry::VALUE, None, None, [1, 1]),
    ] {
        let source = AttackWeatherMemory::new(Some(fault));
        let (copied, job) = split_attack_at(&source, 83);
        assert_eq!(source.fault_once.get(), None);
        assert_eq!(
            job.log.fault, [false; N],
            "appended scalar faults are not prefix faults"
        );
        assert_matches(&copied, &clean_prefix);
        let copied_rows = projected_weather_regions(&copied);
        assert_eq!(
            copied_rows[0].attack,
            rows::Recorded::Value(rows::WeatherRegionAttackInputs {
                sample_hour: 83,
                attack: expected_attack,
                breakthrough: expected_breakthrough,
            })
        );
        assert_eq!(copied.attack_refusals(), refusals);
        // Reset the one-shot fault in a separate independent direct source.
        // Its read prefix/failure must match the raw acquisition exactly.
        let typed_source = AttackWeatherMemory::new(Some(fault));
        let mut typed = Captured::with_capacities(&Capacities {
            rows: [2; N],
            ..Default::default()
        });
        typed.capture_at(&typed_source, 0, 0x1000, 83, Ok(()));
        assert_matches(&copied, &typed);
        assert_eq!(copied_rows, projected_weather_regions(&typed));
        assert_eq!(typed_source.fault_once.get(), None);
        assert_eq!(source.attempts(0x3000 + sweather_per_region::MODIFIERS), 1);
        assert_eq!(
            typed_source.attempts(0x3000 + sweather_per_region::MODIFIERS),
            1
        );
        // A subsequent fresh raw pass may recover; no prior-hour refusal carries.
        let (recovered, _) = split_attack_at(&source, 84);
        assert_eq!(
            projected_weather_regions(&recovered)[0].attack,
            rows::Recorded::Value(rows::WeatherRegionAttackInputs {
                sample_hour: 84,
                attack: Some(-9_007_199_254_740_993),
                breakthrough: Some(100_000),
            })
        );
    }
}

#[test]
fn real_weather_short_append_arena_preserves_flags_then_recovers_after_checked_growth() {
    let source = AttackWeatherMemory::new(None);
    let mut job = fixed_job();
    job.attack.clear(); // Deliberately no admitted logical append slots.
    let context = WeatherCopyContext {
        base: 0,
        hour: 91,
        game_state: 0x1000,
        attack_guard: Ok(()),
    };
    assert!(copy_weather_refresh(&source, context, &mut job).raw_ok);
    assert_eq!(job.attack_required, 1);
    assert_eq!(source.attempts(0x3000 + sweather_per_region::MODIFIERS), 0);
    let mut captured = Captured::with_capacities(&Capacities {
        rows: [2; N],
        ..Default::default()
    });
    captured.decode_weather_payload(0, &job, &mut [0; N]);
    assert_eq!(captured.outcomes(), [Outcome::Rows { len: 1 }; N]);
    assert_eq!(captured.attack_refusals(), [1, 1]);
    let prefix = captured.regions.rows[0];
    assert!(job.grow_for_next());
    job.grow();
    assert!(
        copy_weather_refresh(
            &source,
            WeatherCopyContext {
                hour: 92,
                ..context
            },
            &mut job
        )
        .raw_ok
    );
    let reads = source.reads.borrow().len();
    captured.decode_weather_payload(0, &job, &mut [0; N]);
    assert_eq!(source.reads.borrow().len(), reads);
    assert_eq!(captured.regions.rows[0], prefix);
    assert_eq!(captured.attack_refusals(), [0, 0]);
    assert_eq!(
        projected_weather_regions(&captured)[0].attack,
        rows::Recorded::Value(rows::WeatherRegionAttackInputs {
            sample_hour: 92,
            attack: Some(-9_007_199_254_740_993),
            breakthrough: Some(100_000),
        })
    );
    assert!(!job.grow_for_next());
}

#[test]
fn refused_weather_layout_keeps_prefix_and_reports_real_typed_raw_refusals_without_key_reads() {
    let source = AttackWeatherMemory::new(None);
    let reason = "synthetic changed built-in layout";
    let mut job = fixed_job();
    assert!(
        copy_weather_refresh(
            &source,
            WeatherCopyContext {
                base: 0,
                hour: -71,
                game_state: 0x1000,
                attack_guard: Err(reason)
            },
            &mut job
        )
        .raw_ok
    );
    let mut copied = Captured::with_capacities(&Capacities {
        rows: [2; N],
        ..Default::default()
    });
    copied.decode_weather_payload(0, &job, &mut [0; N]);
    let mut typed = Captured::with_capacities(&Capacities {
        rows: [2; N],
        ..Default::default()
    });
    typed.capture_at(&source, 0, 0x1000, -71, Err(reason));
    assert_matches(&copied, &typed);
    assert_eq!(copied.outcomes(), [Outcome::Rows { len: 1 }; N]);
    assert_eq!(copied.attack_refusals(), [1, 1]);
    assert_eq!(typed.attack_refusals(), [1, 1]);
    assert_eq!(
        projected_weather_regions(&copied),
        projected_weather_regions(&typed)
    );
    assert_eq!(
        projected_weather_regions(&copied)[0].attack,
        rows::Recorded::Value(rows::WeatherRegionAttackInputs {
            sample_hour: -71,
            attack: None,
            breakthrough: None
        })
    );
    assert_eq!(source.attempts(0x3000 + sweather_per_region::MODIFIERS), 0);
    assert_eq!(source.attempts(engine::modifier_definition::TABLE_RVA), 0);
}

fn weather_display_memory(fault: Option<usize>) -> AttackWeatherMemory {
    let mut source = AttackWeatherMemory::new(fault);
    source.metadata.extend(
        0x2800_u64
            .to_le_bytes()
            .into_iter()
            .enumerate()
            .map(|(index, byte)| (engine::cmap::SINGLETON_RVA + index, byte)),
    );
    put(
        &mut source.bytes,
        0x2800 + engine::cmap::WIDTH,
        &12_i32.to_le_bytes(),
    );
    put(
        &mut source.bytes,
        0x2800 + engine::cmap::HEIGHT,
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
    source
}

#[test]
fn weather_display_hint_failure_is_named_counted_and_keeps_capacity_after_healthy_capture() {
    let _guard = crate::country_scale::hint_guard();
    let failures = crate::country_scale::DemandFailures::new();
    let source = weather_display_memory(None);
    let before = display::bytes_hint();
    assert_eq!(display::observe_capacity(&source, 0, &failures), Ok(10));
    let retained = before.max(10);
    assert_eq!(display::bytes_hint(), retained);
    source.reads.borrow_mut().clear();
    source.fault_once.set(Some(0x2800 + engine::cmap::WIDTH));
    assert_eq!(
        display::observe_capacity(&source, 0, &failures),
        Err("weather display dimensions unreadable")
    );
    assert_eq!(source.attempts(0x2800 + engine::cmap::WIDTH), 1);
    assert_eq!(display::bytes_hint(), retained);
    let (captured, _) = weather_display_split(&source, 10);
    assert!(matches!(captured.outcomes()[1], Outcome::Rows { len: 1 }));
    assert_eq!(display::observe_capacity(&source, 0, &failures), Ok(10));
    let diagnostics = serde_json::to_value(failures.diagnostics()).unwrap();
    assert_eq!(diagnostics["failed_observations"], 1);
    assert_eq!(
        diagnostics["last_reason"],
        "weather display dimensions unreadable"
    );
    assert_eq!(display::bytes_hint(), retained);
}

fn weather_display_split(source: &impl Memory, capacity: usize) -> (Captured, WeatherJob) {
    let capacities = Capacities {
        rows: [2; N],
        display_bytes: capacity,
    };
    let mut job = fixed_job();
    job.display = DisplayCapture::new(capacity);
    let context = WeatherCopyContext {
        base: 0,
        hour: 42,
        game_state: 0x1000,
        attack_guard: Ok(()),
    };
    assert!(copy_weather_refresh(source, context, &mut job).raw_ok);
    let mut captured = Captured::with_capacities(&capacities);
    captured.decode_weather_payload(0, &job, &mut [0; N]);
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
        copied.decode_weather_payload(0, &job, &mut [0; N]);
        assert_eq!(
            source.reads.borrow().len(),
            native_reads,
            "owned decode never follows live pointers"
        );
        let mut typed = Captured::with_capacities(&capacity);
        typed.capture_at(&source, 0, 0x1000, 42, Ok(()));
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
        // Ordinary weather dirties only the interior; the recorder's next
        // boundary is read live, and never regenerated from this changed byte.
        put(&mut source.bytes, 0x4800, &[210]);
        put(&mut source.bytes, 0x4d00, &[200]);
    }
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
            row.temperature,
            rows::Recorded::Value(-9_007_199_254_740_993)
        );
        assert_eq!(row.local_hour, rows::Recorded::Value(-7));
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
        typed.capture_at(&typed_source, 0, 0x1000, 42, Ok(()));
        assert_eq!(copied.tables(), typed.tables());
    }
    let source = weather_display_memory(None);
    let (mut captured, mut job) = weather_display_split(&source, 0);
    assert_eq!(job.display.required, 10);
    assert_eq!(
        weather_display_row(&captured).display,
        rows::Recorded::Value(None)
    );
    assert!(job.grow_for_next());
    job.grow();
    let mut capacities = Capacities {
        rows: [2; N],
        display_bytes: 0,
    };
    captured.grow(&mut capacities);
    assert!(capacities.display_bytes >= 10);
    assert!(
        copy_weather_refresh(
            &source,
            WeatherCopyContext {
                base: 0,
                hour: 42,
                game_state: 0x1000,
                attack_guard: Ok(())
            },
            &mut job
        )
        .raw_ok
    );
    captured.decode_weather_payload(0, &job, &mut [0; N]);
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
            captured.capture_at(&source, 0, 0x1000, 42, Ok(()));
            captured
        };
        let row = weather_display_row(&captured);
        assert_eq!(row.display, rows::Recorded::Value(None));
        assert_eq!(
            row.temperature,
            rows::Recorded::Value(-9_007_199_254_740_993)
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
fn weather_display_already_running_refresh_with_equal_token_refuses_only_the_boundary() {
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
            // T was stored before capture. A cell changes (0,0) -> (255,0)
            // -> (255,255) between the old reader's two planar reads; the
            // completion store clears work but does not change T again.
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
        let source = Refreshing {
            source: weather_display_memory(None),
            completed: Cell::new(false),
        };
        // Prove the scheduled interleaving would admit a never-existing pair
        // without the initial completion guard, despite equal token reads.
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
            captured.capture_at(&source, 0, 0x1000, 42, Ok(()));
            captured
        };
        let row = weather_display_row(&captured);
        assert_eq!(row.display, rows::Recorded::Value(None));
        assert_eq!(
            row.temperature,
            rows::Recorded::Value(-9_007_199_254_740_993)
        );
        assert_eq!(row.display_snow, 200);
        assert_eq!(
            captured.display_failure(),
            Some("weather display graphics work pending")
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
        // A later independent acquisition can recover with the same T once
        // the native jobs have completed, even after a mid-game attach.
        source.completed.set(true);
        let recovered = if raw {
            weather_display_split(&source, 10).0
        } else {
            let mut captured = Captured::with_capacities(&Capacities {
                rows: [2; N],
                display_bytes: 10,
            });
            captured.capture_at(&source, 0, 0x1000, 43, Ok(()));
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
            let source = Refreshing {
                source: weather_display_memory(None),
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
                captured.capture_at(&source, 0, 0x1000, 42, Ok(()));
                captured
            };
            let row = weather_display_row(&captured);
            assert_eq!(row.display, rows::Recorded::Value(None));
            assert_eq!(row.local_hour, rows::Recorded::Value(-7));
            assert_eq!(
                captured.display_failure(),
                Some(if fault_after_copy {
                    "weather display work count unreadable"
                } else {
                    "weather display graphics work pending"
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
        &(cweather_manager::VTABLE_RVA as u64).to_le_bytes(),
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
            &(sweather_per_region::VTABLE_RVA as u64).to_le_bytes(),
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
            &(sweather_per_province::VTABLE_RVA as u64).to_le_bytes(),
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
        let report = copy_weather_refresh_growing(
            &source,
            WeatherCopyContext {
                base: 0,
                game_state: 0x1000,
                hour: 11,
                attack_guard: Err("selected attack modifier layout unverified"),
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
        captured.decode_weather_payload(0, &job, &mut [0; N]);
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
