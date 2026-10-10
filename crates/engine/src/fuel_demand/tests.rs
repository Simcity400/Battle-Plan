use super::*;
use crate::walk::Buffer;
use crate::{ccountry, cgame_state, cstrategic_air, cstrategic_air_manager};
use std::{cell::Cell, collections::BTreeMap};

#[derive(Default)]
struct Map {
    spans: BTreeMap<usize, Vec<u8>>,
    reads: Cell<usize>,
    null_reads: Cell<usize>,
}
impl Memory for Map {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.set(self.reads.get() + 1);
        if address == 0 {
            self.null_reads.set(self.null_reads.get() + 1);
            return false;
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

fn buffers<'a>(
    countries: &'a mut [Country],
    contexts: &'a mut [Context],
    context_slots: &'a mut [usize],
) -> Buffers<'a> {
    Buffers {
        countries: BufferRows {
            rows: Buffer::Fixed(countries),
            len: 0,
            required: 0,
        },
        contexts: BufferRows {
            rows: Buffer::Fixed(contexts),
            len: 0,
            required: 0,
        },
        divisions: BufferRows {
            rows: Buffer::Fixed(&mut []),
            len: 0,
            required: 0,
        },
        fleets: BufferRows {
            rows: Buffer::Fixed(&mut []),
            len: 0,
            required: 0,
        },
        tasks: BufferRows {
            rows: Buffer::Fixed(&mut []),
            len: 0,
            required: 0,
        },
        sites: BufferRows {
            rows: Buffer::Fixed(&mut []),
            len: 0,
            required: 0,
        },
        wings: BufferRows {
            rows: Buffer::Fixed(&mut []),
            len: 0,
            required: 0,
        },
        weather: BufferRows {
            rows: Buffer::Fixed(&mut []),
            len: 0,
            required: 0,
        },
        context_slots: Buffer::Fixed(context_slots),
        names: Buffer::Fixed(&mut []),
        name_len: 0,
        name_required: 0,
        roots: Buffer::Fixed(&mut []),
        groups: Buffer::Fixed(&mut []),
        members: Buffer::Fixed(&mut []),
        children: Buffer::Fixed(&mut []),
        roster_scratch: Buffer::Fixed(&mut []),
        group_names: Buffer::Fixed(&mut []),
        group_countries: Buffer::Fixed(&mut []),
        group_owner_reason: None,
        group_outcomes: None,
        roots_outcome: None,
        navy_reason: None,
    }
}

#[test]
fn cold_and_later_context_index_keeps_five_thousand_original_contexts_and_cached_faults() {
    const STATE: usize = 0x1000;
    const REGIONS: usize = 0x100000;
    let mut contexts = Vec::new();
    let mut slots = Vec::new();
    let mut out = buffers(&mut [], &mut [], &mut []);
    out.contexts.rows = Buffer::Growing(&mut contexts);
    out.context_slots = Buffer::Growing(&mut slots);
    for count in [1_500_usize, 5_000] {
        let mut memory = Map::default();
        memory.span(STATE, 0x800);
        memory.span(crate::air_fuel_defines::CONTEXT_THRESHOLD_RVA, 8);
        for index in 0..count {
            let region = REGIONS + index * 0x100;
            memory.span(region, 0x80);
            memory.put(
                region,
                &(crate::cstrategic_region::VTABLE_RVA as u64).to_le_bytes(),
            );
            memory.put(
                region + crate::cstrategic_region::ID,
                &(index as u32 + 1).to_le_bytes(),
            );
        }
        struct Fault<'a> {
            memory: &'a Map,
            at: usize,
            failures: Cell<usize>,
        }
        impl Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == self.at {
                    self.failures.set(self.failures.get() + 1);
                    return false;
                }
                self.memory.copy(address, output)
            }
        }
        let source = Fault {
            memory: &memory,
            at: REGIONS + 37 * 0x100 + crate::cstrategic_region::PROVINCES_COUNT,
            failures: Cell::new(0),
        };
        out.contexts.len = 0;
        out.contexts.required = 0;
        out.context_slots.fill(0);
        for index in 0..count {
            let value = super::air::context(&source, 0, STATE, REGIONS + index * 0x100, &mut out);
            if index == 37 {
                assert!(value.is_err());
            } else {
                assert_eq!(value.unwrap().0, index);
            }
        }
        assert_eq!(out.contexts.len, count);
        assert_eq!(source.failures.get(), 1);
        let reads = memory.reads.get();
        for index in 0..count {
            let value = super::air::context(&source, 0, STATE, REGIONS + index * 0x100, &mut out);
            if index == 37 {
                assert!(value.is_err());
            } else {
                assert_eq!(value.unwrap().0, index);
            }
        }
        assert_eq!(
            source.failures.get(),
            1,
            "growth cannot retry a failed original context"
        );
        assert_eq!(
            memory.reads.get(),
            reads,
            "all contexts must remain indexed after growth"
        );
    }
}

