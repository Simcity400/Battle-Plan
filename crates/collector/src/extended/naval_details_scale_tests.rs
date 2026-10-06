use super::*;
use engine::Memory;
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
fn naval_tree_stack_headroom_refusal_is_counted_in_status() {
    use engine::naval_ship_modifier_breakdown::STACK_HEADROOM;
    let mut work = BreakdownWork::default();
    work.observe(BreakdownStatus {
        outcome: Outcome::Unavailable {
            reason: STACK_HEADROOM,
        },
        required: [0; 5],
        capacity: [0; 5],
    })
    .unwrap();
    let status = serde_json::to_value(work.status()).unwrap();
    assert_eq!(status["hours"], 1);
    assert_eq!(status["available_hours"], 0);
    assert_eq!(status["refused_hours"], 1);
    assert_eq!(status["refusals"], serde_json::json!({STACK_HEADROOM: 1}));
}

#[test]
fn naval_empty_hour_keeps_all_tables_complete_without_tree_storage() {
    let memory = Fixture::new(0);
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.capture(&memory, IMAGE, STATE);
    assert_eq!(captured.outcomes(), [Outcome::Rows { len: 0 }; N]);
    assert_eq!(captured.breakdown_requests.len, 0);
    assert_eq!(captured.breakdown_nodes.len, 0);
    assert_eq!(
        captured.breakdown_requests.allocated_bytes()
            + captured.breakdown_roots.allocated_bytes()
            + captured.breakdown_nodes.allocated_bytes()
            + captured.breakdown_text.allocated_bytes()
            + captured.breakdown_depth.allocated_bytes(),
        0
    );
    assert!(
        captured
            .tables()
            .iter()
            .all(|view| matches!(view, TableView::Complete(_)))
    );
}

#[test]
fn naval_shared_country_roots_walk_once_across_combat_side_and_ship_uses() {
    struct Counted {
        memory: Fixture,
        visits: std::cell::Cell<[u32; COUNTRY_COUNT]>,
    }
    impl Memory for Counted {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            for slot in 0..COUNTRY_COUNT {
                let root =
                    COUNTRY_BASE + slot * COUNTRY_STRIDE + engine::ccountry::MODIFIER_ACCUMULATOR;
                if address == root + 0x18 && destination.len() == 16 {
                    let mut visits = self.visits.get();
                    visits[slot] += 1;
                    self.visits.set(visits);
                }
            }
            self.memory.copy(address, destination)
        }
    }
    let memory = Counted {
        memory: Fixture::new(20),
        visits: std::cell::Cell::new([0; COUNTRY_COUNT]),
    };
    let mut captured = Captured::with_capacities(&Capacities {
        breakdown_requests: 20 * 128,
        breakdown_roots: 20 * 80 + 2100,
        breakdown_nodes: 20 * 80 + 2100,
        breakdown_text: 32_000,
        breakdown_depth: 128,
        ..Capacities::default()
    });
    captured.capture(&memory, IMAGE, STATE);
    assert!(
        matches!(captured.breakdown_outcome, Outcome::Rows { .. }),
        "{:?}",
        captured.breakdown_outcome
    );
    assert_eq!(memory.visits.get(), [1; COUNTRY_COUNT]);
    let rows = captured.project_breakdowns().unwrap();
    let country = COUNTRY_BASE + engine::ccountry::MODIFIER_ACCUMULATOR;
    let shared = rows
        .iter()
        .find(|row| row.accumulator == country as u64 && row.key == 0x1b2)
        .unwrap();
    assert_eq!(shared.uses.len(), 4);
    assert_eq!(shared.nodes.len(), TREE_CHILDREN + 1);
}

#[test]
fn naval_tree_arena_shortage_withholds_only_trees_and_grows_after_capture() {
    let memory = Fixture::new(1);
    for request_capacity in [0, 8] {
        let mut capacities = Capacities {
            breakdown_requests: request_capacity,
            ..Capacities::default()
        };
        let mut captured = Captured::with_capacities(&capacities);
        captured.capture(&memory, IMAGE, STATE);
        assert_eq!(captured.details.outcome, Outcome::Rows { len: 1 });
        assert_eq!(
            captured.ships.outcome,
            Outcome::Rows {
                len: SHIPS_PER_COMBAT
            }
        );
        assert_eq!(
            captured.breakdown_outcome,
            Outcome::Unavailable {
                reason: engine::naval_ship_modifier_breakdown::REQUEST_CAPACITY,
            }
        );
        assert!(matches!(
            captured.breakdown_table(),
            TableView::Unavailable { .. }
        ));
        assert_eq!(captured.breakdown_requests.first.len(), request_capacity);
        assert!(captured.breakdown_requests.required > 1);
        let first_demand = captured.breakdown_status().required;
        let demand = first_demand[0];
        captured.grow(&mut capacities);
        assert!(capacities.breakdown_requests >= demand);
        assert_eq!(
            captured.breakdown_requests.first.len(),
            capacities.breakdown_requests
        );
        captured.reset();
        captured.capture(&memory, IMAGE, STATE);
        assert_eq!(captured.details.outcome, Outcome::Rows { len: 1 });
        assert!(
            matches!(captured.breakdown_table(), TableView::Complete(_)),
            "tree arenas did not grow to the live fixture demand: {:?}",
            captured.breakdown_outcome
        );
        assert_eq!(captured.breakdown_status().required, first_demand);
    }
}

