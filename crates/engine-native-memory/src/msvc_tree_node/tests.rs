use super::*;
use crate::memory::SliceMemory;

const HEAD: usize = 0x100;

fn put(bytes: &mut [u8], node: usize, offset: usize, value: usize) {
    bytes[node + offset..node + offset + 8].copy_from_slice(&(value as u64).to_le_bytes());
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x700];
    bytes[HEAD + NIL] = 1;
    for (node, left, parent, right, value) in [
        (HEAD, 0x200, 0x400, 0x600, 0),
        (0x200, HEAD, 0x400, 0x300, 1),
        (0x300, HEAD, 0x200, HEAD, 2),
        (0x400, 0x200, HEAD, 0x600, 3),
        (0x500, HEAD, 0x600, HEAD, 4),
        (0x600, 0x500, 0x400, HEAD, 5),
    ] {
        for (offset, value) in [
            (LEFT, left),
            (PARENT, parent),
            (RIGHT, right),
            (PAYLOAD, value),
        ] {
            put(&mut bytes, node, offset, value);
        }
    }
    bytes
}

fn walk(bytes: &[u8], size: u64) -> Result<Vec<u64>, SetError> {
    let mut values = Vec::new();
    walk_set(
        &SliceMemory { base: 0, bytes },
        HEAD,
        size,
        |e| e,
        |value| {
            values.push(value);
            Ok(())
        },
    )?;
    Ok(values)
}

#[test]
fn set_successors_descend_left_and_climb_through_right_children() {
    let bytes = fixture();
    assert_eq!(walk(&bytes, 5), Ok(vec![1, 2, 3, 4, 5]));
    assert_eq!(walk(&bytes, 4), Err(SetError::ExcessNodes));
    assert_eq!(walk(&bytes, 6), Err(SetError::MissingNodes));
    assert_eq!(walk(&bytes, u64::MAX), Err(SetError::BudgetOverflow));
}

#[test]
fn set_malformed_child_and_parent_links_are_bounded() {
    let mut bytes = fixture();
    put(&mut bytes, 0x300, LEFT, 0x300);
    assert_eq!(walk(&bytes, 5), Err(SetError::Cycle));
    let mut bytes = fixture();
    put(&mut bytes, 0x300, PARENT, 0x300);
    put(&mut bytes, 0x300, RIGHT, 0x300);
    assert_eq!(walk(&bytes, 5), Err(SetError::ExcessNodes));
}

#[test]
fn set_end_marker_must_be_this_trees_own_head() {
    const FOREIGN: usize = 0x680;
    for (node, offset) in [(0x300, RIGHT), (0x500, LEFT)] {
        let mut bytes = fixture();
        bytes[FOREIGN + NIL] = 1;
        put(&mut bytes, node, offset, FOREIGN);
        assert_eq!(walk(&bytes, 5), Err(SetError::ForeignSentinel));
    }
}
