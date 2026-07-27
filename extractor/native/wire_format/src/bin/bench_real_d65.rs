//! Measure the wire format against the shipping compact stream on REAL d65
//! `special_project_programs_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d65-scheduled capture
//! (`runs/batch3/deepwar_a`). The stream's own d65 DATA records are the shipping
//! format's cost for the domain: a 24-byte payload header plus a fixed 16-byte
//! row per special-project facility, full-replace every hour, exactly what the
//! recorder stores today. This tool replays those records into per-hour states —
//! the same fold `world_fold::fold_special_project_programs` performs — and
//! re-encodes every hour with this crate's grammar: a BASELINE at each 48-hour
//! block start, field-granular stored-minimal DELTAs between.
//!
//! **Nothing here re-derives a contract it can call.** The header split, the row
//! walk, the key and the fold's duplicate rule all go through
//! `wire_format::d65`'s public boundary (`d65::state_from_payload`); H4CR
//! framing, record semantics and sequence continuity go through
//! `compact_contract`'s own parser and stateful validator. A harness that
//! reimplements either drifts from the recorder without anything failing.
//!
//! **The whole stream is validated before any artifact is written.** A stream
//! that is truncated, spliced, or corrupt would otherwise yield a smaller but
//! entirely plausible measurement — a silently wrong number, which is worse than
//! a crash. `scan_stream` therefore enforces, fail-closed:
//!
//! - every record header through `compact_contract::parse_compact_record_header`
//!   (magic, version, header length, payload bound);
//! - every record through `compact_contract::stream_validator::StatefulStreamValidator`
//!   (sequence continuity from the stream's first record, record-type and
//!   record-flag semantics, FRAME_END shape);
//! - the hour boundary rules of `recorder_host::stream_fold::BoundaryTracker`
//!   (`recorder_host/src/stream_fold.rs:277`) that do not require DOMAIN_PERF
//!   accounting: a DOMAIN_DATA record opens its game hour, a FRAME_END closes
//!   the open hour and only that hour, and closed hours strictly increase;
//! - that tracker's DOMAIN_PERF hour binding (`:328`-`:365`), including the DATA
//!   arm's pending-PERF rule (`:378`-`:386`);
//! - FRAME_END seal validation (`stream_fold.rs:46`-`:89`) and the seal's count
//!   cross-check against distinct non-probe PERF domains (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority. This crate cannot depend on `recorder_host` (it pulls in every
//! capture lane), so the relevant predicates are mirrored here.
//!
//! **Two limits of that subset, stated rather than implied.**
//!
//! 1. *Only d65 DATA payloads are validated.* Production runs
//!    `validate_domain_data_payload_at` on the payload of **every** DATA record
//!    (`stream_fold.rs:593`); doing that here would mean owning every domain's
//!    payload authority, which is precisely the `recorder_host` dependency this
//!    crate must not take. The concrete hazard is a d65 record relabelled to
//!    another domain id, or dropped: it would be ignored and the run would
//!    report a plausible measurement over fewer hours. `scan_stream` closes that
//!    specific hole with an explicit completeness check — every closed game hour
//!    must carry exactly one d65 DATA record — and says there why that is a
//!    per-capture assertion rather than a grammar rule.
//! 2. *This harness is deliberately STRICTER than d65's payload authority on one
//!    point: an unassigned v1 status bit.* `compact_contract` instructs callers
//!    to reject a non-zero `compact_status_unknown_flags_v1` before applying any
//!    failure/informational classification, because an unassigned bit has no
//!    defined classification (`compact_contract/src/lib.rs:121`-`:123`,
//!    `status_flags_base.rs:21`-`:24`). `scan_stream` does that. d65's payload
//!    authority does **not**: it records the status word
//!    (`domain_data_authority.rs:12323`) and adds no parse error for an
//!    unassigned bit, and bit 14 is outside `COMPACT_STATUS_FAILURE_MASK`, so
//!    production folds such a record normally. A payload with
//!    `status_flags = 0x4000` is therefore accepted by the recorder and refused
//!    here. The strictness is kept, not relaxed: falling through to "success" on
//!    a bit whose meaning is undefined is how a future failure status would be
//!    silently measured as healthy, and every other measurement bin in this
//!    crate applies the same rule. The gap is on production's side and is
//!    reported as a finding. No number moves either way — all 514 records of the
//!    measured capture carry `status_flags == 0`.
//! 3. *A stream cut exactly at a FRAME_END boundary is accepted, by design.*
//!    `eof_check` treats it as a well-formed shorter stream and so does this
//!    harness; a cut anywhere else — mid-record, or leaving an hour open — is
//!    refused. So the harness measures the hours the file actually closes and
//!    **cannot tell a complete capture from one truncated at an hour boundary**:
//!    a clean 514-hour stream cut after hour 300 yields an honest 300-hour
//!    measurement, not an error. Deciding that a file is the whole capture needs
//!    evidence this harness is not given (an expected final hour, a record
//!    count, a size or hash, a terminal capture identity), so the hour count is
//!    always reported beside the sizes.
//!
//! **The order of the row walk against the status check is d65's, not the
//! pilot's.** `fold_special_project_programs` checks status *first*
//! (`world_fold.rs:12265`) and only then walks rows (`:12290`), so on a
//! failure-status record the fold never sees a malformed or duplicated row. The
//! *status-independent payload authority* does, on every record:
//! `decode_compact_special_project_programs_data_payload`
//! (`domain_data_authority.rs:12305`-`:12361`) re-checks the length, requires
//! `changed_rows <= row_count`, and rejects a zero owner, a zero `program_id`
//! **and a duplicated `(owner, program_id)` pair** whatever the status word says.
//! So this harness runs `d65::state_from_payload` on every d65 record before
//! classification, and a row that walk refuses invalidates the stream rather
//! than being skipped with its hour. That is the opposite of d62, whose
//! authority does not check duplicates and where a duplicate in a
//! failure-status record is therefore excluded rather than refused
//! (`bench_real_d62.rs:1411`); the difference is a property of the two
//! authorities, and copying either one across would be wrong.
//!
//! **d65's non-folding paths, named.** The fold has exactly two, and neither is a
//! stream rejection: (1) a failure status — `handle_compact_failure`
//! (`world_fold.rs:12265`) retains the accumulator and folds nothing, so the
//! hour is an **excluded hour** in both arms; (2) the expected-partial-tolerance
//! branch at `:12272`, which is **dead code** —
//! `COMPACT_EXPECTED_PARTIAL_TOLERANCES` lists only domains 26, 27, 28 and 54
//! (`compact_contract/src/lib.rs:136`-`:158`), so the predicate is constantly
//! false for domain 65. It is also redundant with (1), because the status it
//! would tolerate, `COMPACT_STATUS_PARTIAL_DATA`, is inside
//! `COMPACT_STATUS_FAILURE_MASK` (`lib.rs:117`). This harness implements (1) and
//! does not implement (2); the finding is reported to the orchestrator. Every
//! other `Err` from the fold — a bad length, a zero identity, a duplicated
//! pair — is a **stream rejection** here, because it says the payload is not
//! this grammar.
//!
//! **The only other writer of d65's accumulator slot is proven inert by
//! construction, not by sampling.** `fold_special_project_programs` is the sole
//! writer of the `special_project_programs` key (the only assignments are
//! `world_fold.rs:12335` and `:12339`). The first of those is the d18 roster
//! witness, which seeds `Value::Array(Vec::new())` for every known country and
//! runs *before* the per-owner loop overwrites it, so it can only ever record a
//! country as having **no** facilities — it cannot add, drop or alter a row. The
//! keyed set's members are rows, so it reproduces the fold's row content exactly
//! and needs no roster input.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d65.bin   — the d65 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface, twice: cadence-independent (over
//! measured-hour transitions, taken before the block decision) and along the
//! delta path (over the hours that actually emitted a delta).

use std::collections::BTreeSet;
use std::io::Write;
use std::ops::Range;

use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, compact_status_unknown_flags_v1,
    parse_compact_record_header, COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedState,
};
use wire_format::d65;

const D65: u16 = 65;
const BLOCK_HOURS: usize = 48;

