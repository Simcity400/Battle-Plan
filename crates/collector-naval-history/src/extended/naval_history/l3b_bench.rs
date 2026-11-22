use super::*;
use collector_extended_support::bench_world::{self, Shape, World};

fn time_copies(
    memory: &impl engine_native::Memory,
    world: &World,
    job: &mut NavalJob,
) -> [(f64, f64); 3] {
    let theatres = bench_world::time(50, || {
        job.log.clear();
        copy_naval_theatres(
            &mut job.log,
            &mut job.roster,
            memory,
            world.base,
            world.game_state,
            false,
        );
    });
    let oob = bench_world::time(50, || {
        job.log.clear();
        copy_naval_oob(job, memory, world.base);
        assert!(!job.log.overflowed);
    });
    let mut roots = vec![corders_group::Root::default(); world.groups as usize];
    let census = bench_world::time(50, || {
        let outcome = corders_group::walk_roots(memory, world.base, &mut roots);
        job.capture_roots(&roots, outcome);
    });
    assert!(job.roots_len.is_some());
    [theatres, oob, census]
}

fn census_roots(memory: &impl engine_native::Memory, world: &World, job: &mut NavalJob) {
    let mut roots = vec![corders_group::Root::default(); world.groups as usize];
    let outcome = corders_group::walk_roots(memory, world.base, &mut roots);
    job.capture_roots(&roots, outcome);
}

#[test]
#[ignore = "timing bench"]
fn l3b_bench_hourly_land_theatres_and_order_of_battle_ordinals() {
    for countries in [1_500, 5_000] {
        for theatre_links in [true, false] {
            let world = bench_world::build(
                countries,
                Shape {
                    named: true,
                    theatre_links,
                },
            );
            let slice = world.slice();
            let mut countries_cache = engine_native::country_memory::Cache::try_with_capacity(
                engine_native::country_memory::Cache::cells_for_countries(
                    countries as usize + 1,
                    collector_primitives::country_slots::MIN_COUNTRY_CELLS,
                ),
            )
            .unwrap();
            let _ = countries_cache.prepare(&slice, world.base, world.game_state);
            let live = world.live();
            let slice_roster = countries_cache.roster_view(&slice);
            let live_roster = countries_cache.roster_view(&live);
            let mut job = NavalJob::fixture();
            let ctx = NavalCopyContext {
                hour: 1,
                force_full: false,
                base: world.base,
                game_state: world.game_state,
            };
            while {
                census_roots(&slice, &world, &mut job);
                !copy_naval_refresh(&slice_roster, ctx, &mut job).raw_ok || job.grow_for_next()
            } {
                job.grow();
            }
            assert!(job.roots_len.is_some());
            let ((), census_work) = world.counted(|memory| {
                census_roots(memory, &world, &mut job);
            });
            let ((), theatres_work) = world.counted(|memory| {
                job.log.clear();
                copy_naval_theatres(
                    &mut job.log,
                    &mut job.roster,
                    &countries_cache.roster_view(memory),
                    world.base,
                    world.game_state,
                    false,
                );
            });
            let ((), oob_work) = world.counted(|memory| {
                job.log.clear();
                copy_naval_oob(&mut job, &countries_cache.roster_view(memory), world.base);
            });
            let all = |counts| {
                bench_world::work(bench_world::total(
                    &counts,
                    &replay_data::rows::TableId::ALL,
                ))
            };
            println!(
                "naval-work countries={countries} theatre_links={theatre_links}: LandTheatres copy {}; order-of-battle copy {}; roots census {}",
                all(theatres_work),
                all(oob_work),
                all(census_work)
            );
            for source in ["slice", "live"] {
                let [
                    (theatres, theatres_p95),
                    (oob, oob_p95),
                    (census, census_p95),
                ] = if source == "slice" {
                    time_copies(&slice_roster, &world, &mut job)
                } else {
                    time_copies(&live_roster, &world, &mut job)
                };
                println!(
                    "naval countries={countries} theatres={countries} groups={} theatre_links={theatre_links} source={source}: LandTheatres copy {theatres:.1} us (p95 {theatres_p95:.1}), order-of-battle copy {oob:.1} us (p95 {oob_p95:.1}), roots census {census:.1} us (p95 {census_p95:.1})",
                    world.groups,
                );
            }
            assert!(copy_naval_refresh(&slice_roster, ctx, &mut job).raw_ok);
            let rows = 4 * world.groups as usize;
            let capacity = Capacities {
                rows: [rows; N],
                ..Capacities::fixture()
            };
            let mut captured = Captured::with_capacities(&capacity);
            let mut ticks = [0; NAVAL_PACK_N];
            let (decode, decode_p95) = bench_world::time(20, || {
                decode_naval_payload(&mut captured, world.base, &job, &mut ticks);
            });
            assert_eq!(
                captured.land.outcome,
                Outcome::Rows {
                    len: countries as usize
                }
            );
            assert_eq!(
                captured.groups.outcome,
                Outcome::Rows {
                    len: world.groups as usize
                }
            );
            assert_eq!(
                captured.groups.rows[..world.groups as usize]
                    .iter()
                    .all(|row| row.theatre_ordinal.is_some()),
                theatre_links
            );
            println!(
                "naval-decode countries={countries} theatre_links={theatre_links}: hourly naval decode {decode:.1} us (p95 {decode_p95:.1}), raw bytes {}, spans {}, bulk {}",
                job.log.raw_bytes.iter().sum::<u64>(),
                job.log.spans.len(),
                job.log.has_bulk
            );
        }
    }
}
