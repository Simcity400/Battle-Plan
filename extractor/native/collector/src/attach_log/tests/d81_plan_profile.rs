//! contention and no worker pool; `SyntheticMemory` is a bounds check and a
//! cargo test -p collector --release d81_plan_profile -- --ignored --nocapture --test-threads=1

use super::d81_authoritative::{stage_world_in, walk_with_layout, Layout, StagedWorld, WorldShape};
use super::*;
use crate::attach_log::d81_copy_bracket::{CountingSnapshotMemory, D81PlanCounters};
use crate::attach_log::d81_vector_store::{set_resolution_arm, Arm};
use crate::snapshot_recipes::HourStamp;
use std::time::{Duration, Instant};

const PROFILE_HOUR: HourStamp = HourStamp(1939_09_01);

struct Observed {
    wall: Duration,
    counters: D81PlanCounters,
}

fn observe_once(world: &StagedWorld) -> Observed {
    let mut counters = D81PlanCounters::default();
    let memory = CountingSnapshotMemory::new(&world.memory);
    let started = Instant::now();
    let closed = d81_observe_copy_plan_with(&memory, &world.roots, PROFILE_HOUR, &mut counters);
    let wall = started.elapsed();
    assert!(
        closed.is_some(),
        "custody must admit the profile world at {} slots",
        world.roots.province_slots,
    );
    counters.copies_issued = memory.calls();
    Observed { wall, counters }
}

const SIZES: [u32; 4] = [385, 1027, 2053, 3511];

/// Observations per size in the walk profile. Odd, so `RUNS / 2` is a median.
const RUNS: usize = 15;

const T48_COPIES: u64 = 84_233;
const T49_COPIES: u64 = 84_256;

#[test]
#[ignore]
fn profile_d81_plan_shape_by_size() {
    println!();
    println!("d81 copy plan, uninstrumented, over five observations per size");
    println!(
        "  {:>7} {:>9} {:>10} {:>10} {:>10} {:>10}",
        "slots", "intervals", "copies", "min ms", "med ms", "max ms"
    );
    for size in SIZES {
        let world = stage_world_in(WorldShape::default(), Layout::sized(size));
        let _ = observe_once(&world);
        let mut runs: Vec<Observed> = (0..5).map(|_| observe_once(&world)).collect();
        runs.sort_by_key(|run| run.wall);
        let counters = &runs[0].counters;
        println!(
            "  {:>7} {:>9} {:>10} {:>10.3} {:>10.3} {:>10.3}",
            size,
            counters.intervals_closed,
            counters.copies_issued,
            runs[0].wall.as_secs_f64() * 1.0e3,
            runs[2].wall.as_secs_f64() * 1.0e3,
            runs[4].wall.as_secs_f64() * 1.0e3,
        );
    }
    println!(
        "  production for comparison: t48 {} copies, t49 {} copies",
        T48_COPIES, T49_COPIES
    );
}

// ---------------------------------------------------------------------------
// THE WALK'S OWN PRICE (lane PC, 2026-08-21)
// ---------------------------------------------------------------------------
//
// `profile_d81_plan_shape_by_size` above prices the COPY. It cannot see the
// phase that oz_tip puts ahead of the copy: `d81_phase_walk_total_us` is 17.92
// ms/invocation against the plan's 9.36, and the walk is pure compute over
// bytes this process already owns -- no engine, no bracket, no guarded read.
// That makes it the one phase whose whole cost is reproducible offline, and
// the reason this profile exists: an optimisation of the walk that cannot be
// seen here is not an optimisation of the walk.
//
// WHAT THE NUMBERS HERE ARE AND ARE NOT. `SyntheticMemory` is a bounds check
// and a `copy_from_slice`, so the PLAN column understates live (a live copy
// carries `range_confirmed_good`). The WALK column does not: every byte it
// reads is served out of the snapshot's own slabs by the same code that runs
// live, so its cost transfers, up to the address distribution
// (`Layout::sized` scatters the per-province objects 0x20000 apart, which is
// wider than a live heap and therefore pessimistic for the merged index's
// cache behaviour, not optimistic).
struct Walked {
    plan_wall: Duration,
    walk_wall: Duration,
    counters: D81PlanCounters,
    phases: D81PhaseTimes,
}