/// One usable d65 hour: its game hour, the folded state of that hour, and its
/// byte span in the source stream (the shipping arm is those bytes verbatim,
/// header included).
///
/// The state is built during the scan, not later: every way a row can be
/// rejected — a zero owner, a zero `program_id`, a duplicated pair — is a
/// property of the stream, so it belongs to the pass that either accepts the
/// stream or refuses it. Deferring it would leave the harness able to accept a
/// stream and then unwind while measuring it.
struct D65Hour {
    game_hour: i32,
    state: KeyedState,
    ship_span: Range<usize>,
    /// Whether this measured hour is the immediate successor of the previous
    /// measured one. A `false` here means the arm's prior state is separated
    /// from this one by at least one hour that was never measured — an excluded
    /// failure hour, or a hole in the capture — and the encoder must open a
    /// BASELINE rather than a DELTA across it. See `main`.
    follows_previous: bool,
}

/// What the capture did and did not exercise, counted while scanning. These are
/// not decorations: a path with zero coverage is a caveat on the size claim, and
/// the only honest way to state one is to have counted it.
#[derive(Default)]
struct Coverage {
    /// d65 records of any status — the denominator for everything below.
    records: usize,
    /// Records whose `dropped_ambiguous` diagnostic word is non-zero.
    dropped_ambiguous_records: usize,
    /// Records whose `changed_rows` word differs from `row_count`.
    changed_rows_unequal: usize,
    /// Records carrying any status bit at all (failure or informational).
    nonzero_status_records: usize,
    /// Rows whose `province` word is zero.
    province_zero_rows: usize,
    /// Total rows walked, over records of any status.
    rows_walked: usize,
    /// Game hours the stream actually closed with a FRAME_END.
    closed_hours: usize,
}

struct StreamScan {
    hours: Vec<D65Hour>,
    /// d65 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// Measured d65 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    coverage: Coverage,
}

const FRAME_END_KNOWN_STATUS_MASK: u32 = (1 << 14) - 1;
const PROBE_DOMAIN_ID: u16 = 250;

/// Whether the arm opens a BASELINE for this measured hour.
///
/// Two reasons, and the second is not a cadence choice: a block boundary, or a
/// measured hour that does not immediately follow the previous measured one. A
/// delta across a gap would relate two states that are not consecutive in game
/// time — and where the two happen to be equal it would emit nothing at all,
/// silently asserting "unchanged" over an interval nobody measured. §8 gap
/// normalisation has no implementation in this crate, so a baseline is the only
/// honest thing to write across a gap.
fn opens_baseline(hours_seen: usize, follows_previous: bool) -> bool {
    hours_seen % BLOCK_HOURS == 0 || !follows_previous
}

/// The shipping arm's bytes, split into framing, payload headers and rows.
///
/// Takes the hours and rows that are actually IN the shipping arm — never the
/// all-record coverage counters. An excluded failure record is walked, and so is
/// counted in coverage, but its bytes are not in the arm; deriving the split
/// from coverage would print a decomposition of more records than the arm holds.
/// One derivation, used by `main` and by the test that pins it, so the two
/// cannot drift.
fn shipping_split(hours: usize, rows: usize) -> (usize, usize, usize) {
    (
        hours * COMPACT_RECORD_HEADER_LEN,
        hours * d65::HEADER_LEN,
        rows * d65::ROW_LEN,
    )
}

/// The shipping arm's byte split for a completed scan.
///
/// **The choice of which counters describe the arm lives here, not at the call
/// site.** `StreamScan` carries two populations — `hours`, the records that are
/// in the shipping arm, and `coverage`, every d65 record walked including the
/// excluded ones — and reaching for the second is the defect this function
/// exists to make untestable-by-omission. `main` calls this and nothing else, so
/// a test that calls it on a scan containing an excluded record covers the
/// provenance decision itself rather than merely re-deriving the right answer.
fn shipping_split_of(scan: &StreamScan) -> (usize, usize, usize) {
    let hours = scan.hours.len();
    let rows: usize = scan.hours.iter().map(|h| h.state.len()).sum();
    shipping_split(hours, rows)
}

fn check_frame_end_seal(payload: &[u8], at_byte: usize) -> Result<(u32, u32), String> {
    if payload.len() != COMPACT_FRAME_END_PAYLOAD_LEN {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: payload length {} is not \
             {COMPACT_FRAME_END_PAYLOAD_LEN}",
            payload.len()
        ));
    }
    let emitted_domains = u32::from_le_bytes(payload[16..20].try_into().expect("4 bytes"));
    let scheduled_domains = u32::from_le_bytes(payload[20..24].try_into().expect("4 bytes"));
    let status_flags = u32::from_le_bytes(payload[24..28].try_into().expect("4 bytes"));
    let reserved = u32::from_le_bytes(payload[28..32].try_into().expect("4 bytes"));
    if reserved != 0 {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: reserved word is nonzero \
             ({reserved:#010x})"
        ));
    }
    if emitted_domains != scheduled_domains {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: emitted domain count \
             {emitted_domains} does not match scheduled domain count {scheduled_domains}"
        ));
    }
    let unknown = status_flags & !FRAME_END_KNOWN_STATUS_MASK;
    if unknown != 0 {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: status flags {status_flags:#010x} \
             contain unknown bits {unknown:#010x}"
        ));
    }
    let fatal = status_flags & COMPACT_STATUS_FAILURE_MASK;
    if fatal != 0 {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: status flags {status_flags:#010x} \
             contain fatal bits {fatal:#010x}"
        ));
    }
    Ok((emitted_domains, scheduled_domains))
}

/// Count the coverage facts a `d65::state_from_payload` walk does not return.
///
/// Reads only the two words the boundary does not surface per row: `province`
/// (for the zero-province caveat). The rows have already been accepted by the
/// shared boundary, so this cannot reject anything and never re-derives a rule.
fn note_row_coverage(body: &[u8], coverage: &mut Coverage) {
    for row in body.chunks_exact(d65::ROW_LEN) {
        coverage.rows_walked += 1;
        let province = u32::from_le_bytes(row[8..12].try_into().expect("4 bytes"));
        if province == 0 {
            coverage.province_zero_rows += 1;
        }
    }
}

