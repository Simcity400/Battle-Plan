use super::*;
use crate::{
    carmy_leader,
    crelation_status::bench::{Arena, Counting, report},
    csub_unit_definition as stats, cunit, cunit_leader, modifier_definition, modifier_entry,
};

/// 20 divisions per country in two orders groups, half of them in a combat;
/// every other group's commander bans retreat while defending.
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
    arena.u64(base + modifier_definition::TABLE_RVA, metadata as u64);
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
                // The CModifier's value accumulator is at +0x10 (C0AB20 callers).
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
        // The shared seed gather (now with the exile byte) and the stat
        // block copy (now with the exfiltration byte): no added calls.
        report(
            "DivisionStatus seeds+stat block (existing reads)",
            n,
            &memory,
            || {
                capture_division_seeds(&memory, &mut seeds);
                let mut flags = 0;
                for seed in &seeds {
                    let maxima = crate::csub_unit_definition::capture_division_maxima(
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
        // DivisionRetreatBans: every division of the read owners, a
        // daily slice in a steady hour, every owner in the first.
        let mut packet = StatusPacket::new(seeds.len());
        let mut rows = vec![RetreatBanRow::default(); seeds.len()];
        for (label, slice) in [
            ("DivisionRetreatBans, first hour (every owner)", false),
            ("DivisionRetreatBans, steady hour (one slice)", true),
        ] {
            let owner = move |raw: u32| Ok((!slice || raw % 24 == 7).then_some(raw));
            report(label, n, &memory, || {
                let outcome = packet.retreat_bans(
                    &memory,
                    base,
                    &seeds,
                    Outcome::Rows { len: seeds.len() },
                    &owner,
                    &mut rows,
                );
                format!("{} banned divisions", outcome.len())
            });
        }
    }
}