fn plan_and_walk_once(world: &StagedWorld, layout: &Layout) -> Walked {
    let mut counters = D81PlanCounters::default();
    let memory = CountingSnapshotMemory::new(&world.memory);
    let plan_started = Instant::now();
    let closed = d81_observe_copy_plan_with(&memory, &world.roots, PROFILE_HOUR, &mut counters)
        .expect("custody must admit the profile world");
    let plan_wall = plan_started.elapsed();
    let source = closed
        .source_for(PROFILE_HOUR)
        .expect("the plan captured, so its consumer has a view");
    let phases = D81PhaseAccum::default();
    let walk_started = Instant::now();
    let rows = walk_with_layout(world, layout, &source, &phases);
    let walk_wall = walk_started.elapsed();
    let rows = rows.expect("the clean profile world must publish rows");
    // The world names `slots - 1` provinces and the top triple is railless, so
    // a profile that quietly walked a truncated world would be timing a
    // different shape than the one it prints.
    assert_eq!(
        rows.provinces.len(),
        world.roots.province_slots as usize - 4,
        "every province except the railless triple must publish",
    );
    counters.copies_issued = memory.calls();
    Walked {
        plan_wall,
        walk_wall,
        counters,
        phases: phases.snapshot(),
    }
}

#[test]
#[ignore]
fn profile_d81_walk_shape_by_size() {
    println!();
    println!("d81 copy plan + authoritative walk, five observations per size");
    println!(
        "  {:>7} {:>9} {:>8} {:>8} {:>8} {:>8} {:>7} {:>7} {:>7} {:>7} {:>7}",
        "slots", "copies", "planMIN", "planMED", "walkMIN", "walkMED", "rail", "bind", "sel", "close",
        "act",
    );
    for size in SIZES {
        let layout = Layout::sized(size);
        let world = stage_world_in(WorldShape::default(), layout);
        let _ = plan_and_walk_once(&world, &layout);
        let mut runs: Vec<Walked> = (0..RUNS)
            .map(|_| plan_and_walk_once(&world, &layout))
            .collect();
        // MINIMUM, NOT MEDIAN, and the median beside it so a reader can see how
        // far apart they are. This box runs other lanes; a run that lost the
        // core to something else measures the scheduler, and the fastest of
        // fifteen is the closest this rig gets to the code's own cost. The two
        // phases are minimised INDEPENDENTLY because they are separately timed
        // spans and a run can be unlucky in one and not the other.
        runs.sort_by_key(|run| run.plan_wall);
        let plan_min = runs[0].plan_wall;
        let plan_med = runs[RUNS / 2].plan_wall;
        let copies = runs[0].counters.copies_issued;
        runs.sort_by_key(|run| run.walk_wall);
        let walk = &runs[0];
        let walk_med = runs[RUNS / 2].walk_wall;
        let ms = |value: u64| value as f64 / 1.0e3;
        println!(
            "  {:>7} {:>9} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>7.3} {:>7.3} {:>7.3} {:>7.3} {:>7.3}",
            size,
            copies,
            plan_min.as_secs_f64() * 1.0e3,
            plan_med.as_secs_f64() * 1.0e3,
            walk.walk_wall.as_secs_f64() * 1.0e3,
            walk_med.as_secs_f64() * 1.0e3,
            ms(walk.phases.walk_rail_us),
            ms(walk.phases.walk_bind_us),
            ms(walk.phases.walk_selector_us),
            ms(walk.phases.walk_close_us),
            ms(walk.phases.walk_active_us),
        );
    }
    println!(
        "  oz_tip for comparison, per invocation: plan 9.36 ms, walk_total 17.92 ms \
         (rail 5.89, bind 3.70, selector 3.52, close 1.83, active 1.26, anchor 0.83)"
    );
}

