//! Measure the wire format against the shipping compact stream on REAL d13
//! `tech_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d13-scheduled capture
//! (`runs/batch1/deepwar_t1`). The stream's own d13 DATA records are the
//! shipping format's cost for the domain: a 32-byte header, a fixed 16-byte row
//! per country, then one **variable-length** row per occupied research slot,
//! full-replace every hour, exactly what the recorder stores today. This tool
//! replays those records into per-hour states — the same fold
//! `world_fold::fold_tech` performs — and re-encodes every hour with this
//! crate's grammar: a BASELINE at each 48-hour block start, field-granular
//! stored-minimal DELTAs between.
//!
//! **Nothing here re-derives a contract it can call.** Payload, row and frame
//! decoding go through `wire_format::d13`'s public boundary — including the
//! cross-collection rule that an active row must name a country the same frame
//! carries (`world_fold.rs:13564`), which is why the frame fold lives there and
//! not here. H4CR framing, record semantics, sequence continuity and the
//! failure-status predicate go through `compact_contract`'s own parser, stateful
//! validator and helper.
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
//!   (`recorder_host/src/stream_fold.rs:277`), its DOMAIN_PERF hour binding
//!   (`:328`-`:365`), and the DATA arm's pending-PERF rule (`:378`-`:386`);
//! - FRAME_END seal fields (`stream_fold.rs:46`-`:89`) and their distinct
//!   non-probe PERF-domain count cross-check (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END;
//! - every d13 payload through `d13::state_from_payload_at_hour`, so a row the
//!   recorder would reject stops the scan rather than unwinding later.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority. `validate_domain_data_payload_at` (`stream_fold.rs:594`)
//! invokes its status-independent d13 decoder
//! (`domain_data_authority.rs:2749`-`:3026`) adds predicates the fold boundary
//! does not: both scale words, `changed_rows <= country_count`, nonzero and
//! unique country tags, country flag and slot-count bounds, and nonzero active
//! tags plus slot-index bounds. [`check_authority_payload`] applies exactly those
//! missing predicates to every status. Active-row structure, flags, duplicate
//! `(tag, slot)`, optional-field packing, text, quartet, A29, and trailing-byte
//! predicates already run before exclusion through the domain module and are not
//! re-derived. d13's independent pre-existing policy refuses every row-bearing
//! failure record because `fold_tech` would apply it, so an excluded row cannot
//! serve as an accepted example of a fold-only row rule.
//!
//! The seal, PERF binding, pending-PERF, payload-authority, and stated identity
//! checks are the five-check sweep. Production receives identity mode from its
//! caller (`stream_fold.rs:220`); the measured capture is legacy, which remains
//! `main`'s default.
//!
//! **The status decision, made explicitly.** `fold_tech` (`world_fold.rs:13301`)
//! never reads `status_flags` and has no `handle_compact_failure` call, unlike
//! `fold_country_core_scalars` (`:9252`). So it does NOT retain the accumulator
//! on a failure status — it folds a failure-masked payload as authoritative, and
//! d13 is full-replace, so that payload *replaces* every country's `research`
//! object. Excluding such a record is therefore equivalent to the fold only when
//! the record carries nothing to apply. This harness, like `bench_real_d10`:
//!
//! - refuses outright a failure-status record with any country or active row —
//!   dropping it would leave the measured state permanently behind the fold's,
//!   and folding it would contradict the status word the recorder publishes;
//! - counts and skips an empty one, which applies nothing under either reading.
//!
//! That also closes the second-order hole: with row-bearing failure records
//! refused, every row in an accepted stream has been through the mapper. The
//! reference capture has 0 failure-status records, so none of this affects its
//! numbers.
//!
//! **Two passes**, so peak memory is two states rather than 903: pass 1 validates
//! the whole stream (folding each payload into a scratch state that is then
//! dropped) and retains only record spans; pass 2 re-folds through the same
//! `d13::state_from_payload_at_hour` while encoding.
//!
//! Outputs, for the compression step (`scripts/compress_arms.py`):
//!   ship_d13.bin   — the d13 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface (changed rows per hour at stored
//! precision), the wire arm's baseline/delta raw split, and the header words the
//! fold never reads.
//!
//! **Only d13 DATA payloads are validated.** Production runs
//! `validate_domain_data_payload_at` on the DATA payload of EVERY domain
//! (`stream_fold.rs:593`); this harness cannot, because validating another
//! domain's payload means owning that domain's authority, and `wire_format`
//! must not depend on `recorder_host`. Every DATA record of any other domain
//! therefore passes with its H4CR header, sequence continuity, hour-boundary,
//! PERF-binding and FRAME_END rules checked but its **payload bytes unread**.
//! The concrete hazard — a d13 record relabelled to another domain id, or
//! dropped outright, silently shrinking the measurement — is closed by an
//! explicit completeness check in `scan_stream`: every closed game hour must
//! carry exactly one d13 DATA record, of any status. That is a per-capture
//! assertion, not a grammar rule; a capture where d13 is not scheduled every
//! hour fails loudly there and a human adjusts.

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
    apply, check_minimal, decode_baseline, decode_delta, diff, encode_baseline, encode_delta,
    KeyedDelta, KeyedState,
};
use wire_format::d13::{self, COLLECTION_COUNT};
use wire_format::Schema;

const D13: u16 = 13;
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

/// One usable d13 hour: its game hour, the byte span of the whole record in the
/// source stream (the shipping arm is those bytes verbatim, header included) and
/// the byte span of its payload (what pass 2 re-folds).
struct D13Frame {
    game_hour: i32,
    ship_span: Range<usize>,
    payload_span: Range<usize>,
}

