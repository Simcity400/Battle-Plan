use super::*;
use crate::{
    carmy_leader,
    crelation::status::bench::{Arena, Counting, RUNS, report},
    cunit,
    cunit_leader,
};
use engine_native_parts::{
    csub_unit_definition as stats,
    modifier_definition,
    modifier_entry,
};

fn division_world(n: usize) -> (Arena, Vec<DivisionSeed>) {
    const KEYS: [u32; 8] = [
        0x10,
        0x54,
        0x100,
        0x200,
        carmy_leader::CANNOT_RETREAT_WHILE_DEFENDING,
        carmy_leader::CANNOT_RETREAT_WHILE_ATTACKING,
        0x300,
        0x400,
    ];
    let divisions = n * 20;
    let mut arena = Arena::new(
        divisions * (0x470 + 0x5d0)
            + n * 2 * (0x1c0 + 0x4f0 + 2 * KEYS.len() * 16 + 64)
            + 0x40_0000,
    );
    let base = arena.base();
    let metadata = arena.alloc((0x400 + 1) * modifier_definition::STRIDE);
    arena.u64(base + modifier_definition::TABLE_RVA.0, metadata as u64);
    let combat = arena.alloc(0x10);
    let groups: Vec<usize> = (0..2 * n)
        .map(|index| {
            let group = arena.alloc(0x1c0);
            let leader = arena.alloc(0x4f0);
            arena.u64(group, arena.rva(corders_group::VTABLE_RVA));
            arena.u32(group + corders_group::HANDLE, corders_group::REFERENCE_TYPE);
            arena.u64(group + corders_group::LEADER, leader as u64);
            arena.u64(leader, arena.rva(carmy_leader::VTABLE_RVA));
            arena.u32(leader + cunit_leader::TYPE, cunit_leader::REFERENCE_TYPE);
            for block in [
                carmy_leader::MOVEMENT_MODIFIER,
                carmy_leader::SEPARATE_HQ_MOVEMENT_MODIFIER,
            ] {
                let entries = arena.alloc(KEYS.len() * modifier_entry::STRIDE);
                arena.vector(leader + block + 0x10, entries, KEYS.len() as u32);
                for (slot, key) in KEYS.into_iter().enumerate() {
                    let entry = entries + slot * modifier_entry::STRIDE;
                    arena.u32(entry, key);
                    let banned =
                        key == carmy_leader::CANNOT_RETREAT_WHILE_DEFENDING && index % 2 == 0;
                    arena.u64(entry + 8, u64::from(banned) * 100_000);
                }
            }
            group
        })
        .collect();
    let seeds = (0..divisions)
        .map(|index| {
            let army = arena.alloc(0x470);
            let block = arena.alloc(0x5d0);
            arena.u64(army + cunit::STAT_BLOCK, block as u64);
            arena.u64(army + ORDERS_GROUP, groups[index / 10] as u64);
            arena.u32(army + cunit::ROSTER_OWNER, (index / 20) as u32 + 1);
            arena.u8(army + EXILED, u8::from(index % 50 == 0));
            if index % 2 == 0 {
                arena.vector(army + cunit::COMBAT_REFERENCE_DATA, combat, 1);
            }
            arena.u64(block, arena.rva(stats::VTABLE_RVA));
            arena.u64(block + stats::MAXIMUM_ORGANISATION, 6_000_000);
            arena.u64(block + stats::MAXIMUM_STRENGTH, 2_500_000);
            arena.u8(
                block + stats::CAN_EXFILTRATE_FROM_COAST,
                u8::from(index % 3 == 0),
            );
            DivisionSeed {
                pointer: army,
                ..DivisionSeed::default()
            }
        })
        .collect();
    (arena, seeds)
}

