//! Measure the wire format against the shipping compact stream on REAL d16
//! `decisions_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d16-scheduled capture
//! (`runs/batch1/deepwar_t1`). The stream's own d16 DATA records are the shipping
//! format's cost for the domain: a 24-byte header, a fixed 32-byte row per country
//! the collector managed to read this frame, and a variable-length `timed_live`
//! table. This tool replays those records into the accumulated per-hour country
//! state — the same fold `world_fold::fold_decisions` performs — and re-encodes
//! every hour with this crate's grammar: a BASELINE at each 48-hour block start,
//! field-granular stored-minimal DELTAs between.
//!
//! **The arms are not the same shape, and the report says so.** d16's shipping
//! payload is effectively incremental: `fold_decisions` upserts the tags a payload
//! carries and clears nothing (`world_fold.rs:13773`-`:13776`), and the collector
//! packs only the countries it read this frame. In the reference capture the first
//! record carries 356 countries and every later record carries 0–6. The wire arm's
//! per-hour state is therefore the **accumulator**, not the record, because that
//! is what the fold holds and what a reader must reconstruct — so the wire arm
//! re-states 356 rows at every block baseline while the shipping arm never
//! re-states anything. That is the re-baseline-policy effect the recipe warns
//! about; the baseline/delta split below is what keeps the grammar comparison
//! recoverable from it.
//!
//! **Nothing here re-derives a contract it can call.** Payload and row decoding
//! and the frame-level fold go through `wire_format::d16`'s public boundary; H4CR
//! framing, record semantics and sequence continuity go through
//! `compact_contract`'s own parser and stateful validator.
//!
//! **The whole stream is validated before any artifact is written.** `scan_stream`
//! enforces, fail-closed:
//!
//! - every record header through `compact_contract::parse_compact_record_header`
//!   (magic, version, header length, payload bound);
//! - every record through `compact_contract::stream_validator::StatefulStreamValidator`
//!   (sequence continuity, record-type and record-flag semantics, FRAME_END shape);
//! - the hour boundary rules of `recorder_host::stream_fold::BoundaryTracker`
//!   (`recorder_host/src/stream_fold.rs:277`) that do not require DOMAIN_PERF
//!   accounting: a DOMAIN_DATA record opens its game hour, a FRAME_END closes the
//!   open hour and only that hour, and closed hours strictly increase;
//! - that tracker's DOMAIN_PERF hour binding (`:328`-`:365`), including the DATA
//!   arm's pending-PERF rule (`:378`-`:386`);
//! - FRAME_END seal validation (`recorder_host/src/stream_fold.rs:46`-`:89`) and the seal's count
//!   cross-check against distinct non-probe PERF domains (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END.
//!
//! That is a documented **subset** of the production boundary contract;
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority. `wire_format` must not depend on `recorder_host`, so the
//! relevant boundary predicates are mirrored here, exactly as
//! `bench_real_d63.rs` does.
//!
//! The sweep also mirrors d16's status-independent payload authority,
//! `decode_compact_decisions_data_payload`
//! (`domain_data_authority.rs:3336`-`:3435`). `d16::split_payload` and
//! `parse_timed_rows` already enforce bounds, timed-string decoding, and trailing
//! bytes before exclusion. [`check_authority_payload`] adds the remaining
//! authority predicates: `changed_rows <= country_count`; nonzero unique country
//! tags; country flags confined to bits 0..=1; exact declared timed-row totals;
//! and nonzero timed tags with bounded days and state. It runs before every
//! failure-status exclusion. Identity mode is likewise stated by the caller
//! (`recorder_host/src/stream_fold.rs:220`), never inferred from the first record.
//!
//! ## The failure-status decision, made explicitly
//!
//! `fold_decisions` **never reads `status_flags`** — there is no read of payload
//! `+16` anywhere in `world_fold.rs:13727`-`:13778`, and no `handle_compact_failure`
//! call. So unlike d63 (`:9252`) it does not retain the accumulator on a failure
//! status: it applies a failure-masked payload as authoritative. The recipe
//! requires this case to be an explicit, recorded refuse-or-exclude choice, and it
//! is the same one `bench_real_d10.rs` makes for `fold_fuel`:
//!
//! - a failure-status record that carries **no country rows** applies nothing to
//!   any country under either reading, so it is counted and skipped;
//! - a failure-status record that carries country rows is **refused outright**.
//!   Dropping it would leave the measured state permanently behind the fold's;
//!   folding it would contradict the status word the recorder itself publishes.
//!   Either way the number would describe a state nobody holds.
//!
//! This also closes the second-order hole: a failure-status payload's rows would
//! otherwise never be validated, so a corrupt row could hide behind a failure
//! status and still leave the stream measurable. The same reasoning reaches the
//! part of the payload a *rowless* record can still get wrong, so the payload's
//! STRUCTURE — the header, the country region, and the timed region's walk under
//! the fold's trailing-byte rule (`world_fold.rs:13767`) — is checked on **every**
//! d16 DATA record, before the status word is consulted at all.
//!
//! The reference capture has 0 failure-status d16 records, so none of this affects
//! its numbers — but the rule is what makes the number trustworthy on a stream
//! that does have them.
//!
//! ## The stopped `timed_live` table
//!
//! Its rows are parsed (`d16::parse_timed_rows`, which carries the fold's
//! trailing-byte rule) and **never encoded**: it is a stop condition, reported in
//! `runs/batch1/d16_decisions_live/`. The shipping arm keeps the records verbatim,
//! table included, so the tool also reports the table's byte share of the shipping
//! arm — the fraction of d16's bytes the wire arm makes no claim about.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d16.bin   — the d16 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line
//!
//! **Only d16 DATA payloads are validated.** Production runs
//! `validate_domain_data_payload_at` on the DATA payload of EVERY domain
//! (`recorder_host/src/stream_fold.rs:593`); this harness cannot, because validating another
//! domain's payload means owning that domain's authority, and `wire_format`
//! must not depend on `recorder_host`. Every DATA record of any other domain
//! therefore passes with its H4CR header, sequence continuity, hour-boundary,
//! PERF-binding and FRAME_END rules checked but its **payload bytes unread**.
//! The concrete hazard — a d16 record relabelled to another domain id, or
//! dropped outright, silently shrinking the measurement — is closed by an
//! explicit completeness check in `scan_stream`: every closed game hour must
//! carry exactly one d16 DATA record, of any status. That is a per-capture
//! assertion, not a grammar rule; a capture where d16 is not scheduled every
//! hour fails loudly there and a human adjusts.

use std::collections::BTreeSet;
use std::io::Write;
use std::ops::Range;

use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, parse_compact_record_header,
    COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA,
    COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedState,
};
use wire_format::d16;

const D16: u16 = 16;
const BLOCK_HOURS: usize = 48;

