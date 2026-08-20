//! Tests for the D48V grammar. In their own file only because the module they
//! test is already long; they are its `mod tests` and nothing else includes them.

use super::*;

const MAX_STATE_ID: u32 = (i32::MAX as u32) - 1;
const MAX_PROVINCE_ID: u32 = (i32::MAX as u32) - 1;

fn census() -> CoverageCensus {
    CoverageCensus {
        census_flags: CENSUS_FLAG_STATE_ROOTS_AUTHORITATIVE
            | CENSUS_FLAG_PROVINCE_FAILURES_AUTHORITATIVE,
        scanned_states: (1..=1_081).collect(),
        building_incomplete_states: vec![7, 900],
        resource_incomplete_states: vec![12],
        building_incomplete_provinces: vec![4_000, 9_999],
    }
}

fn encode(coverage: &CoverageCensus) -> Vec<u8> {
    let mut out = Vec::new();
    encode_coverage_trailer(&mut out, coverage).expect("a well-formed census encodes");
    out
}

fn decode(bytes: &[u8], coverage: &CoverageCensus) -> Result<CoverageCensus, String> {
    decode_coverage_trailer(
        bytes,
        coverage.scanned_states.len() as u32,
        // The province root count only has to be at least the failure set's
        // cardinality; the frame's own header carries the real one.
        10_000,
        MAX_STATE_ID,
        MAX_PROVINCE_ID,
    )
}

// kills: any asymmetry between the encoder and the decoder on the ordinary
// shape, and the size claim the whole design rests on (the state partition is
// ~64 bytes a frame, not a per-hour tax).
#[test]
fn the_ordinary_census_round_trips_and_stays_small() {
    let coverage = census();
    let bytes = encode(&coverage);
    assert_eq!(decode(&bytes, &coverage).expect("decodes"), coverage);
    assert!(
        bytes.len() < 80,
        "the state partition must stay tiny, got {} bytes",
        bytes.len()
    );
}

// kills: a healthy frame having no legal spelling. Three of the four sets are
// empty when nothing failed, which is the COMMON case, not an edge one.
#[test]
fn a_healthy_frame_spells_three_empty_sets() {
    let coverage = CoverageCensus {
        census_flags: CENSUS_KNOWN_FLAGS,
        scanned_states: (1..=1_081).collect(),
        ..Default::default()
    };
    let bytes = encode(&coverage);
    assert_eq!(decode(&bytes, &coverage).expect("decodes"), coverage);
}

// kills: appending D48V without teaching the tail split. RLC1's extent must come
// from its own header; if it is taken to be "the rest of the tail", every frame
// carrying a coverage trailer is rejected by the parser that does so.
#[test]
fn the_tail_splits_rlc1_by_its_computed_extent() {
    // A minimal RLC1: 24-byte header, zero rows, zero routes.
    let mut tail = Vec::new();
    tail.extend_from_slice(&RAILWAY_TRAILER_MAGIC);
    tail.extend_from_slice(&1u16.to_le_bytes());
    tail.extend_from_slice(&24u16.to_le_bytes());
    tail.extend_from_slice(&0u32.to_le_bytes()); // row_count
    tail.extend_from_slice(&0u32.to_le_bytes()); // route_count
    tail.extend_from_slice(&64u16.to_le_bytes());
    tail.extend_from_slice(&0u16.to_le_bytes());
    tail.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(tail.len(), 24);
    let coverage = census();
    let trailer = encode(&coverage);
    tail.extend_from_slice(&trailer);

    let (railway, rest) = split_frame_tail(&tail).expect("splits");
    assert_eq!(railway.expect("RLC1 present").len(), 24);
    assert_eq!(rest, trailer.as_slice());
    assert_eq!(decode(rest, &coverage).expect("decodes"), coverage);

    // A tail with no RLC1 at all is the whole D48V, and still legal.
    let (railway, rest) = split_frame_tail(&trailer).expect("splits");
    assert!(railway.is_none());
    assert_eq!(rest, trailer.as_slice());
}

// kills: letting bytes hide between the two trailers. RLC1's length check went
// from `!=` to `<=` so that D48V may follow it; the exact-total-length rule has
// to move to D48V or the change opens a smuggling slot.
#[test]
fn nothing_can_hide_between_the_two_trailers() {
    let coverage = census();
    let trailer = encode(&coverage);
    let mut smuggled = trailer.clone();
    smuggled.push(0xAA);
    assert!(decode(&smuggled, &coverage)
        .expect_err("a byte after the census is refused")
        .contains("length"));

    // And an empty tail cannot evade the mandatory trailer.
    assert!(decode(&[], &coverage)
        .expect_err("an absent D48V is refused")
        .contains("missing D48V"));
}

// kills: accepting a census that contradicts the frame header it travels with,
// or one whose failure sets name states the walk never scanned. Either would let
// a frame claim coverage it does not have.
#[test]
fn a_census_that_contradicts_its_own_frame_is_refused() {
    let coverage = census();
    let bytes = encode(&coverage);
    assert!(decode_coverage_trailer(
        &bytes,
        coverage.scanned_states.len() as u32 + 1,
        10_000,
        MAX_STATE_ID,
        MAX_PROVINCE_ID
    )
    .expect_err("scanned_states must equal observed_state_roots")
    .contains("cardinality"));

    let outside = CoverageCensus {
        building_incomplete_states: vec![5_000],
        ..census()
    };
    let mut out = Vec::new();
    assert!(encode_coverage_trailer(&mut out, &outside)
        .expect_err("the ENCODER refuses a failure state outside the scanned set")
        .contains("outside scanned_states"));
}

// kills: admitting a flag bit no decoder understands. An unknown bit means a
// producer is asserting something this reader cannot evaluate, and silently
// ignoring it publishes a claim nobody checked.
#[test]
fn unknown_flag_bits_and_a_dirty_reserved_field_are_refused() {
    let coverage = census();
    let good = encode(&coverage);
    let mut unknown = good.clone();
    unknown[11] = 0x80;
    assert!(decode(&unknown, &coverage)
        .expect_err("an unknown flag bit is refused")
        .contains("unknown census flag bits"));

    let mut reserved = good.clone();
    reserved[12] = 1;
    assert!(decode(&reserved, &coverage)
        .expect_err("a dirty reserved field is refused")
        .contains("reserved"));

    let mut magic = good.clone();
    magic[0] = b'X';
    assert!(decode(&magic, &coverage)
        .expect_err("a foreign trailer magic is refused")
        .contains("magic"));

    assert!(decode(&good, &coverage).is_ok(), "the unmutated bytes decode");
}

// kills: treating `sorted_subset` as a general set helper. Its contract is
// strictly-increasing inputs, and this pins both the guarantee it gives and the
// documented limit it does not.
#[test]
fn sorted_subset_holds_for_strictly_increasing_inputs() {
    assert!(sorted_subset(&[], &[]));
    assert!(sorted_subset(&[], &[1, 2, 3]));
    assert!(sorted_subset(&[1, 3], &[1, 2, 3]));
    assert!(!sorted_subset(&[4], &[1, 2, 3]));
    assert!(!sorted_subset(&[0], &[1, 2, 3]));
    assert!(!sorted_subset(&[1, 2, 3], &[]));
    // The documented limit: duplicates are outside the contract, and the
    // linear merge consumes the matching element.
    assert!(!sorted_subset(&[1, 1], &[1]));
}
