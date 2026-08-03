//! Measure the wire format against the shipping compact stream on REAL d22
//! `map_control_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d22-scheduled capture
//! (`runs/batch1/deepwar_t1`). The stream's own d22 DATA records are the shipping
//! format's cost for the domain: a 32-byte header plus a fixed 16-byte row per
//! carried state and per carried province, **incremental** — a record with
//! `COMPACT_STATUS_DELTA_DATA` clear replaces both tables, and with it set carries
//! only the rows that changed. That is exactly what the recorder stores today.
//! This tool replays those records into per-hour state/province tables — the same
//! fold `world_fold::fold_map_control` performs, through
//! `wire_format::d22::Accumulator` so there is one implementation of it — and
//! re-encodes every hour with this crate's grammar: a BASELINE at each 48-hour
//! block start, field-granular stored-minimal DELTAs between.
//!
//! **The two incremental layers are distinct.** The shipping baseline/delta split
//! is decoded by the accumulator; the wire baseline/delta split is decided here by
//! block position and by `body::diff`. A shipping DELTA that changed nothing is a
//! wire UNCHANGED hour, and a wire tombstone appears only where a shipping
//! BASELINE dropped a key. See `d22.rs`'s module docs.
//!
//! **Nothing here re-derives a contract it can call.** Payload and row decoding go
//! through `wire_format::d22`'s public boundary; H4CR framing, record semantics,
//! sequence continuity, the domain-hour failure predicate and the delta status bit
//! go through `compact_contract`'s own parser, validator and constants. A harness
//! that reimplements either drifts from the recorder without anything failing.
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
//!   one, no PERF for an already-closed hour, and no conflict with the pending PERF
//!   hour before a DATA record opens one;
//! - the **FRAME_END seal fields**, `validate_frame_end_seal_fields`
//!   (`:46`-`:89`), re-implemented here as [`check_frame_end_seal`]: the 32-byte
//!   payload length, `reserved == 0`, `emitted_domains == scheduled_domains`, no
//!   status bit above 13, and no fatal status bit; plus the seal's domain-count
//!   cross-check against the distinct non-probe PERF domains observed in the hour
//!   (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END;
//! - every d22 payload through both production authorities. The fold-side header
//!   boundary is `d22::split_payload`; the status-independent certification
//!   authority is `decode_compact_map_control_data_payload`
//!   (`recorder_host/src/domain_data_authority.rs:15839`-`:16041`). It additionally
//!   checks baseline/count relationships and walks every state and province row,
//!   including EXCLUDED records, so [`check_authority_payload`] mirrors those
//!   remaining predicates before status exclusion.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority. The only payload gap is malformed data for some *other*
//! domain: `wire_format` cannot depend on `recorder_host`, and this one-domain
//! harness can only replay d22's authority. The measured capture was
//! recorder-validated, so its other domains have already passed that check.
//!
//! The seal fields, PERF binding, DATA pending-PERF rule, status-independent d22
//! payload authority and stated identity mode are the five-check sweep. The
//! capture is legacy and recorder-valid, so the new default and checks preserve
//! every byte of its measurement.
//!
//! **Two passes, deliberately.** d22 carries ~14,500 rows per hour over ~900
//! hours, so retaining every hour's folded state — as the d63 harness does for its
//! ~100 countries — would hold ~13 million records live. Pass 1 validates the
//! whole stream and folds every record through a single running accumulator it
//! then discards, keeping only each usable hour's byte span; pass 2 re-folds those
//! spans through the same `Accumulator` and encodes each hour as it goes. The
//! property the pilot's review established is preserved: everything that can fail
//! on stream *content* fails in pass 1, and pass 2 asserts only about the codec.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d22.bin   — the d22 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface (changed rows per hour at stored
//! precision), per collection.
//!
//! **Only d22 DATA payloads are validated.** Production runs
//! `validate_domain_data_payload_at` on the DATA payload of EVERY domain
//! (`stream_fold.rs:593`); this harness cannot, because validating another
//! domain's payload means owning that domain's authority, and `wire_format`
//! must not depend on `recorder_host`. Every DATA record of any other domain
//! therefore passes with its H4CR header, sequence continuity, hour-boundary,
//! PERF-binding and FRAME_END rules checked but its **payload bytes unread**.
//! The concrete hazard — a d22 record relabelled to another domain id, or
//! dropped outright, silently shrinking the measurement — is closed by an
//! explicit completeness check in `scan_stream`: every closed game hour must
//! carry exactly one d22 DATA record, of any status. That is a per-capture
//! assertion, not a grammar rule; a capture where d22 is not scheduled every
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
use wire_format::d22::{self, Accumulator, FoldOutcome};
use wire_format::Collection;

const D22: u16 = 22;
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

/// Every globally-defined compact status occupies one of bits 0..=14
/// (`recorder_host/src/stream_fold.rs:43`).
// Re-exported, not re-declared: a local `(1 << 14) - 1` became wrong the moment
// the contract assigned bit 14 to the measured-empty badge.
const FRAME_END_KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;

/// The reader-context probe domain, excluded from seal accounting
/// (`recorder_host/src/stream_fold.rs:330`).
const PROBE_DOMAIN_ID: u16 = 250;

/// One usable d22 hour: its game hour and its byte span in the source stream
/// (the shipping arm is those bytes verbatim, header included).
///
/// No folded state is retained — see the module docs on the two passes.
struct D22Hour {
    game_hour: i32,
    ship_span: Range<usize>,
}

