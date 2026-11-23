use super::*;
use replay_data::{rows::CountryFuelRow, rows::TableRows};
use replay_frame::{frame::TableInput, frame::TableView};

#[test]
fn owned_country_index_preserves_signed_cache_name_refusal_and_alias_topology() {
    const COUNT: usize = 3;
    const LAST: usize = COUNT - 1;
    let mut capacity = Capacities { rows: [0; N] };
    capacity.rows[0] = COUNT;
    capacity.rows[1] = COUNT;
    capacity.rows[9] = 1;
    let mut captured = Captured::with_capacities(&capacity);
    captured.hour = Some(123);
    captured.lengths[0] = COUNT;
    captured.lengths[1] = COUNT;
    captured.lengths[9] = 1;
    captured.groups[0] = group::GroupRow {
        handle: 9,
        country: 9001,
        ..Default::default()
    };
    captured.group_countries[0] = Some(1);
    captured.root_outcome = Some(Outcome::Rows { len: 1 });
    captured.group_outcomes = Some([
        Outcome::Rows { len: 1 },
        Outcome::Rows { len: 0 },
        Outcome::Rows { len: 0 },
    ]);
    for index in 0..COUNT {
        captured.countries[index] = native::Country {
            country: index as u32 + 1,
            instance: Some(7),
            divisions: native::Span {
                start: index,
                len: 1,
            },
            ..Default::default()
        };
        captured.divisions[index] = native::CachedUnit {
            reference: (index as u64 + 1) << 32 | 29,
            rate: Some(if index == 0 { 0 } else { -1 }),
            name: native::Name {
                bytes: None,
                reason: Some("synthetic name failure"),
            },
            ..Default::default()
        };
    }
    let owned = captured.owned().unwrap();
    assert_eq!(owned.len(), COUNT);
    assert_eq!(owned[0].1.native_rates(), [Ok(0), Ok(0), Ok(0)]);
    assert_eq!(owned[LAST].1.native_rates(), [Ok(-1), Ok(0), Ok(0)]);
    assert_eq!(owned[0].1.groups[0].country, 9001);
    assert!(owned[1].1.groups.is_empty());
    assert_eq!(
        owned[LAST].1.divisions[0].name_reason,
        b"synthetic name failure"
    );
}

#[test]
fn independent_demand_decode_failure_preserves_prefix_and_surrender_targets_do_not_change_identity()
{
    let mut capacity = Capacities::default();
    capacity.rows[0] = 1;
    let mut captured = Captured::with_capacities(&capacity);
    captured.hour = Some(100);
    captured.lengths[0] = 1;
    captured.countries[0] = native::Country {
        country: 1,
        instance: Some(7),
        divisions: native::Span { start: 99, len: 1 },
        ..Default::default()
    };
    let table = || {
        TableInput::Sampled(TableView::Complete(TableRows::CountryFuel(vec![
            CountryFuelRow {
                country: 1,
                current_raw: 12345,
                country_instance: Recorded::Value(Some(8)),
                ..Default::default()
            },
        ])))
    };
    let mut input = table();
    attach(&mut input, &captured).unwrap();
    let TableInput::Sampled(TableView::Complete(TableRows::CountryFuel(rows))) = &input else {
        panic!()
    };
    assert_eq!(rows[0].current_raw, 12345);
    let Recorded::Value(pack) = &rows[0].demand else {
        panic!()
    };
    assert_eq!(pack.reason, b"fuel current demand owned closure invalid");
    captured.countries[0].divisions = native::Span::default();
    let mut input = table();
    attach(&mut input, &captured).unwrap();
    let TableInput::Sampled(TableView::Complete(TableRows::CountryFuel(rows))) = &input else {
        panic!()
    };
    let Recorded::Value(pack) = &rows[0].demand else {
        panic!()
    };
    assert!(pack.reason.is_empty());
    assert_eq!(pack.country_instance, Some(7));
    assert_eq!(pack.native_rates(), [Ok(0); 3]);
}

