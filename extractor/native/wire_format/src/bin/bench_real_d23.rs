//! Measure the wire format against the shipping compact stream on REAL d23
//! `map_detail_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d23-scheduled capture
//! (`runs/batch1/deepwar_t1`). The stream's own d23 DATA records are the
//! shipping format's cost for the domain: a 48-byte header, then five
//! fixed-stride arrays — states, provinces, and three shared entry tables the
//! rows address by `(first, len)` — full snapshot once and changed rows only
//! thereafter, exactly what the recorder stores today. This tool replays those
//! records into per-hour states — the same fold `world_fold::fold_map_detail`
//! performs — and re-encodes every hour with this crate's grammar: a BASELINE at
//! each 48-hour block start, field-granular stored-minimal DELTAs between.
//!
//! **d23 is incremental, and that shapes this harness.** Unlike d63, one record
//! is not one state: a full snapshot clears everything the domain owns and
//! repopulates (`world_fold.rs:22394`-`:22403`), and every later record upserts
//! the rows the collector saw change. An hour's state is therefore the fold of
//! every usable record up to that hour, which is why the accumulation lives in
//! `wire_format::d23::fold_payload` and not here — one implementation, shared
//! with the tests, so a harness cannot drift from the recorder silently.
//!
//! **Nothing here re-derives a contract it can call.** Payload, row and entry
//! decoding go through `wire_format::d23`'s public boundary; H4CR framing,
//! record semantics, sequence continuity and the failure-status predicate go
//! through `compact_contract`'s own parser, stateful validator and helper.
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
//!   (`recorder_host/src/stream_fold.rs:277`): a DOMAIN_DATA record opens its
//!   game hour, a FRAME_END closes the open hour and only that hour, and closed
//!   hours strictly increase;
//! - that tracker's DOMAIN_PERF hour binding (`:328`-`:365`), the DATA arm's
//!   pending-PERF rule (`:378`-`:386`), and the FRAME_END seal fields
//!   (`:46`-`:89`) plus domain-count cross-check (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END;
//! - every d23 payload through both production authorities. The fold-side header
//!   boundary is `d23::split_payload`; the status-independent authority is
//!   `decode_compact_map_detail_data_payload`
//!   (`recorder_host/src/domain_data_authority.rs:16043`-`:16294`, with its range
//!   and building helpers at `:16679`-`:16731`). It checks header relationships,
//!   ids, uniqueness, ranges, resources and buildings on EXCLUDED records too, so
//!   [`check_authority_payload`] mirrors every remaining predicate before status
//!   exclusion.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority. The only payload gap is malformed DATA for another domain:
//! `wire_format` cannot depend on `recorder_host`, and this harness implements
//! only d23. The measured capture was recorder-validated, so those records have
//! already passed production's authority.
//!
//! These seal, PERF, pending-PERF, payload-authority and stated-identity checks
//! are the five-check sweep. The capture is legacy and recorder-valid, so the
//! default remains legacy and no accepted record or output byte changes.
//!
//! Three exclusion rules, all of them the fold's:
//!
//! - Failure-status records fold nothing (`handle_compact_failure` retains the
//!   accumulator, `world_fold.rs:22266`-`:22273`) — excluded from BOTH arms,
//!   using the fold's own predicate
//!   (`compact_status_failure_flags_after_expected_partial_tolerance`), never a
//!   hand-copied mask literal.
//! - A delta before any baseline folds nothing (`:22275`-`:22280`) — excluded
//!   the same way, and it does not mark the domain baselined.
//! - Hour uniqueness is enforced across ALL d23 records, including
//!   failure-status ones, BEFORE status exclusion, or a success+failure pair in
//!   one hour slips through.
//!
//! A stream cut exactly at a FRAME_END boundary is a well-formed shorter stream
//! and is accepted as one.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d23.bin   — the d23 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface (changed rows per hour at stored
//! precision) and the wire arm's baseline/delta split, because for a domain this
//! static the block re-baseline policy, not the row grammar, is the dominant
//! term.
//!
//! **Only d23 DATA payloads are validated.** Production runs
//! `validate_domain_data_payload_at` on the DATA payload of EVERY domain
//! (`stream_fold.rs:593`); this harness cannot, because validating another
//! domain's payload means owning that domain's authority, and `wire_format`
//! must not depend on `recorder_host`. Every DATA record of any other domain
//! therefore passes with its H4CR header, sequence continuity, hour-boundary,
//! PERF-binding and FRAME_END rules checked but its **payload bytes unread**.
//! The concrete hazard — a d23 record relabelled to another domain id, or
//! dropped outright, silently shrinking the measurement — is closed by an
//! explicit completeness check in `scan_stream`: every closed game hour must
//! carry exactly one d23 DATA record, of any status. That is a per-capture
//! assertion, not a grammar rule; a capture where d23 is not scheduled every
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
    apply, check_minimal, decode_baseline, decode_delta, diff, encode_baseline, encode_delta,
    KeyedDelta, KeyedState,
};
use wire_format::d23::{self, MapDetailState, COLLECTION_COUNT};

const D23: u16 = 23;
const BLOCK_HOURS: usize = 48;

/// Whether the wire arm opens a BASELINE for this measured hour.
///
/// Two reasons, and the second is not a cadence choice: the 48-hour block
/// boundary, or a measured hour that does not immediately follow the previous
/// measured one. An hour this domain did not successfully sample is coverage
/// the domain lost, which is §8's gap, and `WIRE_FORMAT.md:187`-`:190` is
/// explicit: "A gap affecting the state part forces the next state observation
/// to be a `BASELINE`. State may have moved and returned while nobody was
/// looking, so neither a delta nor an `UNCHANGED` can be justified across
/// unobserved time." Delta-encoding across the gap would produce bytes that
/// decode — the reader's prior is never invalidated, so the local replay
/// cannot notice — while asserting a continuity the capture does not have. The
/// delta chain restarts at the forced baseline: every later delta chains from
/// it, exactly as after a scheduled block start.
fn opens_baseline(hours_seen: usize, previous_measured_hour: Option<i32>, game_hour: i32) -> bool {
    if hours_seen % BLOCK_HOURS == 0 {
        return true;
    }
    match previous_measured_hour {
        Some(previous) => game_hour != previous.saturating_add(1),
        None => true,
    }
}
// Re-exported, not re-declared: a local `(1 << 14) - 1` became wrong the moment
// the contract assigned bit 14 to the measured-empty badge.
const FRAME_END_KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;
const PROBE_DOMAIN_ID: u16 = 250;

