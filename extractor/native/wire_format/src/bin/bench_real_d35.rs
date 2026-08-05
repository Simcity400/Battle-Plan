//! Measure the wire format against the shipping compact stream on REAL d35
//! `air_variant_stats` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d35-scheduled capture
//! (`runs/batch2/deepwar_t1`). The stream's own d35 DATA records are the shipping
//! format's cost for the domain: a 36-byte header plus a fixed 64-byte row per
//! *changed* variant. This tool replays those records into per-hour variant
//! states through `wire_format::d35::Accumulator` — the same fold
//! `world_fold::fold_air_variant_stats` performs — and re-encodes every hour with
//! this crate's grammar: a BASELINE at each block start, field-granular
//! stored-minimal DELTAs between.
//!
//! **d35 is incremental at the shipping layer, and that is the whole story of
//! the measurement.** The collector emits a row only when that variant's stats
//! changed since its own previous frame (`attach_log.rs:52773`-`:52777`), and the
//! fold never clears, never deletes and never re-baselines. So one record is
//! *not* one state: the hour's content lives in the accumulator, and a hour whose
//! payload carries no rows is a real hour whose state equals the previous one.
//! On the reference capture 145 of 146 records are bare 36-byte headers.
//!
//! Two layers therefore have to be kept apart, exactly as in d22:
//!
//! *Shipping layer*: an unbounded upsert run with no baseline of any kind. The
//! catalog is stated once, at the first hour that carries it, and never again.
//! *Wire layer*: a BASELINE at every `--block-hours` boundary and a
//! stored-minimal DELTA (§7.1) between. The block cadence is the container's
//! open question, not this grammar's, so the tool takes it on the command line
//! and the report carries both shapes — at 48 h the arm pays for four full
//! re-statements of a catalog the shipping arm stated once, and at
//! `--block-hours 0` (one baseline for the whole run) the two arms are compared
//! on grammar alone. Reporting only the first would measure re-baseline policy
//! and call it a grammar result.
//!
//! **Nothing here re-derives a contract it can call.** Row and payload decoding
//! go through `wire_format::d35`'s public boundary; H4CR framing, record
//! semantics and sequence continuity go through `compact_contract`'s own parser
//! and stateful validator. A harness that reimplements either drifts from the
//! recorder without anything failing.
//!
//! **The whole stream is validated before any artifact is written.** A stream
//! that is truncated, spliced, or corrupt would otherwise yield a smaller but
//! entirely plausible measurement. `scan_stream` therefore enforces, fail-closed:
//!
//! - every record header through `compact_contract::parse_compact_record_header`
//!   (magic, version, header length, payload bound);
//! - every record through `compact_contract::stream_validator::StatefulStreamValidator`
//!   (sequence continuity from the stream's first record, record-type and
//!   record-flag semantics, FRAME_END shape);
//! - the hour boundary rules of `recorder_host::stream_fold::BoundaryTracker`
//!   (`recorder_host/src/stream_fold.rs:277`), its DOMAIN_PERF hour binding
//!   (`:328`-`:365`), and the DATA arm's pending-PERF rule (`:378`-`:386`);
//! - FRAME_END seal validation (`:46`-`:89`) and its distinct non-probe PERF
//!   domain-count cross-check (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority. This crate cannot validate other domains' payloads, but it
//! replays d35's full status-independent payload authority.
//!
//! `decode_compact_air_variant_stats_data_payload`
//! (`domain_data_authority.rs:7680`-`:7904`) adds predicates the fold's
//! pre-exclusion `d35::parse_header` does not run: header count bounds and exact
//! length, unique bounded variant ids, zero flags, nonzero equipment tokens,
//! reserved-row validation, and all ten stats in `0..=1_000_000_000`.
//! [`check_authority_payload`] enforces them on every d35 record, including
//! failure-status records, by calling the domain module's own boundaries wherever
//! they expose the predicate.
//!
//! The five-check sweep also made identity mode a stated input. Legacy remains
//! the default, preserving the existing real-capture measurement.
//!
//! **The one exclusion path, and where it sits.** `fold_air_variant_stats` has
//! exactly one return-without-folding path: `handle_compact_failure`
//! (`world_fold.rs:15149`-`:15155`). It has no delta-before-baseline path (it
//! never reads `COMPACT_STATUS_DELTA_DATA`), no partial-before-baseline path and
//! no rowless-partial path — a rowless payload is ordinary. The position matters:
//! the fold checks the 36-byte minimum and both scale words **before** the status
//! guard (`:15137`-`:15148`) and the exact payload length **after** it (`:15156`).
//! So this harness classifies with `d35::parse_header`, which runs only the
//! pre-guard checks, while the independent status-agnostic payload authority
//! still runs `split_payload`-equivalent length and row validation on every
//! record before this harness applies the fold's exclusion.
//!
//! **No other writer touches the accumulator.** `world_fold.rs` mutates
//! `self.air_variant_stats` in exactly two places: the empty `BTreeMap` at
//! construction (`:2447`) and the insert in the fold (`:15194`). `:8624` and
//! `:8648` read it for the variant projection. Nothing prunes it — unlike d26,
//! whose accumulator the recorder prunes from D20 deletion rows — so the key set
//! is monotone and a wire delta can never carry a tombstone. That is asserted
//! every hour below rather than assumed.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d35.bin   — the d35 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface (changed rows per hour at stored
//! precision) and the wire arm's baseline/delta raw split.
//!
//! **Only d35 DATA payloads are validated.** Production runs
//! `validate_domain_data_payload_at` on the DATA payload of EVERY domain
//! (`stream_fold.rs:593`); this harness cannot, because validating another
//! domain's payload means owning that domain's authority, and `wire_format`
//! must not depend on `recorder_host`. Every DATA record of any other domain
//! therefore passes with its H4CR header, sequence continuity, hour-boundary,
//! PERF-binding and FRAME_END rules checked but its **payload bytes unread**.
//! The concrete hazard — a d35 record relabelled to another domain id, or
//! dropped outright, silently shrinking the measurement — is closed by an
//! explicit completeness check in `scan_stream`: every closed game hour must
//! carry exactly one d35 DATA record, of any status. That is a per-capture
//! assertion, not a grammar rule; a capture where d35 is not scheduled every
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
use wire_format::d35;