/// Whether the wire arm opens a BASELINE for this measured hour.
///
/// Two reasons, and the second is not a cadence choice: the block boundary at
/// `block_hours`, or a measured hour that does not immediately follow the
/// previous measured one. An hour this domain did not successfully sample is
/// coverage the domain lost.
/// State after a gap to resume from a complete baseline. State may have moved and returned while
/// nobody was looking, so neither a delta nor an `UNCHANGED` can be justified
/// across unobserved time." Delta-encoding across the gap would produce bytes
/// that decode — the reader's prior is never invalidated, so the local replay
/// cannot notice — while asserting a continuity the capture does not have. The
/// delta chain restarts at the forced baseline: every later delta chains from
/// it, exactly as after a scheduled block start.
fn opens_baseline(
    hours_seen: usize,
    block_hours: usize,
    previous_measured_hour: Option<i32>,
    game_hour: i32,
) -> bool {
    if hours_seen % block_hours == 0 {
        return true;
    }
    match previous_measured_hour {
        Some(previous) => game_hour != previous.saturating_add(1),
        None => true,
    }
}

/// One usable d16 hour: its game hour, the ACCUMULATED state after folding that
/// hour's record, and the record's byte span in the source stream.
struct D16Hour {
    game_hour: i32,
    /// The accumulator snapshot, not the record's rows: d16's fold retains
    /// countries a payload omits (`world_fold.rs:13773`-`:13776`).
    state: KeyedState,
    ship_span: Range<usize>,
}

#[derive(Default)]
struct Totals {
    /// Country rows carried by the payload this hour (not the accumulator size).
    country_rows: usize,
    timed_rows: usize,
    timed_rows_dropped: usize,
    duplicate_country_tags: usize,
    /// Bytes of the shipping arm, split three ways.
    header_bytes: usize,
    country_bytes: usize,
    timed_bytes: usize,
    /// Records whose `changed_rows` word is below `country_count`. Production's
    /// independent authority rejects only the opposite (`changed_rows > count`);
    /// the fold never reads `+12`, so this remaining mismatch is reported.
    changed_rows_mismatch: usize,
    /// Country rows whose `flags` word has `FLAG_TIMED_LIVE_READ` clear — the
    /// collector could not read the timed vector, and the fold records the country
    /// as measured-empty regardless (`world_fold.rs:13736`, `:13750`).
    unread_timed_vector_rows: usize,
    /// Counter cells that read zero — the cells whose real-vs-unreadable meaning
    /// the payload cannot express (`attach_log.rs:82843`-`:82846`).
    zero_counter_cells: usize,
    counter_cells: usize,
}