#[test]
#[ignore = "synthetic capture bench; run with --release -- --ignored --nocapture"]
fn bench_division_move_flags() {
    for n in [1_500, 5_000] {
        let (arena, mut seeds) = division_world(n);
        let base = arena.base();
        let memory = Counting::new();
        report(
            "DivisionStatus seeds+stat block (existing reads)",
            n,
            &memory,
            || {
                capture_division_seeds(&memory, &mut seeds);
                let mut flags = 0;
                for seed in &seeds {
                    let maxima = engine_native_parts::csub_unit_definition::capture_division_maxima(
                        &memory,
                        base,
                        seed.stat_block.unwrap(),
                    )
                    .unwrap();
                    flags += usize::from(maxima.can_exfiltrate_from_coast != 0)
                        + usize::from(seed.exiled != Some(0));
                }
                format!("{} divisions, {flags} exile/exfiltration bits", seeds.len())
            },
        );
        let mut packet = StatusPacket::new(seeds.len()).unwrap();
        let mut rows = vec![RetreatBanRow::default(); seeds.len()];
        let owner = |raw: u32| Ok(Some(raw));
        for (label, held) in [
            (
                "DivisionRetreatBans, first hour (every commander read)",
                false,
            ),
            (
                "DivisionRetreatBans, steady hour (every commander held)",
                true,
            ),
        ] {
            report(label, n, &memory, || {
                let outcome = packet.retreat_bans(
                    &memory,
                    base,
                    &seeds,
                    Outcome::Rows { len: seeds.len() },
                    &owner,
                    &mut rows,
                    &mut |leader| {
                        held.then_some(crate::carmy_leader::RetreatBans {
                            leader,
                            ordinary: 0,
                            alternate: 0,
                            movement: [[None; 6]; 2],
                        })
                    },
                );
                format!("{} banned divisions", outcome.len())
            });
        }
    }
}

fn evict(scratch: &mut [u8]) {
    for (index, byte) in scratch.iter_mut().step_by(64).enumerate() {
        *byte = byte.wrapping_add(index as u8);
    }
    std::hint::black_box(&scratch);
}

#[test]
#[ignore = "synthetic capture bench; run with --release -- --ignored --nocapture"]
fn bench_division_seed_gather_cold() {
    const STRIDE: usize = 0x4000;
    let mut scratch = vec![0_u8; 256 << 20];
    for divisions in [3_300, 5_000] {
        let mut arena = Arena::new(divisions * STRIDE);
        let armies: Vec<usize> = (0..divisions)
            .map(|index| {
                let army = arena.alloc(STRIDE);
                let army = army + (index % 4) * 16;
                arena.u64(army, arena.rva(VTABLE_RVA));
                arena.u64(army + cunit::HANDLE, ((index as u64 + 1) << 32) | 1);
                army
            })
            .collect();
        let memory = Counting::new();
        let mut seeds: Vec<DivisionSeed> = armies
            .iter()
            .map(|&pointer| DivisionSeed {
                pointer,
                ..DivisionSeed::default()
            })
            .collect();
        let mut times = Vec::with_capacity(RUNS);
        for _ in 0..RUNS {
            evict(&mut scratch);
            let started = std::time::Instant::now();
            capture_division_seeds(&memory, &mut seeds);
            times.push(started.elapsed().as_secs_f64() * 1e9 / divisions as f64);
        }
        times.sort_by(f64::total_cmp);
        assert!(seeds.iter().all(|seed| seed.handle.is_some()));
        println!(
            "BENCH | DivisionInputs seed gather, cold | {divisions} divisions | {:.0} ns/division",
            times[RUNS / 2]
        );
    }
}

