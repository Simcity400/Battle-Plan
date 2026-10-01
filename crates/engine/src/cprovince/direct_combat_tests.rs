use super::*;
use crate::{cland_combat, cnaval_combat, memory::SliceMemory};

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

/// Province 1 has no combats; province 2 holds naval, land, naval in that order.
fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x5000];
    array(&mut bytes, STATE + cgame_state::PROVINCES, SLOTS, 3);
    for (index, province) in PROVINCES.into_iter().enumerate() {
        put(
            &mut bytes,
            SLOTS + 8 + index * 8,
            &(province as u64).to_le_bytes(),
        );
        put(&mut bytes, province, &(VTABLE_RVA as u64).to_le_bytes());
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
            &(cnaval_combat::VTABLE_RVA as u64).to_le_bytes(),
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
        &(cland_combat::VTABLE_RVA as u64).to_le_bytes(),
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
        DirectCombatArenas {
            handles: &mut handles,
            kinds: &mut kinds,
        },
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
    // A short arena truncates the row and still reports the demand.
    let (short, ..) = walk(&bytes, 2, 2);
    assert_eq!(
        short.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert_eq!(short.required, 3);
    // A border war keeps its own class.
    let mut border = bytes.clone();
    put(
        &mut border,
        LAND,
        &(crate::cland_border_war_combat::VTABLE_RVA as u64).to_le_bytes(),
    );
    assert_eq!(walk(&border, 2, 4).3[1], COMBAT_BORDER_WAR);
    // An unknown combat class refuses the table.
    let mut unknown = bytes.clone();
    put(&mut unknown, LAND, &1u64.to_le_bytes());
    assert!(matches!(
        walk(&unknown, 2, 4).0.outcome,
        Outcome::Unavailable { .. }
    ));
}
