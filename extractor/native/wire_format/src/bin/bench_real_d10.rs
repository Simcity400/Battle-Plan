//! Measure the wire format against the shipping compact stream on REAL d10
//! `fuel_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d10-scheduled capture
//! (`runs/batch1/deepwar_t1`). The stream's own d10 DATA records are the shipping
//! format's cost for the domain: a 24-byte header plus a fixed 40-byte row per
//! country, exactly what the recorder stores today. This tool replays those
//! records into per-hour country states — the same fold
//! `world_fold::fold_fuel` performs — and re-encodes every hour with this crate's
//! grammar: a BASELINE at each 48-hour block start, field-granular stored-minimal
//! DELTAs between.
//!
//! **d10's payload is an incremental update, so the measured state is the
//! ACCUMULATED one.** The collector emits a row only for a country whose fuel
//! fingerprint changed since the last frame
//! (`collector/src/attach_log.rs:79595`-`:79602`, gating the read at `:79496`),
//! and `fold_fuel` upserts each row into the country object without ever clearing
//! the countries a frame omits (`world_fold.rs:9220`-`:9224`) — unlike the
//! full-replace domains, which wipe their per-country subtree first. A per-hour
//! state built from one payload alone would therefore delete every country that
//! hour did not mention, and would measure a domain nobody ships. Each hour's
//! state here is the previous hour's state with the hour's rows upserted, which
//! is `fold_fuel`'s own accumulator, and the reader replay asserts equality
//! against exactly that. One consequence is worth naming: the accumulated key set
//! only grows, so a d10 delta never carries a tombstone.
//!
//! **Nothing here re-derives a contract it can call.** Row and payload decoding
//! go through `wire_format::d10`'s public boundary; H4CR framing, record
//! semantics and sequence continuity go through `compact_contract`'s own parser
//! and stateful validator. A harness that reimplements either drifts from the
//! recorder without anything failing.
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
//! - that tracker's DOMAIN_PERF hour binding (`:328`-`:365`) and the DATA arm's
//!   pending-PERF rule (`:378`-`:386`);
//! - the FRAME_END seal fields (`stream_fold.rs:46`-`:89`) and the seal's
//!   domain-count cross-check against distinct non-probe PERF domains
//!   (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END.
//!
//! On top of that it enforces d10's own two stream-level rules: exactly one d10
//! DATA record per game hour, checked over records of every status; and no
//! failure-status d10 record that carries rows, because `fold_fuel` would apply
//! such a record while the exclusion rule would drop it — see `scan_stream`.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority. Production's `validate_domain_data_payload_at`
//! (`stream_fold.rs:594`) also applies `decode_compact_fuel_data_payload`
//! (`domain_data_authority.rs:932`-`:1011`) to every d10 DATA record whatever its
//! status. `d10::split_payload`, already before the exclusion, covers its header,
//! exact-length, scale, changed-row, and duplicate-key predicates. The remaining
//! row predicates (zero key, unknown flags, negative current/max) are replayed by
//! [`check_authority_payload`] before classification. The authority deliberately
//! does not bound positive current/max or gain/cost; those mapper/collector rules
//! remain on folded records. d10's pre-existing status policy is stricter in a
//! different dimension: every row-bearing failure record is refused because
//! `fold_fuel` would apply it, so no excluded row can demonstrate a fold-only row
//! rule being accepted.
//!
//! **Only d10 DATA payloads are validated.** Production runs
//! `validate_domain_data_payload_at` on the DATA payload of EVERY domain
//! (`stream_fold.rs:593`); this harness cannot, because validating another
//! domain's payload means owning that domain's authority, and `wire_format`
//! must not depend on `recorder_host`. Every DATA record of any other domain
//! therefore passes with its H4CR header, sequence continuity, hour-boundary,
//! PERF-binding and FRAME_END rules checked but its **payload bytes unread**.
//! The concrete hazard — a d10 record relabelled to another domain id, or
//! dropped outright, silently shrinking the measurement — is closed by an
//! explicit completeness check in `scan_stream`: every closed game hour must
//! carry exactly one d10 DATA record, of any status. That is a per-capture
//! assertion, not a grammar rule (a domain scheduled at a lower cadence would
//! legitimately emit fewer records); a capture where d10 is not scheduled every
//! hour fails loudly there and a human adjusts, instead of a number quietly
//! shrinking.
//!
//! The seal fields, PERF binding, pending-PERF rule, status-independent payload
//! authority, and stated identity mode are the five-check sweep. Production
//! receives that mode from its caller (`stream_fold.rs:220`); the capture is
//! legacy, so `main` defaults to `legacy`.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d10.bin   — the d10 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface (changed rows per hour at stored
//! precision).

use std::collections::{BTreeMap, BTreeSet};
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
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, Key, KeyedState,
};
use wire_format::d10;
use wire_format::Record;

const D10: u16 = 10;
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