struct StreamScan {
    frames: Vec<D13Frame>,
    /// d13 records whose payload status flags mark the domain-hour failed. Only
    /// row-less ones reach this counter; a row-bearing one refuses the stream.
    failure_records: usize,
    /// Measured d13 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// The header words `fold_tech` never reads, reported rather than enforced.
    scale_words: BTreeSet<(u32, u32)>,
    status_words: BTreeSet<u32>,
    changed_rows_disagreements: usize,
    country_rows_total: usize,
    active_rows_total: usize,
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

/// Authority-only d13 predicates from
/// `decode_compact_tech_data_payload`
/// (`domain_data_authority.rs:2749`-`:3026`). The caller has already run
/// `d13::split_payload`; row boundary discovery stays in the domain module.
fn check_authority_payload(
    header: &d13::PayloadHeader,
    tables: d13::Tables<'_>,
    at_byte: usize,
) -> Result<(), String> {
    if header.progress_scale_raw != d13::TECH_PROGRESS_SCALE_RAW {
        return Err(format!(
            "d13 authority at byte {at_byte}: progress scale {} != {}",
            header.progress_scale_raw,
            d13::TECH_PROGRESS_SCALE_RAW
        ));
    }
    if header.live_scale_raw != d13::TECH_LIVE_SCALE_RAW {
        return Err(format!(
            "d13 authority at byte {at_byte}: live scale {} != {}",
            header.live_scale_raw,
            d13::TECH_LIVE_SCALE_RAW
        ));
    }
    if header.changed_rows > header.country_count {
        return Err(format!(
            "d13 authority at byte {at_byte}: changed_rows {} > country_count {}",
            header.changed_rows, header.country_count
        ));
    }

    let mut seen_countries = BTreeSet::new();
    let country_known_flags = d13::COUNTRY_FLAG_COMPLETED_TECH_COUNT
        | d13::COUNTRY_FLAG_RESEARCH_SLOT_COUNT
        | d13::COUNTRY_FLAG_ACTIVE_RESEARCH;
    for (row_index, row) in tables
        .countries
        .chunks_exact(d13::COUNTRY_ROW_LEN)
        .enumerate()
    {
        let key = d13::key_from_country_row(row)
            .map_err(|e| format!("d13 authority country row {row_index}: {e}"))?;
        let tag = key.0[0];
        if tag == 0 {
            return Err(format!(
                "d13 authority country row {row_index} at byte {at_byte}: zero tag_db_id"
            ));
        }
        if !seen_countries.insert(tag) {
            return Err(format!(
                "d13 authority country row {row_index} at byte {at_byte}: duplicate tag_db_id {tag}"
            ));
        }
        let flags = u32::from_le_bytes(row[4..8].try_into().expect("4 bytes"));
        if flags & !country_known_flags != 0 {
            return Err(format!(
                "d13 authority country row {row_index} at byte {at_byte}: unknown flags {flags:#x}"
            ));
        }
        let slot_count = u32::from_le_bytes(row[12..16].try_into().expect("4 bytes"));
        if slot_count != u32::MAX && slot_count > 32 {
            return Err(format!(
                "d13 authority country row {row_index} at byte {at_byte}: invalid slot_count \
                 {slot_count}"
            ));
        }
    }

    for (row_index, span) in d13::active_row_spans(tables.active, header.active_count)?
        .into_iter()
        .enumerate()
    {
        let key = d13::key_from_active_row(&tables.active[span])
            .map_err(|e| format!("d13 authority active row {row_index}: {e}"))?;
        if key.0[0] == 0 {
            return Err(format!(
                "d13 authority active row {row_index} at byte {at_byte}: zero tag_db_id"
            ));
        }
        if key.0[1] > 32 {
            return Err(format!(
                "d13 authority active row {row_index} at byte {at_byte}: invalid slot_index {}",
                key.0[1]
            ));
        }
    }
    Ok(())
}

/// Validate the whole stream against the canonical contract and record the
/// usable d13 DATA records. Every rejection path is an `Err`, never a silently
/// shorter measurement and never a panic — see the module docs for what is
/// enforced and what the recorder's own validator additionally checks.
fn scan_stream(
    stream: &[u8],
    schema: &Schema,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut scan = StreamScan {
        frames: Vec::new(),
        failure_records: 0,
        non_contiguous: 0,
        scale_words: BTreeSet::new(),
        status_words: BTreeSet::new(),
        changed_rows_disagreements: 0,
        country_rows_total: 0,
        active_rows_total: 0,
    };

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d13 DATA record of ANY status — the uniqueness rule.
    let mut last_d13_hour: Option<i32> = None;
    // Hour of the last d13 record that entered the measurement — the contiguity
    // report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;

    // Closed game hours and d13 DATA records of ANY status — the
    // completeness rule at the end of the scan.
    let mut closed_hours = 0usize;
    let mut d13_records = 0usize;
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

        if header.domain_id == D13 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d13_records += 1;
            let (d13_header, d13_tables) = d13::split_payload(payload)
                .map_err(|e| format!("d13 payload at byte {pos}: {e}"))?;

            // One d13 DATA record per game hour, checked over EVERY d13 record
            // and BEFORE any status-based exclusion. d13 is full-replace, so a
            // second record for the same hour would become a second measurement
            // hour built from the same clock time — and gating this on the
            // record being usable would let a success/failure pair through the
            // rule entirely.
            if let Some(previous) = last_d13_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d13 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d13_hour = Some(header.game_hour_raw);

            // VALIDITY BEFORE CLASSIFICATION, at the first read of the status
            // word. Everything below that reads it — the status-word census, the
            // failure predicate — classifies the v1 vocabulary; none of it is a
            // validity predicate. `compact_contract/src/lib.rs:121`-`:123` says
            // so in as many words: "Callers handling untrusted status words must
            // first reject a non-zero `compact_status_unknown_flags_v1` result;
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
            let unknown_status = compact_status_unknown_flags_v1(d13_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d13 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            check_authority_payload(&d13_header, d13_tables, pos)?;

            scan.scale_words
                .insert((d13_header.progress_scale_raw, d13_header.live_scale_raw));
            scan.status_words.insert(d13_header.status_flags);
            if d13_header.changed_rows != d13_header.country_count {
                scan.changed_rows_disagreements += 1;
            }

            // `fold_tech` refuses a frame whose A29 provenance block fails a
            // ~20-clause predicate (`world_fold.rs:13491`-`:13521`). Two of those
            // clauses recompute SHA-256 digests through `recorder_host` internals
            // this crate cannot reach, so `wire_format::d13` enforces eighteen and
            // says so. Enforcing eighteen is not enough for a harness whose whole
            // job is to refuse streams the recorder would refuse: one flipped bit
            // in `selected_present_receipt_sha256` passes all eighteen. So a
            // stream carrying ANY provenance block is refused outright rather than
            // measured on an unverifiable receipt. The reference capture carries
            // none, in any of its 364,487 active rows.
            if d13::payload_carries_a29_provenance(payload)
                .map_err(|e| format!("d13 payload at byte {pos}: {e}"))?
            {
                return Err(format!(
                    "d13 DATA at byte {pos}: hour {} carries an A29 provenance block, and two \
                     clauses of the fold's acceptance predicate (world_fold.rs:13517, :13520) \
                     recompute SHA-256 digests this crate cannot; measuring it would report a \
                     number for a stream the recorder may refuse",
                    header.game_hour_raw
                ));
            }

            // `fold_tech` never reads the status word, so EVERY d13 payload is
            // decoded and structurally checked here, whatever its status —
            // including a declared-rowless one, whose only remaining rule is the
            // fold's exact-consumption check (`world_fold.rs:13571`). Deferring
            // that behind the status branch let a failure record with a trailing
            // byte through. The frame's own `game_hour_raw` is the A29 gate's
            // clock, and the production dispatcher passes the same value
            // (`:4789`). The state is dropped; pass 2 rebuilds it through this
            // same call.
            let mut scratch = d13::TechState::new();
            d13::fold_payload_at_hour(schema, &mut scratch, payload, Some(header.game_hour_raw))
                .map_err(|e| format!("d13 payload at byte {pos}: {e}"))?;

            // The status decision. `compact_contract`'s own predicate, never a
            // copied bit mask: it already applies d13's lane-local mask and
            // expected partial tolerance, so it cannot drift from the recorder.
            // See the module docs for why a row-bearing failure record is
            // refused rather than skipped.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D13,
                d13_header.status_flags,
            ) != 0
            {
                if d13_header.country_count != 0 || d13_header.active_count != 0 {
                    return Err(format!(
                        "d13 DATA at byte {pos}: hour {} has failure status {:#x} and carries \
                         {} country and {} active row(s); `fold_tech` would apply them, so the \
                         hour can be neither measured nor skipped without diverging from the fold",
                        header.game_hour_raw,
                        d13_header.status_flags,
                        d13_header.country_count,
                        d13_header.active_count
                    ));
                }
                // A row-less payload merges nothing (`fold_payload_at_hour` has no
                // carried tag to upsert), so skipping it and folding it are the
                // same state. It is counted and skipped.
                scan.failure_records += 1;
            } else {
                if let Some(previous) = last_measured_hour {
                    if header.game_hour_raw != previous + 1 {
                        scan.non_contiguous += 1;
                    }
                }
                last_measured_hour = Some(header.game_hour_raw);
                scan.country_rows_total += scratch.get(d13::COUNTRIES_INDEX).len();
                scan.active_rows_total += scratch.get(d13::ACTIVE_RESEARCH_INDEX).len();

                scan.frames.push(D13Frame {
                    game_hour: header.game_hour_raw,
                    ship_span: pos..record_end,
                    payload_span: payload_start..record_end,
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
    if scan.frames.is_empty() {
        return Err("stream carries no usable d13 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d13 DATA
    // record, of any status. Production validates the payload of EVERY domain's
    // DATA record (`stream_fold.rs:593`); this harness cannot (module docs), so
    // a d13 record relabelled to another domain id — or lost outright — would
    // otherwise be silently ignored and the run would report a plausible
    // measurement over fewer hours. Per-hour uniqueness is already enforced
    // above, so equality of the two counts is exactly-one per closed hour. A
    // per-capture assertion, not a grammar rule: a capture that does not
    // schedule d13 every hour fails here loudly and a human adjusts, instead
    // of a number quietly shrinking.
    if d13_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d13_records} d13 DATA \
             record(s); every closed hour of a d13-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    Ok(scan)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args
        .next()
        .expect("usage: bench_real_d13 <collector_stream.bin> <out dir> [legacy|identity-v1]");
    let out_dir = args
        .next()
        .expect("usage: bench_real_d13 <collector_stream.bin> <out dir> [legacy|identity-v1]");
    let identity_mode = match args.next().as_deref().unwrap_or("legacy") {
        "legacy" => StreamIdentityMode::Legacy,
        "identity-v1" => StreamIdentityMode::IdentityV1,
        other => panic!("unknown identity mode {other:?}; expected legacy or identity-v1"),
    };
    assert!(args.next().is_none(), "too many arguments");
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d13::schema();

    // ---- Pass 1: validate the WHOLE stream and fold its d13 DATA records. ---
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

    // ---- Pass 2: encode both arms. Nothing here can fail on stream content:
    // pass 1 already accepted every payload through the same fold. ------------

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut acc = d13::TechState::new();
    let mut reader: Vec<KeyedState> = vec![KeyedState::new(); COLLECTION_COUNT];
    let mut acc_country_rows_total = 0usize;
    let mut acc_active_rows_total = 0usize;
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
    let mut changed_by_collection = [0usize; COLLECTION_COUNT];
    let mut tombstones_by_collection = [0usize; COLLECTION_COUNT];

    for frame in &scan.frames {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[frame.ship_span.clone()]);
        ship_offsets.push(ship.len());

        // d13 upserts by country, so an hour's state is the fold of every usable
        // record up to and including this one — see `wire_format::d13::TechState`.
        // `prior` is what both the encoder and the reader hold.
        let prior = acc.clone();
        d13::fold_payload_at_hour(
            &schema,
            &mut acc,
            &stream[frame.payload_span.clone()],
            Some(frame.game_hour),
        )
        .expect("pass 1 accepted this payload");
        let state = &acc;
        acc_country_rows_total += state.get(d13::COUNTRIES_INDEX).len();
        acc_active_rows_total += state.get(d13::ACTIVE_RESEARCH_INDEX).len();

        // New arm: BASELINE at each block start, stored-minimal DELTA between.
        let hour_start = wire.len();
        let baseline_here = opens_baseline(hours_seen, previous_measured_hour, frame.game_hour);
        if baseline_here && hours_seen % BLOCK_HOURS != 0 {
            gap_forced_baselines += 1;
        }
        let mut emitted = true;
        if baseline_here {
            encode_baseline(&schema, state.as_slice(), &mut wire).expect("baseline encodes");
            baselines += 1;
            baseline_bytes += wire.len() - hour_start;
        } else {
            let deltas: Vec<Option<KeyedDelta>> = (0..COLLECTION_COUNT)
                .map(|i| {
                    let d = diff(&schema.collections[i], prior.get(i), state.get(i));
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
                    changed_by_collection[i] += delta.changed.len();
                    tombstones_by_collection[i] += delta.tombstones.len();
                    // §7.1 minimality, checked rather than assumed: `diff` is
                    // supposed to produce the one minimal delta, and this is the
                    // rule that says so.
                    check_minimal(&schema.collections[i], prior.get(i), state.get(i), delta)
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
                state.get(i).len(),
                "game hour {}: collection {} row count",
                frame.game_hour,
                schema.collections[i].collection_id
            );
            assert!(
                diff(&schema.collections[i], &reader[i], state.get(i)).is_empty(),
                "game hour {}: collection {} is not stored-equal to the fold",
                frame.game_hour,
                schema.collections[i].collection_id
            );
        }

        previous_measured_hour = Some(frame.game_hour);
        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d13.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d13.offsets", &ship_offsets),
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
        "failure-status hours {} (row-less, excluded from both arms)",
        scan.failure_records
    );
    println!(
        "header words         scale pairs {:?}, status words {:?}, changed_rows != country_count in {} record(s)",
        scan.scale_words, scan.status_words, scan.changed_rows_disagreements
    );
    println!(
        "payload rows avg     {:.1} countries, {:.1} active-research",
        scan.country_rows_total as f64 / hours_seen as f64,
        scan.active_rows_total as f64 / hours_seen as f64
    );
    println!(
        "accumulated rows avg {:.1} countries, {:.1} active-research (a country the payload \
         omits is retained, `d13::TechState`)",
        acc_country_rows_total as f64 / hours_seen as f64,
        acc_active_rows_total as f64 / hours_seen as f64
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
    for (i, name) in ["countries", "active_research"].into_iter().enumerate() {
        println!(
            "  {name:<16}   {:.2} changed rows/hour, {:.3} tombstones/hour",
            changed_by_collection[i] as f64 / delta_hours.max(1) as f64,
            tombstones_by_collection[i] as f64 / delta_hours.max(1) as f64
        );
    }
    println!("ship_d13.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.1}%)  baseline {} + delta {}",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64,
        baseline_bytes,
        delta_bytes
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{COMPACT_DOMAIN_FRAME, COMPACT_FRAME_END_PAYLOAD_LEN};

    fn test_schema() -> Schema {
        d13::schema()
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

    /// A one-country, one-slot d13 DATA payload with the flag shape the reference
    /// capture carries (country `0b111`, active `0b11`).
    fn d13_payload(status_flags: u32, tag: u32, progress: u32) -> Vec<u8> {
        d13_payload_with_counts(status_flags, tag, progress, 1, 1)
    }

    fn d13_payload_with_counts(
        status_flags: u32,
        tag: u32,
        progress: u32,
        countries: u32,
        actives: u32,
    ) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&countries.to_le_bytes());
        p.extend_from_slice(&countries.to_le_bytes()); // changed_rows
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&1000u32.to_le_bytes());
        p.extend_from_slice(&100_000u32.to_le_bytes());
        p.extend_from_slice(&actives.to_le_bytes());
        assert_eq!(p.len(), d13::HEADER_LEN);
        for _ in 0..countries {
            p.extend_from_slice(&tag.to_le_bytes());
            p.extend_from_slice(&0b111u32.to_le_bytes());
            p.extend_from_slice(&21u32.to_le_bytes()); // completed_tech_count
            p.extend_from_slice(&3u32.to_le_bytes()); // research_slot_count
        }
        for slot in 0..actives {
            p.extend_from_slice(&tag.to_le_bytes());
            p.extend_from_slice(&slot.to_le_bytes());
            p.extend_from_slice(&0x1234u32.to_le_bytes()); // tech_handle_raw
            p.extend_from_slice(&0x1234u32.to_le_bytes()); // tech_def_index_raw
            p.extend_from_slice(&progress.to_le_bytes());
            p.extend_from_slice(&0b11u32.to_le_bytes());
            p.extend_from_slice(&1_000_000i64.to_le_bytes()); // live_progress_raw
            p.extend_from_slice(&100_000i64.to_le_bytes()); // base_required_points_raw
        }
        p
    }

    /// Two closed hours, each carrying one d13 DATA record.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D13,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d13_payload(0, 7, 10),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D13,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d13_payload(0, 7, 11),
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
    fn a_well_formed_stream_yields_its_d13_hours() {
        let scan = scan_stream(&good_stream(), &test_schema(), StreamIdentityMode::Legacy)
            .expect("accepted");
        assert_eq!(scan.frames.len(), 2);
        assert_eq!(scan.frames[0].game_hour, 100);
        assert_eq!(scan.frames[1].game_hour, 101);
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.non_contiguous, 0);
        assert_eq!(scan.country_rows_total, 2);
        assert_eq!(scan.active_rows_total, 2);
        assert_eq!(scan.scale_words.len(), 1);
        assert_eq!(scan.changed_rows_disagreements, 0);
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
                for (countries, actives) in [(0u32, 0u32), (1, 1)] {
                    let mut s = Vec::new();
                    s.extend(record(
                        1,
                        100,
                        D13,
                        COMPACT_RECORD_TYPE_DOMAIN_DATA,
                        &d13_payload_with_counts(status | extra, 7, 10, countries, actives),
                    ));
                    s.extend(frame_end(2, 100));
                    assert!(
                        scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err(),
                        "status bit {bit} (+{extra:#x}) with {countries} country row(s) must be \
                         refused"
                    );
                }
            }
        }
        // And the whole known v1 mask, minus the failure bits, is still accepted:
        // the check must reject unassigned bits and nothing else. WAR_BRIDGE_APPLIED,
        // EXTRACTION_PERF_GATE, DELTA_DATA, TRANSITION_BASELINE_RESET and
        // WAR_SEGMENT_ABSENT are informational, and `fold_tech` never reads the
        // status word at all.
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D13,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d13_payload(informational, 7, 10),
            ));
            s.extend(frame_end(2, 100));
            let scan = scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scan.frames.len(), 1, "bit {informational:#x}");
            assert_eq!(scan.failure_records, 0, "bit {informational:#x}");
        }
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        // A cut immediately after a d13 DATA record leaves that hour open and
        // ends the stream on a record that is not a FRAME_END.
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&full[..cut], &test_schema(), StreamIdentityMode::Legacy).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary is a different thing: that IS a
        // well-formed shorter stream, and `eof_check` accepts it too.
        for (records, expected_hours) in [(2usize, 1usize), (4, 2)] {
            let cut = boundary(&full, records);
            let scan = scan_stream(&full[..cut], &test_schema(), StreamIdentityMode::Legacy)
                .expect("closed stream is well-formed");
            assert_eq!(scan.frames.len(), expected_hours, "cut after {records}");
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
                scan_stream(&full[..cut], &test_schema(), StreamIdentityMode::Legacy).is_err(),
                "mid-record cut at {cut}"
            );
        }
        assert!(
            scan_stream(&[], &test_schema(), StreamIdentityMode::Legacy).is_err(),
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
                    scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err(),
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
                    scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err(),
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
            D13,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d13_payload(0, 7, 10),
        ));
        s.extend(frame_end(2, 999)); // closes an hour that is not open
        assert!(scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err());

        let mut s = Vec::new();
        s.extend(frame_end(1, 100));
        assert!(scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            101,
            D13,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d13_payload(0, 7, 10),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D13,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d13_payload(0, 7, 11),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_second_d13_record_in_one_hour_is_rejected() {
        // d13 is full-replace; two records in one hour would become two
        // measurement hours built from the same clock time. The rule is checked
        // over EVERY d13 DATA record and before status-based exclusion, so all
        // four success/failure orderings are refused.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            let empty = |status| d13_payload_with_counts(status, 7, 10, 0, 0);
            let full = |status| d13_payload(status, 7, 10);
            let pick = |status: u32| {
                if status == 0 {
                    full(status)
                } else {
                    empty(status)
                }
            };
            s.extend(record(
                1,
                100,
                D13,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &pick(first),
            ));
            s.extend(record(
                2,
                100,
                D13,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &pick(second),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err(),
                "two d13 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn an_empty_failure_status_record_is_excluded_and_a_row_bearing_one_refuses_the_stream() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time.
        // `fold_tech` never reads the status word, so a row-bearing failure
        // record is neither measurable nor skippable — see the module docs.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D13,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d13_payload_with_counts(1 << bit, 7, 10, 0, 0),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D13,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d13_payload(0, 7, 11),
            ));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy)
                .expect("the stream itself is well-formed");
            assert_eq!(scan.failure_records, 1, "bit {bit}");
            assert_eq!(scan.frames.len(), 1, "bit {bit}");
            assert_eq!(scan.frames[0].game_hour, 101, "bit {bit}");

            // The same status with rows present refuses the stream outright.
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D13,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d13_payload(1 << bit, 7, 10),
            ));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err(),
                "row-bearing failure, bit {bit}"
            );
        }
        // And a non-failure bit (DELTA_DATA, 1<<10) is NOT treated as failure.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D13,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d13_payload(1 << 10, 7, 10),
        ));
        s.extend(frame_end(2, 100));
        let scan =
            scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.frames.len(), 1);
    }

    #[test]
    fn a_stream_with_no_d13_records_is_refused_not_reported_as_zero() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &[0u8; 8],
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_malformed_d13_payload_is_refused_not_skipped() {
        // A short payload is a corrupt stream, not an hour to drop quietly.
        let mut payload = d13_payload(0, 7, 10);
        payload.truncate(payload.len() - 1);
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D13,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err());

        // A row the FOLD rejects must make the SCAN fail, not be accepted and
        // then unwind while measuring. One case per rejection class.
        let mut invalid: Vec<Vec<u8>> = Vec::new();
        // An unknown active-row flag bit (`world_fold.rs:13332`).
        let mut unknown_flag = d13_payload(0, 7, 10);
        let at = d13::HEADER_LEN + d13::COUNTRY_ROW_LEN + 20;
        unknown_flag[at..at + 4].copy_from_slice(&(0b11u32 | (1 << 20)).to_le_bytes());
        invalid.push(unknown_flag);
        // A gated-off value carrying a non-zero word (`:13354`).
        let mut ungated = d13_payload(0, 7, 10);
        ungated[at..at + 4].copy_from_slice(&0b10u32.to_le_bytes());
        invalid.push(ungated);
        // An active row naming a country the frame does not carry (`:13564`).
        let mut unknown_tag = d13_payload(0, 7, 10);
        let tag_at = d13::HEADER_LEN + d13::COUNTRY_ROW_LEN;
        unknown_tag[tag_at..tag_at + 4].copy_from_slice(&999u32.to_le_bytes());
        invalid.push(unknown_tag);
        // A country count word without its capture flag and without the sentinel.
        let mut bad_country = d13_payload(0, 7, 10);
        bad_country[d13::HEADER_LEN + 4..d13::HEADER_LEN + 8]
            .copy_from_slice(&0b110u32.to_le_bytes());
        invalid.push(bad_country);
        for (i, payload) in invalid.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D13,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err(),
                "invalid payload {i} must make the scan fail"
            );
        }

        // A (tag, slot_index) repeated within ONE frame is a rejected frame: the
        // fold pushes both rows onto its array and a keyed set holds one.
        let mut payload = d13_payload(0, 7, 10);
        let row: Vec<u8> = payload[d13::HEADER_LEN + d13::COUNTRY_ROW_LEN..].to_vec();
        payload.extend_from_slice(&row);
        payload[28..32].copy_from_slice(&2u32.to_le_bytes()); // active_count
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D13,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        assert!(
            scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err(),
            "duplicate (tag, slot_index)"
        );
    }

    /// A payload whose single active row carries the ahead-of-time quartet and a
    /// **valid** A29 provenance block — one that passes every clause
    /// `wire_format::d13` can evaluate. It is still refused, and that is the
    /// point: the two digest clauses (`world_fold.rs:13517`, `:13520`) are not
    /// evaluable here, so the harness must not measure the stream.
    fn d13_payload_with_a29(game_hour: i32) -> Vec<u8> {
        let mut p = d13_payload(0, 7, 10);
        // Turn the single active row's flags into 0b1_1011 and append the tails.
        let flags_at = d13::HEADER_LEN + d13::COUNTRY_ROW_LEN + 20;
        p[flags_at..flags_at + 4].copy_from_slice(&0b1_1011u32.to_le_bytes());
        for v in [88_000i64, 500_000, 25_000, 375_000] {
            p.extend_from_slice(&v.to_le_bytes());
        }
        let mut block = Vec::new();
        block.extend_from_slice(&3u32.to_le_bytes()); // authority
        block.extend_from_slice(&2u32.to_le_bytes()); // branch (reduction > 0)
        block.extend_from_slice(&9u64.to_le_bytes()); // callback_sequence
        block.extend_from_slice(&1234u32.to_le_bytes()); // callback_thread_id
        block.extend_from_slice(&game_hour.to_le_bytes());
        block.extend_from_slice(&500u64.to_le_bytes()); // callback_qpc_ticks
        block.extend_from_slice(&10_000_000u64.to_le_bytes()); // qpc_frequency_hz
        block.extend_from_slice(&[0xAB; 32]); // source_record_sha256
        block.extend_from_slice(&d13::A29_UI_YEAR_HELPER_RETURN_RVA.to_le_bytes());
        block.extend_from_slice(&3u32.to_le_bytes()); // candidate_count
        block.extend_from_slice(&1u32.to_le_bytes()); // selected_ordinal
        block.extend_from_slice(&game_hour.to_le_bytes());
        block.extend_from_slice(&77u64.to_le_bytes()); // selected_candidate_id
        block.extend_from_slice(&600u64.to_le_bytes()); // selected_qpc_ticks
        block.extend_from_slice(&700u64.to_le_bytes()); // cutoff
        block.extend_from_slice(&[0xCD; 32]); // candidate_set_sha256
        block.extend_from_slice(&[0xEF; 32]); // selected_present_receipt_sha256
        assert_eq!(block.len(), d13::A29_PROVENANCE_LEN);
        p.extend_from_slice(&block);
        p
    }

    #[test]
    fn a_stream_carrying_an_a29_provenance_block_is_refused_not_measured() {
        // Eighteen of the fold's twenty A29 clauses are checkable here and this
        // block passes all eighteen, so the mapper accepts the row...
        let payload = d13_payload_with_a29(100);
        let schema = test_schema();
        let mut acc = d13::TechState::new();
        d13::fold_payload_at_hour(&schema, &mut acc, &payload, Some(100))
            .expect("the row passes every clause this crate can evaluate");
        assert!(d13::payload_carries_a29_provenance(&payload).expect("reads"));

        // ...and the harness still refuses the stream, because a flipped bit in
        // `selected_present_receipt_sha256` would also pass all eighteen and the
        // recorder would refuse it.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D13,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        let Err(err) = scan_stream(&s, &schema, StreamIdentityMode::Legacy) else {
            panic!("the stream must be refused")
        };
        assert!(err.contains("A29 provenance"), "unexpected reason: {err}");

        // Including when it is not the first hour, so the refusal is not an
        // artifact of the stream's opening record.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D13,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d13_payload(0, 7, 10),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D13,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d13_payload_with_a29(101),
        ));
        s.extend(frame_end(4, 101));
        assert!(scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_rowless_failure_record_is_still_structurally_folded() {
        // `fold_tech` never short-circuits on status, so its exact-consumption
        // rule (`world_fold.rs:13571`) applies to a declared-rowless failure
        // payload too. Deferring the fold behind the status branch let one
        // through with a trailing byte.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        let mut payload = d13_payload_with_counts(FAILED, 7, 10, 0, 0);
        payload.push(0);
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D13,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D13,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d13_payload(0, 7, 11),
        ));
        s.extend(frame_end(4, 101));
        assert!(
            scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err(),
            "a trailing byte on a rowless failure payload is a corrupt stream"
        );

        // The same rule for a rowless SUCCESS payload, and for one whose declared
        // counts do not match its bytes in either direction.
        for (status, countries, actives, extra) in
            [(0u32, 0u32, 0u32, 1usize), (FAILED, 0, 0, 3), (0, 1, 1, 1)]
        {
            let mut payload = d13_payload_with_counts(status, 7, 10, countries, actives);
            payload.extend(std::iter::repeat(0u8).take(extra));
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D13,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err(),
                "status {status:#x}, {countries} country / {actives} active rows, +{extra} bytes"
            );
        }
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D13,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d13_payload(0, 7, 10),
            )
        };
        let mut ok = Vec::new();
        ok.extend(data(1, 100));
        ok.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        assert!(scan_stream(&ok, &test_schema(), StreamIdentityMode::Legacy).is_ok());

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
            let mut s = Vec::new();
            s.extend(data(1, 100));
            s.extend(seal);
            assert!(scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err());
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
            assert!(scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err());
        }
        for status in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(data(1, 100));
            s.extend(frame_end_sealed(2, 100, 0, 0, status, 0));
            assert!(scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_ok());
        }
        let mut later = Vec::new();
        later.extend(data(1, 100));
        later.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        later.extend(data(3, 101));
        later.extend(frame_end_sealed(4, 101, 0, 0, 0, 1));
        assert!(scan_stream(&later, &test_schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D13,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d13_payload(0, 7, 10),
            )
        };
        let mut ok = Vec::new();
        ok.extend(perf(1, 100, 20));
        ok.extend(perf(2, 100, D13));
        ok.extend(perf(3, 100, PROBE_DOMAIN_ID));
        ok.extend(data(4, 100));
        ok.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
        assert!(scan_stream(&ok, &test_schema(), StreamIdentityMode::Legacy).is_ok());

        let mut probes = Vec::new();
        probes.extend(perf(1, 100, PROBE_DOMAIN_ID));
        probes.extend(perf(2, 100, PROBE_DOMAIN_ID));
        probes.extend(data(3, 100));
        probes.extend(frame_end_sealed(4, 100, 0, 0, 0, 0));
        assert!(scan_stream(&probes, &test_schema(), StreamIdentityMode::Legacy).is_ok());

        let mut duplicate = Vec::new();
        duplicate.extend(perf(1, 100, 20));
        duplicate.extend(perf(2, 100, 20));
        duplicate.extend(data(3, 100));
        duplicate.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        assert!(scan_stream(&duplicate, &test_schema(), StreamIdentityMode::Legacy).is_err());

        let mut interleaved = Vec::new();
        interleaved.extend(data(1, 100));
        interleaved.extend(perf(2, 999, 20));
        interleaved.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&interleaved, &test_schema(), StreamIdentityMode::Legacy).is_err());

        let mut stale = Vec::new();
        stale.extend(data(1, 100));
        stale.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        stale.extend(perf(3, 100, 20));
        stale.extend(data(4, 101));
        stale.extend(frame_end_sealed(5, 101, 1, 1, 0, 0));
        assert!(scan_stream(&stale, &test_schema(), StreamIdentityMode::Legacy).is_err());

        let mut conflict = Vec::new();
        conflict.extend(perf(1, 100, 20));
        conflict.extend(perf(2, 101, 21));
        conflict.extend(data(3, 100));
        conflict.extend(frame_end_sealed(4, 100, 2, 2, 0, 0));
        assert!(scan_stream(&conflict, &test_schema(), StreamIdentityMode::Legacy).is_err());

        for (emitted, scheduled) in [(1u32, 1u32), (3, 3), (2, 3), (3, 2)] {
            let mut s = Vec::new();
            s.extend(perf(1, 100, 20));
            s.extend(perf(2, 100, D13));
            s.extend(data(3, 100));
            s.extend(frame_end_sealed(4, 100, emitted, scheduled, 0, 0));
            assert!(scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err());
        }

        let mut two_hours = Vec::new();
        two_hours.extend(perf(1, 100, 20));
        two_hours.extend(data(2, 100));
        two_hours.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        two_hours.extend(perf(4, 101, 20));
        two_hours.extend(data(5, 101));
        two_hours.extend(frame_end_sealed(6, 101, 1, 1, 0, 0));
        assert_eq!(
            scan_stream(&two_hours, &test_schema(), StreamIdentityMode::Legacy)
                .expect("PERF state resets")
                .frames
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
                D13,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d13_payload(0, 7, 10),
            )
        };
        for (perf_hour, data_hour) in [(100i32, 101i32), (101, 100), (100, 200), (200, 100)] {
            let mut s = Vec::new();
            s.extend(perf(1, perf_hour, 20));
            s.extend(data(2, data_hour));
            s.extend(frame_end_sealed(3, data_hour, 1, 1, 0, 0));
            assert!(scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err());
        }
        let mut agreeing = Vec::new();
        agreeing.extend(perf(1, 100, 20));
        agreeing.extend(data(2, 100));
        agreeing.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&agreeing, &test_schema(), StreamIdentityMode::Legacy).is_ok());
    }

    #[test]
    fn an_excluded_record_is_still_checked_by_the_payload_authority() {
        const FAILED: u32 = 1;
        let rowless = || d13_payload_with_counts(FAILED, 7, 10, 0, 0);
        let mut bad = Vec::new();

        let mut progress_scale = rowless();
        progress_scale[20..24].copy_from_slice(&999u32.to_le_bytes());
        bad.push(progress_scale);
        let mut live_scale = rowless();
        live_scale[24..28].copy_from_slice(&99_999u32.to_le_bytes());
        bad.push(live_scale);
        let mut changed = rowless();
        changed[12..16].copy_from_slice(&1u32.to_le_bytes());
        bad.push(changed);
        bad.push(d13_payload_with_counts(FAILED, 0, 10, 1, 0));
        bad.push(d13_payload_with_counts(FAILED, 7, 10, 2, 0));
        let mut country_flags = d13_payload_with_counts(FAILED, 7, 10, 1, 0);
        country_flags[d13::HEADER_LEN + 4..d13::HEADER_LEN + 8]
            .copy_from_slice(&(0b111u32 | 1 << 20).to_le_bytes());
        bad.push(country_flags);
        let mut country_slot = d13_payload_with_counts(FAILED, 7, 10, 1, 0);
        country_slot[d13::HEADER_LEN + 12..d13::HEADER_LEN + 16]
            .copy_from_slice(&33u32.to_le_bytes());
        bad.push(country_slot);
        let mut active_tag = d13_payload_with_counts(FAILED, 7, 10, 1, 1);
        let active_at = d13::HEADER_LEN + d13::COUNTRY_ROW_LEN;
        active_tag[active_at..active_at + 4].copy_from_slice(&0u32.to_le_bytes());
        bad.push(active_tag);
        let mut active_slot = d13_payload_with_counts(FAILED, 7, 10, 1, 1);
        active_slot[active_at + 4..active_at + 8].copy_from_slice(&33u32.to_le_bytes());
        bad.push(active_slot);

        for payload in bad {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D13,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            let err = scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy)
                .err()
                .expect("malformed excluded payload must refuse");
            assert!(err.contains("authority"), "unexpected refusal: {err}");
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut payload = d13_payload_with_counts(1 << bit, 7, 10, 0, 0);
            payload[20..24].copy_from_slice(&999u32.to_le_bytes());
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D13,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            let err = scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy)
                .err()
                .expect("authority is independent of the failure bit");
            assert!(err.contains("authority"), "bit {bit}: {err}");
        }

        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D13,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &rowless(),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D13,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d13_payload(0, 7, 11),
        ));
        s.extend(frame_end(4, 101));
        let scan =
            scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.frames.len(), 1);

        // Referencing an uncarried country is a fold-only rule. The authority
        // accepts that shape; d13's independent full fold/status policy still
        // refuses the row-bearing failure record.
        let mut outside = d13_payload_with_counts(FAILED, 7, 10, 1, 1);
        outside[active_at..active_at + 4].copy_from_slice(&9u32.to_le_bytes());
        let (header, tables) = d13::split_payload(&outside).expect("structural");
        check_authority_payload(&header, tables, 0).expect("outside authority reach");
    }

    #[test]
    fn the_identity_protocol_is_stated_not_sniffed() {
        use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;

        let decapitated = good_stream();
        assert!(scan_stream(&decapitated, &test_schema(), StreamIdentityMode::IdentityV1).is_err());
        assert!(scan_stream(&decapitated, &test_schema(), StreamIdentityMode::Legacy).is_ok());

        let mut identity_stream = Vec::new();
        identity_stream.extend(record(
            0,
            0,
            0,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            b"CID1-not-a-real-identity-payload",
        ));
        identity_stream.extend(&good_stream());
        assert!(scan_stream(&identity_stream, &test_schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn scan_stream_never_panics_on_arbitrary_bytes() {
        // Total over hostile input: the harness reads a file from disk and must
        // return a diagnosis, never unwind.
        let schema = test_schema();
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
        for i in 0..good.len() {
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
    fn an_hour_closed_without_a_d13_record_refuses_the_stream() {
        // The completeness bound on what this harness does NOT validate (module
        // docs): it cannot check other domains' payloads, so a d13 record
        // relabelled to another domain id — or dropped — must be caught by
        // counting, not silently measured as fewer hours.
        let mut s = good_stream();
        // Find the LAST d13 DATA record and relabel it to an unclaimed domain
        // id. The stream stays well formed — sequence, hour boundaries and
        // FRAME_END are untouched — so the completeness rule is the only rule
        // that can catch it.
        let mut pos = 0usize;
        let mut target = None;
        while pos < s.len() {
            let header = parse_compact_record_header(&s[pos..]).expect("well-formed");
            if header.domain_id == D13 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
            {
                target = Some(pos);
            }
            pos += COMPACT_RECORD_HEADER_LEN + header.payload_len;
        }
        let target = target.expect("good_stream carries a d13 DATA record");
        // domain_id sits at +20: magic(4) + version(2) + header_len(2) +
        // sequence(8) + game_hour(4).
        s[target + 20..target + 22].copy_from_slice(&99u16.to_le_bytes());
        let err = match scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy) {
            Err(e) => e,
            Ok(_) => panic!("an hour with no d13 record must refuse the stream"),
        };
        assert!(
            err.contains("game hour(s) but carries"),
            "the completeness check must be what rejects it: {err}"
        );
    }
}
