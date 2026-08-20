//! Tests for `id_set_codec`. In their own file only because the module they
//! test is already long; they are its `mod tests` and nothing else includes them.

use super::*;

/// A caller bound above any set these tests build, so the caller's own limit is
/// not what is under test here. The codec still clamps to `MAX_DECODABLE_IDS`;
/// `a_compressed_block_cannot_ask_for_billions_of_ids` pins both halves.
const NO_LIMIT: usize = 1 << 24;

fn round_trip(ids: &[u32]) -> (SetEncoding, Vec<u32>, usize) {
    let mut bytes = Vec::new();
    encode_set_block(&mut bytes, ids).expect("the test sets are well formed");
    let encoding = SetEncoding::from_tag(bytes[0]).expect("a tag the encoder wrote");
    let mut at = 0usize;
    let decoded =
        decode_set_block(&bytes, &mut at, NO_LIMIT).expect("the encoder's own bytes decode");
    assert_eq!(at, bytes.len(), "the decoder consumed the whole block");
    (encoding, decoded, bytes.len())
}

fn block(tag: u8, count: u32, payload: Vec<u8>) -> Vec<u8> {
    let mut out = vec![tag];
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&payload);
    out
}

// kills: an encoder or decoder that loses, reorders or invents an id on any of
// the three shapes the t88 measurements say actually occur - one contiguous run
// (the scanned state set), a scattered set (a failure list), and a holed span
// (the province set). It also pins WHICH encoding wins each, because that is the
// whole size argument for carrying three.
#[test]
fn every_encoding_round_trips_the_shapes_the_capture_actually_holds() {
    let contiguous: Vec<u32> = (1..=1081).collect();
    let (encoding, decoded, len) = round_trip(&contiguous);
    assert_eq!(decoded, contiguous);
    assert_eq!(
        encoding,
        SetEncoding::Runs,
        "1,081 contiguous ids are ONE run"
    );
    assert!(len <= 16, "the scanned state set must be tiny, got {len} bytes");

    let scattered = vec![7u32, 4_000, 900_000];
    let (encoding, decoded, _) = round_trip(&scattered);
    assert_eq!(decoded, scattered);
    assert_eq!(encoding, SetEncoding::Delta);

    let holed: Vec<u32> = (1..=13_000).filter(|id| id % 3 != 0).collect();
    let (encoding, decoded, len) = round_trip(&holed);
    assert_eq!(decoded, holed);
    assert_eq!(encoding, SetEncoding::Bitmap);
    assert!(len < 2_000, "a holed span must be about 1.7 KB, got {len}");
}

// kills: leaving the empty set with no legal encoding. A HEALTHY frame carries
// empty incomplete sets - nothing failed - so this is the ordinary case, not an
// edge one, and none of the three encodings can express it on its own: runs and
// delta both begin with a first id, and a bitmap needs a nonzero bit length.
#[test]
fn the_empty_set_has_exactly_one_spelling() {
    let mut bytes = Vec::new();
    encode_set_block(&mut bytes, &[]).expect("the empty set encodes");
    assert_eq!(bytes, vec![EMPTY_SET_ENCODING.tag(), 0, 0, 0, 0, 0, 0, 0, 0]);
    let mut at = 0usize;
    assert_eq!(
        decode_set_block(&bytes, &mut at, NO_LIMIT).expect("decodes"),
        Vec::<u32>::new()
    );

    for tag in [SetEncoding::Delta.tag(), SetEncoding::Bitmap.tag()] {
        let mut other = bytes.clone();
        other[0] = tag;
        let mut at = 0usize;
        decode_set_block(&other, &mut at, NO_LIMIT)
            .expect_err("a zero-count block in any other encoding is refused");
    }
}

