use super::*;
use engine_naval_history::naval_combat_demand as breakdown;
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use super::fixtures::*;

fn percentiles(mut values: Vec<Duration>) -> (Duration, Duration, Duration) {
    values.sort_unstable();
    let p50 = values[values.len() / 2];
    let p95 = values[values.len() * 95 / 100];
    (p50, p95, *values.last().unwrap())
}

#[test]
fn naval_empty_hour_keeps_all_tables_complete_without_storage() {
    let memory = Fixture::new(0);
    let mut captured = Captured::with_capacities(&Capacities::default());
    assert_eq!(captured.allocated_bytes(), 0);
    captured.capture(&memory, IMAGE, STATE);
    assert_eq!(captured.outcomes(), [Outcome::Rows { len: 0 }; N]);
    assert!(
        captured
            .tables()
            .iter()
            .all(|view| matches!(view, TableView::Complete(_)))
    );
}

struct Singles<'a> {
    source: &'a Fixture,
    singles: std::cell::RefCell<Vec<(usize, usize)>>,
}

impl Memory for Singles<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.singles.borrow_mut().push((address, destination.len()));
        self.source.copy(address, destination)
    }

    fn copy_scattered<const K: usize>(
        &self,
        addresses: [usize; K],
        destinations: &mut [&mut [u8]; K],
    ) -> usize {
        self.source.copy_scattered(addresses, destinations)
    }

    fn copy_scattered_prefetched<const K: usize>(
        &self,
        addresses: [usize; K],
        destinations: &mut [&mut [u8]; K],
    ) -> usize {
        self.source
            .copy_scattered_prefetched(addresses, destinations)
    }

    fn copy_records(
        &self,
        sources: engine_native::memory::RecordSources<'_>,
        layout: &engine_native::memory::RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.source.copy_records(sources, layout, destination)
    }
}

#[test]
fn combat_ship_fields_are_read_in_one_gather() {
    let fixture = Fixture::new(1);
    let memory = Singles {
        source: &fixture,
        singles: Default::default(),
    };
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.capture(&memory, IMAGE, STATE);
    assert_eq!(
        captured.ships.outcome,
        Outcome::Rows {
            len: SHIPS_PER_COMBAT
        }
    );
    let singles = memory.singles.borrow();
    for ordinal in 0..SHIPS_PER_COMBAT {
        let ship = BODIES + 0xc000 + ordinal * 0xa00;
        for (offset, len) in [(0x270, 8), (0x628, 8), (engine_native::cship::STRENGTH, 8)] {
            assert!(
                !singles.contains(&(ship + offset, len)),
                "ship {ordinal} +{offset:#x} read alone"
            );
        }
    }
}

#[test]
fn cold_defaults_capture_the_first_live_shape_before_publication() {
    let mut capacities = Capacities::default();
    let mut captured = Captured::with_capacities(&capacities);
    assert_eq!(captured.allocated_bytes(), 0);
    let memory = live_shape();
    captured.capture(&memory, IMAGE, STATE);
    assert!(
        captured
            .outcomes()
            .into_iter()
            .all(|outcome| matches!(outcome, Outcome::Rows { .. }))
    );
    assert_eq!(captured.details.outcome, Outcome::Rows { len: 3 });
    assert_eq!(captured.ships.outcome, Outcome::Rows { len: 90 });
    assert_eq!(captured.dense_bases.outcome, Outcome::Rows { len: 12 });
    assert_eq!(captured.dense_wings.len, 12);
    captured.grow(&mut capacities);
    let prepared = Captured::with_capacities(&capacities);
    assert!(prepared.details.rows.len() >= captured.details.rows.len());
    assert!(prepared.ships.rows.len() >= captured.ships.rows.len());
    assert!(prepared.dense_bases.rows.len() >= captured.dense_bases.rows.len());
    assert!(prepared.dense_wings.first.len() >= captured.dense_wings.first.len());
}

