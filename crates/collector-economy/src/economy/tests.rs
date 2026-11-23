use super::*;
use replay_frame::frame::TableView;

#[test]
fn final_worker_failure_keeps_owned_economy_and_publishes_terminal_fuel_demand_reason() {
    const REASON: &str = "synthetic final worker failure";
    for raw_owned in [false, true] {
        let mut captured = Captured::with_capacities(&Capacities {
            rows: [1; N],
            ..Default::default()
        });
        captured.set_schedule(0, true);
        captured.fuel.rows[0] = ccountry_fuel_status::Row {
            country: 5_000,
            current_raw: 42,
            country_instance: Some(77),
            ..Default::default()
        };
        captured.fuel.outcome = Outcome::Rows { len: 1 };
        captured.rest_raw_mode = raw_owned;
        let before = captured.outcomes();
        captured.fail_final_capture(777, true, REASON);
        if raw_owned {
            assert!(captured.rest_raw_mode);
            assert_eq!(captured.outcomes(), before);
            let tables = captured.tables();
            let TableView::Complete(rows::TableRows::CountryFuel(fuel)) = &tables[0] else {
                panic!("owned fuel row discarded");
            };
            assert_eq!(fuel[0].current_raw, 42);
            let rows::Recorded::Value(demand) = &fuel[0].demand else {
                panic!("terminal demand verdict missing");
            };
            assert_eq!(demand.hour, rows::FuelDemandHour::Host);
            assert_eq!(demand.country_instance, Some(77));
            assert_eq!(demand.reason, REASON.as_bytes());
            assert!(demand.divisions.is_empty() && demand.sites.is_empty());
        } else {
            assert!(!captured.rest_raw_mode);
            assert_eq!(
                captured.outcomes(),
                [Outcome::Unavailable { reason: REASON }; N]
            );
        }
    }
}

mod resource_fixture {
    use super::*;
    use engine_resources::ccountry_resources::*;
    use engine_resources::{
        cresource_exchange as trade, cstrategic_resource_database as database,
        cstrategic_resource_pool,
    };
    use std::collections::BTreeMap;
    const POOLS: [usize; 9] = [
        PRODUCED_POOL,
        ADJUSTMENT_POOL,
        IMPORTED_POOL,
        USED_POOL,
        SPECIAL_TOTAL_POOL,
        PRODUCTION_LINE_POOL,
        TO_EXPORT_POOL,
        BASE_EXPORT_POOL,
        EXPORTED_POOL,
    ];
    #[derive(Default)]
    pub struct Fixture {
        pub bytes: BTreeMap<usize, u8>,
    }
    impl Memory for Fixture {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            for (index, byte) in destination.iter_mut().enumerate() {
                let Some(value) = address.checked_add(index).and_then(|p| self.bytes.get(&p))
                else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }
    impl Fixture {
        pub fn put(&mut self, address: usize, bytes: &[u8]) {
            for (index, byte) in bytes.iter().enumerate() {
                self.bytes.insert(address + index, *byte);
            }
        }
        pub fn u32(&mut self, address: usize, value: u32) {
            self.put(address, &value.to_le_bytes());
        }
        pub fn u64(&mut self, address: usize, value: u64) {
            self.put(address, &value.to_le_bytes());
        }
        pub fn array(&mut self, address: usize, data: usize, count: u32) {
            self.u64(address, data as u64);
            self.u32(address + 8, count);
            self.u32(address + 12, count);
        }
        pub fn new() -> Self {
            let mut f = Self::default();
            f.array(0x1000 + cgame_state::COUNTRIES, 0x1500, 3);
            f.u64(0x1508, 0x2000);
            f.u64(0x1510, 0x4000);
            f.array(0x1000 + cgame_state::COUNTRY_LINKS, 0x1600, 8);
            f.u32(0x1604, 1);
            f.u32(0x1608, 2);
            f.u32(0x161c, 1);
            for (ordinal, (country, resources, raw)) in [(0x2000, 0x6000, 7), (0x4000, 0x7000, 2)]
                .into_iter()
                .enumerate()
            {
                f.u64(country, ccountry::VTABLE_RVA.0 as u64);
                f.u32(country + ccountry::ID, raw);
                f.u64(country + ccountry::RESOURCES, resources as u64);
                f.u64(resources, VTABLE_RVA.0 as u64);
                f.u32(resources + OWNER, raw);
                f.array(resources + IMPORT_ROUTES, 0, 0);
                let buckets = 0xc000 + ordinal * 0x200;
                f.array(resources + EXPORT_BUCKETS, buckets, 2);
                f.array(buckets, 0, 0);
                f.array(buckets + EXPORT_BUCKET_STRIDE, 0, 0);
                for (pool, offset) in POOLS.into_iter().enumerate() {
                    let data = 0x10000 + ordinal * 0x2000 + pool * 0x80;
                    f.u64(
                        resources + offset,
                        cstrategic_resource_pool::VTABLE_RVA.0 as u64,
                    );
                    f.array(resources + offset + 8, data, 2);
                    f.put(data, &[0; 32]);
                    f.u64(data + 16, (4_294_967_297_i64 + pool as i64) as u64);
                    f.u32(data + 24, 19);
                }
            }
            f.put(0xa000, &[0; trade::SIZE]);
            f.u64(0xa000, trade::VTABLE_RVA.0 as u64);
            f.u64(0xa000 + trade::REFERENCE, 0x1234_5678_0000_0033);
            f.put(0xa000 + trade::REGISTERED, &[1]);
            f.u32(0xa000 + trade::OWNER, 2);
            f.u64(0xa000 + trade::EXPORTER_RESOURCES, 0x6000);
            f.u32(0xa000 + trade::IMPORTER, 2);
            f.u64(0xa000 + trade::RESOURCE, 0xd000);
            f.u64(0xa000 + trade::DELIVERED, 4_294_967_299);
            f.u64(0xa000 + trade::EFFICIENCY_A, 100_000);
            f.u64(0xa000 + trade::EFFICIENCY_B, 99_999);
            f.u32(0xa000 + trade::REQUIRED_FACTORIES, 9);
            f.u32(0xa000 + trade::ASSIGNED_FACTORIES, 8);
            f.array(0x7000 + IMPORT_ROUTES, 0xb100, 1);
            f.u64(0xb100, 0xa000);
            f.array(0xc000 + EXPORT_BUCKET_STRIDE, 0xc100, 1);
            f.u64(0xc100, 0xa000);
            f.u64(0xd000, cstrategic_resource::VTABLE_RVA.0 as u64);
            f.u32(0xd000 + cstrategic_resource::INDEX, 1);
            f.u32(0xd000 + cstrategic_resource::TOKEN_ID, 19);
            f.put(0xd000 + cstrategic_resource::ACTIVE, &[1]);
            f.u64(0xd000 + cstrategic_resource::FACTORY_COST, 4_294_967_300);
            f.u64(database::SINGLETON_RVA.0, 0xe000);
            f.array(0xe000 + database::ITEMS, 0xe100, 2);
            f.u64(0xe100, 0xdf00);
            f.u64(0xe108, 0xd000);
            f.array(0xe000 + database::KEYS, 0xe200, 1);
            f.u32(0xe200, 19);
            f
        }
    }
}

#[test]
fn fuel_suffix_assembly_materializes_before_an_equal_scalar_projection() {
    use crate::projection::{Projection, ProjectionScratch};
    use replay_frame::{frame::FrameEncoder, frame::FrameInput, frame::Metrics};
    let source = |hour, row| {
        let mut input = FrameInput {
            branch: 1,
            hour,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| unavailable("synthetic unused").into()),
        };
        input.tables[TableId::CountryFuel.index()] =
            TableView::Complete(rows::TableRows::CountryFuel(vec![row])).into();
        input
    };
    let mut captured = Captured::with_capacities(&Capacities {
        rows: [1; N],
        foreign_sources: 0,
        demand: Default::default(),
    });
    captured.fuel.rows[0] = ccountry_fuel_status::Row {
        country: 1,
        current_raw: 99,
        country_instance: Some(7),
        ..Default::default()
    };
    captured.fuel.outcome = Outcome::Rows { len: 1 };
    let mut encoder = FrameEncoder::new();
    encoder
        .encode_input(
            source(
                0,
                rows::CountryFuelRow {
                    country: 1,
                    current_raw: 99,
                    country_instance: rows::Recorded::Value(Some(7)),
                    ..Default::default()
                },
            ),
            true,
        )
        .unwrap();
    assert!(
        encoder
            .comparison_baseline::<tables::CountryFuel>(1, 1, false)
            .is_some()
    );
    let scratch = ProjectionScratch::default();
    let mut projection = Projection::new(&encoder, 1, 1, false, &scratch);
    let inputs = captured.table_inputs(Some(&mut projection));
    let TableInput::Sampled(TableView::Complete(rows::TableRows::CountryFuel(rows))) = &inputs[0]
    else {
        panic!("owned suffixes require the materialized supplying rows");
    };
    assert_eq!(rows[0].current_raw, 99);
    assert!(matches!(&rows[0].demand, rows::Recorded::Value(pack) if !pack.reason.is_empty()));
}