const D35: u16 = 35;

/// Whether the wire arm opens a BASELINE for this measured hour.
///
/// Two reasons, and the second is not a cadence choice: the block boundary at
/// `block_hours` (`0` = a single baseline for the whole run), or a measured
/// hour that does not immediately follow the previous measured one. An hour
/// this domain did not successfully sample is coverage the domain lost, which
/// is §8's gap, and `WIRE_FORMAT.md:187`-`:190` is explicit: "A gap affecting
/// the state part forces the next state observation to be a `BASELINE`. State
/// may have moved and returned while nobody was looking, so neither a delta
/// nor an `UNCHANGED` can be justified across unobserved time." The rule
/// applies in BOTH cadence modes — a single-baseline run still may not delta
/// across unobserved time — which is why the gap clause does not consult
/// `block_hours`. The delta chain restarts at the forced baseline.
fn opens_baseline(
    hours_seen: usize,
    block_hours: usize,
    previous_measured_hour: Option<i32>,
    game_hour: i32,
) -> bool {
    if block_hours != 0 && hours_seen % block_hours == 0 {
        return true;
    }
    match previous_measured_hour {
        Some(previous) => game_hour != previous.saturating_add(1),
        None => true,
    }
}
const DEFAULT_BLOCK_HOURS: usize = 48;

/// One usable d35 hour: its game hour, the accumulator's content *after* folding
/// that hour's record, and the record's byte span in the source stream.
///
/// The state is a snapshot of the running accumulator, not the record's rows —
/// that is the difference an incremental shipping layer makes. It is built
/// during the scan, not later: every way a row can be rejected is a property of
/// the stream, so it belongs to the pass that either accepts the stream or
/// refuses it.
struct D35Hour {
    game_hour: i32,
    state: KeyedState,
    /// Rows this record actually carried — the shipping layer's own churn.
    payload_rows: usize,
    ship_span: Range<usize>,
}

struct StreamScan {
    hours: Vec<D35Hour>,
    /// d35 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// Measured d35 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// Variants the collector saw but could not read, summed over the run. With
    /// no delete grammar these keep their previous values in the accumulator.
    invalid_variant_rows: u64,
}

// Re-exported, not re-declared: a local `(1 << 14) - 1` became wrong the moment
// the contract assigned bit 14 to the measured-empty badge.
const FRAME_END_KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;
const PROBE_DOMAIN_ID: u16 = 250;

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

/// Replay d35's status-independent production payload authority
/// (`domain_data_authority.rs:7680`-`:7904`).
fn check_authority_payload(
    collection: &wire_format::Collection,
    payload: &[u8],
    status_flags: u32,
    at_byte: usize,
) -> Result<(), String> {
    const MAX_VARIANT_ID: u32 = i32::MAX as u32;
    const MAX_RAW: i32 = 1_000_000_000;

    let (header, rows) =
        d35::split_payload(payload).map_err(|e| format!("d35 payload at byte {at_byte}: {e}"))?;
    if header.changed_rows > header.row_count {
        return Err(format!(
            "d35 payload at byte {at_byte}: changed_rows > row_count"
        ));
    }
    if status_flags & COMPACT_STATUS_DELTA_DATA == 0
        && header.row_count > header.observed_variant_rows
    {
        return Err(format!(
            "d35 payload at byte {at_byte}: row_count > observed_variant_rows"
        ));
    }
    if header.invalid_variant_rows > header.observed_variant_rows {
        return Err(format!(
            "d35 payload at byte {at_byte}: invalid_variant_rows > observed_variant_rows"
        ));
    }

    let mut seen = BTreeSet::new();
    for (index, row) in rows.chunks_exact(d35::ROW_LEN).enumerate() {
        let key = d35::key_from_row(row)
            .map_err(|e| format!("d35 row {index} at byte {at_byte}: {e}"))?;
        let variant_id = u32::from_le_bytes(row[0..4].try_into().expect("4 bytes"));
        if variant_id > MAX_VARIANT_ID {
            return Err(format!(
                "d35 row {index} at byte {at_byte}: variant_id {variant_id} is out of range"
            ));
        }
        if !seen.insert(key) {
            return Err(format!(
                "d35 row {index} at byte {at_byte}: duplicate variant_id {variant_id}"
            ));
        }
        d35::record_from_row(collection, row, 0, header.scales)
            .map_err(|e| format!("d35 row {index} at byte {at_byte}: {e}"))?;
        let flags = u32::from_le_bytes(row[4..8].try_into().expect("4 bytes"));
        let equipment_token = u32::from_le_bytes(row[8..12].try_into().expect("4 bytes"));
        let equipment_type_token = u32::from_le_bytes(row[12..16].try_into().expect("4 bytes"));
        if flags != 0 {
            return Err(format!(
                "d35 row {index} at byte {at_byte}: flags are nonzero"
            ));
        }
        if equipment_token == 0 || equipment_type_token == 0 {
            return Err(format!(
                "d35 row {index} at byte {at_byte}: equipment token/type is zero"
            ));
        }
        for (_, offset, _) in d35::STATS {
            let value = i32::from_le_bytes(row[offset..offset + 4].try_into().expect("4 bytes"));
            if !(0..=MAX_RAW).contains(&value) {
                return Err(format!(
                    "d35 row {index} at byte {at_byte}: stat at +{offset} is out of range"
                ));
            }
        }
    }
    Ok(())
}

