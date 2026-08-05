//! Measure the wire format against the shipping compact stream on REAL d11
//! `production_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d11-scheduled capture
//! (`runs/batch1/deepwar_t1`). The stream's own d11 DATA records are the shipping
//! format's cost for the domain: a 40-byte header, three fixed-row base tables
//! and a chain of up to ten self-sized trailer segments, **full-replace per
//! country** — `fold_production` rebuilds its `per_tag` map from scratch each
//! frame but writes it into one country at a time, so a country the frame does
//! not mention keeps the production block it had. This tool replays those records
//! into the running accumulator that behaviour implies — the same fold
//! `world_fold::fold_production` performs, through `wire_format::d11`'s public
//! `State`/`fold_payload` boundary so there is one implementation of it — and
//! re-encodes every hour with this crate's grammar: a BASELINE at each 48-hour
//! block start, field-granular stored-minimal DELTAs between.
//!
//! **The accumulator is the state, not the frame.** In `deepwar_t1` a frame
//! touches a median of two countries out of a roster of 356, so a per-frame
//! snapshot would measure a fraction of the domain. The wire arm encodes the
//! accumulated eight collections; a country dropping a keyed row is a wire
//! tombstone.
//!
//! **`PDQ1` remains stop-reported, while the PCV1 reservation list is ported.**
//! `PDQ1` carries variable-length UTF-8 strings that §4 has no kind for.
//! `PCV1`'s convoy priority reservations are an ordered list whose order is not
//! a function of any carried field, so the harness maps them to the implemented
//! occurrence-list shape (see `d11.rs`'s module docs for both decisions and the
//! capture evidence). Because the shipping arm would otherwise
//! be charged for data the wire arm never carries, a second shipping artifact is
//! written with the `PDQ1` suffix removed, and both ratios are reported.
//! `ship_d11_no_pdq1.bin` is a size accounting only: it is a shipping record
//! with its final trailer cut, not a valid record. The reservation run is not
//! excised — it is interleaved inside `PCV1` rather than a suffix — and its
//! 8 bytes/row are now carried by the wire arm and reported separately.
//!
//! **Nothing here re-derives a contract it can call.** Payload, trailer and row
//! decoding go through `wire_format::d11`'s public boundary; H4CR framing, record
//! semantics, sequence continuity and the domain-hour failure predicate go
//! through `compact_contract`'s own parser, validator and constants.
//!
//! **The whole stream is validated before any artifact is written**, exactly as
//! `bench_real_d22.rs` does and for the same reason: a truncated or spliced
//! stream would otherwise yield a smaller but entirely plausible number.
//! `scan_stream` enforces, fail-closed:
//!
//! - every record header through `compact_contract::parse_compact_record_header`;
//! - every record through `compact_contract::stream_validator::StatefulStreamValidator`;
//! - the hour-boundary rules of `recorder_host::stream_fold::BoundaryTracker`
//!   (`recorder_host/src/stream_fold.rs:277`), including DOMAIN_PERF hour binding
//!   (`:328`-`:365`) and the DATA arm's pending-PERF rule (`:378`-`:386`);
//! - FRAME_END seal fields (`:46`-`:89`, `:401`-`:409`) and the seal's
//!   domain-count cross-check against distinct non-probe PERF domains
//!   (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`);
//! - every d11 payload through `d11::split_payload`/`d11::parse_trailers` and
//!   every row through `d11::record_from_*`, via `d11::fold_payload`.
//!
//! Production also runs `decode_compact_production_data_payload` and its strict
//! trailer chain on every d11 record regardless of status
//! (`domain_data_authority.rs:1287`-`:2639`). Beyond the header already read
//! before exclusion, that authority validates the base length, scale/counts,
//! keyed base rows and their ranges, and every trailer's framing, uniqueness,
//! coverage and cross-links. [`check_authority_payload`] runs the d11 public
//! boundary plus the authority-only base predicates on every record.
//!
//! **Only d11 DATA payloads are validated.** Production runs
//! `validate_domain_data_payload_at` on the DATA payload of EVERY domain
//! (`stream_fold.rs:593`); this harness cannot, because validating another
//! domain's payload means owning that domain's authority, and `wire_format`
//! must not depend on `recorder_host`. Every DATA record of any other domain
//! therefore passes with its H4CR header, sequence continuity, hour-boundary,
//! PERF-binding and FRAME_END rules checked but its **payload bytes unread**.
//! The concrete hazard — a d11 record relabelled to another domain id, or
//! dropped outright, silently shrinking the measurement — is closed by an
//! explicit completeness check in `scan_stream`: every closed game hour must
//! carry exactly one d11 DATA record, of any status. That is a per-capture
//! assertion, not a grammar rule; a capture where d11 is not scheduled every
//! hour fails loudly there and a human adjusts.
//!
//! The seal, PERF binding, pending-PERF DATA rule, status-independent authority
//! and stated identity mode are the five-check sweep carried back from d38. The
//! measured capture is recorder-valid, so none changes an artifact byte.
//!
//! **The exclusion and its position.** d11 has exactly one path on which the fold
//! returns without folding: a failure status (`world_fold.rs:12488`). There is no
//! delta grammar, so no delta-before-baseline and no rowless-partial case. The
//! position matters inside the fold, but the independent payload authority still
//! validates base tables and trailers on failure-status records. This harness
//! therefore reads the header, runs that authority, and only then excludes.
//!
//! Hour uniqueness is enforced over EVERY d11 DATA record, including
//! failure-status ones, and before the status exclusion — otherwise a
//! success+failure pair in one hour would slip through.
//!
//! Outputs, for the compression step (`scripts/compress_arms.py`):
//!   ship_d11.bin          — the d11 DATA records verbatim, in stream order
//!   ship_d11_no_pdq1.bin  — the same with each record's PDQ1 suffix removed
//!   wire_new.bin          — the wire-format bodies, in hour order
//!   *.offsets             — hour-boundary byte offsets, one per line