#[test]
#[ignore = "synthetic capture bench; run with --release -- --ignored --nocapture"]
fn bench_army_icon_inputs_from_seeds() {
    const STRIDE: usize = 0x4000;
    let mut scratch = vec![0_u8; 256 << 20];
    for divisions in [12_000, 30_000] {
        let mut arena = Arena::new(divisions * STRIDE);
        let mut sources = Vec::with_capacity(divisions);
        let armies: Vec<usize> = (0..divisions)
            .map(|index| {
                let army = arena.alloc(STRIDE) + (index % 4) * 16;
                let division = ((index as u64 + 1) << 32) | 1;
                arena.u64(army, arena.rva(VTABLE_RVA));
                arena.u64(army + SECONDARY_BASE, arena.rva(SECONDARY_VTABLE_RVA));
                arena.u64(army + cunit::HANDLE, division);
                arena.u32(army + cunit::ROSTER_OWNER, 1);
                arena.u8(army + SEPARATE_COUNTER, (index % 2) as u8);
                sources.push(crate::cequipment_variant::pool::DivisionSource {
                    pointer: army,
                    division,
                    country: 1,
                    template: 0,
                });
                army
            })
            .collect();
        let base = arena.base();
        let memory = Counting::new();
        let mut seeds: Vec<DivisionSeed> = armies
            .iter()
            .zip(&sources)
            .map(|(&pointer, source)| DivisionSeed {
                pointer,
                division: source.division,
                ..DivisionSeed::default()
            })
            .collect();
        let median = |mut times: Vec<f64>| {
            times.sort_by(f64::total_cmp);
            times[RUNS / 2]
        };
        let timed = |run: &mut dyn FnMut(), scratch: &mut Vec<u8>| {
            let mut times = Vec::with_capacity(RUNS);
            for _ in 0..RUNS {
                evict(scratch);
                let started = std::time::Instant::now();
                run();
                times.push(started.elapsed().as_secs_f64() * 1e9 / divisions as f64);
            }
            median(times)
        };
        const BEFORE: engine_native_memory::memory::RecordLayout<'static> =
            engine_native_memory::memory::RecordLayout::new(&OLD_SEED_FIELDS, OLD_SEED_STRIDE)
                .expect("valid layout");
        let pointers: Vec<usize> = armies.clone();
        let mut bytes = vec![0_u8; 64 * OLD_SEED_STRIDE];
        let gather_before = timed(
            &mut || {
                for chunk in pointers.chunks(64) {
                    memory.copy_records(
                        engine_native_memory::memory::RecordSources::Pointers(chunk),
                        &BEFORE,
                        &mut bytes[..chunk.len() * OLD_SEED_STRIDE],
                    );
                }
            },
            &mut scratch,
        );
        const NOW: engine_native_memory::memory::RecordLayout<'static> =
            engine_native_memory::memory::RecordLayout::new(&DIVISION_SEED_FIELDS, DIVISION_SEED_STRIDE)
                .expect("valid layout");
        let mut now_bytes = vec![0_u8; 64 * DIVISION_SEED_STRIDE];
        let gather_now = timed(
            &mut || {
                for chunk in pointers.chunks(64) {
                    memory.copy_records(
                        engine_native_memory::memory::RecordSources::Pointers(chunk),
                        &NOW,
                        &mut now_bytes[..chunk.len() * DIVISION_SEED_STRIDE],
                    );
                }
            },
            &mut scratch,
        );
        capture_division_seeds(&memory, &mut seeds);
        assert!(seeds.iter().all(|seed| seed.separate_counter.is_some()));
        let mut rows = vec![IconInputsRow::default(); divisions];
        let unseeded: Vec<DivisionSeed> = seeds
            .iter()
            .map(|seed| DivisionSeed {
                separate_counter: None,
                alternate_template: None,
                ..*seed
            })
            .collect();
        let mut icons = |seeds: &[DivisionSeed]| {
            for ((row, source), seed) in rows.iter_mut().zip(&sources).zip(seeds) {
                *row = read_icon_inputs(&memory, base, *source, Some(seed)).unwrap();
            }
        };
        let icons_before = timed(&mut || icons(&unseeded), &mut scratch);
        let icons_now = timed(&mut || icons(&seeds), &mut scratch);
        println!(
            "BENCH | ArmyIconInputs, cold | {divisions} divisions | raw seed gather 38 -> 40 fields {gather_before:.0} -> {gather_now:.0} ns/division; icon pass {icons_before:.0} -> {icons_now:.0} ns/division"
        );
    }
}

const OLD_SEED_FIELDS: [engine_native_memory::memory::RecordField; 38] = {
    let mut fields = [engine_native_memory::memory::RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 0,
    }; 38];
    let mut index = 0;
    while index < 38 {
        fields[index] = DIVISION_SEED_FIELDS[index];
        index += 1;
    }
    fields
};
const OLD_SEED_STRIDE: usize = 341;