// ---------------------------------------------------------------------------
// THE TWO ARMS, IN ONE PROCESS (lane PC, 2026-08-21)
// ---------------------------------------------------------------------------
//
// WHY THIS IS NOT A BEFORE-AND-AFTER OF TWO BUILDS. It was, first, and the
// build-to-build numbers could not carry the claim: this box runs a game for
// another lane, and the SAME test binary measured three times in a row put the
// 3,511-slot walk at 10.085, 7.441 and 8.285 ms and its untouched plan phase
// at 7.953, 6.448 and 9.246 ms. A 25-35% spread on identical code is wider
// than most changes worth making, so a cross-build difference at this shape
// says nothing on its own.
//
// Interleaving the arms inside ONE process removes the machine from the
// comparison: both arms see the same core, the same cache state, the same
// neighbours, alternating so that any drift lands on both. The order flips
// every repetition for the same reason. This is the shape
// `d81_guarded_read_cost_model` already uses for the guarded-read primitives.
fn walk_arm(world: &StagedWorld, layout: &Layout, arm: Arm) -> (Duration, usize) {
    let mut counters = D81PlanCounters::default();
    let memory = CountingSnapshotMemory::new(&world.memory);
    let closed = d81_observe_copy_plan_with(&memory, &world.roots, PROFILE_HOUR, &mut counters)
        .expect("custody must admit the profile world");
    let source = closed
        .source_for(PROFILE_HOUR)
        .expect("the plan captured, so its consumer has a view");
    let phases = D81PhaseAccum::default();
    let restore = set_resolution_arm(arm);
    let started = Instant::now();
    let rows = walk_with_layout(world, layout, &source, &phases);
    let elapsed = started.elapsed();
    set_resolution_arm(restore);
    let rows = rows.expect("the clean profile world must publish rows");
    (elapsed, rows.provinces.len())
}

#[test]
#[ignore]
fn profile_d81_walk_resolution_arms() {
    println!();
    println!("d81 walk, resolution cache against the array-and-loop form it replaced");
    println!("  arms interleaved in one process, order flipped each repetition");
    println!(
        "  {:>7} {:>10} {:>10} {:>9}",
        "slots", "legacy ms", "cache ms", "cut",
    );
    for size in SIZES {
        let layout = Layout::sized(size);
        let world = stage_world_in(WorldShape::default(), layout);
        let _ = walk_arm(&world, &layout, Arm::Fast);
        let mut legacy = Duration::MAX;
        let mut cache = Duration::MAX;
        for repetition in 0..RUNS {
            // FLIPPED, so that whichever arm runs second does not carry the
            // cost of being second in every sample.
            let order = if repetition % 2 == 0 {
                [Arm::Legacy, Arm::Fast]
            } else {
                [Arm::Fast, Arm::Legacy]
            };
            for arm in order {
                let (elapsed, _) = walk_arm(&world, &layout, arm);
                let slot = if arm == Arm::Legacy {
                    &mut legacy
                } else {
                    &mut cache
                };
                *slot = (*slot).min(elapsed);
            }
        }
        let legacy_ms = legacy.as_secs_f64() * 1.0e3;
        let cache_ms = cache.as_secs_f64() * 1.0e3;
        println!(
            "  {:>7} {:>10.3} {:>10.3} {:>8.1}%",
            size,
            legacy_ms,
            cache_ms,
            100.0 * (legacy_ms - cache_ms) / legacy_ms,
        );
    }
}

/// EVERY READ OF A WHOLE WALK, BOTH WAYS, ASSERTED EQUAL.
///
/// `Arm::Compare` resolves each read twice and panics on any disagreement, so
/// this walk is a differential test over every `(addr, len)` the production
/// walk actually asks for at the production shape -- roughly two hundred
/// thousand reads across the four sizes, covering served opening reads, served
/// closing reads, the sample images, and the `NotCovered` reads that fall
/// through to the recipe's own view.
///
/// It is not `#[ignore]`d: it is the gate that says the optimisation did not
/// change what the walk sees, and it belongs in the default run.
#[test]
fn the_resolution_cache_decides_every_read_the_old_form_decided() {
    for size in SIZES {
        let layout = Layout::sized(size);
        let world = stage_world_in(WorldShape::default(), layout);
        let (_, compared_rows) = walk_arm(&world, &layout, Arm::Compare);
        let (_, legacy_rows) = walk_arm(&world, &layout, Arm::Legacy);
        assert_eq!(
            compared_rows,
            size as usize - 4,
            "the compared walk must publish the whole world",
        );
        assert_eq!(
            compared_rows, legacy_rows,
            "both arms must publish the same rows at {size} slots",
        );
    }
}