#[test]
fn reached_foreign_and_untagged_commands_retain_ancestor_names_without_losing_known_rates() {
    let mut capacity = Capacities::default();
    capacity.rows[0] = 2;
    capacity.rows[1] = 2;
    capacity.rows[9] = 3;
    let mut captured = Captured::with_capacities(&capacity);
    captured.hour = Some(44);
    captured.lengths[0] = 2;
    captured.lengths[1] = 2;
    captured.lengths[9] = 3;
    captured.groups[0] = group::GroupRow {
        handle: 9,
        country: 0,
        parent: 19,
        ..Default::default()
    };
    captured.groups[1] = group::GroupRow {
        handle: 19,
        country: 2,
        ..Default::default()
    };
    captured.groups[2] = group::GroupRow {
        handle: 29,
        country: 2,
        ..Default::default()
    };
    captured.group_countries[..3].copy_from_slice(&[Some(0), Some(2), Some(2)]);
    for index in 0..2 {
        captured.countries[index] = native::Country {
            country: index as u32 + 1,
            instance: Some(7),
            divisions: native::Span {
                start: index,
                len: 1,
            },
            ..Default::default()
        };
        captured.divisions[index] = native::CachedUnit {
            rate: Some(-1),
            group: Some(if index == 0 { 9 } else { 39 }),
            ..Default::default()
        };
    }
    let owned = captured.owned().unwrap();
    assert_eq!(
        owned[0]
            .1
            .groups
            .iter()
            .map(|group| group.reference)
            .collect::<Vec<_>>(),
        [9, 19]
    );
    assert_eq!(owned[0].1.groups[0].country, 0);
    assert!(owned[0].1.group_reason.is_empty());
    assert_eq!(
        owned[1]
            .1
            .groups
            .iter()
            .map(|group| group.reference)
            .collect::<Vec<_>>(),
        [19, 29]
    );
    assert_eq!(
        owned[1].1.group_reason,
        b"fuel demand referenced command or ancestor was not captured"
    );
    assert_eq!(owned[1].1.native_rates()[0], Ok(-1));
}

#[derive(Default)]
struct AirMemory {
    spans: std::collections::BTreeMap<usize, Vec<u8>>,
    weather_reads: std::cell::Cell<usize>,
}
impl Memory for AirMemory {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if (0x2400000..0x2400000 + 24 * 24).contains(&address) {
            self.weather_reads.set(self.weather_reads.get() + 1);
        }
        let Some((&start, bytes)) = self.spans.range(..=address).next_back() else {
            return false;
        };
        let Some(source) = bytes.get(address - start..address - start + output.len()) else {
            return false;
        };
        output.copy_from_slice(source);
        true
    }
}
impl AirMemory {
    fn span(&mut self, address: usize, count: usize) {
        self.spans.insert(address, vec![0; count]);
    }
    fn put(&mut self, address: usize, value: &[u8]) {
        let (&start, bytes) = self.spans.range_mut(..=address).next_back().unwrap();
        bytes[address - start..address - start + value.len()].copy_from_slice(value);
    }
    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.put(address, &(data as u64).to_le_bytes());
        self.put(address + 8, &count.to_le_bytes());
        self.put(address + 12, &count.to_le_bytes());
    }
    fn registry(&mut self, handle: u64, pointer: usize, table: usize) {
        use engine_native::{creference_registry as registry, reference_bucket as bucket};
        let root = registry::LOW_ROOTS_RVA.0 + handle as u32 as usize * 8;
        let entries = table + 0x100;
        self.span(root, 8);
        self.span(table, 0x20);
        self.span(entries, 14 * bucket::STRIDE);
        self.put(root, &(table as u64).to_le_bytes());
        self.put(table + registry::ENTRIES, &(entries as u64).to_le_bytes());
        self.put(table + registry::COUNT, &1_u32.to_le_bytes());
        self.put(table + registry::MASK, &7_u32.to_le_bytes());
        self.put(table + registry::OVERFLOW_DISTANCE, &[5]);
        self.put(entries + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
        let hash = handle
            .rotate_left(32)
            .wrapping_mul(registry::HASH_MULTIPLIER);
        let index = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
        let entry = entries + index * bucket::STRIDE;
        self.put(entry + bucket::DISTANCE, &[1]);
        self.put(entry + bucket::TYPE, &handle.to_le_bytes());
        self.put(entry + bucket::OBJECT, &(pointer as u64).to_le_bytes());
    }
}