/// One usable d10 hour: its game hour, the rows the hour's payload carried, and
/// its byte span in the source stream (the shipping arm is those bytes verbatim,
/// header included).
///
/// The rows are decoded during the scan, not later: every way a row can be
/// rejected — a zero or duplicated `tag_db_id`, a row flags word no encoder
/// writes, an out-of-range measure — is a property of the stream, so it belongs
/// to the pass that either accepts the stream or refuses it. Deferring it would
/// leave the harness able to accept a stream and then unwind while measuring it.
///
/// These are the hour's *updates*, not the hour's state: d10 is incremental, and
/// the state is accumulated in the measuring pass (module docs).
struct D10Hour {
    game_hour: i32,
    rows: Vec<(Key, Record)>,
    ship_span: Range<usize>,
}

struct StreamScan {
    hours: Vec<D10Hour>,
    /// d10 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// Measured d10 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
}

// Re-exported, not re-declared: a local `(1 << 14) - 1` became wrong the moment
// the contract assigned bit 14 to the measured-empty badge.
const FRAME_END_KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;
const PROBE_DOMAIN_ID: u16 = 250;

/// `recorder_host::stream_fold::validate_frame_end_seal_fields`
/// (`stream_fold.rs:46`-`:89`), kept local because `wire_format` cannot depend on
/// `recorder_host`.
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

/// The predicates in `decode_compact_fuel_data_payload`
/// (`domain_data_authority.rs:932`-`:1011`) that `d10::split_payload` has not
/// already run. This intentionally does not call `record_from_row`: that mapper
/// also enforces collector magnitude bounds which this authority does not.
fn check_authority_payload(body: &[u8], at_byte: usize) -> Result<(), String> {
    for (row_index, row) in body.chunks_exact(d10::ROW_LEN).enumerate() {
        d10::key_from_row(row)
            .map_err(|e| format!("d10 authority row {row_index} at byte {at_byte}: {e}"))?;
        let flags = u32::from_le_bytes(row[4..8].try_into().expect("4 bytes"));
        if flags & !d10::ROW_KNOWN_FLAGS != 0 {
            return Err(format!(
                "d10 authority row {row_index} at byte {at_byte}: unknown flags {flags:#x}"
            ));
        }
        let current = i64::from_le_bytes(row[8..16].try_into().expect("8 bytes"));
        let max = i64::from_le_bytes(row[16..24].try_into().expect("8 bytes"));
        if current < 0 {
            return Err(format!(
                "d10 authority row {row_index} at byte {at_byte}: current_raw is negative"
            ));
        }
        if max < 0 {
            return Err(format!(
                "d10 authority row {row_index} at byte {at_byte}: max_raw is negative"
            ));
        }
    }
    Ok(())
}

