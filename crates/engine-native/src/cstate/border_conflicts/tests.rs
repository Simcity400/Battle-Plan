use super::*;
use crate::{cgame_state, cstate};
use engine_native_memory::{memory::SliceMemory, walk::Outcome};
use std::cell::RefCell;

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}
fn fixture(active: u8, members: u64) -> Vec<u8> {
    let mut bytes = vec![0; 0x9000];
    for (at, value) in [
        (0x1000 + cgame_state::STATES, 0x1800),
        (0x1808, 0x2000),
        (0x2000, cstate::VTABLE_RVA.0),
        (0x2000 + cstate::BORDER_CONFLICT_TREE, 0x3000),
    ] {
        put(&mut bytes, at, &(value as u64).to_le_bytes());
    }
    put(
        &mut bytes,
        0x1000 + cgame_state::STATES + 8,
        &2_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x1000 + cgame_state::STATES + 12,
        &2_u32.to_le_bytes(),
    );
    put(&mut bytes, 0x2000 + cstate::ID, &1_u32.to_le_bytes());
    bytes[0x2000 + cstate::DEMILITARIZED] = 255;
    bytes[0x2000 + cstate::BORDER_CONFLICT_ACTIVE] = active;
    put(
        &mut bytes,
        0x2000 + cstate::BORDER_CONFLICT_SIZE,
        &members.to_le_bytes(),
    );
    bytes[0x3000 + node::NIL] = 1;
    let first = if members == 0 { 0x3000_u64 } else { 0x3100 };
    for offset in [node::LEFT, node::PARENT, node::RIGHT] {
        put(&mut bytes, 0x3000 + offset, &first.to_le_bytes());
    }
    for offset in [node::LEFT, node::PARENT, node::RIGHT] {
        put(&mut bytes, 0x3100 + offset, &0x3000_u64.to_le_bytes());
    }
    put(
        &mut bytes,
        0x3100 + node::PAYLOAD,
        &0x4000_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x4000,
        &(cland_border_war_combat::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    for (offset, side, country) in [
        (cland_combat::ATTACKER, 0x5000, i32::MIN),
        (cland_combat::DEFENDER, 0x6000, 0),
    ] {
        put(&mut bytes, 0x4000 + offset, &(side as u64).to_le_bytes());
        put(
            &mut bytes,
            side,
            &(cland_border_war_combatant::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            side + cland_border_war_combatant::COUNTRY,
            &country.to_le_bytes(),
        );
    }
    bytes
}
struct Trace<'a> {
    bytes: &'a [u8],
    spans: RefCell<Vec<(usize, usize)>>,
}
impl Memory for Trace<'_> {
    fn copy(&self, at: usize, output: &mut [u8]) -> bool {
        self.spans.borrow_mut().push((at, output.len()));
        SliceMemory {
            base: 0,
            bytes: self.bytes,
        }
        .copy(at, output)
    }
}
#[test]
fn inactive_flags_read_adjacent_bytes_and_never_follow_bad_tree() {
    let mut bytes = fixture(0, u64::MAX);
    put(
        &mut bytes,
        0x2000 + cstate::BORDER_CONFLICT_TREE,
        &u64::MAX.to_le_bytes(),
    );
    let memory = Trace {
        bytes: &bytes,
        spans: RefCell::default(),
    };
    let mut rows = [cstate::FlagRow::default()];
    let outcome = cstate::read_flags(
        &memory,
        0,
        0x1000,
        &[cstate::Source {
            pointer: 0x2000,
            id: 1,
        }],
        Outcome::Rows { len: 1 },
        &mut rows,
    );
    assert_eq!(outcome, Outcome::Rows { len: 1 });
    assert_eq!(rows[0].demilitarized, 255);
    assert!(rows[0].border_conflict_countries.is_empty());
    let spans = memory.spans.borrow();
    assert!(spans.contains(&(0x2000 + cstate::DEMILITARIZED, 2)));
    assert!(
        !spans
            .iter()
            .any(|&(at, _)| at == 0x2000 + cstate::BORDER_CONFLICT_TREE
                || at == 0x2000 + cstate::BORDER_CONFLICT_SIZE)
    );
}
#[test]
fn active_pairs_preserve_attacker_defender_signed_values_and_empty_tree() {
    for members in [0, 1] {
        let bytes = fixture(255, members);
        let mut pairs = Vec::new();
        capture(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            0x2000,
            &mut pairs,
        )
        .unwrap();
        assert_eq!(
            pairs,
            if members == 0 {
                vec![]
            } else {
                vec![[i32::MIN, 0]]
            }
        );
    }
}
#[test]
fn active_tree_size_and_class_faults_are_named_failures() {
    let bytes = fixture(1, u64::MAX);
    let mut output = Vec::new();
    assert!(
        capture(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0,
            0x2000,
            &mut output
        )
        .unwrap_err()
        .contains("traversal budget overflow")
    );
    assert_eq!(output.capacity(), 0);
    let mut bytes = fixture(1, 2);
    assert!(
        capture(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0,
            0x2000,
            &mut Vec::new()
        )
        .unwrap_err()
        .contains("shorter")
    );
    put(
        &mut bytes,
        0x2000 + cstate::BORDER_CONFLICT_SIZE,
        &1_u64.to_le_bytes(),
    );
    put(&mut bytes, 0x4000, &0_u64.to_le_bytes());
    assert!(
        capture(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0,
            0x2000,
            &mut Vec::new()
        )
        .unwrap_err()
        .contains("CLandBorderWarCombat identity")
    );
}
