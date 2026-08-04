//! Makespan of the reader fan-out's grant policy (lane AC, 2026-08-04).
//!
//! WHAT THIS IS. The fan-out's cost is not the sum of its domain reads, it is
//! the finishing time of the LAST lane. A live capture is the only instrument
//! that produces `lw_crit_wall_us`, and it needs the game running; this module
//! is the offline stand-in for the one variable this lane changes -- the ORDER
//! in which `ReaderGate` grants a freed lane. It drives the real `ReaderGate`,
//! with the real permit count, over the real job-duration list, and reports
//! the makespan each policy achieves.
//!
//! WHAT IS MEASURED vs INFERRED:
//!  * MEASURED here: the makespan the three grant policies produce on this
//!    machine for this job list, through the actual `ReaderGate` the collector
//!    ships, at the real thread count.
//!  * INFERRED: that the ordering benefit transfers to the live capture. Each
//!    job here is a bounded busy-wait, so its duration is INDEPENDENT of how
//!    many other jobs run beside it. Real domain reads are memory-bound and do
//!    contend, so packing more of them concurrently can lengthen them. This
//!    model therefore OVERSTATES the achievable makespan reduction, and the
//!    live figure should be read as a fraction of it, never as at least it.
//!
//! JOB DURATIONS are run t19's own, from
//! `runs/live_unlock/t19/native_session_manifest.json`: each scheduled
//! domain's `read_time_us.avg` over the run's 501 heavy frames. Domains that a
//! single bundled worker reads together report one shared figure, so entries
//! with an identical statistics signature are collapsed to the ONE job the
//! fan-out actually schedules -- 65 jobs, 351.0 ms total, longest 29.6 ms.
//! Cross-check: the 12 lanes' own `busy_us_total` for the same run sums to
//! 344.1 ms per frame, within 2% of this list.
//!
//! Spawn order is ascending domain id, which is the order the fan-out's spawn
//! block is written in, so the unranked arms see the arrival order production
//! actually presents.
//!
//! These entry points are `#[ignore]`d: they are an instrument, not a gate.
//! Run with
//! `cargo test -p collector --release -- --ignored --nocapture reader_gate_schedule`.

use super::*;

/// (domain id, mean read microseconds) from t19. See the module header for how
/// bundled domains were collapsed.
const T19_JOB_US: [(u16, u64); 65] = [
    (10, 640),
    (11, 10_175),
    (12, 20_535),
    (14, 7_005),
    (15, 3_756),
    (16, 11_920),
    (17, 420),
    (18, 3_519),
    (20, 9_662),
    (21, 1_344),
    (22, 2_147),
    (23, 14_577),
    (24, 2_846),
    (26, 11_860),
    (27, 7_188),
    (28, 12_496),
    (29, 12_628),
    (30, 2_026),
    (31, 1_620),
    (32, 2_576),
    (33, 2_811),
    (34, 2_089),
    (35, 2_229),
    (36, 3_865),
    (37, 2_367),
    (38, 4_368),
    (39, 2_019),
    (40, 1_404),
    (41, 22_717),
    (42, 7_019),
    (43, 16_269),
    (44, 15_432),
    (45, 999),
    (46, 10_410),
    (47, 13_283),
    (48, 10_151),
    (49, 4_273),
    (51, 14_502),
    (52, 6_468),
    (53, 29_608),
    (54, 749),
    (55, 1_421),
    (56, 11_960),
    (60, 296),
    (61, 405),
    (62, 888),
    (63, 1_181),
    (64, 812),
    (65, 293),
    (66, 984),
    (67, 801),
    (68, 86),
    (69, 340),
    (70, 291),
    (81, 0),
    (83, 1_422),
    (85, 4),
    (86, 6),
    (90, 4_345),
    (91, 785),
    (92, 2_779),
    (93, 9_904),
    (94, 2),
    (95, 1),
    (97, 3),
];

/// t19's configured `reader-concurrency`.
const BENCH_PERMITS: usize = 12;

/// The pre-lane-AC gate, reproduced here so the A/B has a real control arm
/// rather than a modelled one. Byte-for-byte the `ReaderGate` this lane
/// replaced: a free-list plus one condvar, so a released lane is raced for by
/// whichever waiter the OS decides to wake.
struct LegacyReaderGate {
    avail: Mutex<Vec<u16>>,
    cv: Condvar,
}

struct LegacyPermit<'a> {
    gate: &'a LegacyReaderGate,
    lane: u16,
}

impl LegacyReaderGate {
    fn new(permits: usize) -> LegacyReaderGate {
        let permits = permits.clamp(1, 4096) as u16;
        LegacyReaderGate {
            avail: Mutex::new((0..permits).rev().collect()),
            cv: Condvar::new(),
        }
    }

    fn acquire(&self) -> LegacyPermit<'_> {
        let mut avail = self.avail.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(lane) = avail.pop() {
                return LegacyPermit { gate: self, lane };
            }
            avail = self.cv.wait(avail).unwrap_or_else(|e| e.into_inner());
        }
    }
}