#[test]
fn naval_tree_live_demand_survives_production_loan_restore_and_growth() {
    use engine::naval_ship_modifier_breakdown::REQUEST_CAPACITY;
    let memory = live_shape();
    let mut capacities = crate::extended::Capacities::default();
    let mut captured = crate::extended::Captured::with_capacities(&capacities);
    let countries =
        std::sync::Arc::new(engine::country_memory::Cache::try_with_capacity(0).unwrap());
    let mut work = BreakdownWork::default();
    let mut first_demand = [0; 5];
    for hour in 0..2 {
        captured.reset();
        let mut request = captured.take_ordinary_request(&countries, IMAGE, STATE);
        request.packs.naval_details.capture(&memory, IMAGE, STATE);
        captured.restore_ordinary_request(request, None);
        let naval = &captured.naval_details;
        assert_eq!(naval.details.outcome, Outcome::Rows { len: 3 });
        assert_eq!(naval.ships.outcome, Outcome::Rows { len: 90 });
        assert_eq!(naval.dense_bases.outcome, Outcome::Rows { len: 12 });
        assert_eq!(naval.dense_wings.len, 12);
        assert!(
            naval.planes.first[..naval.planes.len]
                .iter()
                .all(|plane| plane.efficiency.source == 3
                    && plane.efficiency.pool_type_flags & 0x400 != 0)
        );
        let status = naval.breakdown_status();
        work.observe(status).unwrap();
        if hour == 0 {
            assert_eq!(
                status.outcome,
                Outcome::Unavailable {
                    reason: REQUEST_CAPACITY
                }
            );
            assert_eq!(status.capacity, [0; 5]);
            assert_eq!(status.required, [161, 40, 2008, 17_952, 4]);
            first_demand = status.required;
            assert!(matches!(
                naval.breakdown_table(),
                TableView::Unavailable { .. }
            ));
            eprintln!("naval live-shape first-hour demand: {:?}", first_demand);
            // Production spool-side growth, after owned projection/status and
            // before recycling the frame. No second refusal is permitted.
            captured.grow(&mut capacities);
            continue;
        }
        assert_eq!(status.required, first_demand);
        for (capacity, first) in status.capacity.into_iter().zip(first_demand) {
            assert!(
                capacity >= first,
                "growth missed census {capacity} < {first}"
            );
        }
        let TableView::Complete(rows::TableRows::NavalModifierBreakdowns(trees)) =
            naval.breakdown_table()
        else {
            panic!("second-hour table must be complete: {:?}", status.outcome);
        };
        let used: std::collections::BTreeSet<_> = trees
            .iter()
            .filter(|tree| !tree.uses.is_empty())
            .map(|tree| tree.key)
            .collect();
        for key in engine::naval_ship_modifier_breakdown::KEYS {
            assert!(used.contains(&key), "missing hover key {key:#x}");
        }
        assert!(trees.iter().any(|tree| tree.nodes.len() > 200));
        let json = serde_json::to_value(work.status()).unwrap();
        assert_eq!(json["hours"], 2);
        assert_eq!(json["available_hours"], 1);
        assert_eq!(json["refused_hours"], 1);
        assert_eq!(json["refusals"], serde_json::json!({REQUEST_CAPACITY: 1}));
        eprintln!("naval live-shape complete on hour 2: {json}");
    }
}

