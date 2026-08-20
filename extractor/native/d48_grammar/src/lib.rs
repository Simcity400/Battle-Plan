//! D48's coverage trailer (`D48V`): the byte grammar of the four authenticated
//! id sets that say what the walk ATTEMPTED, and which of those reads came back
//! whole.
//!
//! # Why this is a shared crate and not two transcriptions
//!
//! Two independent parsers read a d48 payload: the recorder lane's own
//! `decode_lane_frame`, and `wire_format::d48`, the deliberately separate
//! reference implementation whose value is catching producer drift. Owner
//! ruling 2026-08-15 ("ONE shared decoder") says the two must not transcribe
//! one byte grammar twice — the precedent that bought that ruling is `d13`,
//! whose transcribed structural walk went stale against the shipping producer
//! within a day and then refused every payload a real recorder wrote. This
//! grammar is the shared half. What stays independent is everything above the
//! bytes: each side keeps its own semantic projection, fold behaviour,
//! diagnostics, and (in `wire_format`'s case) its stricter railway-identity
//! refusal.
//!
//! # What the trailer is for
//!
//! A d48 frame carries rows. Without a census, a state with no rows is
//! ambiguous four ways: scanned and genuinely empty, scanned and failed,
//! scanned and partly skipped, or never in the root list. Treating that absence
//! as zero under-counts every country; treating it as unavailable makes most
//! countries unusable. The four sets below separate them, which is what lets
//! the replay show a definite "0 buildings" for an empty state instead of a
//! "no data" label (owner ruling 2026-08-20).
//!
//! # The tail, and why RLC1's extent had to become computed
//!
//! After the three row tables the payload carries an OPTIONAL `RLC1` railway
//! trailer and then a MANDATORY `D48V`. `RLC1` used to be decoded as "the whole
//! rest of the tail", with an exact-length check; appending anything after it
//! made every shipping parser reject every new frame. [`split_frame_tail`] is
//! the fix: `RLC1`'s extent is computed from its own header, and what follows
//! belongs to `D48V`. Nothing can hide between them, because `D48V` then
//! requires its own exact total length.

use id_set_codec::{decode_set_block, encode_set_block, MAX_DECODABLE_IDS};

/// The `CBSD` payload version both sides must agree on.
///
/// It lives here, in the shared grammar, because a version word that each side
/// declares for itself is a version word that drifts: this port declared v2 for
/// as long as the producer had been writing v3, which is precisely the staleness
/// the shared-decoder ruling exists to stop. Sharing the constant makes the two
/// unable to disagree, which is stronger than a test asserting that they do not.
///
/// v3 is v2 plus the MANDATORY `D48V` trailer.
pub const COMPACT_VERSION: u16 = 3;

pub const COVERAGE_TRAILER_MAGIC: [u8; 4] = *b"D48V";
pub const COVERAGE_TRAILER_VERSION: u16 = 1;
pub const COVERAGE_TRAILER_HEADER_LEN: usize = 24;

pub const RAILWAY_TRAILER_MAGIC: [u8; 4] = *b"RLC1";

/// `scanned_states` faithfully names every unique state root the walk
/// attempted. Cleared when the root list carried duplicates: a list that
/// repeats itself has lied about its own cardinality, so its census is
/// testimony rather than proof.
pub const CENSUS_FLAG_STATE_ROOTS_AUTHORITATIVE: u32 = 1;
/// `building_incomplete_provinces` faithfully names every scanned province
/// whose read failed or skipped rows. Cleared on duplicate province roots.
///
/// Note precisely what this does and does NOT assert. It says the producer's
/// province failure list is complete and self-consistent. It does NOT make
/// membership checkable by the decoder: the frame carries no scanned-province
/// set (design v4 scoped it out at +49% of a median frame), so a decoder can
/// bound this set's size and id range but cannot confirm that a named province
/// was scanned. Per-province building ABSENCE is therefore unavailable and
/// named, which is the limitation the design records rather than hides.
pub const CENSUS_FLAG_PROVINCE_FAILURES_AUTHORITATIVE: u32 = 1 << 1;
pub const CENSUS_KNOWN_FLAGS: u32 =
    CENSUS_FLAG_STATE_ROOTS_AUTHORITATIVE | CENSUS_FLAG_PROVINCE_FAILURES_AUTHORITATIVE;