use std::collections::BTreeSet;
use std::io::Write;
use std::ops::Range;

use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, compact_status_is_failure,
    compact_status_unknown_flags_v1, parse_compact_record_header, COMPACT_FRAME_END_PAYLOAD_LEN,
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_DELTA_DATA, COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::body::{
    apply_shaped, collection_stored_eq, decode_shaped_baseline, decode_shaped_delta, diff_shaped,
    encode_shaped_baseline, encode_shaped_delta, CollectionDelta, CollectionState,
};
use wire_format::d11::{self, Collections, State, StatusPredicates};
use wire_format::Collection;

const D11: u16 = 11;
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

/// The two `compact_contract` predicates `d11::parse_trailers` needs for the
/// `PDQ1` segment check (`world_fold.rs:27478`, `:27484`), handed over as
/// function pointers so the reference library keeps its isolation.
const STATUS: StatusPredicates = StatusPredicates {
    unknown_flags: compact_status_unknown_flags_v1,
    is_failure: compact_status_is_failure,
};

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

/// Production's status-independent d11 decoder
/// (`domain_data_authority.rs:1287`-`:2639`). The d11 public fold boundary
/// supplies the base/trailer grammar and all cross-links; the explicit checks
/// below are the decoder's additional base-row predicates that the fold does not
/// make.
fn check_authority_payload(
    collections: &Collections,
    payload: &[u8],
    at_byte: usize,
) -> Result<(), String> {
    let (header, tables) =
        d11::split_payload(payload).map_err(|e| format!("d11 payload at byte {at_byte}: {e}"))?;
    if header.scale_raw != 100_000 {
        return Err(format!(
            "d11 payload at byte {at_byte}: scale_raw {} != 100000",
            header.scale_raw
        ));
    }
    if header.changed_country_count > header.country_count {
        return Err(format!(
            "d11 payload at byte {at_byte}: changed_rows {} > country_count {}",
            header.changed_country_count, header.country_count
        ));
    }

    for (index, row) in tables
        .stockpile
        .chunks_exact(d11::STOCKPILE_ROW_LEN)
        .enumerate()
    {
        let tag = u32::from_le_bytes(row[0..4].try_into().expect("4 bytes"));
        if tag == 0 {
            return Err(format!(
                "d11 stockpile row {index} at byte {at_byte}: zero tag_db_id"
            ));
        }
    }
    for (index, row) in tables.lines.chunks_exact(d11::LINE_ROW_LEN).enumerate() {
        let tag = u32::from_le_bytes(row[0..4].try_into().expect("4 bytes"));
        let kind = u32::from_le_bytes(row[4..8].try_into().expect("4 bytes"));
        let active_factories = i32::from_le_bytes(row[12..16].try_into().expect("4 bytes"));
        if tag == 0 || !matches!(kind, d11::KIND_MILITARY | d11::KIND_NAVAL) {
            return Err(format!(
                "d11 production line row {index} at byte {at_byte}: invalid tag/kind"
            ));
        }
        if !(0..=100_000).contains(&active_factories) {
            return Err(format!(
                "d11 production line row {index} at byte {at_byte}: invalid active_factories \
                 {active_factories}"
            ));
        }
    }
    let mut seen_construction: BTreeSet<(u32, u32, u32)> = BTreeSet::new();
    for (index, row) in tables
        .construction
        .chunks_exact(d11::CONSTRUCTION_ROW_LEN)
        .enumerate()
    {
        let tag = u32::from_le_bytes(row[0..4].try_into().expect("4 bytes"));
        let line_id = u32::from_le_bytes(row[4..8].try_into().expect("4 bytes"));
        let active_factories = i32::from_le_bytes(row[12..16].try_into().expect("4 bytes"));
        let cost_raw = i32::from_le_bytes(row[16..20].try_into().expect("4 bytes"));
        let produced_raw = i32::from_le_bytes(row[20..24].try_into().expect("4 bytes"));
        let priority = u32::from_le_bytes(row[24..28].try_into().expect("4 bytes"));
        let reserved = u32::from_le_bytes(row[28..32].try_into().expect("4 bytes"));
        if tag == 0
            || !(0..=100_000).contains(&active_factories)
            || cost_raw < 0
            || produced_raw < 0
            || reserved != 0
        {
            return Err(format!(
                "d11 construction row {index} at byte {at_byte}: authority predicate failed"
            ));
        }
        if !seen_construction.insert((tag, line_id, priority)) {
            return Err(format!(
                "d11 construction row {index} at byte {at_byte}: duplicate tag/line/priority"
            ));
        }
    }

    let mut scratch = State::new();
    d11::fold_payload(&mut scratch, collections, payload, STATUS)
        .map_err(|e| format!("d11 payload at byte {at_byte}: {e}"))?;
    Ok(())
}

/// One usable d11 hour: its game hour, its byte span in the source stream, and
/// how many bytes of that span the `PDQ1` suffix occupies.
struct D11Hour {
    game_hour: i32,
    ship_span: Range<usize>,
    pdq1_len: usize,
}