// kills: reconstructing an id in u32 arithmetic. Each of these names u32::MAX + 1
// and passes every length, order, maximality and count rule; only checked u64
// reconstruction catches them.
#[test]
fn an_id_that_does_not_fit_u32_is_refused_in_every_encoding() {
    let mut payload = Vec::new();
    write_leb128(&mut payload, u32::MAX);
    write_leb128(&mut payload, 1);
    let runs = block(SetEncoding::Runs.tag(), 2, payload);
    let mut at = 0usize;
    assert!(decode_set_block(&runs, &mut at, NO_LIMIT)
        .expect_err("a run past u32::MAX is refused")
        .contains("exceeds u32"));

    let mut payload = Vec::new();
    write_leb128(&mut payload, u32::MAX);
    write_leb128(&mut payload, 0);
    let delta = block(SetEncoding::Delta.tag(), 2, payload);
    let mut at = 0usize;
    assert!(decode_set_block(&delta, &mut at, NO_LIMIT)
        .expect_err("a delta past u32::MAX is refused")
        .contains("exceeds u32"));

    let mut payload = Vec::new();
    payload.extend_from_slice(&u32::MAX.to_le_bytes());
    payload.extend_from_slice(&2u32.to_le_bytes());
    payload.push(0b11);
    let bitmap = block(SetEncoding::Bitmap.tag(), 2, payload);
    let mut at = 0usize;
    assert!(decode_set_block(&bitmap, &mut at, NO_LIMIT)
        .expect_err("a bitmap past u32::MAX is refused")
        .contains("exceeds u32"));
}

// kills: allocating a losing candidate. `{1, u32::MAX}` has a 512 MiB bitmap
// candidate; this code runs inside the game, and building half a gigabyte to
// LOSE a comparison is a way to make the game wait.
#[test]
fn the_widest_possible_span_is_priced_without_being_built() {
    let ids = vec![1u32, u32::MAX];
    assert_eq!(
        bitmap_len(&ids),
        Some(8 + (u32::MAX as usize).div_ceil(8)),
        "the bitmap candidate is priced arithmetically"
    );
    let (encoding, payload_len) = choose_encoding(&ids);
    assert_eq!(encoding, SetEncoding::Delta, "and it loses");
    assert!(payload_len < 16);
    let (_, decoded, len) = round_trip(&ids);
    assert_eq!(decoded, ids);
    assert!(len < 32, "only the winner is built: {len} bytes");
}

// kills: a decoder that trusts a declared count. This block is nine bytes of
// header and three of payload, is canonical in every other respect, and asks the
// decoder for four billion pushes.
//
// It kills BOTH halves of the bound. A caller that passes a tight limit gets it
// honoured; a caller that passes a limit it read off the same untrusted payload
// -- which is what every real caller has to hand, d48's `observed_state_roots`
// header word or the structural `i32::MAX - 1` id ceiling -- still cannot raise
// the codec's own resource ceiling. Delete the `.min(MAX_DECODABLE_IDS)` and the
// second half of this test allocates 8 GiB instead of failing.
#[test]
fn a_compressed_block_cannot_ask_for_billions_of_ids() {
    let mut payload = Vec::new();
    write_leb128(&mut payload, 0);
    write_leb128(&mut payload, u32::MAX - 1);
    let bomb = block(SetEncoding::Runs.tag(), u32::MAX, payload);
    assert!(bomb.len() < 20, "a few bytes claiming the whole id space");

    let mut at = 0usize;
    assert!(decode_set_block(&bomb, &mut at, 20_000)
        .expect_err("the ceiling refuses it before anything is built")
        .contains("more than the 20000"));

    // The bound a real caller has is another word off this same payload.
    let mut at = 0usize;
    assert!(decode_set_block(&bomb, &mut at, (i32::MAX - 1) as usize)
        .expect_err("a caller bound off the same payload cannot raise the ceiling")
        .contains(&format!("more than the {MAX_DECODABLE_IDS}")));
}

// kills: letting the encoder emit a block its own decoder would refuse. The gap
// arithmetic is `id - previous - 1`, so a duplicate or a descending pair
// underflows; this was a `debug_assert!`, which is precisely the guard the
// shipping recorder does not run. The collector encodes BEFORE its gate, so the
// gate cannot cover for it either.
#[test]
fn the_encoder_refuses_input_it_cannot_spell() {
    let mut out = Vec::new();
    assert!(encode_set_block(&mut out, &[2, 1])
        .expect_err("a descending pair is refused")
        .contains("strictly increasing"));
    let mut out = Vec::new();
    assert!(encode_set_block(&mut out, &[1, 1])
        .expect_err("a duplicate is refused")
        .contains("strictly increasing"));
    let mut out = Vec::new();
    let too_many: Vec<u32> = (1..=(MAX_DECODABLE_IDS as u32 + 1)).collect();
    assert!(encode_set_block(&mut out, &too_many)
        .expect_err("a set past the resource ceiling is refused")
        .contains("exceeds the"));
    assert!(
        out.is_empty(),
        "a refused encode must leave the caller's buffer untouched"
    );
}

