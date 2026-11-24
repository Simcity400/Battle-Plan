use engine_native_air::{cair_base, cair_wing};
use engine_native::{carmy, cfleet, cship, ctask_force};
use engine_native_memory::{memory::ReadCounts, walk::Outcome};
use crate::support::unit_world::{self as fixture, Shape};

fn run(shape: Shape, timed: bool) {
    let world = fixture::build(shape);
    let memory = world.live();
    let mut divisions = vec![carmy::Row::default(); shape.divisions];
    let mut wings = vec![cair_wing::Row::default(); shape.wings];
    let mut fleets = vec![cfleet::Row::default(); shape.countries];
    let mut tasks = vec![ctask_force::Row::default(); shape.countries];
    let mut ships = vec![cship::Row::default(); shape.ships];
    let mut bases = vec![cair_base::Row::default(); shape.countries];
    assert_eq!(world.armies.len(), world.shape.divisions);
    let measure = |name: &str, bytes: usize, work: &mut dyn FnMut()| {
        ReadCounts::take_current();
        work();
        let counts = ReadCounts::take_current();
        if timed {
            let (p50, p95) = fixture::time(work);
            println!(
                "UNIT_SCALE name={name} divisions={} wings={} planes={} ships={} countries={} p50_ms={p50:.6} p95_ms={p95:.6} read_bytes={} calls={} output_bytes={bytes} fixture_bytes={}",
                shape.divisions,
                shape.wings,
                shape.planes,
                shape.ships,
                shape.countries,
                counts.bytes,
                counts.copy_calls
                    + counts.scattered_calls
                    + counts.record_calls
                    + counts.plan_calls,
                world.bytes.len()
            );
        }
    };
    measure(
        "DivisionsDirect",
        std::mem::size_of_val(divisions.as_slice()),
        &mut || {
            assert_eq!(
                carmy::walk(&memory, world.base, world.state, &mut divisions),
                Outcome::Rows {
                    len: shape.divisions
                }
            );
        },
    );
    measure(
        "AirWings",
        std::mem::size_of_val(wings.as_slice()),
        &mut || {
            assert_eq!(
                cair_wing::walk(&memory, world.base, world.state, &mut wings),
                Outcome::Rows { len: shape.wings }
            );
            assert_eq!(
                wings.iter().map(|r| r.planes as usize).sum::<usize>(),
                shape.planes
            );
        },
    );
    measure(
        "FleetsTaskForcesShips",
        std::mem::size_of_val(fleets.as_slice())
            + std::mem::size_of_val(tasks.as_slice())
            + std::mem::size_of_val(ships.as_slice()),
        &mut || {
            let result = cfleet::walk(
                &memory,
                world.base,
                world.state,
                &mut fleets,
                &mut tasks,
                &mut ships,
            );
            assert_eq!(
                result.fleets,
                Outcome::Rows {
                    len: shape.countries
                }
            );
            assert_eq!(
                result.task_forces,
                Outcome::Rows {
                    len: shape.countries
                }
            );
            assert_eq!(result.ships, Outcome::Rows { len: shape.ships });
        },
    );
    measure(
        "AirBases",
        std::mem::size_of_val(bases.as_slice()),
        &mut || {
            assert_eq!(
                cair_base::walk(&memory, world.base, world.state, &mut bases),
                Outcome::Rows {
                    len: shape.countries
                }
            );
        },
    );
    let mut registrations = vec![cair_base::RegistrationRow::default(); shape.countries];
    let mut members = vec![0; shape.wings];
    let mut resolved = vec![None; shape.countries + 1];
    measure(
        "AirBaseRegistrations",
        std::mem::size_of_val(registrations.as_slice())
            + std::mem::size_of_val(members.as_slice())
            + std::mem::size_of_val(resolved.as_slice()),
        &mut || {
            let walk = cair_base::walk_registrations(
                &memory,
                world.base,
                world.state,
                cair_base::RegistrationBuffers {
                    registrations: &mut registrations,
                    wings: &mut members,
                    resolved: &mut resolved,
                },
            );
            assert_eq!(
                walk.outcome,
                Outcome::Rows {
                    len: shape.countries
                }
            );
            assert_eq!(walk.wings_len, shape.wings);
        },
    );
}

#[test]
fn unit_population_is_complete() {
    run(
        Shape {
            countries: 7,
            divisions: 101,
            wings: 103,
            planes: 1030,
            ships: 107,
        },
        false,
    );
}