#[test]
fn captured_empty_and_negative_region_counts_do_not_dereference_weather_manager() {
    const STATE: usize = 0x1000;
    const REGION: usize = 0x4000;
    for count in [0_i32, -1] {
        let mut memory = Map::default();
        memory.span(STATE, 0x800);
        memory.span(REGION, 0x80);
        memory.span(crate::air_fuel_defines::CONTEXT_THRESHOLD_RVA, 8);
        memory.put(
            REGION,
            &(crate::cstrategic_region::VTABLE_RVA as u64).to_le_bytes(),
        );
        memory.put(REGION + crate::cstrategic_region::ID, &7_u32.to_le_bytes());
        memory.put(
            REGION + crate::cstrategic_region::PROVINCES_COUNT,
            &count.to_le_bytes(),
        );
        memory.put(
            crate::air_fuel_defines::CONTEXT_THRESHOLD_RVA,
            &50_000_i64.to_le_bytes(),
        );
        // The manager pointer is readable and null; its object is absent.
        let mut contexts = [Context::default()];
        let mut slots = [0; 2];
        let mut out = buffers(&mut [], &mut contexts, &mut slots);
        let (_, context) = super::air::context(&memory, 0, STATE, REGION, &mut out).unwrap();
        let input = AirInputs {
            reached: true,
            mission_selected: 1,
            active_count: 1,
            base_fuel: 100_000,
            air_weight: 100_000,
            mission_cost: 100_000,
            period: 1,
            region: Some(7),
            context_counts: context.counts,
            context_thresholds: context.thresholds,
            ..Default::default()
        };
        assert_eq!(
            wing_rate(&input, &[]),
            Ok(if count == 0 { 100_000 } else { 0 })
        );
        assert_eq!(out.weather.len, 0);
        assert_eq!(memory.null_reads.get(), 0);
    }
}