fn resource_job() -> EconomyRestJob {
    EconomyRestJob {
        log: RawLog::new(256, 8192),
        resources: OwnedResources::new([4, 8, 4], 4),
        ..Default::default()
    }
}

#[test]
fn owned_resource_group_matches_direct_rows_and_all_independent_capacity_limits() {
    let memory = engine_native_memory::memory::CountedMemory::new(resource_fixture::Fixture::new());
    for definition_capacity in [0, 1, 2] {
        for resource_capacity in [0, 1, 3] {
            for exchange_capacity in [0, 1, 2] {
                let mut capacities = e_capacities();
                capacities.rows[6..9].copy_from_slice(&[
                    definition_capacity,
                    resource_capacity,
                    exchange_capacity,
                ]);
                let mut direct = Captured::with_capacities(&capacities);
                let native = read_accounting::Capture::new();
                direct.capture(&memory, 0, 0x1000, 10, false);
                let direct_counts = native.finish();
                let mut captured = Captured::with_capacities(&capacities);
                let ctx = captured.resource_copy_context(0, 0x1000, 10, false);
                let mut first = resource_job();
                let native = read_accounting::Capture::new();
                // SAFETY: Synthetic unit-test memory with no live market hooks.
                let report = unsafe { copy_economy_rest_refresh(&memory, ctx, &mut first) };
                assert!(report.raw_ok);
                let counts = read_accounting::snapshot();
                let mut expected_counts = read_accounting::ZERO;
                expected_counts[TABLES[2].index()] = counts[TABLES[2].index()];
                for table in &TABLES[6..9] {
                    expected_counts[table.index()] = direct_counts[table.index()];
                }
                assert!(counts[TABLES[2].index()].spans > 0);
                assert!(counts[TABLES[6].index()].spans > 0);
                assert_eq!(counts, expected_counts);
                captured.decode_economy_rest_payload(0, &first, &mut [0; N]);
                assert_eq!(native.finish(), counts);
                assert_eq!(&captured.outcomes()[6..9], &direct.outcomes()[6..9]);
                assert_eq!(
                    &captured.definitions.rows[..captured.definitions.outcome.len()],
                    &direct.definitions.rows[..direct.definitions.outcome.len()]
                );
                assert_eq!(
                    &captured.resources.rows[..captured.resources.outcome.len()],
                    &direct.resources.rows[..direct.resources.outcome.len()]
                );
                assert_eq!(
                    &captured.exchanges.rows[..captured.exchanges.outcome.len()],
                    &direct.exchanges.rows[..direct.exchanges.outcome.len()]
                );
                assert!(first.log.spans.iter().all(|span| span.mask & 0x1c0 == 0));
                assert_eq!(
                    &report.raw_bytes[6..9],
                    &[0; 3],
                    "owned rows do not masquerade as raw indexed payload bytes"
                );
            }
        }
    }
}

struct ResourceTrace<'a> {
    source: &'a resource_fixture::Fixture,
    reads: std::cell::RefCell<Vec<usize>>,
    fault: Option<usize>,
}
impl Memory for ResourceTrace<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let failed_before = self.reads.borrow().contains(&address);
        self.reads.borrow_mut().push(address);
        if self.fault == Some(address) && !failed_before {
            return false;
        }
        self.source.copy(address, output)
    }
}

#[test]
fn cold_resource_worker_consumes_original_headers_when_the_frame_has_nonzero_live_grants() {
    let source = resource_fixture::Fixture::new();
    let memory = engine_native_memory::memory::CountedMemory::new(ResourceTrace {
        source: &source,
        reads: Default::default(),
        fault: None,
    });
    let mut captured = Captured::with_capacities(&e_capacities());
    let ctx = captured.resource_copy_context(0, 0x1000, 10, false);
    assert!(ctx.resource_rows.iter().all(|&rows| rows > 0));
    assert!(ctx.resource_scratch > 0);
    let mut job = EconomyRestJob::new();
    assert!(job.resources.definitions.is_empty());
    assert!(job.resources.resources.is_empty());
    assert!(job.resources.exchanges.is_empty());
    assert!(job.resources.scratch.is_empty());
    let native = read_accounting::Capture::new();
    // SAFETY: Synthetic unit-test memory with no live market hooks.
    assert!(unsafe { copy_economy_rest_refresh(&memory, ctx, &mut job) }.raw_ok);
    let reads = read_accounting::snapshot();
    captured.decode_economy_rest_payload(0, &job, &mut [0; N]);
    assert_eq!(native.finish(), reads);
    assert_eq!(captured.definitions.outcome, Outcome::Rows { len: 1 });
    assert_eq!(captured.resources.outcome, Outcome::Rows { len: 2 });
    assert_eq!(captured.exchanges.outcome, Outcome::Rows { len: 1 });
    assert_eq!(captured.exchanges.rows[0].id, 0x1234_5678_0000_0033);
}

#[test]
fn every_hour_publishes_the_census_with_its_hourly_consumers() {
    let source = resource_fixture::Fixture::new();
    let memory = engine_native_memory::memory::CountedMemory::new(ResourceTrace {
        source: &source,
        reads: Default::default(),
        fault: None,
    });
    let mut captured = Captured::with_capacities(&e_capacities());
    let context = captured.resource_copy_context(0, 0x1000, 0, false);
    assert!(context.resources_active);
    let mut job = resource_job();
    // SAFETY: Synthetic unit-test memory with no live market hooks.
    let report = unsafe { copy_economy_rest_refresh(&memory, context, &mut job) };
    assert!(report.raw_ok);
    assert!(matches!(
        job.resources.outcomes[0],
        Some(Outcome::Rows { .. })
    ));
    assert!(matches!(
        job.resources.outcomes[1],
        Some(Outcome::Rows { .. })
    ));
    assert!(matches!(
        job.resources.outcomes[2],
        Some(Outcome::Rows { .. })
    ));
    let mut ticks = [0; N];
    captured.decode_economy_rest_payload(0, &job, &mut ticks);
    assert!(matches!(captured.definitions.outcome, Outcome::Rows { .. }));
    assert!(matches!(captured.resources.outcome, Outcome::Rows { .. }));
    assert!(matches!(captured.exchanges.outcome, Outcome::Rows { .. }));

    let mut direct = Captured::with_capacities(&e_capacities());
    direct.capture(&memory, 0, 0x1000, 0, false);
    assert_eq!(direct.definitions.outcome, captured.definitions.outcome);
    assert_eq!(direct.resources.outcome, captured.resources.outcome);
    assert_eq!(direct.exchanges.outcome, captured.exchanges.outcome);
}