/// Validate the whole stream against the canonical contract and decode the usable
/// d10 DATA records. Every rejection path is an `Err`, never a silently shorter
/// measurement and never a panic — see the module docs for what is enforced and
/// what the recorder's own validator additionally checks.
fn scan_stream(
    stream: &[u8],
    collection: &wire_format::Collection,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    // Production takes identity mode from its caller (`stream_fold.rs:220`);
    // sniffing cannot distinguish legacy from IdentityV1 with CID1 removed.
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours: Vec<D10Hour> = Vec::new();
    let mut failure_records = 0usize;
    let mut non_contiguous = 0usize;

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d10 DATA record of ANY status — the uniqueness rule.
    let mut last_d10_hour: Option<i32> = None;
    // Closed game hours and d10 DATA records of ANY status — the completeness
    // rule at the end of the scan.
    let mut closed_hours = 0usize;
    let mut d10_records = 0usize;
    // Hour of the last d10 record that entered the measurement — the contiguity
    // report, which a skipped failure hour legitimately breaks.
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
                let observed_perf_domains =
                    non_probe_perf_domains.len().min(u32::MAX as usize) as u32;
                if observed_perf_domains != emitted_domains
                    || observed_perf_domains != scheduled_domains
                {
                    return Err(format!(
                        "FRAME_END seal violation at byte {pos}: seal reports \
                         emitted={emitted_domains}, scheduled={scheduled_domains}, but \
                         {observed_perf_domains} distinct non-probe DOMAIN_PERF domain(s) \
                         were observed"
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
                closed_hours += 1;
                pending_perf_hour = None;
                non_probe_perf_domains.clear();
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        if header.domain_id == D10 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d10_records += 1;
            let (d10_header, body) = d10::split_payload(payload)
                .map_err(|e| format!("d10 payload at byte {pos}: {e}"))?;

            // One d10 DATA record per game hour, checked over EVERY d10 record and
            // BEFORE any status-based exclusion. d10 is incremental, so a second
            // record for the same hour would apply two updates under one clock
            // time — and gating this on the record being usable would let a
            // success/failure pair through the rule entirely.
            if let Some(previous) = last_d10_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d10 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d10_hour = Some(header.game_hour_raw);

            // VALIDITY BEFORE CLASSIFICATION. Everything below that reads the
            // status word classifies the v1 vocabulary; none of it is a validity
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
            // measured as healthy. The production lane rejects first
            // (`lanes/division_state_movement_live/domain20_compact.rs:419`);
            // this harness does the same rather than re-deriving a lenient rule.
            // No capture carries an unassigned bit, so no measured number moves.
            let unknown_status = compact_status_unknown_flags_v1(d10_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d10 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            check_authority_payload(body, pos)?;

            // Failure-status records are excluded from both arms, with
            // `compact_contract`'s own predicate rather than a copied bit mask so
            // it cannot drift from the recorder's lane-local masks or partial
            // tolerances.
            //
            // d10 makes that rule sharper than it is elsewhere. `fold_fuel`
            // (`world_fold.rs:9203`) never reads `status_flags` and has no
            // `handle_compact_failure` call, so unlike d63 (`:9252`) it does NOT
            // retain the accumulator — it folds a failure-masked payload as
            // authoritative. Excluding such a record is therefore equivalent to
            // the fold only when the record carries nothing to apply. A
            // failure-status record with rows is refused outright rather than
            // measured: dropping it would leave the measured state permanently
            // behind the fold's, and folding it would contradict the status word
            // the recorder itself publishes. Either way the number would be
            // reported for a state nobody holds, which is exactly what this pass
            // exists to prevent. A zero-row failure record applies nothing under
            // either reading, so it is counted and skipped.
            //
            // This also closes the second-order hole: a failure-status payload's
            // rows would otherwise never be validated at all, so a corrupt row
            // could hide behind a failure status and still leave the stream
            // measurable. With row-bearing failure records refused, every row in
            // an accepted stream has been through the mapper.
            //
            // The reference capture has 0 failure-status records, so none of this
            // affects its numbers.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D10,
                d10_header.status_flags,
            ) != 0
            {
                if d10_header.row_count != 0 {
                    return Err(format!(
                        "d10 DATA at byte {pos}: hour {} has failure status {:#x} and carries \
                         {} row(s); `fold_fuel` would apply them, so the hour can be neither \
                         measured nor skipped without diverging from the fold",
                        header.game_hour_raw, d10_header.status_flags, d10_header.row_count
                    ));
                }
                failure_records += 1;
            } else {
                if let Some(previous) = last_measured_hour {
                    if header.game_hour_raw != previous + 1 {
                        non_contiguous += 1;
                    }
                }
                last_measured_hour = Some(header.game_hour_raw);

                // Decode exactly what `fold_fuel` reads. Key uniqueness inside
                // the payload is already `d10::split_payload`'s
                // (`domain_data_authority.rs:984`): the fold's upsert would
                // silently take the last writer (`world_fold.rs:9220`) and a
                // keyed state cannot hold both rows, so the frame is refused
                // there rather than half-measured here. Re-deriving that check in
                // the harness is exactly the drift this crate's boundary exists
                // to prevent.
                let mut rows: Vec<(Key, Record)> = Vec::new();
                for row in body.chunks_exact(d10::ROW_LEN) {
                    let key = d10::key_from_row(row)
                        .map_err(|e| format!("d10 row at byte {pos}: {e}"))?;
                    let record = d10::record_from_row(collection, row, 0)
                        .map_err(|e| format!("d10 row at byte {pos}: {e}"))?;
                    rows.push((key, record));
                }
                if rows.len() != d10_header.row_count as usize {
                    return Err(format!("d10 payload at byte {pos}: row count mismatch"));
                }
                hours.push(D10Hour {
                    game_hour: header.game_hour_raw,
                    rows,
                    ship_span: pos..record_end,
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
        return Err("stream carries no usable d10 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d10 DATA
    // record, of any status. Production validates the payload of EVERY domain's
    // DATA record (`stream_fold.rs:593`); this harness cannot (module docs), so
    // a d10 record relabelled to another domain id — or lost outright — would
    // otherwise be silently ignored and the run would report a plausible
    // measurement over fewer hours. Per-hour uniqueness is already enforced
    // above (strictly increasing d10 hours), so equality of the two counts is
    // exactly-one per closed hour. A per-capture assertion, not a grammar rule:
    // a capture that does not schedule d10 every hour fails here loudly and a
    // human adjusts, instead of a number quietly shrinking.
    if d10_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d10_records} d10 DATA \
             record(s); every closed hour of a d10-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    Ok(StreamScan {
        hours,
        failure_records,
        non_contiguous,
    })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args
        .next()
        .expect("usage: bench_real_d10 <collector_stream.bin> <out dir> [legacy|identity-v1]");
    let out_dir = args
        .next()
        .expect("usage: bench_real_d10 <collector_stream.bin> <out dir> [legacy|identity-v1]");
    let identity_mode = match args.next().as_deref().unwrap_or("legacy") {
        "legacy" => StreamIdentityMode::Legacy,
        "identity-v1" => StreamIdentityMode::IdentityV1,
        other => panic!("unknown identity mode {other:?}; expected legacy or identity-v1"),
    };
    assert!(args.next().is_none(), "too many arguments");
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d10::schema();
    let collection = &schema.collections[0];

    // ---- Pass 1: validate the WHOLE stream and decode its d10 DATA records. ---
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result. Everything that can
    // fail on stream content fails here, with a diagnosis and no unwind.
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

    // The fold's accumulator (`world_fold.rs:9220`-`:9224`): countries persist
    // until a later frame overwrites them.
    let mut prior: Option<KeyedState> = None;
    let mut reader: Option<KeyedState> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    // Baselines the §1.5 gap rule forced outside a scheduled block start.
    let mut gap_forced_baselines = 0usize;
    let mut previous_measured_hour: Option<i32> = None;
    let mut empty_hours = 0usize;
    let mut changed_rows_total = 0usize;
    let mut tombstones_total = 0usize;
    let mut countries_total = 0usize;
    let mut payload_rows_total = 0usize;

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());

        // The hour's state: the previous state with this hour's rows upserted.
        payload_rows_total += hour.rows.len();
        let mut state: KeyedState = prior.clone().unwrap_or_else(BTreeMap::new);
        for (key, record) in &hour.rows {
            state.insert(key.clone(), record.clone());
        }
        countries_total += state.len();

        // New arm: BASELINE at each block start or across a gap in measured
        // hours (see `opens_baseline`), stored-minimal DELTA between.
        let hour_start = wire.len();
        let baseline_here = opens_baseline(hours_seen, previous_measured_hour, hour.game_hour);
        if baseline_here && hours_seen % BLOCK_HOURS != 0 {
            gap_forced_baselines += 1;
        }
        let mut emitted = true;
        if baseline_here || prior.is_none() {
            encode_baseline(&schema, std::slice::from_ref(&state), &mut wire)
                .expect("baseline encodes");
            baselines += 1;
        } else {
            let prev = prior.as_ref().expect("a non-block-start hour has a prior");
            let d = diff(collection, prev, &state);
            if d.is_empty() {
                empty_hours += 1;
                emitted = false;
            } else {
                changed_rows_total += d.changed.len();
                tombstones_total += d.tombstones.len();
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
        let residual = diff(collection, replayed, &state);
        assert!(
            residual.is_empty(),
            "game hour {}: reader state is not stored-equal to the fold",
            hour.game_hour
        );

        prior = Some(state);
        previous_measured_hour = Some(hour.game_hour);
        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d10.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d10.offsets", &ship_offsets),
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
        "payload rows/hour    {:.2} (the shipping incremental update)",
        payload_rows_total as f64 / hours_seen as f64
    );
    println!(
        "countries avg        {:.1} (accumulated state)",
        countries_total as f64 / hours_seen as f64
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
    println!("ship_d10.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.1}%)",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{COMPACT_DOMAIN_FRAME, COMPACT_FRAME_END_PAYLOAD_LEN};

    fn test_collection() -> wire_format::Collection {
        d10::schema().collections[0].clone()
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
        assert_eq!(payload.len(), COMPACT_FRAME_END_PAYLOAD_LEN);
        record(
            sequence,
            game_hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &payload,
        )
    }

    /// A d10 DATA payload: the 24-byte header then one 40-byte row per entry.
    fn d10_payload(status_flags: u32, rows: &[(u32, i64)]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        payload.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
        payload.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // changed_rows
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&d10::FUEL_SCALE_RAW.to_le_bytes());
        for (tag_db_id, current) in rows {
            payload.extend_from_slice(&tag_db_id.to_le_bytes());
            payload.extend_from_slice(&d10::ROW_FLAG_FUEL_PRESENT.to_le_bytes());
            payload.extend_from_slice(&current.to_le_bytes()); // current
            payload.extend_from_slice(&32_768i64.to_le_bytes()); // max
            payload.extend_from_slice(&0i64.to_le_bytes()); // gain
            payload.extend_from_slice(&0i64.to_le_bytes()); // cost
        }
        assert_eq!(payload.len(), d10::HEADER_LEN + rows.len() * d10::ROW_LEN);
        payload
    }

    /// Two closed hours, each carrying one d10 DATA record.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d10_payload(0, &[(7, 32_768)]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d10_payload(0, &[(7, 65_536)]),
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
    fn a_well_formed_stream_yields_its_d10_hours() {
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
        assert_eq!(scan.hours[0].rows.len(), 1);
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helpers classify
        // the v1 vocabulary and are not validity predicates, so an unassigned bit
        // must be rejected BEFORE classification — otherwise it silently reads as
        // "not a failure" and the hour is measured as healthy.
        for bit in 15..32u32 {
            let status = 1u32 << bit;
            // Alone, combined with a known failure bit, and combined with a known
            // informational bit; with rows and without. The rejection must not
            // depend on any of it.
            for extra in [0u32, 1, 1 << 10] {
                for rows in [Vec::new(), vec![(7u32, 32_768i64)]] {
                    let mut s = Vec::new();
                    s.extend(record(
                        1,
                        100,
                        D10,
                        COMPACT_RECORD_TYPE_DOMAIN_DATA,
                        &d10_payload(status | extra, &rows),
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
        // the check must reject unassigned bits and nothing else. WAR_BRIDGE_APPLIED,
        // EXTRACTION_PERF_GATE, DELTA_DATA, TRANSITION_BASELINE_RESET and
        // WAR_SEGMENT_ABSENT are informational, and `fold_fuel` never reads the
        // status word at all.
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D10,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d10_payload(informational, &[(7, 32_768)]),
            ));
            s.extend(frame_end(2, 100));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 1, "bit {informational:#x}");
            assert_eq!(scan.failure_records, 0, "bit {informational:#x}");
        }
    }

    #[test]
    fn a_zero_row_hour_is_an_hour_that_updates_nothing() {
        // The reference capture has exactly one such hour, immediately after the
        // opening full frame: d10 is incremental, so "no country's fuel moved"
        // is a legal payload with no rows — not a missing hour and not an empty
        // state. The accumulated state must survive it unchanged.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d10_payload(0, &[(7, 32_768)]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d10_payload(0, &[]),
        ));
        s.extend(frame_end(4, 101));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert!(scan.hours[1].rows.is_empty());
    }

    #[test]
    fn the_accumulated_state_is_the_folds_state_not_one_payload() {
        // The property the whole measurement rests on: a country the hour's
        // payload does not mention keeps its previous fuel, because `fold_fuel`
        // never clears a missing country (`world_fold.rs:9220`-`:9224`). Building
        // the state from one payload would drop country 7 in hour 101 and report
        // a tombstone the recorder never has.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d10_payload(0, &[(7, 32_768), (9, 65_536)]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d10_payload(0, &[(9, 98_304)]),
        ));
        s.extend(frame_end(4, 101));
        let collection = test_collection();
        let scan = scan_stream(&s, &collection, StreamIdentityMode::Legacy).expect("accepted");

        let mut state: KeyedState = BTreeMap::new();
        for hour in &scan.hours {
            for (key, record) in &hour.rows {
                state.insert(key.clone(), record.clone());
            }
        }
        assert_eq!(state.len(), 2, "country 7 survives an hour that omits it");
        let seven = wire_format::body::key(&[7]);
        let index = collection
            .value_fields()
            .position(|f| f.field_id == d10::field::CURRENT)
            .expect("current is a value field");
        assert_eq!(
            state[&seven].values[index],
            wire_format::Value::Continuous(1.0),
            "the retained value is the one hour 100 carried"
        );
        // And the hour that omitted it produced no tombstone for it.
        let mut prior: KeyedState = BTreeMap::new();
        for (key, record) in &scan.hours[0].rows {
            prior.insert(key.clone(), record.clone());
        }
        let mut next = prior.clone();
        for (key, record) in &scan.hours[1].rows {
            next.insert(key.clone(), record.clone());
        }
        let d = diff(&collection, &prior, &next);
        assert!(
            d.tombstones.is_empty(),
            "an accumulated d10 state never shrinks"
        );
        assert_eq!(
            d.changed.len(),
            1,
            "only the country the hour carried moved"
        );
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        // A cut immediately after a d10 DATA record leaves that hour open and ends
        // the stream on a record that is not a FRAME_END. That would produce a
        // plausible shorter number; it must be refused. Records 1 and 3 of
        // `good_stream` are the DATA records.
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&full[..cut], &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary is a different thing: that IS a
        // well-formed shorter stream, and `eof_check` accepts it too. Refusing it
        // would be wrong. What matters is that it reports the hours it actually
        // closed, not the hours the untruncated file had.
        for (records, expected_hours) in [(2usize, 1usize), (4, 2)] {
            let cut = boundary(&full, records);
            let scan = scan_stream(&full[..cut], &test_collection(), StreamIdentityMode::Legacy)
                .expect("closed stream is well-formed");
            assert_eq!(scan.hours.len(), expected_hours, "cut after {records}");
        }
    }

    #[test]
    fn a_stream_ending_mid_record_is_rejected() {
        let full = good_stream();
        // Every cut that is not a record boundary, across the whole stream.
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
        // Version, magic and header length, at every record in the stream — the
        // canonical parser owns these and the harness must not read past it.
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
        // A swallowed or reordered record changes which hours the measurement
        // sees, so the sequence chain is load-bearing, not decoration.
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
            D10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d10_payload(0, &[(7, 32_768)]),
        ));
        s.extend(frame_end(2, 999)); // closes an hour that is not open
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());

        // A FRAME_END with nothing open at all.
        let mut s = Vec::new();
        s.extend(frame_end(1, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        // A closed hour that repeats or goes backwards would apply two updates
        // under one clock time.
        let mut s = Vec::new();
        s.extend(record(
            1,
            101,
            D10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d10_payload(0, &[(7, 32_768)]),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d10_payload(0, &[(7, 65_536)]),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_second_d10_record_in_one_hour_is_rejected() {
        // The rule is checked over EVERY d10 DATA record and before status-based
        // exclusion, so all four success/failure orderings are refused — gating
        // it on the record being usable would let a success + failure pair
        // through.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            // The failure-status records carry no rows, so the ONLY thing that
            // can refuse these streams is the one-record-per-hour rule — a
            // row-bearing failure record would be refused for its own reason and
            // would not test this one.
            let rows_first: &[(u32, i64)] = if first == 0 { &[(7, 32_768)] } else { &[] };
            let rows_second: &[(u32, i64)] = if second == 0 { &[(7, 65_536)] } else { &[] };
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D10,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d10_payload(first, rows_first),
            ));
            s.extend(record(
                2,
                100,
                D10,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d10_payload(second, rows_second),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "two d10 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn an_empty_failure_status_record_is_excluded_rather_than_measured() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time. A
        // failure record with no rows applies nothing under either reading — the
        // fold's (which ignores the status word) or the exclusion rule's — so the
        // hour is counted and skipped and the next hour still measures.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D10,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d10_payload(1 << bit, &[]),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D10,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d10_payload(0, &[(7, 65_536)]),
            ));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .expect("the stream itself is well-formed");
            assert_eq!(scan.failure_records, 1, "bit {bit}");
            assert_eq!(scan.hours.len(), 1, "bit {bit}");
            assert_eq!(scan.hours[0].game_hour, 101, "bit {bit}");
        }
        // And a non-failure bit (DELTA_DATA, 1<<10) is NOT treated as failure:
        // its rows are measured like any other hour's.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d10_payload(1 << 10, &[(7, 32_768)]),
        ));
        s.extend(frame_end(2, 100));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn a_failure_status_record_that_carries_rows_is_refused() {
        // Round-1 review input, generalized over every failure bit. `fold_fuel`
        // has no `handle_compact_failure` call, so it APPLIES a failure-status
        // payload; skipping the hour would leave the measured state permanently
        // behind the fold's, and the number would describe a state nobody holds.
        // The stream is refused instead — including the second-order case where a
        // corrupt row hides behind a failure status and would otherwise never
        // reach the mapper at all.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            for rows in [
                vec![(7u32, 32_768i64)],
                vec![(0, 32_768)], // a row the fold itself rejects
                vec![(7, 32_768), (9, 65_536)],
            ] {
                let mut s = Vec::new();
                s.extend(record(
                    1,
                    100,
                    D10,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d10_payload(1 << bit, &rows),
                ));
                s.extend(frame_end(2, 100));
                s.extend(record(
                    3,
                    101,
                    D10,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d10_payload(0, &[]),
                ));
                s.extend(frame_end(4, 101));
                assert!(
                    scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                    "failure bit {bit} with {} row(s) must refuse the stream",
                    rows.len()
                );
            }
        }
    }

    #[test]
    fn a_stream_with_no_d10_records_is_refused_not_reported_as_zero() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &[0u8; 8],
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_malformed_d10_payload_is_refused_not_skipped() {
        // A short or inconsistent d10 payload is a corrupt stream, not an hour to
        // drop quietly.
        let mut payload = d10_payload(0, &[(7, 32_768)]);
        payload.truncate(payload.len() - 1);
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());

        // A row the mapper rejects must make the SCAN fail, not be accepted and
        // then unwind while measuring. The whole row-rejection space is covered
        // here, not one sampled case.
        let mut invalid: Vec<Vec<u8>> = Vec::new();
        // zero tag_db_id (`world_fold.rs:9213`)
        invalid.push(d10_payload(0, &[(0, 32_768)]));
        // a wrong header scale word (`domain_data_authority.rs:949`)
        let mut wrong_scale = d10_payload(0, &[(7, 32_768)]);
        wrong_scale[20..24].copy_from_slice(&100_000u32.to_le_bytes());
        invalid.push(wrong_scale);
        // changed_rows above row_count (`domain_data_authority.rs:962`)
        let mut split_counts = d10_payload(0, &[(7, 32_768)]);
        split_counts[12..16].copy_from_slice(&2u32.to_le_bytes());
        invalid.push(split_counts);
        // an unknown row flag bit (`domain_data_authority.rs:989`); bit 0 clear
        // is NOT here, because both authorities accept it
        for flags in [2u32, 3, 1 << 31] {
            let mut bad_flags = d10_payload(0, &[(7, 32_768)]);
            bad_flags[d10::HEADER_LEN + 4..d10::HEADER_LEN + 8]
                .copy_from_slice(&flags.to_le_bytes());
            invalid.push(bad_flags);
        }
        // a negative `current` (`domain_data_authority.rs:998`) and an
        // out-of-range magnitude (`attach_log.rs:83493`)
        invalid.push(d10_payload(0, &[(7, -1)]));
        invalid.push(d10_payload(0, &[(7, d10::MAX_FUEL_RAW + 1)]));
        for (i, payload) in invalid.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D10,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "invalid payload {i} must make the scan fail"
            );
        }

        // A tag_db_id repeated within ONE frame: the fold's upsert would take the
        // last writer silently, and a keyed state cannot hold both, so the frame
        // is refused rather than half-measured.
        let mut payload = d10_payload(0, &[(7, 32_768), (7, 65_536)]);
        assert_eq!(payload.len(), d10::HEADER_LEN + 2 * d10::ROW_LEN);
        payload[8..12].copy_from_slice(&2u32.to_le_bytes());
        payload[12..16].copy_from_slice(&2u32.to_le_bytes());
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        assert!(
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
            "duplicate tag_db_id in one frame"
        );
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D10,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d10_payload(0, &[(7, 32_768)]),
            )
        };
        let mut ok = Vec::new();
        ok.extend(data(1, 100));
        ok.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        assert!(scan_stream(&ok, &test_collection(), StreamIdentityMode::Legacy).is_ok());

        let mut bad = vec![
            frame_end_sealed(2, 100, 0, 0, 0, 1),
            frame_end_sealed(2, 100, 0, 1, 0, 0),
            frame_end_sealed(2, 100, 1, 0, 0, 0),
        ];
        for bit in 15..32u32 {
            bad.push(frame_end_sealed(2, 100, 0, 0, 1 << bit, 0));
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            bad.push(frame_end_sealed(2, 100, 0, 0, 1 << bit, 0));
        }
        for seal in bad {
            let mut s = Vec::new();
            s.extend(data(1, 100));
            s.extend(seal);
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
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
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }
        for status in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(data(1, 100));
            s.extend(frame_end_sealed(2, 100, 0, 0, status, 0));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_ok());
        }
        let mut later = Vec::new();
        later.extend(data(1, 100));
        later.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        later.extend(data(3, 101));
        later.extend(frame_end_sealed(4, 101, 0, 0, 0, 1));
        assert!(scan_stream(&later, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D10,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d10_payload(0, &[(7, 32_768)]),
            )
        };
        let mut ok = Vec::new();
        ok.extend(perf(1, 100, 20));
        ok.extend(perf(2, 100, D10));
        ok.extend(perf(3, 100, PROBE_DOMAIN_ID));
        ok.extend(data(4, 100));
        ok.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
        assert!(scan_stream(&ok, &test_collection(), StreamIdentityMode::Legacy).is_ok());

        let mut probes = Vec::new();
        probes.extend(perf(1, 100, PROBE_DOMAIN_ID));
        probes.extend(perf(2, 100, PROBE_DOMAIN_ID));
        probes.extend(data(3, 100));
        probes.extend(frame_end_sealed(4, 100, 0, 0, 0, 0));
        assert!(scan_stream(&probes, &test_collection(), StreamIdentityMode::Legacy).is_ok());

        let mut duplicate = Vec::new();
        duplicate.extend(perf(1, 100, 20));
        duplicate.extend(perf(2, 100, 20));
        duplicate.extend(data(3, 100));
        duplicate.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        assert!(scan_stream(&duplicate, &test_collection(), StreamIdentityMode::Legacy).is_err());

        let mut interleaved = Vec::new();
        interleaved.extend(data(1, 100));
        interleaved.extend(perf(2, 999, 20));
        interleaved.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&interleaved, &test_collection(), StreamIdentityMode::Legacy).is_err());

        let mut stale = Vec::new();
        stale.extend(data(1, 100));
        stale.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        stale.extend(perf(3, 100, 20));
        stale.extend(data(4, 101));
        stale.extend(frame_end_sealed(5, 101, 1, 1, 0, 0));
        assert!(scan_stream(&stale, &test_collection(), StreamIdentityMode::Legacy).is_err());

        let mut conflict = Vec::new();
        conflict.extend(perf(1, 100, 20));
        conflict.extend(perf(2, 101, 21));
        conflict.extend(data(3, 100));
        conflict.extend(frame_end_sealed(4, 100, 2, 2, 0, 0));
        assert!(scan_stream(&conflict, &test_collection(), StreamIdentityMode::Legacy).is_err());

        for (emitted, scheduled) in [(1u32, 1u32), (3, 3), (2, 3), (3, 2)] {
            let mut s = Vec::new();
            s.extend(perf(1, 100, 20));
            s.extend(perf(2, 100, D10));
            s.extend(data(3, 100));
            s.extend(frame_end_sealed(4, 100, emitted, scheduled, 0, 0));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }

        let mut two_hours = Vec::new();
        two_hours.extend(perf(1, 100, 20));
        two_hours.extend(data(2, 100));
        two_hours.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        two_hours.extend(perf(4, 101, 20));
        two_hours.extend(data(5, 101));
        two_hours.extend(frame_end_sealed(6, 101, 1, 1, 0, 0));
        assert_eq!(
            scan_stream(&two_hours, &test_collection(), StreamIdentityMode::Legacy)
                .expect("PERF state resets")
                .hours
                .len(),
            2
        );
    }

    #[test]
    fn a_data_record_must_open_the_hour_the_pending_perf_records_bound() {
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D10,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d10_payload(0, &[(7, 32_768)]),
            )
        };
        for (perf_hour, data_hour) in [(100i32, 101i32), (101, 100), (100, 200), (200, 100)] {
            let mut s = Vec::new();
            s.extend(perf(1, perf_hour, 20));
            s.extend(data(2, data_hour));
            s.extend(frame_end_sealed(3, data_hour, 1, 1, 0, 0));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }
        let mut agreeing = Vec::new();
        agreeing.extend(perf(1, 100, 20));
        agreeing.extend(data(2, 100));
        agreeing.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&agreeing, &test_collection(), StreamIdentityMode::Legacy).is_ok());
    }

    #[test]
    fn an_excluded_record_is_still_checked_by_the_payload_authority() {
        const FAILED: u32 = 1;
        let mut bad = Vec::new();
        bad.push(d10_payload(FAILED, &[(0, 32_768)]));
        let mut flags = d10_payload(FAILED, &[(7, 32_768)]);
        flags[d10::HEADER_LEN + 4..d10::HEADER_LEN + 8].copy_from_slice(&2u32.to_le_bytes());
        bad.push(flags);
        bad.push(d10_payload(FAILED, &[(7, -1)]));
        let mut negative_max = d10_payload(FAILED, &[(7, 32_768)]);
        negative_max[d10::HEADER_LEN + 16..d10::HEADER_LEN + 24]
            .copy_from_slice(&(-1i64).to_le_bytes());
        bad.push(negative_max);
        for payload in bad {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D10,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            let err = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .err()
                .expect("malformed excluded row must refuse");
            assert!(err.contains("authority row"), "unexpected refusal: {err}");
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let payload = d10_payload(1 << bit, &[(0, 32_768)]);
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D10,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            let err = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .err()
                .expect("authority is independent of the failure bit");
            assert!(err.contains("authority row"), "bit {bit}: {err}");
        }

        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d10_payload(FAILED, &[]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d10_payload(0, &[(7, 32_768)]),
        ));
        s.extend(frame_end(4, 101));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 1);

        // Positive magnitude bounds are outside this authority. The independent
        // d10 status policy still refuses every row-bearing failure record, so
        // demonstrate the authority boundary directly rather than claiming that
        // such a record is an accepted excluded stream.
        let outside = d10_payload(FAILED, &[(7, d10::MAX_FUEL_RAW + 1)]);
        check_authority_payload(&outside[d10::HEADER_LEN..], 0)
            .expect("positive upper bounds are not authority predicates");
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

        let mut identity_stream = Vec::new();
        identity_stream.extend(record(
            0,
            0,
            0,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            b"CID1-not-a-real-identity-payload",
        ));
        identity_stream.extend(&good_stream());
        assert!(scan_stream(
            &identity_stream,
            &test_collection(),
            StreamIdentityMode::Legacy
        )
        .is_err());
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
    fn an_hour_closed_without_a_d10_record_refuses_the_stream() {
        // The completeness bound on what this harness does NOT validate (module
        // docs): it cannot check other domains' payloads, so a d10 record
        // relabelled to another domain id — or dropped — must be caught by
        // counting, not silently measured as fewer hours.
        //
        // Relabelled: hour 101's d10 record becomes a d20 record. The stream
        // stays well formed — sequence, hour boundaries and FRAME_END are
        // untouched.
        let mut relabelled = Vec::new();
        relabelled.extend(record(
            1,
            100,
            D10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d10_payload(0, &[(7, 32_768)]),
        ));
        relabelled.extend(frame_end(2, 100));
        relabelled.extend(record(
            3,
            101,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d10_payload(0, &[(7, 65_536)]),
        ));
        relabelled.extend(frame_end(4, 101));
        let err = match scan_stream(&relabelled, &test_collection(), StreamIdentityMode::Legacy) {
            Err(e) => e,
            Ok(_) => panic!("an hour with no d10 record must refuse the stream"),
        };
        assert!(err.contains("closes 2 game hour(s) but carries 1"), "{err}");

        // Dropped from the middle, so the check fires on position as well as on
        // count: hours 100 and 102 carry their d10 record, hour 101 carries only
        // another domain's DATA record.
        let mut dropped = Vec::new();
        dropped.extend(record(
            1,
            100,
            D10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d10_payload(0, &[(7, 32_768)]),
        ));
        dropped.extend(frame_end(2, 100));
        dropped.extend(record(
            3,
            101,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d10_payload(0, &[(7, 65_536)]),
        ));
        dropped.extend(frame_end(4, 101));
        dropped.extend(record(
            5,
            102,
            D10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d10_payload(0, &[(7, 98_304)]),
        ));
        dropped.extend(frame_end(6, 102));
        let err = match scan_stream(&dropped, &test_collection(), StreamIdentityMode::Legacy) {
            Err(e) => e,
            Ok(_) => panic!("an hour carrying no d10 record must refuse the stream"),
        };
        assert!(
            err.contains("closes 3 game hour(s) but carries 2"),
            "the completeness check must be what rejects it, not an earlier rule: {err}"
        );

        // And a failure-status record still counts as the hour's d10 record: the
        // rule is over records of every status, so an excluded hour does not
        // trip it.
        let mut with_failure = Vec::new();
        with_failure.extend(record(
            1,
            100,
            D10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d10_payload(0, &[(7, 32_768)]),
        ));
        with_failure.extend(frame_end(2, 100));
        with_failure.extend(record(
            3,
            101,
            D10,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d10_payload(1 << 0, &[]), // STATE_UNAVAILABLE, a failure bit
        ));
        with_failure.extend(frame_end(4, 101));
        let scan = scan_stream(&with_failure, &test_collection(), StreamIdentityMode::Legacy)
            .expect("a failure-status record is still the hour's d10 record");
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.failure_records, 1);
    }
}