#[test]
fn carrier_capacity_acquires_primary_ship_formation_and_reached_module_paths() {
    use crate::{
        cair_base, cair_wing, cair_wing_pool, creference_registry as registry, cship,
        csub_unit_definition,
    };
    const WING: usize = 0x10000;
    const POOL: usize = 0x20000;
    const BASE: usize = 0x30000;
    const SHIP: usize = 0x40000;
    const DEFINITION: usize = 0x50000;
    let base_ref = 1_u64 << 32 | u64::from(cair_base::TYPE);
    let ship_ref = 2_u64 << 32 | u64::from(cship::HANDLE_TYPE);
    let mut refs = registry::tests::Fixture::new(cair_base::TYPE);
    assert_ne!(
        registry::tests::Fixture::index(base_ref),
        registry::tests::Fixture::index(ship_ref)
    );
    refs.put(
        registry::LOW_ROOTS_RVA + cship::HANDLE_TYPE as usize * 8,
        &0x1000_u64.to_le_bytes(),
    );
    refs.put(0x1000 + registry::COUNT, &2_u32.to_le_bytes());
    refs.entry(
        registry::tests::Fixture::index(base_ref),
        1,
        base_ref,
        BASE as u64,
    );
    refs.entry(
        registry::tests::Fixture::index(ship_ref),
        1,
        ship_ref,
        SHIP as u64,
    );
    let mut memory = Map::default();
    for (address, len) in [
        (WING, 0x40),
        (POOL, 0x48),
        (BASE, 0x90),
        (SHIP, 0x803),
        (DEFINITION, 0x610),
        (0x60000, 16),
        (0x70000, 0xc8),
        (0x80000, 16),
        (0x90000, 0x2d1),
    ] {
        memory.span(address, len);
    }
    memory.put(WING + cair_wing::POOL, &(POOL as u64).to_le_bytes());
    memory.put(POOL, &(cair_wing_pool::VTABLE_RVA as u64).to_le_bytes());
    memory.put(
        POOL + cair_wing_pool::AIR_BASE_REFERENCE,
        &base_ref.to_le_bytes(),
    );
    memory.put(
        POOL + cair_wing_pool::DEFINITION,
        &(DEFINITION as u64).to_le_bytes(),
    );
    memory.put(BASE, &(cair_base::VTABLE_RVA as u64).to_le_bytes());
    memory.put(BASE + cair_base::HOST_REFERENCE, &ship_ref.to_le_bytes());
    memory.put(BASE + cair_base::CAPACITY, &(-7_i32).to_le_bytes());
    memory.put(SHIP, &(cship::VTABLE_RVA as u64).to_le_bytes());
    memory.put(SHIP + cship::HANDLE, &ship_ref.to_le_bytes());
    memory.put(
        DEFINITION,
        &(csub_unit_definition::VTABLE_RVA as u64).to_le_bytes(),
    );
    for kind in 0..4 {
        memory.put(
            DEFINITION + 0x600 + kind * 4,
            &((kind as i32 + 1) * 32).to_le_bytes(),
        );
    }
    memory.array(SHIP + 0x60, 0x60000, 1);
    memory.put(0x60000, &0x70000_u64.to_le_bytes());
    memory.array(0x70000 + 0xb8, 0x80000, 1);
    memory.put(0x80000 + 8, &0x90000_u64.to_le_bytes());
    memory.put(0x90000 + 0x2d0, &[1]);
    struct Joined<'a>(&'a Map, &'a registry::tests::Fixture);
    impl Memory for Joined<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.0.copy(address, output) || self.1.copy(address, output)
        }
    }
    for (formation, cached, module, expected) in [
        (0x80, 0, 0, 128),
        (0, 1, 0, 64),
        (0, 1, 1, 96),
        (0, 0, 0, 96),
    ] {
        memory.put(SHIP + cship::FORMATION_FLAGS, &[formation]);
        memory.put(SHIP + 0x802, &[cached]);
        memory.put(SHIP + 0x801, &[module]);
        assert_eq!(
            super::air::capacity(&Joined(&memory, &refs), 0, WING),
            Ok(expected)
        );
    }
    memory.put(DEFINITION + 0x5a8, &(1_u64 << 33).to_le_bytes());
    assert_eq!(
        super::air::capacity(&Joined(&memory, &refs), 0, WING),
        Ok(-7)
    );
}
impl Map {
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
}

#[test]
fn native_wing_uses_retained_active_count_signed_rates_and_wrapping_daily_projection() {
    let mut input = AirInputs {
        reached: true,
        mission_first: 1,
        mission_selected: 1,
        active_count: 7,
        planes: 999,
        base_fuel: 100_000,
        air_weight: 100_000,
        mission_cost: 100_000,
        period: 2,
        ..Default::default()
    };
    assert_eq!(wing_rate(&input, &[]), Ok(700_000));
    input.active_count = -3;
    assert_eq!(wing_rate(&input, &[]), Ok(-300_000));
    input.mission_selected = 0;
    input.planes = 2;
    assert_eq!(wing_rate(&input, &[]), Ok(200_000));
    input.country_modifier.effective = -200_000;
    assert_eq!(wing_rate(&input, &[]), Ok(0));
    input.reached = false;
    assert_eq!(wing_rate(&input, &[]), Ok(0));
    assert_eq!(daily_required(i64::MAX), -24);
    assert_eq!(daily_required(i64::MIN), 0);
    assert!(capacity_gate(1, 0, 100_000));
    assert!(!capacity_gate(1, 0, 100_001));
}