#[test]
fn naval_tree_later_demand_rise_grows_all_arenas_after_a_full_capture() {
    use engine::naval_ship_modifier_breakdown::{KEYS, REQUEST_CAPACITY};
    let mut low = live_shape();
    // Keep the existing three-combat storage, but expose only the first combat.
    low.0.u32(STATE + engine::cgame_state::COMBATS + 8 + 12, 1);
    let mut high = live_shape();
    let mut parent = COUNTRY_BASE + engine::ccountry::MODIFIER_ACCUMULATOR;
    for _ in 0..3 {
        parent = high.0.pointer(high.0.pointer(parent + 0x18)) + 0x10;
    }
    // Retain four more levels under a synthetic four-level country tree. The
    // rise therefore exceeds depth as well as requests, roots, nodes and text.
    for _ in 0..4 {
        let children = high.0.allocate(8 + 0xc0 + KEYS.len() * 16);
        let child = children + 8;
        high.0.array(parent + 0x18, children, 1);
        high.0.u64(children, (child - 0x10) as u64);
        high.0.array(child, child + 0xc0, KEYS.len());
        high.0.inline(child + 0x48, b"late doctrine");
        for (index, key) in KEYS.into_iter().enumerate() {
            high.0.u32(child + 0xc0 + index * 16, key);
            high.0.u64(child + 0xc0 + index * 16 + 8, 2_000);
        }
        parent = child;
    }
    let mut capacities = crate::extended::Capacities::default();
    let mut captured = crate::extended::Captured::with_capacities(&capacities);
    let countries =
        std::sync::Arc::new(engine::country_memory::Cache::try_with_capacity(0).unwrap());
    let capture_hour = |captured: &mut crate::extended::Captured, memory: &LiveShape| {
        captured.reset();
        let mut request = captured.take_ordinary_request(&countries, IMAGE, STATE);
        request.packs.naval_details.capture(memory, IMAGE, STATE);
        captured.restore_ordinary_request(request, None);
    };
    // Establish the initial population using production zero capacities and
    // growth, before the five-hour interval whose later refusal is counted.
    capture_hour(&mut captured, &low);
    assert_eq!(
        captured.naval_details.breakdown_outcome,
        Outcome::Unavailable {
            reason: REQUEST_CAPACITY
        }
    );
    captured.grow(&mut capacities);
    let mut work = BreakdownWork::default();
    let mut rise_demand = [0; 5];
    let mut retained_capacity = [0; 5];
    for (hour, memory) in [&low, &high, &high, &low, &high].into_iter().enumerate() {
        capture_hour(&mut captured, memory);
        let naval = &captured.naval_details;
        let status = naval.breakdown_status();
        work.observe(status).unwrap();
        assert_eq!(
            naval.details.outcome,
            Outcome::Rows {
                len: if hour == 0 || hour == 3 { 1 } else { 3 }
            }
        );
        if hour == 1 {
            assert_eq!(
                status.outcome,
                Outcome::Unavailable {
                    reason: REQUEST_CAPACITY
                }
            );
            for (index, (required, capacity)) in
                status.required.into_iter().zip(status.capacity).enumerate()
            {
                assert!(
                    required > capacity,
                    "arena {index} did not exceed earlier full-capture capacity: {required} <= {capacity}"
                );
            }
            assert!(matches!(
                naval.breakdown_table(),
                TableView::Unavailable { .. }
            ));
            rise_demand = status.required;
        } else {
            assert!(
                matches!(
                    naval.breakdown_table(),
                    TableView::Complete(rows::TableRows::NavalModifierBreakdowns(_))
                ),
                "hour N+{hour}: {:?}",
                status.outcome
            );
            if hour == 2 || hour == 4 {
                assert_eq!(status.required, rise_demand);
            }
            if hour >= 3 {
                assert_eq!(status.capacity, retained_capacity);
            }
            if hour == 3 {
                for (fallen, risen) in status.required.into_iter().zip(rise_demand) {
                    assert!(fallen < risen, "demand did not fall: {fallen} >= {risen}");
                }
            }
        }
        // Same spool-side grow-before-recycle ordering as runtime. A previous
        // complete hour must neither latch growth nor clear this hour's demand.
        captured.grow(&mut capacities);
        if hour == 2 {
            retained_capacity = captured.naval_details.breakdown_status().capacity;
        } else if hour >= 3 {
            assert_eq!(
                captured.naval_details.breakdown_status().capacity,
                retained_capacity
            );
        }
    }
    let status = serde_json::to_value(work.status()).unwrap();
    assert_eq!(status["hours"], 5);
    assert_eq!(status["available_hours"], 4);
    assert_eq!(status["refused_hours"], 1);
    assert_eq!(status["refusals"], serde_json::json!({REQUEST_CAPACITY: 1}));
    eprintln!(
        "later naval tree rise demand {rise_demand:?}; retained capacity {retained_capacity:?}"
    );
}