#[test]
fn cold_and_later_terrain_names_keep_mod_heap_bytes_in_publication() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    assert_eq!(captured.allocated_bytes(), 0);
    for length in [257, 4097, 5000] {
        let mut memory = Fixture::new(1);
        let name = vec![b'x'; length];
        let name_data = memory.allocate(length + 1);
        memory.put(name_data, &name);
        memory.u64(TERRAIN + 0x18, name_data as u64);
        memory.u64(TERRAIN + 0x28, length as u64);
        memory.u64(TERRAIN + 0x30, length as u64);
        let empty_bucket = memory.allocate(12);
        memory.u64(TERRAIN + 0x1b8, empty_bucket as u64);
        let province_weather = memory.allocate(2 * 0x330);
        memory.array(WEATHER + 0x370, province_weather, 2);
        for country in 0..COUNTRY_COUNT {
            let task = TASK_BASE + country * TASK_STRIDE;
            memory.u64(
                task + engine_native::cunit::LOCATION,
                (BODIES + 0xb00) as u64,
            );
            let diplomacy = memory.allocate(0x400);
            memory.u64(
                diplomacy,
                (IMAGE + engine_native_politics::cdiplomacy_status::VTABLE_RVA.0) as u64,
            );
            let relations = memory.allocate(8);
            memory.array(
                diplomacy + engine_native_politics::cdiplomacy_status::RELATIONS,
                relations,
                1,
            );
            memory.u64(
                COUNTRY_BASE + country * COUNTRY_STRIDE + engine_native::ccountry::DIPLOMACY_STATUS,
                diplomacy as u64,
            );
        }
        assert!(engine_native::msvc_string::read_text::<256>(&memory, TERRAIN + 0x18).is_none());
        captured.reset();
        captured.capture(&memory, IMAGE, STATE);
        assert_eq!(captured.details.outcome, Outcome::Rows { len: 1 });
        assert_eq!(
            captured.ships.outcome,
            Outcome::Rows {
                len: SHIPS_PER_COMBAT
            }
        );
        assert!(
            captured.ships.rows[..SHIPS_PER_COMBAT].iter().all(|row| row
                .weapon_context
                .terrain_name
                .as_bytes()
                == name)
        );
        let TableView::Complete(rows::TableRows::NavalCombatShips(ships)) = &captured.tables()[1]
        else {
            panic!("complete terrain operands were not projected");
        };
        assert!(
            ships
                .iter()
                .all(|row| row.weapon_context.terrain_name == name)
        );
        assert!(captured.allocated_bytes() >= SHIPS_PER_COMBAT * length);
    }
}

#[test]
fn naval_live_headers_presize_attach_and_combat_growth_before_capture() {
    let mut capacities = Capacities::default();
    assert_eq!(capacities.details, 0);
    for combats in [1, 66, 71] {
        let memory = Fixture::new(combats);
        let demand = breakdown::demand(&memory, IMAGE, STATE).unwrap();
        assert_eq!(demand.rows, combats);
        let old = capacities;
        capacities.raise_for_live_details(demand);
        let mut captured = Captured::with_capacities(&old);
        captured.try_grow_hint(&capacities).unwrap();
        captured.capture(&memory, IMAGE, STATE);
        assert_eq!(captured.details.outcome, Outcome::Rows { len: combats });
        assert!(captured.ships.rows.len() <= capacities.ships.max(captured.ships.rows.len()));
        let retained = capacities;
        capacities.raise_for_live_details(Default::default());
        assert!(capacities == retained);
    }
}

#[test]
fn naval_hint_reserve_failure_keeps_existing_lengths() {
    let capacities = Capacities::default();
    let mut captured = Captured::with_capacities(&capacities);
    let mut hint = capacities;
    hint.details += 1;
    hint.dense_wings = usize::MAX;
    assert!(captured.try_grow_hint(&hint).is_err());
    assert_eq!(captured.details.rows.len(), capacities.details);
    assert!(captured.dense_wings.first.is_empty());
    captured.try_grow_hint(&capacities).unwrap();
}