#[test]
fn context_keeps_native_slot_province_multiplicity_threshold_and_period_law() {
    let mut input = AirInputs {
        reached: true,
        mission_selected: 1,
        active_count: 1,
        base_fuel: 100_000,
        air_weight: 100_000,
        mission_cost: 100_000,
        period: 1,
        region: Some(7),
        context_counts: [2; 24],
        context_thresholds: [50_000; 24],
        ..Default::default()
    };
    let weather: Vec<_> = (0..24)
        .flat_map(|slot| {
            [Weather {
                slot,
                selector: 17,
                coefficients: [100_000, 0, 0],
                cell: [75_000, 0, 0],
                scale: 100_000,
                ..Default::default()
            }; 2]
        })
        .collect();
    assert_eq!(wing_rate(&input, &weather), Ok(100_000));
    input.period = 0;
    assert_eq!(wing_rate(&input, &weather), Ok(0));
    assert!(wing_rate(&input, &weather[..47]).is_err());
    let mut wrong_order = weather.clone();
    wrong_order[0].slot = 1;
    assert!(wing_rate(&input, &wrong_order).is_err());
    let sums = context_sums(&input.context_counts, &weather).unwrap();
    assert_eq!(sums, [150_000; 24]);
    let wrapped: Vec<_> = (0..24)
        .map(|slot| Weather {
            slot,
            coefficients: [i64::MAX, 100_000, i64::MAX],
            cell: [2, 100_000, 100_000],
            subtract: -1,
            scale: 100_000,
            ..Default::default()
        })
        .collect();
    // MAX*2 truncates -2/ONE to zero; MAX*ONE truncates to -1.
    assert_eq!(context_sums(&[1; 24], &wrapped), Ok([100_000; 24]));
    for count in [i32::MIN, -1, 0, 2] {
        input.context_counts = [count; 24];
        let samples = if count > 0 { &weather[..] } else { &[] };
        let sums = context_sums(&input.context_counts, samples).unwrap();
        for (active, fuel, weight, cost) in [(7, 1, 100_000, 33_333), (-3, i64::MAX, i64::MIN, -1)]
        {
            input.active_count = active;
            input.base_fuel = fuel;
            input.air_weight = weight;
            input.mission_cost = cost;
            for period in [0, 1] {
                input.period = period;
                assert_eq!(
                    wing_rate(&input, samples),
                    wing_rate_with_context(&input, Some(&sums))
                );
            }
        }
    }
}

#[test]
fn admitted_5000_country_capture_is_linear_and_keeps_empty_family_zero_independent_of_topology() {
    const STATE: usize = 0x1000;
    const COUNTRIES: usize = 0x10000;
    const LINKS: usize = 0x30000;
    const MANAGER: usize = 0x40000;
    const AIRS: usize = 0x50000;
    const COUNTRY: usize = 0x100000;
    const AIR: usize = 0x800000;
    let mut memory = Map::default();
    memory.span(STATE, 0x2000);
    memory.span(COUNTRIES, 5001 * 8);
    memory.span(LINKS, 5001 * 4);
    memory.span(MANAGER, 0x50);
    memory.span(AIRS, 5001 * 8);
    memory.array(STATE + cgame_state::COUNTRIES, COUNTRIES, 5001);
    memory.array(STATE + cgame_state::COUNTRY_LINKS, LINKS, 5001);
    memory.put(
        STATE + cgame_state::STRATEGIC_AIR_MANAGER,
        &(MANAGER as u64).to_le_bytes(),
    );
    memory.put(
        MANAGER,
        &(cstrategic_air_manager::VTABLE_RVA as u64).to_le_bytes(),
    );
    memory.array(MANAGER + cstrategic_air_manager::COUNTRY_AIR, AIRS, 5001);
    for id in 1_u32..=5000 {
        let country = COUNTRY + id as usize * 0x400;
        let air = AIR + id as usize * 0x200;
        memory.span(country, 0x300);
        memory.span(air, 0x100);
        memory.put(COUNTRIES + id as usize * 8, &(country as u64).to_le_bytes());
        memory.put(LINKS + id as usize * 4, &id.to_le_bytes());
        memory.put(country, &(ccountry::VTABLE_RVA as u64).to_le_bytes());
        memory.put(country + ccountry::ID, &id.to_le_bytes());
        memory.put(country + ccountry::ID_INSTANCE, &(id + 7000).to_le_bytes());
        memory.put(AIRS + id as usize * 8, &(air as u64).to_le_bytes());
        memory.put(air, &(cstrategic_air::VTABLE_RVA as u64).to_le_bytes());
        memory.put(air + cstrategic_air::COUNTRY, &id.to_le_bytes());
    }
    let mut countries = vec![Country::default(); 5000];
    let mut out = buffers(&mut countries, &mut [], &mut []);
    capture(&memory, 0, STATE, &mut out).unwrap();
    assert_eq!(out.countries.len, 5000);
    assert_eq!(out.navy_reason, None);
    assert!(matches!(
        out.roots_outcome,
        Some(crate::walk::Outcome::Unavailable { .. })
    ));
    assert!(
        out.countries
            .rows
            .iter()
            .all(|country| country.army_reason.is_none()
                && country.air_reason.is_none()
                && country.divisions.len == 0
                && country.sites.len == 0)
    );
    assert!(
        memory.reads.get() < 5000 * 80,
        "world census must not rewalk per country"
    );
}