#[test]
fn owned_resource_value_faults_recover_once_and_remain_local() {
    let reads_at = |memory: &ResourceTrace<'_>| {
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&at| at == 0x10010)
            .count()
    };
    let source = resource_fixture::Fixture::new();
    let memory = ResourceTrace {
        source: &source,
        reads: Default::default(),
        fault: Some(0x10010),
    };
    let mut captured = Captured::with_capacities(&e_capacities());
    let ctx = captured.resource_copy_context(0, 0x1000, 10, false);
    let mut job = resource_job();
    // SAFETY: Synthetic unit-test memory with no live market hooks.
    assert!(unsafe { copy_economy_rest_refresh(&memory, ctx, &mut job) }.raw_ok);
    captured.decode_economy_rest_payload(0, &job, &mut [0; N]);
    assert!(matches!(captured.resources.outcome, Outcome::Rows { .. }));
    assert_eq!(captured.definitions.outcome, Outcome::Rows { len: 1 });
    assert_eq!(captured.exchanges.outcome, Outcome::Rows { len: 1 });
    assert_eq!(reads_at(&memory), 2);

    let mut source = resource_fixture::Fixture::new();
    source.bytes.remove(&0x10010);
    let memory = ResourceTrace {
        source: &source,
        reads: Default::default(),
        fault: None,
    };
    let mut captured = Captured::with_capacities(&e_capacities());
    let ctx = captured.resource_copy_context(0, 0x1000, 10, false);
    let mut job = resource_job();
    // SAFETY: Synthetic unit-test memory with no live market hooks.
    assert!(unsafe { copy_economy_rest_refresh(&memory, ctx, &mut job) }.raw_ok);
    captured.decode_economy_rest_payload(0, &job, &mut [0; N]);
    assert!(matches!(
        captured.resources.outcome,
        Outcome::Unavailable { .. }
    ));
    assert_eq!(captured.definitions.outcome, Outcome::Rows { len: 1 });
    assert_eq!(captured.exchanges.outcome, Outcome::Rows { len: 1 });
    assert_eq!(reads_at(&memory), 2);
    let before = memory.reads.borrow().len();
    captured.decode_economy_rest_payload(0, &job, &mut [0; N]);
    assert_eq!(
        memory.reads.borrow().len(),
        before,
        "decode does not read native memory"
    );
}

#[test]
fn owned_resource_shared_fault_is_terminal() {
    let source = resource_fixture::Fixture::new();
    let mut captured = Captured::with_capacities(&e_capacities());
    let ctx = captured.resource_copy_context(0, 0x1000, 0, true);
    let memory = engine_native_memory::memory::CountedMemory::new(ResourceTrace {
        source: &source,
        reads: Default::default(),
        fault: Some(0x1000 + cgame_state::COUNTRIES),
    });
    let mut job = resource_job();
    let native = read_accounting::Capture::new();
    // SAFETY: Synthetic unit-test memory with no live market hooks.
    assert!(unsafe { copy_economy_rest_refresh(&memory, ctx, &mut job) }.raw_ok);
    let mut expected_counts = read_accounting::ZERO;
    expected_counts[TABLES[2].index()] = engine_native_memory::memory::ReadCounts {
        copy_calls: 2,
        spans: 2,
        bytes: 24,
        ..engine_native_memory::memory::ReadCounts::ZERO
    };
    assert_eq!(read_accounting::snapshot(), expected_counts);
    assert_eq!(
        memory.0.reads.borrow().as_slice(),
        &[
            0x1000 + cgame_state::SUPPLY_SYSTEM,
            0x1000 + cgame_state::COUNTRIES,
        ]
    );
    assert!(
        job.resources
            .outcomes
            .iter()
            .all(|o| matches!(o, Some(Outcome::Unavailable { .. })))
    );
    let mut decoded = Captured::with_capacities(&e_capacities());
    decoded.decode_economy_rest_payload(0, &job, &mut [0; N]);
    assert_eq!(native.finish(), expected_counts);
    assert!(
        decoded.outcomes()[6..9]
            .iter()
            .all(|o| matches!(o, Outcome::Unavailable { .. }))
    );
}

#[test]
fn worker_supply_growth_stages_and_decodes_mod_sized_rows_in_the_same_hour() {
    use engine_native::ccountry_supply_system::world as csupply_system;
    const GAME: usize = 0x1000;
    const ROOT: usize = 0x2000;
    const SUPPLY: usize = 0x3000;
    const COUNT: usize = 20_152;
    let mut source = resource_fixture::Fixture::default();
    source.u64(GAME + cgame_state::SUPPLY_SYSTEM, ROOT as u64);
    source.u64(ROOT, csupply_system::VTABLE_RVA.0 as u64);
    source.u64(ROOT + 8, csupply_system::SECONDARY_VTABLE_RVA.0 as u64);
    source.array(GAME + cgame_state::PROVINCES, 0x4000, COUNT as u32);
    source.array(ROOT + csupply_system::PROVINCE_DATA, SUPPLY, COUNT as u32);
    source.put(SUPPLY, &vec![0; COUNT * sprovince_supply_data::STRIDE]);
    source.put(
        SUPPLY + (COUNT - 1) * sprovince_supply_data::STRIDE + sprovince_supply_data::FLAGS,
        &[0xff],
    );
    let memory = ResourceTrace {
        source: &source,
        reads: Default::default(),
        fault: None,
    };
    let mut job = EconomyRestJob {
        log: RawLog::new(1, 1),
        ..Default::default()
    };
    let context = EconomyRestCopyContext {
        base: 0,
        game_state: GAME,
        resource_rows: [0; 3],
        resource_scratch: 0,
        resources_active: false,
        other_active: false,
    };
    // SAFETY: Synthetic unit-test memory with no live market hooks.
    assert!(unsafe { copy_economy_rest_refresh(&memory, context, &mut job) }.raw_ok);
    assert!(!job.log.overflowed);
    let native_reads = memory.reads.borrow().len();
    let mut capacity = e_capacities();
    capacity.rows[2] = 1;
    let mut captured = Captured::with_capacities(&capacity);
    captured.decode_economy_rest_payload(0, &job, &mut [0; N]);
    assert_eq!(captured.supply.outcome, Outcome::Rows { len: COUNT - 1 });
    assert_eq!(captured.supply.rows[COUNT - 2].flags, 0xff);
    assert_eq!(
        memory.reads.borrow().len(),
        native_reads,
        "owned decode never reads live memory"
    );
    assert_eq!(native_reads, 5);
}