// ---------------------------------------------------------------------------
// WHY THE PER-PROVINCE ALLOCATIONS WERE LEFT ALONE (lane PC, 2026-08-21)
// ---------------------------------------------------------------------------
//
// The walk mints two `vec![0u8; pair_bytes_len]` and one `BTreeSet<u32>` per
// RAILED province -- 10,533 allocations per invocation at oz_tip's 3,511 --
// and the obvious next cut is to hoist the two buffers out of the loop
// (`MAX_NEIGHBORS_PER_PROVINCE` bounds the blob, and the bound is checked one
// branch above the read) and to answer the duplicate test by scanning the
// neighbour rows already being built.
//
// THAT CHANGE WAS WRITTEN, PRICED HERE, AND REVERTED. This bench is what
// priced it: the removed shape and the shape that would have replaced it, at
// the production count, interleaved and minimised the way the arm profile
// above does. The answer is 0.135 ms per invocation, reproducible to 0.002 --
// 0.3% of d81's 48.9 ms read. The duplicate half of it also moves the
// `Duplicate` / `InvalidNeighbor` decision onto new code, and a refusal path
// is not worth rewriting for three tenths of one percent. If the walk is ever
// reworked for a larger reason, this is the number that says the allocator was
// not the problem.
//
// THIS IS A PRICE, NOT A WALK MEASUREMENT, and the two must not be confused:
// it says what the allocations cost, not what a walk without them would save.
#[test]
#[ignore]
fn profile_d81_walk_per_province_allocations() {
    use std::collections::BTreeSet;

    const RAILED: usize = 3_511;
    const NEIGHBORS: usize = 4;

    let mut allocating = Duration::MAX;
    let mut borrowed = Duration::MAX;
    for repetition in 0..RUNS {
        let mut run_allocating = || {
            let started = Instant::now();
            let mut sink = 0u64;
            for province in 0..RAILED {
                let bytes = NEIGHBORS * 8;
                let first = vec![0u8; bytes];
                let second = vec![0u8; bytes];
                sink = sink.wrapping_add((first.len() + second.len()) as u64);
                let mut seen = BTreeSet::new();
                for neighbor in 0..NEIGHBORS {
                    sink = sink.wrapping_add(u64::from(
                        seen.insert((province * NEIGHBORS + neighbor) as u32),
                    ));
                }
            }
            (started.elapsed(), sink)
        };
        let mut run_borrowed = || {
            let started = Instant::now();
            let mut sink = 0u64;
            let mut first = [0u8; rail_replay::MAX_NEIGHBORS_PER_PROVINCE * 8];
            let mut second = [0u8; rail_replay::MAX_NEIGHBORS_PER_PROVINCE * 8];
            for province in 0..RAILED {
                let bytes = NEIGHBORS * 8;
                let first = &mut first[..bytes];
                let second = &mut second[..bytes];
                sink = sink.wrapping_add((first.len() + second.len()) as u64);
                let mut seen: Vec<u32> = Vec::with_capacity(NEIGHBORS);
                for neighbor in 0..NEIGHBORS {
                    let id = (province * NEIGHBORS + neighbor) as u32;
                    let fresh = !seen.iter().any(|&held| held == id);
                    if fresh {
                        seen.push(id);
                    }
                    sink = sink.wrapping_add(u64::from(fresh));
                }
            }
            (started.elapsed(), sink)
        };
        // Flipped each repetition, for the reason the arm profile flips.
        let (a, b) = if repetition % 2 == 0 {
            let a = run_allocating();
            let b = run_borrowed();
            (a, b)
        } else {
            let b = run_borrowed();
            let a = run_allocating();
            (a, b)
        };
        assert_eq!(a.1, b.1, "both shapes must do the same work");
        allocating = allocating.min(a.0);
        borrowed = borrowed.min(b.0);
    }
    let allocating_ms = allocating.as_secs_f64() * 1.0e3;
    let borrowed_ms = borrowed.as_secs_f64() * 1.0e3;
    println!();
    println!(
        "d81 per-province allocations at {RAILED} railed provinces, {NEIGHBORS} neighbours each"
    );
    println!("  two vecs + a BTreeSet : {allocating_ms:.3} ms");
    println!("  slices + a small vec  : {borrowed_ms:.3} ms");
    println!("  removed               : {:.3} ms", allocating_ms - borrowed_ms);
}