/// One usable d23 record: its game hour, its byte span in the source stream (the
/// shipping arm is those bytes verbatim, header included), the span of its
/// payload, and whether it is an incremental update.
///
/// The state is NOT stored per hour — d23's rosters run to ~14,000 rows and the
/// capture to ~900 hours, so a state per hour is a gigabyte of records for no
/// gain. The scan keeps spans and folds into a scratch accumulator to prove
/// every record decodes; the measuring pass folds the same records again into
/// the real accumulator.
struct D23Frame {
    game_hour: i32,
    ship_span: Range<usize>,
    payload_span: Range<usize>,
    delta: bool,
}

struct StreamScan {
    frames: Vec<D23Frame>,
    /// d23 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// Incremental records that arrived before any full snapshot.
    delta_before_baseline: usize,
    /// Measured d23 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
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

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("bounded row word"),
    )
}

fn i32_at(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("bounded row word"),
    )
}

/// The status-independent d23 authority
/// (`domain_data_authority.rs:16043`-`:16294`, helpers `:16679`-`:16731`).
fn check_authority_payload(
    header: &d23::PayloadHeader,
    tables: &d23::Tables<'_>,
    at_byte: usize,
) -> Result<(), String> {
    const MIN_STATE_ROWS: usize = 1_000;
    const MIN_PROVINCE_ROWS: usize = 10_000;
    const MAX_STATE_ID: u32 = 65_535;
    const MAX_PROVINCE_ID: u32 = 65_536;
    const MAX_RESOURCE_AMOUNT_RAW: i32 = 100_000_000;
    const MAX_BUILDING_LEVEL: u32 = 1_000;

    let delta = header.status_flags & COMPACT_STATUS_DELTA_DATA != 0;
    let state_count = header.state_count as usize;
    let province_count = header.province_count as usize;
    let emitted_rows = state_count.saturating_add(province_count);
    let invalid = |reason: String| Err(format!("d23 authority at byte {at_byte}: {reason}"));
    if !delta && state_count < MIN_STATE_ROWS {
        return invalid(format!("state_count {state_count} < {MIN_STATE_ROWS}"));
    }
    if province_count > 0 && !delta && province_count < MIN_PROVINCE_ROWS {
        return invalid(format!(
            "province_count {province_count} < {MIN_PROVINCE_ROWS}"
        ));
    }
    if state_count > header.observed_state_rows as usize {
        return invalid("state_count exceeds observed_state_rows".to_string());
    }
    if province_count > header.observed_province_rows as usize {
        return invalid("province_count exceeds observed_province_rows".to_string());
    }
    if header.changed_rows as usize > emitted_rows {
        return invalid("changed_rows exceeds emitted rows".to_string());
    }
    if delta && header.changed_rows as usize != emitted_rows {
        return invalid("delta changed_rows does not equal emitted rows".to_string());
    }

    let mut seen_states = BTreeSet::new();
    for (row_index, row) in tables.states.chunks_exact(d23::STATE_ROW_LEN).enumerate() {
        d23::state_row_ranges(header, row)
            .map_err(|e| format!("d23 authority state row {row_index} at byte {at_byte}: {e}"))?;
        let state_id = u32_at(row, 0);
        if state_id > MAX_STATE_ID {
            return invalid(format!(
                "state row {row_index} has invalid state_id {state_id}"
            ));
        }
        if !seen_states.insert(state_id) {
            return invalid(format!(
                "state row {row_index} duplicates state_id {state_id}"
            ));
        }
    }

    let mut seen_provinces = BTreeSet::new();
    for (row_index, row) in tables
        .provinces
        .chunks_exact(d23::PROVINCE_ROW_LEN)
        .enumerate()
    {
        d23::province_row_range(header, row).map_err(|e| {
            format!("d23 authority province row {row_index} at byte {at_byte}: {e}")
        })?;
        let province_id = u32_at(row, 0);
        let state_id = u32_at(row, 4);
        if province_id > MAX_PROVINCE_ID {
            return invalid(format!(
                "province row {row_index} has invalid province_id {province_id}"
            ));
        }
        if !seen_provinces.insert(province_id) {
            return invalid(format!(
                "province row {row_index} duplicates province_id {province_id}"
            ));
        }
        if state_id > MAX_STATE_ID {
            return invalid(format!(
                "province row {row_index} has invalid state_id {state_id}"
            ));
        }
    }

    for (row_index, row) in tables
        .resources
        .chunks_exact(d23::ENTRY_ROW_LEN)
        .enumerate()
    {
        d23::resource_type_id(row).map_err(|e| {
            format!("d23 authority resource row {row_index} at byte {at_byte}: {e}")
        })?;
        let amount = i32_at(row, 4);
        if !(0..=MAX_RESOURCE_AMOUNT_RAW).contains(&amount) {
            return invalid(format!(
                "resource row {row_index} has invalid amount_raw {amount}"
            ));
        }
    }
    for (scope, rows) in [
        ("state", tables.state_buildings),
        ("province", tables.province_buildings),
    ] {
        for (row_index, row) in rows.chunks_exact(d23::ENTRY_ROW_LEN).enumerate() {
            d23::building_type_id(row).map_err(|e| {
                format!("d23 authority {scope} building row {row_index} at byte {at_byte}: {e}")
            })?;
            let level = u32_at(row, 4);
            if level > MAX_BUILDING_LEVEL {
                return invalid(format!(
                    "{scope} building row {row_index} has invalid level {level}"
                ));
            }
        }
    }
    Ok(())
}