struct StreamScan {
    hours: Vec<D11Hour>,
    /// d11 records whose payload status flags mark the domain-hour failed
    /// (`world_fold.rs:12488`). The fold retains the accumulator and folds
    /// nothing, so the hour is not d11 data.
    failure_records: usize,
    /// d11 records that set `COMPACT_STATUS_DELTA_DATA`. `fold_production` never
    /// reads that bit — d11 has no delta grammar — so this is reported, not acted
    /// on. A non-zero count is a finding about the stream, not about this arm.
    delta_bit_records: usize,
    /// Measured d11 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// Frames carrying no country at all. 55 of `deepwar_t1`'s 903 hours are
    /// exactly this and they touch nothing.
    empty_frames: usize,
    /// `world_fold.rs:12855`: hours where `country_count` exceeded the keyed set.
    uncovered_country_hours: usize,
    /// Rows the shipping base tables carried, summed over measured hours.
    shipped_stockpile_rows: usize,
    shipped_line_rows: usize,
    shipped_construction_rows: usize,
    /// Countries the frames touched, summed — the real replacement surface.
    touched_countries: usize,
    /// `PDQ1` segment bytes over the measured hours.
    pdq1_bytes: usize,
    /// Ported `PCV1` reservation rows over the measured hours. Each is 8 bytes of
    /// the shipping arm.
    reservation_rows: usize,
    /// `PCV1` variant runs that were not ascending by `variant_id` — the runs
    /// whose order `convoy_variant_ordinal` is load-bearing for rather than
    /// merely redundant with key order. See `d11.rs`'s module docs.
    unordered_variant_runs: usize,
    /// `scale_raw` words at `+20` that were not 100_000. The fold never reads the
    /// word (`world_fold.rs` uses its own constant), so this is an observation.
    scale_raw_mismatches: usize,
}

