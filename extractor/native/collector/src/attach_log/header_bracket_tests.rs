use super::land_combat_side_vector_capture_is_stable;
use std::cell::Cell;

type Header = (usize, u32, u32);

const OPENING_HEADER: Header = (0x1234_0000, 8, 3);

fn opening_body() -> Vec<u8> {
    let mut body = Vec::new();
    for slot in [0xAAAA_0000u64, 0xBBBB_0000, 0xCCCC_0000] {
        body.extend_from_slice(&slot.to_le_bytes());
    }
    body
}

fn torn_body() -> Vec<u8> {
    let mut body = Vec::new();
    for slot in [0xAAAA_0000u64, 0xDDDD_0000, 0xEEEE_0000] {
        body.extend_from_slice(&slot.to_le_bytes());
    }
    body
}

#[test]
fn accepts_a_vector_that_did_not_move_under_the_capture() {
    assert!(land_combat_side_vector_capture_is_stable(
        &OPENING_HEADER,
        &opening_body(),
        || Some(OPENING_HEADER),
        || Some(opening_body()),
    ));
}

#[test]
fn refuses_a_same_length_body_rewrite_under_an_identical_header() {
    assert!(!land_combat_side_vector_capture_is_stable(
        &OPENING_HEADER,
        &opening_body(),
        || Some(OPENING_HEADER),
        || Some(torn_body()),
    ));
}

#[test]
fn refuses_a_changed_header_over_an_unchanged_body() {
    assert!(!land_combat_side_vector_capture_is_stable(
        &OPENING_HEADER,
        &opening_body(),
        || Some((OPENING_HEADER.0, OPENING_HEADER.1, OPENING_HEADER.2 + 1)),
        || Some(opening_body()),
    ));
}

#[test]
fn refuses_an_unreadable_closing_body() {
    assert!(!land_combat_side_vector_capture_is_stable(
        &OPENING_HEADER,
        &opening_body(),
        || Some(OPENING_HEADER),
        || None,
    ));
}

#[test]
fn refuses_an_unreadable_closing_header() {
    assert!(!land_combat_side_vector_capture_is_stable(
        &OPENING_HEADER,
        &opening_body(),
        || None::<Header>,
        || Some(opening_body()),
    ));
}

#[test]
fn refuses_a_body_that_only_matches_on_a_second_look() {
    let body_reads = Cell::new(0u32);
    assert!(!land_combat_side_vector_capture_is_stable(
        &OPENING_HEADER,
        &opening_body(),
        || Some(OPENING_HEADER),
        || {
            body_reads.set(body_reads.get() + 1);
            if body_reads.get() == 1 {
                Some(torn_body())
            } else {
                Some(opening_body())
            }
        },
    ));
}
