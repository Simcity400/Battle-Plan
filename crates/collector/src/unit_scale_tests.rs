#[path = "../../engine/tests/support/unit_world.rs"]
pub(crate) mod fixture;
use crate::{division_census::Census, division_detail::OwnedDetails};
use engine::{carmy, cprovince, memory::ReadCounts, walk::Outcome};
use fixture::Shape;

#[test]
fn stored_air_wing_names_presize_and_handoff_at_5000_countries() {
    use crate::{
        extended::air,
        map_capture::{MapCapacities, MapCaptured},
    };
    let mut world = fixture::build(Shape {
        countries: 5000,
        divisions: 0,
        wings: 5000,
        planes: 5000,
        ships: 0,
    });
    let singleton = engine::cgame_state::SINGLETON_RVA;
    world.bytes[singleton..singleton + 8].copy_from_slice(&(world.state as u64).to_le_bytes());
    let memory = world.live();
    let demand = crate::country_scale::observe_air_wing_names(&memory, world.base)
        .unwrap()
        .unwrap();
    assert_eq!(demand, 5000 * 4);
    let mut capacities = MapCapacities::default();
    capacities.raise_for_air_wing_names(demand);
    let mut map = MapCaptured::with_capacities(&capacities);
    map.air_wings
        .rows
        .resize(5000, engine::cair_wing::Row::default());
    let mut job = air::AirWingJob::new();
    job.names.resize(demand, 0);
    let report = air::copy_air_refresh_growing(
        &memory,
        air::AirWingCopyContext {
            base: world.base,
            game_state: world.state,
            hour: 0,
            force_full: true,
            limits: [5000, 0, 0],
            naval_limit: 0,
            game_score: false,
        },
        &mut job,
    );
    assert!(report.raw_ok);
    assert_eq!(job.map_outcome, Outcome::Rows { len: 5000 });
    assert_eq!((job.names_len, job.name_failures), (demand, 0));
    map.install_owned_air(&mut job);
    assert_eq!(map.air_wings.outcome, Outcome::Rows { len: 5000 });
    assert_eq!(map.air_wing_names_len, demand);
    for (ordinal, row) in map.air_wings.rows[..5000].iter().enumerate() {
        assert_eq!(row.name, Some((ordinal * 4, 4)));
        assert_eq!(&map.air_wing_names[ordinal * 4..ordinal * 4 + 4], b"Wing");
    }
}

fn measure(name: &str, shape: Shape, retained: usize, mut work: impl FnMut()) {
    let accounting = crate::read_accounting::Capture::new();
    use replay::rows::TableId;
    let anchor = match name {
        "DivisionInputsCallback" | "DivisionCensusAndCore" => TableId::Divisions,
        "DivisionMovementIdle" | "DivisionMovement25Percent" => TableId::DivisionMovement,
        "DivisionStatusIdle" => TableId::DivisionStatus,
        "UnitIntelThreeViewers" => TableId::UnitIntel,
        "DivisionEquipmentAndUpgradeGroups" => TableId::DivisionEquipment,
        "ArmyOrders" => TableId::ArmyOrders,
        "ArmyAndGroupPlans" => TableId::ArmyOrderPlans,
        "AirWingWorker" | "AirWingWorkerOrdinary" | "AirWingMapCallbackCopy" => TableId::AirWings,
        _ => panic!("unassigned benchmark {name}"),
    };
    crate::read_accounting::with_table(anchor, &mut work);
    let tables = accounting.finish();
    let mut counts = ReadCounts::ZERO;
    for (index, table) in tables.iter().enumerate() {
        counts += *table;
        if table.bytes > 0 {
            println!(
                "UNIT_TABLE job={name} table={:?} bytes={} calls={}",
                TableId::ALL[index],
                table.bytes,
                table.copy_calls + table.scattered_calls + table.record_calls + table.plan_calls
            );
        }
    }
    let (p50, p95) = fixture::time(work);
    let retained = if retained == 0 {
        "unknown".to_owned()
    } else {
        retained.to_string()
    };
    println!(
        "UNIT_SCALE name={name} divisions={} wings={} ships={} countries={} p50_ms={p50:.6} p95_ms={p95:.6} read_bytes={} calls={} retained_bytes={retained}",
        shape.divisions,
        shape.wings,
        shape.ships,
        shape.countries,
        counts.bytes,
        counts.copy_calls + counts.scattered_calls + counts.record_calls + counts.plan_calls
    );
}