/// Validate the whole stream and fold every usable d11 DATA record. Every
/// rejection path is an `Err`, never a silently shorter measurement and never a
/// panic.
fn scan_stream(
    stream: &[u8],
    collections: &Collections,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut scan = StreamScan {
        hours: Vec::new(),
        failure_records: 0,
        delta_bit_records: 0,
        non_contiguous: 0,
        empty_frames: 0,
        uncovered_country_hours: 0,
        shipped_stockpile_rows: 0,
        shipped_line_rows: 0,
        shipped_construction_rows: 0,
        touched_countries: 0,
        pdq1_bytes: 0,
        reservation_rows: 0,
        unordered_variant_runs: 0,
        scale_raw_mismatches: 0,
    };

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d11 DATA record of ANY status — the uniqueness rule.
    let mut last_d11_hour: Option<i32> = None;
    // Hour of the last d11 record that entered the measurement — the contiguity
    // report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;
    // The pass-1 accumulator, kept only so every payload, trailer and row is
    // checked here rather than in pass 2.
    let mut state = State::new();

    // Closed game hours and d11 DATA records of ANY status — the
    // completeness rule at the end of the scan.
    let mut closed_hours = 0usize;
    let mut d11_records = 0usize;
    let mut pos = 0usize;
    while pos < stream.len() {
        let header = parse_compact_record_header(&stream[pos..])
            .map_err(|e| format!("record at byte {pos}: {e}"))?;
        let payload_start = pos + COMPACT_RECORD_HEADER_LEN;
        let record_end = payload_start
            .checked_add(header.payload_len)
            .ok_or_else(|| format!("record at byte {pos}: length overflow"))?;
        if record_end > stream.len() {
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

        if header.domain_id == D11 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d11_records += 1;
            // `world_fold.rs:12482` checks the 40-byte header length BEFORE
            // `:12488` checks the status, so a malformed failure record is still
            // a malformed record. Nothing past the header is validated yet.
            let d11_header = d11::split_header(payload)
                .map_err(|e| format!("d11 payload at byte {pos}: {e}"))?;

            // One d11 DATA record per game hour, over EVERY d11 record and
            // BEFORE any status-based exclusion.
            if let Some(previous) = last_d11_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d11 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d11_hour = Some(header.game_hour_raw);

            // VALIDITY BEFORE CLASSIFICATION. Everything below that reads the
            // status word classifies the v1 vocabulary; none of it is a validity
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
            let unknown_status = compact_status_unknown_flags_v1(d11_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d11 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            check_authority_payload(collections, payload, pos)?;

            if compact_status_failure_flags_after_expected_partial_tolerance(
                D11,
                d11_header.status_flags,
            ) != 0
            {
                scan.failure_records += 1;
                pos = record_end;
                continue;
            }
            if d11_header.status_flags & COMPACT_STATUS_DELTA_DATA != 0 {
                scan.delta_bit_records += 1;
            }
            if d11_header.scale_raw != 100_000 {
                scan.scale_raw_mismatches += 1;
            }

            let outcome = d11::fold_payload(&mut state, collections, payload, STATUS)
                .map_err(|e| format!("d11 payload at byte {pos}: {e}"))?;

            if let Some(previous) = last_measured_hour {
                if header.game_hour_raw != previous + 1 {
                    scan.non_contiguous += 1;
                }
            }
            last_measured_hour = Some(header.game_hour_raw);
            if d11_header.country_count == 0 {
                scan.empty_frames += 1;
            }
            if outcome.uncovered_countries > 0 {
                scan.uncovered_country_hours += 1;
            }
            scan.shipped_stockpile_rows += d11_header.stockpile_count as usize;
            scan.shipped_line_rows += d11_header.line_count() as usize;
            scan.shipped_construction_rows += d11_header.construction_count as usize;
            scan.touched_countries += outcome.touched_countries;
            scan.pdq1_bytes += outcome.deployment_segment_len;
            scan.reservation_rows += outcome.reservation_rows;
            scan.unordered_variant_runs += outcome.unordered_variant_runs;
            scan.hours.push(D11Hour {
                game_hour: header.game_hour_raw,
                ship_span: pos..record_end,
                pdq1_len: outcome.deployment_segment_len,
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
        return Err("stream carries no usable d11 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d11 DATA
    // record, of any status. Production validates the payload of EVERY domain's
    // DATA record (`stream_fold.rs:593`); this harness cannot (module docs), so
    // a d11 record relabelled to another domain id — or lost outright — would
    // otherwise be silently ignored and the run would report a plausible
    // measurement over fewer hours. Per-hour uniqueness is already enforced
    // above, so equality of the two counts is exactly-one per closed hour. A
    // per-capture assertion, not a grammar rule: a capture that does not
    // schedule d11 every hour fails here loudly and a human adjusts, instead
    // of a number quietly shrinking.
    if d11_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d11_records} d11 DATA \
             record(s); every closed hour of a d11-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    Ok(scan)
}

/// The schema's own collection names, so the report cannot drift from it.
const COLLECTION_NAMES: [&str; 8] = Collections::NAMES;

fn main() {
    const USAGE: &str =
        "usage: bench_real_d11 <collector_stream.bin> <out dir> [legacy|identity-v1]";
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d11::schema();
    let collections = Collections::from_schema(&schema).expect("the built-in schema resolves");

    // ---- Pass 1: validate the WHOLE stream and fold its d11 DATA records. ----
    let scan = match scan_stream(&stream, &collections, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: re-fold the accepted spans and encode both arms. ------------

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut ship_no_pdq1 = Vec::new();
    let mut ship_no_pdq1_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut state = State::new();
    let mut prior: Option<Vec<CollectionState>> = None;
    let mut reader: Option<Vec<CollectionState>> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    // Baselines the §1.5 gap rule forced outside a scheduled block start.
    let mut gap_forced_baselines = 0usize;
    let mut previous_measured_hour: Option<i32> = None;
    let mut empty_hours = 0usize;
    let mut baseline_bytes = 0usize;
    let mut changed_rows = [0usize; 8];
    let mut tombstones = [0usize; 8];
    let mut ordered_transition_rows = [0usize; 8];
    let mut ordered_transition_events = [0usize; 8];
    let mut ordered_delta_rows = [0usize; 8];
    let mut ordered_delta_events = [0usize; 8];
    let mut transitions = 0usize;
    let mut state_rows_total = [0usize; 8];

    let collection_list: Vec<&Collection> = collections.as_slice().to_vec();

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());
        // And the same record with its PDQ1 suffix cut. PDQ1 is required to be
        // the FINAL trailer (`world_fold.rs:27418`), so the cut is a suffix.
        ship_no_pdq1
            .extend_from_slice(&stream[hour.ship_span.start..hour.ship_span.end - hour.pdq1_len]);
        ship_no_pdq1_offsets.push(ship_no_pdq1.len());

        // Re-fold this hour's record into the running accumulator, through the
        // same boundary pass 1 used.
        let payload = &stream[hour.ship_span.start + COMPACT_RECORD_HEADER_LEN..hour.ship_span.end];
        d11::fold_payload(&mut state, &collections, payload, STATUS)
            .expect("pass 1 accepted this payload");
        let current = state.states();
        for (i, rows) in current.iter().enumerate() {
            state_rows_total[i] += rows.len();
        }

        // Cadence-independent ordered replacement cost. Baseline boundaries hide
        // deltas on the wire, so count every measured transition separately.
        if let Some(prev) = prior.as_ref() {
            transitions += 1;
            for (i, collection) in collection_list.iter().enumerate() {
                match diff_shaped(collection, &prev[i], &current[i])
                    .expect("fold state matches schema")
                {
                    Some(CollectionDelta::OrderedKeyed(rows)) => {
                        ordered_transition_rows[i] += rows.len();
                        ordered_transition_events[i] += 1;
                    }
                    Some(CollectionDelta::OrderedOccurrence(rows)) => {
                        ordered_transition_rows[i] += rows.len();
                        ordered_transition_events[i] += 1;
                    }
                    Some(CollectionDelta::Keyed(_)) | None => {}
                }
            }
        }

        // New arm: BASELINE at each block start, stored-minimal DELTA between.
        let hour_start = wire.len();
        let baseline_here = opens_baseline(hours_seen, previous_measured_hour, hour.game_hour);
        if baseline_here && hours_seen % BLOCK_HOURS != 0 {
            gap_forced_baselines += 1;
        }
        let mut emitted = true;
        if baseline_here || prior.is_none() {
            encode_shaped_baseline(&schema, &current, &mut wire).expect("baseline encodes");
            baselines += 1;
            baseline_bytes += wire.len() - hour_start;
        } else {
            let prev = prior.as_ref().expect("prior");
            let deltas: Vec<_> = collection_list
                .iter()
                .enumerate()
                .map(|(i, collection)| {
                    diff_shaped(collection, &prev[i], &current[i])
                        .expect("fold state matches schema")
                })
                .collect();
            if deltas.iter().all(Option::is_none) {
                empty_hours += 1;
                emitted = false;
            } else {
                for (i, delta) in deltas.iter().enumerate() {
                    if let Some(delta) = delta {
                        match delta {
                            CollectionDelta::Keyed(delta) => {
                                changed_rows[i] += delta.changed.len();
                                tombstones[i] += delta.tombstones.len();
                            }
                            CollectionDelta::OrderedKeyed(rows) => {
                                ordered_delta_rows[i] += rows.len();
                                ordered_delta_events[i] += 1;
                            }
                            CollectionDelta::OrderedOccurrence(rows) => {
                                ordered_delta_rows[i] += rows.len();
                                ordered_delta_events[i] += 1;
                            }
                        }
                    }
                }
                encode_shaped_delta(&schema, prev, &deltas, &mut wire).expect("delta encodes");
            }
        }
        wire_offsets.push(wire.len());

        // Reader replay: decode the bytes just written against the reader's OWN
        // reconstructed prior, and require the result to be stored-equal to this
        // hour's fold. An hour that emitted nothing is still checked — the
        // reader's retained state must already equal the fold's.
        if emitted {
            let hour_bytes = &wire[hour_start..];
            if baseline_here || reader.is_none() {
                let (decoded, consumed) =
                    decode_shaped_baseline(&schema, hour_bytes, 0).expect("baseline decodes");
                assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
                assert_eq!(decoded.len(), 8, "eight collections in the body");
                reader = Some(decoded);
            } else {
                let reader_prior = reader.take().expect("reader prior");
                let (deltas, consumed) =
                    decode_shaped_delta(&schema, &reader_prior, hour_bytes, 0)
                        .expect("delta decodes");
                assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
                let mut next = reader_prior;
                for (i, delta) in deltas.iter().enumerate() {
                    if let Some(delta) = delta {
                        apply_shaped(&mut next[i], delta).expect("delta shape matches state");
                    }
                }
                reader = Some(next);
            }
        }
        let replayed = reader.as_ref().expect("reader state");
        for (i, collection) in collection_list.iter().enumerate() {
            assert_eq!(
                replayed[i].len(),
                current[i].len(),
                "game hour {}: collection {} row count",
                hour.game_hour,
                COLLECTION_NAMES[i]
            );
            assert!(
                collection_stored_eq(collection, &replayed[i], &current[i])
                    .expect("states encode"),
                "game hour {}: reader collection {} is not stored-equal to the fold",
                hour.game_hour,
                COLLECTION_NAMES[i]
            );
        }

        prior = Some(current);
        previous_measured_hour = Some(hour.game_hour);
        hours_seen += 1;
    }

    for (name, bytes) in [
        ("ship_d11.bin", &ship),
        ("ship_d11_no_pdq1.bin", &ship_no_pdq1),
        ("wire_new.bin", &wire),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d11.offsets", &ship_offsets),
        ("ship_d11_no_pdq1.offsets", &ship_no_pdq1_offsets),
        ("wire_new.offsets", &wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    let delta_hours = hours_seen - baselines - empty_hours;
    println!(
        "hours                 {hours_seen} ({} non-contiguous steps, {} carrying no country)",
        scan.non_contiguous, scan.empty_frames
    );
    println!(
        "failure-status hours  {} (excluded from both arms)",
        scan.failure_records
    );
    println!(
        "delta-bit records     {} (d11 has no delta grammar; the fold never reads the bit)",
        scan.delta_bit_records
    );
    println!(
        "scale_raw != 100000   {} (the fold never reads the word)",
        scan.scale_raw_mismatches
    );
    println!(
        "uncovered-country hrs {} (world_fold.rs:12855 badges, folds the rest)",
        scan.uncovered_country_hours
    );
    println!(
        "shipped base rows     {} stockpile, {} line, {} construction",
        scan.shipped_stockpile_rows, scan.shipped_line_rows, scan.shipped_construction_rows
    );
    println!(
        "countries touched     {} total, {:.2}/hour (the real replacement surface)",
        scan.touched_countries,
        scan.touched_countries as f64 / hours_seen as f64
    );
    println!("accumulated rows avg:");
    for (i, name) in COLLECTION_NAMES.iter().enumerate() {
        println!(
            "  {name:<20} {:>9.1}",
            state_rows_total[i] as f64 / hours_seen as f64
        );
    }
    println!(
        "baselines             {baselines} ({} every {BLOCK_HOURS} h, {gap_forced_baselines} \
         forced by a gap in measured hours — WIRE_FORMAT §1.5), {baseline_bytes} raw bytes",
        baselines - gap_forced_baselines
    );
    println!(
        "delta hours           {delta_hours}   empty hours {empty_hours}   {} raw delta bytes",
        wire.len() - baseline_bytes
    );
    println!(
        "baseline/delta split  {:.1}% baseline / {:.1}% delta of the wire arm's raw bytes",
        100.0 * baseline_bytes as f64 / wire.len() as f64,
        100.0 * (wire.len() - baseline_bytes) as f64 / wire.len() as f64
    );
    println!("real churn per delta hour (stored precision):");
    for (i, name) in COLLECTION_NAMES.iter().take(7).enumerate() {
        println!(
            "  {name:<20} {:>8.3} changed, {:>8.3} tombstones",
            changed_rows[i] as f64 / delta_hours.max(1) as f64,
            tombstones[i] as f64 / delta_hours.max(1) as f64
        );
    }
    let reservations = 7;
    println!(
        "ordered replacements convoy_priority_reservations {} events / {transitions} measured \
         transitions, {} events / {delta_hours} delta hours; {:.3} rows per measured \
         transition, {:.3} rows per delta hour",
        ordered_transition_events[reservations],
        ordered_delta_events[reservations],
        ordered_transition_rows[reservations] as f64 / transitions.max(1) as f64,
        ordered_delta_rows[reservations] as f64 / delta_hours.max(1) as f64,
    );
    println!(
        "PDQ1 bytes            {} of {} shipping bytes ({:.1}%) — stop-reported, not measured",
        scan.pdq1_bytes,
        ship.len(),
        100.0 * scan.pdq1_bytes as f64 / ship.len() as f64
    );
    println!(
        "PCV1 reservation rows {} ({} bytes, {:.2}% of shipping) — ported as ordered occurrences",
        scan.reservation_rows,
        scan.reservation_rows * 8,
        100.0 * (scan.reservation_rows * 8) as f64 / ship.len() as f64
    );
    println!(
        "unordered variant runs {} (runs where convoy_variant_ordinal is load-bearing rather \
         than redundant with key order)",
        scan.unordered_variant_runs
    );
    println!("ship_d11.bin          {} bytes", ship.len());
    println!("ship_d11_no_pdq1.bin  {} bytes", ship_no_pdq1.len());
    println!(
        "wire_new.bin          {} bytes ({:.1}% of ship, {:.1}% of ship without PDQ1)",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64,
        100.0 * wire.len() as f64 / ship_no_pdq1.len() as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{COMPACT_DOMAIN_FRAME, COMPACT_FRAME_END_PAYLOAD_LEN};

    fn scan(stream: &[u8]) -> Result<StreamScan, String> {
        let collections = Collections::default();
        scan_stream(stream, &collections, StreamIdentityMode::Legacy)
    }

    /// One H4CR record, framed the way the recorder frames it
    /// (`compact_contract/src/record_header.rs:60`).
    fn record(sequence: u64, game_hour: i32, domain: u16, kind: u16, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(COMPACT_RECORD_HEADER_LEN + payload.len());
        out.extend_from_slice(b"H4CR");
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour.to_le_bytes());
        out.extend_from_slice(&domain.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
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
        record(
            sequence,
            game_hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &payload,
        )
    }

    fn stockpile_row(tag: u32, variant_id: u32, amount_raw: i64) -> Vec<u8> {
        let mut row = Vec::new();
        row.extend_from_slice(&tag.to_le_bytes());
        row.extend_from_slice(&variant_id.to_le_bytes());
        row.extend_from_slice(&amount_raw.to_le_bytes());
        row
    }

    /// A d11 DATA payload with one stockpile table and a PCP1 trailer.
    fn d11_payload(status_flags: u32, tags: &[(u32, u32, i64)]) -> Vec<u8> {
        let mut countries: Vec<u32> = tags.iter().map(|(t, _, _)| *t).collect();
        countries.sort_unstable();
        countries.dedup();
        let mut out = Vec::new();
        out.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        out.extend_from_slice(&(countries.len() as u32).to_le_bytes());
        out.extend_from_slice(&(countries.len() as u32).to_le_bytes());
        out.extend_from_slice(&status_flags.to_le_bytes());
        out.extend_from_slice(&100_000u32.to_le_bytes());
        out.extend_from_slice(&(tags.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for (tag, variant_id, amount) in tags {
            out.extend_from_slice(&stockpile_row(*tag, *variant_id, *amount));
        }
        if !countries.is_empty() {
            out.extend_from_slice(b"PCP1");
            out.extend_from_slice(&1u16.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&((20 + countries.len() * 8) as u32).to_le_bytes());
            out.extend_from_slice(&(countries.len() as u32).to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            for tag in &countries {
                out.extend_from_slice(&tag.to_le_bytes());
                out.extend_from_slice(&7u32.to_le_bytes());
            }
        }
        out
    }

    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D11,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d11_payload(0, &[(7, 500, 1_000)]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D11,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d11_payload(0, &[(8, 600, 2_000)]),
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

    #[test]
    fn a_well_formed_stream_yields_its_d11_hours() {
        let s = scan(&good_stream()).expect("accepted");
        assert_eq!(s.hours.len(), 2);
        assert_eq!(s.hours[0].game_hour, 100);
        assert_eq!(s.hours[1].game_hour, 101);
        assert_eq!(s.failure_records, 0);
        assert_eq!(s.non_contiguous, 0);
        assert_eq!(s.shipped_stockpile_rows, 2);
        assert_eq!(s.touched_countries, 2);
        assert_eq!(s.pdq1_bytes, 0, "this stream carries no PDQ1 segment");
        assert_eq!(s.scale_raw_mismatches, 0);
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helpers classify
        // the v1 vocabulary and are not validity predicates, so an unassigned bit
        // must be rejected BEFORE classification — otherwise it silently reads as
        // "not a failure" and the hour is measured as healthy.
        // Bit 15 is ASSIGNED (`COMPACT_STATUS_BASELINE_CLEAR`, the stream's deliberate
        // resync primitive), so the UNASSIGNED run starts at 16.
        for bit in 16..32u32 {
            let status = 1u32 << bit;
            // Alone, combined with a known failure bit, and combined with a known
            // informational bit; with rows and without. The rejection must not
            // depend on any of it.
            for extra in [0u32, 1, 1 << 10] {
                for tags in [Vec::new(), vec![(7u32, 500u32, 1_000i64)]] {
                    let mut s = Vec::new();
                    s.extend(record(
                        1,
                        100,
                        D11,
                        COMPACT_RECORD_TYPE_DOMAIN_DATA,
                        &d11_payload(status | extra, &tags),
                    ));
                    s.extend(frame_end(2, 100));
                    assert!(
                        scan(&s).is_err(),
                        "status bit {bit} (+{extra:#x}) with {} row(s) must be refused",
                        tags.len()
                    );
                }
            }
        }
        // And the whole known v1 mask, minus the failure bits, is still accepted:
        // the check must reject unassigned bits and nothing else. WAR_BRIDGE_APPLIED,
        // EXTRACTION_PERF_GATE, DELTA_DATA, TRANSITION_BASELINE_RESET and
        // WAR_SEGMENT_ABSENT are informational; DELTA_DATA is the one d11 itself
        // reads, and it is counted rather than excluded.
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D11,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d11_payload(informational, &[(7, 500, 1_000)]),
            ));
            s.extend(frame_end(2, 100));
            let scan =
                scan(&s).unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 1, "bit {informational:#x}");
            assert_eq!(scan.failure_records, 0, "bit {informational:#x}");
        }
    }

    #[test]
    fn a_failure_status_record_is_excluded_and_leaves_the_accumulator_alone() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time. The
        // fold retains the accumulator and folds nothing (`world_fold.rs:12488`),
        // so the hour is not d11 data and must not enter either arm.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D11,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d11_payload(1 << bit, &[(7, 500, 1)]),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D11,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d11_payload(0, &[(8, 600, 2)]),
            ));
            s.extend(frame_end(4, 101));
            let result = scan(&s).expect("the stream itself is well-formed");
            assert_eq!(result.failure_records, 1, "bit {bit}");
            assert_eq!(result.hours.len(), 1, "bit {bit}");
            assert_eq!(result.hours[0].game_hour, 101, "bit {bit}");
        }
        // And the DELTA bit (1<<10) is not a failure bit — d11 never reads it,
        // but it must not exclude the hour either.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D11,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d11_payload(1 << 10, &[(7, 500, 1)]),
        ));
        s.extend(frame_end(2, 100));
        let result = scan(&s).expect("well-formed");
        assert_eq!(result.failure_records, 0);
        assert_eq!(result.delta_bit_records, 1);
        assert_eq!(result.hours.len(), 1);
    }

    #[test]
    fn a_failure_record_whose_counts_do_not_add_up_is_refused_by_the_payload_authority() {
        // The fold itself exits at `world_fold.rs:12488` before its `base_len`
        // check, but production's independent decoder
        // (`domain_data_authority.rs:1313`-`:1326`) runs regardless of status and
        // makes the malformed record a certification failure.
        let mut payload = d11_payload(1, &[(7, 500, 1)]);
        payload[24..28].copy_from_slice(&9_999u32.to_le_bytes());
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D11,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D11,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d11_payload(0, &[(8, 600, 2)]),
        ));
        s.extend(frame_end(4, 101));
        assert!(scan(&s).is_err());
        // A SUCCESS record with the same broken counts is also refused.
        let mut payload = d11_payload(0, &[(7, 500, 1)]);
        payload[24..28].copy_from_slice(&9_999u32.to_le_bytes());
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D11,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        assert!(scan(&s).is_err());
        // And a failure record too short even for the 40-byte header is refused,
        // because `:12482` precedes `:12488`.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D11,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &vec![0u8; 39],
        ));
        s.extend(frame_end(2, 100));
        assert!(scan(&s).is_err());
    }

    #[test]
    fn a_second_d11_record_in_one_hour_is_rejected() {
        // The accumulator is incremental across hours, so two records in one
        // hour would apply two frames to one measurement hour. The rule is
        // checked over every d11 record and before status exclusion, so all four
        // success/failure orderings are refused.
        const FAILED: u32 = 1;
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D11,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d11_payload(first, &[(7, 500, 1)]),
            ));
            s.extend(record(
                2,
                100,
                D11,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d11_payload(second, &[(7, 500, 2)]),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan(&s).is_err(),
                "two d11 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan(&full[..cut]).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary IS a well-formed shorter stream.
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
                (0usize, vec![b'X']),
                (4, 2u16.to_le_bytes().to_vec()),
                (6, 33u16.to_le_bytes().to_vec()),
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
            D11,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d11_payload(0, &[(7, 500, 1)]),
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
            D11,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d11_payload(0, &[(7, 500, 1)]),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D11,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d11_payload(0, &[(7, 500, 2)]),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan(&s).is_err());
    }

    #[test]
    fn a_stream_with_no_d11_records_is_refused_not_reported_as_zero() {
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
    fn a_malformed_d11_payload_is_refused_not_skipped() {
        let base = d11_payload(0, &[(7, 500, 1)]);
        let mut bad: Vec<Vec<u8>> = Vec::new();
        for len in [0usize, 8, 39] {
            bad.push(base[..len].to_vec());
        }
        let mut short = base.clone();
        short.pop();
        bad.push(short);
        // A trailing byte that is not a known trailer magic.
        let mut long = base.clone();
        long.push(0);
        bad.push(long);
        for offset in [24usize, 28, 32, 36] {
            let mut huge = base.clone();
            huge[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            bad.push(huge);
            let mut big = base.clone();
            big[offset..offset + 4].copy_from_slice(&1000u32.to_le_bytes());
            bad.push(big);
        }
        for (i, payload) in bad.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D11,
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
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D11,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d11_payload(0, &[(7, 500, 1)]),
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
        for sealed in bad {
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
            assert!(scan(&s).is_err(), "FRAME_END length {len}");
        }
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(data(1, 100));
            s.extend(frame_end_sealed(2, 100, 0, 0, informational, 0));
            assert!(scan(&s).is_ok(), "informational status {informational:#x}");
        }
        let mut later = Vec::new();
        later.extend(data(1, 100));
        later.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        later.extend(data(3, 101));
        later.extend(frame_end_sealed(4, 101, 0, 0, 0, 1));
        assert!(scan(&later).is_err());
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D11,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d11_payload(0, &[(7, 500, 1)]),
            )
        };
        let mut ok = Vec::new();
        ok.extend(perf(1, 100, 20));
        ok.extend(perf(2, 100, D11));
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
            s.extend(perf(2, 100, D11));
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
        assert!(scan(&two_hours).is_ok());
    }

    #[test]
    fn a_data_record_must_open_the_hour_the_pending_perf_records_bound() {
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D11,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d11_payload(0, &[(7, 500, 1)]),
            )
        };
        for (perf_hour, data_hour) in [(100i32, 101i32), (101, 100), (100, 200), (200, 100)] {
            let mut s = Vec::new();
            s.extend(perf(1, perf_hour, 20));
            s.extend(data(2, data_hour));
            s.extend(frame_end_sealed(3, data_hour, 1, 1, 0, 0));
            assert!(scan(&s).is_err());
        }
        let mut agreeing = Vec::new();
        agreeing.extend(perf(1, 100, 20));
        agreeing.extend(data(2, 100));
        agreeing.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan(&agreeing).is_ok());
    }

    #[test]
    fn an_excluded_record_is_still_checked_by_the_payload_authority() {
        // Every bad payload rides in a stream that WOULD otherwise measure: a
        // second, well-formed hour follows it. Without that trailing hour the
        // scan refuses for an unrelated reason — "no usable d11 DATA records" —
        // and the assertion would hold with `check_authority_payload` deleted,
        // which is a test that cannot fail for its own cause.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut bad = d11_payload(1 << bit, &[(7, 500, 1)]);
            bad[20..24].copy_from_slice(&1u32.to_le_bytes());
            let mut s = Vec::new();
            s.extend(record(1, 100, D11, COMPACT_RECORD_TYPE_DOMAIN_DATA, &bad));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D11,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d11_payload(0, &[(8, 600, 2)]),
            ));
            s.extend(frame_end(4, 101));
            assert!(scan(&s).is_err(), "failure bit {bit}");
        }

        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D11,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d11_payload(1, &[(7, 500, 1)]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D11,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d11_payload(0, &[(8, 600, 2)]),
        ));
        s.extend(frame_end(4, 101));
        let scanned = scan(&s).expect("well-formed excluded record");
        assert_eq!(scanned.failure_records, 1);
        assert_eq!(scanned.hours.len(), 1);

        // The authority places no range on the stockpile amount word.
        let mut outside = Vec::new();
        outside.extend(record(
            1,
            100,
            D11,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d11_payload(1, &[(7, 500, i64::MIN)]),
        ));
        outside.extend(frame_end(2, 100));
        outside.extend(record(
            3,
            101,
            D11,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d11_payload(0, &[(8, 600, 2)]),
        ));
        outside.extend(frame_end(4, 101));
        assert!(scan(&outside).is_ok());
    }

    #[test]
    fn the_identity_protocol_is_stated_not_sniffed() {
        use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;

        let collections = Collections::default();
        let decapitated = good_stream();
        assert!(
            super::scan_stream(&decapitated, &collections, StreamIdentityMode::IdentityV1).is_err()
        );
        assert!(super::scan_stream(&decapitated, &collections, StreamIdentityMode::Legacy).is_ok());

        let mut identity_stream = Vec::new();
        identity_stream.extend(record(
            0,
            0,
            0,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            b"CID1-not-a-real-identity-payload",
        ));
        identity_stream.extend(good_stream());
        assert!(
            super::scan_stream(&identity_stream, &collections, StreamIdentityMode::Legacy).is_err()
        );
    }

    #[test]
    fn scan_stream_never_panics_on_arbitrary_bytes() {
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
            let _ = scan(&bytes);
        }
        let good = good_stream();
        for i in 0..good.len() {
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
    fn an_hour_closed_without_a_d11_record_refuses_the_stream() {
        // The completeness bound on what this harness does NOT validate (module
        // docs): it cannot check other domains' payloads, so a d11 record
        // relabelled to another domain id — or dropped — must be caught by
        // counting, not silently measured as fewer hours.
        let mut s = good_stream();
        // Find the LAST d11 DATA record and relabel it to an unclaimed domain
        // id. The stream stays well formed — sequence, hour boundaries and
        // FRAME_END are untouched — so the completeness rule is the only rule
        // that can catch it.
        let mut pos = 0usize;
        let mut target = None;
        while pos < s.len() {
            let header = parse_compact_record_header(&s[pos..]).expect("well-formed");
            if header.domain_id == D11 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
            {
                target = Some(pos);
            }
            pos += COMPACT_RECORD_HEADER_LEN + header.payload_len;
        }
        let target = target.expect("good_stream carries a d11 DATA record");
        // domain_id sits at +20: magic(4) + version(2) + header_len(2) +
        // sequence(8) + game_hour(4).
        s[target + 20..target + 22].copy_from_slice(&99u16.to_le_bytes());
        let err = match scan(&s) {
            Err(e) => e,
            Ok(_) => panic!("an hour with no d11 record must refuse the stream"),
        };
        assert!(
            err.contains("game hour(s) but carries"),
            "the completeness check must be what rejects it: {err}"
        );
    }
}
