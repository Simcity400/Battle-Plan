use super::*;
use engine_native_parts::{
    cland_border_war_combat,
    cland_border_war_combatant,
    creference_registry as registry,
    cterrain_type,
    reference_bucket as bucket,
};
use crate::{
    cland_combat,
    corders_group,
    crelation::status::bench::{Arena, Counting, report},
    cprovince::definition as definition,
};
use engine_native_memory::msvc_tree_node as node;

#[test]
#[ignore = "synthetic capture bench; run with --release -- --ignored --nocapture"]
fn bench_province_gates_and_border_conflicts() {
    for (n, states, provinces, conflicted) in [
        (1_500, 3_000, 13_000_usize, false),
        (1_500, 3_000, 13_000, true),
        (5_000, 10_000, 40_000, false),
        (5_000, 10_000, 40_000, true),
    ] {
        let mut arena = Arena::new(provinces * 0x340 + states * 0xe0 + 0x40_0000);
        let base = arena.base();
        let game_state = arena.alloc(0x1000);
        let table = arena.alloc((provinces + 1) * 8);
        arena.vector(
            game_state + cgame_state::PROVINCES,
            table,
            provinces as u32 + 1,
        );
        let state_objects: Vec<usize> = (0..states)
            .map(|index| {
                let state = arena.alloc(0xd0);
                arena.u64(state, arena.rva(cstate::VTABLE_RVA));
                arena.u32(state + cstate::ID, index as u32 + 1);
                arena.u32(state + cstate::OWNER, (index % n) as u32 + 1);
                state
            })
            .collect();
        let terrain = arena.alloc(0x90);
        arena.u64(terrain, arena.rva(cterrain_type::VTABLE_RVA));
        arena.u32(terrain + cterrain_type::DATABASE_INDEX, 7);
        let area = arena.alloc(0x10);
        let registry_table = arena.alloc(0x20);
        let entries = arena.alloc(16 * bucket::STRIDE);
        arena.u64(
            base + registry::LOW_ROOTS_RVA.0 + corders_group::REFERENCE_TYPE as usize * 8,
            registry_table as u64,
        );
        arena.u64(registry_table + registry::ENTRIES, entries as u64);
        arena.u32(registry_table + registry::COUNT, 2);
        arena.u32(registry_table + registry::MASK, 7);
        arena.u8(registry_table + registry::OVERFLOW_DISTANCE, 5);
        arena.u8(
            entries + 13 * bucket::STRIDE + bucket::DISTANCE,
            bucket::TERMINAL_DISTANCE,
        );
        let mut handles = Vec::new();
        for id in [57_u32, 58] {
            let handle = (u64::from(id) << 32) | u64::from(corders_group::REFERENCE_TYPE);
            let group = arena.alloc(0x1c0);
            arena.u64(group, arena.rva(corders_group::VTABLE_RVA));
            arena.u64(group + corders_group::HANDLE, handle);
            arena.u64(group + corders_group::BORDER_WAR, 1);
            let hash = handle
                .rotate_left(32)
                .wrapping_mul(registry::HASH_MULTIPLIER);
            let cell =
                entries + ((((hash >> 32) as u32 ^ hash as u32) & 7) as usize) * bucket::STRIDE;
            arena.u8(cell + bucket::DISTANCE, 1);
            arena.u64(cell + bucket::TYPE, handle);
            arena.u64(cell + bucket::OBJECT, group as u64);
            handles.push(handle);
        }
        let mut combat = 0;
        let mut conflicts = 0;
        for id in 1..=provinces {
            let province = arena.alloc(0x200);
            let def = arena.alloc(0xe0);
            arena.u64(table + id * 8, province as u64);
            arena.u64(province, arena.rva(VTABLE_RVA));
            arena.u32(province + ID, id as u32);
            let state = state_objects[(id - 1) * states / provinces];
            let land = id % 4 != 0;
            if land {
                arena.u64(province + STATE, state as u64);
                arena.u64(province + OWNER_AREA, area as u64);
            }
            arena.u32(province + CONTROLLER, ((id % n) + 1) as u32);
            arena.u64(province + DEFINITION, def as u64);
            arena.u32(def + definition::ID, id as u32);
            arena.u64(def + definition::TERRAIN, terrain as u64);
            arena.u8(def + definition::TERRAIN_FLAGS, u8::from(land));
            if land && id % 3 == 0 {
                arena.u32(def + definition::PORT_SEA, (id + 1) as u32);
            }
            if !conflicted || id % 100 != 0 {
                continue;
            }
            if conflicts % 10 == 0 {
                combat = arena.alloc(0x40);
                arena.u64(combat, arena.rva(cland_border_war_combat::VTABLE_RVA));
                for (offset, handle) in [
                    (cland_combat::DEFENDER, handles[0]),
                    (cland_combat::ATTACKER, handles[1]),
                ] {
                    let side = arena.alloc(0x4f0);
                    arena.u64(combat + offset, side as u64);
                    arena.u64(side, arena.rva(cland_border_war_combatant::VTABLE_RVA));
                    arena.u64(side + BORDER_WAR_SIDE_GROUP, handle);
                }
            }
            conflicts += 1;
            let (head, leaf) = (arena.alloc(0x30), arena.alloc(0x30));
            arena.u64(province + BORDER_CONFLICT_TREE, head as u64);
            arena.u64(province + BORDER_CONFLICT_NONZERO_QWORD, 1);
            for link in [node::LEFT, node::PARENT, node::RIGHT] {
                arena.u64(head + link, leaf as u64);
                arena.u64(leaf + link, head as u64);
            }
            arena.u8(head + node::NIL, 1);
            arena.u64(leaf + node::PAYLOAD, combat as u64);
        }
        let memory = Counting::new();
        let mut rows = vec![Row::default(); provinces];
        let mut conflict_rows = vec![ConflictRow::default(); conflicts];
        let mut groups = vec![0; 2 * conflicts];
        let label = if conflicted {
            "Provinces+BorderConflicts (1% in conflict)"
        } else {
            "Provinces (no conflict)"
        };
        report(label, n, &memory, || {
            let (outcome, walk) = walk_with_conflicts_buffer(&memory, base, game_state, engine_native_memory::walk::Buffer::Growing(&mut rows), ConflictBuffers {
            rows: engine_native_memory::walk::Buffer::Growing(&mut conflict_rows),
            groups: engine_native_memory::walk::Buffer::Growing(&mut groups),
        }, engine_native_memory::walk::Buffer::Count, None);
            assert_eq!(outcome, Outcome::Rows { len: provinces });
            assert_eq!(walk.outcome, Outcome::Rows { len: conflicts });
            format!(
                "{provinces} province rows, {conflicts} conflict rows, {} group handles",
                walk.groups_len
            )
        });
        let mut capture = Capture::try_with_capacity(provinces, conflicts, 2 * conflicts).unwrap();
        capture.province_store = Some(Box::new(carry::RowStore::new()));
        let take = Some([1, 1]);
        assert_eq!(
            capture.walk_carrying(&memory, base, game_state, take),
            Outcome::Rows { len: provinces }
        );
        report(&format!("{label}, quiet carry"), n, &memory, || {
            let outcome = capture.walk_carrying(&memory, base, game_state, take);
            assert_eq!(outcome, Outcome::Rows { len: provinces });
            assert_eq!(
                capture.conflict_walk.outcome,
                Outcome::Rows { len: conflicts }
            );
            let counts = capture.province_store.as_ref().unwrap().counts();
            format!("{} carried, {} read", counts.carried, counts.read)
        });
        report(
            &format!("{label}, marked hour (6 provinces), area pointers"),
            n,
            &memory,
            || {
                let (marks, _) = capture.province_store.as_mut().unwrap().marks_mut();
                marks.extend((1..=6).map(|index| index * provinces as u32 / 7));
                let outcome = capture.walk_carrying(&memory, base, game_state, take);
                assert_eq!(outcome, Outcome::Rows { len: provinces });
                capture.refresh_carried_areas(&memory, &|_| true).unwrap();
                let counts = capture.province_store.as_ref().unwrap().counts();
                format!("{} carried, {} read", counts.carried, counts.read)
            },
        );
        let marked: Vec<u32> = (1..=6).map(|index| index * provinces as u32 / 7).collect();
        report(
            &format!("{label}, marked hour (6 provinces), bounded area pointers"),
            n,
            &memory,
            || {
                let (marks, _) = capture.province_store.as_mut().unwrap().marks_mut();
                marks.extend(marked.iter().copied());
                let outcome = capture.walk_carrying(&memory, base, game_state, take);
                assert_eq!(outcome, Outcome::Rows { len: provinces });
                capture
                    .refresh_carried_areas(&memory, &|id| {
                        marked.iter().any(|&mark| mark.abs_diff(id) <= 60)
                    })
                    .unwrap();
                let counts = capture.province_store.as_ref().unwrap().counts();
                format!("{} carried, {} read", counts.carried, counts.read)
            },
        );
    }
}