/// Validate the whole stream against the canonical contract and prove every
/// usable d23 DATA record folds. Every rejection path is an `Err`, never a
/// silently shorter measurement and never a panic — see the module docs for what
/// is enforced and what the recorder's own validator additionally checks.
fn scan_stream(
    stream: &[u8],
    schema: &wire_format::Schema,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut frames: Vec<D23Frame> = Vec::new();
    let mut failure_records = 0usize;
    let mut delta_before_baseline = 0usize;
    let mut non_contiguous = 0usize;

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d23 DATA record of ANY status — the uniqueness rule.
    let mut last_d23_hour: Option<i32> = None;
    // Hour of the last d23 record that entered the measurement — the contiguity
    // report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;
    // `delta_baselines_seen` for domain 23 (`world_fold.rs:22443`): set only by a
    // full snapshot that folded successfully.
    let mut baseline_seen = false;
    // The scratch accumulator: folding here is what proves every row decodes.
    let mut scratch = MapDetailState::new();

    // Closed game hours and d23 DATA records of ANY status — the
    // completeness rule at the end of the scan.
    let mut closed_hours = 0usize;
    let mut d23_records = 0usize;
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
                                "DATA hour conflict at byte {pos}: pending PERF hour {pending}, \
                                 then DATA hour {}",
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
                let observed_perf_domains =
                    non_probe_perf_domains.len().min(u32::MAX as usize) as u32;
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
                open_hour = None;
                pending_perf_hour = None;
                non_probe_perf_domains.clear();
                last_closed_hour = Some(header.game_hour_raw);
                closed_hours += 1;
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        if header.domain_id == D23 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d23_records += 1;
            // The fold checks the payload's structure BEFORE it looks at the
            // status (`world_fold.rs:22246`-`:22265` precedes `:22266`), so a
            // malformed payload is an error even on a record that would have
            // folded nothing.
            let (d23_header, tables) = d23::split_payload(payload)
                .map_err(|e| format!("d23 payload at byte {pos}: {e}"))?;

            // One d23 DATA record per game hour, checked over EVERY d23 record
            // and BEFORE any status-based exclusion. Two records in one hour
            // would become two measurement hours built from the same clock time,
            // and gating this on the record being usable would let a
            // success/failure pair through the rule entirely.
            if let Some(previous) = last_d23_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d23 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d23_hour = Some(header.game_hour_raw);

            // VALIDITY BEFORE CLASSIFICATION. Everything below that reads the
            // status word — the failure predicate, the DELTA_DATA read —
            // classifies the v1 vocabulary; neither is a validity predicate.
            // `compact_contract/src/lib.rs:121`-`:123` says so in as many words:
            // "Callers handling untrusted status words must first reject a
            // non-zero `compact_status_unknown_flags_v1` result; otherwise an
            // unassigned bit has no defined classification." Bits 15..=31 are
            // unassigned (`status_flags_base.rs:40`-`:44`), and an unassigned bit
            // "is not evidence of a future informational status".
            //
            // Without this, a record with `status_flags = 1 << 14` reads as "not
            // a failure" — the failure helper masks with the *known* failure set
            // and returns zero for a bit it does not know — and the hour is
            // measured as healthy. The production lane rejects first
            // (`lanes/division_state_movement_live/domain20_compact.rs:419`);
            // this harness does the same rather than re-deriving a lenient rule.
            // No capture carries an unassigned bit, so no measured number moves.
            let unknown_status = compact_status_unknown_flags_v1(d23_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d23 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            check_authority_payload(&d23_header, &tables, pos)?;

            // `handle_compact_failure` (`world_fold.rs:2843`, called at `:22266`)
            // marks the domain-hour unknown and RETAINS the prior accumulator on
            // a failure status, folding nothing from this record. The predicate
            // is `compact_contract`'s own, not a copied bit mask: it already
            // applies d23's lane-local mask and expected partial tolerance, so it
            // cannot drift from the recorder.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D23,
                d23_header.status_flags,
            ) != 0
            {
                failure_records += 1;
                pos = record_end;
                continue;
            }

            let delta = d23_header.status_flags & COMPACT_STATUS_DELTA_DATA != 0;
            // `world_fold.rs:22275`-`:22280`: an incremental record with no
            // baseline behind it marks the domain unknown for the hour and folds
            // nothing.
            if delta && !baseline_seen {
                delta_before_baseline += 1;
                pos = record_end;
                continue;
            }

            // Fold exactly as `fold_map_detail` does. Every row the fold rejects
            // is rejected here, at the same point in the pass — so the measuring
            // pass cannot meet a record the scan accepted and then unwind.
            d23::fold_payload(schema, &mut scratch, payload, delta, 0)
                .map_err(|e| format!("d23 payload at byte {pos}: {e}"))?;
            if !delta {
                // `:22443`, and only after a fold that succeeded.
                baseline_seen = true;
            }

            if let Some(previous) = last_measured_hour {
                if header.game_hour_raw != previous + 1 {
                    non_contiguous += 1;
                }
            }
            last_measured_hour = Some(header.game_hour_raw);

            frames.push(D23Frame {
                game_hour: header.game_hour_raw,
                ship_span: pos..record_end,
                payload_span: payload_start..record_end,
                delta,
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
    if frames.is_empty() {
        return Err("stream carries no usable d23 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d23 DATA
    // record, of any status. Production validates the payload of EVERY domain's
    // DATA record (`stream_fold.rs:593`); this harness cannot (module docs), so
    // a d23 record relabelled to another domain id — or lost outright — would
    // otherwise be silently ignored and the run would report a plausible
    // measurement over fewer hours. Per-hour uniqueness is already enforced
    // above, so equality of the two counts is exactly-one per closed hour. A
    // per-capture assertion, not a grammar rule: a capture that does not
    // schedule d23 every hour fails here loudly and a human adjusts, instead
    // of a number quietly shrinking.
    if d23_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d23_records} d23 DATA \
             record(s); every closed hour of a d23-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    Ok(StreamScan {
        frames,
        failure_records,
        delta_before_baseline,
        non_contiguous,
    })
}

fn main() {
    const USAGE: &str = "usage: bench_real_d23 <collector_stream.bin> <out dir> \
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
    let schema = d23::schema();

    // ---- Pass 1: validate the WHOLE stream and prove every d23 record folds.
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result. Everything that
    // can fail on stream content fails here, with a diagnosis and no unwind.
    let scan = match scan_stream(&stream, &schema, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: fold the same records again and encode both arms. The fold is
    // the identical call sequence on the identical bytes, so nothing here can
    // fail on stream content. ------------------------------------------------

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut acc = MapDetailState::new();
    let mut reader: Vec<KeyedState> = vec![KeyedState::new(); COLLECTION_COUNT];
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    // Baselines the §1.5 gap rule forced outside a scheduled block start.
    let mut gap_forced_baselines = 0usize;
    let mut previous_measured_hour: Option<i32> = None;
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    let mut empty_hours = 0usize;
    let mut changed_rows_total = 0usize;
    let mut tombstones_total = 0usize;
    let mut rows_total = 0usize;

    for frame in &scan.frames {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[frame.ship_span.clone()]);
        ship_offsets.push(ship.len());

        // The hour's state: the fold of every usable record up to and including
        // this one. `prior` is what both the encoder and the reader hold.
        let prior = acc.clone();
        d23::fold_payload(
            &schema,
            &mut acc,
            &stream[frame.payload_span.clone()],
            frame.delta,
            0,
        )
        .expect("pass 1 accepted this payload");
        rows_total += acc.rows();

        // New arm: BASELINE at each block start, stored-minimal DELTA between.
        let hour_start = wire.len();
        let baseline_here = opens_baseline(hours_seen, previous_measured_hour, frame.game_hour);
        if baseline_here && hours_seen % BLOCK_HOURS != 0 {
            gap_forced_baselines += 1;
        }
        let mut emitted = true;
        if baseline_here {
            encode_baseline(&schema, acc.as_slice(), &mut wire).expect("baseline encodes");
            baselines += 1;
            baseline_bytes += wire.len() - hour_start;
        } else {
            let deltas: Vec<Option<KeyedDelta>> = (0..COLLECTION_COUNT)
                .map(|i| {
                    let d = diff(&schema.collections[i], prior.get(i), acc.get(i));
                    (!d.is_empty()).then_some(d)
                })
                .collect();
            if deltas.iter().all(Option::is_none) {
                empty_hours += 1;
                emitted = false;
            } else {
                for (i, delta) in deltas.iter().enumerate() {
                    let Some(delta) = delta else { continue };
                    changed_rows_total += delta.changed.len();
                    tombstones_total += delta.tombstones.len();
                    // §7.1 minimality, checked rather than assumed: `diff` is
                    // supposed to produce the one minimal delta, and this is the
                    // rule that says so.
                    check_minimal(&schema.collections[i], prior.get(i), acc.get(i), delta)
                        .unwrap_or_else(|e| panic!("hour {}: {e:?}", frame.game_hour));
                }
                encode_delta(&schema, prior.as_slice(), &deltas, &mut wire).expect("delta encodes");
                delta_bytes += wire.len() - hour_start;
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
            if baseline_here {
                let (states, consumed) =
                    decode_baseline(&schema, hour_bytes, 0).expect("baseline decodes");
                assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
                reader = states;
            } else {
                let (deltas, consumed) =
                    decode_delta(&schema, &reader, hour_bytes, 0).expect("delta decodes");
                assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
                for (i, delta) in deltas.iter().enumerate() {
                    if let Some(delta) = delta {
                        apply(&mut reader[i], delta);
                    }
                }
            }
        }
        for i in 0..COLLECTION_COUNT {
            assert_eq!(
                reader[i].len(),
                acc.get(i).len(),
                "game hour {}: collection {} row count",
                frame.game_hour,
                schema.collections[i].collection_id
            );
            assert!(
                diff(&schema.collections[i], &reader[i], acc.get(i)).is_empty(),
                "game hour {}: collection {} is not stored-equal to the fold",
                frame.game_hour,
                schema.collections[i].collection_id
            );
        }

        previous_measured_hour = Some(frame.game_hour);
        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d23.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d23.offsets", &ship_offsets),
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
        "rows avg             {:.1} across the five collections",
        rows_total as f64 / hours_seen as f64
    );
    println!(
        "baselines            {baselines} ({} every {BLOCK_HOURS} h, {gap_forced_baselines} \
         forced by a gap in measured hours — WIRE_FORMAT §1.5)",
        baselines - gap_forced_baselines
    );
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "real churn           {:.2} changed rows/hour, {:.3} tombstones/hour (stored precision)",
        changed_rows_total as f64 / delta_hours.max(1) as f64,
        tombstones_total as f64 / delta_hours.max(1) as f64
    );
    println!("ship_d23.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.1}%) = {baseline_bytes} baseline + {delta_bytes} delta",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::COMPACT_DOMAIN_FRAME;

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

    /// A one-state, one-province d23 DATA payload: the 48-byte header, one state
    /// row carrying one resource entry, one province row carrying no buildings.
    fn d23_payload(status_flags: u32, state_id: u32, amount_raw: i32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        payload.extend_from_slice(&1u32.to_le_bytes()); // state_count
        payload.extend_from_slice(&2u32.to_le_bytes()); // changed_rows
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes()); // province_count
        payload.extend_from_slice(&1u32.to_le_bytes()); // observed_state_rows
        payload.extend_from_slice(&1u32.to_le_bytes()); // observed_province_rows
        payload.extend_from_slice(&1u32.to_le_bytes()); // resource_count
        payload.extend_from_slice(&0u32.to_le_bytes()); // state_building_count
        payload.extend_from_slice(&0u32.to_le_bytes()); // province_building_count
        payload.extend_from_slice(&0u32.to_le_bytes()); // reserved
                                                        // state row: resources flagged, one entry at 0.
        payload.extend_from_slice(&state_id.to_le_bytes());
        payload.extend_from_slice(&d23::STATE_FLAG_RESOURCES.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes()); // resource_first
        payload.extend_from_slice(&1u32.to_le_bytes()); // resource_len
        payload.extend_from_slice(&0u32.to_le_bytes()); // building_first
        payload.extend_from_slice(&0u32.to_le_bytes()); // building_len
        payload.extend_from_slice(&0u32.to_le_bytes()); // reserved
                                                        // province row: in the state, no building list.
        payload.extend_from_slice(&11u32.to_le_bytes());
        payload.extend_from_slice(&state_id.to_le_bytes());
        payload.extend_from_slice(&d23::PROVINCE_FLAG_STATE.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        // resource entry.
        payload.extend_from_slice(&0x3E03u32.to_le_bytes());
        payload.extend_from_slice(&amount_raw.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(
            payload.len(),
            d23::HEADER_LEN + d23::STATE_ROW_LEN + d23::PROVINCE_ROW_LEN + d23::ENTRY_ROW_LEN
        );
        payload
    }

    fn valid_baseline_payload(status_flags: u32, amount_raw: i32) -> Vec<u8> {
        assert_eq!(status_flags & COMPACT_STATUS_DELTA_DATA, 0);
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&1_000u32.to_le_bytes());
        payload.extend_from_slice(&1_000u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&1_000u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        for index in 0..1_000u32 {
            let state_id = index + 7;
            payload.extend_from_slice(&state_id.to_le_bytes());
            payload.extend_from_slice(
                &(if index == 0 {
                    d23::STATE_FLAG_RESOURCES
                } else {
                    0
                })
                .to_le_bytes(),
            );
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&(u32::from(index == 0)).to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        payload.extend_from_slice(&0x3E03u32.to_le_bytes());
        payload.extend_from_slice(&amount_raw.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload
    }

    fn perf(sequence: u64, game_hour: i32, domain: u16) -> Vec<u8> {
        record(
            sequence,
            game_hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &vec![0u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN],
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
        let mut payload = Vec::with_capacity(COMPACT_FRAME_END_PAYLOAD_LEN);
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&emitted.to_le_bytes());
        payload.extend_from_slice(&scheduled.to_le_bytes());
        payload.extend_from_slice(&status.to_le_bytes());
        payload.extend_from_slice(&reserved.to_le_bytes());
        record(
            sequence,
            game_hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &payload,
        )
    }

    /// Two closed hours: a full snapshot, then an incremental update.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D23,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &valid_baseline_payload(0, 100_000),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D23,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d23_payload(COMPACT_STATUS_DELTA_DATA, 7, 200_000),
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

    fn scan(stream: &[u8]) -> Result<StreamScan, String> {
        scan_stream(stream, &d23::schema(), StreamIdentityMode::Legacy)
    }

    #[test]
    fn a_well_formed_stream_yields_its_d23_hours() {
        let s = scan(&good_stream()).expect("accepted");
        assert_eq!(s.frames.len(), 2);
        assert_eq!(s.frames[0].game_hour, 100);
        assert!(!s.frames[0].delta, "the first record is a full snapshot");
        assert_eq!(s.frames[1].game_hour, 101);
        assert!(s.frames[1].delta);
        assert_eq!(s.failure_records, 0);
        assert_eq!(s.delta_before_baseline, 0);
        assert_eq!(s.non_contiguous, 0);
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helpers classify
        // the v1 vocabulary and are not validity predicates, so an unassigned bit
        // must be rejected BEFORE classification — otherwise it silently reads as
        // "not a failure", gets a baseline/delta meaning from the DELTA_DATA
        // read, and the hour is measured as healthy.
        // Bit 15 is ASSIGNED (`COMPACT_STATUS_BASELINE_CLEAR`, the stream's deliberate
        // resync primitive), so the UNASSIGNED run starts at 16.
        for bit in 16..32u32 {
            let status = 1u32 << bit;
            // Alone, combined with a known failure bit, and combined with the
            // informational DELTA_DATA bit. The rejection must not depend on it,
            // nor on whether a baseline has been seen.
            for extra in [0u32, 1, COMPACT_STATUS_DELTA_DATA] {
                let mut s = Vec::new();
                s.extend(record(
                    1,
                    100,
                    D23,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &valid_baseline_payload(0, 100_000),
                ));
                s.extend(frame_end(2, 100));
                s.extend(record(
                    3,
                    101,
                    D23,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d23_payload(status | extra, 7, 200_000),
                ));
                s.extend(frame_end(4, 101));
                assert!(
                    scan(&s).is_err(),
                    "status bit {bit} (+{extra:#x}) after a baseline must be refused"
                );
                // And as the stream's first record, before any baseline.
                let mut first = Vec::new();
                first.extend(record(
                    1,
                    100,
                    D23,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d23_payload(status | extra, 7, 100_000),
                ));
                first.extend(frame_end(2, 100));
                assert!(
                    scan(&first).is_err(),
                    "status bit {bit} (+{extra:#x}) as the first record must be refused"
                );
            }
        }
        // And the whole known v1 mask, minus the failure bits, is still accepted:
        // the check must reject unassigned bits and nothing else. The second hour
        // carries the informational bit, so DELTA_DATA lands after a baseline and
        // is measured as the incremental record it is.
        for informational in [
            1u32 << 4,
            1 << 5,
            COMPACT_STATUS_DELTA_DATA,
            1 << 11,
            1 << 13,
        ] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D23,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &valid_baseline_payload(0, 100_000),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D23,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &if informational & COMPACT_STATUS_DELTA_DATA != 0 {
                    d23_payload(informational, 7, 200_000)
                } else {
                    valid_baseline_payload(informational, 200_000)
                },
            ));
            s.extend(frame_end(4, 101));
            let result =
                scan(&s).unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(result.frames.len(), 2, "bit {informational:#x}");
            assert_eq!(result.failure_records, 0, "bit {informational:#x}");
            assert_eq!(result.delta_before_baseline, 0, "bit {informational:#x}");
        }
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        // A cut immediately after a d23 DATA record leaves that hour open and
        // ends the stream on a record that is not a FRAME_END. That would
        // otherwise produce a plausible shorter number.
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan(&full[..cut]).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary IS a well-formed shorter stream,
        // and `eof_check` accepts it too. What matters is that it reports the
        // hours it actually closed.
        for (records, expected_hours) in [(2usize, 1usize), (4, 2)] {
            let cut = boundary(&full, records);
            let s = scan(&full[..cut]).expect("closed stream is well-formed");
            assert_eq!(s.frames.len(), expected_hours, "cut after {records}");
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
            assert!(scan(&full[..cut]).is_err(), "mid-record cut at {cut}");
        }
        assert!(scan(&[]).is_err(), "an empty stream is not a measurement");
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
                    scan(&s).is_err(),
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
                assert!(scan(&s).is_err(), "record {records} sequence {wrong}");
            }
        }
    }

    #[test]
    fn a_frame_end_that_does_not_close_its_open_hour_is_rejected() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D23,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &valid_baseline_payload(0, 1),
        ));
        s.extend(frame_end(2, 999)); // closes an hour that is not open
        assert!(scan(&s).is_err());

        let mut s = Vec::new();
        s.extend(frame_end(1, 100));
        assert!(scan(&s).is_err());
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            101,
            D23,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &valid_baseline_payload(0, 1),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D23,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d23_payload(0, 7, 2),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan(&s).is_err());
    }

    #[test]
    fn a_second_d23_record_in_one_hour_is_rejected() {
        // Two records in one hour would become two measurement hours built from
        // the same clock time. The rule is checked over EVERY d23 DATA record
        // and before status-based exclusion, so all four success/failure
        // orderings are refused.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D23,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &valid_baseline_payload(first, 1),
            ));
            s.extend(record(
                2,
                100,
                D23,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d23_payload(second, 7, 2),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan(&s).is_err(),
                "two d23 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time: the
        // fold retains the accumulator and folds nothing, so the hour is not d23
        // data and must not enter either arm.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D23,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &valid_baseline_payload(1 << bit, 1),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D23,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &valid_baseline_payload(0, 2),
            ));
            s.extend(frame_end(4, 101));
            let scanned = scan(&s).expect("the stream itself is well-formed");
            assert_eq!(scanned.failure_records, 1, "bit {bit}");
            assert_eq!(scanned.frames.len(), 1, "bit {bit}");
            assert_eq!(scanned.frames[0].game_hour, 101, "bit {bit}");
        }
        // And DELTA_DATA (1 << 10) is NOT a failure bit: behind a baseline it is
        // an ordinary measured hour.
        let scanned = scan(&good_stream()).expect("well-formed");
        assert_eq!(scanned.failure_records, 0);
        assert_eq!(scanned.frames.len(), 2);
        assert!(scanned.frames[1].delta);
    }

    #[test]
    fn a_delta_before_any_baseline_is_excluded_and_does_not_baseline_the_domain() {
        // `world_fold.rs:22275`-`:22280`. Two deltas, then a snapshot, then a
        // delta: only the last two hours fold.
        let mut s = Vec::new();
        let mut seq = 1u64;
        let mut hour = 100i32;
        for status in [
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
            0,
            COMPACT_STATUS_DELTA_DATA,
        ] {
            let payload = if status & COMPACT_STATUS_DELTA_DATA != 0 {
                d23_payload(status, 7, 1)
            } else {
                valid_baseline_payload(status, 1)
            };
            s.extend(record(
                seq,
                hour,
                D23,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(seq + 1, hour));
            seq += 2;
            hour += 1;
        }
        let scanned = scan(&s).expect("well-formed");
        assert_eq!(scanned.delta_before_baseline, 2);
        assert_eq!(
            scanned
                .frames
                .iter()
                .map(|f| f.game_hour)
                .collect::<Vec<_>>(),
            vec![102, 103]
        );
        // The measured hours are contiguous with each other; the excluded ones
        // are not counted as a break because measurement starts at 102.
        assert_eq!(scanned.non_contiguous, 0);
    }

    #[test]
    fn a_failure_status_snapshot_does_not_baseline_the_domain() {
        // A snapshot that folds nothing cannot license the deltas after it
        // (`world_fold.rs:22266` returns before `:22443`). The trailing good
        // snapshot is what keeps this a stream with a measurement in it, so the
        // assertion is about the two excluded hours and not about emptiness.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D23,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &valid_baseline_payload(1, 1),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D23,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d23_payload(COMPACT_STATUS_DELTA_DATA, 7, 2),
        ));
        s.extend(frame_end(4, 101));
        s.extend(record(
            5,
            102,
            D23,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &valid_baseline_payload(0, 3),
        ));
        s.extend(frame_end(6, 102));
        let scanned = scan(&s).expect("well-formed");
        assert_eq!(scanned.failure_records, 1);
        assert_eq!(scanned.delta_before_baseline, 1);
        assert_eq!(
            scanned
                .frames
                .iter()
                .map(|f| f.game_hour)
                .collect::<Vec<_>>(),
            vec![102]
        );
    }

    #[test]
    fn a_stream_with_no_d23_records_is_refused_not_reported_as_zero() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &[0u8; 8],
        ));
        s.extend(frame_end(2, 100));
        assert!(scan(&s).is_err());
    }

    #[test]
    fn a_malformed_d23_payload_is_refused_not_skipped() {
        // A short or inconsistent payload is a corrupt stream, not an hour to
        // drop quietly.
        let mut payload = d23_payload(0, 7, 1);
        payload.truncate(payload.len() - 1);
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D23,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        assert!(scan(&s).is_err());

        // A row the FOLD rejects must make the SCAN fail, not be accepted and
        // then unwind while measuring. One case per rejection class.
        let mut invalid: Vec<Vec<u8>> = Vec::new();
        // zero state_id (`world_fold.rs:22295`)
        invalid.push(d23_payload(0, 0, 1));
        // an unknown state flag bit (`:22300`)
        let mut unknown_flag = d23_payload(0, 7, 1);
        let flags_at = d23::HEADER_LEN + 4;
        unknown_flag[flags_at..flags_at + 4]
            .copy_from_slice(&(d23::STATE_FLAG_RESOURCES | 1 << 20).to_le_bytes());
        invalid.push(unknown_flag);
        // a nonzero header reserved word (`:22263`)
        let mut reserved = d23_payload(0, 7, 1);
        reserved[44..48].copy_from_slice(&1u32.to_le_bytes());
        invalid.push(reserved);
        // a resource range past its table (`:29892`)
        let mut range = d23_payload(0, 7, 1);
        let len_at = d23::HEADER_LEN + 12;
        range[len_at..len_at + 4].copy_from_slice(&2u32.to_le_bytes());
        invalid.push(range);
        // an entry with a zero type_id (`:22377`)
        let mut entry = d23_payload(0, 7, 1);
        let entry_at = d23::HEADER_LEN + d23::STATE_ROW_LEN + d23::PROVINCE_ROW_LEN;
        entry[entry_at..entry_at + 4].copy_from_slice(&0u32.to_le_bytes());
        invalid.push(entry);
        // a province whose state flag and state id disagree (`:22349`)
        let mut province = d23_payload(0, 7, 1);
        let state_at = d23::HEADER_LEN + d23::STATE_ROW_LEN + 4;
        province[state_at..state_at + 4].copy_from_slice(&0u32.to_le_bytes());
        invalid.push(province);
        for (i, payload) in invalid.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D23,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            assert!(
                scan(&s).is_err(),
                "invalid payload {i} must make the scan fail"
            );
        }
    }

    #[test]
    fn a_malformed_payload_is_refused_even_on_a_failure_status_record() {
        // The fold checks the payload's structure before it looks at the status
        // (`world_fold.rs:22246` precedes `:22266`), so a corrupt frame is
        // corrupt whatever it claims about itself.
        let mut payload = d23_payload(1, 7, 1);
        payload[44..48].copy_from_slice(&1u32.to_le_bytes());
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D23,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        assert!(scan(&s).is_err());
    }

    #[test]
    fn the_scan_folds_incrementally_so_a_delta_row_is_validated_in_context() {
        // The scan's scratch accumulator must carry across records, or a delta
        // record's rows would be checked against an empty world. Fold the whole
        // good stream and confirm the second record's amount is what survived.
        let schema = d23::schema();
        let s = scan(&good_stream()).expect("accepted");
        let stream = good_stream();
        let mut acc = MapDetailState::new();
        for frame in &s.frames {
            d23::fold_payload(
                &schema,
                &mut acc,
                &stream[frame.payload_span.clone()],
                frame.delta,
                0,
            )
            .expect("folds");
        }
        assert_eq!(acc.get(d23::STATE_RESOURCES_INDEX).len(), 1);
        assert_eq!(acc.get(d23::PROVINCES_INDEX).len(), 1);
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        let data = |sequence, hour| {
            record(
                sequence,
                hour,
                D23,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &valid_baseline_payload(0, 1),
            )
        };
        let mut ok = Vec::new();
        ok.extend(data(1, 100));
        ok.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        assert!(scan(&ok).is_ok());

        let mut bad = vec![
            frame_end_sealed(2, 100, 0, 0, 0, 1),
            frame_end_sealed(2, 100, 0, 1, 0, 0),
        ];
        // Bit 15 is ASSIGNED (`COMPACT_STATUS_BASELINE_CLEAR`, the stream's deliberate
        // resync primitive), so the UNASSIGNED run starts at 16.
        for bit in 16..32u32 {
            bad.push(frame_end_sealed(2, 100, 0, 0, 1 << bit, 0));
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            bad.push(frame_end_sealed(2, 100, 0, 0, 1 << bit, 0));
        }
        for seal in bad {
            let mut s = Vec::new();
            s.extend(data(1, 100));
            s.extend(seal);
            assert!(scan(&s).is_err());
        }
        for len in [0usize, 1, 16, 31, 33, 64] {
            let mut s = Vec::new();
            s.extend(data(1, 100));
            s.extend(record(
                2,
                100,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                &vec![0u8; len],
            ));
            assert!(scan(&s).is_err(), "{len}-byte FRAME_END");
        }
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(data(1, 100));
            s.extend(frame_end_sealed(2, 100, 0, 0, informational, 0));
            assert!(scan(&s).is_ok(), "informational status {informational:#x}");
        }
        let mut later = good_stream();
        let fourth = boundary(&later, 3);
        later[fourth + COMPACT_RECORD_HEADER_LEN + 28..fourth + COMPACT_RECORD_HEADER_LEN + 32]
            .copy_from_slice(&1u32.to_le_bytes());
        assert!(scan(&later).is_err());
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        let data = |sequence, hour| {
            record(
                sequence,
                hour,
                D23,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &valid_baseline_payload(0, 1),
            )
        };
        let mut ok = Vec::new();
        ok.extend(perf(1, 100, 20));
        ok.extend(perf(2, 100, D23));
        ok.extend(perf(3, 100, PROBE_DOMAIN_ID));
        ok.extend(data(4, 100));
        ok.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
        assert!(scan(&ok).is_ok());

        let mut probe_twice = Vec::new();
        probe_twice.extend(perf(1, 100, PROBE_DOMAIN_ID));
        probe_twice.extend(perf(2, 100, PROBE_DOMAIN_ID));
        probe_twice.extend(data(3, 100));
        probe_twice.extend(frame_end_sealed(4, 100, 0, 0, 0, 0));
        assert!(scan(&probe_twice).is_ok());

        let mut duplicate = Vec::new();
        duplicate.extend(perf(1, 100, 20));
        duplicate.extend(perf(2, 100, 20));
        duplicate.extend(data(3, 100));
        duplicate.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        assert!(scan(&duplicate).is_err());

        let mut interleaved = Vec::new();
        interleaved.extend(data(1, 100));
        interleaved.extend(perf(2, 999, 20));
        interleaved.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan(&interleaved).is_err());

        let mut stale = Vec::new();
        stale.extend(data(1, 100));
        stale.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        stale.extend(perf(3, 100, 20));
        stale.extend(data(4, 101));
        stale.extend(frame_end_sealed(5, 101, 1, 1, 0, 0));
        assert!(scan(&stale).is_err());

        let mut conflict = Vec::new();
        conflict.extend(perf(1, 100, 20));
        conflict.extend(perf(2, 101, 21));
        conflict.extend(data(3, 100));
        conflict.extend(frame_end_sealed(4, 100, 2, 2, 0, 0));
        assert!(scan(&conflict).is_err());

        for (emitted, scheduled) in [(1u32, 1u32), (3, 3), (2, 3), (3, 2)] {
            let mut s = Vec::new();
            s.extend(perf(1, 100, 20));
            s.extend(perf(2, 100, D23));
            s.extend(data(3, 100));
            s.extend(frame_end_sealed(4, 100, emitted, scheduled, 0, 0));
            assert!(scan(&s).is_err());
        }

        let mut two_hours = Vec::new();
        two_hours.extend(perf(1, 100, 20));
        two_hours.extend(data(2, 100));
        two_hours.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        two_hours.extend(perf(4, 101, 20));
        two_hours.extend(data(5, 101));
        two_hours.extend(frame_end_sealed(6, 101, 1, 1, 0, 0));
        assert_eq!(scan(&two_hours).expect("PERF state resets").frames.len(), 2);
    }

    #[test]
    fn a_data_record_must_open_the_hour_the_pending_perf_records_bound() {
        let data = |sequence, hour| {
            record(
                sequence,
                hour,
                D23,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &valid_baseline_payload(0, 1),
            )
        };
        for (perf_hour, data_hour) in [(100i32, 101i32), (101, 100), (100, 200), (200, 100)] {
            let mut s = Vec::new();
            s.extend(perf(1, perf_hour, 20));
            s.extend(data(2, data_hour));
            s.extend(frame_end_sealed(3, data_hour, 1, 1, 0, 0));
            assert!(scan(&s).is_err(), "PERF {perf_hour}, DATA {data_hour}");
        }
        let mut agreeing = Vec::new();
        agreeing.extend(perf(1, 100, 20));
        agreeing.extend(data(2, 100));
        agreeing.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan(&agreeing).is_ok());
    }

    #[test]
    fn an_excluded_record_is_still_checked_by_the_payload_authority() {
        const FAILED_DELTA: u32 = 1 | COMPACT_STATUS_DELTA_DATA;
        let follow_with_baseline = |payload: Vec<u8>| {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D23,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D23,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &valid_baseline_payload(0, 1),
            ));
            s.extend(frame_end(4, 101));
            s
        };

        let mut bad = Vec::new();
        bad.push(d23_payload(1, 7, 1)); // full-snapshot minimum counts
        let mut header = d23_payload(FAILED_DELTA, 7, 1);
        header[24..28].copy_from_slice(&0u32.to_le_bytes());
        bad.push(header);
        let mut changed = d23_payload(FAILED_DELTA, 7, 1);
        changed[12..16].copy_from_slice(&1u32.to_le_bytes());
        bad.push(changed);
        bad.push(d23_payload(FAILED_DELTA, 65_536, 1));

        let mut duplicate_state = d23_payload(FAILED_DELTA, 7, 1);
        let state = duplicate_state[d23::HEADER_LEN..d23::HEADER_LEN + d23::STATE_ROW_LEN].to_vec();
        duplicate_state.splice(
            d23::HEADER_LEN + d23::STATE_ROW_LEN..d23::HEADER_LEN + d23::STATE_ROW_LEN,
            state,
        );
        duplicate_state[8..12].copy_from_slice(&2u32.to_le_bytes());
        duplicate_state[12..16].copy_from_slice(&3u32.to_le_bytes());
        duplicate_state[24..28].copy_from_slice(&2u32.to_le_bytes());
        bad.push(duplicate_state);

        let mut province_id = d23_payload(FAILED_DELTA, 7, 1);
        province_id[d23::HEADER_LEN + d23::STATE_ROW_LEN..d23::HEADER_LEN + d23::STATE_ROW_LEN + 4]
            .copy_from_slice(&65_537u32.to_le_bytes());
        bad.push(province_id);
        let mut province_state = d23_payload(FAILED_DELTA, 7, 1);
        province_state
            [d23::HEADER_LEN + d23::STATE_ROW_LEN + 4..d23::HEADER_LEN + d23::STATE_ROW_LEN + 8]
            .copy_from_slice(&65_536u32.to_le_bytes());
        bad.push(province_state);

        let mut unknown_flags = d23_payload(FAILED_DELTA, 7, 1);
        unknown_flags[d23::HEADER_LEN + 4..d23::HEADER_LEN + 8]
            .copy_from_slice(&(1u32 << 20).to_le_bytes());
        bad.push(unknown_flags);
        let mut bad_range = d23_payload(FAILED_DELTA, 7, 1);
        bad_range[d23::HEADER_LEN + 12..d23::HEADER_LEN + 16].copy_from_slice(&2u32.to_le_bytes());
        bad.push(bad_range);
        bad.push(d23_payload(FAILED_DELTA, 7, -1));
        bad.push(d23_payload(FAILED_DELTA, 7, 100_000_001));

        for (type_id, level, reserved) in [(0u32, 1u32, 0u32), (1, 1_001, 0), (1, 1, 1)] {
            let mut building = d23_payload(FAILED_DELTA, 7, 1);
            building[36..40].copy_from_slice(&1u32.to_le_bytes());
            building.extend_from_slice(&type_id.to_le_bytes());
            building.extend_from_slice(&level.to_le_bytes());
            building.extend_from_slice(&reserved.to_le_bytes());
            bad.push(building);
        }

        for (index, payload) in bad.into_iter().enumerate() {
            assert!(
                scan(&follow_with_baseline(payload)).is_err(),
                "excluded malformed authority payload {index}"
            );
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let payload = d23_payload((1 << bit) | COMPACT_STATUS_DELTA_DATA, 0, 1);
            assert!(
                scan(&follow_with_baseline(payload)).is_err(),
                "failure bit {bit}"
            );
        }

        let scanned = scan(&follow_with_baseline(d23_payload(FAILED_DELTA, 7, 1)))
            .expect("a well-formed excluded delta does not reach the fold-only baseline rule");
        assert_eq!(scanned.failure_records, 1);
        assert_eq!(scanned.delta_before_baseline, 0);
        assert_eq!(scanned.frames.len(), 1);
    }

    #[test]
    fn the_identity_protocol_is_stated_not_sniffed() {
        use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;

        let schema = d23::schema();
        let legacy = good_stream();
        assert!(scan_stream(&legacy, &schema, StreamIdentityMode::IdentityV1).is_err());
        assert!(scan_stream(&legacy, &schema, StreamIdentityMode::Legacy).is_ok());
        let mut identity = record(
            0,
            0,
            0,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            b"CID1-not-a-real-identity-payload",
        );
        identity.extend(legacy);
        assert!(scan_stream(&identity, &schema, StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn scan_stream_never_panics_on_arbitrary_bytes() {
        // Total over hostile input: the harness reads a file from disk and must
        // return a diagnosis, never unwind.
        let schema = d23::schema();
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
            if len >= 4 && state & 1 == 0 {
                bytes[..4].copy_from_slice(b"H4CR");
            }
            let _ = scan_stream(&bytes, &schema, StreamIdentityMode::Legacy);
        }
        // And corruption of a genuinely well-formed stream, byte by byte.
        let good = good_stream();
        for i in (0..good.len()).step_by(67) {
            let mut s = good.clone();
            s[i] ^= 0xFF;
            let _ = scan_stream(&s, &schema, StreamIdentityMode::Legacy);
        }
    }

    #[test]
    fn a_gap_between_measured_hours_forces_a_baseline() {
        // `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the state part forces
        // the next state observation to be a `BASELINE`." An excluded or missing
        // hour between two measured ones is such a gap; delta-encoding across it
        // would decode cleanly while asserting a continuity the capture does not
        // have. The decision function, over its whole input space:
        for hours_seen in 0..(2 * BLOCK_HOURS) {
            assert!(
                opens_baseline(hours_seen, None, 100),
                "the first measured hour at position {hours_seen} opens a baseline"
            );
            assert!(
                opens_baseline(hours_seen, Some(100), 102),
                "a skipped hour at position {hours_seen} must open a baseline"
            );
            assert!(
                opens_baseline(hours_seen, Some(100), 100),
                "a non-advancing hour at position {hours_seen} must open a baseline"
            );
            assert_eq!(
                opens_baseline(hours_seen, Some(100), 101),
                hours_seen % BLOCK_HOURS == 0,
                "a contiguous hour at position {hours_seen} is cadence-only"
            );
        }
        // The clock edge cannot wrap into a false "contiguous".
        assert!(opens_baseline(1, Some(i32::MAX), i32::MIN));
    }

    #[test]
    fn an_hour_closed_without_a_d23_record_refuses_the_stream() {
        // The completeness bound on what this harness does NOT validate (module
        // docs): it cannot check other domains' payloads, so a d23 record
        // relabelled to another domain id — or dropped — must be caught by
        // counting, not silently measured as fewer hours.
        let mut s = good_stream();
        // Find the LAST d23 DATA record and relabel it to an unclaimed domain
        // id. The stream stays well formed — sequence, hour boundaries and
        // FRAME_END are untouched — so the completeness rule is the only rule
        // that can catch it.
        let mut pos = 0usize;
        let mut target = None;
        while pos < s.len() {
            let header = parse_compact_record_header(&s[pos..]).expect("well-formed");
            if header.domain_id == D23 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
            {
                target = Some(pos);
            }
            pos += COMPACT_RECORD_HEADER_LEN + header.payload_len;
        }
        let target = target.expect("good_stream carries a d23 DATA record");
        // domain_id sits at +20: magic(4) + version(2) + header_len(2) +
        // sequence(8) + game_hour(4).
        s[target + 20..target + 22].copy_from_slice(&99u16.to_le_bytes());
        let err = match scan(&s) {
            Err(e) => e,
            Ok(_) => panic!("an hour with no d23 record must refuse the stream"),
        };
        assert!(
            err.contains("game hour(s) but carries"),
            "the completeness check must be what rejects it: {err}"
        );
    }
}
