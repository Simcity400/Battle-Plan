//! `cargo test -p collector --release -- --ignored --nocapture reader_gate_schedule`.

use super::*;

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

const BENCH_PERMITS: usize = 12;

/// replaced: a free-list plus one condvar, so a released lane is raced for by
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
    LegacyCondvar,
    RankedButUnlearned,
    RankedByMeasuredCost,
}

/// When the worker calls `acquire`. This is the variable the grant policy
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ArrivalModel {
    AsSpawned,
    /// with the clock starting at the release. This is the FLOOR the ranking
    AllPresentFloor,
    AllPresentCharged,
    /// `acquire` yet be in the queue -- and that residual is exactly what
    AdmissionHold,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Arm {
    policy: GrantPolicy,
    arrival: ArrivalModel,
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
        let admission =
            (arm.arrival == ArrivalModel::AdmissionHold).then(|| ranked.hold_admission());
        let handles: Vec<_> = jobs
            .iter()
            .map(|&us| {
                let ranked = &ranked;
                let legacy = &legacy;
                let gate_open = &gate_open;
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
                            let _permit = ranked.acquire_ranked(us, arrival);
                            burn_us(us);
                        }
                    }
                })
            })
            .collect();
        drop(admission);
        let measured_from = if barriered {
            gate_open.wait();
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
