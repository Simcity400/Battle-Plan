#[path = "../../engine/tests/native/support/unit_world.rs"]
pub(crate) mod fixture;
use crate::division_detail::OwnedDetails;
use collector_divisions::division_census::Census;
use engine_native::{carmy, cprovince, memory::ReadCounts, walk::Outcome};
use fixture::Shape;

fn measure(name: &str, shape: Shape, retained: usize, mut work: impl FnMut()) {
    let accounting = collector_memory::read_accounting::Capture::new();
    use replay_data::rows::TableId;
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
    collector_memory::read_accounting::with_table(anchor, &mut work);
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
            census.capture(&memory, world.base, world.state);
            if census.overflowed {
                census.grow();
            } else {
                break;
            }
        }
        let inputs = census.unit_inputs().unwrap();
        let mut intel =
            vec![engine_visibility::unit_viewer_entry::Row::default(); shape.divisions * 3];
        measure(
            "UnitIntelThreeViewers",
            shape,
            std::mem::size_of_val(intel.as_slice()),
            || {
                assert_eq!(
                    engine_visibility::unit_viewer_entry::walk_with_divisions_buffer(
                        &memory,
                        world.base,
                        world.state,
                        engine_native::walk::Buffer::Growing(&mut intel),
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
    use collector_extended_support::bench_world;
    use collector_map_capture::map_capture::MapCaptured;
    for shape in fixture::shapes() {
        let divisions = shape.divisions;
        let groups = divisions.div_ceil(24);
        let world = bench_world::build_groups(
            1_500,
            groups,
            bench_world::Shape {
                named: true,
                theatre_links: true,
            },
        );
        let mut capacities = collector_map_capture::map_capture::MapCapacities::default();
        let mut map =
        // SAFETY: These are session-measured map capacities or capacities owned by an isolated
        // fixture.
        unsafe { MapCaptured::with_capacities(&capacities) };
        let live = world.live();
        for _ in 0..4 {
            map.reset();
            map.orders.walk(&live, world.base);
            // SAFETY: This fixture owns its frame and worker state and runs without a live
            // attachment.
            unsafe { map.grow(&mut capacities) };
        }
        assert_eq!(map.orders.rows.outcome, Outcome::Rows { len: groups * 2 });
        let retained = map.orders.allocated_bytes();
        measure("ArmyOrders", shape, retained, || {
            map.reset();
            map.orders.walk(&live, world.base);
            assert_eq!(map.orders.rows.outcome, Outcome::Rows { len: groups * 2 });
        });
        let mut plans = collector_plans::extended::plans::Captured::vacant();
        let mut capacities = collector_plans::extended::plans::Capacities::default();
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
            );
            plans.visit_child_fronts(&live, world.base, map.orders.roots(), None);
            plans.capture_fronts(&live, world.base);
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
            );
            plans.visit_child_fronts(&live, world.base, map.orders.roots(), None);
            plans.capture_fronts(&live, world.base);
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
            census.capture(&memory, world.base, world.state);
            if census.overflowed {
                census.grow();
            } else {
                break;
            }
        }
        assert_eq!(
            census.outcome(),
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
                census.capture(&memory, world.base, world.state);
                assert_eq!(
                    census.outcome(),
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
        let (sources, outcome) = census.sources();
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
        use collector_air::extended::air;
        let charged = || {
            serde_json::to_value(collector_primitives::raw_memory::diagnostics()).unwrap()["current"]["total_capacity_lower_bound_bytes"]
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
                !replay_data::cadence::should_walk(
                    replay_data::rows::TableId::WingCumulative,
                    hour,
                    false,
                ) && !replay_data::cadence::should_walk(
                    replay_data::rows::TableId::AirTheatreMembers,
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
        let mut map =
        // SAFETY: This fixture owns the map allocation and runs without a live attachment.
        unsafe { collector_map_capture::map_capture::MapCaptured::new() };
        map.air_wings.rows.resize(shape.wings, Default::default());
        map.air_wing_names.resize(job.names_len, 0);
        let copied = std::mem::size_of_val(job.map_rows[..shape.wings].as_ref());
        println!("UNIT_OWNED_COPY name=AirWingMapCallbackCopy bytes={copied}");
        measure(
            "AirWingMapCallbackCopy",
            shape,
            map.air_wings.allocated_bytes(),
            || {
                // SAFETY: This is the completed air job being restored to its original frame once.
                unsafe { map.install_owned_air(&mut job) };
                assert_eq!(map.air_wings.outcome, Outcome::Rows { len: shape.wings });
            },
        );
    }
}

#[test]
#[ignore = "release unit scaling benchmark"]
fn division_input_parts_scale() {
    use collector_divisions::division_census::{MAX_PARTS, parts_for};
    for (countries, divisions) in [(1_500, 3_300), (1_500, 6_000), (5_000, 30_000)] {
        let world = fixture::build(Shape {
            countries,
            divisions,
            wings: 1,
            planes: 1,
            ships: 0,
        });
        let mut serial = Census::new_worker();
        let serial_memory = world.live();
        serial.capture_inputs(&serial_memory, world.base, world.state);
        let mut starts = [0; MAX_PARTS - 1];
        let split = serial.part_starts(&mut starts);
        let parts = split + 1;
        assert!(parts <= parts_for(divisions));
        let ranges: Vec<(u32, u32)> = (0..parts)
            .map(|part| {
                let first = if part == 0 { 1 } else { starts[part - 1] };
                (first, if part < split { starts[part] } else { u32::MAX })
            })
            .collect();
        let mut censuses: Vec<Census> = (0..parts).map(|_| Census::new_worker()).collect();
        let time = |work: &mut dyn FnMut()| {
            let mut samples: Vec<f64> = (0..15)
                .map(|_| {
                    let begin = std::time::Instant::now();
                    work();
                    begin.elapsed().as_secs_f64() * 1e3
                })
                .collect();
            samples.sort_by(f64::total_cmp);
            samples[samples.len() / 2]
        };
        let serial_ms = time(&mut || {
            serial.capture_inputs(&serial_memory, world.base, world.state);
        });
        let (base, state) = (world.base, world.state);
        let parted_ms = time(&mut || {
            std::thread::scope(|scope| {
                for (census, &slots) in censuses.iter_mut().zip(&ranges) {
                    scope.spawn(move || {
                        // SAFETY: the synthetic world is immutable and outlives the scoped worker joins.
                        let memory = unsafe { engine_native::memory::LiveMemory::new() };
                        census.capture_inputs_part(&memory, base, state, slots);
                    });
                }
            });
            let (first, rest) = censuses.split_at_mut(1);
            for part in rest {
                first[0].absorb(part).unwrap();
            }
        });
        let joined = censuses[0].unit_inputs().unwrap();
        let whole = serial.unit_inputs().unwrap();
        assert_eq!(joined.seeds(), whole.seeds());
        assert_eq!(joined.outcome(), whole.outcome());
        println!(
            "DIVISION_INPUT_PARTS countries={countries} divisions={divisions} parts={parts} serial_ms={serial_ms:.3} parted_ms={parted_ms:.3}"
        );
    }
}