#[test]
fn failed_daily_country_header_keeps_hourly_supply_and_first_fault() {
    use engine_native::ccountry_supply_system::world as csupply_system;

    const GAME: usize = 0x1000;
    const ROOT: usize = 0x20000;
    const SUPPLY: usize = 0x21000;
    let mut source = resource_fixture::Fixture::new();
    source.u64(GAME + cgame_state::SUPPLY_SYSTEM, ROOT as u64);
    source.u64(ROOT, csupply_system::VTABLE_RVA.0 as u64);
    source.u64(ROOT + 8, csupply_system::SECONDARY_VTABLE_RVA.0 as u64);
    source.array(GAME + cgame_state::PROVINCES, 0x22000, 3);
    source.array(ROOT + csupply_system::PROVINCE_DATA, SUPPLY, 3);
    source.array(ROOT + csupply_system::COUNTRY_SYSTEM_DATA, 0, 0);
    source.array(ROOT + csupply_system::COUNTRY_COMPANION_DATA, 0, 0);
    source.put(SUPPLY, &[0; 3 * sprovince_supply_data::STRIDE]);
    source.u64(SUPPLY + sprovince_supply_data::STRIDE, 5);
    source.u64(SUPPLY + 2 * sprovince_supply_data::STRIDE, 9);
    let mut expected = vec![sprovince_supply_data::Row::default(); 2];
    assert_eq!(
        sprovince_supply_data::walk(&source, 0, GAME, &mut expected),
        Outcome::Rows { len: 2 }
    );

    for (hour, force_full) in [(17, false), (0, true)] {
        let memory = engine_native_memory::memory::CountedMemory::new(ResourceTrace {
            source: &source,
            reads: Default::default(),
            fault: Some(GAME + cgame_state::COUNTRIES),
        });
        let mut captured = Captured::with_capacities(&e_capacities());
        let context = captured.resource_copy_context(0, GAME, hour, force_full);
        let mut job = resource_job();
        let native = read_accounting::Capture::new();
        // SAFETY: Synthetic unit-test memory with no live market hooks.
    let report = unsafe { copy_economy_rest_refresh(&memory, context, &mut job) };
        assert!(report.raw_ok);
        assert!(job.staging_failure.is_none());
        assert!(job.log.fault[0]);
        assert!(!job.log.fault[2]);
        let mut expected_counts = read_accounting::ZERO;
        expected_counts[TableId::ProvinceSupply.index()] = engine_native_memory::memory::ReadCounts {
            copy_calls: 8,
            spans: 8,
            bytes: 104 + 3 * sprovince_supply_data::STRIDE as u64,
            ..engine_native_memory::memory::ReadCounts::ZERO
        };
        assert_eq!(read_accounting::snapshot(), expected_counts);
        captured.decode_economy_rest_payload(0, &job, &mut [0; N]);
        assert_eq!(native.finish(), expected_counts);
        assert_eq!(captured.supply.outcome, Outcome::Rows { len: 2 });
        assert_eq!(&captured.supply.rows[..2], &expected);
        for index in [0, 1, 3, 4, 5] {
            assert_eq!(
                captured.outcomes()[index],
                Outcome::Unavailable {
                    reason: "country array unreadable or invalid",
                }
            );
        }
        assert!(matches!(
            captured.purchases.outcome,
            Outcome::Unavailable { reason } if reason != replay_data::cadence::SKIPPED_REASON
        ));
        for outcome in &captured.outcomes()[6..9] {
            assert!(matches!(
                outcome,
                Outcome::Unavailable { reason } if *reason != replay_data::cadence::SKIPPED_REASON
            ));
        }
        let reads = memory.0.reads.borrow();
        assert_eq!(reads.len(), 8);
        for address in [
            GAME + cgame_state::COUNTRIES,
            GAME + cgame_state::SUPPLY_SYSTEM,
            SUPPLY,
        ] {
            assert_eq!(reads.iter().filter(|&&read| read == address).count(), 1);
        }
    }
}

use replay_data::rows::TableSchema;

fn small() -> (Capacities, Captured) {
    let capacities = Capacities {
        rows: [2; N],
        foreign_sources: 4,
        demand: Default::default(),
    };
    let captured = Captured::with_capacities(&capacities);
    (capacities, captured)
}

fn complete<S: TableSchema>(view: &TableView) -> &[S::Row] {
    let TableView::Complete(rows) = view else {
        panic!("expected complete table")
    };
    rows.rows::<S>().unwrap()
}

#[test]
fn projection_matches_full_conversion_after_reordering_changes_and_faults() {
    use crate::projection::ProjectionScratch;
    use replay_frame::{frame::FrameEncoder, frame::FrameInput, frame::Metrics};

    fn with_award_refusal(mut inputs: [TableInput; N]) -> [TableInput; N] {
        let fuel = match &mut inputs[0] {
            TableInput::Sampled(TableView::Complete(rows::TableRows::CountryFuel(fuel)))
            | TableInput::Sampled(TableView::Truncated {
                rows: rows::TableRows::CountryFuel(fuel),
                ..
            }) => fuel,
            _ => return inputs,
        };
        for row in fuel {
            let rows::Recorded::Value(Some(country_instance)) = &row.country_instance else {
                panic!("projection fixture must supply the country instance")
            };
            row.award_day = rows::Recorded::Value(rows::FuelAwardDay {
                country_instance: *country_instance,
                reason: b"fuel awards not provided by projection fixture".to_vec(),
                ..Default::default()
            });
        }
        inputs
    }

    let (_, mut captured) = small();
    macro_rules! sampled {
        ($field:ident, $row:expr) => {
            captured.$field.outcome = Outcome::Rows { len: 1 };
            captured.$field.rows[0] = $row;
        };
    }
    sampled!(
        fuel,
        ccountry_fuel_status::Row {
            country: u32::MAX,
            country_instance: Some(u32::MAX - 1),
            current_raw: i64::MIN,
            ..Default::default()
        }
    );
    sampled!(
        experience,
        ccountry_experience_status::Row {
            country: u32::MAX,
            armies_for_training_raw: i64::MAX,
            ..Default::default()
        }
    );
    sampled!(
        supply,
        sprovince_supply_data::Row {
            province: u32::MAX,
            flags: u8::MAX,
            ..Default::default()
        }
    );
    sampled!(
        hubs,
        ccountry_supply_system::Row {
            owner: u32::MAX,
            actor: u32::MAX - 1,
            province: i32::MIN,
            node_type: -1,
            map_entry_index: 19,
            ..Default::default()
        }
    );
    captured.hubs.outcome = Outcome::Rows { len: 2 };
    captured.hubs.rows[1] = ccountry_supply_system::Row {
        province: i32::MAX,
        ..captured.hubs.rows[0]
    };
    sampled!(
        disrupted,
        disrupted_supply_bucket::Row {
            country: u32::MAX,
            bucket_index: u32::MAX - 3,
            province: u32::MAX - 1,
            node_type: u32::MAX - 2,
            value: i64::MIN,
        }
    );
    sampled!(
        foreign,
        foreign_homebase_node::Row {
            country: u32::MAX,
            ordinal: u32::MAX - 1,
            source_start: 1,
            source_len: 3,
            ..Default::default()
        }
    );
    captured.sources.first.copy_from_slice(&[0, 0xff, 0, b'a']);
    captured.sources.len = 4;
    captured.sources.required = 4;
    let mut definition = cstrategic_resource::Row::default();
    definition.id = u32::MAX;
    definition.factory_cost = i64::MAX;
    sampled!(definitions, definition);
    sampled!(
        resources,
        ccountry_resources::Row {
            country: u32::MAX,
            resource: u32::MAX - 1,
            exported: i64::MIN,
            ..Default::default()
        }
    );
    sampled!(
        exchanges,
        cresource_exchange::Row {
            id: u64::MAX,
            delivered: i64::MAX,
            ..Default::default()
        }
    );
    sampled!(
        purchases,
        cpurchase_contract::Row {
            id: u64::MAX,
            completed_deliveries: i32::MAX,
            ..Default::default()
        }
    );
    sampled!(
        fuel_consumers,
        ccountry_fuel_status::ConsumerRow {
            country: u32::MAX,
            index: 2,
            received_raw: i64::MIN,
            ..Default::default()
        }
    );

    let mut baseline = FrameInput {
        branch: 1,
        hour: 100,
        tables: std::array::from_fn(|_| unavailable("outside economy fixture").into()),
        metrics: Metrics::default(),
    };
    let original_inputs = with_award_refusal(captured.table_inputs(None));
    for (table, input) in TABLES.into_iter().zip(original_inputs.iter().cloned()) {
        baseline.tables[table.index()] = input;
    }
    let mut encoder = FrameEncoder::new();
    encoder.encode_input(baseline, false).unwrap();
    let scratch = ProjectionScratch::default();
    let mut projection = Projection::new(&encoder, 1, 101, false, &scratch);
    let assert_unchanged = |inputs: [TableInput; N]| {
        for index in [2, 6, 7, 8] {
            assert!(matches!(inputs[index], TableInput::ComparedEqual(_)));
        }
        for index in [0, 1, 3, 4, 5, 9, 10] {
            assert_eq!(inputs[index], original_inputs[index], "table {index}");
        }
    };
    assert_unchanged(with_award_refusal(
        captured.table_inputs(Some(&mut projection)),
    ));
    captured.hubs.rows.swap(0, 1);
    assert_unchanged(with_award_refusal(
        captured.table_inputs(Some(&mut projection)),
    ));

    captured.supply.rows[0].flags -= 1;
    captured.definitions.rows[0].factory_cost -= 1;
    captured.resources.rows[0].exported += 1;
    captured.exchanges.rows[0].delivered -= 1;
    let changed = with_award_refusal(captured.table_inputs(Some(&mut projection)));
    let converted = with_award_refusal(captured.table_inputs(None));
    for index in [2, 6, 7, 8] {
        match &changed[index] {
            TableInput::ComparedChanges { indices, rows, .. } => {
                assert_eq!(indices, &[0]);
                assert_eq!(
                    TableInput::Sampled(TableView::Complete(rows.clone())),
                    converted[index]
                );
            }
            _ => assert_eq!(changed[index], converted[index]),
        }
        assert_ne!(changed[index], original_inputs[index]);
    }

    let original = captured.foreign.rows[0];
    for field in 0..13 {
        captured.foreign.rows[0] = original;
        captured.sources.first[2] = 0;
        let row = &mut captured.foreign.rows[0];
        match field {
            0 => row.country -= 1,
            1 => row.ordinal -= 1,
            2 => row.province = 1,
            3 => row.supply = i64::MIN,
            4 => row.start = i64::MAX,
            5 => row.penalty = -1,
            6 => row.added_penalty = 1,
            7 => row.duration = 2,
            8 => row.hours = 3,
            9 => row.decay = 4,
            10 => row.base = u8::MAX,
            11 => row.source_len -= 1,
            _ => captured.sources.first[2] = b'b',
        }
        let input = captured.foreign_table(Some(&mut projection), captured.foreign.outcome);
        assert_eq!(
            input,
            captured.foreign_table(None, captured.foreign.outcome)
        );
        assert_ne!(input, original_inputs[5], "field {field}");
        assert!(matches!(input, TableInput::Sampled(_)), "field {field}");
    }
    captured.foreign.rows[0] = original;
    captured.sources.first[2] = 0;
    captured.sources.first.copy_within(1..4, 0);
    captured.foreign.rows[0].source_start = 0;
    assert_eq!(
        captured.foreign_table(Some(&mut projection), captured.foreign.outcome),
        original_inputs[5]
    );
    captured.foreign.rows[0].source_start = usize::MAX;
    assert!(matches!(
        captured.foreign_table(Some(&mut projection), captured.foreign.outcome),
        TableInput::Sampled(TableView::Unavailable { .. })
    ));
    captured.fuel.outcome = Outcome::Unavailable {
        reason: "fuel source unreadable",
    };
    captured.hubs.outcome = Outcome::Truncated {
        len: 2,
        unread_rows: 1,
    };
    let faulted = with_award_refusal(captured.table_inputs(Some(&mut projection)));
    let converted = with_award_refusal(captured.table_inputs(None));
    for index in [0, 3, 5] {
        assert_eq!(faulted[index], converted[index]);
    }
    assert!(matches!(
        faulted[0],
        TableInput::Sampled(TableView::Unavailable { .. })
    ));
    assert!(matches!(
        faulted[3],
        TableInput::Sampled(TableView::Truncated { missing: 1, .. })
    ));
}