impl Drop for LegacyPermit<'_> {
    fn drop(&mut self) {
        let mut avail = self.gate.avail.lock().unwrap_or_else(|e| e.into_inner());
        avail.push(self.lane);
        self.gate.cv.notify_one();
    }
}

/// Occupy the calling thread for `us` microseconds. Wall-clock bounded rather
/// than iteration bounded so a job's length does not drift with CPU frequency
/// or with how many other jobs are running.
fn burn_us(us: u64) {
    if us == 0 {
        return;
    }
    let budget = Duration::from_micros(us);
    let started = Instant::now();
    while started.elapsed() < budget {
        std::hint::spin_loop();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GrantPolicy {
    /// The gate this lane replaced: whichever waiter the OS wakes.
    LegacyCondvar,
    /// The ranked gate with an empty cost table, i.e. strict arrival order.
    RankedButUnlearned,
    /// The ranked gate with a converged cost table: longest first.
    RankedByMeasuredCost,
}

/// When the worker calls `acquire`. This is the variable the grant policy
/// CANNOT control: the gate can only rank the waiters that have already
/// enqueued, so a costly domain whose thread does not exist yet is invisible
/// to it however good the ranking is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ArrivalModel {
    /// Production today: 65 threads created one after another, each racing to
    /// the gate as soon as it starts.
    AsSpawned,
    /// Every thread created AND provably at the gate before the first grant,
    /// with the clock starting at the release. This is the FLOOR the ranking
    /// can reach, not a shape production can have: it charges nothing for
    /// creating the threads and it uses a counted barrier the fan-out has no
    /// way to size.
    AllPresentFloor,
    /// The same counted barrier, charged for thread creation. Not a shape
    /// production can have either -- the fan-out cannot know its own worker
    /// count ahead of the spawn block, since every domain is conditionally
    /// enabled -- but it isolates ONE variable against `AdmissionHold`: what
    /// it costs that the hold cannot guarantee every worker has enqueued.
    AllPresentCharged,
    /// The production shape: the spawning thread takes `hold_admission` before
    /// its first spawn and drops it after its last, and the clock runs from
    /// before the first thread is created. Weaker than the floor on purpose --
    /// the hold stops GRANTS, it cannot make a thread that has not reached
    /// `acquire` yet be in the queue -- and that residual is exactly what
    /// separates this arm from the floor.
    AdmissionHold,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Arm {
    policy: GrantPolicy,
    arrival: ArrivalModel,
    /// Spawn in descending expected cost instead of source (domain id) order.
    /// The gate cannot do this itself; it measures what reordering the spawn
    /// block would be worth on top of the ranking.
    costliest_first_spawn: bool,
    name: &'static str,
}

const ARMS: [Arm; 7] = [
    Arm {
        policy: GrantPolicy::LegacyCondvar,
        arrival: ArrivalModel::AsSpawned,
        costliest_first_spawn: false,
        name: "legacy_condvar",
    },
    Arm {
        policy: GrantPolicy::RankedButUnlearned,
        arrival: ArrivalModel::AsSpawned,
        costliest_first_spawn: false,
        name: "ranked_unlearned",
    },
    Arm {
        policy: GrantPolicy::RankedByMeasuredCost,
        arrival: ArrivalModel::AsSpawned,
        costliest_first_spawn: false,
        name: "ranked_by_cost",
    },
    Arm {
        policy: GrantPolicy::RankedByMeasuredCost,
        arrival: ArrivalModel::AllPresentFloor,
        costliest_first_spawn: false,
        name: "ranked_all_present_floor",
    },
    Arm {
        policy: GrantPolicy::RankedByMeasuredCost,
        arrival: ArrivalModel::AllPresentCharged,
        costliest_first_spawn: false,
        name: "ranked_all_present_charged",
    },
    Arm {
        policy: GrantPolicy::RankedByMeasuredCost,
        arrival: ArrivalModel::AdmissionHold,
        costliest_first_spawn: false,
        name: "ranked_admission_hold",
    },
    Arm {
        policy: GrantPolicy::RankedByMeasuredCost,
        arrival: ArrivalModel::AsSpawned,
        costliest_first_spawn: true,
        name: "ranked_costliest_first_spawn",
    },
];

/// Run the whole 65-job fan-out once under `arm` and return its makespan in
/// microseconds: the wall from just before the first spawn to just after the
/// last join, which is the quantity `lw_worker_span_us` reports live.
fn run_fan_out(arm: Arm) -> u64 {
    let ranked = ReaderGate::new(BENCH_PERMITS);
    let legacy = LegacyReaderGate::new(BENCH_PERMITS);
    let mut jobs: Vec<u64> = T19_JOB_US.iter().map(|&(_, us)| us).collect();
    if arm.costliest_first_spawn {
        jobs.sort_unstable_by(|a, b| b.cmp(a));
    }
    let barriered = matches!(
        arm.arrival,
        ArrivalModel::AllPresentFloor | ArrivalModel::AllPresentCharged
    );
    let gate_open = std::sync::Barrier::new(if barriered { jobs.len() + 1 } else { 1 });
    let started = Instant::now();
    let makespan = thread::scope(|scope| {
        // The real production primitive, held over the real spawn block.
        let admission = (arm.arrival == ArrivalModel::AdmissionHold)
            .then(|| ranked.hold_admission());
        let handles: Vec<_> = jobs
            .iter()
            .map(|&us| {
                let ranked = &ranked;
                let legacy = &legacy;
                let gate_open = &gate_open;
                // What `spawn_gated` does on the spawning thread: promise the
                // arrival AND carry the ticket into the worker, so the worker
                // redeems its own promise. Discarding the ticket here made
                // every worker arrive untitled, `pending_arrivals` never
                // reached zero, and this arm silently measured the full
                // arrival bound instead of the hold.
                let arrival = if arm.arrival == ArrivalModel::AdmissionHold {
                    ranked.note_spawn_under_hold()
                } else {
                    None
                };
                scope.spawn(move || {
                    if barriered {
                        gate_open.wait();
                    }
                    match arm.policy {
                        GrantPolicy::LegacyCondvar => {
                            let _permit = legacy.acquire();
                            burn_us(us);
                        }
                        GrantPolicy::RankedButUnlearned => {
                            let _permit = ranked.acquire_ranked(0, None);
                            burn_us(us);
                        }
                        GrantPolicy::RankedByMeasuredCost => {
                            // A converged `LANE_COST_US`: the estimate equals
                            // the hold. Passed explicitly rather than through
                            // the process-global table so the arms cannot
                            // contaminate each other across runs.
                            let _permit = ranked.acquire_ranked(us, arrival);
                            burn_us(us);
                        }
                    }
                })
            })
            .collect();
        // Exactly where production drops it: after the last spawn, before the
        // first join.
        drop(admission);
        let measured_from = if barriered {
            gate_open.wait();
            // The FLOOR arm restarts the clock here, so it charges only the
            // scheduling and nothing for creating the threads; the CHARGED arm
            // keeps the original origin and pays for both.
            if arm.arrival == ArrivalModel::AllPresentFloor {
                Instant::now()
            } else {
                started
            }
        } else {
            started
        };
        for handle in handles {
            handle.join().unwrap();
        }
        duration_micros_u64(measured_from.elapsed())
    });
    makespan
}

fn median(values: &mut [u64]) -> u64 {
    values.sort_unstable();
    values[values.len() / 2]
}

#[test]
#[ignore]
fn reader_gate_schedule_makespan() {
    const RUNS: usize = 31;
    let total_us: u64 = T19_JOB_US.iter().map(|&(_, us)| us).sum();
    let longest_us = T19_JOB_US.iter().map(|&(_, us)| us).max().unwrap_or(0);
    let mut samples: Vec<Vec<u64>> = ARMS.iter().map(|_| Vec::with_capacity(RUNS)).collect();
    // INTERLEAVED, one run of each arm per round, with the order inside the
    // round rotating. Taking one arm's runs and then the next measures
    // whatever else the machine was doing between them, and this lane's
    // predecessors saw that flip a comparison's sign.
    for round in 0..RUNS {
        for offset in 0..ARMS.len() {
            let index = (round + offset) % ARMS.len();
            let us = run_fan_out(ARMS[index]);
            samples[index].push(us);
        }
    }
    println!(
        "reader_gate_schedule: {} jobs, {:.1} ms total work, longest {:.1} ms, {} permits\n  \
         balanced-share floor {:.1} ms, longest-job floor {:.1} ms, so OPT >= {:.1} ms",
        T19_JOB_US.len(),
        total_us as f64 / 1000.0,
        longest_us as f64 / 1000.0,
        BENCH_PERMITS,
        total_us as f64 / BENCH_PERMITS as f64 / 1000.0,
        longest_us as f64 / 1000.0,
        (total_us as f64 / BENCH_PERMITS as f64).max(longest_us as f64) / 1000.0,
    );
    let mut medians = Vec::with_capacity(ARMS.len());
    for (arm, arm_samples) in ARMS.iter().zip(samples.iter_mut()) {
        let lo = *arm_samples.iter().min().unwrap();
        let hi = *arm_samples.iter().max().unwrap();
        let mid = median(arm_samples);
        medians.push(mid);
        println!(
            "reader_gate_schedule/{}: median {:.2} ms  (min {:.2}, max {:.2}, n={})",
            arm.name,
            mid as f64 / 1000.0,
            lo as f64 / 1000.0,
            hi as f64 / 1000.0,
            arm_samples.len(),
        );
    }
    let baseline = medians[0] as f64;
    for (arm, &mid) in ARMS.iter().zip(medians.iter()).skip(1) {
        println!(
            "reader_gate_schedule/delta {}: {:+.2} ms ({:+.1}%) against legacy_condvar",
            arm.name,
            (mid as f64 - baseline) / 1000.0,
            100.0 * (mid as f64 - baseline) / baseline,
        );
    }
}
