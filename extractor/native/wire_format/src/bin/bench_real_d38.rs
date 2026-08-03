//! Measure the wire format against the shipping compact stream on REAL d38
//! `air_superiority_inputs` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d38-scheduled capture
//! (`runs/batch2/deepwar_t1`). The stream's own d38 DATA records are the shipping
//! format's cost for the domain: a 52-byte header plus a fixed 48-byte row per
//! *changed* `(region, owner, mission, variant)` bucket, sparse after the first
//! frame, exactly what the recorder stores today. This tool replays those records
//! through `wire_format::d38::Accumulator` — the same fold
//! `world_fold::fold_air_superiority_inputs` performs — into per-hour states, and
//! re-encodes every hour with this crate's grammar: a BASELINE at each 48-hour
//! block start, field-granular stored-minimal DELTAs between.
//!
//! **d38 is incremental, so one record is not one state.** The shipping arm carries
//! a few changed rows per hour and the wire arm's delta carries the diff of two
//! reconstructed states; the two therefore measure the same thing — the cost of
//! transporting one hour of change — but only the wire arm ever re-sends a whole
//! state. That asymmetry is what the baseline/delta split in the output is for: the
//! block ratio measures re-baseline policy, the delta bytes measure the grammar.
//!
//! **Nothing here re-derives a contract it can call.** Header, row and frame
//! decoding go through `wire_format::d38`'s public boundary
//! (`peek_header`/`split_payload`/`parse_row`/`record_from_row`/`Accumulator`);
//! H4CR framing, record semantics and sequence continuity go through
//! `compact_contract`'s own parser and stateful validator. A harness that
//! reimplements either drifts from the recorder without anything failing.
//!
//! **The whole stream is validated before any artifact is written.** A stream that
//! is truncated, spliced, or corrupt would otherwise yield a smaller but entirely
//! plausible measurement — a silently wrong number, which is worse than a crash.
//! `scan_stream` therefore enforces, fail-closed:
//!
//! - every record header through `compact_contract::parse_compact_record_header`
//!   (magic, version, header length, payload bound);
//! - every record through `compact_contract::stream_validator::StatefulStreamValidator`
//!   (sequence continuity from the stream's first record, record-type and
//!   record-flag semantics, FRAME_END shape);
//! - the hour boundary rules of `recorder_host::stream_fold::BoundaryTracker`
//!   (`recorder_host/src/stream_fold.rs:277`): a DOMAIN_DATA record opens its game
//!   hour, a FRAME_END closes the open hour and only that hour, and closed hours
//!   strictly increase;
//! - that tracker's **DOMAIN_PERF hour binding** (`:328`-`:365`): no duplicate
//!   non-probe PERF domain inside one hour, no PERF for an hour other than the open
//!   one, no PERF for an hour already closed, and no conflict with the pending PERF
//!   hour before a DATA record opens one;
//! - the **FRAME_END seal fields**, `validate_frame_end_seal_fields`
//!   (`:46`-`:89`), re-implemented here as [`check_frame_end_seal`]: the 32-byte
//!   payload length, `reserved == 0`, `emitted_domains == scheduled_domains`, no
//!   status bit above 13, and no fatal status bit; plus the seal's **domain-count
//!   cross-check** against the distinct non-probe PERF domains actually observed in
//!   the hour (`:411`-`:419`). A seal violation is a `violation` in the production
//!   tracker (`:402`-`:409`), i.e. a refused stream, so it is one here too;
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the full
//! authority. **Exactly one production check is outside this subset, and it is a
//! crate-boundary limitation rather than a choice**: production calls
//! `validate_domain_data_payload_at` on EVERY DOMAIN_DATA record (`:593`), decoding
//! each domain's payload grammar through `recorder_host::domain_data_authority`, so
//! a stream carrying a malformed payload for some *other* domain is refused there
//! and accepted here. `wire_format` cannot depend on `recorder_host` (it pulls in
//! every capture lane) and this crate implements one domain's grammar, so the only
//! DOMAIN_DATA payload this harness can validate is d38's — which it does, fully
//! and fail-closed. The measured capture is a recorder-validated run, so its other
//! domains' payloads have been through that check; a hand-made stream carrying a
//! corrupt non-d38 payload would measure here and not there.
//!
//! Both the seal fields and the PERF hour binding were review findings against this
//! harness (rounds 1 and 2). Neither moved a measured number: the capture's 146
//! FRAME_END records all carry `emitted == scheduled == 23`, `status == 0`,
//! `reserved == 0`, its 3,505 PERF records show 23 distinct non-probe domains per
//! hour plus the probe domain, and it has zero hour-binding violations.
//!
//! **The two paths on which `fold_air_superiority_inputs` returns without folding**
//! are excluded, at the position the fold puts them
//! (`world_fold.rs:15433`-`:15446`), and neither is a stream rejection — the fold
//! returns `Ok` on both, so the hour simply carries no d38 data:
//!
//! 1. **failure status** — after the six header-structural checks and before the
//!    exact-length check, so `d38::peek_header` runs on EVERY d38 record and
//!    `d38::split_payload` only on the ones that are folded. The predicate is
//!    `compact_contract`'s own
//!    `compact_status_failure_flags_after_expected_partial_tolerance`, never a
//!    hand-copied bit mask: d38 has no expected-partial tolerance, so the whole of
//!    `COMPACT_STATUS_FAILURE_MASK` (`COMPACT_STATUS_PARTIAL_DATA` included) counts.
//! 2. **delta before baseline** — owned by `d38::Accumulator::fold_payload`, which
//!    holds the `baseline_seen` state the rule consults.
//!
//! The third outcome, a **stream rejection**, is everything else: a payload the fold
//! would `Err` on (a short header, any of the six header invariants, a length that
//! disagrees with `row_count` once the exclusions have passed, an unknown row flag
//! bit, a non-zero row reserved word) invalidates the run rather than skipping an
//! hour, because the recorder would have aborted the stream fold on it.
//!
//! Hour uniqueness is enforced across ALL d38 DATA records and BEFORE the status
//! exclusion — a success + failure pair in one hour would otherwise slip through the
//! rule entirely. **This is the harness's one deliberate strictness beyond the
//! production boundary**, which accepts a second DOMAIN_DATA record for the open hour
//! (`stream_fold.rs:369`) and folds both; the measurement model here is one hour =
//! one state = one wire record, and a second record for one hour has no
//! representation on the wire side. It is recorded at the check itself, with what it
//! would take to lift it.
//!
//! **Other writers of the accumulator: none.** `fold_air_superiority_inputs` is the
//! only function in `world_fold.rs` that writes `self.air_superiority_inputs`; the
//! projection at `:9074` reads it. No other domain prunes it (unlike d26's
//! accumulator) and no end-of-hour orphan sweep touches it (unlike d32's), so there
//! is nothing to model, prove inert, or assert inert per capture.
//!
//! **One caveat on the block shape that this harness cannot fix alone**: baselines
//! are placed on GAME-CLOCK 48-hour boundaries (see `block_index` in `main`), while
//! `scripts/compress_arms.py` slices its spans by *offset-list index*, i.e. per
//! measured hour. On a capture with no excluded or missing hours — this one, 146
//! contiguous hours — the two indexings coincide exactly. On a capture with excluded
//! hours they would drift, and the block-shape ratio would compress spans whose
//! edges are not the edges the baselines were written for. Fixing that means giving
//! the shared script a clock-aware span mode, which is a cross-domain change every
//! ported domain shares rather than something this bin should do privately.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d38.bin   — the d38 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface twice — cadence-independent and along the
//! delta path — beside the shipping stream's own changed-row counts.
//!
//! **Only d38 DATA payloads are validated.** Production runs
//! `validate_domain_data_payload_at` on the DATA payload of EVERY domain
//! (`stream_fold.rs:593`); this harness cannot, because validating another
//! domain's payload means owning that domain's authority, and `wire_format`
//! must not depend on `recorder_host`. Every DATA record of any other domain
//! therefore passes with its H4CR header, sequence continuity, hour-boundary,
//! PERF-binding and FRAME_END rules checked but its **payload bytes unread**.
//! The concrete hazard — a d38 record relabelled to another domain id, or
//! dropped outright, silently shrinking the measurement — is closed by an
//! explicit completeness check in `scan_stream`: every closed game hour must
//! carry exactly one d38 DATA record, of any status. That is a per-capture
//! assertion, not a grammar rule; a capture where d38 is not scheduled every
//! hour fails loudly there and a human adjusts.

use std::collections::BTreeSet;
use std::io::Write;
use std::ops::Range;

use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, compact_status_unknown_flags_v1,
    parse_compact_record_header, COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_DELTA_DATA, COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedState,
};
use wire_format::d38;

const D38: u16 = 38;
const BLOCK_HOURS: usize = 48;