#[test]
fn naval_bench_in_place_changes_keep_headers_and_reach_capture() {
    let mut memory = Fixture::new(1);
    let root = COUNTRY_BASE + engine::ccountry::MODIFIER_ACCUMULATOR;
    let root_header = memory.0[root - STATE..root - STATE + 16].to_vec();
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.capture(&memory, IMAGE, STATE);
    let terms = captured.weapon_terms.first[..captured.weapon_terms.len].to_vec();
    let speed: Vec<_> = captured.ships.rows[..SHIPS_PER_COMBAT]
        .iter()
        .filter(|row| row.member.state == 8)
        .map(|row| row.escape_speed_base)
        .collect();
    assert_eq!(speed.len(), 5);
    memory.advance(1, Hour::Quiet, 7);
    captured.reset();
    captured.capture(&memory, IMAGE, STATE);
    assert_eq!(
        &captured.weapon_terms.first[..captured.weapon_terms.len],
        terms
    );
    memory.advance(1, Hour::AllChanged, 7);
    assert_eq!(&memory.0[root - STATE..root - STATE + 16], root_header);
    captured.reset();
    captured.capture(&memory, IMAGE, STATE);
    assert_eq!(captured.details.outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        captured.ships.outcome,
        Outcome::Rows {
            len: SHIPS_PER_COMBAT
        }
    );
    assert_ne!(
        &captured.weapon_terms.first[..captured.weapon_terms.len],
        terms
    );
    for (row, before) in captured.ships.rows[..SHIPS_PER_COMBAT]
        .iter()
        .filter(|row| row.member.state == 8)
        .zip(speed)
    {
        assert_eq!(row.escape_speed_base, before + 7);
    }
    assert!(
        captured.planes.first[..4]
            .iter()
            .all(|plane| plane.current_planes == 107)
    );
}

/// Pack reset and real collector capture callback, then separate spool projection.
/// Synthetic population: ten countries with 200 children each, eight sorted
/// entries per child and 300 root entries; 30 live ships per combat, including
/// four carrier-linked wing planes and five state-8 ships. Each side has one
/// damage-map node. These are fixture dimensions, not a measured live census.
/// Quiet changes nothing; typical changes two roots and one damage cell per
/// side; all-changed also changes every country child value, task-force fuel,
/// ship stat and plane count. All three run the same production capture path.
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
                    breakdown_requests: combats * 128,
                    breakdown_roots: combats * 80 + 2100,
                    breakdown_nodes: combats * 80 + 2100,
                    breakdown_text: 32_000,
                    breakdown_depth: 128,
                    ..Capacities::default()
                })
            };
            let mut block = Vec::with_capacity(256);
            let mut projection = Vec::with_capacity(256);
            let mut allocated = None;
            for pass in 0..320 {
                memory.advance(combats, hour, pass);
                let began = Instant::now();
                captured.reset();
                black_box(captured.capture(&memory, IMAGE, STATE));
                let elapsed = began.elapsed();
                assert_eq!(captured.details.outcome, Outcome::Rows { len: combats });
                assert_eq!(
                    captured.ships.outcome,
                    Outcome::Rows {
                        len: combats * SHIPS_PER_COMBAT
                    }
                );
                assert_eq!(captured.planes.len, combats * 4);
                assert!(
                    matches!(captured.breakdown_outcome, Outcome::Rows { len }
                    if if combats == 0 { len == 0 } else { len > COUNTRY_COUNT }),
                    "tree capture refused the fixture: {:?}",
                    captured.outcomes()
                );
                if pass == 64 {
                    allocated = Some(captured.allocated_bytes());
                }
                if pass >= 64 {
                    assert_eq!(Some(captured.allocated_bytes()), allocated);
                }
                if pass == 0 {
                    assert!(matches!(captured.breakdown_table(), TableView::Complete(_)));
                }
                let began_projection = Instant::now();
                black_box(captured.tables());
                let projected = began_projection.elapsed();
                if pass >= 64 {
                    block.push(elapsed);
                    projection.push(projected);
                }
            }
            if combats == 0 {
                assert_eq!(captured.allocated_bytes(), 0);
            }
            let (b50, b95, bmax) = percentiles(block);
            let (p50, p95, pmax) = percentiles(projection);
            let ms = |d: Duration| d.as_secs_f64() * 1000.0;
            eprintln!(
                "NAVAL_BENCH combats={combats} hour={} block={:.6}/{:.6}/{:.6} projection={:.6}/{:.6}/{:.6} arena_mib={:.3}",
                hour.name(),
                ms(b50),
                ms(b95),
                ms(bmax),
                ms(p50),
                ms(p95),
                ms(pmax),
                captured.allocated_bytes() as f64 / 1048576.0
            );
        }
    }
}