// kills: pricing the runs encoding by BUILDING the run list. The module header
// promises all three candidates are priced without being built, and this code
// runs inside the game; a set with a run boundary at every id used to allocate a
// Vec of half a million pairs to discover that runs lose.
#[test]
fn pricing_the_runs_encoding_allocates_nothing() {
    // Every id isolated: the worst case for a run list, and runs lose outright.
    let alternating: Vec<u32> = (0..50_000u32).map(|index| index * 2).collect();
    let mut observed = 0usize;
    for_each_run(&alternating, |_, len| {
        assert_eq!(len, 1, "no two of these ids are adjacent");
        observed += 1;
    });
    assert_eq!(observed, alternating.len(), "every run was visited");
    let (encoding, _) = choose_encoding(&alternating);
    assert_ne!(encoding, SetEncoding::Runs, "and runs lose this one");
}

// kills: accepting a second spelling of one set. Adjacent runs, an overlong
// varint, a set padding bit and a popcount that disagrees with the declared count
// are each a way for two byte strings to mean one set - and the popcount one
// turns a NAMED unavailability into silence, which is the exact failure this
// census exists to prevent.
#[test]
fn a_second_spelling_of_one_set_is_refused() {
    let mut payload = Vec::new();
    write_leb128(&mut payload, 1);
    write_leb128(&mut payload, 1);
    write_leb128(&mut payload, 0); // gap - 1 == 0: an adjacent run
    write_leb128(&mut payload, 1);
    let adjacent = block(SetEncoding::Runs.tag(), 4, payload);
    let mut at = 0usize;
    assert!(decode_set_block(&adjacent, &mut at, NO_LIMIT)
        .expect_err("adjacent runs are refused")
        .contains("maximal"));

    let overlong = block(SetEncoding::Delta.tag(), 1, vec![0x81, 0x00]);
    let mut at = 0usize;
    assert!(decode_set_block(&overlong, &mut at, NO_LIMIT)
        .expect_err("an overlong varint is refused")
        .contains("overlong"));

    let mut payload = Vec::new();
    payload.extend_from_slice(&5u32.to_le_bytes());
    payload.extend_from_slice(&8u32.to_le_bytes());
    payload.push(0);
    let no_bits = block(SetEncoding::Bitmap.tag(), 1, payload);
    let mut at = 0usize;
    assert!(decode_set_block(&no_bits, &mut at, NO_LIMIT)
        .expect_err("a bitmap whose popcount is not its count is refused")
        .contains("declared count"));

    let mut payload = Vec::new();
    payload.extend_from_slice(&5u32.to_le_bytes());
    payload.extend_from_slice(&2u32.to_le_bytes());
    payload.push(0b1000_0011);
    let padded = block(SetEncoding::Bitmap.tag(), 2, payload);
    let mut at = 0usize;
    assert!(decode_set_block(&padded, &mut at, NO_LIMIT)
        .expect_err("a set padding bit is refused")
        .contains("padding"));
}

// kills: canonicality WITHIN an encoding being mistaken for canonicality ACROSS
// them. `{1}` is a legal one-run block, a legal one-delta block and a legal
// one-bit bitmap, and each is internally canonical. Only recomputing the winner
// after decoding makes the spelling unique; without it two recorders reading one
// world can disagree byte for byte.
#[test]
fn one_set_has_one_legal_block_across_all_three_encodings() {
    // A single id is one byte as a delta and two as a run (a run must also
    // spell its length), so DELTA is the canonical spelling here. Which one
    // wins is the encoder's business; that only ONE wins is this test's.
    let mut canonical = Vec::new();
    encode_set_block(&mut canonical, &[1u32]).expect("a single id encodes");
    assert_eq!(canonical[0], SetEncoding::Delta.tag());

    let mut payload = Vec::new();
    write_leb128(&mut payload, 1);
    write_leb128(&mut payload, 0);
    let as_runs = block(SetEncoding::Runs.tag(), 1, payload);
    let mut at = 0usize;
    assert!(decode_set_block(&as_runs, &mut at, NO_LIMIT)
        .expect_err("the runs spelling of a single id is refused")
        .contains("canonical encoding"));

    let mut payload = Vec::new();
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.push(1);
    let as_bitmap = block(SetEncoding::Bitmap.tag(), 1, payload);
    let mut at = 0usize;
    assert!(decode_set_block(&as_bitmap, &mut at, NO_LIMIT)
        .expect_err("the bitmap spelling of a single id is refused")
        .contains("canonical encoding"));
}