/// The four authenticated id sets of the `D48V` trailer. Every set is strictly
/// increasing; the two incomplete STATE sets are subsets of `scanned_states`,
/// and the decoder refuses anything else.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoverageCensus {
    /// See the `CENSUS_FLAG_*` constants. Unknown bits are refused.
    pub census_flags: u32,
    /// State roots whose read was ATTEMPTED this frame — the set completeness
    /// is measured against. The frame's `observed_state_roots` is exactly its
    /// cardinality.
    pub scanned_states: Vec<u32>,
    /// Scanned states whose own STATE-SCOPE building read failed or skipped rows.
    pub building_incomplete_states: Vec<u32>,
    /// Scanned states whose resource read failed or skipped rows.
    pub resource_incomplete_states: Vec<u32>,
    /// Scanned provinces whose building read failed or skipped rows. Bounded
    /// and range-checked, but NOT membership-checked — see
    /// [`CENSUS_FLAG_PROVINCE_FAILURES_AUTHORITATIVE`].
    pub building_incomplete_provinces: Vec<u32>,
}

/// Encode the `D48V` trailer, refusing any census the decoder would refuse.
///
/// Fail-closed on the producer side as well as the consumer side: the shipping
/// collector encodes a frame BEFORE its gate runs, so the gate cannot be what
/// stops a malformed census from reaching the wire.
pub fn encode_coverage_trailer(out: &mut Vec<u8>, coverage: &CoverageCensus) -> Result<(), String> {
    if coverage.census_flags & !CENSUS_KNOWN_FLAGS != 0 {
        return Err(format!(
            "D48V census flags {:#010x} include bits no decoder understands",
            coverage.census_flags & !CENSUS_KNOWN_FLAGS
        ));
    }
    for (name, set) in [
        ("building_incomplete_states", &coverage.building_incomplete_states),
        (
            "resource_incomplete_states",
            &coverage.resource_incomplete_states,
        ),
    ] {
        if !sorted_subset(set, &coverage.scanned_states) {
            return Err(format!(
                "D48V {name} names a state outside scanned_states"
            ));
        }
    }
    let mut payload = Vec::new();
    encode_set_block(&mut payload, &coverage.scanned_states)
        .map_err(|reason| format!("D48V scanned_states: {reason}"))?;
    encode_set_block(&mut payload, &coverage.building_incomplete_states)
        .map_err(|reason| format!("D48V building_incomplete_states: {reason}"))?;
    encode_set_block(&mut payload, &coverage.resource_incomplete_states)
        .map_err(|reason| format!("D48V resource_incomplete_states: {reason}"))?;
    encode_set_block(&mut payload, &coverage.building_incomplete_provinces)
        .map_err(|reason| format!("D48V building_incomplete_provinces: {reason}"))?;
    out.extend_from_slice(&COVERAGE_TRAILER_MAGIC);
    out.extend_from_slice(&COVERAGE_TRAILER_VERSION.to_le_bytes());
    out.extend_from_slice(&(COVERAGE_TRAILER_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&coverage.census_flags.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&(payload.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(())
}

/// Split the tail after the three row tables into its optional `RLC1` prefix
/// and the `D48V` remainder.
///
/// The `RLC1` extent is COMPUTED from its own header, never taken to be the
/// rest of the tail. Returns `(rlc1_bytes, d48v_bytes)`; `rlc1_bytes` is `None`
/// when the tail does not begin with the `RLC1` magic, which is the legal shape
/// for a frame that carries no railway snapshot.
pub fn split_frame_tail(tail: &[u8]) -> Result<(Option<&[u8]>, &[u8]), String> {
    if tail.len() < 4 || tail[0..4] != RAILWAY_TRAILER_MAGIC {
        return Ok((None, tail));
    }
    let extent = railway_trailer_extent(tail)?;
    Ok((Some(&tail[..extent]), &tail[extent..]))
}

/// How many bytes an `RLC1` trailer sitting at the START of `tail` occupies,
/// computed from its own header alone.
///
/// The row and route counts are attacker-shaped words off a stream on disk, so
/// every multiplication is overflow-checked, and the computed extent must fit
/// inside the tail actually present.
pub fn railway_trailer_extent(tail: &[u8]) -> Result<usize, String> {
    const HEADER_LEN: usize = 24;
    const ROW_LEN: usize = 64;
    if tail.len() < HEADER_LEN {
        return Err("RLC1 trailer truncated".to_string());
    }
    let row_count = u32::from_le_bytes(tail[8..12].try_into().expect("4 bytes")) as usize;
    let route_count = u32::from_le_bytes(tail[12..16].try_into().expect("4 bytes")) as usize;
    let rows_len = row_count
        .checked_mul(ROW_LEN)
        .ok_or_else(|| "RLC1 row length overflow".to_string())?;
    let routes_len = route_count
        .checked_mul(4)
        .ok_or_else(|| "RLC1 route length overflow".to_string())?;
    let extent = HEADER_LEN
        .checked_add(rows_len)
        .and_then(|value| value.checked_add(routes_len))
        .ok_or_else(|| "RLC1 total length overflow".to_string())?;
    if extent > tail.len() {
        return Err(format!(
            "RLC1 computed extent {extent} runs past the {}-byte tail",
            tail.len()
        ));
    }
    Ok(extent)
}

/// Every id in `sub` appears in `sup`.
///
/// BOTH must be STRICTLY INCREASING — which is what the codec returns and what
/// the encoder refuses to violate. It is deliberately not a duplicate-tolerant
/// set helper: `[1, 1]` is reported not to be a subset of `[1]`, because the
/// matching element of `sup` is consumed by the first `1`. That input cannot
/// reach here from the wire (the codec rejects a non-increasing block) nor from
/// the encoder (which refuses it), so the linear merge is the right shape; the
/// contract is stated rather than defended against.
pub fn sorted_subset(sub: &[u32], sup: &[u32]) -> bool {
    let mut sup_iter = sup.iter();
    'outer: for &id in sub {
        for &candidate in sup_iter.by_ref() {
            if candidate == id {
                continue 'outer;
            }
            if candidate > id {
                return false;
            }
        }
        return false;
    }
    true
}

/// Decode the mandatory `D48V` coverage trailer, refusing every shape that is
/// not the one canonical spelling and every claim the header cannot support.
///
/// `observed_state_roots` and `observed_province_roots` come from the frame
/// header and are UNTRUSTED; they bound the sets below, and are themselves
/// bounded here against the id space and the codec's resource ceiling before
/// anything is built.
pub fn decode_coverage_trailer(
    trailer: &[u8],
    observed_state_roots: u32,
    observed_province_roots: u32,
    max_state_id: u32,
    max_province_id: u32,
) -> Result<CoverageCensus, String> {
    if trailer.is_empty() {
        return Err("missing D48V coverage trailer".to_string());
    }
    if trailer.len() < COVERAGE_TRAILER_HEADER_LEN {
        return Err("D48V trailer truncated".to_string());
    }
    if trailer[0..4] != COVERAGE_TRAILER_MAGIC {
        return Err("unknown D48 trailer magic where D48V is required".to_string());
    }
    if u16::from_le_bytes(trailer[4..6].try_into().expect("2 bytes")) != COVERAGE_TRAILER_VERSION
        || u16::from_le_bytes(trailer[6..8].try_into().expect("2 bytes")) as usize
            != COVERAGE_TRAILER_HEADER_LEN
    {
        return Err("D48V header mismatch".to_string());
    }
    let census_flags = u32::from_le_bytes(trailer[8..12].try_into().expect("4 bytes"));
    if census_flags & !CENSUS_KNOWN_FLAGS != 0 {
        return Err(format!(
            "D48V has unknown census flag bits {:#010x}",
            census_flags & !CENSUS_KNOWN_FLAGS
        ));
    }
    if u64::from_le_bytes(trailer[12..20].try_into().expect("8 bytes")) != 0 {
        return Err("D48V reserved field is nonzero".to_string());
    }
    let payload_len = u32::from_le_bytes(trailer[20..24].try_into().expect("4 bytes")) as usize;
    let expected = COVERAGE_TRAILER_HEADER_LEN
        .checked_add(payload_len)
        .ok_or_else(|| "D48V length overflow".to_string())?;
    if trailer.len() != expected {
        return Err(format!(
            "D48V length {} != expected {expected}",
            trailer.len()
        ));
    }
    // The declared root counts bound every set below, so they must themselves be
    // bounded: distinct ids in `1..=MAX` cannot outnumber it, and no set this
    // product can produce outnumbers the codec's resource ceiling.
    if observed_state_roots > max_state_id {
        return Err(format!(
            "observed_state_roots {observed_state_roots} exceeds the state id space"
        ));
    }
    if observed_province_roots > max_province_id {
        return Err(format!(
            "observed_province_roots {observed_province_roots} exceeds the province id space"
        ));
    }
    let payload = &trailer[COVERAGE_TRAILER_HEADER_LEN..];
    let mut at = 0usize;
    let scanned_states = decode_set_block(
        payload,
        &mut at,
        (observed_state_roots as usize).min(MAX_DECODABLE_IDS),
    )
    .map_err(|reason| format!("D48V scanned_states: {reason}"))?;
    if scanned_states.len() != observed_state_roots as usize {
        return Err(format!(
            "D48V scanned_states cardinality {} != observed_state_roots {observed_state_roots}",
            scanned_states.len()
        ));
    }
    let building_incomplete_states = decode_set_block(payload, &mut at, scanned_states.len())
        .map_err(|reason| format!("D48V building_incomplete_states: {reason}"))?;
    let resource_incomplete_states = decode_set_block(payload, &mut at, scanned_states.len())
        .map_err(|reason| format!("D48V resource_incomplete_states: {reason}"))?;
    let building_incomplete_provinces = decode_set_block(
        payload,
        &mut at,
        (observed_province_roots as usize).min(MAX_DECODABLE_IDS),
    )
    .map_err(|reason| format!("D48V building_incomplete_provinces: {reason}"))?;
    if at != payload.len() {
        return Err("D48V trailer has trailing bytes".to_string());
    }
    for (name, set, ceiling) in [
        ("scanned_states", &scanned_states, max_state_id),
        (
            "building_incomplete_provinces",
            &building_incomplete_provinces,
            max_province_id,
        ),
    ] {
        if let (Some(&first), Some(&last)) = (set.first(), set.last()) {
            if first == 0 || last > ceiling {
                return Err(format!("D48V {name} id out of range"));
            }
        }
    }
    for (name, set) in [
        ("building_incomplete_states", &building_incomplete_states),
        ("resource_incomplete_states", &resource_incomplete_states),
    ] {
        if !sorted_subset(set, &scanned_states) {
            return Err(format!(
                "D48V {name} names a state outside scanned_states"
            ));
        }
    }
    Ok(CoverageCensus {
        census_flags,
        scanned_states,
        building_incomplete_states,
        resource_incomplete_states,
        building_incomplete_provinces,
    })
}

#[cfg(test)]
mod tests;