struct StreamScan {
    hours: Vec<D22Hour>,
    /// d22 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// d22 DELTA records that arrived before any BASELINE. `world_fold.rs:22173`
    /// marks the domain-hour unknown and folds nothing, so the hour carries no
    /// d22 data and enters neither arm.
    delta_before_baseline_records: usize,
    /// Measured d22 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// How the shipping layer split across the measured hours.
    shipping_baselines: usize,
    shipping_deltas: usize,
    /// Rows the shipping records carried, summed over measured hours.
    shipped_state_rows: usize,
    shipped_province_rows: usize,
}

/// `COMPACT_STATUS_DELTA_DATA`, read through `compact_contract` rather than as a
/// copied literal (`world_fold.rs:22172` reads the same constant).
fn is_delta(status_flags: u32) -> bool {
    status_flags & COMPACT_STATUS_DELTA_DATA != 0
}

/// Mirror `validate_frame_end_seal_fields`
/// (`recorder_host/src/stream_fold.rs:46`-`:89`).
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

/// Replay the d22 predicates made by
/// `decode_compact_map_control_data_payload`
/// (`recorder_host/src/domain_data_authority.rs:15839`-`:16041`) that
/// `d22::split_payload` does not already enforce. This authority runs on every
/// record regardless of status, so this function must run before exclusion.
fn check_authority_payload(
    header: &d22::PayloadHeader,
    state_rows: &[u8],
    province_rows: &[u8],
    at_byte: usize,
) -> Result<(), String> {
    const MIN_STATE_ROWS: usize = 1_000;
    const MIN_PROVINCE_ROWS: usize = 10_000;
    const MAX_STATE_ID: u32 = 65_535;
    const MAX_PROVINCE_ID: u32 = 65_536;
    const MAX_TAG_DB_ID: u32 = 1_024;

    let state_count = header.state_count as usize;
    let province_count = header.province_count as usize;
    let changed_state_rows = header.changed_state_rows as usize;
    let changed_province_rows = header.changed_province_rows as usize;
    let delta = is_delta(header.status_flags);
    let invalid = |reason: String| Err(format!("d22 authority at byte {at_byte}: {reason}"));

    if !delta && state_count < MIN_STATE_ROWS {
        return invalid(format!("state_count {state_count} < {MIN_STATE_ROWS}"));
    }
    if !delta && province_count < MIN_PROVINCE_ROWS {
        return invalid(format!(
            "province_count {province_count} < {MIN_PROVINCE_ROWS}"
        ));
    }
    if changed_state_rows > state_count {
        return invalid("changed_state_rows exceeds state_count".to_string());
    }
    if changed_province_rows > province_count {
        return invalid("changed_province_rows exceeds province_count".to_string());
    }
    if header.changed_rows as usize != changed_state_rows.saturating_add(changed_province_rows) {
        return invalid("changed_rows does not equal changed state+province rows".to_string());
    }
    if delta && changed_state_rows != state_count {
        return invalid("delta changed_state_rows does not equal state_count".to_string());
    }
    if delta && changed_province_rows != province_count {
        return invalid("delta changed_province_rows does not equal province_count".to_string());
    }

    let mut seen_states = BTreeSet::new();
    for (row_index, row) in state_rows.chunks_exact(d22::STATE_ROW_LEN).enumerate() {
        let state_id = u32_at(row, 0);
        let owner = u32_at(row, 4);
        let controller = u32_at(row, 8);
        let flags = u32_at(row, 12);
        if state_id == 0 || state_id > MAX_STATE_ID {
            return invalid(format!(
                "state row {row_index} has invalid state_id {state_id}"
            ));
        }
        if !seen_states.insert(state_id) {
            return invalid(format!(
                "state row {row_index} duplicates state_id {state_id}"
            ));
        }
        if owner > MAX_TAG_DB_ID || controller > MAX_TAG_DB_ID {
            return invalid(format!(
                "state row {row_index} has an out-of-range country id"
            ));
        }
        if flags & !(d22::FLAG_STATE_OWNER | d22::FLAG_STATE_CONTROLLER) != 0 {
            return invalid(format!(
                "state row {row_index} has unknown flags {flags:#x}"
            ));
        }
        if (flags & d22::FLAG_STATE_OWNER != 0) != (owner != 0) {
            return invalid(format!("state row {row_index} has inconsistent owner gate"));
        }
        if (flags & d22::FLAG_STATE_CONTROLLER != 0) != (controller != 0) {
            return invalid(format!(
                "state row {row_index} has inconsistent controller gate"
            ));
        }
    }

    let mut seen_provinces = BTreeSet::new();
    for (row_index, row) in province_rows
        .chunks_exact(d22::PROVINCE_ROW_LEN)
        .enumerate()
    {
        let province_id = u32_at(row, 0);
        let controller = u32_at(row, 4);
        let state_id = u32_at(row, 8);
        let flags = u32_at(row, 12);
        if province_id == 0 || province_id > MAX_PROVINCE_ID {
            return invalid(format!(
                "province row {row_index} has invalid province_id {province_id}"
            ));
        }
        if !seen_provinces.insert(province_id) {
            return invalid(format!(
                "province row {row_index} duplicates province_id {province_id}"
            ));
        }
        if controller > MAX_TAG_DB_ID || state_id > MAX_STATE_ID {
            return invalid(format!("province row {row_index} has an out-of-range id"));
        }
        if !delta && state_id != 0 && !seen_states.contains(&state_id) {
            return invalid(format!(
                "province row {row_index} references unknown state_id {state_id}"
            ));
        }
        if flags & !(d22::FLAG_PROVINCE_CONTROLLER | d22::FLAG_PROVINCE_STATE_ID) != 0 {
            return invalid(format!(
                "province row {row_index} has unknown flags {flags:#x}"
            ));
        }
        if (flags & d22::FLAG_PROVINCE_CONTROLLER != 0) != (controller != 0) {
            return invalid(format!(
                "province row {row_index} has inconsistent controller gate"
            ));
        }
        if (flags & d22::FLAG_PROVINCE_STATE_ID != 0) != (state_id != 0) {
            return invalid(format!(
                "province row {row_index} has inconsistent state gate"
            ));
        }
    }
    Ok(())
}

