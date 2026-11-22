use super::*;
use engine_land_inputs::{active_ability_record as active, cability};
use engine_native::{ccountry, cgame_state, cpolitical_status, crelation, cunit_leader};
use engine_production::nindustrial_organisation_corganisation as mio;
use std::{hint::black_box, time::Instant};
const HEAP: usize = 0x350_0000;
const STATE: usize = 0x10000;
struct World {
    bytes: Vec<u8>,
    base: usize,
    countries: usize,
    receivers: Vec<usize>,
}
impl World {
    fn new(countries: usize) -> Self {
        let slots = countries + 1;
        let mut bytes = vec![0u8; HEAP + slots * 0x3000 + slots * 12];
        let base = bytes.as_ptr() as usize;
        let table = HEAP;
        let links = table + slots * 8;
        let objects = links + slots * 4;
        let put = |bytes: &mut [u8], at: usize, value: &[u8]| {
            bytes[at..at + value.len()].copy_from_slice(value)
        };
        put(
            &mut bytes,
            cgame_state::SINGLETON_RVA.0,
            &((base + STATE) as u64).to_le_bytes(),
        );
        for (header, data) in [
            (cgame_state::COUNTRIES, table),
            (cgame_state::COUNTRY_LINKS, links),
        ] {
            put(
                &mut bytes,
                STATE + header,
                &((base + data) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                STATE + header + 8,
                &(slots as u32).to_le_bytes(),
            );
            put(
                &mut bytes,
                STATE + header + 12,
                &(slots as u32).to_le_bytes(),
            );
        }
        let mut receivers = Vec::new();
        for country in 1..slots {
            let pointer = objects + country * 0x3000;
            let politics = pointer + 0x2000;
            put(
                &mut bytes,
                table + country * 8,
                &((base + pointer) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                links + country * 4,
                &(country as u32).to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer,
                &((base + ccountry::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + ccountry::ID,
                &(country as u32).to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + ccountry::POLITICAL_STATUS,
                &((base + politics) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                politics,
                &((base + cpolitical_status::VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                politics + cpolitical_status::COUNTRY,
                &((base + pointer) as u64).to_le_bytes(),
            );
            for record in 0..4 {
                let at = pointer + 0x2200 + record * 0x60;
                put(
                    &mut bytes,
                    at + crelation::TYPE_ID,
                    &(record as u32 + 1).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    at + crelation::FIRST_COUNTRY,
                    &(country as u32).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    at + crelation::SECOND_COUNTRY,
                    &((country % countries + 1) as u32).to_le_bytes(),
                );
                receivers.push(base + at);
            }
            for record in 0..2 {
                let at = pointer + 0x2400 + record * 0x80;
                put(
                    &mut bytes,
                    at,
                    &((base + mio::VTABLE_RVA.0) as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    at + mio::REFERENCE,
                    &(((country as u64 * 2 + record as u64) << 32) | mio::REFERENCE_TYPE_ID as u64)
                        .to_le_bytes(),
                );
                put(
                    &mut bytes,
                    at + mio::TOKEN,
                    &(record as u32 + 10).to_le_bytes(),
                );
                receivers.push(base + at);
            }
            for record in 0..2 {
                let definition = pointer + 0x2600 + record * 0x100;
                let leader = pointer + 0x2800 + record * 0x40;
                let at = pointer + 0x2a00 + record * 0x40;
                put(
                    &mut bytes,
                    definition,
                    &((base + cability::VTABLE_RVA.0) as u64).to_le_bytes(),
                );
                put(&mut bytes, definition + cability::TOKEN, b"bench_item");
                put(
                    &mut bytes,
                    definition + cability::TOKEN_LENGTH,
                    &10u64.to_le_bytes(),
                );
                put(
                    &mut bytes,
                    definition + cability::TOKEN_CAPACITY,
                    &15u64.to_le_bytes(),
                );
                put(
                    &mut bytes,
                    leader,
                    &((base + cunit_leader::VTABLE_RVA.0) as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    leader + cunit_leader::TYPE,
                    &cunit_leader::REFERENCE_TYPE.to_le_bytes(),
                );
                put(
                    &mut bytes,
                    leader + cunit_leader::ID,
                    &(country as u32 * 2 + record as u32).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    at + active::DEFINITION,
                    &((base + definition) as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    at + active::CASTER,
                    &((base + leader) as u64).to_le_bytes(),
                );
                receivers.push(base + at);
            }
        }
        Self {
            bytes,
            base,
            countries,
            receivers,
        }
    }
    fn events(&self) -> Vec<Event> {
        let mut events = Vec::new();
        for country in 1..=self.countries {
            let common = Event {
                country: country as u32,
                instance: 0,
                hour: 0,
                ..Default::default()
            };
            events.push(Event {
                kind: Kind::Cost,
                cost: 1,
                ..common
            });
            for component in 0..6 {
                events.push(Event {
                    kind: Kind::Cost,
                    cost: 1,
                    component: if component < 4 {
                        PowerDailyCostKind::Relation
                    } else if component == 4 {
                        PowerDailyCostKind::MioResearch
                    } else {
                        PowerDailyCostKind::MioProduction
                    },
                    ..common
                });
            }
            events.push(Event {
                kind: Kind::Cost,
                cost: 1,
                component: PowerDailyCostKind::Blockade,
                ..common
            });
            events.push(Event {
                kind: Kind::Daily,
                gross: 10,
                cost: 8,
                net: 2,
                ..common
            });
            events.push(Event {
                before: 0,
                after: 2,
                requested: Some(2),
                cause: PowerCause::DailyGain,
                ..common
            });
            events.push(Event {
                currency: PowerCurrency::Command,
                before: 0,
                after: 2,
                requested: Some(2),
                cause: PowerCause::DailyGain,
                ..common
            });
            for ability in 6..8 {
                events.push(Event {
                    currency: PowerCurrency::Command,
                    before: (8 - ability) as i64,
                    after: (7 - ability) as i64,
                    requested: Some(-1),
                    cause: PowerCause::AbilityUpkeep,
                    ..common
                });
            }
        }
        events
    }
}
fn measure(values: &mut [std::time::Duration]) -> (u128, u128) {
    values.sort();
    (
        values[values.len() / 2].as_nanos(),
        values[(values.len() * 95 / 100).min(values.len() - 1)].as_nanos(),
    )
}

#[test]
#[ignore = "release production daily tap observer profile; no game"]
fn politics_daily_tap_profile() {
    let _guard = hooks::staging_test();
    for countries in [1500, 5000] {
        let mut world = World::new(countries);
        let define = engine_events::politics_sites::BLOCKADE_COST_DEFINE_RVA.0;
        world.bytes[define..define + 8].copy_from_slice(&1i64.to_le_bytes());
        let mut modifier = [0u8; 0x400];
        let mut rate = [0u8; 0x400];
        for (frame, cost) in [(&mut modifier, 1i64), (&mut rate, 8i64)] {
            frame[8..16].copy_from_slice(&cost.to_le_bytes());
            frame[0x398..0x3a0].copy_from_slice(&((world.base + 0xbfb952) as u64).to_le_bytes());
        }
        let modifier_frame = modifier.as_ptr() as usize + 0x40;
        let rate_frame = rate.as_ptr() as usize + 0x40;
        let mut output = vec![Event::default(); countries * 3];
        let mut bytes = vec![0u8; countries * 3];
        let mut times = Vec::new();
        for iteration in 0..110 {
            // SAFETY: Benchmark holds `hooks::staging_test()` across setup and drain.
            unsafe { hooks::bench_begin(world.base, countries * 3, 1) };
            let start = Instant::now();
            for receivers in world.receivers.chunks_exact(8) {
                hooks::bench_daily_taps(receivers[0] - 0x200, modifier_frame, rate_frame);
            }
            if iteration >= 10 {
                times.push(start.elapsed());
            }
            // SAFETY: Benchmark holds `hooks::staging_test()` as the sole consumer.
            let drain = unsafe { hooks::bench_drain(&mut output, &mut bytes) };
            assert_eq!(drain.copied, countries * 3);
            for rows in output.chunks_exact(3) {
                assert_eq!(
                    rows[0].component,
                    PowerDailyCostKind::PoliticalPowerCostModifier
                );
                assert_eq!(rows[0].cost, 1);
                assert_eq!((rows[1].gross, rows[1].cost, rows[1].net), (10, 8, 2));
                assert_eq!(rows[2].component, PowerDailyCostKind::Blockade);
                assert_eq!(rows[2].cost, 1);
            }
        }
        println!(
            "daily_taps countries={countries} production_tap_bodies_p50_p95_ns={:?} observations_per_country=3; patched_tap_stub_excluded",
            measure(&mut times)
        );
        hooks::bench_end();
    }
}
#[test]
#[ignore = "release offline politics detour/staging and worker benchmark"]
fn politics_daily_scale() {
    let _guard = hooks::staging_test();
    for countries in [1500, 5000] {
        let world = World::new(countries);
        let events = world.events();
        let core: Vec<_> = (1..=countries)
            .map(|n| engine_native::ccountry::Row {
                id: n as u32,
                political_power_raw: Some(2),
                command_power_raw: 0,
                politics_witness: Some((0, None)),
                ..Default::default()
            })
            .collect();
        let baseline: Vec<_> = core
            .iter()
            .map(|r| engine_native::ccountry::Row {
                political_power_raw: Some(0),
                ..*r
            })
            .collect();
        let mut capture = Captured::with_capacities(&Capacities {
            countries,
            entries: events.len(),
            bytes: events.len() * 32,
        });
        // SAFETY: Benchmark holds `hooks::staging_test()` across setup, capture, and assembly.
        unsafe { hooks::bench_begin(world.base, events.len(), 32) };
        let mut observation = Vec::new();
        let mut handoff = Vec::new();
        let mut worker = Vec::new();
        let mut total = Vec::new();
        for iteration in 0..100 {
            let mut ledger = Ledger::default();
            let mut tables: Vec<_> = (0..replay_data::rows::TABLE_COUNT)
                .map(|_| TableInput::Sampled(unavailable("fixture")))
                .collect();
            // SAFETY: benchmark holds the politics lock as the sole capture and spool caller.
            unsafe {
                capture.capture();
                ledger.assemble_owned(
                    1,
                    -1,
                    &capture,
                    &baseline,
                    Outcome::Rows { len: countries },
                    &mut tables,
                )
            }
            .unwrap();
            let all = Instant::now();
            let start = Instant::now();
            for (owner, receivers) in world.receivers.chunks_exact(8).enumerate() {
                hooks::bench_daily_country(owner as u32 + 1, receivers);
            }
            let observed = start.elapsed();
            let start = Instant::now();
            // SAFETY: benchmark holds the politics lock as the sole capture caller.
            black_box(unsafe { capture.capture() });
            let handed = start.elapsed();
            let start = Instant::now();
            // SAFETY: benchmark holds the politics lock as the sole spool consumer.
            unsafe {
                ledger.assemble_owned(
                    1,
                    0,
                    &capture,
                    &core,
                    Outcome::Rows { len: countries },
                    &mut tables,
                )
            }
            .unwrap();

            let Some(TableRows::CountryPowerChanges(changes)) =
                rows(&tables[TableId::CountryPowerChanges.index()])
            else {
                panic!()
            };
            assert_eq!(changes.len(), countries * 4);
            let Some(TableRows::PoliticsCaptureCoverage(coverage)) =
                rows(&tables[TableId::PoliticsCaptureCoverage.index()])
            else {
                panic!()
            };
            assert!(coverage.iter().all(|r| r.streams == 15));
            let frame = replay_frame::frame::FrameEncoder::new()
                .encode_input(
                    replay_frame::frame::FrameInput {
                        branch: 1,
                        hour: 0,
                        tables: tables.try_into().unwrap(),
                        metrics: Default::default(),
                    },
                    true,
                )
                .unwrap();
            let mut encoded = Vec::new();
            frame.encode(&mut encoded).unwrap();
            black_box(encoded);
            let assembled = start.elapsed();
            let elapsed = all.elapsed();
            if iteration >= 10 {
                observation.push(observed);
                handoff.push(handed);
                worker.push(assembled);
                total.push(elapsed);
            }
        }
        println!(
            "politics countries={countries} events={} game_midnight_median_p95_ns={:?} game_handoff_median_p95_ns={:?} worker_median_p95_ns={:?} total_median_p95_ns={:?}",
            events.len(),
            measure(&mut observation),
            measure(&mut handoff),
            measure(&mut worker),
            measure(&mut total)
        );
        hooks::bench_end();
        black_box(world.bytes);
    }
}

#[test]
#[ignore = "release staging component profile"]
fn politics_staging_profile() {
    let _guard = hooks::staging_test();
    for countries in [1500, 5000] {
        let world = World::new(countries);
        // SAFETY: Benchmark holds `hooks::staging_test()`.
        unsafe { hooks::bench_begin(world.base, countries * 13, 32) };
        let mut identities = Vec::new();
        for _ in 0..100 {
            let start = Instant::now();
            for batch in world.receivers.chunks_exact(8) {
                for (n, receiver) in batch.iter().enumerate() {
                    black_box(if n < 6 {
                        hooks::bench_identity(
                            if n < 4 {
                                PowerDailyCostKind::Relation
                            } else {
                                PowerDailyCostKind::MioResearch
                            },
                            *receiver,
                        )
                    } else {
                        hooks::bench_upkeep_identity(*receiver)
                    });
                }
            }
            identities.push(start.elapsed());
        }
        println!(
            "identity_part countries={countries} median_p95_ns={:?}",
            measure(&mut identities)
        );
        // SAFETY: Benchmark holds `hooks::staging_test()`.
        unsafe { hooks::bench_staging_parts(&world.events()) };
        hooks::bench_end();
    }
}

#[test]
#[ignore = "release populated per-event midnight profile"]
fn politics_midnight_profile() {
    let _guard = hooks::staging_test();
    for countries in [1500, 5000] {
        let world = World::new(countries);
        // SAFETY: Benchmark holds `hooks::staging_test()`.
        unsafe {
            hooks::bench_begin(world.base, countries * 13, 32);
            hooks::bench_midnight_parts(&world.receivers);
        }
        hooks::bench_end();
    }
}

#[test]
fn native_operand_rows_are_byte_identical_to_guarded_rows() {
    let world = World::new(2);
    let mut original = world.events();
    let mut fast = original.clone();
    for (n, receivers) in world.receivers.chunks_exact(8).enumerate() {
        for (kind, pointer) in receivers.iter().enumerate() {
            let (a, b) = hooks::fixture_identities(*pointer, kind);
            assert_eq!(a, b);
            let index = n * 13 + if kind < 6 { kind + 1 } else { kind + 5 };
            original[index].identity = a;
            fast[index].identity = b;
        }
    }
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let (_, a, akeys) = hooks::stage_fixture(&original, &memory);
    let (_, b, bkeys) = hooks::stage_fixture(&fast, &unsafe {
        engine_events::politics_sites::NativeOperands::new()
    });
    assert_eq!(a, b);
    assert_eq!(akeys, bkeys);
    super::tests::compact_handoff_rows_are_byte_identical_to_original_event_stream();
}
