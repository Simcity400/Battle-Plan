//! contention and no worker pool; `SyntheticMemory` is a bounds check and a
//! cargo test -p collector --release d81_plan_profile -- --ignored --nocapture --test-threads=1

use super::d81_authoritative::{stage_world_in, Layout, StagedWorld, WorldShape};
use super::*;
use crate::attach_log::d81_copy_bracket::{CountingSnapshotMemory, D81PlanCounters};
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