struct StreamScan {
    hours: Vec<D16Hour>,
    /// d16 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// Measured d16 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    totals: Totals,
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

/// The d16 payload-authority predicates not already enforced by
/// `split_payload`/`parse_timed_rows`, from
/// `domain_data_authority.rs:3336`-`:3435`.
fn check_authority_payload(
    header: &d16::PayloadHeader,
    country_rows: &[u8],
    timed_region: &[u8],
    at_byte: usize,
) -> Result<(), String> {
    if header.changed_rows > header.country_count {
        return Err(format!(
            "d16 payload at byte {at_byte}: changed_rows {} > country_count {}",
            header.changed_rows, header.country_count
        ));
    }
    let mut seen = BTreeSet::new();
    let mut declared_timed = 0usize;
    for (index, row) in country_rows.chunks_exact(d16::COUNTRY_ROW_LEN).enumerate() {
        let tag = u32::from_le_bytes(row[0..4].try_into().expect("4 bytes"));
        let flags = u32::from_le_bytes(row[4..8].try_into().expect("4 bytes"));
        let timed = u32::from_le_bytes(row[28..32].try_into().expect("4 bytes")) as usize;
        if tag == 0 {
            return Err(format!(
                "d16 country row {index} at byte {at_byte}: zero tag_db_id"
            ));
        }
        if !seen.insert(tag) {
            return Err(format!(
                "d16 country row {index} at byte {at_byte}: duplicate tag_db_id {tag}"
            ));
        }
        if flags & !0b11 != 0 {
            return Err(format!(
                "d16 country row {index} at byte {at_byte}: unknown flags {flags:#x}"
            ));
        }
        declared_timed = declared_timed.saturating_add(timed);
    }
    if declared_timed != header.timed_count as usize {
        return Err(format!(
            "d16 payload at byte {at_byte}: timed_count {} != declared {declared_timed}",
            header.timed_count
        ));
    }
    let timed_rows = d16::parse_timed_rows(timed_region, header.timed_count)
        .map_err(|e| format!("d16 payload at byte {at_byte}: {e}"))?;
    for (index, row) in timed_rows.iter().enumerate() {
        if row.tag_db_id == 0 {
            return Err(format!(
                "d16 timed row {index} at byte {at_byte}: zero tag_db_id"
            ));
        }
        if !(0..=10_000).contains(&row.days) {
            return Err(format!(
                "d16 timed row {index} at byte {at_byte}: invalid days {}",
                row.days
            ));
        }
        if !(0..=16).contains(&row.state) {
            return Err(format!(
                "d16 timed row {index} at byte {at_byte}: invalid state {}",
                row.state
            ));
        }
    }
    Ok(())
}

/// Validate the whole stream against the canonical contract and fold the usable
/// d16 DATA records into the accumulated per-hour states. Every rejection path is
/// an `Err`, never a silently shorter measurement and never a panic.
fn scan_stream(
    stream: &[u8],
    collection: &wire_format::Collection,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours: Vec<D16Hour> = Vec::new();
    let mut failure_records = 0usize;
    let mut non_contiguous = 0usize;
    let mut totals = Totals::default();

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d16 DATA record of ANY status — the uniqueness rule.
    let mut last_d16_hour: Option<i32> = None;
    // Hour of the last d16 record that entered the measurement.
    let mut last_measured_hour: Option<i32> = None;

    // The fold's accumulator, carried across records: this is the whole reason
    // d16 needs a frame-level fold boundary rather than a per-record state.
    let mut accumulator = d16::Accumulator::new();

    // Closed game hours and d16 DATA records of ANY status — the
    // completeness rule at the end of the scan.
    let mut closed_hours = 0usize;
    let mut d16_records = 0usize;
    let mut pos = 0usize;
    while pos < stream.len() {
        let header = parse_compact_record_header(&stream[pos..])
            .map_err(|e| format!("record at byte {pos}: {e}"))?;
        let payload_start = pos + COMPACT_RECORD_HEADER_LEN;
        let record_end = payload_start
            .checked_add(header.payload_len)
            .ok_or_else(|| format!("record at byte {pos}: length overflow"))?;
        if record_end > stream.len() {
            // `eof_check`: a stream ending mid-record is not a short stream, it is
            // a corrupt one.
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
                closed_hours += 1;
                pending_perf_hour = None;
                non_probe_perf_domains.clear();
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        if header.domain_id == D16 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d16_records += 1;
            let (d16_header, country_rows, timed_region) = d16::split_payload(payload)
                .map_err(|e| format!("d16 payload at byte {pos}: {e}"))?;

            // One d16 DATA record per game hour, checked over EVERY d16 record and
            // BEFORE any status-based exclusion. Two records in one hour would
            // become two measurement hours built from the same clock time, and
            // gating this on the record being usable would let a success/failure
            // pair through the rule entirely.
            if let Some(previous) = last_d16_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d16 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d16_hour = Some(header.game_hour_raw);

            // STRUCTURE BEFORE STATUS, over EVERY d16 record. `fold_decisions`
            // decodes the whole payload before it can accept it, and its
            // trailing-byte rule (`world_fold.rs:13767`) rejects the frame when
            // the timed region is short, over-long, or carries invalid UTF-8. A
            // record excluded from the arms is still a record in the stream, so a
            // malformed one is a corrupt stream rather than an hour to drop
            // quietly — otherwise a malformed payload could hide behind a failure
            // status and still leave the stream measurable.
            //
            // and the whole property it implies rather
            // than the one reported input: status flag 1 with `country_count = 0`,
            // `timed_count = 1` and no timed-row bytes was accepted here while
            // `fold_decisions` rejects it. `split_payload` covers the header and
            // the country region; this covers the rest. Measured records parse the
            // region again inside `fold_payload`, which is 311 rows over the whole
            // reference capture and not worth an entry point to avoid.
            d16::parse_timed_rows(timed_region, d16_header.timed_count)
                .map_err(|e| format!("d16 payload at byte {pos}: {e}"))?;

            // VALIDITY BEFORE CLASSIFICATION. `compact_status_is_failure` and the
            // tolerance-aware predicate below are classifiers over the v1 status
            // vocabulary, not validity checks — `compact_contract/src/lib.rs:121`-
            // `:123` says so in as many words: "Callers handling untrusted status
            // words must first reject a non-zero `compact_status_unknown_flags_v1`
            // result; otherwise an unassigned bit has no defined classification."
            // Bits 15..=31 are unassigned (`status_flags_base.rs:40`-`:44`), and an
            // unassigned bit "is not evidence of a future informational status".
            //
            // without this, a record with
            // `status_flags = 1 << 14` and a valid country row was measured as a
            // healthy hour, because the failure helper masks with the *known*
            // failure set and returns zero for a bit it does not know. Every
            // production lane already rejects first (e.g.
            // `lanes/division_state_movement_live/domain20_compact.rs:419`); this
            // harness now does the same rather than re-deriving a lenient rule.
            let unknown_status =
                compact_contract::compact_status_unknown_flags_v1(d16_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d16 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            check_authority_payload(&d16_header, country_rows, timed_region, pos)?;

            // The explicit refuse-or-exclude predicate is `compact_contract`'s own,
            // cannot drift from the recorder's lane-local masks or tolerances.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D16,
                d16_header.status_flags,
            ) != 0
            {
                if d16_header.country_count != 0 {
                    return Err(format!(
                        "d16 DATA at byte {pos}: hour {} has failure status {:#x} and carries \
                         {} country row(s); `fold_decisions` would apply them, so the hour can \
                         be neither measured nor skipped without diverging from the fold",
                        header.game_hour_raw, d16_header.status_flags, d16_header.country_count
                    ));
                }
                failure_records += 1;
                pos = record_end;
                continue;
            }

            if let Some(previous) = last_measured_hour {
                if header.game_hour_raw != previous + 1 {
                    non_contiguous += 1;
                }
            }
            last_measured_hour = Some(header.game_hour_raw);

            // Shipping-arm byte accounting, split so the stopped table's share is
            // reported rather than folded into one number.
            totals.header_bytes += COMPACT_RECORD_HEADER_LEN + d16::HEADER_LEN;
            totals.country_bytes += country_rows.len();
            totals.timed_bytes += timed_region.len();
            if d16_header.changed_rows != d16_header.country_count {
                totals.changed_rows_mismatch += 1;
            }
            // Two properties of the row form that the fold discards and this
            // deliverable reports: the timed-vector read flag (`attach_log.rs:69239`,
            // dropped at `world_fold.rs:13736`) and the zero counter cells whose
            // real-vs-unreadable meaning the payload cannot express.
            for row in country_rows.chunks_exact(d16::COUNTRY_ROW_LEN) {
                let flags = u32::from_le_bytes([row[4], row[5], row[6], row[7]]);
                if flags & d16::FLAG_TIMED_LIVE_READ == 0 {
                    totals.unread_timed_vector_rows += 1;
                }
                for (_, offset) in d16::COUNTER_OFFSETS {
                    let v = u32::from_le_bytes([
                        row[offset],
                        row[offset + 1],
                        row[offset + 2],
                        row[offset + 3],
                    ]);
                    totals.counter_cells += 1;
                    if v == 0 {
                        totals.zero_counter_cells += 1;
                    }
                }
            }

            // Fold exactly as `fold_decisions` does, through the shared boundary.
            // Every way the payload can be malformed — a short country region, a
            // timed row that runs past the end, invalid UTF-8 in a decision token,
            // trailing bytes — is an `Err` here, in the pass that either accepts
            // the stream or refuses it.
            let outcome = accumulator
                .fold_payload(collection, payload)
                .map_err(|e| format!("d16 payload at byte {pos}: {e}"))?;
            totals.country_rows += outcome.country_rows;
            totals.timed_rows += outcome.timed_rows;
            totals.timed_rows_dropped += outcome.timed_rows_dropped;
            totals.duplicate_country_tags += outcome.duplicate_country_tags;

            hours.push(D16Hour {
                game_hour: header.game_hour_raw,
                // The accumulator AFTER this record: what the fold now holds.
                state: accumulator.countries.clone(),
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
    if hours.is_empty() {
        return Err("stream carries no usable d16 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d16 DATA
    // record, of any status. Production validates the payload of EVERY domain's
    // DATA record (`recorder_host/src/stream_fold.rs:593`); this harness cannot (module docs), so
    // a d16 record relabelled to another domain id — or lost outright — would
    // otherwise be silently ignored and the run would report a plausible
    // measurement over fewer hours. Per-hour uniqueness is already enforced
    // above, so equality of the two counts is exactly-one per closed hour. A
    // per-capture assertion, not a grammar rule: a capture that does not
    // schedule d16 every hour fails here loudly and a human adjusts, instead
    // of a number quietly shrinking.
    if d16_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d16_records} d16 DATA \
             record(s); every closed hour of a d16-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    Ok(StreamScan {
        hours,
        failure_records,
        non_contiguous,
        totals,
    })
}

/// Both arms and the statistics taken while building them.
///
/// The churn figures are split two ways on purpose. `delta_*` describes what the
/// **delta path** carried and is a property of this block cadence; `real_*`
/// describes what the **data** did and is a property of the campaign. A change
/// that lands on a block boundary is carried by that hour's baseline and so never
/// reference capture's change at game hour 60833079 (hour index 672, a block
/// start) moved tag 38's `active_targeted_timed` from 328 to 331 and was invisible
/// in the delta accounting. Quoting a delta-path count as "the campaign's real
/// changes" undercounts by exactly the changes the cadence happened to swallow.
#[derive(Default)]
struct Measurement {
    ship: Vec<u8>,
    ship_offsets: Vec<usize>,
    wire: Vec<u8>,
    wire_offsets: Vec<usize>,
    hours_seen: usize,
    /// Sum of the accumulator's row count over every hour.
    state_rows_total: usize,
    baselines: usize,
    /// Baselines the gap handling forced outside a scheduled block start.
    gap_forced_baselines: usize,
    baseline_bytes: usize,
    /// Hours that emitted a wire DELTA, and the bytes they cost.
    delta_hours: usize,
    delta_bytes: usize,
    /// Hours that emitted nothing because the state did not move.
    empty_hours: usize,
    /// Rows and tombstones carried by the emitted deltas.
    delta_changed_rows: usize,
    delta_tombstones: usize,
    /// Rows and tombstones that CHANGED, over every hour with a prior — baseline
    /// hours included. Cadence-independent.
    real_changed_rows: usize,
    real_tombstones: usize,
    /// Hours in which anything changed at all, baseline hours included.
    real_changed_hours: usize,
}

/// Build both arms, verifying the reader replay every hour.
///
/// Factored out of `main` so the block-cadence behaviour is testable with a small
/// block size and a change placed on a boundary.
fn measure(
    schema: &wire_format::Schema,
    collection: &wire_format::Collection,
    stream: &[u8],
    hours: &[D16Hour],
    block_hours: usize,
) -> Measurement {
    let mut m = Measurement {
        ship_offsets: vec![0usize],
        wire_offsets: vec![0usize],
        ..Measurement::default()
    };
    let mut prior: Option<&KeyedState> = None;
    let mut reader: Option<KeyedState> = None;
    let mut previous_measured_hour: Option<i32> = None;

    for hour in hours {
        // Shipping arm: the record's exact bytes, header included.
        m.ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        m.ship_offsets.push(m.ship.len());

        // The hour's state is the ACCUMULATOR after this record, folded during the
        // scan: d16 upserts and clears nothing, so the record's rows are not the
        // hour's state.
        let state: &KeyedState = &hour.state;
        m.state_rows_total += state.len();

        // Cadence-independent churn, taken BEFORE the block decision so a change
        // on a block boundary is counted like any other.
        if let Some(prev) = prior {
            let real = diff(collection, prev, state);
            if !real.is_empty() {
                m.real_changed_hours += 1;
                m.real_changed_rows += real.changed.len();
                m.real_tombstones += real.tombstones.len();
            }
        }

        let hour_start = m.wire.len();
        let baseline_here = opens_baseline(
            m.hours_seen,
            block_hours,
            previous_measured_hour,
            hour.game_hour,
        );
        if baseline_here && m.hours_seen % block_hours != 0 {
            m.gap_forced_baselines += 1;
        }
        let mut emitted = true;
        if baseline_here || prior.is_none() {
            encode_baseline(schema, std::slice::from_ref(state), &mut m.wire)
                .expect("baseline encodes");
            m.baselines += 1;
            m.baseline_bytes += m.wire.len() - hour_start;
        } else {
            let prev = prior.unwrap();
            let d = diff(collection, prev, state);
            if d.is_empty() {
                m.empty_hours += 1;
                emitted = false;
            } else {
                m.delta_hours += 1;
                m.delta_changed_rows += d.changed.len();
                m.delta_tombstones += d.tombstones.len();
                encode_delta(schema, std::slice::from_ref(prev), &[Some(d)], &mut m.wire)
                    .expect("delta encodes");
                m.delta_bytes += m.wire.len() - hour_start;
            }
        }
        m.wire_offsets.push(m.wire.len());

        // Reader replay: decode the bytes just written against the reader's OWN
        // reconstructed prior, exactly as a conforming reader would, and require
        // the result to be stored-equal to this hour's state. An hour that emitted
        // nothing is still checked: the reader's retained state must already equal
        // the fold's, which is precisely what "nothing changed" asserts.
        if emitted {
            let hour_bytes = &m.wire[hour_start..];
            if baseline_here || reader.is_none() {
                let (states, consumed) =
                    decode_baseline(schema, hour_bytes, 0).expect("baseline decodes");
                assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
                reader = Some(states.into_iter().next().unwrap());
            } else {
                let reader_prior = reader.take().unwrap();
                let (deltas, consumed) =
                    decode_delta(schema, std::slice::from_ref(&reader_prior), hour_bytes, 0)
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
        m.hours_seen += 1;
    }
    m
}

fn main() {
    const USAGE: &str = "usage: bench_real_d16 <collector_stream.bin> <out dir> \
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
    let schema = d16::schema();
    let collection = &schema.collections[0];

    // ---- Pass 1: validate the WHOLE stream and fold its d16 DATA records. ----
    let scan = match scan_stream(&stream, collection, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: encode both arms. Nothing here reads stream content again. --
    let m = measure(&schema, collection, &stream, &scan.hours, BLOCK_HOURS);
    let (ship, wire) = (&m.ship, &m.wire);
    let hours_seen = m.hours_seen;

    for (name, bytes) in [("ship_d16.bin", ship), ("wire_new.bin", wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d16.offsets", &m.ship_offsets),
        ("wire_new.offsets", &m.wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    let t = &scan.totals;
    println!(
        "hours                {hours_seen} ({} non-contiguous steps), replay-verified {hours_seen}",
        scan.non_contiguous
    );
    println!(
        "failure-status hours {} (rowless; excluded from both arms)",
        scan.failure_records
    );
    println!(
        "payload country rows {} total, {:.2}/hour; accumulator {:.1} rows/hour (max {})",
        t.country_rows,
        t.country_rows as f64 / hours_seen as f64,
        m.state_rows_total as f64 / hours_seen as f64,
        scan.hours.iter().map(|h| h.state.len()).max().unwrap_or(0),
    );
    println!(
        "duplicate tags/frame {}   changed_rows != country_count: {} record(s)",
        t.duplicate_country_tags, t.changed_rows_mismatch
    );
    println!(
        "timed_live (STOPPED) {} rows ({} dropped by the fold for an unknown tag)",
        t.timed_rows, t.timed_rows_dropped
    );
    let ship_total = t.header_bytes + t.country_bytes + t.timed_bytes;
    println!(
        "ship byte split      headers {} ({:.2}%)  country rows {} ({:.2}%)  timed_live {} ({:.2}%)",
        t.header_bytes,
        100.0 * t.header_bytes as f64 / ship_total as f64,
        t.country_bytes,
        100.0 * t.country_bytes as f64 / ship_total as f64,
        t.timed_bytes,
        100.0 * t.timed_bytes as f64 / ship_total as f64,
    );
    println!(
        "zero counter cells   {} of {} ({:.2}%) — real-zero vs unreadable is not expressible",
        t.zero_counter_cells,
        t.counter_cells,
        100.0 * t.zero_counter_cells as f64 / t.counter_cells.max(1) as f64,
    );
    println!(
        "rows with the timed-vector read flag clear: {} (the fold records them as measured-empty)",
        t.unread_timed_vector_rows
    );
    println!(
        "baselines            {} ({} every {BLOCK_HOURS} h, {} forced by a gap in measured \
         hours —, {} raw bytes",
        m.baselines,
        m.baselines - m.gap_forced_baselines,
        m.gap_forced_baselines,
        m.baseline_bytes
    );
    println!(
        "delta hours          {} ({} raw bytes)   empty hours {}",
        m.delta_hours, m.delta_bytes, m.empty_hours
    );
    // Two churn figures, because they answer different questions. The first is a
    // property of the DATA and is cadence-independent — it counts every hour with a
    // prior, baseline hours included. The second is what the delta path actually
    // carried at this block size; the gap between them is the change this cadence
    // swallowed into a baseline.
    println!(
        "real churn           {} changed rows over {} changed hour(s) of {}; \
         {:.4} rows/hour, {} tombstone(s) (cadence-independent)",
        m.real_changed_rows,
        m.real_changed_hours,
        hours_seen.saturating_sub(1),
        m.real_changed_rows as f64 / hours_seen.saturating_sub(1).max(1) as f64,
        m.real_tombstones,
    );
    println!(
        "delta-path churn     {} changed rows, {} tombstone(s) over {} delta hour(s); \
         {} change(s) fell on a block boundary and were carried by a baseline",
        m.delta_changed_rows,
        m.delta_tombstones,
        m.delta_hours,
        m.real_changed_rows.saturating_sub(m.delta_changed_rows),
    );
    println!("ship_d16.bin         {} bytes", ship.len());
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
        d16::schema().collections[0].clone()
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

    fn country_row(tag: u32, flags: u32, active_timed: u32) -> Vec<u8> {
        let mut row = Vec::new();
        row.extend_from_slice(&tag.to_le_bytes());
        row.extend_from_slice(&flags.to_le_bytes());
        row.extend_from_slice(&active_timed.to_le_bytes());
        row.extend_from_slice(&[0u8; 20]);
        assert_eq!(row.len(), d16::COUNTRY_ROW_LEN);
        row
    }

    /// A d16 DATA payload: 24-byte header, then the given country rows, then the
    /// given already-encoded timed rows.
    fn d16_payload(status_flags: u32, rows: &[Vec<u8>], timed: &[Vec<u8>]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes());
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&(timed.len() as u32).to_le_bytes());
        for r in rows {
            let mut r = r.clone();
            let tag = u32::from_le_bytes(r[0..4].try_into().expect("tag"));
            let declared = timed
                .iter()
                .filter(|timed_row| {
                    u32::from_le_bytes(timed_row[0..4].try_into().expect("timed tag")) == tag
                })
                .count() as u32;
            r[28..32].copy_from_slice(&declared.to_le_bytes());
            p.extend_from_slice(&r);
        }
        for r in timed {
            p.extend_from_slice(r);
        }
        p
    }

    fn timed_row(tag: u32, days: i32, state: i32, decision: &str) -> Vec<u8> {
        let mut row = Vec::new();
        row.extend_from_slice(&tag.to_le_bytes());
        row.extend_from_slice(&days.to_le_bytes());
        row.extend_from_slice(&state.to_le_bytes());
        row.extend_from_slice(&(decision.len() as u16).to_le_bytes());
        row.extend_from_slice(decision.as_bytes());
        row
    }

    /// Two closed hours, each carrying one d16 DATA record.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D16,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d16_payload(0, &[country_row(7, 3, 1), country_row(8, 3, 0)], &[]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D16,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d16_payload(0, &[country_row(7, 3, 2)], &[timed_row(7, 30, 0, "GER_x")]),
        ));
        s.extend(frame_end(4, 101));
        s
    }

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
            D16,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d16_payload(0, &[country_row(7, 3, 0)], &[]),
        )
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        let mut ok = Vec::new();
        ok.extend(domain_data(1, 100));
        ok.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        assert!(scan_stream(&ok, &test_collection(), StreamIdentityMode::Legacy).is_ok());

        let mut bad = vec![
            frame_end_sealed(2, 100, 0, 0, 0, 1),
            frame_end_sealed(2, 100, 0, 1, 0, 0),
            frame_end_sealed(2, 100, 1, 0, 0, 0),
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
        let mut later = domain_data(1, 100);
        later.extend(frame_end(2, 100));
        later.extend(domain_data(3, 101));
        later.extend(frame_end_sealed(4, 101, 0, 0, 0, 1));
        assert!(scan_stream(&later, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        let mut ok = perf(1, 100, 20);
        ok.extend(perf(2, 100, D16));
        ok.extend(perf(3, 100, PROBE_DOMAIN_ID));
        ok.extend(domain_data(4, 100));
        ok.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
        assert!(scan_stream(&ok, &test_collection(), StreamIdentityMode::Legacy).is_ok());

        let mut probes = perf(1, 100, PROBE_DOMAIN_ID);
        probes.extend(perf(2, 100, PROBE_DOMAIN_ID));
        probes.extend(domain_data(3, 100));
        probes.extend(frame_end(4, 100));
        assert!(scan_stream(&probes, &test_collection(), StreamIdentityMode::Legacy).is_ok());

        let mut duplicate = perf(1, 100, 20);
        duplicate.extend(perf(2, 100, 20));
        duplicate.extend(domain_data(3, 100));
        duplicate.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        assert!(scan_stream(&duplicate, &test_collection(), StreamIdentityMode::Legacy).is_err());

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

        let mut conflict = perf(1, 100, 20);
        conflict.extend(perf(2, 101, 21));
        conflict.extend(domain_data(3, 100));
        conflict.extend(frame_end_sealed(4, 100, 2, 2, 0, 0));
        assert!(scan_stream(&conflict, &test_collection(), StreamIdentityMode::Legacy).is_err());

        for (emitted, scheduled) in [(1, 1), (3, 3), (2, 3), (3, 2)] {
            let mut s = perf(1, 100, 20);
            s.extend(perf(2, 100, D16));
            s.extend(domain_data(3, 100));
            s.extend(frame_end_sealed(4, 100, emitted, scheduled, 0, 0));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }

        let mut two = perf(1, 100, 20);
        two.extend(domain_data(2, 100));
        two.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        two.extend(perf(4, 101, 20));
        two.extend(domain_data(5, 101));
        two.extend(frame_end_sealed(6, 101, 1, 1, 0, 0));
        assert_eq!(
            scan_stream(&two, &test_collection(), StreamIdentityMode::Legacy)
                .expect("PERF state resets")
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
    fn an_excluded_record_is_still_checked_by_the_payload_authority() {
        fn authority(payload: &[u8]) -> Result<(), String> {
            let (header, countries, timed) = d16::split_payload(payload)?;
            d16::parse_timed_rows(timed, header.timed_count)?;
            check_authority_payload(&header, countries, timed, 0)
        }

        let mut bad = Vec::new();
        let mut changed = d16_payload(0, &[], &[]);
        changed[12..16].copy_from_slice(&1u32.to_le_bytes());
        bad.push(changed);
        bad.push(d16_payload(0, &[country_row(0, 3, 0)], &[]));
        bad.push(d16_payload(
            0,
            &[country_row(7, 3, 0), country_row(7, 3, 0)],
            &[],
        ));
        bad.push(d16_payload(0, &[country_row(7, 1 << 20, 0)], &[]));
        let mut declares_one = country_row(7, 3, 0);
        declares_one[28..32].copy_from_slice(&1u32.to_le_bytes());
        let mut mismatched_declaration = d16_payload(0, &[declares_one.clone()], &[]);
        mismatched_declaration[d16::HEADER_LEN + 28..d16::HEADER_LEN + 32]
            .copy_from_slice(&1u32.to_le_bytes());
        bad.push(mismatched_declaration);
        bad.push(d16_payload(
            0,
            &[declares_one.clone()],
            &[timed_row(0, 30, 0, "x")],
        ));
        bad.push(d16_payload(
            0,
            &[declares_one.clone()],
            &[timed_row(7, -1, 0, "x")],
        ));
        bad.push(d16_payload(
            0,
            &[declares_one],
            &[timed_row(7, 30, 17, "x")],
        ));
        for payload in &bad {
            assert!(authority(payload).is_err());
        }

        // Every failure bit reaches the status-independent authority. A rowless
        // record avoids d16's separate "failure plus country rows" refusal, so
        // this specifically detects loss of the authority call.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut excluded = d16_payload(1 << bit, &[], &[]);
            excluded[12..16].copy_from_slice(&1u32.to_le_bytes());
            let mut s = record(1, 100, D16, COMPACT_RECORD_TYPE_DOMAIN_DATA, &excluded);
            s.extend(frame_end(2, 100));
            s.extend(domain_data(3, 101));
            s.extend(frame_end(4, 101));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }

        let mut excluded = d16_payload(1, &[], &[]);
        excluded[0..8].copy_from_slice(&u64::MAX.to_le_bytes()); // outside authority reach
        let mut s = record(1, 100, D16, COMPACT_RECORD_TYPE_DOMAIN_DATA, &excluded);
        s.extend(frame_end(2, 100));
        s.extend(domain_data(3, 101));
        s.extend(frame_end(4, 101));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 1);
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
    fn a_well_formed_stream_yields_its_d16_hours() {
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
        assert_eq!(scan.totals.timed_rows, 1);
        assert_eq!(scan.totals.changed_rows_mismatch, 0);
    }

    #[test]
    fn the_measured_state_is_the_accumulator_not_the_record() {
        // The property that would silently drop 355 of 356 countries if the
        // harness took each record's rows as the hour's state.
        let scan = scan_stream(
            &good_stream(),
            &test_collection(),
            StreamIdentityMode::Legacy,
        )
        .expect("accepted");
        assert_eq!(scan.hours[0].state.len(), 2);
        assert_eq!(
            scan.hours[1].state.len(),
            2,
            "hour 101 carries one row but the fold still holds both countries"
        );
    }

    #[test]
    fn the_stopped_table_is_accounted_but_never_encoded() {
        // The timed region's bytes are reported so the deliverable can state what
        // fraction of d16 the wire arm makes no claim about.
        let scan = scan_stream(
            &good_stream(),
            &test_collection(),
            StreamIdentityMode::Legacy,
        )
        .expect("accepted");
        let expected = timed_row(7, 30, 0, "GER_x").len();
        assert_eq!(scan.totals.timed_bytes, expected);
        assert_eq!(scan.totals.country_bytes, 3 * d16::COUNTRY_ROW_LEN);
        assert_eq!(
            scan.totals.header_bytes,
            2 * (COMPACT_RECORD_HEADER_LEN + d16::HEADER_LEN)
        );
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&full[..cut], &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary is a well-formed shorter stream.
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
            D16,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d16_payload(0, &[country_row(7, 3, 1)], &[]),
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
            D16,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d16_payload(0, &[country_row(7, 3, 1)], &[]),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D16,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d16_payload(0, &[country_row(7, 3, 2)], &[]),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_second_d16_record_in_one_hour_is_rejected() {
        // The rule is checked over EVERY d16 DATA record and before status-based
        // exclusion, so all four success/failure orderings are refused — gating it
        // on the record being usable would let a success + failure pair through.
        // The failure-status members carry no rows, so the ONLY thing that can
        // refuse them is the uniqueness rule itself.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let rows_for = |status: u32| {
                if status == 0 {
                    vec![country_row(7, 3, 1)]
                } else {
                    Vec::new()
                }
            };
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D16,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d16_payload(first, &rows_for(first), &[]),
            ));
            s.extend(record(
                2,
                100,
                D16,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d16_payload(second, &rows_for(second), &[]),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "two d16 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_rowless_failure_status_record_is_excluded_rather_than_measured() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time. A
        // rowless failure record applies nothing under either reading — the fold's
        // (which ignores the status word) or the exclusion rule's.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D16,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d16_payload(1 << bit, &[], &[]),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D16,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d16_payload(0, &[country_row(7, 3, 1)], &[]),
            ));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .expect("the stream itself is well-formed");
            assert_eq!(scan.failure_records, 1, "bit {bit}");
            assert_eq!(scan.hours.len(), 1, "bit {bit}");
            assert_eq!(scan.hours[0].game_hour, 101, "bit {bit}");
        }
        // A non-failure bit (DELTA_DATA, 1<<10) is NOT treated as failure. It also
        // changes nothing about the fold: `fold_decisions` never reads the status
        // word, so there is no baseline/delta grammar to switch on.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D16,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d16_payload(1 << 10, &[country_row(7, 3, 1)], &[]),
        ));
        s.extend(frame_end(2, 100));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn a_failure_status_record_that_carries_country_rows_is_refused() {
        // `fold_decisions` has no `handle_compact_failure` call, so it APPLIES a
        // failure-status payload's rows. Measuring it would contradict the status
        // word; skipping it would leave the measured state behind the fold's. It
        // is refused, and the refusal is what stops a corrupt row from hiding
        // behind a failure status.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D16,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d16_payload(1 << bit, &[country_row(7, 3, 1)], &[]),
            ));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "failure bit {bit} with country rows must be refused"
            );
        }
    }

    #[test]
    fn an_excluded_records_payload_is_still_validated() {
        // A record excluded from the arms is
        // still a record in the stream: `fold_decisions` decodes the whole payload
        // before accepting it, so a malformed one is a corrupt stream, not an hour
        // to drop quietly. The exact regression input is the first case; the
        // sweep is every failure bit crossed with every way a rowless payload's
        // timed region can be malformed.
        let reviewers_input = {
            let mut p = d16_payload(1, &[], &[]);
            p[20..24].copy_from_slice(&1u32.to_le_bytes()); // timed_count, no bytes
            p
        };
        let short_string = {
            let mut p = d16_payload(0, &[], &[]);
            p[20..24].copy_from_slice(&1u32.to_le_bytes());
            p.extend_from_slice(&7u32.to_le_bytes());
            p.extend_from_slice(&0i32.to_le_bytes());
            p.extend_from_slice(&0i32.to_le_bytes());
            p.extend_from_slice(&9u16.to_le_bytes()); // claims 9 bytes, supplies 1
            p.push(b'x');
            p
        };
        let bad_utf8 = {
            let mut p = d16_payload(0, &[], &[]);
            p[20..24].copy_from_slice(&1u32.to_le_bytes());
            p.extend_from_slice(&7u32.to_le_bytes());
            p.extend_from_slice(&0i32.to_le_bytes());
            p.extend_from_slice(&0i32.to_le_bytes());
            p.extend_from_slice(&2u16.to_le_bytes());
            p.extend_from_slice(&[0xFF, 0xFE]);
            p
        };
        let trailing = {
            let mut p = d16_payload(0, &[], &[]);
            p.push(0); // timed_count 0, one byte over
            p
        };
        for (name, template) in [
            ("reviewer's input", reviewers_input),
            ("string runs past the end", short_string),
            ("invalid utf-8", bad_utf8),
            ("trailing byte", trailing),
        ] {
            // Every failure bit, plus the no-failure case: the structural check
            // must not depend on the status word at all.
            for status in [
                0u32,
                1,
                1 << 1,
                1 << 2,
                1 << 3,
                1 << 6,
                1 << 7,
                1 << 8,
                1 << 9,
                1 << 12,
            ] {
                let mut payload = template.clone();
                payload[16..20].copy_from_slice(&status.to_le_bytes());
                let mut s = Vec::new();
                s.extend(record(
                    1,
                    100,
                    D16,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &payload,
                ));
                s.extend(frame_end(2, 100));
                // A well-formed measured hour after it, so the stream would
                // otherwise have something to report.
                s.extend(record(
                    3,
                    101,
                    D16,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d16_payload(0, &[country_row(7, 3, 1)], &[]),
                ));
                s.extend(frame_end(4, 101));
                assert!(
                    scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                    "{name} with status {status:#x} must refuse the stream"
                );
            }
        }
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // generalized to every unassigned bit rather than
        // the one reported. `compact_contract/src/lib.rs:121`-`:123`: the failure
        // helpers classify the v1 vocabulary and are not validity predicates, so an
        // unassigned bit must be rejected BEFORE classification — otherwise it
        // silently reads as "not a failure" and the hour is measured as healthy.
        // Bit 15 is ASSIGNED (`COMPACT_STATUS_BASELINE_CLEAR`, the stream's deliberate
        // resync primitive), so the UNASSIGNED run starts at 16.
        for bit in 16..32u32 {
            let status = 1u32 << bit;
            // With rows, without rows, and combined with a known failure bit and a
            // known informational bit: the rejection must not depend on any of it.
            for extra in [0u32, 1, 1 << 10] {
                for rows in [Vec::new(), vec![country_row(7, 3, 1)]] {
                    let mut s = Vec::new();
                    s.extend(record(
                        1,
                        100,
                        D16,
                        COMPACT_RECORD_TYPE_DOMAIN_DATA,
                        &d16_payload(status | extra, &rows, &[]),
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
        // the check must reject unassigned bits and nothing else. DELTA_DATA,
        // WAR_BRIDGE_APPLIED, EXTRACTION_PERF_GATE, TRANSITION_BASELINE_RESET and
        // WAR_SEGMENT_ABSENT are informational.
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D16,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d16_payload(informational, &[country_row(7, 3, 1)], &[]),
            ));
            s.extend(frame_end(2, 100));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 1, "bit {informational:#x}");
            assert_eq!(scan.failure_records, 0, "bit {informational:#x}");
        }
    }

    #[test]
    fn a_stream_with_no_d16_records_is_refused_not_reported_as_zero() {
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
    fn a_malformed_d16_payload_is_refused_not_skipped() {
        // Short header, short country region, a timed row running past the end,
        // invalid UTF-8 in a decision token, and trailing bytes — the whole
        // rejection space of `fold_decisions`, each one a corrupt stream rather
        // than an hour to drop quietly.
        let base = d16_payload(0, &[country_row(7, 3, 1)], &[timed_row(7, 30, 0, "GER_x")]);
        let mut malformed: Vec<Vec<u8>> = Vec::new();
        // Trailing byte (`world_fold.rs:13767`).
        let mut trailing = base.clone();
        trailing.push(0);
        malformed.push(trailing);
        // Truncated anywhere.
        for len in 0..base.len() {
            malformed.push(base[..len].to_vec());
        }
        // A country_count larger than the payload can hold.
        let mut over = base.clone();
        over[8..12].copy_from_slice(&99u32.to_le_bytes());
        malformed.push(over);
        // Invalid UTF-8 in the decision token.
        let mut bad_utf8 = d16_payload(0, &[country_row(7, 3, 1)], &[]);
        bad_utf8[20..24].copy_from_slice(&1u32.to_le_bytes()); // timed_count
        bad_utf8.extend_from_slice(&7u32.to_le_bytes());
        bad_utf8.extend_from_slice(&0i32.to_le_bytes());
        bad_utf8.extend_from_slice(&0i32.to_le_bytes());
        bad_utf8.extend_from_slice(&2u16.to_le_bytes());
        bad_utf8.extend_from_slice(&[0xFF, 0xFE]);
        malformed.push(bad_utf8);

        for (i, payload) in malformed.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D16,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "malformed payload {i} must make the scan fail"
            );
        }
    }

    #[test]
    fn the_folds_permissive_cases_are_measured_not_refused() {
        // The fold alone accepts these, but production's independent payload
        // authority (`domain_data_authority.rs:3336`-`:3435`) refuses them.
        let mut rows = vec![country_row(0, 0, 1), country_row(5, 0xFFFF_FFFF, 2)];
        rows.push(country_row(5, 3, 9)); // repeated tag, last writer wins
        let mut payload = d16_payload(0, &rows, &[]);
        payload[12..16].copy_from_slice(&123u32.to_le_bytes()); // changed_rows
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D16,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D16,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d16_payload(0, &[], &[]),
        ));
        s.extend(frame_end(4, 101));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    /// A stream of `values.len()` hours, each carrying one d16 record that sets
    /// country 7's `active_timed` to that hour's value.
    fn stream_of(values: &[u32]) -> Vec<u8> {
        let mut s = Vec::new();
        let mut sequence = 1u64;
        for (i, v) in values.iter().enumerate() {
            let hour = 100 + i as i32;
            s.extend(record(
                sequence,
                hour,
                D16,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d16_payload(0, &[country_row(7, 3, *v)], &[]),
            ));
            sequence += 1;
            s.extend(frame_end(sequence, hour));
            sequence += 1;
        }
        s
    }