#[test]
#[ignore = "release unit scaling benchmark"]
fn unit_scale_moving() {
    for shape in fixture::shapes() {
        let world = fixture::build_with_activity(shape, true);
        let memory = world.live();
        let mut census = Census::new();
        for _ in 0..3 {
            census.clear();
            census.capture(&memory, world.base, world.state, shape.divisions);
            if census.overflowed {
                census.grow();
            } else {
                break;
            }
        }
        let inputs = census.unit_inputs().unwrap();
        let mut intel = vec![engine::unit_viewer_entry::Row::default(); shape.divisions * 3];
        measure(
            "UnitIntelThreeViewers",
            shape,
            std::mem::size_of_val(intel.as_slice()),
            || {
                assert_eq!(
                    engine::unit_viewer_entry::walk_with_divisions(
                        &memory,
                        world.base,
                        world.state,
                        &mut intel,
                        Some(census.intel_sources())
                    ),
                    Outcome::Rows {
                        len: shape.divisions * 3
                    }
                );
            },
        );
        let mut geometry =
            cprovince::GeometryPacket::try_with_capacities(cprovince::GeometryCapacities {
                provinces: shape.countries + 1,
                neighbour_entries: 0,
            })
            .unwrap();
        let mut packet = carmy::MovementPacket::try_new(shape.divisions, shape.divisions).unwrap();
        measure(
            "DivisionMovement25Percent",
            shape,
            packet.allocated_bytes() + geometry.allocated_bytes(),
            || {
                geometry.begin(&memory, world.base, world.state).unwrap();
                let walk = carmy::capture_movement_inputs(
                    &memory,
                    world.base,
                    world.state,
                    inputs.seeds(),
                    inputs.outcome(),
                    shape.divisions,
                    shape.divisions,
                    &mut geometry,
                    &mut packet,
                );
                assert_eq!(
                    walk.outcome,
                    Outcome::Rows {
                        len: shape.divisions
                    }
                );
                assert_eq!(walk.paths_len, shape.divisions / 2);
            },
        );
    }
}