/// Whether the wire arm opens a BASELINE for this measured hour.
///
/// Two reasons, and the second is not a cadence choice: the game-clock block
/// boundary the caller computed (`block_start` — see `main` for why d38's
/// blocks are measured on the clock, not the record count), or a measured hour
/// that does not immediately follow the previous measured one. An hour this
/// domain did not successfully sample is coverage the domain lost, which is
/// §8's gap, and `WIRE_FORMAT.md:187`-`:190` is explicit: "A gap affecting
/// the state part forces the next state observation to be a `BASELINE`. State
/// may have moved and returned while nobody was looking, so neither a delta
/// nor an `UNCHANGED` can be justified across unobserved time." Delta-encoding
/// across the gap would produce bytes that decode — the reader's prior is
/// never invalidated, so the local replay cannot notice — while asserting a
/// continuity the capture does not have. The delta chain restarts at the
/// forced baseline.
fn opens_baseline(
    clock_block_start: bool,
    previous_measured_hour: Option<i32>,
    game_hour: i32,
) -> bool {
    if clock_block_start {
        return true;
    }
    match previous_measured_hour {
        Some(previous) => game_hour != previous.saturating_add(1),
        None => true,
    }
}

/// One usable d38 hour: its game hour, the accumulated state after folding that
/// hour's record, and the record's byte span in the source stream (the shipping arm
/// is those bytes verbatim, header included).
///
/// The state is built during the scan, not later: every way a row can be rejected —
/// an unknown flag bit, a non-zero reserved word, a payload whose length disagrees
/// with its row count — is a property of the stream, so it belongs to the pass that
/// either accepts the stream or refuses it.
struct D38Hour {
    game_hour: i32,
    state: KeyedState,
    ship_span: Range<usize>,
    /// The frame's own counts, for the shipping-side churn report.
    shipped_rows: u32,
    shipped_deletes: u32,
}

struct StreamScan {
    hours: Vec<D38Hour>,
    /// d38 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// d38 delta records that arrived before any baseline (`world_fold.rs:15440`).
    delta_before_baseline: usize,
    /// Measured d38 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// How many records were full replaces (`FoldOutcome::FullReplace`). The fold
    /// clears only on the first, so anything but 1 on a healthy stream is news.
    full_replaces: usize,
}

/// Every globally-defined compact status occupies one of bits 0..=13. FRAME_END
/// carries the frame-wide aggregate, so lane-local interpretations do not apply
/// there and any higher bit is an unknown wire-contract extension
/// (`recorder_host/src/stream_fold.rs:43`).
// Re-exported, not re-declared: a local `(1 << 14) - 1` became wrong the moment
// the contract assigned bit 14 to the measured-empty badge.
const FRAME_END_KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;

/// The reader-context probe domain, excluded from the seal's domain accounting
/// (`recorder_host/src/stream_fold.rs:330`; the collector's own
/// `RC_PROBE_DOMAIN_ID`, `collector/src/attach_log.rs:87316`).
const PROBE_DOMAIN_ID: u16 = 250;

/// The FRAME_END seal check, re-implemented from
/// `recorder_host::stream_fold::validate_frame_end_seal_fields`
/// (`recorder_host/src/stream_fold.rs:46`-`:89`) because `wire_format` must not
/// depend on `recorder_host`.
///
/// Wire layout: `elapsed_wall_ms:u64`, `frame_time_us:u64`, `emitted_domains:u32`,
/// `scheduled_domains:u32`, `status_flags:u32`, `reserved:u32`. The two timing
/// fields carry no seal-time predicate; the rest are authoritative. A violation is
/// a refused stream, exactly as it is in production (`:402`-`:409`) — a corrupt seal
/// means the hour did not close cleanly, and measuring the hours behind it would
/// report a plausible number for a stream the recorder rejects.
///
/// Returns the seal's `(emitted_domains, scheduled_domains)` so the caller can run
/// production's cross-check against the PERF domains it actually observed
/// (`:411`-`:419`).
///
/// Total: no payload of any length panics.
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

/// The d38 checks production's **second** authority makes on every DOMAIN_DATA
/// record — status-independent, and therefore also on the records this harness
/// excludes — that the fold's own header set (`d38::peek_header`) does not already
/// imply.
///
/// Production runs two independent passes over a d38 DATA record and a stream is
/// refused if EITHER complains:
///
/// 1. `WorldFold::ingest_record` → `fold_air_superiority_inputs`, whose `Err`
///    aborts the stream fold. Its header checks run before the status early exits
///    and are what `d38::peek_header` mirrors; its exact-length and row checks run
///    after them, so an excluded record never reaches those.
/// 2. `validate_domain_data_payload_at` (`recorder_host/src/stream_fold.rs:594`,
///    `:1059`) → `decode_compact_air_superiority_inputs_data_payload`
///    (`domain_data_authority.rs:15247`-`:15303`). This one runs on EVERY
///    DOMAIN_DATA record whatever its status, and any error it reports makes the
///    record `malformed_payload_records`, which
///    `strict_stream_certification_verdict` (`stream_fold.rs:507`-`:510`) refuses.
///
/// Round-4 review finding: excluding a record before checking its exact length let
/// a stream production refuses produce a plausible measurement. Two of the second
/// authority's checks are not implied by the first authority's header set, and both
/// are applied here to every record:
///
/// - the **exact payload length** (`domain_data_authority.rs:15303`), which in the
///   fold runs only after its status exits;
/// - `row_count > observed_equipment_rows` when `DELTA_DATA` is clear
///   (`:15281`), which is neither implied by nor implies the fold's
///   `row_count <= observed_equipment_rows + deleted_rows`.
///
/// **Where this deliberately stops:** the second authority does **not** walk d38
/// rows at all — it reads the header, checks the counts and the length, and
/// accumulates totals (`:15247`-`:15330`). So an excluded record carrying an unknown
/// row flag bit or a non-zero row reserved word is a stream production certifies
/// (the fold returned before its row loop, and the authority never looks at rows),
/// and refusing it here would be stricter than production — which is its own defect.
/// Those row checks therefore stay where the fold puts them, inside
/// `Accumulator::fold_payload`, and apply only to records that are actually folded.
fn check_authority_payload(
    payload: &[u8],
    header: &d38::PayloadHeader,
    at_byte: usize,
) -> Result<(), String> {
    // `domain_data_authority.rs:15281`.
    if header.status_flags & COMPACT_STATUS_DELTA_DATA == 0
        && header.row_count > header.observed_equipment_rows
    {
        return Err(format!(
            "d38 payload at byte {at_byte}: row_count {} > observed_equipment_rows {} on a \
             non-delta record",
            header.row_count, header.observed_equipment_rows
        ));
    }
    // `domain_data_authority.rs:15300`-`:15303`. Checked-multiplied rather than
    // saturating, which rejects the same payloads by a more direct route.
    let expected = (header.row_count as usize)
        .checked_mul(d38::ROW_LEN)
        .and_then(|len| len.checked_add(d38::HEADER_LEN))
        .ok_or_else(|| {
            format!(
                "d38 payload at byte {at_byte}: row_count {} overflows",
                header.row_count
            )
        })?;
    if payload.len() != expected {
        return Err(format!(
            "d38 payload at byte {at_byte}: length {} != {expected} (52 + 48 * row_count {})",
            payload.len(),
            header.row_count
        ));
    }
    Ok(())
}