#[test]
#[ignore]
fn naval_window_operand_scale_bench() {
    for combats in [0, 20, 40] {
        for hour in [Hour::Quiet, Hour::Typical, Hour::AllChanged] {
            let mut memory = Fixture::new(combats);
            let mut captured = if combats == 0 {
                Captured::vacant()
            } else {
                Captured::with_capacities(&Capacities {
                    details: combats,
                    ships: combats * SHIPS_PER_COMBAT,
                    planes: combats * 4,
                    side_units: combats * 2,
                    weapon_sources: combats * SHIPS_PER_COMBAT * 12,
                    weapon_terms: 16_384,
                    ..Capacities::default()
                })
            };
            let mut block = Vec::with_capacity(256);
            let mut projection = Vec::with_capacity(256);
            let mut ships = Vec::with_capacity(256);
            let mut allocated = None;
            for pass in 0..320 {
                memory.advance(combats, hour, pass);
                let began = Instant::now();
                captured.reset();
                let ticks = black_box(captured.capture(&memory, IMAGE, STATE));
                let elapsed = began.elapsed();
                let charged = ticks.iter().sum::<u64>().max(1);
                let ship_share = elapsed.mul_f64(ticks[1] as f64 / charged as f64);
                assert_eq!(captured.details.outcome, Outcome::Rows { len: combats });
                assert_eq!(
                    captured.ships.outcome,
                    Outcome::Rows {
                        len: combats * SHIPS_PER_COMBAT
                    }
                );
                assert_eq!(captured.planes.len, combats * 4);
                if pass == 64 {
                    allocated = Some(captured.allocated_bytes());
                }
                if pass >= 64 {
                    assert_eq!(Some(captured.allocated_bytes()), allocated);
                }
                let began_projection = Instant::now();
                black_box(captured.tables());
                let projected = began_projection.elapsed();
                if pass >= 64 {
                    block.push(elapsed);
                    projection.push(projected);
                    ships.push(ship_share);
                }
            }
            if combats == 0 {
                assert_eq!(captured.allocated_bytes(), 0);
            }
            let (b50, b95, bmax) = percentiles(block);
            let (p50, p95, pmax) = percentiles(projection);
            let (s50, _, _) = percentiles(ships);
            let ms = |d: Duration| d.as_secs_f64() * 1000.0;
            eprintln!(
                "NAVAL_BENCH combats={combats} hour={} block={:.6}/{:.6}/{:.6} ships_median={:.6} projection={:.6}/{:.6}/{:.6} arena_mib={:.3}",
                hour.name(),
                ms(b50),
                ms(b95),
                ms(bmax),
                ms(s50),
                ms(p50),
                ms(p95),
                ms(pmax),
                captured.allocated_bytes() as f64 / 1048576.0
            );
        }
    }
}

const DOCTRINE_NODES: usize = 30;

#[test]
fn doctrine_sources_reemitted_equal_the_walks() {
    let capture = |sources: bool| {
        super::SOURCE_MEMO_OFF.set(!sources);
        let memory = Fixture::live_sized(2)
            .with_doctrines(DOCTRINE_NODES)
            .with_locations();
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.capture(&memory, IMAGE, STATE);
        super::SOURCE_MEMO_OFF.set(false);
        assert_eq!(
            captured.ships.outcome,
            Outcome::Rows {
                len: 2 * SHIPS_PER_COMBAT
            }
        );
        (
            captured.weapon_sources.first[..captured.weapon_sources.len].to_vec(),
            captured.weapon_terms.first[..captured.weapon_terms.len].to_vec(),
            captured.ships.rows[..2 * SHIPS_PER_COMBAT]
                .iter()
                .map(|row| row.weapon_context.clone())
                .collect::<Vec<_>>(),
        )
    };
    let walked = capture(false);
    assert!(
        walked.0.iter().filter(|source| source.scope == 4).count()
            >= 2 * SHIPS_PER_COMBAT * DOCTRINE_NODES
    );
    for scope in [5, 6, 7, 8, 20] {
        assert_eq!(
            walked
                .0
                .iter()
                .filter(|source| source.scope == scope)
                .count(),
            2 * SHIPS_PER_COMBAT,
            "scope {scope}"
        );
    }
    assert_eq!(capture(true), walked);
}