/// Validate the whole stream against the canonical contract and fold every usable
/// d22 DATA record. Every rejection path is an `Err`, never a silently shorter
/// measurement and never a panic — see the module docs for what is enforced and
/// what the recorder's own validator additionally checks.
fn scan_stream(
    stream: &[u8],
    states: &Collection,
    provinces: &Collection,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    // The identity protocol is an input, never sniffed. Production takes it from
    // its caller (`recorder_host/src/stream_fold.rs:220`); otherwise a decapitated
    // IdentityV1 stream is indistinguishable from legacy bytes beginning at
    // sequence 1 and would be measured instead of refused.
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut scan = StreamScan {
        hours: Vec::new(),
        failure_records: 0,
        delta_before_baseline_records: 0,
        non_contiguous: 0,
        shipping_baselines: 0,
        shipping_deltas: 0,
        shipped_state_rows: 0,
        shipped_province_rows: 0,
    };

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d22 DATA record of ANY status — the uniqueness rule.
    let mut last_d22_hour: Option<i32> = None;
    // Hour of the last d22 record that entered the measurement — the contiguity
    // report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;
    // The pass-1 accumulator, kept only so that the fold's own ordering state
    // (`baseline_seen`) and every row's decodability are checked here.
    let mut accumulator = Accumulator::new();

    // Closed game hours and d22 DATA records of ANY status — the
    // completeness rule at the end of the scan.
    let mut closed_hours = 0usize;
    let mut d22_records = 0usize;
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

        if header.domain_id == D22 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d22_records += 1;
            // `world_fold.rs:22156` checks the length BEFORE `:22164` checks the
            // status, so a malformed failure record is still a malformed record.
            let (d22_header, state_rows, province_rows) = d22::split_payload(payload)
                .map_err(|e| format!("d22 payload at byte {pos}: {e}"))?;

            // One d22 DATA record per game hour, checked over EVERY d22 record and
            // BEFORE any status-based exclusion. The domain is incremental, so a
            // second record in one hour would apply two shipping frames to one
            // measurement hour — and gating this on the record being usable would
            // let a success/failure pair through the rule entirely.
            if let Some(previous) = last_d22_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d22 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d22_hour = Some(header.game_hour_raw);

            // VALIDITY BEFORE CLASSIFICATION. Everything below that reads the
            // status word — the failure predicate, `is_delta` — classifies the v1
            // vocabulary; neither is a validity predicate.
            // `compact_contract/src/lib.rs:121`-`:123` says so in as many words:
            // "Callers handling untrusted status words must first reject a
            // non-zero `compact_status_unknown_flags_v1` result; otherwise an
            // unassigned bit has no defined classification." Bits 15..=31 are
            // unassigned (`status_flags_base.rs:21`-`:24`), and an unassigned bit
            // "is not evidence of a future informational status".
            //
            // Without this, a record with `status_flags = 1 << 14` reads as "not
            // a failure" — the failure helper masks with the *known* failure set
            // and returns zero for a bit it does not know — and the hour is
            // measured as healthy. The production lane rejects first
            // (`lanes/division_state_movement_live/domain20_compact.rs:419`);
            // this harness does the same rather than re-deriving a lenient rule.
            // No capture carries an unassigned bit, so no measured number moves.
            let unknown_status = compact_status_unknown_flags_v1(d22_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d22 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            check_authority_payload(&d22_header, state_rows, province_rows, pos)?;

            // `world_fold::handle_compact_failure` marks the domain-hour unknown
            // and RETAINS the prior accumulator on a failure status
            // (`world_fold.rs:22164`-`:22171`), folding nothing from this record.
            // The predicate is `compact_contract`'s own, not a copied bit mask: it
            // already applies d22's lane-local mask and expected partial
            // tolerance, so it cannot drift from the recorder.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D22,
                d22_header.status_flags,
            ) != 0
            {
                scan.failure_records += 1;
                pos = record_end;
                continue;
            }

            let delta = is_delta(d22_header.status_flags);
            let outcome = accumulator
                .fold_payload(states, provinces, payload, delta)
                .map_err(|e| format!("d22 payload at byte {pos}: {e}"))?;
            if outcome == FoldOutcome::DeltaBeforeBaseline {
                // `world_fold.rs:22173`-`:22178`: nothing folded, accumulator
                // retained, domain-hour unknown. Not this domain's data.
                scan.delta_before_baseline_records += 1;
                pos = record_end;
                continue;
            }

            if let Some(previous) = last_measured_hour {
                if header.game_hour_raw != previous + 1 {
                    scan.non_contiguous += 1;
                }
            }
            last_measured_hour = Some(header.game_hour_raw);
            match outcome {
                FoldOutcome::Baseline => scan.shipping_baselines += 1,
                FoldOutcome::Delta => scan.shipping_deltas += 1,
                FoldOutcome::DeltaBeforeBaseline => unreachable!("handled above"),
            }
            scan.shipped_state_rows += d22_header.state_count as usize;
            scan.shipped_province_rows += d22_header.province_count as usize;
            scan.hours.push(D22Hour {
                game_hour: header.game_hour_raw,
                ship_span: pos..record_end,
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
    if scan.hours.is_empty() {
        return Err("stream carries no usable d22 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d22 DATA
    // record, of any status. Production validates the payload of EVERY domain's
    // DATA record (`stream_fold.rs:593`); this harness cannot (module docs), so
    // a d22 record relabelled to another domain id — or lost outright — would
    // otherwise be silently ignored and the run would report a plausible
    // measurement over fewer hours. Per-hour uniqueness is already enforced
    // above, so equality of the two counts is exactly-one per closed hour. A
    // per-capture assertion, not a grammar rule: a capture that does not
    // schedule d22 every hour fails here loudly and a human adjusts, instead
    // of a number quietly shrinking.
    if d22_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d22_records} d22 DATA \
             record(s); every closed hour of a d22-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    Ok(scan)
}

fn main() {
    const USAGE: &str = "usage: bench_real_d22 <collector_stream.bin> <out dir> \
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
    let schema = d22::schema();
    let states = schema.collections[0].clone();
    let provinces = schema.collections[1].clone();

    // ---- Pass 1: validate the WHOLE stream and fold its d22 DATA records. ----
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result. Everything that can
    // fail on stream content fails here, with a diagnosis and no unwind.
    let scan = match scan_stream(&stream, &states, &provinces, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: re-fold the accepted spans and encode both arms. Nothing here
    // can fail on stream content — pass 1 already accepted every byte it reads —
    // so every assertion below is about the codec. -----------------------------

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut accumulator = Accumulator::new();
    let mut prior: Option<[KeyedState; 2]> = None;
    let mut reader: Option<[KeyedState; 2]> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    // Baselines the §1.5 gap rule forced outside a scheduled block start.
    let mut gap_forced_baselines = 0usize;
    let mut previous_measured_hour: Option<i32> = None;
    let mut empty_hours = 0usize;
    let mut changed_states_total = 0usize;
    let mut changed_provinces_total = 0usize;
    let mut tombstones_total = 0usize;
    let mut state_rows_total = 0usize;
    let mut province_rows_total = 0usize;

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());

        // Re-fold this hour's shipping record into the running accumulator. Both
        // calls go through `d22`'s own boundary, so pass 2 cannot read the payload
        // differently from pass 1.
        let payload = &stream[hour.ship_span.start + COMPACT_RECORD_HEADER_LEN..hour.ship_span.end];
        let (d22_header, _, _) = d22::split_payload(payload).expect("pass 1 accepted this payload");
        let outcome = accumulator
            .fold_payload(
                &states,
                &provinces,
                payload,
                is_delta(d22_header.status_flags),
            )
            .expect("pass 1 accepted this payload");
        assert_ne!(
            outcome,
            FoldOutcome::DeltaBeforeBaseline,
            "game hour {}: pass 1 excluded every unknown hour",
            hour.game_hour
        );
        let state = [accumulator.states.clone(), accumulator.provinces.clone()];
        state_rows_total += state[0].len();
        province_rows_total += state[1].len();

        // New arm: BASELINE at each block start, stored-minimal DELTA between.
        let hour_start = wire.len();
        let baseline_here = opens_baseline(hours_seen, previous_measured_hour, hour.game_hour);
        if baseline_here && hours_seen % BLOCK_HOURS != 0 {
            gap_forced_baselines += 1;
        }
        let mut emitted = true;
        if baseline_here || prior.is_none() {
            encode_baseline(&schema, &state, &mut wire).expect("baseline encodes");
            baselines += 1;
        } else {
            let prev = prior.as_ref().unwrap();
            let ds = diff(&states, &prev[0], &state[0]);
            let dp = diff(&provinces, &prev[1], &state[1]);
            if ds.is_empty() && dp.is_empty() {
                empty_hours += 1;
                emitted = false;
            } else {
                changed_states_total += ds.changed.len();
                changed_provinces_total += dp.changed.len();
                tombstones_total += ds.tombstones.len() + dp.tombstones.len();
                // §7's collection change bitmap: a collection that did not move
                // contributes nothing at all, which is the common case here.
                let contributions = [
                    if ds.is_empty() { None } else { Some(ds) },
                    if dp.is_empty() { None } else { Some(dp) },
                ];
                encode_delta(&schema, prev, &contributions, &mut wire).expect("delta encodes");
            }
        }
        wire_offsets.push(wire.len());

        // Reader replay: decode the bytes just written against the reader's OWN
        // reconstructed prior, exactly as a conforming reader would, and require
        // the result to be stored-equal to this hour's state. This is what makes
        // the size claim a claim about decodable bytes. An hour that emitted
        // nothing is still checked: the reader's retained state must already equal
        // the fold's, which is precisely what "nothing changed" asserts.
        if emitted {
            let hour_bytes = &wire[hour_start..];
            if baseline_here || reader.is_none() {
                let (decoded, consumed) =
                    decode_baseline(&schema, hour_bytes, 0).expect("baseline decodes");
                assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
                assert_eq!(decoded.len(), 2, "two collections in the body");
                let mut it = decoded.into_iter();
                reader = Some([it.next().unwrap(), it.next().unwrap()]);
            } else {
                let reader_prior = reader.take().unwrap();
                let (deltas, consumed) =
                    decode_delta(&schema, &reader_prior, hour_bytes, 0).expect("delta decodes");
                assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
                let mut next = reader_prior;
                for (i, delta) in deltas.iter().enumerate() {
                    if let Some(delta) = delta {
                        apply(&mut next[i], delta);
                    }
                }
                reader = Some(next);
            }
        }
        let replayed = reader.as_ref().unwrap();
        for (i, collection) in [&states, &provinces].into_iter().enumerate() {
            assert_eq!(
                replayed[i].len(),
                state[i].len(),
                "game hour {}: collection {} row count",
                hour.game_hour,
                collection.collection_id
            );
            assert!(
                diff(collection, &replayed[i], &state[i]).is_empty(),
                "game hour {}: reader collection {} is not stored-equal to the fold",
                hour.game_hour,
                collection.collection_id
            );
        }

        prior = Some(state);
        previous_measured_hour = Some(hour.game_hour);
        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d22.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d22.offsets", &ship_offsets),
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
        scan.delta_before_baseline_records
    );
    println!(
        "shipping layer       {} BASELINE, {} DELTA records; {} state rows + {} province rows carried",
        scan.shipping_baselines,
        scan.shipping_deltas,
        scan.shipped_state_rows,
        scan.shipped_province_rows
    );
    println!(
        "table size avg       {:.1} states, {:.1} provinces",
        state_rows_total as f64 / hours_seen as f64,
        province_rows_total as f64 / hours_seen as f64
    );
    println!(
        "baselines            {baselines} ({} every {BLOCK_HOURS} h, {gap_forced_baselines} \
         forced by a gap in measured hours — WIRE_FORMAT §1.5)",
        baselines - gap_forced_baselines
    );
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "real churn           {:.3} changed states/hour, {:.3} changed provinces/hour, \
         {:.3} tombstones/hour (stored precision, over {delta_hours} delta hours)",
        changed_states_total as f64 / delta_hours.max(1) as f64,
        changed_provinces_total as f64 / delta_hours.max(1) as f64,
        tombstones_total as f64 / delta_hours.max(1) as f64,
    );
    println!("ship_d22.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.1}%)",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::COMPACT_DOMAIN_FRAME;

    fn collections() -> (Collection, Collection) {
        let schema = d22::schema();
        (schema.collections[0].clone(), schema.collections[1].clone())
    }

    fn scan(stream: &[u8]) -> Result<StreamScan, String> {
        let (states, provinces) = collections();
        scan_stream(stream, &states, &provinces, StreamIdentityMode::Legacy)
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

    fn state_row(state_id: u32, owner: u32, controller: u32, flags: u32) -> Vec<u8> {
        let mut row = Vec::new();
        for v in [state_id, owner, controller, flags] {
            row.extend_from_slice(&v.to_le_bytes());
        }
        row
    }

    fn province_row(province_id: u32, controller: u32, state_id: u32, flags: u32) -> Vec<u8> {
        let mut row = Vec::new();
        for v in [province_id, controller, state_id, flags] {
            row.extend_from_slice(&v.to_le_bytes());
        }
        row
    }

    /// A d22 DATA payload: the 32-byte header then the two row runs.
    fn d22_payload(status_flags: u32, states: &[Vec<u8>], provinces: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        out.extend_from_slice(&(states.len() as u32).to_le_bytes());
        out.extend_from_slice(&((states.len() + provinces.len()) as u32).to_le_bytes());
        out.extend_from_slice(&status_flags.to_le_bytes());
        out.extend_from_slice(&(provinces.len() as u32).to_le_bytes());
        out.extend_from_slice(&(states.len() as u32).to_le_bytes());
        out.extend_from_slice(&(provinces.len() as u32).to_le_bytes());
        for row in states.iter().chain(provinces) {
            out.extend_from_slice(row);
        }
        out
    }

    /// The authority's minimum full snapshot: 1,000 states and 10,000 provinces.
    fn valid_baseline_payload(status_flags: u32) -> Vec<u8> {
        assert_eq!(status_flags & COMPACT_STATUS_DELTA_DATA, 0);
        let states: Vec<Vec<u8>> = (1..=1_000).map(|id| state_row(id, 7, 7, 3)).collect();
        let provinces: Vec<Vec<u8>> = (1..=10_000)
            .map(|id| province_row(id, 7, (id - 1) % 1_000 + 1, 3))
            .collect();
        d22_payload(status_flags, &states, &provinces)
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

    /// Two closed hours: a shipping BASELINE then a shipping DELTA.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D22,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &valid_baseline_payload(0),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D22,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d22_payload(COMPACT_STATUS_DELTA_DATA, &[state_row(1, 8, 8, 3)], &[]),
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
    fn a_well_formed_stream_yields_its_d22_hours() {
        let s = scan(&good_stream()).expect("accepted");
        assert_eq!(s.hours.len(), 2);
        assert_eq!(s.hours[0].game_hour, 100);
        assert_eq!(s.hours[1].game_hour, 101);
        assert_eq!(s.failure_records, 0);
        assert_eq!(s.delta_before_baseline_records, 0);
        assert_eq!(s.non_contiguous, 0);
        assert_eq!(s.shipping_baselines, 1);
        assert_eq!(s.shipping_deltas, 1);
        assert_eq!(s.shipped_state_rows, 1_001);
        assert_eq!(s.shipped_province_rows, 10_000);
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helpers classify
        // the v1 vocabulary and are not validity predicates, so an unassigned bit
        // must be rejected BEFORE classification — otherwise it silently reads as
        // "not a failure", `is_delta` gives it a baseline/delta meaning, and the
        // hour is measured as healthy.
        for bit in 15..32u32 {
            let status = 1u32 << bit;
            // Alone, combined with a known failure bit, and combined with the
            // informational DELTA_DATA bit; with rows and without. The rejection
            // must not depend on any of it.
            for extra in [0u32, 1, COMPACT_STATUS_DELTA_DATA] {
                for states in [Vec::new(), vec![state_row(1, 7, 7, 3)]] {
                    let mut s = Vec::new();
                    s.extend(record(
                        1,
                        100,
                        D22,
                        COMPACT_RECORD_TYPE_DOMAIN_DATA,
                        &d22_payload(status | extra, &states, &[]),
                    ));
                    s.extend(frame_end(2, 100));
                    assert!(
                        scan(&s).is_err(),
                        "status bit {bit} (+{extra:#x}) with {} state row(s) must be refused",
                        states.len()
                    );
                }
            }
        }
        // And the whole known v1 mask, minus the failure bits, is still accepted:
        // the check must reject unassigned bits and nothing else. The second hour
        // carries the informational bit, so DELTA_DATA lands after a baseline and
        // is measured as the delta it is.
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
                D22,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &valid_baseline_payload(0),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D22,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &if informational & COMPACT_STATUS_DELTA_DATA != 0 {
                    d22_payload(informational, &[state_row(1, 8, 8, 3)], &[])
                } else {
                    valid_baseline_payload(informational)
                },
            ));
            s.extend(frame_end(4, 101));
            let result =
                scan(&s).unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(result.hours.len(), 2, "bit {informational:#x}");
            assert_eq!(result.failure_records, 0, "bit {informational:#x}");
            assert_eq!(
                result.delta_before_baseline_records, 0,
                "bit {informational:#x}"
            );
        }
    }

    #[test]
    fn a_delta_before_any_baseline_is_excluded_rather_than_measured() {
        // `world_fold.rs:22173`-`:22178`: the hour is marked unknown and folds
        // nothing, so it is not d22 data. Measuring it would report a state the
        // recorder never held.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D22,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d22_payload(COMPACT_STATUS_DELTA_DATA, &[state_row(1, 7, 7, 3)], &[]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D22,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &valid_baseline_payload(0),
        ));
        s.extend(frame_end(4, 101));
        let result = scan(&s).expect("the stream itself is well-formed");
        assert_eq!(result.delta_before_baseline_records, 1);
        assert_eq!(result.hours.len(), 1);
        assert_eq!(result.hours[0].game_hour, 101);
        // And a stream that is nothing BUT pre-baseline deltas has no measurement
        // in it at all, rather than an empty one.
        let mut only = Vec::new();
        only.extend(record(
            1,
            100,
            D22,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d22_payload(COMPACT_STATUS_DELTA_DATA, &[state_row(1, 7, 7, 3)], &[]),
        ));
        only.extend(frame_end(2, 100));
        assert!(scan(&only).is_err());
    }

    #[test]
    fn a_failure_status_record_is_excluded_and_leaves_the_baseline_state_alone() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time: the
        // fold retains the accumulator and folds nothing, so the hour is not d22
        // data and must not enter either arm. Critically for an incremental
        // domain, a failed BASELINE must not count as a baseline — a later delta
        // is still delta-before-baseline.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D22,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &valid_baseline_payload(1 << bit),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D22,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d22_payload(COMPACT_STATUS_DELTA_DATA, &[state_row(1, 8, 8, 3)], &[]),
            ));
            s.extend(frame_end(4, 101));
            // A third hour with a genuine BASELINE, so the stream has a
            // measurement in it and the two exclusions above are observable
            // rather than collapsing into "no usable records".
            s.extend(record(
                5,
                102,
                D22,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &valid_baseline_payload(0),
            ));
            s.extend(frame_end(6, 102));
            let result = scan(&s).expect("the stream itself is well-formed");
            assert_eq!(result.failure_records, 1, "bit {bit}");
            assert_eq!(
                result.delta_before_baseline_records, 1,
                "bit {bit}: a failed baseline is not a baseline"
            );
            assert_eq!(result.hours.len(), 1, "bit {bit}");
            assert_eq!(result.hours[0].game_hour, 102, "bit {bit}");
            assert_eq!(result.shipping_baselines, 1, "bit {bit}");
            assert_eq!(result.shipping_deltas, 0, "bit {bit}");
        }
        // And the DELTA bit (1<<10) is NOT a failure bit.
        let result = scan(&good_stream()).expect("well-formed");
        assert_eq!(result.failure_records, 0);
        assert_eq!(result.hours.len(), 2);
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        // A cut immediately after a d22 DATA record leaves that hour open and ends
        // the stream on a record that is not a FRAME_END. That would otherwise
        // produce a plausible shorter number. Records 1 and 3 are the DATA records.
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan(&full[..cut]).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary IS a well-formed shorter stream,
        // and `eof_check` accepts it. Refusing it would be wrong; what matters is
        // that it reports the hours it actually closed.
        for (records, expected_hours) in [(2usize, 1usize), (4, 2)] {
            let cut = boundary(&full, records);
            let s = scan(&full[..cut]).expect("closed stream is well-formed");
            assert_eq!(s.hours.len(), expected_hours, "cut after {records}");
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
            D22,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &valid_baseline_payload(0),
        ));
        s.extend(frame_end(2, 999));
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
            D22,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &valid_baseline_payload(0),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D22,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d22_payload(0, &[state_row(1, 8, 8, 3)], &[]),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan(&s).is_err());
    }

    #[test]
    fn a_second_d22_record_in_one_hour_is_rejected() {
        // d22 is incremental, so two records in one hour would apply two shipping
        // frames to one measurement hour. The rule is checked over EVERY d22 DATA
        // record and before status-based exclusion, so all four success/failure
        // orderings are refused.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D22,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &valid_baseline_payload(first),
            ));
            s.extend(record(
                2,
                100,
                D22,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d22_payload(second, &[state_row(1, 8, 8, 3)], &[]),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan(&s).is_err(),
                "two d22 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_stream_with_no_d22_records_is_refused_not_reported_as_zero() {
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
    fn a_malformed_d22_payload_is_refused_not_skipped() {
        // Short of the header, and every length that does not match the header's
        // two counts. A structurally sized payload whose rows do not add up is a
        // corrupt stream, not an hour to drop quietly.
        let base = d22_payload(0, &[state_row(1, 7, 7, 3)], &[province_row(5, 7, 1, 3)]);
        let mut bad: Vec<Vec<u8>> = Vec::new();
        for len in [0usize, 8, 31] {
            bad.push(base[..len].to_vec());
        }
        let mut short = base.clone();
        short.pop();
        bad.push(short);
        let mut long = base.clone();
        long.push(0);
        bad.push(long);
        // Counts whose byte length overflows `usize`, and counts far too large.
        for offset in [8usize, 20] {
            let mut huge = base.clone();
            huge[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            bad.push(huge);
            let mut big = base.clone();
            big[offset..offset + 4].copy_from_slice(&1000u32.to_le_bytes());
            bad.push(big);
        }
        // And a failure-status record with a malformed length: the fold checks the
        // length first (`world_fold.rs:22156` before `:22164`), so this is refused
        // rather than silently excluded.
        let mut failed = d22_payload(1, &[state_row(1, 7, 7, 3)], &[]);
        failed.pop();
        bad.push(failed);
        for (i, payload) in bad.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D22,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            assert!(
                scan(&s).is_err(),
                "malformed payload {i} must refuse the stream"
            );
        }
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        let data = |sequence, hour| {
            record(
                sequence,
                hour,
                D22,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &valid_baseline_payload(0),
            )
        };
        let mut ok = Vec::new();
        ok.extend(data(1, 100));
        ok.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        assert!(scan(&ok).is_ok());

        let mut bad_seals = vec![
            frame_end_sealed(2, 100, 0, 0, 0, 1),
            frame_end_sealed(2, 100, 0, 1, 0, 0),
        ];
        for bit in 15..32u32 {
            bad_seals.push(frame_end_sealed(2, 100, 0, 0, 1 << bit, 0));
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            bad_seals.push(frame_end_sealed(2, 100, 0, 0, 1 << bit, 0));
        }
        for sealed in bad_seals {
            let mut s = Vec::new();
            s.extend(data(1, 100));
            s.extend(sealed);
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
        let last = boundary(&later, 3);
        later[last + COMPACT_RECORD_HEADER_LEN + 28..last + COMPACT_RECORD_HEADER_LEN + 32]
            .copy_from_slice(&1u32.to_le_bytes());
        assert!(
            scan(&later).is_err(),
            "a later bad seal refuses the whole stream"
        );
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        let data = |sequence, hour| {
            record(
                sequence,
                hour,
                D22,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &valid_baseline_payload(0),
            )
        };
        let mut ok = Vec::new();
        ok.extend(perf(1, 100, 20));
        ok.extend(perf(2, 100, D22));
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
            s.extend(perf(2, 100, D22));
            s.extend(data(3, 100));
            s.extend(frame_end_sealed(4, 100, emitted, scheduled, 0, 0));
            assert!(
                scan(&s).is_err(),
                "{emitted}/{scheduled} against two PERF domains"
            );
        }

        let mut two_hours = Vec::new();
        two_hours.extend(perf(1, 100, 20));
        two_hours.extend(data(2, 100));
        two_hours.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        two_hours.extend(perf(4, 101, 20));
        two_hours.extend(data(5, 101));
        two_hours.extend(frame_end_sealed(6, 101, 1, 1, 0, 0));
        assert_eq!(scan(&two_hours).expect("PERF state resets").hours.len(), 2);
    }

    #[test]
    fn a_data_record_must_open_the_hour_the_pending_perf_records_bound() {
        let data = |sequence, hour| {
            record(
                sequence,
                hour,
                D22,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &valid_baseline_payload(0),
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
                D22,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D22,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &valid_baseline_payload(0),
            ));
            s.extend(frame_end(4, 101));
            s
        };

        let mut bad_payloads = Vec::new();
        bad_payloads.push(d22_payload(1, &[state_row(1, 7, 7, 3)], &[])); // baseline minima
        let mut bad_changed = d22_payload(
            FAILED_DELTA,
            &[state_row(1, 7, 7, 3)],
            &[province_row(1, 7, 1, 3)],
        );
        bad_changed[12..16].copy_from_slice(&1u32.to_le_bytes());
        bad_payloads.push(bad_changed);
        bad_payloads.push(d22_payload(FAILED_DELTA, &[state_row(0, 7, 7, 3)], &[]));
        bad_payloads.push(d22_payload(FAILED_DELTA, &[state_row(1, 1_025, 7, 3)], &[]));
        bad_payloads.push(d22_payload(
            FAILED_DELTA,
            &[state_row(1, 7, 7, 1 << 8)],
            &[],
        ));
        bad_payloads.push(d22_payload(
            FAILED_DELTA,
            &[state_row(1, 7, 7, d22::FLAG_STATE_CONTROLLER)],
            &[],
        ));
        bad_payloads.push(d22_payload(
            FAILED_DELTA,
            &[],
            &[province_row(0, 7, 0, d22::FLAG_PROVINCE_CONTROLLER)],
        ));
        bad_payloads.push(d22_payload(
            FAILED_DELTA,
            &[],
            &[province_row(1, 1_025, 0, d22::FLAG_PROVINCE_CONTROLLER)],
        ));
        bad_payloads.push(d22_payload(
            FAILED_DELTA,
            &[],
            &[province_row(
                1,
                7,
                0,
                d22::FLAG_PROVINCE_CONTROLLER | 1 << 8,
            )],
        ));
        bad_payloads.push(d22_payload(
            FAILED_DELTA,
            &[],
            &[province_row(1, 7, 1, d22::FLAG_PROVINCE_CONTROLLER)],
        ));
        bad_payloads.push(d22_payload(
            FAILED_DELTA,
            &[state_row(1, 7, 7, 3), state_row(1, 8, 8, 3)],
            &[],
        ));
        bad_payloads.push(d22_payload(
            FAILED_DELTA,
            &[],
            &[
                province_row(1, 7, 0, d22::FLAG_PROVINCE_CONTROLLER),
                province_row(1, 8, 0, d22::FLAG_PROVINCE_CONTROLLER),
            ],
        ));
        let mut unknown_state = valid_baseline_payload(1);
        let first_province = d22::HEADER_LEN + 1_000 * d22::STATE_ROW_LEN;
        unknown_state[first_province + 8..first_province + 12]
            .copy_from_slice(&2_000u32.to_le_bytes());
        bad_payloads.push(unknown_state);

        for (index, payload) in bad_payloads.into_iter().enumerate() {
            assert!(
                scan(&follow_with_baseline(payload)).is_err(),
                "excluded malformed authority payload {index}"
            );
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let payload = d22_payload(
                (1 << bit) | COMPACT_STATUS_DELTA_DATA,
                &[state_row(0, 7, 7, 3)],
                &[],
            );
            assert!(
                scan(&follow_with_baseline(payload)).is_err(),
                "failure bit {bit}"
            );
        }

        let well_formed = d22_payload(
            FAILED_DELTA,
            &[state_row(1, 7, 7, 3)],
            &[province_row(1, 7, 1, 3)],
        );
        let scanned = scan(&follow_with_baseline(well_formed))
            .expect("a well-formed excluded delta is outside the fold-only baseline rule");
        assert_eq!(scanned.failure_records, 1);
        assert_eq!(scanned.delta_before_baseline_records, 0);
        assert_eq!(scanned.hours.len(), 1);
    }

    #[test]
    fn the_identity_protocol_is_stated_not_sniffed() {
        use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;

        let (states, provinces) = collections();
        let legacy = good_stream();
        assert!(scan_stream(&legacy, &states, &provinces, StreamIdentityMode::IdentityV1).is_err());
        assert!(scan_stream(&legacy, &states, &provinces, StreamIdentityMode::Legacy).is_ok());

        let mut identity = record(
            0,
            0,
            0,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            b"CID1-not-a-real-identity-payload",
        );
        identity.extend(legacy);
        assert!(scan_stream(&identity, &states, &provinces, StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn scan_stream_never_panics_on_arbitrary_bytes() {
        // Total over hostile input: the harness reads a file from disk and must
        // return a diagnosis, never unwind.
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
            let _ = scan(&bytes);
        }
        // And corruption of a genuinely well-formed stream, byte by byte.
        let good = good_stream();
        for i in (0..good.len()).step_by(257) {
            let mut s = good.clone();
            s[i] ^= 0xFF;
            let _ = scan(&s);
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
    fn an_hour_closed_without_a_d22_record_refuses_the_stream() {
        // The completeness bound on what this harness does NOT validate (module
        // docs): it cannot check other domains' payloads, so a d22 record
        // relabelled to another domain id — or dropped — must be caught by
        // counting, not silently measured as fewer hours.
        let mut s = good_stream();
        // Find the LAST d22 DATA record and relabel it to an unclaimed domain
        // id. The stream stays well formed — sequence, hour boundaries and
        // FRAME_END are untouched — so the completeness rule is the only rule
        // that can catch it.
        let mut pos = 0usize;
        let mut target = None;
        while pos < s.len() {
            let header = parse_compact_record_header(&s[pos..]).expect("well-formed");
            if header.domain_id == D22 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
            {
                target = Some(pos);
            }
            pos += COMPACT_RECORD_HEADER_LEN + header.payload_len;
        }
        let target = target.expect("good_stream carries a d22 DATA record");
        // domain_id sits at +20: magic(4) + version(2) + header_len(2) +
        // sequence(8) + game_hour(4).
        s[target + 20..target + 22].copy_from_slice(&99u16.to_le_bytes());
        let err = match scan(&s) {
            Err(e) => e,
            Ok(_) => panic!("an hour with no d22 record must refuse the stream"),
        };
        assert!(
            err.contains("game hour(s) but carries"),
            "the completeness check must be what rejects it: {err}"
        );
    }
}