/// Validate the whole stream against the canonical contract and fold the usable d38
/// DATA records into per-hour states. Every rejection path is an `Err`, never a
/// silently shorter measurement and never a panic.
fn scan_stream(
    stream: &[u8],
    collection: &wire_format::Collection,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    // The library names the two status constants rather than importing
    // `compact_contract`; this is where the two are checked to agree, so the
    // library's copies cannot drift from the contract unnoticed.
    if d38::STATUS_DELTA_DATA != COMPACT_STATUS_DELTA_DATA {
        return Err("d38::STATUS_DELTA_DATA disagrees with compact_contract".to_string());
    }
    if d38::FAILURE_STATUS_MASK != COMPACT_STATUS_FAILURE_MASK {
        return Err("d38::FAILURE_STATUS_MASK disagrees with compact_contract".to_string());
    }

    // The identity protocol is an INPUT, never sniffed. `StatefulStreamValidator`
    // deliberately never sniffs (`compact_contract/src/stream_validator.rs:328`) and
    // production takes the mode from its caller
    // (`recorder_host/src/stream_fold.rs:220`), for a reason this harness has to
    // honour: an `IdentityV1` stream whose mandatory CID1 record has been spliced off
    // is byte-indistinguishable from a legacy stream — both begin with an ordinary
    // record at sequence 1. A harness that sniffs therefore accepts a decapitated
    // identity stream as a healthy legacy one and measures it, which production
    // (invoked with `IdentityV1`) refuses as `WrongRecordSequence(1)`.
    // Round-5 review finding: the mode is now a typed parameter, so the caller
    // states which protocol the capture is and a mismatch fails closed in both
    // directions — a legacy call on an identity stream trips on the CID1 record, an
    // identity call on a legacy stream trips on the missing one.
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours: Vec<D38Hour> = Vec::new();
    let mut failure_records = 0usize;
    let mut delta_before_baseline = 0usize;
    let mut non_contiguous = 0usize;
    let mut full_replaces = 0usize;

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    // The DOMAIN_PERF hour-binding state, `BoundaryTracker`'s own
    // (`stream_fold.rs:328`-`:365`, reset at `:449`-`:450`).
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d38 DATA record of ANY status — the uniqueness rule.
    let mut last_d38_hour: Option<i32> = None;
    // Hour of the last d38 record that entered the measurement — the contiguity
    // report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;

    // The shipping fold's own accumulator, replayed through the domain module.
    let mut acc = d38::Accumulator::new();

    // Closed game hours and d38 DATA records of ANY status — the
    // completeness rule at the end of the scan.
    let mut closed_hours = 0usize;
    let mut d38_records = 0usize;
    let mut pos = 0usize;
    while pos < stream.len() {
        let header = parse_compact_record_header(&stream[pos..])
            .map_err(|e| format!("record at byte {pos}: {e}"))?;
        let payload_start = pos + COMPACT_RECORD_HEADER_LEN;
        let record_end = payload_start
            .checked_add(header.payload_len)
            .ok_or_else(|| format!("record at byte {pos}: length overflow"))?;
        if record_end > stream.len() {
            // `eof_check`: a stream ending mid-record is not a short stream, it is a
            // corrupt one.
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
            // `stream_fold.rs:328`-`:365`. PERF records carry no d38 data, but they
            // bind the hour: a duplicate non-probe domain, a PERF for an hour other
            // than the open one, a PERF for an already-closed hour, or two different
            // pending hours before any DATA record opens one are each a refused
            // stream in production, so each is one here. (Round-2 review finding.)
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
                    // `stream_fold.rs:378`-`:386`, and it comes BEFORE the
                    // monotonicity check, as it does there: a DATA record may only
                    // open the hour the pending PERF records already bound.
                    // (Round-3 review finding — the PERF arm alone does not cover
                    // this, because a single pending PERF hour is legal until a DATA
                    // record disagrees with it.)
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
                // The seal fields come first, as they do in the production tracker
                // (`stream_fold.rs:401`-`:409`): a FRAME_END that does not seal is a
                // refused stream whatever hour it claims to close.
                let (emitted_domains, scheduled_domains) = check_frame_end_seal(payload, pos)?;
                // `:411`-`:419`: the seal's counts must equal the distinct non-probe
                // PERF domains actually observed in the hour it closes.
                let observed_perf_domains = non_probe_perf_domains.len().min(u32::MAX as usize) as u32;
                if observed_perf_domains != emitted_domains
                    || observed_perf_domains != scheduled_domains
                {
                    return Err(format!(
                        "FRAME_END domain counts at byte {pos} claim emitted={emitted_domains} \
                         scheduled={scheduled_domains}, but observed {observed_perf_domains} \
                         unique non-probe DOMAIN_PERF domains"
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
                // `stream_fold.rs:448`-`:451`.
                open_hour = None;
                pending_perf_hour = None;
                non_probe_perf_domains.clear();
                last_closed_hour = Some(header.game_hour_raw);
                closed_hours += 1;
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        if header.domain_id == D38 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d38_records += 1;
            // One d38 DATA record per game hour, checked over EVERY d38 record and
            // BEFORE any status-based exclusion — gating it on the record being
            // usable would let a success/failure pair through the rule entirely.
            //
            // **This is the one place this harness is deliberately STRICTER than the
            // production stream boundary, and it is a property of the measurement,
            // not of the grammar** (round-6 review finding, recorded rather than
            // removed). `BoundaryTracker` accepts a second DOMAIN_DATA record for the
            // hour already open (`stream_fold.rs:369`), and `ingest_record` would
            // fold both — the first establishing the baseline, the second applied
            // sparsely on top. Nothing in production refuses that stream.
            //
            // The harness refuses it because its measurement model is **one hour =
            // one state = one wire record**: both arms are indexed by game hour, the
            // shipping arm concatenates the hour's record and the wire arm encodes
            // the hour's state, and a second record for one hour has no
            // representation on the wire side. Folding both into one hour would
            // publish a size comparison in which the two arms carry different
            // numbers of records for the same clock time, which is a silently wrong
            // number — the thing this validation exists to prevent. Refusing is the
            // honest option, and it costs nothing real: the collector emits exactly
            // one d38 record per frame (`attach_log.rs:29175`) and all 146 hours of
            // the capture carry exactly one.
            //
            // The alternative — folding both and measuring the hour once — is a
            // defensible design, but it would have to be built and justified rather
            // than assumed, and no capture exercises it.
            if let Some(previous) = last_d38_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d38 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d38_hour = Some(header.game_hour_raw);

            // `world_fold.rs:15383`-`:15432`: the six header-structural checks run on
            // every record, before either exclusion, and are hard errors.
            let payload_header = d38::peek_header(payload)
                .map_err(|e| format!("d38 payload at byte {pos}: {e}"))?;

            // VALIDITY BEFORE CLASSIFICATION. Everything below that reads the status
            // word — the failure predicate, the DELTA_DATA read — classifies the v1
            // vocabulary; neither is a validity predicate.
            // `compact_contract/src/lib.rs:121`-`:123` says so in as many words:
            // "Callers handling untrusted status words must first reject a non-zero
            // `compact_status_unknown_flags_v1` result; otherwise an unassigned bit
            // has no defined classification." Bits 14..=31 are unassigned
            // (`status_flags_base.rs:21`-`:24`), and an unassigned bit "is not
            // evidence of a future informational status".
            //
            // Without this, a record with `status_flags = 1 << 14` reads as "not a
            // failure" — the failure helper masks with the *known* failure set and
            // returns zero for a bit it does not know — and the hour is measured as
            // healthy. No capture carries an unassigned bit, so no measured number
            // moves.
            let unknown_status = compact_status_unknown_flags_v1(payload_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d38 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            // Production's SECOND authority, which runs on every DOMAIN_DATA record
            // whatever its status and whose complaint refuses the stream at
            // certification. This must come BEFORE the exclusions, or a record the
            // recorder would refuse gets quietly skipped instead. (Round-4 review
            // finding.)
            check_authority_payload(payload, &payload_header, pos)?;

            // `world_fold::handle_compact_failure` (`world_fold.rs:2863`, called at
            // `:15433`) marks the domain-hour unknown and RETAINS the prior
            // accumulator on a failure status, folding nothing from this record. The
            // predicate is `compact_contract`'s own, not a copied bit mask: it already
            // applies d38's lane-local mask (none) and expected partial tolerance
            // (none), so it cannot drift from the recorder.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D38,
                payload_header.status_flags,
            ) != 0
            {
                failure_records += 1;
                pos = record_end;
                continue;
            }

            let is_delta = payload_header.status_flags & COMPACT_STATUS_DELTA_DATA != 0;
            // Everything else — the exact-length check, the clear-or-sparse decision,
            // the row loop, the delete grammar — is the domain module's, in the fold's
            // own order.
            let outcome = acc
                .fold_payload(collection, payload, is_delta, 0)
                .map_err(|e| format!("d38 payload at byte {pos}: {e}"))?;
            match outcome {
                d38::FoldOutcome::DeltaBeforeBaseline => {
                    delta_before_baseline += 1;
                    pos = record_end;
                    continue;
                }
                d38::FoldOutcome::FullReplace => full_replaces += 1,
                d38::FoldOutcome::Sparse => {}
            }

            if let Some(previous) = last_measured_hour {
                if header.game_hour_raw != previous + 1 {
                    non_contiguous += 1;
                }
            }
            last_measured_hour = Some(header.game_hour_raw);

            hours.push(D38Hour {
                game_hour: header.game_hour_raw,
                state: acc.rows.clone(),
                ship_span: pos..record_end,
                shipped_rows: payload_header.row_count,
                shipped_deletes: payload_header.deleted_rows,
            });
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
        return Err("stream carries no usable d38 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d38 DATA
    // record, of any status. Production validates the payload of EVERY domain's
    // DATA record (`stream_fold.rs:593`); this harness cannot (module docs), so
    // a d38 record relabelled to another domain id — or lost outright — would
    // otherwise be silently ignored and the run would report a plausible
    // measurement over fewer hours. Per-hour uniqueness is already enforced
    // above, so equality of the two counts is exactly-one per closed hour. A
    // per-capture assertion, not a grammar rule: a capture that does not
    // schedule d38 every hour fails here loudly and a human adjusts, instead
    // of a number quietly shrinking.
    if d38_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d38_records} d38 DATA \
             record(s); every closed hour of a d38-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    Ok(StreamScan {
        hours,
        failure_records,
        delta_before_baseline,
        non_contiguous,
        full_replaces,
    })
}

fn main() {
    const USAGE: &str = "usage: bench_real_d38 <collector_stream.bin> <out dir> \
                         [legacy|identity-v1]";
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    // The identity protocol is stated, never sniffed — see `scan_stream`. `legacy`
    // is the default because every capture in `runs/` is one (`deepwar_t1`'s first
    // record is an ordinary record at sequence 1), and because a default is not a
    // sniff: it is a claim the validator then holds the stream to, and an identity
    // stream passed under it is refused at its CID1 record rather than measured.
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => {
            eprintln!("unknown identity mode {other:?}; {USAGE}");
            std::process::exit(2);
        }
    };
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d38::schema();
    let collection = &schema.collections[0];

    // ---- Pass 1: validate the WHOLE stream and fold its d38 DATA records. ----
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result.
    let scan = match scan_stream(&stream, collection, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: encode both arms. Nothing here reads stream content again, so
    // every assertion below is about the codec, not about the input. -----------

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut prior: Option<&KeyedState> = None;
    let mut reader: Option<KeyedState> = None;
    let mut hours_seen = 0usize;
    // Block boundaries are measured on the game clock, from the first measured hour
    // — see the `block_index` computation below for why not on the record count.
    let first_measured_hour = scan.hours[0].game_hour;
    let mut last_block_index: Option<i64> = None;
    let mut baselines = 0usize;
    // Baselines the §1.5 gap rule forced outside a scheduled block start.
    let mut gap_forced_baselines = 0usize;
    let mut previous_measured_hour: Option<i32> = None;
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    let mut empty_hours = 0usize;
    // Delta-path churn: what the delta arm actually carried at this block cadence.
    let mut delta_changed_rows = 0usize;
    let mut delta_tombstones = 0usize;
    // Cadence-independent churn: what the DATA did, over every measured-hour
    // transition, taken BEFORE the block decision so a change landing on a block
    // boundary still counts.
    let mut real_changed_rows = 0usize;
    let mut real_tombstones = 0usize;
    let mut real_changed_hours = 0usize;
    let mut buckets_total = 0usize;
    let mut shipped_rows_total = 0usize;
    let mut shipped_deletes_total = 0usize;
    let mut shipped_empty_frames = 0usize;

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());
        shipped_rows_total += hour.shipped_rows as usize;
        shipped_deletes_total += hour.shipped_deletes as usize;
        if hour.shipped_rows == 0 {
            shipped_empty_frames += 1;
        }

        // The hour's state, accumulated during the scan through the domain module's
        // own fold: d38 is sparse-after-first, so the hour's rows are NOT the hour's
        // state.
        let state: &KeyedState = &hour.state;
        buckets_total += state.len();

        // Cadence-independent churn, taken before the baseline/delta decision.
        if let Some(prev) = prior {
            let d = diff(collection, prev, state);
            if !d.is_empty() {
                real_changed_hours += 1;
                real_changed_rows += d.changed.len();
                real_tombstones += d.tombstones.len();
            }
        }

        // New arm: BASELINE at each block start, stored-minimal DELTA between.
        //
        // The block index is taken from the GAME HOUR, not from the count of usable
        // records (round-7 review finding). Keying it on the record count would let a
        // single excluded hour — a failure status, which d38's `PARTIAL_DATA` path
        // makes producer-reachable — slide every later baseline by one, so the first
        // "48-hour block" would span 49 game-clock hours and a change landing on the
        // real boundary would be encoded as a delta instead of the baseline the
        // cadence promises. Blocks are a statement about the clock, so they are
        // measured against the clock.
        let hour_start = wire.len();
        let block_index = (hour.game_hour as i64 - first_measured_hour as i64)
            .div_euclid(BLOCK_HOURS as i64);
        let block_start = Some(block_index) != last_block_index;
        last_block_index = Some(block_index);
        // §1.5: a gap in measured hours forces the baseline even when the
        // game-clock block has not turned over (see `opens_baseline`).
        let baseline_here = opens_baseline(block_start, previous_measured_hour, hour.game_hour);
        if baseline_here && !block_start {
            gap_forced_baselines += 1;
        }
        let mut emitted = true;
        if baseline_here || prior.is_none() {
            encode_baseline(&schema, std::slice::from_ref(state), &mut wire)
                .expect("baseline encodes");
            baselines += 1;
            baseline_bytes += wire.len() - hour_start;
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
                delta_bytes += wire.len() - hour_start;
            }
        }
        wire_offsets.push(wire.len());

        // Reader replay: decode the bytes just written against the reader's OWN
        // reconstructed prior, exactly as a conforming reader would, and require the
        // result to be stored-equal to this hour's state. This is what makes the size
        // claim a claim about decodable bytes. An hour that emitted nothing is still
        // checked: the reader's retained state must already equal the fold's, which is
        // precisely what "nothing changed" asserts.
        if emitted {
            let hour_bytes = &wire[hour_start..];
            if baseline_here || reader.is_none() {
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
        previous_measured_hour = Some(hour.game_hour);
        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d38.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d38.offsets", &ship_offsets),
        ("wire_new.offsets", &wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    let delta_hours = hours_seen - baselines - empty_hours;
    println!(
        "hours                {hours_seen} ({} non-contiguous steps)",
        scan.non_contiguous
    );
    println!(
        "failure-status hours {} (excluded from both arms)",
        scan.failure_records
    );
    println!(
        "delta-before-baseline {} (excluded from both arms)",
        scan.delta_before_baseline
    );
    println!(
        "full replaces        {} (the fold clears only on the first record)",
        scan.full_replaces
    );
    println!(
        "buckets avg          {:.1}   final {}",
        buckets_total as f64 / hours_seen as f64,
        scan.hours.last().map_or(0, |h| h.state.len())
    );
    println!(
        "baselines            {baselines} ({} every {BLOCK_HOURS} h on the game clock, \
         {gap_forced_baselines} forced by a gap in measured hours — WIRE_FORMAT §1.5), \
         {baseline_bytes} raw bytes",
        baselines - gap_forced_baselines
    );
    println!("delta hours          {delta_hours} ({delta_bytes} raw bytes)   empty hours {empty_hours}");
    // Two churn figures, because one number cannot answer both questions. The first
    // is a property of the DATA and is cadence-independent — it counts every
    // measured-hour transition, baseline hours included. The second is what the
    // delta path actually carried at this block size; the gap between them is the
    // change this cadence swallowed into a baseline.
    println!(
        "real churn           {real_changed_rows} changed rows and {real_tombstones} tombstone(s) \
         over {real_changed_hours} changed hour(s) of {}; {:.2} rows/hour (cadence-independent)",
        hours_seen.saturating_sub(1),
        real_changed_rows as f64 / hours_seen.saturating_sub(1).max(1) as f64
    );
    println!(
        "delta-path churn     {delta_changed_rows} changed rows, {delta_tombstones} tombstone(s) \
         over {delta_hours} delta hour(s); {:.2} rows per delta hour; {} changed row(s) and {} \
         tombstone(s) fell on a block boundary and were carried by a baseline",
        delta_changed_rows as f64 / delta_hours.max(1) as f64,
        // Both, not just the rows: a key deleted on a block boundary with no upsert
        // that hour is a real change the delta path never carried, and reporting
        // only the row difference printed "0 changes fell on a boundary" for it
        // (round-7 review finding). Tombstones are an ordinary producer output —
        // this capture has 82 of them.
        real_changed_rows.saturating_sub(delta_changed_rows),
        real_tombstones.saturating_sub(delta_tombstones)
    );
    println!(
        "shipping churn       {:.2} serialized rows/hour, {:.3} deletes/hour, \
         {shipped_empty_frames} zero-row frames",
        shipped_rows_total as f64 / hours_seen.max(1) as f64,
        shipped_deletes_total as f64 / hours_seen.max(1) as f64
    );
    println!("ship_d38.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.1}%)",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64
    );
    println!(
        "  of which baseline  {baseline_bytes} bytes ({:.1}%)   delta {delta_bytes} bytes ({:.1}%)",
        100.0 * baseline_bytes as f64 / wire.len().max(1) as f64,
        100.0 * delta_bytes as f64 / wire.len().max(1) as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{COMPACT_DOMAIN_FRAME, COMPACT_FRAME_END_PAYLOAD_LEN};

    fn test_collection() -> wire_format::Collection {
        d38::schema().collections[0].clone()
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

    /// One 48-byte d38 row, packed as the collector packs one
    /// (`attach_log.rs:71549`-`:71559`).
    #[allow(clippy::too_many_arguments)]
    fn row(
        region: u32,
        owner: u32,
        mask: u32,
        variant: u32,
        flags: u32,
        token: u32,
        type_token: u32,
        iface: u32,
        wings: u32,
        amount_raw: i64,
    ) -> Vec<u8> {
        let mut r = Vec::new();
        for w in [
            region, owner, mask, variant, flags, token, type_token, iface, wings, 0,
        ] {
            r.extend_from_slice(&w.to_le_bytes());
        }
        r.extend_from_slice(&amount_raw.to_le_bytes());
        assert_eq!(r.len(), d38::ROW_LEN);
        r
    }

    fn live(region: u32, owner: u32, mask: u32, variant: u32, amount_raw: i64) -> Vec<u8> {
        row(region, owner, mask, variant, 0, 20_190, 17, 5, 3, amount_raw)
    }

    fn dead(region: u32, owner: u32, mask: u32, variant: u32) -> Vec<u8> {
        row(region, owner, mask, variant, d38::ROW_FLAG_DELETED, 0, 0, 0, 0, 0)
    }

    /// A d38 DATA payload with the collector's own header arithmetic
    /// (`attach_log.rs:71531`-`:71542`).
    fn d38_payload(status_flags: u32, rows: &[Vec<u8>], observed_equipment_rows: u32) -> Vec<u8> {
        let deleted = rows
            .iter()
            .filter(|r| {
                u32::from_le_bytes(r[16..20].try_into().unwrap()) & d38::ROW_FLAG_DELETED != 0
            })
            .count() as u32;
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // changed_rows
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&643u32.to_le_bytes()); // observed_air_wing_rows
        p.extend_from_slice(&643u32.to_le_bytes()); // observed_loadout_rows
        p.extend_from_slice(&observed_equipment_rows.to_le_bytes());
        p.extend_from_slice(&204u32.to_le_bytes()); // observed_variant_rows
        p.extend_from_slice(&0u32.to_le_bytes()); // invalid_air_wing_rows
        p.extend_from_slice(&0u32.to_le_bytes()); // invalid_variant_rows
        p.extend_from_slice(&deleted.to_le_bytes());
        p.extend_from_slice(&d38::FIXED_SCALE_RAW.to_le_bytes());
        for r in rows {
            p.extend_from_slice(r);
        }
        p
    }

    /// Two closed hours: a baseline then a sparse patch, the shape the real stream
    /// has (d38's collector never sets DELTA_DATA).
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D38,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d38_payload(0, &[live(37, 12, 512, 7249, 100_000)], 1999),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D38,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d38_payload(0, &[live(37, 12, 512, 7249, 200_000)], 1999),
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

    #[test]
    fn a_well_formed_stream_yields_its_d38_hours() {
        let scan = scan_stream(&good_stream(), &test_collection(), StreamIdentityMode::Legacy).expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].game_hour, 100);
        assert_eq!(scan.hours[1].game_hour, 101);
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.delta_before_baseline, 0);
        assert_eq!(scan.non_contiguous, 0);
        assert_eq!(scan.full_replaces, 1, "only the first record clears");
        assert_eq!(scan.hours[0].state.len(), 1);
        assert_eq!(scan.hours[1].state.len(), 1);
        assert_ne!(scan.hours[0].state, scan.hours[1].state);
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helper classifies the
        // v1 vocabulary and is not a validity predicate, so an unassigned bit must be
        // rejected BEFORE classification — otherwise it silently reads as "not a
        // failure", gets a baseline/delta meaning, and the hour is measured as healthy.
        for bit in 15..32u32 {
            let status = 1u32 << bit;
            for extra in [0u32, 1, COMPACT_STATUS_DELTA_DATA] {
                let mut first = Vec::new();
                first.extend(record(
                    1,
                    100,
                    D38,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d38_payload(status | extra, &[live(37, 12, 512, 7249, 100_000)], 1999),
                ));
                first.extend(frame_end(2, 100));
                assert!(
                    scan_stream(&first, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                    "status bit {bit} (+{extra:#x}) as the first record must be refused"
                );
                let mut after = Vec::new();
                after.extend(record(
                    1,
                    100,
                    D38,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d38_payload(0, &[live(37, 12, 512, 7249, 100_000)], 1999),
                ));
                after.extend(frame_end(2, 100));
                after.extend(record(
                    3,
                    101,
                    D38,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d38_payload(status | extra, &[live(37, 12, 512, 7249, 200_000)], 1999),
                ));
                after.extend(frame_end(4, 101));
                assert!(
                    scan_stream(&after, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                    "status bit {bit} (+{extra:#x}) after a baseline must be refused"
                );
            }
        }
        // And the whole known v1 mask, minus the failure bits, is still accepted: the
        // check must reject unassigned bits and nothing else.
        for informational in [1u32 << 4, 1 << 5, COMPACT_STATUS_DELTA_DATA, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D38,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d38_payload(0, &[live(37, 12, 512, 7249, 100_000)], 1999),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D38,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d38_payload(informational, &[live(37, 12, 512, 7249, 200_000)], 1999),
            ));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 2, "bit {informational:#x}");
            assert_eq!(scan.failure_records, 0, "bit {informational:#x}");
            assert_eq!(scan.delta_before_baseline, 0, "bit {informational:#x}");
        }
    }

    #[test]
    fn the_scan_accumulates_rather_than_snapshotting() {
        // The property that separates d38 from a full-replace domain: an hour's state
        // is the fold's retained map, not the hour's rows. A later record that
        // mentions one bucket must leave the other one standing — and d38's records
        // carry NO delta flag, so a harness keying off DELTA_DATA would read every one
        // of them as a snapshot and drop ~200 buckets an hour.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D38,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d38_payload(
                0,
                &[live(37, 12, 512, 7249, 100_000), live(9, 3, 1, 836, 200_000)],
                1999,
            ),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D38,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d38_payload(0, &[live(9, 3, 1, 836, 300_000)], 1999),
        ));
        s.extend(frame_end(4, 101));
        // An empty frame: 47 of the capture's 146 records are this shape.
        s.extend(record(
            5,
            102,
            D38,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d38_payload(0, &[], 1999),
        ));
        s.extend(frame_end(6, 102));
        // A delete row removes its key and only its key.
        s.extend(record(
            7,
            103,
            D38,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d38_payload(0, &[dead(37, 12, 512, 7249)], 1999),
        ));
        s.extend(frame_end(8, 103));

        let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("accepted");
        assert_eq!(scan.hours.len(), 4);
        assert_eq!(scan.full_replaces, 1);
        assert_eq!(scan.hours[0].state.len(), 2);
        assert_eq!(scan.hours[1].state.len(), 2, "a later record does not clear");
        assert_eq!(
            scan.hours[2].state, scan.hours[1].state,
            "an empty record is a no-op"
        );
        assert_eq!(scan.hours[3].state.len(), 1, "the delete removed one bucket");
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        // A cut immediately after a d38 DATA record leaves that hour open and ends the
        // stream on a record that is not a FRAME_END.
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&full[..cut], &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary IS a well-formed shorter stream, and
        // `eof_check` accepts it too. What matters is that it reports the hours it
        // actually closed, not the hours the untruncated file had.
        for (records, expected_hours) in [(2usize, 1usize), (4, 2)] {
            let cut = boundary(&full, records);
            let scan =
                scan_stream(&full[..cut], &test_collection(), StreamIdentityMode::Legacy).expect("closed stream is well-formed");
            assert_eq!(scan.hours.len(), expected_hours, "cut after {records}");
        }
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
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D38,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d38_payload(0, &[live(37, 12, 512, 7249, 100_000)], 1999),
        ));
        s.extend(frame_end(2, 999)); // closes an hour that is not open
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());

        let mut s = Vec::new();
        s.extend(frame_end(1, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            101,
            D38,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d38_payload(0, &[live(37, 12, 512, 7249, 100_000)], 1999),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D38,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d38_payload(0, &[live(37, 12, 512, 7249, 200_000)], 1999),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_second_d38_record_in_one_hour_is_rejected() {
        // The harness's one deliberate strictness beyond the production stream
        // boundary, which accepts a second DOMAIN_DATA record for the open hour
        // (`stream_fold.rs:369`) and folds both. See `scan_stream` for why the
        // measurement model cannot represent that: one hour is one state is one wire
        // record, and both arms are indexed by game hour. Round-6 review finding,
        // recorded as a divergence rather than removed.
        //
        // The rule is checked over EVERY d38 DATA record and before status-based
        // exclusion, so all four success/failure orderings are refused.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D38,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d38_payload(first, &[live(37, 12, 512, 7249, 100_000)], 1999),
            ));
            s.extend(record(
                2,
                100,
                D38,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d38_payload(second, &[live(37, 12, 512, 7249, 200_000)], 1999),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "two d38 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time: the fold
        // retains the accumulator and folds nothing, so the hour is not d38 data and
        // must not enter either arm. d38 has NO expected-partial tolerance, so
        // `PARTIAL_DATA` (bit 9) is in the list — and it is the bit the collector
        // actually raises when a variant read fails (`attach_log.rs:55657`).
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D38,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d38_payload(1 << bit, &[live(37, 12, 512, 7249, 100_000)], 1999),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D38,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d38_payload(0, &[live(9, 3, 1, 836, 200_000)], 1999),
            ));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("the stream itself is well-formed");
            assert_eq!(scan.failure_records, 1, "bit {bit}");
            assert_eq!(scan.hours.len(), 1, "bit {bit}");
            assert_eq!(scan.hours[0].game_hour, 101, "bit {bit}");
            // And the excluded record folded NOTHING: hour 101's state carries only
            // the bucket hour 101 serialized.
            assert_eq!(scan.hours[0].state.len(), 1, "bit {bit}");
            assert!(
                scan.hours[0]
                    .state
                    .contains_key(&wire_format::body::key(&[9, 3, 1, 836])),
                "bit {bit}: the failure record's row must not be folded"
            );
        }
        // And a non-failure bit (DELTA_DATA) is NOT treated as failure — though as the
        // first record it becomes delta-before-baseline instead.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D38,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d38_payload(
                COMPACT_STATUS_DELTA_DATA,
                &[live(37, 12, 512, 7249, 100_000)],
                1999,
            ),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D38,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d38_payload(0, &[live(9, 3, 1, 836, 200_000)], 1999),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.delta_before_baseline, 1);
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.hours[0].game_hour, 101);
    }

    #[test]
    fn a_failure_record_still_has_its_header_checked() {
        // `world_fold.rs:15383`-`:15432` runs BEFORE `handle_compact_failure` at
        // `:15433`, so a failure-status record with a malformed header is a hard
        // error, not a quietly skipped hour. One case per header invariant.
        let bad_headers: [(&str, fn(&mut Vec<u8>)); 5] = [
            ("changed_rows > row_count", |p| {
                p[12..16].copy_from_slice(&2u32.to_le_bytes())
            }),
            ("observed_loadout > observed_air_wing", |p| {
                p[24..28].copy_from_slice(&644u32.to_le_bytes())
            }),
            ("invalid_variant > observed_equipment", |p| {
                p[40..44].copy_from_slice(&2000u32.to_le_bytes())
            }),
            ("deleted_rows > row_count", |p| {
                p[44..48].copy_from_slice(&2u32.to_le_bytes())
            }),
            ("scale_raw != FIXED_SCALE", |p| {
                p[48..52].copy_from_slice(&32_768u32.to_le_bytes())
            }),
        ];
        for (label, patch) in bad_headers {
            let mut payload = d38_payload(1, &[live(37, 12, 512, 7249, 100_000)], 1999);
            patch(&mut payload);
            let mut s = Vec::new();
            s.extend(record(1, 100, D38, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "{label} must fail even under a failure status"
            );
        }
        // `row_count > observed_equipment_rows + deleted_rows` too.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D38,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d38_payload(1, &[live(37, 12, 512, 7249, 100_000)], 0),
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());

        // The exact-length check runs AFTER the exclusion in the FOLD (`:15447`), so
        // the fold itself returns Ok on a short failure-status record — but
        // production's second authority checks the length on every DOMAIN_DATA
        // record whatever its status, and its complaint refuses the stream at
        // certification. See `an_excluded_record_is_still_structurally_validated`,
        // which is where that case now lives (round-4 review finding).
    }

    #[test]
    fn an_excluded_record_is_still_structurally_validated() {
        // Round-4 review finding. A record this harness excludes — failure status or
        // delta-before-baseline — is still validated by production's second
        // authority (`domain_data_authority.rs:15247`, run unconditionally from
        // `stream_fold.rs:1059`), so a violation there refuses the stream rather
        // than quietly skipping an hour.
        //
        // The reported reproducer is the first case: a failure-status record whose
        // payload is one byte short of its own `row_count`.
        let good_second_hour = |seq| {
            record(
                seq,
                101,
                D38,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d38_payload(0, &[live(9, 3, 1, 836, 200_000)], 1999),
            )
        };
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE

        // Swept over both exclusion paths and both authority checks, rather than the
        // one reported input.
        let mut cases: Vec<(String, Vec<u8>)> = Vec::new();
        for (label, status) in [
            ("failure status", FAILED),
            ("delta before baseline", COMPACT_STATUS_DELTA_DATA),
        ] {
            // Short by one byte.
            let mut short = d38_payload(status, &[live(37, 12, 512, 7249, 100_000)], 1999);
            short.pop();
            cases.push((format!("{label}: payload one byte short"), short));
            // Long by one byte.
            let mut long = d38_payload(status, &[live(37, 12, 512, 7249, 100_000)], 1999);
            long.push(0);
            cases.push((format!("{label}: payload one byte long"), long));
            // A row_count whose byte length overflows `usize`.
            let mut huge = d38_payload(status, &[live(37, 12, 512, 7249, 100_000)], 1999);
            huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
            huge[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
            huge[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
            cases.push((format!("{label}: row_count overflows"), huge));
        }
        // `row_count > observed_equipment_rows` on a NON-delta record: the fold
        // accepts it (`row_count <= observed_equipment_rows + deleted_rows` holds
        // when the deletes cover it) but the second authority refuses it
        // (`domain_data_authority.rs:15281`). This one is not a length case at all,
        // and it is why the check is a set rather than the single reported input.
        let mut over = d38_payload(
            FAILED,
            &[
                live(37, 12, 512, 7249, 100_000),
                row(9, 3, 1, 836, d38::ROW_FLAG_DELETED, 0, 0, 0, 0, 0),
            ],
            1,
        );
        over[44..48].copy_from_slice(&1u32.to_le_bytes()); // deleted_rows = 1
        cases.push((
            "row_count > observed_equipment_rows on a non-delta record".to_string(),
            over,
        ));

        for (label, payload) in cases {
            let mut s = Vec::new();
            s.extend(record(1, 100, D38, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
            s.extend(frame_end(2, 100));
            s.extend(good_second_hour(3));
            s.extend(frame_end(4, 101));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "{label}: an excluded record must not bypass structural validation"
            );
        }

        // The boundary of the second authority, stated as a test: it does NOT walk
        // rows (`domain_data_authority.rs:15247`-`:15330` reads the header and
        // stops), and the fold returns before its own row loop on an excluded
        // record. So an excluded record carrying an unknown row flag or a non-zero
        // row reserved word is a stream production CERTIFIES, and refusing it here
        // would be stricter than production — a defect in the other direction.
        for bad_row in [
            row(37, 12, 512, 7249, 1 << 5, 0, 0, 0, 0, 0), // unknown flag bit
            {
                let mut r = live(37, 12, 512, 7249, 100_000);
                r[36..40].copy_from_slice(&1u32.to_le_bytes()); // reserved word
                r
            },
        ] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D38,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d38_payload(FAILED, &[bad_row.clone()], 1999),
            ));
            s.extend(frame_end(2, 100));
            s.extend(good_second_hour(3));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).unwrap_or_else(|e| {
                panic!("an excluded record's rows are not production's business: {e}")
            });
            assert_eq!(scan.failure_records, 1);
            assert_eq!(scan.hours.len(), 1);
            assert_eq!(scan.hours[0].game_hour, 101);
        }
        // ...but the same row inside a FOLDED record is still refused, because the
        // fold does reach its row loop there.
        for bad_row in [
            row(37, 12, 512, 7249, 1 << 5, 0, 0, 0, 0, 0),
            {
                let mut r = live(37, 12, 512, 7249, 100_000);
                r[36..40].copy_from_slice(&1u32.to_le_bytes());
                r
            },
        ] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D38,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d38_payload(0, &[bad_row], 1999),
            ));
            s.extend(frame_end(2, 100));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }
    }

    /// One DOMAIN_PERF record with the minimal valid 40-byte payload
    /// (`COMPACT_DOMAIN_PERF_BASE_LEN`).
    fn perf(sequence: u64, game_hour: i32, domain: u16) -> Vec<u8> {
        record(
            sequence,
            game_hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &vec![0u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN],
        )
    }

    /// One FRAME_END with explicit seal fields, so a test can perturb exactly one.
    fn frame_end_sealed(
        sequence: u64,
        game_hour: i32,
        emitted: u32,
        scheduled: u32,
        status: u32,
        reserved: u32,
    ) -> Vec<u8> {
        let mut payload = Vec::with_capacity(COMPACT_FRAME_END_PAYLOAD_LEN);
        payload.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        payload.extend_from_slice(&0u64.to_le_bytes()); // frame_time_us
        payload.extend_from_slice(&emitted.to_le_bytes());
        payload.extend_from_slice(&scheduled.to_le_bytes());
        payload.extend_from_slice(&status.to_le_bytes());
        payload.extend_from_slice(&reserved.to_le_bytes());
        assert_eq!(payload.len(), COMPACT_FRAME_END_PAYLOAD_LEN);
        record(
            sequence,
            game_hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &payload,
        )
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        // Round-1 review finding, swept over the whole seal predicate rather than the
        // one reported input. The reviewer's own case is the first row: `good_stream()`
        // with the first FRAME_END's reserved word set to 1 (overall byte 192) was
        // accepted and produced an unchanged, plausible measurement, while production's
        // `validate_frame_end_seal_fields` (`stream_fold.rs:46`) refuses it.
        //
        // A well-formed seal, for contrast. These streams carry no DOMAIN_PERF
        // records, so the seal's domain counts must be 0 to match the observed
        // non-probe PERF domains — the cross-check `perf_hour_binding_is_enforced`
        // covers with real PERF records. (The capture's own shape is
        // `emitted == scheduled == 23` against 23 observed PERF domains.)
        let good = |seq, hour| frame_end_sealed(seq, hour, 0, 0, 0, 0);
        let mut ok = Vec::new();
        ok.extend(record(
            1,
            100,
            D38,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d38_payload(0, &[live(37, 12, 512, 7249, 100_000)], 1999),
        ));
        ok.extend(good(2, 100));
        assert_eq!(
            scan_stream(&ok, &test_collection(), StreamIdentityMode::Legacy)
                .expect("a sealed stream is accepted")
                .hours
                .len(),
            1
        );

        // Every way the seal can fail, each on its own.
        let mut bad_seals: Vec<(String, Vec<u8>)> = vec![
            (
                "reserved nonzero (the reported input)".to_string(),
                frame_end_sealed(2, 100, 0, 0, 0, 1),
            ),
            (
                "emitted != scheduled".to_string(),
                frame_end_sealed(2, 100, 0, 1, 0, 0),
            ),
        ];
        // Every unassigned status bit, 14..=31.
        for bit in 15..32u32 {
            bad_seals.push((
                format!("unknown seal status bit {bit}"),
                frame_end_sealed(2, 100, 0, 0, 1 << bit, 0),
            ));
        }
        // Every fatal status bit in COMPACT_STATUS_FAILURE_MASK.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            bad_seals.push((
                format!("fatal seal status bit {bit}"),
                frame_end_sealed(2, 100, 0, 0, 1 << bit, 0),
            ));
        }
        for (label, sealed) in bad_seals {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D38,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d38_payload(0, &[live(37, 12, 512, 7249, 100_000)], 1999),
            ));
            s.extend(sealed);
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "{label} must refuse the stream, not yield a measurement"
            );
        }

        // A non-fatal informational seal status is still accepted — the check must
        // reject the unknown and the fatal, and nothing else.
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D38,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d38_payload(0, &[live(37, 12, 512, 7249, 100_000)], 1999),
            ));
            s.extend(frame_end_sealed(2, 100, 0, 0, informational, 0));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_ok(),
                "informational seal status {informational:#x} must be accepted"
            );
        }

        // And a seal violation on a LATER frame refuses the whole stream rather than
        // silently truncating the measurement to the hours before it.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D38,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d38_payload(0, &[live(37, 12, 512, 7249, 100_000)], 1999),
        ));
        s.extend(good(2, 100));
        s.extend(record(
            3,
            101,
            D38,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d38_payload(0, &[live(37, 12, 512, 7249, 200_000)], 1999),
        ));
        s.extend(frame_end_sealed(4, 101, 0, 0, 0, 1));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        // Round-2 review finding: DOMAIN_PERF records carry no d38 data, but they
        // bind the hour, and production refuses a stream that violates that binding
        // (`stream_fold.rs:328`-`:365`). Swept over the whole predicate rather than
        // the one reported input, which is the second row below.
        //
        // The shape of a healthy hour: PERF for each scheduled domain, the d38 DATA
        // record, then a FRAME_END whose seal counts equal the observed non-probe
        // PERF domains. The capture's own shape, at 2 domains instead of 23.
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D38,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d38_payload(0, &[live(37, 12, 512, 7249, 100_000)], 1999),
            )
        };
        let mut ok = Vec::new();
        ok.extend(perf(1, 100, 20));
        ok.extend(perf(2, 100, D38));
        ok.extend(perf(3, 100, PROBE_DOMAIN_ID)); // the probe is not counted
        ok.extend(data(4, 100));
        ok.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
        assert_eq!(
            scan_stream(&ok, &test_collection(), StreamIdentityMode::Legacy)
                .expect("a PERF-bound hour is accepted")
                .hours
                .len(),
            1
        );
        // The probe domain may repeat without counting as a duplicate.
        let mut probe_twice = Vec::new();
        probe_twice.extend(perf(1, 100, PROBE_DOMAIN_ID));
        probe_twice.extend(perf(2, 100, PROBE_DOMAIN_ID));
        probe_twice.extend(data(3, 100));
        probe_twice.extend(frame_end_sealed(4, 100, 0, 0, 0, 0));
        assert!(scan_stream(&probe_twice, &test_collection(), StreamIdentityMode::Legacy).is_ok());

        // Duplicate non-probe PERF domain inside one hour.
        let mut dup = Vec::new();
        dup.extend(perf(1, 100, 20));
        dup.extend(perf(2, 100, 20));
        dup.extend(data(3, 100));
        dup.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        assert!(scan_stream(&dup, &test_collection(), StreamIdentityMode::Legacy).is_err(), "duplicate PERF domain");

        // The reported input: a PERF for hour 999 while hour 100 is open.
        let mut interleaved = Vec::new();
        interleaved.extend(data(1, 100));
        interleaved.extend(perf(2, 999, 20));
        interleaved.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(
            scan_stream(&interleaved, &test_collection(), StreamIdentityMode::Legacy).is_err(),
            "PERF for a hour other than the open one"
        );

        // A PERF for an hour already closed.
        let mut stale = Vec::new();
        stale.extend(data(1, 100));
        stale.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        stale.extend(perf(3, 100, 20));
        stale.extend(data(4, 101));
        stale.extend(frame_end_sealed(5, 101, 1, 1, 0, 0));
        assert!(
            scan_stream(&stale, &test_collection(), StreamIdentityMode::Legacy).is_err(),
            "non-monotonic PERF"
        );

        // Two different pending PERF hours before any DATA record opens one.
        let mut conflict = Vec::new();
        conflict.extend(perf(1, 100, 20));
        conflict.extend(perf(2, 101, 21));
        conflict.extend(data(3, 100));
        conflict.extend(frame_end_sealed(4, 100, 2, 2, 0, 0));
        assert!(
            scan_stream(&conflict, &test_collection(), StreamIdentityMode::Legacy).is_err(),
            "PERF hour conflict"
        );

        // A DATA record opening an hour OTHER than the one the pending PERF records
        // bound (`stream_fold.rs:378`-`:386`). Round-3 review finding: the PERF arm
        // alone does not cover this, because one pending PERF hour is legal until a
        // DATA record disagrees with it — the reproducer below passed before the
        // DATA arm learned to consult `pending_perf_hour`, seal count and all.
        let mut data_conflict = Vec::new();
        data_conflict.extend(perf(1, 100, 20));
        data_conflict.extend(data(2, 101));
        data_conflict.extend(frame_end_sealed(3, 101, 1, 1, 0, 0));
        assert!(
            scan_stream(&data_conflict, &test_collection(), StreamIdentityMode::Legacy).is_err(),
            "DATA hour conflict against the pending PERF hour"
        );
        // Both directions of the disagreement, and with the DATA hour on either side
        // of the pending one, so the rule is not accidentally a monotonicity check.
        for (perf_hour, data_hour) in [(100i32, 101i32), (101, 100), (100, 200), (200, 100)] {
            let mut s = Vec::new();
            s.extend(perf(1, perf_hour, 20));
            s.extend(data(2, data_hour));
            s.extend(frame_end_sealed(3, data_hour, 1, 1, 0, 0));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "pending PERF hour {perf_hour} then DATA hour {data_hour}"
            );
        }
        // ...and the agreeing case is still accepted.
        let mut agreeing = Vec::new();
        agreeing.extend(perf(1, 100, 20));
        agreeing.extend(data(2, 100));
        agreeing.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert_eq!(
            scan_stream(&agreeing, &test_collection(), StreamIdentityMode::Legacy)
                .expect("a DATA record matching the pending PERF hour is accepted")
                .hours
                .len(),
            1
        );

        // And the seal's domain counts must equal the observed non-probe PERF
        // domains (`stream_fold.rs:411`-`:419`), in both directions.
        for (emitted, scheduled) in [(1u32, 1u32), (3, 3), (2, 3), (3, 2)] {
            let mut s = Vec::new();
            s.extend(perf(1, 100, 20));
            s.extend(perf(2, 100, D38));
            s.extend(data(3, 100));
            s.extend(frame_end_sealed(4, 100, emitted, scheduled, 0, 0));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "seal claiming emitted={emitted} scheduled={scheduled} against 2 observed \
                 PERF domains must be refused"
            );
        }

        // The PERF domain set resets at each FRAME_END: the same domain may appear
        // again in the next hour.
        let mut two_hours = Vec::new();
        two_hours.extend(perf(1, 100, 20));
        two_hours.extend(data(2, 100));
        two_hours.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        two_hours.extend(perf(4, 101, 20));
        two_hours.extend(data(5, 101));
        two_hours.extend(frame_end_sealed(6, 101, 1, 1, 0, 0));
        assert_eq!(
            scan_stream(&two_hours, &test_collection(), StreamIdentityMode::Legacy)
                .expect("the PERF domain set resets per hour")
                .hours
                .len(),
            2
        );
    }

    #[test]
    fn the_identity_protocol_is_stated_not_sniffed() {
        // Round-5 review finding. A sniffing harness reads a decapitated IdentityV1
        // stream — one whose mandatory CID1 record has been spliced off — as a
        // healthy legacy stream and measures it, because both begin with an ordinary
        // record at sequence 1. Production takes the mode from its caller
        // (`stream_fold.rs:220`) and the validator never sniffs
        // (`stream_validator.rs:328`), so the mode is a parameter here too and a
        // mismatch fails closed in BOTH directions.
        use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;

        // The reported input: a legacy-looking stream that is really an identity
        // stream with its CID1 record removed. Under the mode its capture actually
        // has, it is refused.
        let decapitated = good_stream();
        assert!(
            scan_stream(
                &decapitated,
                &test_collection(),
                StreamIdentityMode::IdentityV1
            )
            .is_err(),
            "an identity-mode stream whose CID1 record is missing must be refused, \
             not measured as legacy"
        );
        // And the same bytes under the legacy claim are a healthy legacy stream —
        // which is the point: the bytes cannot tell the two apart, only the caller can.
        assert_eq!(
            scan_stream(&decapitated, &test_collection(), StreamIdentityMode::Legacy)
                .expect("a genuine legacy stream")
                .hours
                .len(),
            2
        );

        // The other direction: an identity-bearing stream read under the legacy
        // claim trips on its CID1 record instead of being silently accepted.
        let mut identity_stream = Vec::new();
        identity_stream.extend(record(
            0,
            0,
            0,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            b"CID1-not-a-real-identity-payload",
        ));
        identity_stream.extend(&good_stream());
        assert!(
            scan_stream(&identity_stream, &test_collection(), StreamIdentityMode::Legacy).is_err(),
            "a CID1 record under the legacy claim must be refused"
        );
    }

    #[test]
    fn a_stream_with_no_d38_records_is_refused_not_reported_as_zero() {
        let mut s = Vec::new();
        s.extend(record(1, 100, 20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &[0u8; 8]));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_malformed_d38_payload_is_refused_not_skipped() {
        // A short or inconsistent d38 payload is a corrupt stream, not an hour to drop
        // quietly. The whole rejection space, not one sample.
        let mut invalid: Vec<Vec<u8>> = Vec::new();
        // Truncated payload (`world_fold.rs:15449`).
        let mut short = d38_payload(0, &[live(37, 12, 512, 7249, 100_000)], 1999);
        short.truncate(short.len() - 1);
        invalid.push(short);
        // Shorter than the header (`:15383`).
        invalid.push(vec![0u8; d38::HEADER_LEN - 1]);
        // changed_rows > row_count (`:15397`).
        let mut bad = d38_payload(0, &[live(37, 12, 512, 7249, 100_000)], 1999);
        bad[12..16].copy_from_slice(&2u32.to_le_bytes());
        invalid.push(bad);
        // observed_loadout_rows > observed_air_wing_rows (`:15404`).
        let mut bad = d38_payload(0, &[live(37, 12, 512, 7249, 100_000)], 1999);
        bad[24..28].copy_from_slice(&644u32.to_le_bytes());
        invalid.push(bad);
        // invalid_variant_rows > observed_equipment_rows (`:15411`).
        let mut bad = d38_payload(0, &[live(37, 12, 512, 7249, 100_000)], 1999);
        bad[40..44].copy_from_slice(&2000u32.to_le_bytes());
        invalid.push(bad);
        // deleted_rows > row_count (`:15418`).
        let mut bad = d38_payload(0, &[live(37, 12, 512, 7249, 100_000)], 1999);
        bad[44..48].copy_from_slice(&2u32.to_le_bytes());
        invalid.push(bad);
        // row_count > observed_equipment_rows + deleted_rows (`:15425`).
        invalid.push(d38_payload(0, &[live(37, 12, 512, 7249, 100_000)], 0));
        // scale_raw != FIXED_SCALE (`:15432`).
        let mut bad = d38_payload(0, &[live(37, 12, 512, 7249, 100_000)], 1999);
        bad[48..52].copy_from_slice(&32_768u32.to_le_bytes());
        invalid.push(bad);
        // An unknown row flag bit (`:15467`).
        invalid.push(d38_payload(
            0,
            &[row(37, 12, 512, 7249, 1 << 5, 0, 0, 0, 0, 0)],
            1999,
        ));
        // A non-zero row reserved word (`:15470`).
        let mut bad = d38_payload(0, &[live(37, 12, 512, 7249, 100_000)], 1999);
        let at = d38::HEADER_LEN + 36;
        bad[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
        invalid.push(bad);

        for (i, payload) in invalid.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(1, 100, D38, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "invalid payload {i} must make the scan fail"
            );
        }
    }

    #[test]
    fn scan_stream_never_panics_on_arbitrary_bytes() {
        // Total over hostile input: the harness reads a file from disk and must return
        // a diagnosis, never unwind.
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..4_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let len = (state as usize) % 400;
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            // Half the cases keep a valid magic so the parse gets further in.
            if len >= 4 && state & 1 == 0 {
                bytes[..4].copy_from_slice(b"H4CR");
            }
            let _ = scan_stream(&bytes, &test_collection(), StreamIdentityMode::Legacy);
        }
        // And corruption of a genuinely well-formed stream, byte by byte.
        let good = good_stream();
        for i in 0..good.len() {
            let mut s = good.clone();
            s[i] ^= 0xFF;
            let _ = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy);
        }
    }

    #[test]
    fn a_gap_between_measured_hours_forces_a_baseline() {
        // `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the state part forces
        // the next state observation to be a `BASELINE`." d38's block cadence is
        // measured on the game clock, so the caller hands the clock decision in;
        // the gap clause is what this function adds on top of it.
        assert!(opens_baseline(true, None, 100), "a clock block start always opens");
        assert!(opens_baseline(true, Some(99), 100));
        assert!(opens_baseline(false, None, 100), "the first measured hour opens");
        assert!(
            opens_baseline(false, Some(100), 102),
            "a skipped hour must open a baseline mid-block"
        );
        assert!(
            opens_baseline(false, Some(100), 100),
            "a non-advancing hour must open a baseline"
        );
        assert!(
            !opens_baseline(false, Some(100), 101),
            "a contiguous mid-block hour stays on the delta chain"
        );
        // The clock edge cannot wrap into a false "contiguous".
        assert!(opens_baseline(false, Some(i32::MAX), i32::MIN));
    }

    #[test]
    fn an_hour_closed_without_a_d38_record_refuses_the_stream() {
        // The completeness bound on what this harness does NOT validate (module
        // docs): it cannot check other domains' payloads, so a d38 record
        // relabelled to another domain id — or dropped — must be caught by
        // counting, not silently measured as fewer hours.
        let mut s = good_stream();
        let mut pos = 0usize;
        let mut target = None;
        while pos < s.len() {
            let header = parse_compact_record_header(&s[pos..]).expect("well-formed");
            if header.domain_id == D38 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
            {
                target = Some(pos);
            }
            pos += COMPACT_RECORD_HEADER_LEN + header.payload_len;
        }
        let target = target.expect("good_stream carries a d38 DATA record");
        // domain_id sits at +20: magic(4) + version(2) + header_len(2) +
        // sequence(8) + game_hour(4).
        s[target + 20..target + 22].copy_from_slice(&99u16.to_le_bytes());
        let err = match scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy) {
            Err(e) => e,
            Ok(_) => panic!("an hour with no d38 record must refuse the stream"),
        };
        assert!(
            err.contains("game hour(s) but carries"),
            "the completeness check must be what rejects it: {err}"
        );
    }
}