#[test]
fn location_terms_are_read_once_per_task_force() {
    let fixture = Fixture::live_sized(2).with_locations();
    let memory = Singles {
        source: &fixture,
        singles: Default::default(),
    };
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.capture(&memory, IMAGE, STATE);
    assert_eq!(
        captured.ships.outcome,
        Outcome::Rows {
            len: 2 * SHIPS_PER_COMBAT
        }
    );
    let controller = (BODIES + 0xb00 + 0x188, 4);
    let reads = memory
        .singles
        .borrow()
        .iter()
        .filter(|&&read| read == controller)
        .count();
    assert!(
        reads < SHIPS_PER_COMBAT,
        "{reads} reads of the location's controller for {} ships",
        2 * SHIPS_PER_COMBAT
    );
    let defines = memory
        .singles
        .borrow()
        .iter()
        .filter(|&&read| read == (IMAGE + 0x033d_fa90, 8))
        .count();
    assert!(
        defines < SHIPS_PER_COMBAT,
        "{defines} reads of the fuel defines"
    );
}

#[test]
#[ignore]
fn naval_live_accumulator_bench() {
    for combats in [20, 40] {
        for (live, memo, ships_live, doctrines, sources) in [
            (true, false, true, 0, true),
            (true, true, true, 0, true),
            (false, true, true, 0, true),
            (false, true, false, 0, true),
            (true, true, true, DOCTRINE_NODES, false),
            (true, true, true, DOCTRINE_NODES, true),
        ] {
            let located = doctrines != 0;
            super::TERM_MEMO_OFF.set(!memo);
            super::SOURCE_MEMO_OFF.set(!sources);
            let mut memory = if live {
                Fixture::live_sized(combats)
            } else {
                Fixture::new(combats)
            };
            if !ships_live {
                memory = memory.without_live_ships(combats);
            }
            if doctrines != 0 {
                memory = memory.with_doctrines(doctrines);
            }
            if located {
                memory = memory.with_locations();
            }
            let mut captured = Captured::with_capacities(&Capacities {
                details: combats,
                ships: combats * SHIPS_PER_COMBAT,
                planes: combats * 4,
                side_units: combats * 2,
                weapon_sources: combats * SHIPS_PER_COMBAT * 12,
                weapon_terms: combats * SHIPS_PER_COMBAT * 256,
                ..Capacities::default()
            });
            let mut block = Vec::with_capacity(256);
            let mut ships = Vec::with_capacity(256);
            for pass in 0..320 {
                memory.advance(combats, Hour::Typical, pass);
                let began = Instant::now();
                captured.reset();
                let ticks = black_box(captured.capture(&memory, IMAGE, STATE));
                let elapsed = began.elapsed();
                assert_eq!(
                    captured.ships.outcome,
                    Outcome::Rows {
                        len: combats * SHIPS_PER_COMBAT
                    }
                );
                let charged = ticks.iter().sum::<u64>().max(1);
                if pass >= 64 {
                    block.push(elapsed);
                    ships.push(elapsed.mul_f64(ticks[1] as f64 / charged as f64));
                }
            }
            super::TERM_MEMO_OFF.set(false);
            super::SOURCE_MEMO_OFF.set(false);
            let mut scopes = std::collections::BTreeMap::<u8, (usize, usize)>::new();
            let mut owned = std::collections::BTreeSet::new();
            for source in &captured.weapon_sources.first[..captured.weapon_sources.len] {
                let entry = scopes.entry(source.scope).or_default();
                entry.0 += 1;
                if source.terms[1] != 0 && owned.insert(source.terms[0]) {
                    entry.1 += source.terms[1];
                }
            }
            eprintln!("NAVAL_LIVE combats={combats} sources and owned terms by scope: {scopes:?}");
            let ms = |d: Duration| d.as_secs_f64() * 1000.0;
            eprintln!(
                "NAVAL_LIVE combats={combats} live_sized={live} memo={memo} live_ships={ships_live} doctrine_nodes={doctrines} source_memo={sources} block={:.3} ships={:.3} terms={}",
                ms(percentiles(block).0),
                ms(percentiles(ships).0),
                captured.weapon_terms.len
            );
        }
    }
}