#[test]
#[ignore = "release unit scaling benchmark"]
fn unit_scale_orders() {
    use crate::{extended::bench_world, map_capture::MapCaptured};
    for shape in fixture::shapes() {
        let divisions = shape.divisions;
        // One group per 24 divisions is a fixture topology, not a native cap.
        let groups = divisions.div_ceil(24);
        let world = bench_world::build_groups(
            1_500,
            groups,
            bench_world::Shape {
                named: true,
                theatre_links: true,
            },
        );
        let mut capacities = crate::map_capture::MapCapacities::default();
        let mut map = MapCaptured::with_capacities(&capacities);
        let live = world.live();
        for _ in 0..4 {
            map.reset();
            map.orders.walk(&live, world.base);
            map.grow(&mut capacities);
        }
        assert_eq!(map.orders.rows.outcome, Outcome::Rows { len: groups * 2 });
        let retained = map.orders.allocated_bytes();
        measure("ArmyOrders", shape, retained, || {
            map.reset();
            map.orders.walk(&live, world.base);
            assert_eq!(map.orders.rows.outcome, Outcome::Rows { len: groups * 2 });
        });
        let mut plans = crate::extended::plans::Captured::vacant();
        let mut capacities = crate::extended::plans::Capacities::default();
        for _ in 0..3 {
            plans.reset();
            plans.visit(
                &live,
                world.base,
                world.game_state,
                map.orders.roots(),
                map.orders.visits(),
                map.orders.successor_lists(),
                map.orders.root_outcome,
                map.orders.rows.outcome,
                &map.orders.rows.rows[..map.orders.rows.outcome.len()],
                &map.orders.geometry[..map.orders.geometry_len],
                None,
            );
            plans.capture();
            plans.grow(&mut capacities);
        }
        measure("ArmyAndGroupPlans", shape, plans.allocated_bytes(), || {
            plans.reset();
            plans.visit(
                &live,
                world.base,
                world.game_state,
                map.orders.roots(),
                map.orders.visits(),
                map.orders.successor_lists(),
                map.orders.root_outcome,
                map.orders.rows.outcome,
                &map.orders.rows.rows[..map.orders.rows.outcome.len()],
                &map.orders.geometry[..map.orders.geometry_len],
                None,
            );
            plans.capture();
            assert_eq!(
                plans.outcomes(),
                [
                    Outcome::Rows { len: groups * 2 },
                    Outcome::Rows { len: groups },
                    Outcome::Rows { len: 0 },
                    Outcome::Rows { len: 0 }
                ]
            );
        });
    }
}