#[test]
fn cold_and_later_capture_grows_air_text_and_weather_from_original_headers() {
    use engine_native::{
        air_fuel_defines as defines,
        cair_wing_pool,
        ccountry,
        cgame_state,
        cprovince,
        cstrategic_region,
        cweather_manager,
        msvc_string,
    };
    use engine_native_air::{
        air_mission,
        cair_base,
        cair_wing,
        ccountry_air_container as container,
        cstrategic_air,
        cstrategic_air_manager,
    };
    const STATE: usize = 0x1000;
    const COUNTRIES: usize = 0x10000;
    const LINKS: usize = 0x30000;
    const MANAGER: usize = 0x40000;
    const AIRS: usize = 0x50000;
    const BASE: usize = 0x2000000;
    const WING: usize = 0x2100000;
    const REGION: usize = 0x2200000;
    const WEATHER: usize = 0x2300000;
    const GRID: usize = 0x2400000;
    const LIST: usize = 0x2500000;
    const CONTAINERS: usize = 0x2600000;
    const ENTRY: usize = 0x2610000;
    const POOL: usize = 0x2620000;
    const POOLS: usize = 0x2630000;
    const WINGS: usize = 0x2640000;
    const PROVINCES: usize = 0x2650000;
    const PROVINCE: usize = 0x2660000;
    const COSTS: usize = 0x2680000;
    const NAME: usize = 0x2690000;
    const COUNT: usize = 5;
    let mut memory = AirMemory::default();
    for (address, size) in [
        (STATE, 0x2000),
        (COUNTRIES, (COUNT + 1) * 8),
        (LINKS, (COUNT + 1) * 4),
        (MANAGER, 0x50),
        (AIRS, (COUNT + 1) * 8),
        (BASE, 0x90),
        (WING, 0xa00),
        (REGION, 0x80),
        (WEATHER, 0x380),
        (GRID, 24 * 24),
        (LIST, 32),
        (CONTAINERS, (COUNT + 1) * 8),
        (ENTRY, 0xf8),
        (POOL, 0x48),
        (POOLS, 8),
        (WINGS, 24),
        (PROVINCES, 250 * 8),
        (PROVINCE, 0xb0),
        (COSTS, 8),
        (NAME, 301),
    ] {
        memory.span(address, size);
    }
    memory.array(STATE + cgame_state::COUNTRIES, COUNTRIES, COUNT as u32 + 1);
    memory.array(STATE + cgame_state::COUNTRY_LINKS, LINKS, COUNT as u32 + 1);
    memory.put(
        STATE + cgame_state::STRATEGIC_AIR_MANAGER,
        &(MANAGER as u64).to_le_bytes(),
    );
    memory.put(
        STATE + cgame_state::WEATHER_MANAGER,
        &(WEATHER as u64).to_le_bytes(),
    );
    memory.put(
        MANAGER,
        &(cstrategic_air_manager::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    memory.array(
        MANAGER + cstrategic_air_manager::COUNTRY_AIR,
        AIRS,
        COUNT as u32 + 1,
    );
    for id in 1..=COUNT as u32 {
        let country = 0x100000 + id as usize * 0x800;
        let air = 0x1000000 + id as usize * 0x100;
        memory.span(country, 0x600);
        memory.span(air, 0x100);
        memory.put(COUNTRIES + id as usize * 8, &(country as u64).to_le_bytes());
        memory.put(LINKS + id as usize * 4, &id.to_le_bytes());
        memory.put(country, &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
        memory.put(country + ccountry::ID, &id.to_le_bytes());
        memory.put(
            country + ccountry::SURRENDER_TARGET,
            &(id + 7000).to_le_bytes(),
        );
        memory.put(AIRS + id as usize * 8, &(air as u64).to_le_bytes());
        memory.put(air, &(cstrategic_air::VTABLE_RVA.0 as u64).to_le_bytes());
        memory.put(air + cstrategic_air::COUNTRY, &id.to_le_bytes());
        memory.array(air + cstrategic_air::FIRST_AIR_SITES, LIST, 2);
        memory.array(air + cstrategic_air::SECOND_AIR_SITES, LIST, 1);
        memory.put(CONTAINERS + id as usize * 8, &(ENTRY as u64).to_le_bytes());
    }
    let base_ref = 1_u64 << 32 | u64::from(cair_base::TYPE);
    let wing_ref = 2_u64 << 32 | u64::from(cair_wing::TYPE);
    memory.registry(base_ref, BASE, 0x2700000);
    memory.registry(wing_ref, WING + cair_wing::SECONDARY, 0x2710000);
    memory.put(BASE, &(cair_base::VTABLE_RVA.0 as u64).to_le_bytes());
    memory.put(BASE + cair_base::HANDLE, &base_ref.to_le_bytes());
    memory.array(
        BASE + cair_base::COUNTRY_CONTAINERS,
        CONTAINERS,
        COUNT as u32 + 1,
    );
    for ordinal in 0..4 {
        memory.put(LIST + ordinal * 8, &(BASE as u64).to_le_bytes());
    }
    for ordinal in 0..3 {
        memory.put(WINGS + ordinal * 8, &(WING as u64).to_le_bytes());
    }
    memory.put(ENTRY, &(container::VTABLE_RVA.0 as u64).to_le_bytes());
    memory.put(ENTRY + container::COUNTRY, &1_u32.to_le_bytes());
    memory.put(ENTRY + container::CAPACITY_RAW, &100_000_i64.to_le_bytes());
    memory.array(ENTRY + container::POOLS, POOLS, 1);
    memory.put(POOLS, &(POOL as u64).to_le_bytes());
    memory.put(POOL, &(cair_wing_pool::VTABLE_RVA.0 as u64).to_le_bytes());
    memory.array(POOL + cair_wing_pool::WINGS, WINGS, 2);
    memory.put(WING, &(cair_wing::VTABLE_RVA.0 as u64).to_le_bytes());
    memory.put(WING + cair_wing::REFERENCE_TYPE, &wing_ref.to_le_bytes());
    memory.put(WING + 0xa4, &1_u32.to_le_bytes());
    memory.put(WING + 0x170, &1_i32.to_le_bytes());
    memory.put(WING + cair_wing::BASE_FUEL, &100_000_i64.to_le_bytes());
    memory.put(WING + cair_wing::COUNTRY_OVERRIDE, &1_i32.to_le_bytes());
    memory.put(WING + 0xa8, &[1]);
    memory.put(
        WING + cair_wing::MISSION + air_mission::REGION,
        &(REGION as u64).to_le_bytes(),
    );
    memory.put(WING + cair_wing::NAME, &(NAME as u64).to_le_bytes());
    memory.put(
        REGION,
        &(cstrategic_region::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    memory.put(REGION + cstrategic_region::ID, &7001_u32.to_le_bytes());
    memory.array(REGION + cstrategic_region::PROVINCES, PROVINCES, 100);
    memory.put(PROVINCE, &(cprovince::VTABLE_RVA.0 as u64).to_le_bytes());
    for ordinal in 0..250 {
        memory.put(PROVINCES + ordinal * 8, &(PROVINCE as u64).to_le_bytes());
    }
    memory.put(
        WEATHER,
        &(cweather_manager::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    memory.put(WEATHER + 0x350, &100_000_i64.to_le_bytes());
    memory.put(WEATHER + 0x370, &(GRID as u64).to_le_bytes());
    for slot in 0..24 {
        memory.put(GRID + slot * 24, &75_000_i64.to_le_bytes());
    }
    for (address, value) in [
        (defines::AIR_WEIGHT_RVA, 100_000_i64),
        (defines::MISSION_COSTS_RVA, COSTS as i64),
        (defines::CONTEXT_SUBTRACT_RVA, 0),
        (defines::CONTEXT_SCALE_RVA, 100_000),
        (defines::CONTEXT_THRESHOLD_RVA, 50_000),
    ] {
        memory.span(address.0, 8);
        memory.put(address.0, &value.to_le_bytes());
    }
    memory.put(COSTS, &100_000_i64.to_le_bytes());
    let mut captured = Captured::with_capacities(&Capacities { rows: [0; N] });
    for (hour, weather_cell, expected, first_sites, wings, provinces, name_len) in [
        (
            123,
            75_000_i64,
            600_000_i64,
            2_u32,
            2_u32,
            100_u32,
            64_usize,
        ),
        (124, 25_000, 0, 4, 3, 250, 300),
    ] {
        for id in 1..=COUNT {
            let air = 0x1000000 + id * 0x100;
            memory.array(air + cstrategic_air::FIRST_AIR_SITES, LIST, first_sites);
        }
        memory.array(POOL + cair_wing_pool::WINGS, WINGS, wings);
        memory.array(REGION + cstrategic_region::PROVINCES, PROVINCES, provinces);
        memory.put(NAME, &vec![b'x'; name_len]);
        memory.put(NAME + name_len, &[0]);
        memory.put(
            WING + cair_wing::NAME + msvc_string::LENGTH,
            &(name_len as u64).to_le_bytes(),
        );
        memory.put(
            WING + cair_wing::NAME + msvc_string::CAPACITY,
            &(name_len as u64).to_le_bytes(),
        );
        for slot in 0..24 {
            memory.put(GRID + slot * 24, &weather_cell.to_le_bytes());
        }
        memory.weather_reads.set(0);
        captured.capture(&memory, 0, STATE, hour);
        assert_eq!(captured.reason, None);
        let weather_rows = provinces as usize * 24;
        assert_eq!(captured.lengths[0], COUNT);
        assert_eq!(captured.lengths[4], COUNT * (first_sites as usize + 1));
        assert_eq!(captured.lengths[5], captured.lengths[4] * wings as usize);
        assert_eq!(captured.lengths[6], weather_rows);
        assert_eq!(captured.lengths[7], 1);
        assert_eq!(captured.lengths[8], captured.lengths[5] * name_len);
        assert_eq!(
            memory.weather_reads.get(),
            weather_rows,
            "one raw context per supplying boundary"
        );
        let raw_rate = native::wing_rate(
            captured.wings[0].inputs.as_ref().unwrap(),
            &captured.weather[..weather_rows],
        )
        .unwrap();
        let owned = captured.owned().unwrap();
        assert_eq!(owned.len(), COUNT);
        assert_eq!(
            owned
                .iter()
                .map(|(_, pack)| pack.contexts.len() * 24)
                .sum::<usize>(),
            COUNT * 24
        );
        for (_, pack) in &owned {
            assert_eq!(pack.hour, rows::FuelDemandHour::Host);
            assert_eq!(
                pack.sites
                    .iter()
                    .map(|site| (site.list, site.ordinal, site.wings.len()))
                    .collect::<Vec<_>>(),
                (0..first_sites)
                    .map(|ordinal| (0, ordinal, wings as usize))
                    .chain(std::iter::once((1, 0, wings as usize)))
                    .collect::<Vec<_>>()
            );
            assert_eq!(pack.native_rates(), [Ok(0), Ok(0), Ok(expected)]);
            assert_eq!(pack.wing_rate(&pack.sites[0].wings[0]), Ok(raw_rate));
            assert_eq!(pack.sites[0].wings[0].country_override, Some(1));
            assert_eq!(pack.contexts[0].sums, [weather_cell * provinces as i64; 24]);
            assert_eq!(
                pack.sites[0].wings[0].name.as_ref().unwrap(),
                &vec![b'x'; name_len]
            );
        }
        let mut input = TableInput::Sampled(TableView::Complete(TableRows::CountryFuel(
            owned
                .iter()
                .map(|(country, pack)| CountryFuelRow {
                    country: *country,
                    country_instance: Recorded::Value(pack.country_instance),
                    ..Default::default()
                })
                .collect(),
        )));
        let parts = attach(&mut input, &captured).unwrap();
        let TableView::Complete(TableRows::FuelDemandParts(parts)) = parts else {
            panic!("fuel demand parts were not split")
        };
        assert_eq!(
            parts.len(),
            owned
                .iter()
                .map(|(_, pack)| pack.divisions.len()
                    + pack.groups.len()
                    + pack.fleets.len()
                    + pack.task_forces.len()
                    + pack.sites.len()
                    + pack.contexts.len())
                .sum::<usize>()
        );
        let TableInput::Sampled(TableView::Complete(TableRows::CountryFuel(headers))) = &input
        else {
            panic!()
        };
        assert!(headers.iter().all(|row| matches!(
            &row.demand,
            Recorded::Value(pack) if pack.sites.is_empty() && pack.contexts.is_empty()
        )));
        let assembled = replay_frame::rows::with_demand_parts(
            headers,
            &TableView::Complete(TableRows::FuelDemandParts(parts)),
            replay_data::format::FormatVersion::CURRENT,
        )
        .unwrap();
        for (row, (_, pack)) in assembled.iter().zip(&owned) {
            assert_eq!(row.demand, Recorded::Value(pack.clone()));
        }
        for prefix in [0, 3] {
            let mut input = TableInput::Sampled(TableView::Truncated {
                rows: TableRows::CountryFuel(
                    owned[..prefix]
                        .iter()
                        .map(|(country, pack)| CountryFuelRow {
                            country: *country,
                            country_instance: Recorded::Value(pack.country_instance),
                            ..Default::default()
                        })
                        .collect(),
                ),
                missing: 1,
            });
            let parts = attach(&mut input, &captured).unwrap();
            assert_eq!(
                parts,
                TableView::Unavailable {
                    reason: TRUNCATED_PARTS.into()
                }
            );
            let TableInput::Sampled(TableView::Truncated {
                rows: TableRows::CountryFuel(whole),
                ..
            }) = &input
            else {
                panic!()
            };
            for (row, (_, pack)) in whole.iter().zip(&owned) {
                assert_eq!(row.demand, Recorded::Value(pack.clone()));
            }
        }
        let mut input = TableInput::Sampled(TableView::Unavailable {
            reason: "fuel walk failed".into(),
        });
        assert_eq!(
            attach(&mut input, &captured).unwrap(),
            TableView::Unavailable {
                reason: "fuel walk failed".into()
            }
        );
    }
}