/// Validate the whole stream against the canonical contract and fold the usable
/// d35 DATA records into per-hour states. Every rejection path is an `Err`,
/// never a silently shorter measurement and never a panic.
fn scan_stream(
    stream: &[u8],
    collection: &wire_format::Collection,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours: Vec<D35Hour> = Vec::new();
    let mut failure_records = 0usize;
    let mut non_contiguous = 0usize;
    let mut invalid_variant_rows = 0u64;

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d35 DATA record of ANY status — the uniqueness rule.
    let mut last_d35_hour: Option<i32> = None;
    // Hour of the last d35 record that entered the measurement — the contiguity
    // report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;

    // The running accumulator. d35 is incremental, so this survives across
    // records and IS the domain's state; a per-record state would be wrong.
    let mut accumulator = d35::Accumulator::new();

    // Closed game hours and d35 DATA records of ANY status — the
    // completeness rule at the end of the scan.
    let mut closed_hours = 0usize;
    let mut d35_records = 0usize;
    let mut pos = 0usize;
    while pos < stream.len() {
        let header = parse_compact_record_header(&stream[pos..])
            .map_err(|e| format!("record at byte {pos}: {e}"))?;
        let payload_start = pos + COMPACT_RECORD_HEADER_LEN;
        let record_end = payload_start
            .checked_add(header.payload_len)
            .ok_or_else(|| format!("record at byte {pos}: length overflow"))?;
        if record_end > stream.len() {
            // `eof_check`: a stream ending mid-record is not a short stream, it
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

        if header.domain_id == D35 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d35_records += 1;
            // First reproduce the checks the FOLD makes before its status guard:
            // the 36-byte minimum (`world_fold.rs:15137`) and nonzero scale words
            // (`:15146`-`:15148`). The independent payload authority below then
            // validates exact length and rows on every status.
            let d35_header = d35::parse_header(payload)
                .map_err(|e| format!("d35 payload at byte {pos}: {e}"))?;

            // One d35 DATA record per game hour, checked over EVERY d35 record
            // and BEFORE any status-based exclusion. Gating this on the record
            // being usable would let a success+failure pair in one hour through
            // the rule entirely.
            if let Some(previous) = last_d35_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d35 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d35_hour = Some(header.game_hour_raw);

            // VALIDITY BEFORE CLASSIFICATION. The failure predicate below reads
            // the status word to classify the v1 vocabulary; it is not a validity
            // predicate. `compact_contract/src/lib.rs:121`-`:123` says so in as
            // many words: "Callers handling untrusted status words must first
            // reject a non-zero `compact_status_unknown_flags_v1` result;
            // otherwise an unassigned bit has no defined classification." Bits
            // 15..=31 are unassigned (`status_flags_base.rs:40`-`:44`), and an
            // unassigned bit "is not evidence of a future informational status".
            //
            // Without this, a record with `status_flags = 1 << 14` reads as "not
            // a failure" — the failure helper masks with the *known* failure set
            // and returns zero for a bit it does not know — and the hour is
            // measured as healthy. The production lane rejects first
            // (`lanes/division_state_movement_live/domain20_compact.rs:419`);
            // this harness does the same rather than re-deriving a lenient rule.
            // No capture carries an unassigned bit, so no measured number moves.
            let unknown_status = compact_status_unknown_flags_v1(d35_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d35 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            check_authority_payload(collection, payload, d35_header.status_flags, pos)?;

            // `handle_compact_failure` (`world_fold.rs:2843`, called at `:15149`)
            // marks the domain-hour unknown and RETAINS the accumulator, folding
            // nothing from this record. The predicate is `compact_contract`'s
            // own, not a copied bit mask, so it cannot drift from the recorder.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D35,
                d35_header.status_flags,
            ) != 0
            {
                failure_records += 1;
            } else {
                if let Some(previous) = last_measured_hour {
                    if header.game_hour_raw != previous + 1 {
                        non_contiguous += 1;
                    }
                }
                last_measured_hour = Some(header.game_hour_raw);

                // Fold exactly as `fold_air_variant_stats` does, through the
                // domain's own public boundary. This runs the exact-length check
                // and every per-row rejection, in the fold's order.
                let folded = accumulator
                    .fold_payload(collection, payload)
                    .map_err(|e| format!("d35 payload at byte {pos}: {e}"))?;
                invalid_variant_rows += folded.invalid_variant_rows as u64;

                hours.push(D35Hour {
                    game_hour: header.game_hour_raw,
                    // The accumulator's content, not the record's rows: d35's
                    // shipping layer is incremental.
                    state: accumulator.variants.clone(),
                    payload_rows: folded.row_count as usize,
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
        return Err("stream carries no usable d35 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d35 DATA
    // record, of any status. Production validates the payload of EVERY domain's
    // DATA record (`stream_fold.rs:593`); this harness cannot (module docs), so
    // a d35 record relabelled to another domain id — or lost outright — would
    // otherwise be silently ignored and the run would report a plausible
    // measurement over fewer hours. Per-hour uniqueness is already enforced
    // above, so equality of the two counts is exactly-one per closed hour. A
    // per-capture assertion, not a grammar rule: a capture that does not
    // schedule d35 every hour fails here loudly and a human adjusts, instead
    // of a number quietly shrinking.
    if d35_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d35_records} d35 DATA \
             record(s); every closed hour of a d35-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    Ok(StreamScan {
        hours,
        failure_records,
        non_contiguous,
        invalid_variant_rows,
    })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let usage = "usage: bench_real_d35 <collector_stream.bin> <out dir> \
                 [block_hours; 0 = one baseline for the whole run] [legacy|identity-v1]\n\
                 identity mode may also be the third argument when block_hours is omitted";
    let input = args.next().unwrap_or_else(|| panic!("{usage}"));
    let out_dir = args.next().unwrap_or_else(|| panic!("{usage}"));
    let third = args.next();
    let (block_hours, identity_mode) = match third.as_deref() {
        None | Some("legacy") => (DEFAULT_BLOCK_HOURS, StreamIdentityMode::Legacy),
        Some("identity-v1") => (DEFAULT_BLOCK_HOURS, StreamIdentityMode::IdentityV1),
        Some(value) => {
            let block_hours = value.parse().unwrap_or_else(|_| panic!("{usage}"));
            let identity_mode = match args.next().as_deref() {
                None | Some("legacy") => StreamIdentityMode::Legacy,
                Some("identity-v1") => StreamIdentityMode::IdentityV1,
                Some(_) => panic!("{usage}"),
            };
            (block_hours, identity_mode)
        }
    };
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d35::schema();
    let collection = &schema.collections[0];

    // ---- Pass 1: validate the WHOLE stream and fold its d35 DATA records. ---
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

    // ---- Pass 2: encode both arms. Nothing here reads stream content again. --

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut prior: Option<&KeyedState> = None;
    let mut reader: Option<KeyedState> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    // Baselines the §1.5 gap rule forced outside a scheduled block start.
    let mut gap_forced_baselines = 0usize;
    let mut previous_measured_hour: Option<i32> = None;
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    let mut empty_hours = 0usize;
    let mut changed_rows_total = 0usize;
    let mut shipping_rows_total = 0usize;
    let mut variants_total = 0usize;

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());
        shipping_rows_total += hour.payload_rows;

        let state: &KeyedState = &hour.state;
        variants_total += state.len();

        // New arm: BASELINE at each block start, stored-minimal DELTA between.
        // `block_hours == 0` means a single baseline for the whole run, which is
        // the shipping layer's own cadence and so the grammar-only comparison.
        let hour_start = wire.len();
        let cadence_start = block_hours != 0 && hours_seen % block_hours == 0;
        let baseline_here =
            opens_baseline(hours_seen, block_hours, previous_measured_hour, hour.game_hour);
        if baseline_here && !cadence_start && hours_seen != 0 {
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
            // The theorem this domain's grammar guarantees: the fold has no
            // delete row and never clears, and nothing else writes the
            // accumulator, so a key cannot leave it. If this ever fires, either
            // the fold gained a delete grammar or the replay is wrong.
            assert!(
                d.tombstones.is_empty(),
                "game hour {}: an upsert-only accumulator produced a tombstone",
                hour.game_hour
            );
            if d.is_empty() {
                empty_hours += 1;
                emitted = false;
            } else {
                changed_rows_total += d.changed.len();
                encode_delta(&schema, std::slice::from_ref(prev), &[Some(d)], &mut wire)
                    .expect("delta encodes");
                delta_bytes += wire.len() - hour_start;
            }
        }
        wire_offsets.push(wire.len());

        // Reader replay: decode the bytes just written against the reader's OWN
        // reconstructed prior, exactly as a conforming reader would, and require
        // the result to be stored-equal to this hour's state. This is what makes
        // the size claim a claim about decodable bytes. An hour that emitted
        // nothing is still checked: the reader's retained state must already
        // equal the fold's, which is precisely what "nothing changed" asserts —
        // and on this domain that is the overwhelming majority of hours.
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

    for (name, bytes) in [("ship_d35.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d35.offsets", &ship_offsets),
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
        "invalid variant rows {} (omitted by the collector; kept stale by the fold)",
        scan.invalid_variant_rows
    );
    println!(
        "variants avg         {:.1}",
        variants_total as f64 / hours_seen as f64
    );
    println!(
        "shipping churn       {shipping_rows_total} rows over {hours_seen} hours \
         ({:.2}/hour) — the shipping layer is incremental",
        shipping_rows_total as f64 / hours_seen as f64
    );
    if block_hours == 0 {
        println!(
            "baselines            {baselines} (one for the whole run, plus \
             {gap_forced_baselines} forced by a gap in measured hours — WIRE_FORMAT §1.5)"
        );
    } else {
        println!(
            "baselines            {baselines} ({} every {block_hours} h, {gap_forced_baselines} \
             forced by a gap in measured hours — WIRE_FORMAT §1.5)",
            baselines - gap_forced_baselines
        );
    }
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "real churn           {:.2} changed rows/hour, 0 tombstones/hour \
         (upsert-only: a tombstone is unreachable, asserted every hour)",
        changed_rows_total as f64 / delta_hours.max(1) as f64
    );
    println!("wire raw split       baseline {baseline_bytes} B + delta {delta_bytes} B");
    println!("ship_d35.bin         {} bytes", ship.len());
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
        d35::schema().collections[0].clone()
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
        frame_end_sealed(sequence, game_hour, 0, 0, 0, 0)
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

    fn perf(sequence: u64, game_hour: i32, domain: u16) -> Vec<u8> {
        record(
            sequence,
            game_hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &vec![0u8; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN],
        )
    }

    /// One 64-byte d35 row, packed as the collector packs one
    /// (`attach_log.rs:70136`-`:70151`).
    fn d35_row(variant_id: u32, speed_cache: i32) -> Vec<u8> {
        let mut row = Vec::with_capacity(d35::ROW_LEN);
        row.extend_from_slice(&variant_id.to_le_bytes());
        row.extend_from_slice(&0u32.to_le_bytes()); // flags
        for token in [20_201u32, 17, 4] {
            row.extend_from_slice(&token.to_le_bytes());
        }
        row.extend_from_slice(&speed_cache.to_le_bytes());
        for stat in [350_000i32, 0, 0, 0, 80_000, 0, 0, 0, 1_950_000] {
            row.extend_from_slice(&stat.to_le_bytes());
        }
        row.extend_from_slice(&0u32.to_le_bytes()); // reserved
        assert_eq!(row.len(), d35::ROW_LEN);
        row
    }

    /// A d35 DATA payload: the 36-byte header then `rows`.
    fn d35_payload(status_flags: u32, rows: &[Vec<u8>]) -> Vec<u8> {
        d35_payload_scaled(status_flags, (100_000, 1_000), rows)
    }

    fn d35_payload_scaled(status_flags: u32, scales: (u32, u32), rows: &[Vec<u8>]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // changed_rows
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&204u32.to_le_bytes()); // observed_variant_rows
        p.extend_from_slice(&0u32.to_le_bytes()); // invalid_variant_rows
        p.extend_from_slice(&scales.0.to_le_bytes());
        p.extend_from_slice(&scales.1.to_le_bytes());
        for row in rows {
            p.extend_from_slice(row);
        }
        assert_eq!(p.len(), d35::HEADER_LEN + rows.len() * d35::ROW_LEN);
        p
    }

    /// Three closed hours: a stating hour, then two rowless ones — the shape the
    /// real capture has (1 record with rows, 145 bare headers).
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D35,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d35_payload(0, &[d35_row(7, 45_000_000), d35_row(9, 46_000_000)]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D35,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d35_payload(0, &[]),
        ));
        s.extend(frame_end(4, 101));
        s.extend(record(
            5,
            102,
            D35,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d35_payload(0, &[d35_row(7, 47_000_000)]),
        ));
        s.extend(frame_end(6, 102));
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
    fn a_well_formed_stream_yields_its_d35_hours() {
        let scan = scan_stream(
            &good_stream(),
            &test_collection(),
            StreamIdentityMode::Legacy,
        )
        .expect("accepted");
        assert_eq!(scan.hours.len(), 3);
        assert_eq!(scan.hours[0].game_hour, 100);
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.non_contiguous, 0);
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helper classifies
        // the v1 vocabulary and is not a validity predicate, so an unassigned bit
        // must be rejected BEFORE classification — otherwise it silently reads as
        // "not a failure" and the hour is measured as healthy.
        // Bit 15 is ASSIGNED (`COMPACT_STATUS_BASELINE_CLEAR`, the stream's deliberate
        // resync primitive), so the UNASSIGNED run starts at 16.
        for bit in 16..32u32 {
            let status = 1u32 << bit;
            // Alone, combined with a known failure bit, and combined with a known
            // informational bit; with rows and without (the capture's own shape is
            // one stating hour and 145 bare headers). The rejection must not
            // depend on any of it.
            for extra in [0u32, 1, 1 << 10] {
                for rows in [Vec::new(), vec![d35_row(7, 45_000_000)]] {
                    let mut s = Vec::new();
                    s.extend(record(
                        1,
                        100,
                        D35,
                        COMPACT_RECORD_TYPE_DOMAIN_DATA,
                        &d35_payload(status | extra, &rows),
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
        // `fold_air_variant_stats` never reads an informational bit at all.
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D35,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d35_payload(informational, &[d35_row(7, 45_000_000)]),
            ));
            s.extend(frame_end(2, 100));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 1, "bit {informational:#x}");
            assert_eq!(scan.failure_records, 0, "bit {informational:#x}");
        }
    }

    #[test]
    fn a_rowless_hour_carries_the_accumulator_forward_not_an_empty_state() {
        // The defining property of an incremental shipping layer, and the one a
        // harness copied from a full-replace domain would get catastrophically
        // wrong: hour 101 carries no rows and its state is still both variants.
        let scan = scan_stream(
            &good_stream(),
            &test_collection(),
            StreamIdentityMode::Legacy,
        )
        .expect("accepted");
        assert_eq!(scan.hours[0].state.len(), 2);
        assert_eq!(scan.hours[0].payload_rows, 2);
        assert_eq!(
            scan.hours[1].state.len(),
            2,
            "a bare header is not an empty hour"
        );
        assert_eq!(scan.hours[1].payload_rows, 0);
        assert_eq!(scan.hours[2].state.len(), 2);
        // And the upsert landed: variant 7's speed changed at hour 102, variant
        // 9's did not, and neither key left.
        let c = test_collection();
        let idx = c
            .value_fields()
            .position(|f| f.field_id == d35::field::SPEED_CACHE)
            .expect("field");
        use wire_format::body::key;
        assert_eq!(
            scan.hours[1].state[&key(&[7])].values[idx],
            wire_format::Value::Continuous(450.0)
        );
        assert_eq!(
            scan.hours[2].state[&key(&[7])].values[idx],
            wire_format::Value::Continuous(470.0)
        );
        assert_eq!(
            scan.hours[2].state[&key(&[9])].values[idx],
            wire_format::Value::Continuous(460.0),
            "an unrestated variant keeps its value"
        );
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        let full = good_stream();
        // A cut immediately after a DATA record leaves that hour open and ends
        // the stream on a record that is not a FRAME_END.
        for records in [1usize, 3, 5] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&full[..cut], &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary IS a well-formed shorter stream.
        for (records, expected_hours) in [(2usize, 1usize), (4, 2), (6, 3)] {
            let cut = boundary(&full, records);
            let scan = scan_stream(&full[..cut], &test_collection(), StreamIdentityMode::Legacy)
                .expect("closed stream is well-formed");
            assert_eq!(scan.hours.len(), expected_hours, "cut after {records}");
        }
    }

    #[test]
    fn a_stream_ending_mid_record_is_rejected() {
        let full = good_stream();
        let boundaries: Vec<usize> = (0..=6).map(|n| boundary(&full, n)).collect();
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
        for records in 0..6 {
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
        for records in 0..6 {
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
            D35,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d35_payload(0, &[]),
        ));
        s.extend(frame_end(2, 999));
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
            D35,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d35_payload(0, &[]),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D35,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d35_payload(0, &[]),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_second_d35_record_in_one_hour_is_rejected() {
        // The rule runs over EVERY d35 DATA record and before status exclusion,
        // so all four success/failure orderings are refused — gating it on the
        // record being usable would let a success+failure pair through. On an
        // incremental domain this matters more than on a full-replace one: two
        // records in one hour would fold twice into one measurement step.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D35,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d35_payload(first, &[d35_row(7, 1_000_000)]),
            ));
            s.extend(record(
                2,
                100,
                D35,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d35_payload(second, &[d35_row(7, 2_000_000)]),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "two d35 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_failure_status_record_is_excluded_and_folds_nothing() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time: the
        // fold retains the accumulator and folds nothing, so the hour is not d35
        // data AND its rows must not reach the accumulator. On an incremental
        // domain the second half is the one that would corrupt every later hour
        // if it were wrong, so it is asserted on the surviving hour's state.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D35,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d35_payload(1 << bit, &[d35_row(7, 1_000_000), d35_row(8, 1_000_000)]),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D35,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d35_payload(0, &[d35_row(9, 2_000_000)]),
            ));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .expect("the stream itself is well-formed");
            assert_eq!(scan.failure_records, 1, "bit {bit}");
            assert_eq!(scan.hours.len(), 1, "bit {bit}");
            assert_eq!(scan.hours[0].game_hour, 101, "bit {bit}");
            assert_eq!(
                scan.hours[0].state.len(),
                1,
                "bit {bit}: the excluded record's rows must not reach the accumulator"
            );
        }
        // A non-failure bit is NOT treated as failure. d35's fold never reads
        // DELTA_DATA at all, so a record carrying it folds exactly as one
        // without it — there is no baseline/delta distinction to make.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D35,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d35_payload(1 << 10, &[d35_row(7, 1_000_000)]),
        ));
        s.extend(frame_end(2, 100));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.hours[0].state.len(), 1);
    }

    #[test]
    fn a_failure_record_is_still_checked_by_the_length_authority() {
        // The fold returns before its own length check, but production's second,
        // status-independent authority still checks exact length
        // (`domain_data_authority.rs:7720`-`:7727`).
        let mut bad = d35_payload(1, &[]);
        bad[8..12].copy_from_slice(&9u32.to_le_bytes()); // row_count lies
        let mut s = Vec::new();
        s.extend(record(1, 100, D35, COMPACT_RECORD_TYPE_DOMAIN_DATA, &bad));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());

        // But a check the fold makes BEFORE the guard still applies to a failure
        // record: a zero scale word rejects the payload regardless of status.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D35,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d35_payload_scaled(1, (0, 1_000), &[]),
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_stream_with_no_d35_records_is_refused_not_reported_as_zero() {
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
    fn a_malformed_d35_payload_is_refused_not_skipped() {
        // A short or inconsistent payload on a SUCCESS record is a corrupt
        // stream, not an hour to drop quietly.
        let mut payload = d35_payload(0, &[d35_row(7, 1_000_000)]);
        payload.truncate(payload.len() - 1);
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D35,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());

        // A row the FOLD rejects must make the SCAN fail, not be accepted and
        // then unwind while measuring. d35's row rejections are exactly two.
        let mut invalid: Vec<Vec<u8>> = Vec::new();
        invalid.push(d35_payload(0, &[d35_row(0, 1_000_000)])); // zero variant_id
        let mut reserved = d35_payload(0, &[d35_row(7, 1_000_000)]);
        let at = d35::HEADER_LEN + 60;
        reserved[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
        invalid.push(reserved);
        // And both scale words, on a success record.
        invalid.push(d35_payload_scaled(0, (0, 1_000), &[d35_row(7, 1_000_000)]));
        invalid.push(d35_payload_scaled(
            0,
            (100_000, 0),
            &[d35_row(7, 1_000_000)],
        ));
        for (i, payload) in invalid.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D35,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "invalid payload {i} must make the scan fail"
            );
        }
    }

    #[test]
    fn a_repeated_variant_is_refused_by_the_payload_authority() {
        // The fold is last-writer-wins, but certification's second authority
        // rejects a repeated variant id (`domain_data_authority.rs:7793`-`:7796`).
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D35,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d35_payload(0, &[d35_row(7, 1_000_000), d35_row(7, 2_000_000)]),
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D35,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d35_payload(0, &[d35_row(7, 10)]),
            )
        };
        for (emitted, scheduled, reserved) in [(0, 1, 0), (1, 0, 0), (0, 0, 1)] {
            let mut s = data(1, 100);
            s.extend(frame_end_sealed(2, 100, emitted, scheduled, 0, reserved));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }
        // Bit 15 is ASSIGNED (`COMPACT_STATUS_BASELINE_CLEAR`, the stream's deliberate
        // resync primitive), so the UNASSIGNED run starts at 16.
        for bit in 16..32u32 {
            let mut s = data(1, 100);
            s.extend(frame_end_sealed(2, 100, 0, 0, 1 << bit, 0));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = data(1, 100);
            s.extend(frame_end_sealed(2, 100, 0, 0, 1 << bit, 0));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }
        for len in [0usize, 1, 16, 31, 33, 64] {
            let mut s = data(1, 100);
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
            let mut s = data(1, 100);
            s.extend(frame_end_sealed(2, 100, 0, 0, status, 0));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_ok());
        }
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D35,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d35_payload(0, &[d35_row(7, 10)]),
            )
        };
        let mut ok = perf(1, 100, 20);
        ok.extend(perf(2, 100, D35));
        ok.extend(perf(3, 100, PROBE_DOMAIN_ID));
        ok.extend(data(4, 100));
        ok.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
        assert!(scan_stream(&ok, &test_collection(), StreamIdentityMode::Legacy).is_ok());

        let mut probe_twice = perf(1, 100, PROBE_DOMAIN_ID);
        probe_twice.extend(perf(2, 100, PROBE_DOMAIN_ID));
        probe_twice.extend(data(3, 100));
        probe_twice.extend(frame_end(4, 100));
        assert!(scan_stream(&probe_twice, &test_collection(), StreamIdentityMode::Legacy).is_ok());

        let mut cases = Vec::new();
        let mut duplicate = perf(1, 100, 20);
        duplicate.extend(perf(2, 100, 20));
        duplicate.extend(data(3, 100));
        duplicate.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        cases.push(duplicate);
        let mut interleaved = data(1, 100);
        interleaved.extend(perf(2, 101, 20));
        interleaved.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        cases.push(interleaved);
        let mut stale = data(1, 100);
        stale.extend(frame_end(2, 100));
        stale.extend(perf(3, 100, 20));
        stale.extend(data(4, 101));
        stale.extend(frame_end_sealed(5, 101, 1, 1, 0, 0));
        cases.push(stale);
        let mut conflict = perf(1, 100, 20);
        conflict.extend(perf(2, 101, 21));
        conflict.extend(data(3, 100));
        conflict.extend(frame_end_sealed(4, 100, 2, 2, 0, 0));
        cases.push(conflict);
        for (emitted, scheduled) in [(1, 1), (3, 3), (2, 3), (3, 2)] {
            let mut counts = perf(1, 100, 20);
            counts.extend(perf(2, 100, D35));
            counts.extend(data(3, 100));
            counts.extend(frame_end_sealed(4, 100, emitted, scheduled, 0, 0));
            cases.push(counts);
        }
        for s in cases {
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }

        let mut two_hours = perf(1, 100, 20);
        two_hours.extend(data(2, 100));
        two_hours.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        two_hours.extend(perf(4, 101, 20));
        two_hours.extend(data(5, 101));
        two_hours.extend(frame_end_sealed(6, 101, 1, 1, 0, 0));
        assert!(scan_stream(&two_hours, &test_collection(), StreamIdentityMode::Legacy).is_ok());
    }

    #[test]
    fn a_data_record_must_open_the_hour_the_pending_perf_records_bound() {
        for (perf_hour, data_hour) in [(100, 101), (101, 100), (100, 200), (200, 100)] {
            let mut s = perf(1, perf_hour, 20);
            s.extend(record(
                2,
                data_hour,
                D35,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d35_payload(0, &[d35_row(7, 10)]),
            ));
            s.extend(frame_end_sealed(3, data_hour, 1, 1, 0, 0));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }
        let mut agreeing = perf(1, 100, 20);
        agreeing.extend(record(
            2,
            100,
            D35,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d35_payload(0, &[d35_row(7, 10)]),
        ));
        agreeing.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&agreeing, &test_collection(), StreamIdentityMode::Legacy).is_ok());
    }

    #[test]
    fn an_excluded_record_is_still_checked_by_the_payload_authority() {
        let failed = 1u32;
        let mut bad = Vec::new();
        let mut changed = d35_payload(failed, &[d35_row(7, 10)]);
        changed[12..16].copy_from_slice(&2u32.to_le_bytes());
        bad.push(changed);
        let mut observed = d35_payload(failed, &[d35_row(7, 10)]);
        observed[20..24].copy_from_slice(&0u32.to_le_bytes());
        bad.push(observed);
        let mut invalid = d35_payload(failed, &[]);
        invalid[24..28].copy_from_slice(&205u32.to_le_bytes());
        bad.push(invalid);
        let mut short = d35_payload(failed, &[d35_row(7, 10)]);
        short.pop();
        bad.push(short);
        bad.push(d35_payload(failed, &[d35_row(0, 10)]));
        bad.push(d35_payload(failed, &[d35_row(u32::MAX, 10)]));
        bad.push(d35_payload(failed, &[d35_row(7, 10), d35_row(7, 11)]));
        let mut flags = d35_payload(failed, &[d35_row(7, 10)]);
        flags[d35::HEADER_LEN + 4..d35::HEADER_LEN + 8].copy_from_slice(&1u32.to_le_bytes());
        bad.push(flags);
        for offset in [8usize, 12] {
            let mut token = d35_payload(failed, &[d35_row(7, 10)]);
            token[d35::HEADER_LEN + offset..d35::HEADER_LEN + offset + 4]
                .copy_from_slice(&0u32.to_le_bytes());
            bad.push(token);
        }
        let mut reserved = d35_payload(failed, &[d35_row(7, 10)]);
        reserved[d35::HEADER_LEN + 60..d35::HEADER_LEN + 64].copy_from_slice(&1u32.to_le_bytes());
        bad.push(reserved);
        for (_, offset, _) in d35::STATS {
            for value in [-1i32, 1_000_000_001] {
                let mut stat = d35_payload(failed, &[d35_row(7, 10)]);
                stat[d35::HEADER_LEN + offset..d35::HEADER_LEN + offset + 4]
                    .copy_from_slice(&value.to_le_bytes());
                bad.push(stat);
            }
        }
        // Every bad payload rides in a stream that WOULD otherwise measure: a
        // second, well-formed hour follows it. Without that trailing hour the
        // scan refuses for an unrelated reason — "no usable d35 DATA records" —
        // and the assertion would hold with `check_authority_payload` deleted,
        // which is a test that cannot fail for its own cause.
        for payload in bad {
            let mut s = record(1, 100, D35, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload);
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D35,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d35_payload(0, &[d35_row(8, 11)]),
            ));
            s.extend(frame_end(4, 101));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let payload = d35_payload(1 << bit, &[d35_row(0, 10)]);
            let mut s = record(1, 100, D35, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload);
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D35,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d35_payload(0, &[d35_row(8, 11)]),
            ));
            s.extend(frame_end(4, 101));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }

        // `interface_type_raw` is read but unconstrained by the authority
        // (`domain_data_authority.rs:7776`), so zero there must not become a new
        // harness-only predicate.
        let mut allowed = d35_payload(failed, &[d35_row(7, 10)]);
        allowed[d35::HEADER_LEN + 16..d35::HEADER_LEN + 20].copy_from_slice(&0u32.to_le_bytes());
        let mut s = record(1, 100, D35, COMPACT_RECORD_TYPE_DOMAIN_DATA, &allowed);
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D35,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d35_payload(0, &[d35_row(8, 11)]),
        ));
        s.extend(frame_end(4, 101));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn the_identity_protocol_is_stated_not_sniffed() {
        use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;

        let legacy = good_stream();
        assert!(scan_stream(&legacy, &test_collection(), StreamIdentityMode::IdentityV1).is_err());
        assert!(scan_stream(&legacy, &test_collection(), StreamIdentityMode::Legacy).is_ok());
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
            if len >= 4 && state & 1 == 0 {
                bytes[..4].copy_from_slice(b"H4CR");
            }
            let _ = scan_stream(&bytes, &test_collection(), StreamIdentityMode::Legacy);
        }
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
        // the next state observation to be a `BASELINE`." The decision function
        // over its whole input space, in BOTH cadence modes: the gap clause
        // must not consult `block_hours`, because a single-baseline run still
        // may not delta across unobserved time.
        for block_hours in [0usize, 48] {
            for hours_seen in 0..96 {
                assert!(opens_baseline(hours_seen, block_hours, None, 100));
                assert!(
                    opens_baseline(hours_seen, block_hours, Some(100), 102),
                    "a skipped hour at position {hours_seen} (block {block_hours}) must \
                     open a baseline"
                );
                assert!(opens_baseline(hours_seen, block_hours, Some(100), 100));
                assert_eq!(
                    opens_baseline(hours_seen, block_hours, Some(100), 101),
                    block_hours != 0 && hours_seen % block_hours == 0,
                    "a contiguous hour at position {hours_seen} (block {block_hours}) is \
                     cadence-only"
                );
            }
        }
        assert!(opens_baseline(1, 48, Some(i32::MAX), i32::MIN));
    }

    #[test]
    fn an_hour_closed_without_a_d35_record_refuses_the_stream() {
        // The completeness bound on what this harness does NOT validate (module
        // docs): it cannot check other domains' payloads, so a d35 record
        // relabelled to another domain id — or dropped — must be caught by
        // counting, not silently measured as fewer hours.
        let mut s = good_stream();
        let mut pos = 0usize;
        let mut target = None;
        while pos < s.len() {
            let header = parse_compact_record_header(&s[pos..]).expect("well-formed");
            if header.domain_id == D35 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
            {
                target = Some(pos);
            }
            pos += COMPACT_RECORD_HEADER_LEN + header.payload_len;
        }
        let target = target.expect("good_stream carries a d35 DATA record");
        // domain_id sits at +20: magic(4) + version(2) + header_len(2) +
        // sequence(8) + game_hour(4).
        s[target + 20..target + 22].copy_from_slice(&99u16.to_le_bytes());
        let err = match scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy) {
            Err(e) => e,
            Ok(_) => panic!("an hour with no d35 record must refuse the stream"),
        };
        assert!(
            err.contains("game hour(s) but carries"),
            "the completeness check must be what rejects it: {err}"
        );
    }
}
