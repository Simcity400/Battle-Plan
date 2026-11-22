use super::*;
use collector_division_parts::{army_stat_changes::Watches, attack_stats::ArmyStatStore};
use engine_native::memory::LiveMemory;
use std::{hint::black_box, time::Instant};

const RUNS: usize = 25;

struct Arena {
    bytes: Vec<u8>,
    next: usize,
}

impl Arena {
    fn new(divisions: usize) -> Self {
        Self {
            bytes: vec![0; divisions * 256],
            next: 0,
        }
    }

    fn base(&self) -> usize {
        self.bytes.as_ptr() as usize
    }

    fn alloc(&mut self, len: usize) -> usize {
        let at = self.next.next_multiple_of(16);
        self.next = at + len;
        assert!(self.next <= self.bytes.len());
        self.base() + at
    }

    fn put(&mut self, address: usize, bytes: &[u8]) {
        let at = address - self.base();
        self.bytes[at..at + bytes.len()].copy_from_slice(bytes);
    }
}

fn world(countries: usize, divisions: usize) -> (Arena, Vec<DivisionSeed>) {
    let mut arena = Arena::new(divisions);
    let base = arena.base();
    let seeds = (0..divisions)
        .map(|index| {
            let pointer = arena.alloc(8);
            let name = arena.alloc(cname_group_member::INPUT_SPAN);
            let mut span = [0; cname_group_member::INPUT_SPAN];
            span[..8]
                .copy_from_slice(&((base + cname_group_member::VTABLE_RVA.0) as u64).to_le_bytes());
            let order = cname_group_member::ORDER;
            span[order..order + 4].copy_from_slice(&(index as i32).to_le_bytes());
            let custom = cname_group_member::OVERRIDE_NAME;
            span[custom..custom + 4].copy_from_slice(b"Unit");
            span[custom + 16..custom + 24].copy_from_slice(&4_u64.to_le_bytes());
            span[custom + 24..custom + 32].copy_from_slice(&15_u64.to_le_bytes());
            let cache = cname_group_member::CACHED_NAME;
            if index.is_multiple_of(4) {
                span[cache..cache + 9].copy_from_slice(b"Unit name");
                span[cache + 16..cache + 24].copy_from_slice(&9_u64.to_le_bytes());
                span[cache + 24..cache + 32].copy_from_slice(&15_u64.to_le_bytes());
            } else {
                let text = arena.alloc(32);
                arena.put(text, b"Synthetic division name");
                span[cache..cache + 8].copy_from_slice(&(text as u64).to_le_bytes());
                span[cache + 16..cache + 24].copy_from_slice(&23_u64.to_le_bytes());
                span[cache + 24..cache + 32].copy_from_slice(&31_u64.to_le_bytes());
            }
            arena.put(name, &span);
            DivisionSeed {
                pointer,
                division: ((index as u64 + 1) << 32) | 9,
                roster_owner: Some((index % countries + 1) as u32),
                country_ordinal: Some((index / countries) as u32),
                name_member: Some(name),
                ..DivisionSeed::default()
            }
        })
        .collect();
    (arena, seeds)
}

fn same_rows(actual: &Captured, fresh: &Captured) {
    assert_eq!(actual.naming.len, fresh.naming.len);
    for (actual, fresh) in actual.naming.first[..actual.naming.len]
        .iter()
        .zip(&fresh.naming.first)
    {
        assert_eq!(
            (
                actual.division,
                actual.country_ordinal,
                actual.inputs,
                actual.failure,
                actual.cached_name,
                actual.cached_failure
            ),
            (
                fresh.division,
                fresh.country_ordinal,
                fresh.inputs,
                fresh.failure,
                fresh.cached_name,
                fresh.cached_failure
            )
        );
    }
    assert_eq!(
        &actual.names.first[..actual.names.len],
        &fresh.names.first[..fresh.names.len]
    );
    assert_eq!(
        &actual.cached_names.first[..actual.cached_names.len],
        &fresh.cached_names.first[..fresh.cached_names.len]
    );
}

#[test]
#[ignore = "synthetic capture bench; run with --release -- --ignored --nocapture"]
fn bench_division_status_naming() {
    if cfg!(debug_assertions) {
        panic!("run with --release");
    }
    for countries in [1_500, 5_000] {
        for divisions in [20_000, 60_000] {
            let (arena, seeds) = world(countries, divisions);
            let base = arena.base();
            let memory = unsafe { LiveMemory::new() };
            let capacities = Capacities {
                status: divisions,
                name_bytes: divisions * 4,
                cached_name_bytes: divisions * 23,
                ..Capacities::default()
            };
            let mut fresh = Captured::with_capacities(&capacities);
            fresh.capture_naming(&memory, base, &seeds);
            assert_eq!(fresh.naming.len, divisions);
            assert!(
                fresh.naming.first[..divisions]
                    .iter()
                    .all(|row| row.inputs.is_some()
                        && row.cached_name.is_some()
                        && row.failure.is_none()
                        && row.cached_failure.is_none())
            );
            for carried in [false, true] {
                let mut capture = Captured::with_capacities(&capacities);
                let mut store = ArmyStatStore::new();
                let watches = Watches::enabled(2 * divisions);
                let mut samples = Vec::with_capacity(RUNS);
                for run in 0..RUNS + 3 {
                    if carried {
                        for seed in seeds.iter().step_by(769) {
                            drop(watches.notify(seed.name_member.unwrap(), 1));
                        }
                    }
                    capture.attack.certify(
                        &memory,
                        base,
                        &seeds,
                        Outcome::Rows { len: divisions },
                        carried.then_some(&mut store),
                        &watches,
                    );
                    let begin = Instant::now();
                    capture.capture_naming_holding(
                        &memory,
                        base,
                        &seeds,
                        carried.then_some(&mut store),
                        &watches,
                    );
                    let elapsed = begin.elapsed();
                    black_box(&capture);
                    if carried {
                        store.hold(capture.attack.epoch());
                    }
                    if run >= 3 {
                        samples.push(elapsed.as_nanos() as f64 / divisions as f64);
                    }
                }
                same_rows(&capture, &fresh);
                assert!(!watches.poisoned());
                samples.sort_by(f64::total_cmp);
                let form = if carried { "carried" } else { "fresh" };
                let reused = capture.attack.status_counts.names_reused;
                println!(
                    "DIVISION_STATUS_BENCH path=naming_{form} countries={countries} divisions={divisions} ns_per_division={:.1} carried_percent={:.3}",
                    samples[RUNS / 2],
                    reused as f64 * 100.0 / divisions as f64
                );
            }
        }
    }
}