/// Validate the whole stream against the canonical contract and fold the usable
/// d65 DATA records into per-hour states. Every rejection path is an `Err`,
/// never a silently shorter measurement and never a panic — see the module docs
/// for what is enforced and what the recorder's own validator additionally
/// checks.
fn scan_stream(
    stream: &[u8],
    collection: &wire_format::Collection,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    // Production receives the identity mode from its caller
    // (`stream_fold.rs:220`); the first record cannot state it safely.
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours: Vec<D65Hour> = Vec::new();
    let mut failure_records = 0usize;
    let mut non_contiguous = 0usize;
    let mut coverage = Coverage::default();

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d65 DATA record of ANY status — the uniqueness rule.
    let mut last_d65_hour: Option<i32> = None;
    // Hour of the last d65 record that entered the measurement — the
    // contiguity report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;

    let mut pos = 0usize;
    while pos < stream.len() {
        let header = parse_compact_record_header(&stream[pos..])
            .map_err(|e| format!("record at byte {pos}: {e}"))?;
        let payload_start = pos + COMPACT_RECORD_HEADER_LEN;
        let record_end = payload_start
            .checked_add(header.payload_len)
            .ok_or_else(|| format!("record at byte {pos}: length overflow"))?;
        if record_end > stream.len() {
            // §`eof_check`: a stream ending mid-record is not a short stream, it
            // is a corrupt one.
            return Err(format!(
                "stream ends mid-record at byte {pos}: {} byte(s) present, record needs {}",
                stream.len() - pos,
                record_end - pos
            ));
        }
        let payload = &stream[payload_start..record_end];

        validator
            .validate_record(&header, payload)
            .map_err(|e| format!("record at byte {pos}: {e:?}"))?;

        // The hour-boundary subset of `BoundaryTracker::observe`.
        match header.record_type {
            COMPACT_RECORD_TYPE_DOMAIN_PERF => {
                if header.domain_id != PROBE_DOMAIN_ID
                    && !non_probe_perf_domains.insert(header.domain_id)
                {
                    return Err(format!(
                        "duplicate DOMAIN_PERF for domain {} in hour {} at byte {pos}",
                        header.domain_id, header.game_hour_raw
                    ));
                }
                match open_hour {
                    Some(open) if open != header.game_hour_raw => {
                        return Err(format!(
                            "hour interleaving at byte {pos}: DOMAIN_PERF for hour {} while \
                             hour {open} is still open",
                            header.game_hour_raw
                        ))
                    }
                    Some(_) => {}
                    None => {
                        if let Some(last_closed) = last_closed_hour {
                            if header.game_hour_raw <= last_closed {
                                return Err(format!(
                                    "non-monotonic DOMAIN_PERF at byte {pos}: closed hour \
                                     {last_closed}, then {}",
                                    header.game_hour_raw
                                ));
                            }
                        }
                        match pending_perf_hour {
                            Some(pending) if pending != header.game_hour_raw => {
                                return Err(format!(
                                    "PERF hour conflict at byte {pos}: pending hour {pending}, \
                                     then {}",
                                    header.game_hour_raw
                                ))
                            }
                            Some(_) => {}
                            None => pending_perf_hour = Some(header.game_hour_raw),
                        }
                    }
                }
            }
            COMPACT_RECORD_TYPE_DOMAIN_DATA => match open_hour {
                Some(open) if open != header.game_hour_raw => {
                    return Err(format!(
                        "DOMAIN_DATA at byte {pos} carries hour {} while hour {open} is open",
                        header.game_hour_raw
                    ))
                }
                Some(_) => {}
                None => {
                    if let Some(pending) = pending_perf_hour {
                        if pending != header.game_hour_raw {
                            return Err(format!(
                                "DATA hour conflict at byte {pos}: pending PERF hour \
                                 {pending}, then DATA hour {}",
                                header.game_hour_raw
                            ));
                        }
                    }
                    if let Some(last_closed) = last_closed_hour {
                        if header.game_hour_raw <= last_closed {
                            return Err(format!(
                                "DOMAIN_DATA at byte {pos} reopens hour {} after hour \
                                 {last_closed} closed",
                                header.game_hour_raw
                            ));
                        }
                    }
                    open_hour = Some(header.game_hour_raw);
                }
            },
            COMPACT_RECORD_TYPE_FRAME_END => {
                let (emitted_domains, scheduled_domains) = check_frame_end_seal(payload, pos)?;
                let observed = non_probe_perf_domains.len().min(u32::MAX as usize) as u32;
                if observed != emitted_domains || observed != scheduled_domains {
                    return Err(format!(
                        "FRAME_END seal count mismatch at byte {pos}: observed {observed} \
                         non-probe PERF domains, emitted {emitted_domains}, scheduled \
                         {scheduled_domains}"
                    ));
                }
                match open_hour {
                    Some(open) if open == header.game_hour_raw => {}
                    Some(open) => {
                        return Err(format!(
                            "FRAME_END at byte {pos} closes hour {} while hour {open} is open",
                            header.game_hour_raw
                        ))
                    }
                    None => {
                        return Err(format!(
                            "FRAME_END at byte {pos} closes hour {} without an open \
                             DOMAIN_DATA hour",
                            header.game_hour_raw
                        ))
                    }
                }
                if let Some(last_closed) = last_closed_hour {
                    if header.game_hour_raw <= last_closed {
                        return Err(format!(
                            "non-monotonic FRAME_END at byte {pos}: closed hour \
                             {last_closed}, then {}",
                            header.game_hour_raw
                        ));
                    }
                }
                open_hour = None;
                last_closed_hour = Some(header.game_hour_raw);
                coverage.closed_hours += 1;
                pending_perf_hour = None;
                non_probe_perf_domains.clear();
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        if header.domain_id == D65 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            // One d65 DATA record per game hour, checked over EVERY d65 record
            // and BEFORE any status-based exclusion. d65 is full-replace, so a
            // second record for the same hour would become a second measurement
            // hour built from the same clock time — and gating this on the
            // record being usable would let a success/failure pair through the
            // rule entirely.
            if let Some(previous) = last_d65_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d65 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d65_hour = Some(header.game_hour_raw);

            // VALIDITY BEFORE CLASSIFICATION. The failure predicate below reads
            // the status word to classify the v1 vocabulary; it is not a validity
            // predicate. `compact_contract/src/lib.rs:121`-`:123` says so in as
            // many words: "Callers handling untrusted status words must first
            // reject a non-zero `compact_status_unknown_flags_v1` result;
            // otherwise an unassigned bit has no defined classification." Bits
            // 14..=31 are unassigned (`status_flags_base.rs:21`-`:24`), and an
            // unassigned bit "is not evidence of a future informational status".
            //
            // Without this, a record with `status_flags = 1 << 14` reads as "not
            // a failure" — the failure helper masks with the *known* failure set
            // and returns zero for a bit it does not know — and the hour is
            // measured as healthy. No capture carries an unassigned bit, so no
            // measured number moves.
            //
            // This is stricter than d65's own payload authority, which records
            // the status word (`domain_data_authority.rs:12323`) without
            // rejecting an unassigned bit, so the recorder folds such a record
            // normally. Kept deliberately — see limit 2 in the module docs, and
            // the finding raised in `measurement.md`.
            //
            // The status word is read through the shared boundary's header
            // rather than sliced here, so this cannot disagree with the split.
            // Calling that splitter twice — once for the status word, once
            // inside the row walk below — is deliberate, not an oversight: the
            // production lanes reject an invalid status word before parsing a
            // body at all, and re-entering the same pure function on the same
            // bytes cannot drift the way a local re-slice could. The
            // `debug_assert` below pins the two reads against each other.
            let (d65_header, body) = d65::split_payload(payload)
                .map_err(|e| format!("d65 payload at byte {pos}: {e}"))?;
            let unknown_status = compact_status_unknown_flags_v1(d65_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d65 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            // **The whole row walk runs BEFORE the status check, on every d65
            // record, because the payload AUTHORITY is status-independent.**
            // `decode_compact_special_project_programs_data_payload`
            // (`domain_data_authority.rs:12305`) re-checks the length, requires
            // `changed_rows <= row_count` (`:12335`) and rejects a zero owner
            // (`:12346`), a zero `program_id` (`:12351`) and a duplicated
            // `(owner, program_id)` pair (`:12356`) whatever the status word
            // says. Every one of those is enforced by `d65::state_from_payload`,
            // which is the same code the fold's own row loop is modelled on, so
            // a record the authority would flag refuses the stream here instead
            // of being skipped with its hour.
            //
            // The FOLD's order is the other way round — `handle_compact_failure`
            // at `world_fold.rs:12265` precedes the row loop at `:12290` — but
            // that only means the fold never *looks*; the authority looked
            // already, and a stream carrying a payload the authority rejects is
            // not a stream this measurement may report a number for.
            let (walk_header, state) = d65::state_from_payload(collection, payload, 0)
                .map_err(|e| format!("d65 payload at byte {pos}: {e}"))?;
            debug_assert_eq!(walk_header, d65_header);

            coverage.records += 1;
            if d65_header.dropped_ambiguous != 0 {
                coverage.dropped_ambiguous_records += 1;
            }
            if d65_header.changed_rows != d65_header.row_count {
                coverage.changed_rows_unequal += 1;
            }
            if d65_header.status_flags != 0 {
                coverage.nonzero_status_records += 1;
            }
            note_row_coverage(body, &mut coverage);

            // `world_fold::handle_compact_failure` (called at
            // `world_fold.rs:12265`) marks the domain-hour unknown and RETAINS
            // the prior accumulator on a failure status, folding nothing from
            // this record. The predicate is `compact_contract`'s own, not a
            // copied bit mask: it applies d65's lane-local mask (none) and
            // expected partial tolerance (none — see the module docs on the dead
            // branch), so it cannot drift from the recorder.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D65,
                d65_header.status_flags,
            ) != 0
            {
                failure_records += 1;
            } else {
                let follows_previous = match last_measured_hour {
                    Some(previous) if header.game_hour_raw != previous + 1 => {
                        non_contiguous += 1;
                        false
                    }
                    Some(_) => true,
                    // The first measured hour follows nothing; it opens the arm
                    // with a baseline either way.
                    None => false,
                };
                last_measured_hour = Some(header.game_hour_raw);
                hours.push(D65Hour {
                    game_hour: header.game_hour_raw,
                    state,
                    ship_span: pos..record_end,
                    follows_previous,
                });
            }
        }

        pos = record_end;
    }

    // `BoundaryTracker::eof_check` (`recorder_host/src/stream_fold.rs:459`).
    if let Some(open) = open_hour {
        return Err(format!("game hour {open} left open at EOF"));
    }
    if last_closed_hour.is_none() {
        return Err("stream closes no game hour (no FRAME_END records)".to_string());
    }
    if last_record_type != Some(COMPACT_RECORD_TYPE_FRAME_END) {
        return Err(format!(
            "stream ends after record type {}; final record must be a canonical FRAME_END",
            last_record_type.map_or_else(|| "none".to_string(), |k| k.to_string())
        ));
    }
    if hours.is_empty() {
        return Err("stream carries no usable d65 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d65 DATA
    // record, of any status.
    //
    // This exists because of a real gap in what this harness can check.
    // Production's `verify_stream_logical_boundary` runs
    // `validate_domain_data_payload_at` on the DATA payload of EVERY domain
    // (`recorder_host/src/stream_fold.rs:593`); this harness cannot, because
    // validating another domain's payload means owning that domain's authority
    // and `wire_format` must not depend on `recorder_host`. So a d65 record
    // whose header was relabelled to another domain id — or lost outright —
    // would otherwise be silently ignored, and the run would report a plausible
    // measurement over fewer hours, which is exactly the class of silent wrong
    // number this harness exists to prevent.
    //
    // The rule is a per-capture completeness assertion, NOT a grammar rule: a
    // domain scheduled at a lower cadence would legitimately emit fewer records.
    // It is stated this way deliberately, per the recipe's treatment of a writer
    // that cannot be modelled — fail louder than the recorder would, rather than
    // absorb the difference. A capture where d65 is not scheduled every hour
    // fails here loudly and a human adjusts, instead of a number quietly
    // shrinking.
    if coverage.records != coverage.closed_hours {
        return Err(format!(
            "stream closes {} game hour(s) but carries {} d65 DATA record(s); every closed \
             hour of a d65-scheduled capture must carry exactly one, and this harness cannot \
             validate the payloads of other domains to see where the difference went",
            coverage.closed_hours, coverage.records
        ));
    }

    Ok(StreamScan {
        hours,
        failure_records,
        non_contiguous,
        coverage,
    })
}

fn main() {
    const USAGE: &str = "usage: bench_real_d65 <collector_stream.bin> <out dir> \
                         [legacy|identity-v1]";
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d65::schema();
    let collection = &schema.collections[0];

    // ---- Pass 1: validate the WHOLE stream and fold its d65 DATA records. ---
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result. Everything that
    // can fail on stream content fails here, with a diagnosis and no unwind.
    let scan = match scan_stream(&stream, collection, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: encode both arms. Nothing here reads stream content again,
    // so every assertion below is about the codec, not about the input. -------

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut prior: Option<&KeyedState> = None;
    let mut reader: Option<KeyedState> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut empty_hours = 0usize;
    let mut entries_total = 0usize;

    // Churn, counted twice. Cadence-independent: every measured-hour transition,
    // taken BEFORE the block decision, so a change landing on a block boundary
    // still counts. Delta-path: only the hours that actually emitted a delta.
    // One denominator cannot answer both questions — dividing an idle domain's
    // churn by emitted-delta hours makes it look far busier than it is.
    let mut cadence_changed_rows = 0usize;
    let mut cadence_tombstones = 0usize;
    let mut cadence_transitions = 0usize;
    let mut cadence_inserts = 0usize;
    let mut cadence_removals = 0usize;
    let mut delta_changed_rows = 0usize;
    let mut delta_tombstones = 0usize;

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());

        // The hour's state, folded during the scan: d65 is full-replace, so the
        // hour's rows ARE the hour's whole state.
        let state: &KeyedState = &hour.state;
        entries_total += state.len();

        // Cadence-independent churn, before the block decision touches anything,
        // so a change landing on a block boundary still counts. Only transitions
        // between CONSECUTIVE game hours enter the rate: a transition across an
        // excluded hour spans more than one hour of game time, so counting it
        // per-hour would overstate the rate by however wide the gap was. On this
        // capture every transition is contiguous.
        if let Some(prev) = prior {
            if hour.follows_previous {
                let d = diff(collection, prev, state);
                cadence_transitions += 1;
                cadence_changed_rows += d.changed.len();
                cadence_tombstones += d.tombstones.len();
                cadence_inserts += state.keys().filter(|k| !prev.contains_key(*k)).count();
                cadence_removals += prev.keys().filter(|k| !state.contains_key(*k)).count();
            }
        }

        // New arm: BASELINE at each block start or across a gap (see
        // `opens_baseline`), stored-minimal DELTA between. On this capture there
        // are zero non-contiguous steps, so the gap arm changes no reported
        // number; it makes the arm's semantics defensible on a capture with one.
        let hour_start = wire.len();
        let block_start = opens_baseline(hours_seen, hour.follows_previous);
        let mut emitted = true;
        if block_start || prior.is_none() {
            encode_baseline(&schema, std::slice::from_ref(state), &mut wire)
                .expect("baseline encodes");
            baselines += 1;
        } else {
            let prev = prior.unwrap();
            let d = diff(collection, prev, state);
            if d.is_empty() {
                empty_hours += 1;
                emitted = false;
            } else {
                delta_changed_rows += d.changed.len();
                delta_tombstones += d.tombstones.len();
                encode_delta(&schema, std::slice::from_ref(prev), &[Some(d)], &mut wire)
                    .expect("delta encodes");
            }
        }
        wire_offsets.push(wire.len());

        // Reader replay: decode the bytes just written against the reader's OWN
        // reconstructed prior, exactly as a conforming reader would, and require
        // the result to be stored-equal to this hour's state. This is what makes
        // the size claim a claim about decodable bytes. An hour that emitted
        // nothing is still checked: the reader's retained state must already
        // equal the fold's, which is precisely what "nothing changed" asserts.
        if emitted {
            let hour_bytes = &wire[hour_start..];
            if block_start || reader.is_none() {
                let (states, consumed) =
                    decode_baseline(&schema, hour_bytes, 0).expect("baseline decodes");
                assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
                reader = Some(states.into_iter().next().unwrap());
            } else {
                let reader_prior = reader.take().unwrap();
                let (deltas, consumed) =
                    decode_delta(&schema, std::slice::from_ref(&reader_prior), hour_bytes, 0)
                        .expect("delta decodes");
                assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
                let mut next = reader_prior;
                apply(&mut next, deltas[0].as_ref().unwrap());
                reader = Some(next);
            }
        }
        let replayed = reader.as_ref().unwrap();
        assert_eq!(
            replayed.len(),
            state.len(),
            "game hour {}: row count",
            hour.game_hour
        );
        let residual = diff(collection, replayed, state);
        assert!(
            residual.is_empty(),
            "game hour {}: reader state is not stored-equal to the fold",
            hour.game_hour
        );

        prior = Some(state);
        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d65.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d65.offsets", &ship_offsets),
        ("wire_new.offsets", &wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    let delta_hours = hours_seen - baselines - empty_hours;
    let c = &scan.coverage;
    println!(
        "hours                {hours_seen} ({} non-contiguous steps)",
        scan.non_contiguous
    );
    println!(
        "failure-status hours {} (excluded from both arms)",
        scan.failure_records
    );
    println!(
        "rows avg             {:.1}",
        entries_total as f64 / hours_seen as f64
    );
    println!("baselines            {baselines} (every {BLOCK_HOURS} h)");
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "churn cadence-indep  {:.3} changed rows/hour, {:.3} tombstones/hour over \
         {cadence_transitions} measured-hour transitions ({cadence_inserts} inserts, \
         {cadence_removals} removals in total)",
        cadence_changed_rows as f64 / cadence_transitions.max(1) as f64,
        cadence_tombstones as f64 / cadence_transitions.max(1) as f64
    );
    println!(
        "churn delta-path     {:.3} changed rows/hour, {:.3} tombstones/hour over \
         {delta_hours} emitted-delta hours",
        delta_changed_rows as f64 / delta_hours.max(1) as f64,
        delta_tombstones as f64 / delta_hours.max(1) as f64
    );
    let (ship_framing, ship_payload_headers, ship_rows) = shipping_split_of(&scan);
    assert_eq!(
        ship_framing + ship_payload_headers + ship_rows,
        ship.len(),
        "the shipping byte split must sum to the shipping arm it describes"
    );
    println!(
        "ship_d65.bin         {} bytes = {ship_framing} framing + {ship_payload_headers} \
         payload headers + {ship_rows} rows",
        ship.len()
    );
    println!(
        "wire_new.bin         {} bytes ({:.1}%)",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64
    );
    println!("--- coverage, over all {} d65 records ---", c.records);
    println!(
        "  rows walked                {} ({} with province == 0)",
        c.rows_walked, c.province_zero_rows
    );
    println!(
        "  records with any status    {}",
        c.nonzero_status_records
    );
    println!(
        "  records changed_rows != row_count  {}",
        c.changed_rows_unequal
    );
    println!(
        "  records dropped_ambiguous != 0     {}",
        c.dropped_ambiguous_records
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{COMPACT_DOMAIN_FRAME, COMPACT_FRAME_END_PAYLOAD_LEN};

    fn test_collection() -> wire_format::Collection {
        d65::schema().collections[0].clone()
    }

    /// Build one H4CR record the way the recorder frames it
    /// (`compact_contract/src/record_header.rs:60`).
    fn record(sequence: u64, game_hour: i32, domain: u16, kind: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(COMPACT_RECORD_HEADER_LEN + payload.len());
        out.extend_from_slice(b"H4CR");
        out.extend_from_slice(&1u16.to_le_bytes()); // version
        out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour.to_le_bytes());
        out.extend_from_slice(&domain.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // record flags
        out.extend_from_slice(payload);
        out
    }

    fn frame_end(sequence: u64, game_hour: i32) -> Vec<u8> {
        record(
            sequence,
            game_hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN],
        )
    }

    fn perf(sequence: u64, game_hour: i32, domain: u16) -> Vec<u8> {
        record(
            sequence,
            game_hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &vec![0; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN],
        )
    }

    fn frame_end_sealed(
        sequence: u64,
        game_hour: i32,
        emitted: u32,
        scheduled: u32,
        status: u32,
        reserved: u32,
    ) -> Vec<u8> {
        let mut payload = vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN];
        payload[16..20].copy_from_slice(&emitted.to_le_bytes());
        payload[20..24].copy_from_slice(&scheduled.to_le_bytes());
        payload[24..28].copy_from_slice(&status.to_le_bytes());
        payload[28..32].copy_from_slice(&reserved.to_le_bytes());
        record(
            sequence,
            game_hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &payload,
        )
    }

    /// One 16-byte d65 row, in the lane's row order
    /// (`attach_log.rs:65086`-`:65089`).
    fn row(owner: u32, program_id: u32, province: u32, unread: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity(d65::ROW_LEN);
        for word in [owner, program_id, province, unread] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out
    }

    /// A d65 DATA payload: the 24-byte header then the rows.
    fn d65_payload(status_flags: u32, rows: &[Vec<u8>]) -> Vec<u8> {
        d65_payload_with(status_flags, rows.len() as u32, 0, rows)
    }

    fn d65_payload_with(
        status_flags: u32,
        changed_rows: u32,
        dropped_ambiguous: u32,
        rows: &[Vec<u8>],
    ) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        payload.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
        payload.extend_from_slice(&changed_rows.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&dropped_ambiguous.to_le_bytes());
        assert_eq!(payload.len(), d65::HEADER_LEN);
        for r in rows {
            payload.extend_from_slice(r);
        }
        payload
    }

    fn one_row_payload(status_flags: u32, unread: u32) -> Vec<u8> {
        d65_payload(status_flags, &[row(68, 14, 12_627, unread)])
    }

    /// Two closed hours, each carrying one d65 DATA record.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D65,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 1),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D65,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 2),
        ));
        s.extend(frame_end(4, 101));
        s
    }

    /// The byte offset of the Nth record boundary in a stream of framed records.
    fn boundary(stream: &[u8], records: usize) -> usize {
        let mut pos = 0usize;
        for _ in 0..records {
            let h = parse_compact_record_header(&stream[pos..]).expect("framed");
            pos += COMPACT_RECORD_HEADER_LEN + h.payload_len;
        }
        pos
    }

    fn domain_data(sequence: u64, game_hour: i32) -> Vec<u8> {
        record(
            sequence,
            game_hour,
            D65,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 1),
        )
    }

    #[test]
    fn a_well_formed_stream_yields_its_d65_hours() {
        let scan = scan_stream(
            &good_stream(),
            &test_collection(),
            StreamIdentityMode::Legacy,
        )
        .expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].game_hour, 100);
        assert_eq!(scan.hours[1].game_hour, 101);
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.non_contiguous, 0);
        assert_eq!(scan.hours[0].state.len(), 1);
        // The key is the fold's pair in the fold's order, not row order.
        assert_eq!(
            scan.hours[0].state.keys().next().unwrap(),
            &wire_format::body::key(&[68, 14])
        );
        // The shipping byte split adds up to the records actually consumed.
        let c = &scan.coverage;
        assert_eq!(c.records, 2);
        assert_eq!(c.rows_walked, 2);
        assert_eq!(c.closed_hours, 2, "every closed hour carries its d65 record");
        // The shipping byte split is derived from the admitted hours, not from
        // these counters — see
        // `the_shipping_byte_split_describes_only_the_hours_in_the_shipping_arm`.
        let ship_bytes: usize = scan.hours.iter().map(|h| h.ship_span.len()).sum();
        assert_eq!(
            ship_bytes,
            2 * (COMPACT_RECORD_HEADER_LEN + d65::HEADER_LEN + d65::ROW_LEN)
        );
        assert_eq!(c.province_zero_rows, 0);
        assert_eq!(c.nonzero_status_records, 0);
        assert_eq!(c.changed_rows_unequal, 0);
        assert_eq!(c.dropped_ambiguous_records, 0);
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        let mut ok = domain_data(1, 100);
        ok.extend(frame_end(2, 100));
        assert!(scan_stream(&ok, &test_collection(), StreamIdentityMode::Legacy).is_ok());
        let mut bad = vec![
            frame_end_sealed(2, 100, 0, 0, 0, 1),
            frame_end_sealed(2, 100, 0, 1, 0, 0),
            frame_end_sealed(2, 100, 1, 0, 0, 0),
        ];
        for bit in 14..32u32 {
            bad.push(frame_end_sealed(2, 100, 0, 0, 1 << bit, 0));
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            bad.push(frame_end_sealed(2, 100, 0, 0, 1 << bit, 0));
        }
        for seal in bad {
            let mut s = domain_data(1, 100);
            s.extend(seal);
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }
        for len in [0usize, 1, 16, 31, 33, 64] {
            let mut s = domain_data(1, 100);
            s.extend(record(
                2,
                100,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                &vec![0; len],
            ));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }
        for status in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = domain_data(1, 100);
            s.extend(frame_end_sealed(2, 100, 0, 0, status, 0));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_ok());
        }
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        let mut ok = perf(1, 100, 20);
        ok.extend(perf(2, 100, D65));
        ok.extend(perf(3, 100, PROBE_DOMAIN_ID));
        ok.extend(domain_data(4, 100));
        ok.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
        assert!(scan_stream(&ok, &test_collection(), StreamIdentityMode::Legacy).is_ok());
        let mut duplicate = perf(1, 100, 20);
        duplicate.extend(perf(2, 100, 20));
        duplicate.extend(domain_data(3, 100));
        duplicate.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        assert!(scan_stream(&duplicate, &test_collection(), StreamIdentityMode::Legacy).is_err());
        // Repeated PROBE-domain PERF records stay accepted: the probe domain is
        // excluded from the duplicate rule and from the seal count.
        let mut probes = perf(1, 100, PROBE_DOMAIN_ID);
        probes.extend(perf(2, 100, PROBE_DOMAIN_ID));
        probes.extend(domain_data(3, 100));
        probes.extend(frame_end(4, 100));
        assert!(scan_stream(&probes, &test_collection(), StreamIdentityMode::Legacy).is_ok());
        // Two PERF records pending for DIFFERENT hours before any DATA opens one
        // must be rejected at the second: the pending hour is a single binding.
        let mut conflict = perf(1, 100, 20);
        conflict.extend(perf(2, 101, 21));
        conflict.extend(domain_data(3, 100));
        conflict.extend(frame_end_sealed(4, 100, 2, 2, 0, 0));
        assert!(scan_stream(&conflict, &test_collection(), StreamIdentityMode::Legacy).is_err());
        let mut interleaved = domain_data(1, 100);
        interleaved.extend(perf(2, 999, 20));
        interleaved.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&interleaved, &test_collection(), StreamIdentityMode::Legacy).is_err());
        let mut stale = domain_data(1, 100);
        stale.extend(frame_end(2, 100));
        stale.extend(perf(3, 100, 20));
        stale.extend(domain_data(4, 101));
        stale.extend(frame_end_sealed(5, 101, 1, 1, 0, 0));
        assert!(scan_stream(&stale, &test_collection(), StreamIdentityMode::Legacy).is_err());
        for (emitted, scheduled) in [(1, 1), (3, 3), (2, 3), (3, 2)] {
            let mut s = perf(1, 100, 20);
            s.extend(perf(2, 100, D65));
            s.extend(domain_data(3, 100));
            s.extend(frame_end_sealed(4, 100, emitted, scheduled, 0, 0));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }
        // Two valid consecutive hours REUSING the same PERF domain. This is what
        // pins the FRAME_END resets: without clearing the seen-domain set the
        // second hour's PERF would look like a duplicate, and without clearing
        // the pending hour it would look like a conflict.
        let mut two = perf(1, 100, 20);
        two.extend(domain_data(2, 100));
        two.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        two.extend(perf(4, 101, 20));
        two.extend(domain_data(5, 101));
        two.extend(frame_end_sealed(6, 101, 1, 1, 0, 0));
        assert_eq!(
            scan_stream(&two, &test_collection(), StreamIdentityMode::Legacy)
                .expect("PERF state resets at FRAME_END")
                .hours
                .len(),
            2
        );
    }

    #[test]
    fn a_data_record_must_open_the_hour_the_pending_perf_records_bound() {
        for (perf_hour, data_hour) in [(100, 101), (101, 100), (100, 200), (200, 100)] {
            let mut s = perf(1, perf_hour, 20);
            s.extend(domain_data(2, data_hour));
            s.extend(frame_end_sealed(3, data_hour, 1, 1, 0, 0));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }
        let mut agreeing = perf(1, 100, 20);
        agreeing.extend(domain_data(2, 100));
        agreeing.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&agreeing, &test_collection(), StreamIdentityMode::Legacy).is_ok());
    }

    #[test]
    fn the_identity_protocol_is_stated_not_sniffed() {
        use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;

        let decapitated = good_stream();
        assert!(scan_stream(
            &decapitated,
            &test_collection(),
            StreamIdentityMode::IdentityV1
        )
        .is_err());
        assert!(scan_stream(&decapitated, &test_collection(), StreamIdentityMode::Legacy).is_ok());
        let mut identity = record(
            0,
            0,
            0,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            b"CID1-not-a-real-identity-payload",
        );
        identity.extend(good_stream());
        assert!(scan_stream(&identity, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helper classifies
        // the v1 vocabulary and is not a validity predicate, so an unassigned bit
        // must be rejected BEFORE classification — otherwise it silently reads as
        // "not a failure" and the hour is measured as healthy.
        for bit in 14..32u32 {
            let status = 1u32 << bit;
            for extra in [0u32, 1, 1 << 10] {
                for rows in [Vec::new(), vec![row(68, 14, 12_627, 1)]] {
                    let mut s = Vec::new();
                    s.extend(record(
                        1,
                        100,
                        D65,
                        COMPACT_RECORD_TYPE_DOMAIN_DATA,
                        &d65_payload(status | extra, &rows),
                    ));
                    s.extend(frame_end(2, 100));
                    assert!(
                        scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                        "status bit {bit} (+{extra:#x}) with {} row(s) must be refused",
                        rows.len()
                    );
                }
            }
        }
        // And the whole known v1 mask, minus the failure bits, is still accepted:
        // the check must reject unassigned bits and nothing else.
        // `fold_special_project_programs` never reads an informational bit.
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D65,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(informational, 1),
            ));
            s.extend(frame_end(2, 100));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 1, "bit {informational:#x}");
            assert_eq!(scan.failure_records, 0, "bit {informational:#x}");
            assert_eq!(scan.coverage.nonzero_status_records, 1);
        }
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time:
        // `handle_compact_failure` (`world_fold.rs:12265`) retains the
        // accumulator and folds nothing, so the hour is not d65 data and must
        // not enter either arm.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D65,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(1 << bit, 1),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D65,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(0, 2),
            ));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .expect("the stream itself is well-formed");
            assert_eq!(scan.failure_records, 1, "bit {bit}");
            assert_eq!(scan.hours.len(), 1, "bit {bit}");
            assert_eq!(scan.hours[0].game_hour, 101, "bit {bit}");
            // The excluded record is still counted in coverage: it was walked.
            assert_eq!(scan.coverage.records, 2, "bit {bit}");
        }
        // PARTIAL_DATA (1<<9) is the bit d65's dead tolerance branch would have
        // caught (`world_fold.rs:12272`). Because
        // `COMPACT_EXPECTED_PARTIAL_TOLERANCES` has no domain-65 entry
        // (`compact_contract/src/lib.rs:136`-`:158`) and PARTIAL_DATA is inside
        // `COMPACT_STATUS_FAILURE_MASK` (`:117`), the record is an ordinary
        // excluded failure hour and the branch never runs. Pinned so that
        // adding a d65 tolerance entry upstream breaks this test rather than
        // silently changing what the measurement counts.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D65,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(1 << 9, 1),
        ));
        s.extend(frame_end(2, 100));
        s.extend(domain_data(3, 101));
        s.extend(frame_end(4, 101));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 1);
        // And a non-failure bit (DELTA_DATA, 1<<10) is NOT treated as failure.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D65,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(1 << 10, 1),
        ));
        s.extend(frame_end(2, 100));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn a_malformed_row_refuses_the_stream_whatever_the_records_status() {
        // The payload authority is status-independent
        // (`domain_data_authority.rs:12305`), so a zero owner or a zero
        // `program_id` invalidates the stream even inside a record whose status
        // excludes it from the measurement. Skipping it with its hour would
        // report a number for a stream production rejects.
        for bad in [
            row(0, 14, 12_627, 1),
            row(68, 0, 12_627, 1),
            row(0, 0, 0, 0),
        ] {
            for status in [0u32, 1, 1 << 9, 1 << 10] {
                let mut s = Vec::new();
                s.extend(record(
                    1,
                    100,
                    D65,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d65_payload(status, &[bad.clone()]),
                ));
                s.extend(frame_end(2, 100));
                s.extend(domain_data(3, 101));
                s.extend(frame_end(4, 101));
                assert!(
                    scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                    "a malformed row under status {status:#x} must refuse the stream"
                );
            }
        }
    }

    #[test]
    fn a_duplicate_pair_refuses_the_stream_even_in_a_failure_status_record() {
        // This is where d65 differs from d62. d62's authority does not check
        // duplicates, so a duplicate inside a failure-status record is excluded
        // rather than refused (`bench_real_d62.rs:1411`). d65's authority DOES
        // check them (`domain_data_authority.rs:12356`), status-independently,
        // so the same input must refuse the stream here. Copying d62's rule
        // across would accept a stream production rejects.
        let dup = [row(68, 14, 12_627, 1), row(68, 14, 12_627, 2)];
        for status in [0u32, 1, 1 << 9, 1 << 10] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D65,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d65_payload(status, &dup),
            ));
            s.extend(frame_end(2, 100));
            s.extend(domain_data(3, 101));
            s.extend(frame_end(4, 101));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "a duplicate pair under status {status:#x} must refuse the stream"
            );
        }
        // A repeated owner under different programs is not a duplicate.
        let ok = [row(68, 14, 12_627, 1), row(68, 15, 12_628, 2)];
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D65,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d65_payload(0, &ok),
        ));
        s.extend(frame_end(2, 100));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours[0].state.len(), 2);
    }

    #[test]
    fn changed_rows_greater_than_row_count_refuses_the_stream_and_equality_is_not_required() {
        // The authority's bound (`domain_data_authority.rs:12335`) is `<=`, and
        // the fold reads the word not at all. Both halves are pinned: a strictly
        // greater word refuses, and an unequal-but-bounded one is measured
        // normally rather than being mistaken for a partial frame.
        let rows = [row(68, 14, 12_627, 1), row(68, 15, 12_628, 2)];
        for changed in [3u32, 4, u32::MAX] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D65,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d65_payload_with(0, changed, 0, &rows),
            ));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "changed_rows {changed} > row_count 2"
            );
        }
        for changed in 0..=2u32 {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D65,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d65_payload_with(0, changed, 0, &rows),
            ));
            s.extend(frame_end(2, 100));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("changed_rows {changed}: {e}"));
            assert_eq!(scan.hours.len(), 1, "changed_rows {changed}");
            assert_eq!(scan.hours[0].state.len(), 2, "changed_rows {changed}");
            assert_eq!(
                scan.coverage.changed_rows_unequal,
                usize::from(changed != 2),
                "changed_rows {changed}"
            );
        }
    }

    #[test]
    fn the_diagnostic_dropped_ambiguous_word_never_changes_the_measurement() {
        // `world_fold.rs:12317`-`:12321` reads it only to emit a note. It must
        // be reported and must not gate, exclude or alter any row.
        let rows = [row(68, 14, 12_627, 1)];
        for dropped in [0u32, 1, 7, u32::MAX] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D65,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d65_payload_with(0, 1, dropped, &rows),
            ));
            s.extend(frame_end(2, 100));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("dropped_ambiguous {dropped}: {e}"));
            assert_eq!(scan.hours.len(), 1);
            assert_eq!(scan.hours[0].state.len(), 1);
            assert_eq!(
                scan.coverage.dropped_ambiguous_records,
                usize::from(dropped != 0)
            );
        }
    }

    #[test]
    fn a_zero_row_record_is_a_measured_empty_state_not_a_skipped_hour() {
        // A clean walk that finds no facility is a successful zero-row
        // replacement (`attach_log.rs:64846`-`:64850`), and the fold folds it as
        // one. Treating it as "nothing to measure" would drop a real state
        // transition — every country losing its last facility.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D65,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d65_payload(0, &[]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(domain_data(3, 101));
        s.extend(frame_end(4, 101));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 2);
        assert!(scan.hours[0].state.is_empty());
        assert_eq!(scan.hours[1].state.len(), 1);
        assert_eq!(scan.coverage.rows_walked, 1);
    }

    #[test]
    fn a_province_of_zero_is_measured_and_counted_not_refused() {
        // The collector applies no range check to `province`
        // (`attach_log.rs:65006`-`:65013` only requires the two copies to
        // agree), and the fold folds it raw, so a zero province is data. The
        // harness counts it as a coverage fact rather than gating on it.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D65,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d65_payload(0, &[row(68, 14, 0, 1)]),
        ));
        s.extend(frame_end(2, 100));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.coverage.province_zero_rows, 1);
    }

    #[test]
    fn a_second_d65_record_in_one_hour_is_rejected() {
        // d65 is full-replace; two records in one hour would become two
        // measurement hours built from the same clock time. The rule is checked
        // over EVERY d65 DATA record and before status-based exclusion, so all
        // four success/failure orderings are refused.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D65,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(first, 1),
            ));
            s.extend(record(
                2,
                100,
                D65,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(second, 2),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "two d65 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_malformed_d65_payload_is_refused_not_skipped() {
        // Every header-level rejection `d65::split_payload` owns, reached
        // through the scanner so the refusal is the stream's, not a unit test's.
        let full = one_row_payload(0, 1);
        let mut cases: Vec<Vec<u8>> = Vec::new();
        // Short of the header.
        for len in 0..d65::HEADER_LEN {
            cases.push(full[..len].to_vec());
        }
        // Length disagreeing with row_count, both directions.
        cases.push(full[..full.len() - 1].to_vec());
        let mut long = full.clone();
        long.push(0);
        cases.push(long);
        // A `row_count` far larger than the payload. On a 64-bit target
        // `24 + u32::MAX * 16` fits `usize`, so this is an ordinary length
        // mismatch and would fail without the `checked_mul` in `split_payload`;
        // that checked arithmetic guards a 32-bit target, where the same input
        // wraps to a plausible small length. This case does not exercise that
        // branch and does not claim to.
        let mut huge = full.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        cases.push(huge);
        for payload in cases {
            let mut s = record(1, 100, D65, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload);
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "payload of {} byte(s) must refuse the stream",
                payload.len()
            );
        }
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        // A cut immediately after a d65 DATA record leaves that hour open and
        // ends the stream on a record that is not a FRAME_END.
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&full[..cut], &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary IS a well-formed shorter stream,
        // and `eof_check` accepts it. Refusing it would be wrong; what matters
        // is that it reports the hours it actually closed.
        for (records, expected_hours) in [(2usize, 1usize), (4, 2)] {
            let cut = boundary(&full, records);
            let scan = scan_stream(&full[..cut], &test_collection(), StreamIdentityMode::Legacy)
                .expect("closed stream is well-formed");
            assert_eq!(scan.hours.len(), expected_hours, "cut after {records}");
        }
    }

    #[test]
    fn a_stream_whose_final_record_is_not_a_frame_end_is_rejected() {
        // `eof_check`'s last clause, which none of the truncation fixtures above
        // reach: they all leave a DATA hour open, so the "hour left open" clause
        // fires first. Here hour 100 is complete and closed, and the stream then
        // ends on a PERF record that opens nothing — so the ONLY thing wrong is
        // that the final physical record is not a canonical FRAME_END.
        let mut s = domain_data(1, 100);
        s.extend(frame_end(2, 100));
        s.extend(perf(3, 101, 20));
        let err = match scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy) {
            Err(e) => e,
            Ok(_) => panic!("a stream ending on a PERF record must be refused"),
        };
        assert!(
            err.contains("final record must be a canonical FRAME_END"),
            "the EOF final-record rule must be what rejects it: {err}"
        );

        // The same stream WITH its closing FRAME_END is accepted, so the fixture
        // isolates that one rule and nothing else.
        let mut closed = s.clone();
        closed.extend(domain_data(4, 101));
        closed.extend(frame_end_sealed(5, 101, 1, 1, 0, 0));
        assert_eq!(
            scan_stream(&closed, &test_collection(), StreamIdentityMode::Legacy)
                .expect("the closed form is well-formed")
                .hours
                .len(),
            2
        );
    }

    #[test]
    fn a_stream_ending_mid_record_is_rejected() {
        let full = good_stream();
        let boundaries: Vec<usize> = (0..=4).map(|n| boundary(&full, n)).collect();
        for cut in 1..full.len() {
            if boundaries.contains(&cut) {
                continue;
            }
            assert!(
                scan_stream(&full[..cut], &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "mid-record cut at {cut}"
            );
        }
        assert!(
            scan_stream(&[], &test_collection(), StreamIdentityMode::Legacy).is_err(),
            "an empty stream is not a measurement"
        );
    }

    #[test]
    fn a_header_the_canonical_parser_rejects_is_rejected_here() {
        for records in 0..4 {
            let at = boundary(&good_stream(), records);
            for (offset, patch) in [
                (0usize, vec![b'X']),              // magic
                (4, 2u16.to_le_bytes().to_vec()),  // unsupported version
                (6, 33u16.to_le_bytes().to_vec()), // unsupported header length
            ] {
                let mut s = good_stream();
                s[at + offset..at + offset + patch.len()].copy_from_slice(&patch);
                assert!(
                    scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                    "record {records} patched at +{offset} must be refused"
                );
            }
        }
    }

    #[test]
    fn a_sequence_discontinuity_is_rejected() {
        for records in 0..4 {
            let at = boundary(&good_stream(), records);
            for wrong in [0u64, 9] {
                let mut s = good_stream();
                s[at + 8..at + 16].copy_from_slice(&wrong.to_le_bytes());
                assert!(
                    scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                    "record {records} sequence {wrong}"
                );
            }
        }
    }

    #[test]
    fn a_frame_end_that_does_not_close_its_open_hour_is_rejected() {
        let mut s = domain_data(1, 100);
        s.extend(frame_end(2, 999)); // closes an hour that is not open
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());

        let mut s = Vec::new();
        s.extend(frame_end(1, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        let mut s = domain_data(1, 101);
        s.extend(frame_end(2, 101));
        s.extend(domain_data(3, 100));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_stream_with_no_d65_records_is_refused_not_reported_as_zero() {
        let mut s = record(1, 100, 20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &[0u8; 8]);
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_hour_that_loses_its_d65_record_refuses_the_stream() {
        // Production validates every domain's DATA payload
        // (`stream_fold.rs:593`); this harness can only validate d65's, so a d65
        // record relabelled to another domain — or dropped — would otherwise be
        // ignored and the run would report a plausible measurement over fewer
        // hours. The completeness check in `scan_stream` closes that hole.
        //
        // Relabelled: hour 101's d65 record becomes a d20 record. The stream
        // stays well formed — sequence, hour and FRAME_END are untouched.
        let mut relabelled = Vec::new();
        relabelled.extend(domain_data(1, 100));
        relabelled.extend(frame_end(2, 100));
        relabelled.extend(record(
            3,
            101,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 2),
        ));
        relabelled.extend(frame_end(4, 101));
        let err = match scan_stream(&relabelled, &test_collection(), StreamIdentityMode::Legacy) {
            Err(e) => e,
            Ok(_) => panic!("an hour with no d65 record must refuse the stream"),
        };
        assert!(err.contains("closes 2 game hour(s) but carries 1"), "{err}");

        // Dropped from the MIDDLE of the stream, so the check is shown to fire
        // on position as well as on count: hours 100 and 102 carry their d65
        // record, hour 101 carries only another domain's DATA record. Every hour
        // is structurally well formed — a DOMAIN_DATA record opens it and a
        // FRAME_END closes it — so the missing d65 record is the only thing
        // wrong, and the completeness check is the only rule that can catch it.
        let mut dropped = Vec::new();
        dropped.extend(domain_data(1, 100));
        dropped.extend(frame_end(2, 100));
        dropped.extend(record(
            3,
            101,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 2),
        ));
        dropped.extend(frame_end(4, 101));
        dropped.extend(domain_data(5, 102));
        dropped.extend(frame_end(6, 102));
        let err = match scan_stream(&dropped, &test_collection(), StreamIdentityMode::Legacy) {
            Err(e) => e,
            Ok(_) => panic!("an hour carrying no d65 record must refuse the stream"),
        };
        assert!(
            err.contains("closes 3 game hour(s) but carries 2"),
            "the completeness check must be what rejects it, not an earlier rule: {err}"
        );

        // And the honest case still passes: one d65 record per closed hour,
        // including one whose status excludes it from the measurement.
        let mut with_failure = Vec::new();
        with_failure.extend(record(
            1,
            100,
            D65,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(1, 1),
        ));
        with_failure.extend(frame_end(2, 100));
        with_failure.extend(domain_data(3, 101));
        with_failure.extend(frame_end(4, 101));
        let scan = scan_stream(&with_failure, &test_collection(), StreamIdentityMode::Legacy)
            .expect("a failure record still occupies its hour");
        assert_eq!(scan.coverage.closed_hours, 2);
        assert_eq!(scan.coverage.records, 2);
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn the_shipping_byte_split_describes_only_the_hours_in_the_shipping_arm() {
        // An excluded failure record is walked, so it counts in coverage — but
        // its bytes are NOT in the shipping arm. Deriving the printed
        // decomposition from the all-record counters would therefore describe
        // two records while the arm held one. The split is derived from the
        // admitted hours instead, and this pins the identity `main` asserts.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D65,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d65_payload(1, &[row(68, 14, 12_627, 1), row(68, 15, 12_628, 2)]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(domain_data(3, 101));
        s.extend(frame_end(4, 101));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 1, "only the clean hour is in the arm");
        // Coverage spans both records; the arm spans one.
        assert_eq!(scan.coverage.records, 2);
        assert_eq!(scan.coverage.rows_walked, 3);

        // Through `shipping_split_of`, which is the whole of what `main` calls —
        // so the counter-provenance decision is INSIDE the function under test.
        // Rewriting it to use `scan.coverage` fails here.
        let ship_bytes: usize = scan.hours.iter().map(|h| h.ship_span.len()).sum();
        let (framing, headers, rows) = shipping_split_of(&scan);
        assert_eq!(
            framing + headers + rows,
            ship_bytes,
            "the split must sum to the shipping arm it describes"
        );
        assert_eq!((framing, headers, rows), shipping_split(1, 1));

        // And the fixture provably distinguishes the two derivations: the
        // coverage counters describe both records and cannot reproduce the arm.
        let (wrong_framing, wrong_headers, wrong_rows) =
            shipping_split(scan.coverage.records, scan.coverage.rows_walked);
        assert_ne!(
            wrong_framing + wrong_headers + wrong_rows,
            ship_bytes,
            "this fixture must actually distinguish the two derivations"
        );
    }

    #[test]
    fn a_measured_hour_after_a_gap_opens_a_baseline() {
        // An excluded hour between two measured ones must not be crossed by a
        // delta: the two states are not consecutive in game time, and where they
        // are equal the arm would otherwise emit nothing and silently assert
        // "unchanged" over an interval nobody measured.
        let mut s = Vec::new();
        s.extend(domain_data(1, 100)); // measured
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D65,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(1, 9), // failure status -> excluded
        ));
        s.extend(frame_end(4, 101));
        s.extend(domain_data(5, 102)); // measured, but does NOT follow 100
        s.extend(frame_end(6, 102));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.non_contiguous, 1);
        assert!(!scan.hours[0].follows_previous, "the first hour follows none");
        assert!(
            !scan.hours[1].follows_previous,
            "hour 102 does not follow hour 100"
        );
        // ...and that is what makes the encoder open a baseline for it, even
        // though it is only the second measured hour of the block.
        assert!(opens_baseline(1, scan.hours[1].follows_previous));

        // The decision function over its whole input space: a block boundary
        // always opens one, and so does any gap, at every position in a block.
        for hours_seen in 0..(2 * BLOCK_HOURS) {
            assert!(
                opens_baseline(hours_seen, false),
                "a gap at position {hours_seen} must open a baseline"
            );
            assert_eq!(
                opens_baseline(hours_seen, true),
                hours_seen % BLOCK_HOURS == 0,
                "contiguous position {hours_seen}"
            );
        }

        // A contiguous run is unaffected: only the block boundary baselines.
        let contiguous = good_stream();
        let scan = scan_stream(&contiguous, &test_collection(), StreamIdentityMode::Legacy)
            .expect("well-formed");
        assert_eq!(scan.non_contiguous, 0);
        assert!(scan.hours[1].follows_previous);
        assert!(!opens_baseline(1, scan.hours[1].follows_previous));
    }

    #[test]
    fn scan_stream_never_panics_on_arbitrary_bytes() {
        // The scanner reads a file; a panic there is an unrecoverable
        // measurement rather than a diagnosis. Fuzz both free-form bytes and
        // single-byte corruptions of a valid stream, which is the shape that
        // reaches the deepest parsing paths.
        let collection = test_collection();
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..2_048 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state % 256) as usize;
            let bytes: Vec<u8> = (0..len).map(|i| (state >> ((i % 8) * 8)) as u8).collect();
            let _ = scan_stream(&bytes, &collection, StreamIdentityMode::Legacy);
        }
        let good = good_stream();
        for i in 0..good.len() {
            for patch in [0x00u8, 0xFF, 0x7F] {
                let mut s = good.clone();
                s[i] = patch;
                let _ = scan_stream(&s, &collection, StreamIdentityMode::Legacy);
            }
        }
    }
}