#[test]
fn conversion_preserves_raw_quantities_full_hub_identity_and_literal_source_bytes() {
    let (_, mut captured) = small();
    captured.fuel.outcome = Outcome::Rows { len: 1 };
    captured.fuel.rows[0] = ccountry_fuel_status::Row {
        country: u32::MAX,
        country_instance: Some(u32::MAX - 1),
        current_raw: i64::MIN,
        maximum_raw: i64::MAX,
        lend_lease_gain_raw: 4_294_967_317,
        ring_gain_raw: i64::MIN,
        ring_consumed_raw: -1,
        ring_other_raw: i64::MAX,
        ring_received_raw: [-5, 6, i64::MIN + 1],
    };
    captured.fuel_consumers.outcome = Outcome::Rows { len: 1 };
    captured.fuel_consumers.rows[0] = ccountry_fuel_status::ConsumerRow {
        country: u32::MAX,
        index: u32::MAX,
        priority: i32::MIN,
        requested_raw: i64::MAX,
        received_raw: i64::MIN,
    };
    captured.hubs.outcome = Outcome::Rows { len: 1 };
    captured.hubs.rows[0] = ccountry_supply_system::Row {
        owner: u32::MAX,
        actor: u32::MAX - 1,
        province: i32::MIN,
        node_type: -1,
        level: u8::MAX,
        node_index: 37,
        map_entry_index: 91,
    };
    captured.foreign.outcome = Outcome::Rows { len: 1 };
    captured.foreign.rows[0] = foreign_homebase_node::Row {
        country: 7,
        ordinal: 19,
        province: 500,
        supply: i64::MAX,
        added_penalty: i64::MIN,
        source_start: 1,
        source_len: 3,
        ..Default::default()
    };
    captured.sources.first.copy_from_slice(&[99, 0xff, 0, b'a']);
    captured.sources.len = 4;
    captured.sources.required = 4;
    let views = captured.tables();
    let fuel = &complete::<tables::CountryFuel>(&views[0])[0];
    assert_eq!(
        fuel.country_instance,
        replay_data::rows::Recorded::Value(Some(u32::MAX - 1))
    );
    assert_eq!(
        (fuel.current_raw, fuel.maximum_raw, fuel.lend_lease_gain_raw),
        (i64::MIN, i64::MAX, 4_294_967_317)
    );
    let retired = replay_data::rows::RetiredI64::default();
    assert_eq!(
        [
            fuel.gain_raw,
            fuel.cost_raw,
            fuel.gain_from_states_raw,
            fuel.lend_lease_consumption_raw,
            fuel.gain_per_oil_raw,
            fuel.ring_requested_0_raw,
            fuel.ring_requested_1_raw,
            fuel.ring_requested_2_raw,
        ],
        [retired; 8]
    );
    assert_eq!(
        (fuel.history_index, fuel.remaining_hours),
        (
            replay_data::rows::RetiredI32::default(),
            replay_data::rows::RetiredI32::default()
        )
    );
    let consumer = &complete::<tables::CountryFuelConsumers>(&views[10])[0];
    assert_eq!(consumer.key(), u128::MAX >> 64);
    assert_eq!(
        (
            consumer.priority,
            consumer.requested_raw,
            consumer.received_raw
        ),
        (i32::MIN, i64::MAX, i64::MIN)
    );
    assert_eq!(
        [
            fuel.ring_gain_raw,
            fuel.ring_consumed_raw,
            fuel.ring_other_raw,
            fuel.ring_received_0_raw,
            fuel.ring_received_1_raw,
            fuel.ring_received_2_raw,
        ],
        [i64::MIN, -1, i64::MAX, -5, 6, i64::MIN + 1]
    );
    let hub = &complete::<tables::HubMotorization>(&views[3])[0];
    let expected_key = (u128::from(u32::MAX) << 96)
        | (u128::from(i32::MIN as u32) << 64)
        | (u128::from(u32::MAX) << 32)
        | u128::from(u32::MAX - 1);
    assert_eq!(hub.key(), expected_key);
    assert_eq!(
        (hub.node_index, hub.map_entry_index, hub.level),
        (37, 91, 255)
    );
    let foreign = &complete::<tables::ForeignHomebases>(&views[5])[0];
    assert_eq!(foreign.source, [0xff, 0, b'a']);
    assert_eq!(
        (foreign.supply, foreign.added_penalty),
        (i64::MAX, i64::MIN)
    );
}