#[test]
#[ignore = "release unit scaling benchmark"]
fn unit_scale_capture() {
    for shape in fixture::shapes() {
        let world = fixture::build(shape);
        assert_eq!(world.armies.len(), world.shape.divisions);
        println!("UNIT_SCALE fixture_bytes={}", world.bytes.len());
        let memory = world.live();
        let mut census = Census::new();
        for _ in 0..3 {
            census.clear();
            census.capture(&memory, world.base, world.state, shape.divisions);
            if census.overflowed {
                census.grow();
            } else {
                break;
            }
        }
        assert_eq!(
            census.outcome(shape.divisions),
            Outcome::Rows {
                len: shape.divisions
            }
        );
        measure(
            "DivisionInputsCallback",
            shape,
            census.allocated_bytes() as usize,
            || {
                census.clear();
                census.capture_inputs(&memory, world.base, world.state);
                assert_eq!(
                    census.unit_inputs().unwrap().outcome(),
                    Outcome::Rows {
                        len: shape.divisions
                    }
                );
            },
        );
        measure(
            "DivisionCensusAndCore",
            shape,
            census.allocated_bytes() as usize,
            || {
                census.clear();
                census.capture(&memory, world.base, world.state, shape.divisions);
                assert_eq!(
                    census.outcome(shape.divisions),
                    Outcome::Rows {
                        len: shape.divisions
                    }
                );
            },
        );
        let inputs = census.unit_inputs().unwrap();
        let mut geometry =
            cprovince::GeometryPacket::try_with_capacities(cprovince::GeometryCapacities {
                provinces: shape.countries + 1,
                neighbour_entries: 0,
            })
            .unwrap();
        let mut movement = carmy::MovementPacket::try_new(shape.divisions, 0).unwrap();
        measure(
            "DivisionMovementIdle",
            shape,
            movement.allocated_bytes() + geometry.allocated_bytes(),
            || {
                geometry.begin(&memory, world.base, world.state).unwrap();
                let result = carmy::capture_movement_inputs(
                    &memory,
                    world.base,
                    world.state,
                    inputs.seeds(),
                    inputs.outcome(),
                    shape.divisions,
                    0,
                    &mut geometry,
                    &mut movement,
                );
                assert_eq!(
                    result.outcome,
                    Outcome::Rows {
                        len: shape.divisions
                    }
                );
            },
        );
        let mut status = carmy::StatusPacket::new(shape.divisions).unwrap();
        measure(
            "DivisionStatusIdle",
            shape,
            status.allocated_bytes(),
            || {
                assert_eq!(
                    status.capture(
                        &memory,
                        world.base,
                        world.state,
                        inputs.seeds(),
                        inputs.outcome(),
                        shape.divisions
                    ),
                    Outcome::Rows {
                        len: shape.divisions
                    }
                );
            },
        );
        let mut details = OwnedDetails::new(shape.divisions);
        let limits = [shape.divisions * 3, shape.divisions, 0, 0, 0];
        details.ready_for(limits);
        details.grow(shape.divisions);
        let (sources, outcome) = census.sources(shape.divisions);
        measure(
            "DivisionEquipmentAndUpgradeGroups",
            shape,
            details.allocated_bytes(),
            || {
                details.capture(&memory, world.base, sources, outcome, limits);
                assert_eq!(
                    details.outcomes(),
                    [
                        Outcome::Rows {
                            len: shape.divisions * 3
                        },
                        Outcome::Rows {
                            len: shape.divisions
                        },
                        Outcome::Rows { len: 0 },
                        Outcome::Rows { len: 0 },
                        Outcome::Rows { len: 0 }
                    ]
                );
            },
        );
        use crate::extended::air;
        let charged = || {
            serde_json::to_value(crate::raw_memory::diagnostics()).unwrap()["current"]["total_capacity_lower_bound_bytes"]
                .as_u64()
                .unwrap()
        };
        let before_job = charged();
        let mut job = air::AirWingJob::new();
        let context = air::AirWingCopyContext {
            base: world.base,
            game_state: world.state,
            hour: 0,
            force_full: true,
            limits: [shape.wings, shape.wings, shape.wings],
            naval_limit: shape.wings,
            game_score: false,
        };
        for _ in 0..4 {
            air::copy_air_refresh(&memory, context, &mut job);
            if job.grow_for_next() {
                job.grow();
            } else {
                break;
            }
        }
        let retained = (charged() - before_job) as usize;
        measure("AirWingWorker", shape, retained, || {
            let report = air::copy_air_refresh(&memory, context, &mut job);
            assert!(report.raw_ok);
            assert_eq!(job.map_outcome, Outcome::Rows { len: shape.wings });
        });
        let mut capacities = air::Capacities::default();
        capacities.rows[0] = shape.wings;
        capacities.rows[2] = shape.wings;
        let mut captured = air::Captured::with_capacities(&capacities);
        air::decode_air_payload(&mut captured, &job, &mut [0; 2]);
        assert_eq!(captured.outcomes()[0], Outcome::Rows { len: shape.wings });
        assert_eq!(captured.outcomes()[2], Outcome::Rows { len: shape.wings });
        let ordinary_hour = (0..24)
            .find(|&hour| {
                !replay::cadence::should_walk(replay::rows::TableId::WingCumulative, hour, false)
                    && !replay::cadence::should_walk(
                        replay::rows::TableId::AirTheatreMembers,
                        hour,
                        false,
                    )
            })
            .unwrap();
        let ordinary = air::AirWingCopyContext {
            hour: ordinary_hour,
            force_full: false,
            ..context
        };
        measure("AirWingWorkerOrdinary", shape, retained, || {
            assert!(air::copy_air_refresh(&memory, ordinary, &mut job).raw_ok);
            assert_eq!(job.map_outcome, Outcome::Rows { len: shape.wings });
        });
        let mut map = crate::map_capture::MapCaptured::new();
        map.air_wings.rows.resize(shape.wings, Default::default());
        let copied = std::mem::size_of_val(job.map_rows[..shape.wings].as_ref());
        println!("UNIT_OWNED_COPY name=AirWingMapCallbackCopy bytes={copied}");
        measure(
            "AirWingMapCallbackCopy",
            shape,
            map.air_wings.allocated_bytes(),
            || {
                map.install_owned_air(&mut job);
                assert_eq!(map.air_wings.outcome, Outcome::Rows { len: shape.wings });
            },
        );
    }
}
