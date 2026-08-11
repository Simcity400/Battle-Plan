//! WHERE d81'S COPY-PLAN PHASE SPENDS ITS TIME — lane GL's instrument.
//!
//! Lane GJ measured `d81_phase_plan_us` at 67.0 ms per observation in t49 and
//! 59.2 ms in t48, 84,256 substrate transactions apiece, and said in terms that
//! it could not say WHERE those milliseconds went: it may not run a capture,
//! and lane GA's instrument was armed on d47's shape. This module is that
//! instrument, re-armed on d81's.
//!
//! # What it runs
//!
//! The SHIPPED entry point, `d81_observe_copy_plan_with`, over the staged world
//! `d81_authoritative` gates the conversion on — the same recipe, the same
//! closing program, the same four-image sequence — at several SIZES. The
//! default fixture is 385 province slots; production observes 3,512. Reporting
//! a 385-slot number as d81's cost would be the extrapolation this lane exists
//! to refuse, so the world is staged at both and at the sizes between them, and
//! the shape that matters is the one whose TRANSACTION COUNT matches the
//! capture's: t48's 84,233 and t49's 84,256.
//!
//! # What it is not
//!
//! It is not a wall-clock claim about a capture. There is no game, no
//! contention and no worker pool; `SyntheticMemory` is a bounds check and a
//! `memcpy` where the live substrate goes through the SEH-guarded primitive
//! against another process's pages. Absolute nanoseconds off this fixture do
//! not travel to another box or another session — lane GA's receipt says so of
//! its own figures and lane GJ withdrew a conclusion built on ignoring it. So
//! this bench exists to be run TWICE, once per binary, INTERLEAVED — see
//! `runs/gl_plan_profile/pair_driver.py`, which is what turns it into a
//! before/after that survives a box whose load nobody controls.
//!
//! # What decomposed the phase, and where it went
//!
//! A levelled timer scaffold, applied to the executor, measured with, and then
//! REVERTED: no capture should pay for a timer check in the substrate's hottest
//! loop. `runs/gl_plan_profile/instrumentation.patch` re-applies the whole of
//! it, including the two decomposition benches that lived beside this one, and
//! `runs/gl_plan_profile/DECOMPOSITION.txt` is what it measured.
//!
//! # How to run it
//!
//! ```text
//! cargo test -p collector --release d81_plan_profile -- --ignored --nocapture --test-threads=1
//! ```
//!
//! `--test-threads=1` is not optional: a second bench running beside this one
//! contends for the cores of the observation it is timing.

use super::d81_authoritative::{stage_world_in, Layout, StagedWorld, WorldShape};
use super::*;
use crate::attach_log::d81_copy_bracket::{CountingSnapshotMemory, D81PlanCounters};
use crate::snapshot_recipes::HourStamp;
use std::time::{Duration, Instant};

const PROFILE_HOUR: HourStamp = HourStamp(1939_09_01);

/// What one observation of one staged world cost and issued.
struct Observed {
    wall: Duration,
    counters: D81PlanCounters,
}

/// Run the SHIPPED plan entry point once over `world`.
///
/// The window is `d81_observe_copy_plan_with`'s own — recipe build, observation
/// and `close_witnesses` — which is the window `D81PlanCounters::plan_us`
/// measures and the window `d81_phase_plan_us` encloses. Anything outside it is
/// outside the number this module is about.
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

/// The sizes the profile runs. `1 (mod 3)`, because the topology's neighbour
/// rings are triples.
///
/// 385 is the fixture every conversion gate in `d81_authoritative` runs; 3,511
/// is production's own province count at t49 (`d81_phase_provinces_railed` =
/// 3,511). The two between them are there so a term that grows FASTER than the
/// transaction count is visible as such rather than assumed away.
const SIZES: [u32; 4] = [385, 1027, 2053, 3511];

/// The transaction counts the two post-conversion captures measured, so the
/// table can say which staged size is production's shape rather than leaving a
/// reader to guess.
const T48_COPIES: u64 = 84_233;
const T49_COPIES: u64 = 84_256;

/// THE SHAPE TABLE: what each staged size costs and issues, uninstrumented.
///
/// Run first and read first. Every share in the decomposition below is a share
/// OF one of these numbers, and a share of a total nobody published is not a
/// measurement.
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
        // One warm observation before the five that are kept: the first one
        // into a freshly staged world pays the fixture's own page faults, which
        // is the harness's cost and not the plan's.
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