    #[test]
    fn churn_counts_a_change_that_lands_on_a_block_boundary() {
        // `real_*` must be a property of the
        // data and `delta_*` a property of the cadence; a change on a block start
        // is carried by that hour's baseline and never reaches the delta path, so
        // quoting the delta count as "the campaign's real changes" undercounts.
        //
        // With block_hours = 2, hour indices 0, 2, 4 ... are baselines. Place a
        // change at index 2 (a baseline) and another at index 3 (a delta hour).
        let schema = d16::schema();
        let collection = &schema.collections[0];
        let values = [1u32, 1, 5, 9, 9, 9];
        let stream = stream_of(&values);
        let scan =
            scan_stream(&stream, collection, StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), values.len());

        let m = measure(&schema, collection, &stream, &scan.hours, 2);
        assert_eq!(m.hours_seen, 6);
        assert_eq!(m.baselines, 3, "indices 0, 2, 4");
        assert_eq!(
            m.real_changed_rows, 2,
            "index 2 (1 -> 5) and index 3 (5 -> 9) both changed"
        );
        assert_eq!(m.real_changed_hours, 2);
        assert_eq!(
            m.delta_changed_rows, 1,
            "only index 3 is a delta hour; index 2's change rode its baseline"
        );
        assert_eq!(m.delta_hours, 1);
        assert_eq!(m.empty_hours, 2, "indices 1 and 5 changed nothing");
        assert_eq!(m.real_tombstones, 0);