#[test]
fn invalid_or_overflowing_source_ranges_and_false_counts_refuse_without_panicking() {
    let (_, mut captured) = small();
    captured.foreign.outcome = Outcome::Rows { len: 1 };
    captured.sources.len = 2;
    captured.sources.required = 2;
    for (start, len) in [(1, 2), (usize::MAX, 1), (4, 0)] {
        captured.foreign.rows[0].source_start = start;
        captured.foreign.rows[0].source_len = len;
        assert!(matches!(
            captured.foreign_table(None, captured.foreign.outcome),
            TableInput::Sampled(TableView::Unavailable { .. })
        ));
    }
    assert!(matches!(
        bounded(Outcome::Rows { len: 3 }, 2),
        Outcome::Unavailable { .. }
    ));
    assert!(matches!(
        bounded(
            Outcome::Truncated {
                len: 1,
                unread_rows: 0
            },
            2
        ),
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn growth_keeps_primary_membership_scratch_and_source_capacity_in_step() {
    let (mut capacities, mut captured) = small();
    captured.purchases.outcome = Outcome::Truncated {
        len: 2,
        unread_rows: 63,
    };
    captured.exchanges.outcome = Outcome::Truncated {
        len: 2,
        unread_rows: 9,
    };
    captured.sources.required = 37;
    captured.grow(&mut capacities);
    assert!(capacities.rows[9] >= 65 && capacities.rows[8] >= 11);
    assert_eq!(
        captured.purchases.rows.len(),
        captured.purchase_scratch.len()
    );
    assert_eq!(
        captured.exchanges.rows.len(),
        captured.exchange_scratch.len()
    );
    assert!(captured.sources.first.len() >= 37);
    let before = captured.allocated_bytes();
    captured.reset();
    captured.grow(&mut capacities);
    assert_eq!(captured.allocated_bytes(), before);
}


const E_BASE: usize = 0;
const E_GAME: usize = 0x1000;
const E_COUNTRIES: usize = 0x1500;
const E_LINKS: usize = 0x1600;
const E_C1: usize = 0x2000;
const E_C2: usize = 0x4000;

fn e_put(bytes: &mut [u8], address: usize, value: &[u8]) {
    bytes[address - E_GAME..address - E_GAME + value.len()].copy_from_slice(value);
}

fn e_array(bytes: &mut [u8], header: usize, data: usize, count: u32) {
    e_put(&mut *bytes, header, &(data as u64).to_le_bytes());
    e_put(&mut *bytes, header + 8, &count.to_le_bytes());
    e_put(&mut *bytes, header + 12, &count.to_le_bytes());
}

fn e_census(bytes: &mut [u8]) {
    use engine_native::{ccountry, cgame_state};
    e_array(&mut *bytes, E_GAME + cgame_state::COUNTRIES, E_COUNTRIES, 3);
    e_array(&mut *bytes, E_GAME + cgame_state::COUNTRY_LINKS, E_LINKS, 8);
    e_put(&mut *bytes, E_COUNTRIES + 8, &(E_C1 as u64).to_le_bytes());
    e_put(&mut *bytes, E_COUNTRIES + 16, &(E_C2 as u64).to_le_bytes());
    for (slot, id) in [(1_u32, 1_u32), (2, 2)] {
        e_put(
            &mut *bytes,
            E_LINKS + slot as usize * 4,
            &slot.to_le_bytes(),
        );
        e_put(&mut *bytes, E_LINKS + id as usize * 4, &slot.to_le_bytes());
    }
    for (country, id) in [(E_C1, 1_u32), (E_C2, 2)] {
        e_put(
            &mut *bytes,
            country,
            &(engine_native::ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        e_put(&mut *bytes, country + ccountry::ID, &id.to_le_bytes());
        let _ = cgame_state::COUNTRIES;
    }
}

fn e_capacities() -> Capacities {
    Capacities {
        rows: [4, 4, 8, 4, 4, 4, 4, 8, 4, 4, 8, 8],
        foreign_sources: 64,
        demand: Default::default(),
    }
}

fn e_job() -> EconomyRestJob {
    EconomyRestJob {
        log: RawLog::new(8_192, 1 << 20),
        resources: OwnedResources::new([4, 8, 4], 4),
        ..Default::default()
    }
}

fn e_direct(memory: &engine_native_memory::memory::SliceMemory<'_>, cap: &Capacities) -> Captured {
    let mut captured = Captured::with_capacities(cap);
    captured.capture(memory, E_BASE, E_GAME, 0, true);
    captured
}

fn e_split(
    memory: &engine_native_memory::memory::SliceMemory<'_>,
    cap: &Capacities,
) -> (Captured, EconomyRestCopyReport) {
    let mut captured = Captured::with_capacities(cap);
    let ctx = captured.resource_copy_context(E_BASE, E_GAME, 0, true);
    let mut first = e_job();
    // SAFETY: Synthetic unit-test memory with no live market hooks.
    let report = unsafe { crate::economy::copy_economy_rest_refresh(memory, ctx, &mut first) };
    assert!(report.raw_ok);
    captured.rest_raw_mode = report.raw_ok;
    let mut ticks = [0u64; N];
    captured.decode_economy_rest_payload(E_BASE, &first, &mut ticks);
    (captured, report)
}

fn e_assert_table<const R: usize>(split: &Captured, direct: &Captured, index: usize) {
    assert_eq!(
        split.outcomes()[index],
        direct.outcomes()[index],
        "outcome index {index}"
    );
    let _ = R;
}

fn e_fuel_world() -> Vec<u8> {
    use engine_fuel::ccountry_fuel_status;
    use engine_native::ccountry;
    let mut bytes = vec![0; 0x8000];
    e_census(&mut bytes);
    e_fuel_arrays(&mut bytes);
    for (country, fuel) in [(E_C1, 0x6000_usize), (E_C2, 0x6200)] {
        e_put(
            &mut bytes,
            country + ccountry::FUEL_STATUS,
            &(fuel as u64).to_le_bytes(),
        );
        e_put(
            &mut bytes,
            fuel,
            &(ccountry_fuel_status::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        e_put(
            &mut bytes,
            fuel + ccountry_fuel_status::COUNTRY,
            &(country as u64).to_le_bytes(),
        );
        e_put(
            &mut bytes,
            fuel + ccountry_fuel_status::CURRENT,
            &123_i64.to_le_bytes(),
        );
        e_put(
            &mut bytes,
            fuel + ccountry_fuel_status::MAXIMUM,
            &456_i64.to_le_bytes(),
        );
        e_put(
            &mut bytes,
            fuel + ccountry_fuel_status::GAIN,
            &7_i64.to_le_bytes(),
        );
        e_put(
            &mut bytes,
            fuel + ccountry_fuel_status::COST,
            &9_i64.to_le_bytes(),
        );
    }
    bytes
}

fn e_assert_fuel_matches_direct(bytes: &[u8]) {
    let memory = engine_native_memory::memory::SliceMemory {
        base: E_GAME,
        bytes,
    };
    let cap = e_capacities();
    let direct = e_direct(&memory, &cap);
    let (split, _) = e_split(&memory, &cap);
    assert_eq!(split.fuel.outcome, Outcome::Rows { len: 2 });
    assert_eq!(split.fuel.outcome, direct.fuel.outcome);
    assert_eq!(
        split.fuel.rows[..split.fuel.outcome.len()],
        direct.fuel.rows[..direct.fuel.outcome.len()]
    );
    e_assert_table::<0>(&split, &direct, 0);
}

#[test]
fn economy_fuel_copy_matches_direct_walk() {
    e_assert_fuel_matches_direct(&e_fuel_world());
}

#[test]
fn economy_copy_skips_positive_aliases_of_the_reserved_country() {
    use engine_native::{ccountry, cgame_state};
    const RESERVED: usize = 0x1800;
    let mut bytes = e_fuel_world();
    e_array(&mut bytes, E_GAME + cgame_state::COUNTRIES, E_COUNTRIES, 4);
    for slot in [0, 3] {
        e_put(
            &mut bytes,
            E_COUNTRIES + slot * 8,
            &(RESERVED as u64).to_le_bytes(),
        );
    }
    e_put(
        &mut bytes,
        RESERVED,
        &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    e_assert_fuel_matches_direct(&bytes);
}

fn e_fuel_arrays(bytes: &mut [u8]) {
    use engine_fuel::ccountry_fuel_status as fuel;
    use engine_native::ccountry;
    for (index, (country, status)) in [(E_C1, 0x6000_usize), (E_C2, 0x6200)]
        .into_iter()
        .enumerate()
    {
        let entries = 0x6400 + index * 0x80;
        let ring = 0x7000 + index * 0x800;
        e_put(
            &mut *bytes,
            country + ccountry::FUEL_STATUS,
            &(status as u64).to_le_bytes(),
        );
        e_put(
            &mut *bytes,
            status,
            &(fuel::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        e_put(
            &mut *bytes,
            status + fuel::COUNTRY,
            &(country as u64).to_le_bytes(),
        );
        e_put(
            &mut *bytes,
            status + fuel::GAIN_PER_OIL,
            &(360_000 + index as i64).to_le_bytes(),
        );
        e_put(
            &mut *bytes,
            status + fuel::HISTORY_INDEX,
            &13_i32.to_le_bytes(),
        );
        e_array(&mut *bytes, status + fuel::CONSUMERS, entries, 3);
        e_array(&mut *bytes, status + fuel::HISTORY_RING, ring, 24);
        for offset in (0..3 * fuel_consumer_data::STRIDE).step_by(8) {
            e_put(
                &mut *bytes,
                entries + offset,
                &(offset as i64 - 7).to_le_bytes(),
            );
        }
        for offset in (0..24 * fuel_history_entry::STRIDE).step_by(8) {
            e_put(
                &mut *bytes,
                ring + offset,
                &(offset as i64 * 3 - 11).to_le_bytes(),
            );
        }
        e_put(
            &mut *bytes,
            status + fuel::PENDING_HISTORY,
            &(-(index as i64) - 1).to_le_bytes(),
        );
    }
}

#[test]
fn economy_fuel_arrays_copy_matches_direct_walk_with_independent_failures() {
    use engine_native_memory::memory::SliceMemory;
    let mut bytes = vec![0; 0x9000];
    e_census(&mut bytes);
    e_fuel_arrays(&mut bytes);
    for (case, header) in [
        ("complete", None),
        (
            "unreadable ring",
            Some(0x6200 + ccountry_fuel_status::HISTORY_RING),
        ),
        (
            "unreadable consumer entries",
            Some(0x6000 + ccountry_fuel_status::CONSUMERS),
        ),
    ] {
        let mut bytes = bytes.clone();
        if let Some(header) = header {
            e_put(&mut bytes, header, &0x10_0000_u64.to_le_bytes());
        }
        let memory = SliceMemory {
            base: E_GAME,
            bytes: &bytes,
        };
        for fuel_rows in [0, 1, 4] {
            let mut cap = e_capacities();
            cap.rows[0] = fuel_rows;
            let direct = e_direct(&memory, &cap);
            let (split, _) = e_split(&memory, &cap);
            for index in [0, 10] {
                e_assert_table::<0>(&split, &direct, index);
            }
            assert_eq!(
                split.fuel.rows[..split.fuel.outcome.len()],
                direct.fuel.rows[..direct.fuel.outcome.len()]
            );
            assert_eq!(
                split.fuel_consumers.rows[..split.fuel_consumers.outcome.len()],
                direct.fuel_consumers.rows[..direct.fuel_consumers.outcome.len()]
            );
            let outcomes = direct.outcomes();
            match case {
                "unreadable ring" => assert_eq!(
                    outcomes[0],
                    Outcome::Unavailable {
                        reason: "fuel history ring unreadable"
                    }
                ),
                _ => {
                    assert_eq!(outcomes[0], Outcome::Rows { len: 2 }, "{case}");
                    let sum = |field: usize| {
                        (0..24)
                            .map(|slot| (slot * fuel_history_entry::STRIDE + field) as i64 * 3 - 11)
                            .sum::<i64>()
                    };
                    let row = &direct.fuel.rows[1];
                    assert_eq!(row.ring_gain_raw, sum(fuel_history_entry::GAIN));
                    assert_eq!(row.ring_other_raw, sum(fuel_history_entry::OTHER));
                    assert_eq!(
                        row.ring_received_raw[2],
                        sum(fuel_history_entry::RECEIVED + 16)
                    );
                }
            }
            if case == "unreadable consumer entries" {
                assert!(matches!(
                    outcomes[10],
                    Outcome::Unavailable {
                        reason: "fuel consumer entries unreadable"
                    }
                ));
            } else {
                assert_eq!(outcomes[10], Outcome::Rows { len: 6 }, "{case}");
            }
        }
    }
}

#[test]
fn economy_experience_copy_matches_direct_walk() {
    use engine_events::ccountry_experience_status;
    use engine_native::ccountry;
    use engine_native_memory::memory::SliceMemory;
    let mut bytes = vec![0; 0x8000];
    e_census(&mut bytes);
    for (country, status) in [(E_C1, 0x6000_usize), (E_C2, 0x6200)] {
        e_put(
            &mut bytes,
            country + ccountry::EXPERIENCE_STATUS,
            &(status as u64).to_le_bytes(),
        );
        e_put(
            &mut bytes,
            status,
            &(ccountry_experience_status::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        e_put(
            &mut bytes,
            status + ccountry_experience_status::COUNTRY,
            &(country as u64).to_le_bytes(),
        );
        e_put(
            &mut bytes,
            status + ccountry_experience_status::ARMY,
            &11_i64.to_le_bytes(),
        );
    }
    let memory = SliceMemory {
        base: E_GAME,
        bytes: &bytes,
    };
    let cap = e_capacities();
    let direct = e_direct(&memory, &cap);
    let (split, _) = e_split(&memory, &cap);
    assert_eq!(split.experience.outcome, direct.experience.outcome);
    e_assert_table::<1>(&split, &direct, 1);
}

#[test]
fn economy_supply_is_hourly_with_daily_gates_and_identical_truncation() {
    use engine_native::{cgame_state, ccountry_supply_system::world as csupply_system};
    use engine_native_memory::memory::SliceMemory;
    use engine_railways::sprovince_supply_data;
    let mut bytes = vec![0; 0x20000];
    e_census(&mut bytes);
    const ROOT: usize = 0x8000;
    const PROVINCES: usize = 0x9000;
    const SUPPLY: usize = 0xA000;
    e_put(
        &mut bytes,
        E_GAME + cgame_state::SUPPLY_SYSTEM,
        &(ROOT as u64).to_le_bytes(),
    );
    e_put(
        &mut bytes,
        ROOT,
        &(csupply_system::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    e_put(
        &mut bytes,
        ROOT + 8,
        &(csupply_system::SECONDARY_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    e_array(&mut bytes, E_GAME + cgame_state::PROVINCES, PROVINCES, 3);
    e_array(&mut bytes, ROOT + csupply_system::PROVINCE_DATA, SUPPLY, 3);
    e_put(
        &mut bytes,
        SUPPLY + sprovince_supply_data::STRIDE,
        &5_i64.to_le_bytes(),
    );
    let memory = engine_native_memory::memory::CountedMemory::new(SliceMemory {
        base: E_GAME,
        bytes: &bytes,
    });
    let mut job = e_job();
    let skipped = Outcome::Unavailable {
        reason: replay_data::cadence::SKIPPED_REASON,
    };
    for supply_capacity in [0, 1, 8] {
        let mut cap = e_capacities();
        cap.rows[2] = supply_capacity;
        let mut direct = Captured::with_capacities(&cap);
        let mut split = Captured::with_capacities(&cap);
        for (hour, force_full) in (-24_i32..24).map(|hour| (hour, false)).chain([(0, true)]) {
            let native = read_accounting::Capture::new();
            let typed_ticks = direct.capture(&memory, E_BASE, E_GAME, hour, force_full);
            let typed_counts = native.finish();
            let context = split.resource_copy_context(E_BASE, E_GAME, hour, force_full);
            let native = read_accounting::Capture::new();
            // SAFETY: Synthetic unit-test memory with no live market hooks.
    let report = unsafe { copy_economy_rest_refresh(&memory, context, &mut job) };
            assert!(report.raw_ok);
            let raw_counts = read_accounting::snapshot();
            split.set_schedule(0, true);
            let mut decode_ticks = [0; N];
            split.decode_economy_rest_payload(E_BASE, &job, &mut decode_ticks);
            assert_eq!(read_accounting::snapshot(), raw_counts);
            assert_eq!(split.supply.outcome, direct.supply.outcome);
            assert_eq!(split.supply.outcome.len(), 2, "capacity {supply_capacity}");
            assert_eq!(
                split.supply.rows[..split.supply.outcome.len()],
                direct.supply.rows[..direct.supply.outcome.len()],
            );
            assert!(typed_counts[TABLES[2].index()].spans > 0);
            assert!(raw_counts[TABLES[2].index()].spans > 0);
            for (active, indices) in [
                (true, &[6, 7, 8][..]),
                (
                    force_full || hour.rem_euclid(24) == 17,
                    &[0, 1, 3, 4, 5, 9, 10][..],
                ),
            ] {
                for &index in indices {
                    if active {
                        assert_ne!(direct.outcomes()[index], skipped);
                        assert_ne!(split.outcomes()[index], skipped);
                    } else {
                        assert_eq!(direct.outcomes()[index], skipped);
                        assert_eq!(split.outcomes()[index], skipped);
                        assert_eq!(typed_ticks[index], 0);
                        assert_eq!(decode_ticks[index], 0);
                        assert_eq!(report.copy_ticks[index], 0);
                        assert_eq!(report.raw_bytes[index], 0);
                        assert_eq!(
                            typed_counts[TABLES[index].index()],
                            engine_native_memory::memory::ReadCounts::ZERO
                        );
                        assert_eq!(
                            raw_counts[TABLES[index].index()],
                            engine_native_memory::memory::ReadCounts::ZERO
                        );
                    }
                }
            }
            job.staging_failure = Some("accepted acquisition failed");
            split.set_schedule(0, true);
            let mut failure_ticks = [0; N];
            split.decode_economy_rest_payload(E_BASE, &job, &mut failure_ticks);
            assert_eq!(failure_ticks, [0; N]);
            for (index, outcome) in split.outcomes().into_iter().enumerate() {
                let active = index == 2
                    || (6..9).contains(&index)
                    || force_full
                    || hour.rem_euclid(24) == 17;
                assert_eq!(
                    outcome,
                    if active {
                        Outcome::Unavailable {
                            reason: "accepted acquisition failed",
                        }
                    } else {
                        skipped
                    }
                );
            }
            assert_eq!(native.finish(), raw_counts);
        }
    }
}

#[test]
fn economy_census_tables_copy_matches_direct_walk() {
    use engine_native_memory::memory::SliceMemory;
    let mut bytes = vec![0; 0x8000];
    e_census(&mut bytes);
    let memory = SliceMemory {
        base: E_GAME,
        bytes: &bytes,
    };
    let cap = e_capacities();
    let direct = e_direct(&memory, &cap);
    let (split, _) = e_split(&memory, &cap);
    assert_eq!(split.hubs.outcome, direct.hubs.outcome);
    assert_eq!(split.disrupted.outcome, direct.disrupted.outcome);
    assert_eq!(split.foreign.outcome, direct.foreign.outcome);
    assert_eq!(split.definitions.outcome, direct.definitions.outcome);
    assert_eq!(split.resources.outcome, direct.resources.outcome);
    assert_eq!(split.exchanges.outcome, direct.exchanges.outcome);
    assert_eq!(split.purchases.outcome, direct.purchases.outcome);
    e_assert_table::<3>(&split, &direct, 3);
    e_assert_table::<4>(&split, &direct, 4);
    e_assert_table::<5>(&split, &direct, 5);
    e_assert_table::<9>(&split, &direct, 9);
}

#[test]
fn economy_truncation_matrix_matches_direct_with_same_caps() {
    use engine_fuel::ccountry_fuel_status;
    use engine_native::ccountry;
    use engine_native_memory::memory::SliceMemory;
    let mut bytes = vec![0; 0x8000];
    e_census(&mut bytes);
    e_fuel_arrays(&mut bytes);
    for (country, fuel) in [(E_C1, 0x6000_usize), (E_C2, 0x6200)] {
        e_put(
            &mut bytes,
            country + ccountry::FUEL_STATUS,
            &(fuel as u64).to_le_bytes(),
        );
        e_put(
            &mut bytes,
            fuel,
            &(ccountry_fuel_status::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        e_put(
            &mut bytes,
            fuel + ccountry_fuel_status::COUNTRY,
            &(country as u64).to_le_bytes(),
        );
    }
    let memory = SliceMemory {
        base: E_GAME,
        bytes: &bytes,
    };
    for rows in [0_usize, 1, 2, 4] {
        let cap = Capacities {
            rows: [rows, 4, 8, 4, 4, 4, 4, 8, 4, 4, 8, 8],
            foreign_sources: 64,
            demand: Default::default(),
        };
        let direct = e_direct(&memory, &cap);
        let (split, _) = e_split(&memory, &cap);
        assert_eq!(split.fuel.outcome, direct.fuel.outcome, "rows={rows}");
    }
}

#[test]
fn economy_gate_skips_typed_walks_when_raw_mode_set() {
    use engine_native_memory::memory::SliceMemory;
    use std::cell::RefCell;
    struct Traced<'a> {
        source: SliceMemory<'a>,
        reads: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, output.len()));
            self.source.copy(address, output)
        }
    }
    let bytes = vec![0; 0x8000];
    let traced = || Traced {
        source: SliceMemory {
            base: E_GAME,
            bytes: &bytes,
        },
        reads: RefCell::new(Vec::new()),
    };
    let memory = traced();
    let reference = traced();
    let cap = e_capacities();
    let mut captured = Captured::with_capacities(&cap);
    let mut demand = crate::fuel_demand::Captured::with_capacities(&cap.demand);
    let pending = captured.outcomes();
    captured.rest_raw_mode = true;
    let ticks = captured.capture(&memory, E_BASE, E_GAME, 0, true);
    demand.capture(&reference, E_BASE, E_GAME, 0);
    assert_eq!(*memory.reads.borrow(), *reference.reads.borrow());
    assert!(!memory.reads.borrow().is_empty());
    assert_eq!(captured.outcomes(), pending);
    assert_eq!(ticks[1..], [0; N - 1]);
}
