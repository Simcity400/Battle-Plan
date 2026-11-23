use super::*;
use crate::{cland_combat, cnaval_combat};
use engine_native_memory::memory::SliceMemory;

const STATE: usize = 0x1000;
const SLOTS: usize = 0x1800;
const PROVINCES: [usize; 2] = [0x2000, 0x2400];
const VECTOR: usize = 0x3000;
const NAVAL: [usize; 2] = [0x4000, 0x4100];
const LAND: usize = 0x4200;

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}

fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    put(bytes, at, &(data as u64).to_le_bytes());
    put(bytes, at + 8, &count.to_le_bytes());
    put(bytes, at + 12, &count.to_le_bytes());
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x5000];
    array(&mut bytes, STATE + cgame_state::PROVINCES, SLOTS, 3);
    for (index, province) in PROVINCES.into_iter().enumerate() {
        put(
            &mut bytes,
            SLOTS + 8 + index * 8,
            &(province as u64).to_le_bytes(),
        );
        put(&mut bytes, province, &(VTABLE_RVA.0 as u64).to_le_bytes());
        put(&mut bytes, province + ID, &(index as u32 + 1).to_le_bytes());
    }
    array(&mut bytes, PROVINCES[1] + COMBATS, VECTOR, 3);
    for (index, combat) in [NAVAL[0], LAND, NAVAL[1]].into_iter().enumerate() {
        put(
            &mut bytes,
            VECTOR + index * 8,
            &(combat as u64).to_le_bytes(),
        );
    }
    for (index, combat) in NAVAL.into_iter().enumerate() {
        put(
            &mut bytes,
            combat,
            &(cnaval_combat::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            combat + cnaval_combat::HANDLE,
            &((index as u64 + 7) << 32 | 4713).to_le_bytes(),
        );
    }
    put(
        &mut bytes,
        LAND,
        &(cland_combat::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        LAND + cland_combat::HANDLE,
        &(9u64 << 32 | 62).to_le_bytes(),
    );
    bytes
}

fn walk(
    bytes: &[u8],
    rows: usize,
    arena: usize,
) -> (DirectCombatsWalk, Vec<DirectCombatsRow>, Vec<u64>, Vec<u8>) {
    let memory = SliceMemory { base: 0, bytes };
    let mut output = vec![DirectCombatsRow::default(); rows];
    let (mut handles, mut kinds) = (vec![0; arena], vec![0; arena]);
    let result = walk_direct_combats(
        &memory,
        0,
        STATE,
        &mut output,
        &mut handles,
        &mut kinds,
        None,
    );
    (result, output, handles, kinds)
}

#[test]
fn every_combat_keeps_its_vector_place_and_class() {
    let bytes = fixture();
    let (result, rows, handles, kinds) = walk(&bytes, 2, 4);
    assert_eq!(result.outcome, Outcome::Rows { len: 1 });
    assert_eq!(result.required, 3);
    assert_eq!(
        rows[0],
        DirectCombatsRow {
            province: 2,
            combats: [0, 3]
        }
    );
    assert_eq!(handles[..3], [7 << 32 | 4713, 9 << 32 | 62, 8 << 32 | 4713]);
    assert_eq!(kinds[..3], [COMBAT_NAVAL, COMBAT_LAND, COMBAT_NAVAL]);
    let (short, _, handles, _) = walk(&bytes, 2, 2);
    assert_eq!(short.outcome, Outcome::Rows { len: 1 });
    assert_eq!(short.required, 3);
    assert_eq!(handles[..3], [7 << 32 | 4713, 9 << 32 | 62, 8 << 32 | 4713]);
    let mut border = bytes.clone();
    put(
        &mut border,
        LAND,
        &(engine_native_parts::cland_border_war_combat::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    assert_eq!(walk(&border, 2, 4).3[1], COMBAT_BORDER_WAR);
    let mut unknown = bytes.clone();
    put(&mut unknown, LAND, &1u64.to_le_bytes());
    assert!(matches!(
        walk(&unknown, 2, 4).0.outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn growing_province_combat_rows_and_parallel_arenas_follow_mod_demand() {
    const COUNT: usize = 5_000;
    let mut rows = Vec::new();
    let mut handles = Vec::new();
    let mut kinds = Vec::new();
    for entries in [3_usize, 37] {
        let mut bytes = fixture();
        let slots = bytes.len();
        let provinces = slots + (COUNT + 1) * 8;
        let combat_slots = provinces + COUNT * 0x200;
        bytes.resize(combat_slots + entries * 8, 0);
        array(
            &mut bytes,
            STATE + cgame_state::PROVINCES,
            slots,
            COUNT as u32 + 1,
        );
        for index in 0..COUNT {
            let pointer = provinces + index * 0x200;
            put(
                &mut bytes,
                slots + (index + 1) * 8,
                &(pointer as u64).to_le_bytes(),
            );
            put(&mut bytes, pointer, &(VTABLE_RVA.0 as u64).to_le_bytes());
            put(&mut bytes, pointer + ID, &(index as u32 + 1).to_le_bytes());
            array(&mut bytes, pointer + COMBATS, combat_slots, entries as u32);
        }
        for index in 0..entries {
            put(
                &mut bytes,
                combat_slots + index * 8,
                &(NAVAL[0] as u64).to_le_bytes(),
            );
        }
        let captured = walk_direct_combats(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            STATE,
            &mut rows,
            &mut handles,
            &mut kinds,
            None,
        );
        assert_eq!(captured.outcome, Outcome::Rows { len: COUNT });
        assert_eq!(captured.required, COUNT * entries);
        for (index, row) in rows[..COUNT].iter().enumerate() {
            assert_eq!(row.combats, [index * entries, entries]);
            assert_eq!(row.province, index as u32 + 1);
        }
        assert!(
            kinds[..captured.required]
                .iter()
                .all(|&kind| kind == COMBAT_NAVAL)
        );
        assert!(
            handles[..captured.required]
                .iter()
                .all(|&handle| handle == 7 << 32 | 4713)
        );
    }
}

fn listed(bytes: &[u8], ids: &[u32]) -> (DirectCombatsWalk, Vec<DirectCombatsRow>, Vec<u64>, Vec<u8>) {
    let (mut rows, mut handles, mut kinds) = (Vec::new(), Vec::new(), Vec::new());
    let result = walk_direct_combats(
        &SliceMemory { base: 0, bytes },
        0,
        STATE,
        &mut rows,
        &mut handles,
        &mut kinds,
        Some(ids),
    );
    (result, rows, handles, kinds)
}

#[test]
fn a_listed_walk_reads_only_its_provinces_as_the_whole_walk_does() {
    let bytes = fixture();
    let (whole, rows, handles, kinds) = walk(&bytes, 2, 4);
    let (some, listed_rows, listed_handles, listed_kinds) = listed(&bytes, &[1, 2]);
    assert_eq!((some.outcome, some.required), (whole.outcome, whole.required));
    assert_eq!(listed_rows[..1], rows[..1]);
    assert_eq!(listed_handles[..3], handles[..3]);
    assert_eq!(listed_kinds[..3], kinds[..3]);
    let mut unread = bytes.clone();
    put(&mut unread, PROVINCES[1], &0u64.to_le_bytes());
    assert_eq!(listed(&unread, &[1]).0.outcome, Outcome::Rows { len: 0 });
    assert_eq!(listed(&bytes, &[]).0.outcome, Outcome::Rows { len: 0 });
    assert!(matches!(
        listed(&bytes, &[3]).0.outcome,
        Outcome::Unavailable { .. }
    ));
}