        // The same data with every hour a baseline: real churn is unchanged and
        // the delta path carries nothing. That is the cadence-independence claim.
        let all_baselines = measure(&schema, collection, &stream, &scan.hours, 1);
        assert_eq!(all_baselines.real_changed_rows, 2, "same data, same churn");
        assert_eq!(all_baselines.delta_changed_rows, 0);
        assert_eq!(all_baselines.baselines, 6);

        // And with one block over the whole run: every change reaches the delta
        // path, so the two figures agree.
        let one_block = measure(&schema, collection, &stream, &scan.hours, 1_000);
        assert_eq!(one_block.baselines, 1);
        assert_eq!(one_block.real_changed_rows, 2);
        assert_eq!(one_block.delta_changed_rows, 2);
    }

    #[test]
    fn the_reader_replay_holds_at_every_block_size() {
        // The replay assertions live inside `measure`, so exercising a range of
        // cadences over the same data is what proves they are not accidentally
        // satisfied by one alignment.
        let schema = d16::schema();
        let collection = &schema.collections[0];
        let values = [1u32, 1, 5, 9, 9, 9, 2, 2, 7];
        let stream = stream_of(&values);
        let scan =
            scan_stream(&stream, collection, StreamIdentityMode::Legacy).expect("well-formed");
        for block in 1..=12usize {
            let m = measure(&schema, collection, &stream, &scan.hours, block);
            assert_eq!(m.hours_seen, values.len(), "block {block}");
            assert_eq!(m.real_changed_rows, 4, "block {block}: churn is the data's");
            assert_eq!(
                m.baselines + m.delta_hours + m.empty_hours,
                values.len(),
                "block {block}: every hour is exactly one of the three"
            );
        }
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
        // A complete
        // baseline after a gap. The decision function
        // over its whole input space:
        for hours_seen in 0..(2 * BLOCK_HOURS) {
            assert!(opens_baseline(hours_seen, BLOCK_HOURS, None, 100));
            assert!(
                opens_baseline(hours_seen, BLOCK_HOURS, Some(100), 102),
                "a skipped hour at position {hours_seen} must open a baseline"
            );
            assert!(opens_baseline(hours_seen, BLOCK_HOURS, Some(100), 100));
            assert_eq!(
                opens_baseline(hours_seen, BLOCK_HOURS, Some(100), 101),
                hours_seen % BLOCK_HOURS == 0,
                "a contiguous hour at position {hours_seen} is cadence-only"
            );
        }
        assert!(opens_baseline(1, BLOCK_HOURS, Some(i32::MAX), i32::MIN));

        // And through `measure`: an excluded (failure-status) hour between two
        // measured ones forces a baseline even mid-block, and the reader replay
        // still verifies. Without the rule this delta would be EMPTY — the two
        // measured states are equal — silently asserting "unchanged" across an
        // hour nobody measured.
        let schema = d16::schema();
        let collection = &schema.collections[0];
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D16,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d16_payload(0, &[country_row(7, 3, 1)], &[]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D16,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d16_payload(1 << 0, &[], &[]), // STATE_UNAVAILABLE, rowless
        ));
        s.extend(frame_end(4, 101));
        s.extend(record(
            5,
            102,
            D16,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d16_payload(0, &[country_row(7, 3, 1)], &[]),
        ));
        s.extend(frame_end(6, 102));
        let scan = scan_stream(&s, collection, StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 2, "hour 101 is excluded");
        let m = measure(&schema, collection, &s, &scan.hours, 1_000);
        assert_eq!(
            m.baselines, 2,
            "hour 100 opens the block; hour 102 is forced by the gap"
        );
        assert_eq!(m.gap_forced_baselines, 1);

        // The same two measured states with no gap: one baseline, one empty hour.
        let contiguous = stream_of(&[1, 1]);
        let scan = scan_stream(&contiguous, collection, StreamIdentityMode::Legacy)
            .expect("well-formed");
        let m = measure(&schema, collection, &contiguous, &scan.hours, 1_000);
        assert_eq!(m.baselines, 1);
        assert_eq!(m.gap_forced_baselines, 0);
        assert_eq!(m.empty_hours, 1);
    }

    #[test]
    fn an_hour_closed_without_a_d16_record_refuses_the_stream() {
        // The completeness bound on what this harness does NOT validate (module
        // docs): it cannot check other domains' payloads, so a d16 record
        // relabelled to another domain id — or dropped — must be caught by
        // counting, not silently measured as fewer hours.
        let mut s = good_stream();
        let mut pos = 0usize;
        let mut target = None;
        while pos < s.len() {
            let header = parse_compact_record_header(&s[pos..]).expect("well-formed");
            if header.domain_id == D16 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
            {
                target = Some(pos);
            }
            pos += COMPACT_RECORD_HEADER_LEN + header.payload_len;
        }
        let target = target.expect("good_stream carries a d16 DATA record");
        // domain_id sits at +20: magic(4) + version(2) + header_len(2) +
        // sequence(8) + game_hour(4).
        s[target + 20..target + 22].copy_from_slice(&99u16.to_le_bytes());
        let err = match scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy) {
            Err(e) => e,
            Ok(_) => panic!("an hour with no d16 record must refuse the stream"),
        };
        assert!(
            err.contains("game hour(s) but carries"),
            "the completeness check must be what rejects it: {err}"
        );
    }
}
