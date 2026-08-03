//! Measure the wire format against the shipping compact stream on REAL d17
//! `diplomacy_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d17-scheduled capture
//! (`runs/batch1/deepwar_t1`). The stream's own d17 DATA records are the
//! shipping format's cost for the domain: a 24-byte header, a fixed 24-byte
//! country row per country the frame carries, and a fixed 16-byte at-war row per
//! war-graph edge, exactly what the recorder stores today. This tool replays
//! those records into the running state `world_fold::fold_diplomacy` builds —
//! an accumulator, not a full replacement, see `wire_format::d17`'s module docs
//! — and re-encodes every hour with this crate's grammar: a BASELINE at each
//! 48-hour block start, field-granular stored-minimal DELTAs between.
//!
//! **Nothing here re-derives a contract it can call.** Payload and row decoding,
//! the accumulator grammar and the war-segment semantics go through
//! `wire_format::d17`'s public boundary; H4CR framing, record semantics,
//! sequence continuity and the status-flag predicates go through
//! `compact_contract`'s own parser, validator and helpers. A harness that
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
//!   (`recorder_host/src/stream_fold.rs:277`), its DOMAIN_PERF hour binding
//!   (`:328`-`:365`), and the DATA arm's pending-PERF rule (`:378`-`:386`);
//! - FRAME_END seal fields (`stream_fold.rs:46`-`:89`) and their distinct
//!   non-probe PERF-domain count cross-check (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority, and it additionally checks FRAME_END seal fields and
//! DOMAIN_PERF domain accounting. This crate cannot depend on `recorder_host`
//! (it pulls in every capture lane), so the subset is enforced here and the full
//! check remains the recorder's.
//!
//! **Exclusion, and its position.** `fold_diplomacy` reads its status word
//! FIRST (`world_fold.rs:13789`-`:13796`), before it reads either row count, and
//! returns without folding on a failure status — so that is the one path on
//! which this domain returns without folding, and the exclusion happens here
//! before any row is parsed. d17 has no baseline/delta grammar and therefore no
//! delta-before-baseline path. The informational `WAR_SEGMENT_ABSENT` bit is
//! explicitly NOT in the failure mask (`:13787`-`:13788`) and its frames are
//! data: they are measured, with the war-segment semantics `d17::WarSegment`
//! carries. Hour uniqueness is enforced over EVERY d17 DATA record, including
//! failure-status ones, and before the exclusion, so a success+failure pair in
//! one hour cannot slip through.
//!
//! **Validity before classification.** Every read of the status word here —
//! the failure predicate, the war segment, the bridge bit — classifies the v1
//! vocabulary and is not a validity check, so a status word carrying a bit
//! version 1 does not assign refuses the stream before any of them run
//! (`compact_contract/src/lib.rs:121`-`:123`). An unassigned bit is not
//! evidence of a future informational status, and classifying one would report
//! a measured hour for a record with no defined meaning.
//!
//! Production's second, status-independent payload pass is
//! `validate_domain_data_payload_at` (`stream_fold.rs:594`), which invokes
//! `decode_compact_diplomacy_data_payload`
//! (`domain_data_authority.rs:4407`-`:4515`). `d17::split_payload` already covers
//! its header and exact-length shape before exclusion. [`check_authority_payload`]
//! adds the remaining predicates on every status: `changed_rows <=
//! country_count`; nonzero/unique country tags and known country flags; declared
//! relation totals; and relation owner, token-length, token-byte, padding, and
//! uniqueness checks. It deliberately does not add mapper/fold-only predicates
//! such as casualty zero-packing, so a well-formed excluded record retains
//! production's early-exit behavior.
//!
//! These payload checks plus the seal, PERF binding, pending-PERF, and explicitly
//! stated identity mode are the five-check sweep. Production receives identity
//! mode from its caller (`stream_fold.rs:220`); the measured capture is legacy,
//! which remains `main`'s default.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d17.bin   — the d17 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! **Only d17 DATA payloads are validated.** Production runs
//! `validate_domain_data_payload_at` on the DATA payload of EVERY domain
//! (`stream_fold.rs:593`); this harness cannot, because validating another
//! domain's payload means owning that domain's authority, and `wire_format`
//! must not depend on `recorder_host`. Every DATA record of any other domain
//! therefore passes with its H4CR header, sequence continuity, hour-boundary,
//! PERF-binding and FRAME_END rules checked but its **payload bytes unread**.
//! The concrete hazard — a d17 record relabelled to another domain id, or
//! dropped outright, silently shrinking the measurement — is closed by an
//! explicit completeness check in `scan_stream`: every closed game hour must
//! carry exactly one d17 DATA record, of any status. That is a per-capture
//! assertion, not a grammar rule; a capture where d17 is not scheduled every
//! hour fails loudly there and a human adjusts.

use std::collections::BTreeSet;
use std::io::Write;
use std::ops::Range;

use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, compact_status_unknown_flags_v1,
    parse_compact_record_header, COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_FAILURE_MASK, COMPACT_STATUS_WAR_BRIDGE_APPLIED,
    COMPACT_STATUS_WAR_SEGMENT_ABSENT,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedDelta,
    KeyedState,
};
use wire_format::d17::{self, Accumulator, WarSegment};
use wire_format::Schema;

const D17: u16 = 17;
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

/// One usable d17 hour: its game hour, its war-segment status, and the byte
/// spans of its record (the shipping arm is those bytes verbatim, header
/// included) and of its payload (which pass 2 re-folds).
struct D17Hour {
    game_hour: i32,
    war_segment: WarSegment,
    ship_span: Range<usize>,
    payload_span: Range<usize>,
}

#[derive(Default)]
struct Counts {
    /// d17 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// Measured d17 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// Measured hours carrying `WAR_SEGMENT_ABSENT` — the at-war collection's
    /// membership was not measured in any of them. The accumulator still folds
    /// exactly as `fold_diplomacy` does; the format cannot mark a collection-hour
    /// unknown (§8 has no code here), so the count is reported.
    war_segment_absent: usize,
    /// Measured hours whose `WAR_BRIDGE_APPLIED` bit disagrees with the
    /// complement of `WAR_SEGMENT_ABSENT`. The collector sets one exactly when
    /// the other is clear (`attach_log.rs:59398`-`:59413`); a disagreement would
    /// mean the two accountings have drifted apart.
    war_bit_disagreements: usize,
    /// Measured hours whose `changed_rows` word does not equal `country_count`.
    /// The fold never reads `+12`, so this is a report and never a rejection.
    changed_rows_disagreements: usize,
    country_rows: usize,
    /// At-war rows the stream carried. **None are stored**: the war graph is a
    /// reported stop condition (`d17`'s module docs — §5 has no symbol-id
    /// assignment rule for the enemy TAG, and §4 has no string kind). Printed so
    /// the stop's cost is a number rather than a silence.
    at_war_rows_not_stored: usize,
    /// Of those, rows the fold itself drops (`world_fold.rs:13822`).
    orphan_at_war_rows: usize,
    at_war_count_mismatches: usize,
    at_war_gap_hours: usize,
    /// At-war rows whose token is not one `is_country_tag` +
    /// `append_fixed_ascii_token8` could have produced.
    non_collector_shaped_tokens: usize,
}

struct StreamScan {
    hours: Vec<D17Hour>,
    counts: Counts,
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

/// The status-independent predicates in
/// `decode_compact_diplomacy_data_payload`
/// (`domain_data_authority.rs:4407`-`:4515`) not already supplied by
/// `d17::split_payload`.
fn check_authority_payload(
    header: &d17::PayloadHeader,
    country_rows: &[u8],
    relation_rows: &[u8],
    at_byte: usize,
) -> Result<(), String> {
    if header.changed_rows > header.country_count {
        return Err(format!(
            "d17 authority at byte {at_byte}: changed_rows {} > country_count {}",
            header.changed_rows, header.country_count
        ));
    }

    let mut seen_countries = BTreeSet::new();
    let mut declared_relations = 0usize;
    for (row_index, row) in country_rows.chunks_exact(d17::COUNTRY_ROW_LEN).enumerate() {
        let key = d17::key_from_country_row(row)
            .map_err(|e| format!("d17 authority country row {row_index}: {e}"))?;
        let tag = key.0[0];
        if tag == 0 {
            return Err(format!(
                "d17 authority country row {row_index} at byte {at_byte}: zero tag_db_id"
            ));
        }
        if !seen_countries.insert(tag) {
            return Err(format!(
                "d17 authority country row {row_index} at byte {at_byte}: duplicate tag_db_id {tag}"
            ));
        }
        let flags = u32::from_le_bytes(row[4..8].try_into().expect("4 bytes"));
        if flags & !(d17::FLAG_AT_WAR | d17::FLAG_CASUALTIES) != 0 {
            return Err(format!(
                "d17 authority country row {row_index} at byte {at_byte}: unknown flags {flags:#x}"
            ));
        }
        let at_war_count = u32::from_le_bytes(row[12..16].try_into().expect("4 bytes"));
        declared_relations = declared_relations.saturating_add(at_war_count as usize);
    }
    if declared_relations != header.relation_count as usize {
        return Err(format!(
            "d17 authority at byte {at_byte}: relation_count {} != declared country relations \
             {declared_relations}",
            header.relation_count
        ));
    }

    let mut seen_relations = BTreeSet::new();
    for (row_index, row) in relation_rows.chunks_exact(d17::AT_WAR_ROW_LEN).enumerate() {
        let owner = d17::at_war_owner_of_row(row)
            .map_err(|e| format!("d17 authority relation row {row_index}: {e}"))?;
        if owner == 0 {
            return Err(format!(
                "d17 authority relation row {row_index} at byte {at_byte}: zero tag_db_id"
            ));
        }
        let token_len = d17::enemy_tag_token_len(row)
            .map_err(|e| format!("d17 authority relation row {row_index}: {e}"))?
            as usize;
        let word = d17::enemy_tag_word(row)
            .map_err(|e| format!("d17 authority relation row {row_index}: {e}"))?;
        let token = d17::enemy_tag_token(word);
        if token_len == 0 || token_len > token.len() {
            return Err(format!(
                "d17 authority relation row {row_index} at byte {at_byte}: invalid token_len \
                 {token_len}"
            ));
        }
        if token[..token_len]
            .iter()
            .any(|byte| !(byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'-'))
        {
            return Err(format!(
                "d17 authority relation row {row_index} at byte {at_byte}: non-token bytes"
            ));
        }
        if token[token_len..].iter().any(|byte| *byte != 0) {
            return Err(format!(
                "d17 authority relation row {row_index} at byte {at_byte}: nonzero padding"
            ));
        }
        if !seen_relations.insert((owner, token)) {
            return Err(format!(
                "d17 authority relation row {row_index} at byte {at_byte}: duplicate relation"
            ));
        }
    }
    Ok(())
}

/// Whether this frame's war-relations segment captured, from the frame's own
/// status word (`attach_log.rs:59410`-`:59413`).
fn war_segment_of(status_flags: u32) -> WarSegment {
    if status_flags & COMPACT_STATUS_WAR_SEGMENT_ABSENT != 0 {
        WarSegment::Absent
    } else {
        WarSegment::Captured
    }
}

/// Fold one payload into `acc` and accumulate its report into `counts`. The one
/// implementation both passes use, so pass 2 cannot fold differently from the
/// pass that accepted the stream.
fn apply_frame(
    acc: &mut Accumulator,
    schema: &Schema,
    payload: &[u8],
    war_segment: WarSegment,
    counts: &mut Counts,
) -> Result<(), String> {
    let report = acc.fold_payload(&schema.collections[0], payload, war_segment)?;
    counts.country_rows += report.country_rows;
    counts.at_war_rows_not_stored += report.at_war_rows_not_stored;
    counts.orphan_at_war_rows += report.orphan_at_war_rows;
    counts.non_collector_shaped_tokens += report.non_collector_shaped_tokens;
    counts.at_war_count_mismatches += report.at_war_count_mismatches;
    if report.at_war_hour_gap {
        counts.at_war_gap_hours += 1;
    }
    Ok(())
}

/// Validate the whole stream against the canonical contract and fold every
/// usable d17 DATA record. Every rejection path is an `Err`, never a silently
/// shorter measurement and never a panic.
fn scan_stream(
    stream: &[u8],
    schema: &Schema,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours: Vec<D17Hour> = Vec::new();
    let mut counts = Counts::default();
    // The scan folds into its own accumulator: every way a row or a frame can be
    // rejected is a property of the stream, so it belongs to the pass that
    // either accepts the stream or refuses it. Deferring it would leave the
    // harness able to accept a stream and then unwind while measuring it.
    let mut acc = Accumulator::new();
    let mut scan_counts = Counts::default();

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d17 DATA record of ANY status — the uniqueness rule.
    let mut last_d17_hour: Option<i32> = None;
    // Hour of the last d17 record that entered the measurement — the contiguity
    // report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;

    // Closed game hours and d17 DATA records of ANY status — the
    // completeness rule at the end of the scan.
    let mut closed_hours = 0usize;
    let mut d17_records = 0usize;
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

        if header.domain_id == D17 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d17_records += 1;
            let (d17_header, country_rows, relation_rows) = d17::split_payload(payload)
                .map_err(|e| format!("d17 payload at byte {pos}: {e}"))?;

            // One d17 DATA record per game hour, checked over EVERY d17 record
            // and BEFORE any status-based exclusion: a second record for the
            // same hour would fold twice into one measurement step, and gating
            // this on the record being usable would let a success/failure pair
            // through the rule entirely.
            if let Some(previous) = last_d17_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d17 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d17_hour = Some(header.game_hour_raw);

            // VALIDITY BEFORE CLASSIFICATION. Everything below that reads the
            // status word — the failure predicate, `war_segment_of`, the
            // `WAR_BRIDGE_APPLIED` read — classifies the v1 vocabulary; none of
            // it is a validity predicate. `compact_contract/src/lib.rs:121`-
            // `:123` says so in as many words: "Callers handling untrusted status
            // words must first reject a non-zero `compact_status_unknown_flags_v1`
            // result; otherwise an unassigned bit has no defined classification."
            // Bits 15..=31 are unassigned (`status_flags_base.rs:40`-`:44`), and
            // an unassigned bit "is not evidence of a future informational
            // status".
            //
            // This check previously ran INSIDE the measured branch, after
            // `war_segment_of` and the failure predicate had already classified
            // the word, and only incremented a reported counter. A record with
            // `status_flags = 1 << 14` was therefore classified as "not a
            // failure", given a war segment, and measured as a healthy hour with
            // a note beside it. The production lane rejects first
            // (`lanes/division_state_movement_live/domain20_compact.rs:419`);
            // this harness now does the same. No capture carries an unassigned
            // bit — the reference stream's count was 0 — so no measured number
            // moves.
            let unknown_status = compact_status_unknown_flags_v1(d17_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d17 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            check_authority_payload(&d17_header, country_rows, relation_rows, pos)?;

            // `fold_diplomacy` reads the status word before anything else
            // (`world_fold.rs:13789`) and returns without folding on a failure
            // status, retaining the accumulator. The predicate is
            // `compact_contract`'s own, not a copied bit mask: it already applies
            // d17's lane-local mask and expected partial tolerance (neither
            // exists), so it cannot drift from the recorder.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D17,
                d17_header.status_flags,
            ) != 0
            {
                counts.failure_records += 1;
            } else {
                if let Some(previous) = last_measured_hour {
                    if header.game_hour_raw != previous + 1 {
                        counts.non_contiguous += 1;
                    }
                }
                last_measured_hour = Some(header.game_hour_raw);

                let war_segment = war_segment_of(d17_header.status_flags);
                if war_segment == WarSegment::Absent {
                    counts.war_segment_absent += 1;
                }
                let bridge_applied =
                    d17_header.status_flags & COMPACT_STATUS_WAR_BRIDGE_APPLIED != 0;
                if bridge_applied != (war_segment == WarSegment::Captured) {
                    counts.war_bit_disagreements += 1;
                }
                if d17_header.changed_rows != d17_header.country_count {
                    counts.changed_rows_disagreements += 1;
                }
                // Fold exactly as `fold_diplomacy` does, at the same point in
                // the pass. Every frame the fold refuses is refused here, and
                // the at-war census (rows the stop condition leaves unstored)
                // comes back in the same report.
                apply_frame(&mut acc, schema, payload, war_segment, &mut scan_counts)
                    .map_err(|e| format!("d17 payload at byte {pos}: {e}"))?;

                hours.push(D17Hour {
                    game_hour: header.game_hour_raw,
                    war_segment,
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
    if hours.is_empty() {
        return Err("stream carries no usable d17 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d17 DATA
    // record, of any status. Production validates the payload of EVERY domain's
    // DATA record (`stream_fold.rs:593`); this harness cannot (module docs), so
    // a d17 record relabelled to another domain id — or lost outright — would
    // otherwise be silently ignored and the run would report a plausible
    // measurement over fewer hours. Per-hour uniqueness is already enforced
    // above, so equality of the two counts is exactly-one per closed hour. A
    // per-capture assertion, not a grammar rule: a capture that does not
    // schedule d17 every hour fails here loudly and a human adjusts, instead
    // of a number quietly shrinking.
    if d17_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d17_records} d17 DATA \
             record(s); every closed hour of a d17-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    counts.country_rows = scan_counts.country_rows;
    counts.at_war_rows_not_stored = scan_counts.at_war_rows_not_stored;
    counts.orphan_at_war_rows = scan_counts.orphan_at_war_rows;
    counts.non_collector_shaped_tokens = scan_counts.non_collector_shaped_tokens;
    counts.at_war_count_mismatches = scan_counts.at_war_count_mismatches;
    counts.at_war_gap_hours = scan_counts.at_war_gap_hours;
    Ok(StreamScan { hours, counts })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let input = args
        .next()
        .expect("usage: bench_real_d17 <collector_stream.bin> <out dir> [legacy|identity-v1]");
    let out_dir = args
        .next()
        .expect("usage: bench_real_d17 <collector_stream.bin> <out dir> [legacy|identity-v1]");
    let identity_mode = match args.next().as_deref().unwrap_or("legacy") {
        "legacy" => StreamIdentityMode::Legacy,
        "identity-v1" => StreamIdentityMode::IdentityV1,
        other => panic!("unknown identity mode {other:?}; expected legacy or identity-v1"),
    };
    assert!(args.next().is_none(), "too many arguments");
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d17::schema();

    // ---- Pass 1: validate the WHOLE stream and fold its d17 DATA records. ---
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result.
    let scan = match scan_stream(&stream, &schema, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };

    // ---- Pass 2: re-fold the accepted hours and encode both arms. -----------
    // The fold runs through the identical `apply_frame` the scan proved every
    // hour against, so nothing here can fail on stream content; every assertion
    // below is about the codec.
    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut acc = Accumulator::new();
    let mut sink = Counts::default();
    let mut prior: Option<Vec<KeyedState>> = None;
    let mut reader: Option<Vec<KeyedState>> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    // Baselines the §1.5 gap rule forced outside a scheduled block start.
    let mut gap_forced_baselines = 0usize;
    let mut previous_measured_hour: Option<i32> = None;
    let mut empty_hours = 0usize;
    let mut delta_hours_with_change = 0usize;
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    // Churn is measured over EVERY consecutive measured-hour transition, not
    // only over the hours that emitted a DELTA. Dividing by emitted deltas
    // answers "how big is a delta when there is one", which on a stream whose
    // domain is idle most hours reports a per-hour rate hundreds of times too
    // high; the recipe asks for real churn per hour, so the denominator is the
    // number of transitions.
    let mut changed_rows_over_transitions = 0usize;
    let mut tombstones_over_transitions = 0usize;
    let mut transitions = 0usize;
    // The same quantities restricted to hours that emitted a DELTA, which is
    // what sizes one delta body.
    let mut changed_rows_in_deltas = 0usize;
    let mut tombstones_in_deltas = 0usize;
    let mut rows_total = 0usize;

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());

        let payload = &stream[hour.payload_span.clone()];
        if let Err(reason) = apply_frame(&mut acc, &schema, payload, hour.war_segment, &mut sink) {
            eprintln!("{input}: game hour {} re-fold: {reason}", hour.game_hour);
            std::process::exit(1);
        }
        let states = vec![acc.countries.clone()];
        rows_total += states[0].len();

        // Churn against the previous measured hour, independent of what this
        // hour emits: a block-start baseline and an unchanged hour are both real
        // transitions and both belong in the rate.
        if let Some(previous) = prior.as_ref() {
            transitions += 1;
            for (i, collection) in schema.collections.iter().enumerate() {
                let d = diff(collection, &previous[i], &states[i]);
                changed_rows_over_transitions += d.changed.len();
                tombstones_over_transitions += d.tombstones.len();
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
            encode_baseline(&schema, &states, &mut wire).expect("baseline encodes");
            baselines += 1;
            baseline_bytes += wire.len() - hour_start;
        } else {
            let previous = prior.as_ref().expect("prior exists");
            let deltas: Vec<Option<KeyedDelta>> = schema
                .collections
                .iter()
                .enumerate()
                .map(|(i, collection)| {
                    let d = diff(collection, &previous[i], &states[i]);
                    if d.is_empty() {
                        None
                    } else {
                        Some(d)
                    }
                })
                .collect();
            if deltas.iter().all(Option::is_none) {
                empty_hours += 1;
                emitted = false;
            } else {
                delta_hours_with_change += 1;
                for delta in deltas.iter().flatten() {
                    changed_rows_in_deltas += delta.changed.len();
                    tombstones_in_deltas += delta.tombstones.len();
                }
                encode_delta(&schema, previous, &deltas, &mut wire).expect("delta encodes");
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
            if baseline_here || reader.is_none() {
                let (decoded, consumed) =
                    decode_baseline(&schema, hour_bytes, 0).expect("baseline decodes");
                assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
                reader = Some(decoded);
            } else {
                let mut reader_state = reader.take().expect("reader exists");
                let (deltas, consumed) =
                    decode_delta(&schema, &reader_state, hour_bytes, 0).expect("delta decodes");
                assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
                for (i, delta) in deltas.iter().enumerate() {
                    if let Some(d) = delta {
                        apply(&mut reader_state[i], d);
                    }
                }
                reader = Some(reader_state);
            }
        }
        let replayed = reader.as_ref().expect("reader exists");
        for (i, collection) in schema.collections.iter().enumerate() {
            assert_eq!(
                replayed[i].len(),
                states[i].len(),
                "game hour {}: collection {} row count",
                hour.game_hour,
                collection.collection_id
            );
            assert!(
                diff(collection, &replayed[i], &states[i]).is_empty(),
                "game hour {}: collection {} reader state is not stored-equal to the fold",
                hour.game_hour,
                collection.collection_id
            );
        }

        prior = Some(states);
        previous_measured_hour = Some(hour.game_hour);
        hours_seen += 1;
    }

    std::fs::create_dir_all(&out_dir).expect("out dir");
    for (name, bytes) in [("ship_d17.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d17.offsets", &ship_offsets),
        ("wire_new.offsets", &wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    let c = &scan.counts;
    println!(
        "hours                {hours_seen} ({} non-contiguous steps)",
        c.non_contiguous
    );
    println!(
        "excluded hours       {} failure-status (the domain's only return-without-folding path)",
        c.failure_records
    );
    println!(
        "war segment          {} of {hours_seen} hours absent (informational bit 13; folded as \
         the fold folds it, and the collection-hour reported unmeasured: {} gap hours), {} \
         bridge-bit disagreements",
        c.war_segment_absent, c.at_war_gap_hours, c.war_bit_disagreements
    );
    println!(
        "status/header notes  {} hours where changed_rows != country_count (reported, not \
         enforced); an unassigned v1 status bit refuses the stream rather than being reported",
        c.changed_rows_disagreements
    );
    println!("payload rows         {} country rows", c.country_rows);
    println!(
        "STOPPED war graph    {} at-war rows carried and NOT stored ({} of them the fold itself \
         drops, {} tokens outside the collector's own shape, {} at_war_count disagreements); \
         see d17.rs's stop condition",
        c.at_war_rows_not_stored,
        c.orphan_at_war_rows,
        c.non_collector_shaped_tokens,
        c.at_war_count_mismatches
    );
    println!(
        "state rows avg       {:.1} countries",
        rows_total as f64 / hours_seen as f64
    );
    println!(
        "baselines            {baselines} ({} every {BLOCK_HOURS} h, {gap_forced_baselines} \
         forced by a gap in measured hours — WIRE_FORMAT §1.5)",
        baselines - gap_forced_baselines
    );
    println!("delta hours          {delta_hours_with_change}   empty hours {empty_hours}");
    println!(
        "real churn           {:.3} changed rows/hour, {:.3} tombstones/hour over {transitions} \
         measured-hour transitions (stored precision)",
        changed_rows_over_transitions as f64 / transitions.max(1) as f64,
        tombstones_over_transitions as f64 / transitions.max(1) as f64
    );
    println!(
        "  per emitted delta  {:.2} changed rows, {:.3} tombstones across \
         {delta_hours_with_change} delta hours",
        changed_rows_in_deltas as f64 / delta_hours_with_change.max(1) as f64,
        tombstones_in_deltas as f64 / delta_hours_with_change.max(1) as f64
    );
    println!("ship_d17.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.2}%), raw split {baseline_bytes} baseline + \
         {delta_bytes} delta",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{COMPACT_DOMAIN_FRAME, COMPACT_FRAME_END_PAYLOAD_LEN};

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

    fn country_row(
        tag: u32,
        flags: u32,
        faction_group: i32,
        at_war_count: u32,
        casualties: i64,
    ) -> Vec<u8> {
        let mut row = Vec::new();
        row.extend_from_slice(&tag.to_le_bytes());
        row.extend_from_slice(&flags.to_le_bytes());
        row.extend_from_slice(&faction_group.to_le_bytes());
        row.extend_from_slice(&at_war_count.to_le_bytes());
        row.extend_from_slice(&casualties.to_le_bytes());
        row
    }

    fn at_war_row(tag: u32, token_len: u32, enemy: &[u8]) -> Vec<u8> {
        let mut row = Vec::new();
        row.extend_from_slice(&tag.to_le_bytes());
        row.extend_from_slice(&token_len.to_le_bytes());
        let mut token = [0u8; 8];
        token[..enemy.len().min(8)].copy_from_slice(&enemy[..enemy.len().min(8)]);
        row.extend_from_slice(&token);
        row
    }

    fn d17_payload(status_flags: u32, countries: &[Vec<u8>], at_war: &[Vec<u8>]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes());
        p.extend_from_slice(&(countries.len() as u32).to_le_bytes());
        p.extend_from_slice(&(countries.len() as u32).to_le_bytes());
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&(at_war.len() as u32).to_le_bytes());
        for row in countries {
            p.extend_from_slice(row);
        }
        for row in at_war {
            p.extend_from_slice(row);
        }
        p
    }

    fn simple_payload(status_flags: u32, faction_group: i32) -> Vec<u8> {
        d17_payload(
            status_flags,
            &[country_row(
                7,
                d17::FLAG_AT_WAR | d17::FLAG_CASUALTIES,
                faction_group,
                1,
                5,
            )],
            &[at_war_row(7, 3, b"GER")],
        )
    }

    /// Two closed hours, each carrying one d17 DATA record with a captured war
    /// segment.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D17,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &simple_payload(COMPACT_STATUS_WAR_BRIDGE_APPLIED, 3),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D17,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &simple_payload(COMPACT_STATUS_WAR_BRIDGE_APPLIED, 4),
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
    fn a_well_formed_stream_yields_its_d17_hours() {
        let scan = scan_stream(&good_stream(), &d17::schema(), StreamIdentityMode::Legacy)
            .expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].game_hour, 100);
        assert_eq!(scan.hours[0].war_segment, WarSegment::Captured);
        assert_eq!(scan.counts.failure_records, 0);
        assert_eq!(scan.counts.non_contiguous, 0);
        assert_eq!(scan.counts.war_segment_absent, 0);
        assert_eq!(scan.counts.war_bit_disagreements, 0);
        assert_eq!(scan.counts.country_rows, 2);
        assert_eq!(
            scan.counts.at_war_rows_not_stored, 2,
            "the stopped table's rows are counted, never passed over"
        );
        assert_eq!(scan.counts.non_collector_shaped_tokens, 0);
        assert_eq!(scan.counts.changed_rows_disagreements, 0);
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helpers classify
        // the v1 vocabulary and are not validity predicates, so an unassigned bit
        // must be rejected BEFORE classification — otherwise it silently reads as
        // "not a failure", is given a war segment, and the hour is measured as
        // healthy with a counter incremented beside it.
        for bit in 15..32u32 {
            let status = 1u32 << bit;
            // Alone, combined with a known failure bit, and combined with the two
            // informational bits d17 itself reads; with rows and without. The
            // rejection must not depend on any of it.
            for extra in [
                0u32,
                1,
                COMPACT_STATUS_WAR_SEGMENT_ABSENT,
                COMPACT_STATUS_WAR_BRIDGE_APPLIED,
            ] {
                for countries in [Vec::new(), vec![country_row(7, 0, 3, 0, 0)]] {
                    let mut s = Vec::new();
                    s.extend(record(
                        1,
                        100,
                        D17,
                        COMPACT_RECORD_TYPE_DOMAIN_DATA,
                        &d17_payload(status | extra, &countries, &[]),
                    ));
                    s.extend(frame_end(2, 100));
                    assert!(
                        scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).is_err(),
                        "status bit {bit} (+{extra:#x}) with {} country row(s) must be refused",
                        countries.len()
                    );
                }
            }
        }
        // And the whole known v1 mask, minus the failure bits, is still accepted:
        // the check must reject unassigned bits and nothing else. WAR_SEGMENT_ABSENT
        // (bit 13) is the one every hour of the reference capture carries, and
        // WAR_BRIDGE_APPLIED (bit 4) is its complement; DELTA_DATA,
        // EXTRACTION_PERF_GATE and TRANSITION_BASELINE_RESET are informational too.
        for informational in [
            COMPACT_STATUS_WAR_BRIDGE_APPLIED,
            1u32 << 5,
            1 << 10,
            1 << 11,
            COMPACT_STATUS_WAR_SEGMENT_ABSENT,
        ] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D17,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d17_payload(informational, &[country_row(7, 0, 3, 0, 0)], &[]),
            ));
            s.extend(frame_end(2, 100));
            let scan = scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 1, "bit {informational:#x}");
            assert_eq!(scan.counts.failure_records, 0, "bit {informational:#x}");
        }
    }

    #[test]
    fn a_war_segment_absent_hour_is_measured_and_reported_as_a_gap() {
        // Bit 13 is informational and NOT in the failure mask
        // (`world_fold.rs:13787`-`:13788`): its frames are data.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D17,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d17_payload(
                COMPACT_STATUS_WAR_SEGMENT_ABSENT,
                &[country_row(7, 0, 3, 0, 0)],
                &[],
            ),
        ));
        s.extend(frame_end(2, 100));
        let scan = scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).expect("accepted");
        assert_eq!(
            scan.counts.failure_records, 0,
            "informational, never excluded"
        );
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.hours[0].war_segment, WarSegment::Absent);
        assert_eq!(scan.counts.war_segment_absent, 1);
        assert_eq!(scan.counts.at_war_gap_hours, 1);
        assert_eq!(scan.counts.war_bit_disagreements, 0);
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time: the
        // fold returns before reading a row and retains the accumulator, so the
        // hour is not d17 data and must not enter either arm.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D17,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &simple_payload(1 << bit, 3),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D17,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &simple_payload(COMPACT_STATUS_WAR_BRIDGE_APPLIED, 4),
            ));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy)
                .expect("the stream itself is well-formed");
            assert_eq!(scan.counts.failure_records, 1, "bit {bit}");
            assert_eq!(scan.hours.len(), 1, "bit {bit}");
            assert_eq!(scan.hours[0].game_hour, 101, "bit {bit}");
        }
        // And two informational bits are NOT treated as failures.
        for flag in [COMPACT_STATUS_WAR_SEGMENT_ABSENT, 1 << 10] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D17,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &simple_payload(flag, 3),
            ));
            s.extend(frame_end(2, 100));
            let scan =
                scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).expect("well-formed");
            assert_eq!(scan.counts.failure_records, 0, "flag {flag:#x}");
            assert_eq!(scan.hours.len(), 1, "flag {flag:#x}");
        }
    }

    #[test]
    fn a_second_d17_record_in_one_hour_is_rejected() {
        // Checked over EVERY d17 DATA record and before status-based exclusion,
        // so all four success/failure orderings are refused.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        let ok = COMPACT_STATUS_WAR_BRIDGE_APPLIED;
        for (first, second) in [(ok, ok), (ok, FAILED), (FAILED, ok), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D17,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &simple_payload(first, 3),
            ));
            s.extend(record(
                2,
                100,
                D17,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &simple_payload(second, 4),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).is_err(),
                "two d17 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&full[..cut], &d17::schema(), StreamIdentityMode::Legacy).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary IS a well-formed shorter stream.
        for (records, expected_hours) in [(2usize, 1usize), (4, 2)] {
            let cut = boundary(&full, records);
            let scan = scan_stream(&full[..cut], &d17::schema(), StreamIdentityMode::Legacy)
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
                scan_stream(&full[..cut], &d17::schema(), StreamIdentityMode::Legacy).is_err(),
                "mid-record cut at {cut}"
            );
        }
        assert!(
            scan_stream(&[], &d17::schema(), StreamIdentityMode::Legacy).is_err(),
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
                    scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).is_err(),
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
                    scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).is_err(),
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
            D17,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &simple_payload(0, 3),
        ));
        s.extend(frame_end(2, 999));
        assert!(scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).is_err());

        let mut s = Vec::new();
        s.extend(frame_end(1, 100));
        assert!(scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            101,
            D17,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &simple_payload(0, 3),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D17,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &simple_payload(0, 4),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_stream_with_no_d17_records_is_refused_not_reported_as_zero() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &[0u8; 8],
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_malformed_d17_payload_is_refused_not_skipped() {
        // A short or inconsistent payload is a corrupt stream, not an hour to
        // drop quietly.
        let mut payload = simple_payload(0, 3);
        payload.truncate(payload.len() - 1);
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D17,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).is_err());

        // A row the MAPPER refuses must make the SCAN fail, not be accepted and
        // then unwind while measuring: a gated-off field carrying a value (both
        // fields) and a negative casualties total.
        let invalid: Vec<Vec<u8>> = vec![
            d17_payload(0, &[country_row(7, 0, 3, 1, 0)], &[]),
            d17_payload(0, &[country_row(7, 0, 3, 0, 9)], &[]),
            d17_payload(
                0,
                &[country_row(
                    7,
                    d17::FLAG_AT_WAR | d17::FLAG_CASUALTIES,
                    3,
                    0,
                    -1,
                )],
                &[],
            ),
        ];
        for (i, payload) in invalid.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D17,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).is_err(),
                "invalid payload {i} must make the scan fail"
            );
        }

        // The fold does not store the at-war rows, but the independent production
        // authority rejects a repeated `(owner, token)` pair
        // (`domain_data_authority.rs:4508`).
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D17,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d17_payload(
                0,
                &[country_row(7, d17::FLAG_AT_WAR, 3, 2, 0)],
                &[at_war_row(7, 3, b"GER"), at_war_row(7, 3, b"GER")],
            ),
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_non_collector_shaped_token_is_counted_and_still_measured() {
        // Lowercase ASCII is legal to the production authority but not
        // collector-shaped, so the report remains meaningful without feeding
        // the stream a token production certification rejects.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D17,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d17_payload(
                0,
                &[country_row(7, d17::FLAG_AT_WAR, 3, 1, 0)],
                &[at_war_row(7, 3, b"ger")],
            ),
        ));
        s.extend(frame_end(2, 100));
        let scan = scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).expect("accepted");
        assert_eq!(scan.counts.non_collector_shaped_tokens, 1);
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn a_rowless_hour_is_a_measured_no_op() {
        // 901 of the capture's 903 hours are exactly this shape.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D17,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &simple_payload(COMPACT_STATUS_WAR_BRIDGE_APPLIED, 3),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D17,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d17_payload(COMPACT_STATUS_WAR_BRIDGE_APPLIED, &[], &[]),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(
            scan.counts.country_rows, 1,
            "the second hour carries no rows"
        );
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D17,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &simple_payload(COMPACT_STATUS_WAR_BRIDGE_APPLIED, 3),
            )
        };
        let mut ok = Vec::new();
        ok.extend(data(1, 100));
        ok.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        assert!(scan_stream(&ok, &d17::schema(), StreamIdentityMode::Legacy).is_ok());

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
            assert!(scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).is_err());
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
            assert!(scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).is_err());
        }
        for status in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(data(1, 100));
            s.extend(frame_end_sealed(2, 100, 0, 0, status, 0));
            assert!(scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).is_ok());
        }
        let mut later = Vec::new();
        later.extend(data(1, 100));
        later.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        later.extend(data(3, 101));
        later.extend(frame_end_sealed(4, 101, 0, 0, 0, 1));
        assert!(scan_stream(&later, &d17::schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D17,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &simple_payload(COMPACT_STATUS_WAR_BRIDGE_APPLIED, 3),
            )
        };
        let mut ok = Vec::new();
        ok.extend(perf(1, 100, 20));
        ok.extend(perf(2, 100, D17));
        ok.extend(perf(3, 100, PROBE_DOMAIN_ID));
        ok.extend(data(4, 100));
        ok.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
        assert!(scan_stream(&ok, &d17::schema(), StreamIdentityMode::Legacy).is_ok());

        let mut probes = Vec::new();
        probes.extend(perf(1, 100, PROBE_DOMAIN_ID));
        probes.extend(perf(2, 100, PROBE_DOMAIN_ID));
        probes.extend(data(3, 100));
        probes.extend(frame_end_sealed(4, 100, 0, 0, 0, 0));
        assert!(scan_stream(&probes, &d17::schema(), StreamIdentityMode::Legacy).is_ok());

        let mut duplicate = Vec::new();
        duplicate.extend(perf(1, 100, 20));
        duplicate.extend(perf(2, 100, 20));
        duplicate.extend(data(3, 100));
        duplicate.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        assert!(scan_stream(&duplicate, &d17::schema(), StreamIdentityMode::Legacy).is_err());

        let mut interleaved = Vec::new();
        interleaved.extend(data(1, 100));
        interleaved.extend(perf(2, 999, 20));
        interleaved.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&interleaved, &d17::schema(), StreamIdentityMode::Legacy).is_err());

        let mut stale = Vec::new();
        stale.extend(data(1, 100));
        stale.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        stale.extend(perf(3, 100, 20));
        stale.extend(data(4, 101));
        stale.extend(frame_end_sealed(5, 101, 1, 1, 0, 0));
        assert!(scan_stream(&stale, &d17::schema(), StreamIdentityMode::Legacy).is_err());

        let mut conflict = Vec::new();
        conflict.extend(perf(1, 100, 20));
        conflict.extend(perf(2, 101, 21));
        conflict.extend(data(3, 100));
        conflict.extend(frame_end_sealed(4, 100, 2, 2, 0, 0));
        assert!(scan_stream(&conflict, &d17::schema(), StreamIdentityMode::Legacy).is_err());

        for (emitted, scheduled) in [(1u32, 1u32), (3, 3), (2, 3), (3, 2)] {
            let mut s = Vec::new();
            s.extend(perf(1, 100, 20));
            s.extend(perf(2, 100, D17));
            s.extend(data(3, 100));
            s.extend(frame_end_sealed(4, 100, emitted, scheduled, 0, 0));
            assert!(scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).is_err());
        }

        let mut two_hours = Vec::new();
        two_hours.extend(perf(1, 100, 20));
        two_hours.extend(data(2, 100));
        two_hours.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        two_hours.extend(perf(4, 101, 20));
        two_hours.extend(data(5, 101));
        two_hours.extend(frame_end_sealed(6, 101, 1, 1, 0, 0));
        assert_eq!(
            scan_stream(&two_hours, &d17::schema(), StreamIdentityMode::Legacy)
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
                D17,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &simple_payload(COMPACT_STATUS_WAR_BRIDGE_APPLIED, 3),
            )
        };
        for (perf_hour, data_hour) in [(100i32, 101i32), (101, 100), (100, 200), (200, 100)] {
            let mut s = Vec::new();
            s.extend(perf(1, perf_hour, 20));
            s.extend(data(2, data_hour));
            s.extend(frame_end_sealed(3, data_hour, 1, 1, 0, 0));
            assert!(scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).is_err());
        }
        let mut agreeing = Vec::new();
        agreeing.extend(perf(1, 100, 20));
        agreeing.extend(data(2, 100));
        agreeing.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&agreeing, &d17::schema(), StreamIdentityMode::Legacy).is_ok());
    }

    #[test]
    fn an_excluded_record_is_still_checked_by_the_payload_authority() {
        const FAILED: u32 = 1;
        let mut bad = Vec::new();
        let mut changed = d17_payload(FAILED, &[], &[]);
        changed[12..16].copy_from_slice(&1u32.to_le_bytes());
        bad.push(changed);
        bad.push(d17_payload(FAILED, &[country_row(0, 0, 3, 0, 0)], &[]));
        bad.push(d17_payload(
            FAILED,
            &[country_row(7, 0, 3, 0, 0), country_row(7, 0, 4, 0, 0)],
            &[],
        ));
        bad.push(d17_payload(
            FAILED,
            &[country_row(7, 1 << 20, 3, 0, 0)],
            &[],
        ));
        bad.push(d17_payload(
            FAILED,
            &[country_row(7, d17::FLAG_AT_WAR, 3, 0, 0)],
            &[at_war_row(7, 3, b"GER")],
        ));
        bad.push(d17_payload(
            FAILED,
            &[country_row(7, d17::FLAG_AT_WAR, 3, 1, 0)],
            &[at_war_row(0, 3, b"GER")],
        ));
        for token_len in [0u32, 9] {
            bad.push(d17_payload(
                FAILED,
                &[country_row(7, d17::FLAG_AT_WAR, 3, 1, 0)],
                &[at_war_row(7, token_len, b"GER")],
            ));
        }
        bad.push(d17_payload(
            FAILED,
            &[country_row(7, d17::FLAG_AT_WAR, 3, 1, 0)],
            &[at_war_row(7, 3, b"G!R")],
        ));
        bad.push(d17_payload(
            FAILED,
            &[country_row(7, d17::FLAG_AT_WAR, 3, 1, 0)],
            &[at_war_row(7, 3, b"GERX")],
        ));
        bad.push(d17_payload(
            FAILED,
            &[country_row(7, d17::FLAG_AT_WAR, 3, 2, 0)],
            &[at_war_row(7, 3, b"GER"), at_war_row(7, 3, b"GER")],
        ));
        for payload in bad {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D17,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            let err = scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy)
                .err()
                .expect("malformed excluded payload must refuse");
            assert!(err.contains("authority"), "unexpected refusal: {err}");
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut payload = d17_payload(1 << bit, &[], &[]);
            payload[12..16].copy_from_slice(&1u32.to_le_bytes());
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D17,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            let err = scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy)
                .err()
                .expect("authority is independent of the failure bit");
            assert!(err.contains("authority"), "bit {bit}: {err}");
        }

        let excluded = d17_payload(FAILED, &[country_row(7, 0, 3, 0, 5)], &[]);
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D17,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &excluded,
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D17,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &simple_payload(COMPACT_STATUS_WAR_BRIDGE_APPLIED, 4),
        ));
        s.extend(frame_end(4, 101));
        let scan =
            scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.counts.failure_records, 1);
        assert_eq!(scan.hours.len(), 1);
        // Casualty zero-packing is a mapper/fold-only rule. The excluded record
        // above violates it and is still skipped rather than refused.
        assert_eq!(scan.hours[0].game_hour, 101);
    }

    #[test]
    fn the_identity_protocol_is_stated_not_sniffed() {
        use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;

        let decapitated = good_stream();
        assert!(scan_stream(&decapitated, &d17::schema(), StreamIdentityMode::IdentityV1).is_err());
        assert!(scan_stream(&decapitated, &d17::schema(), StreamIdentityMode::Legacy).is_ok());

        let mut identity_stream = Vec::new();
        identity_stream.extend(record(
            0,
            0,
            0,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            b"CID1-not-a-real-identity-payload",
        ));
        identity_stream.extend(&good_stream());
        assert!(scan_stream(&identity_stream, &d17::schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn scan_stream_never_panics_on_arbitrary_bytes() {
        // Total over hostile input: the harness reads a file from disk and must
        // return a diagnosis, never unwind.
        let schema = d17::schema();
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
    fn an_hour_closed_without_a_d17_record_refuses_the_stream() {
        // The completeness bound on what this harness does NOT validate (module
        // docs): it cannot check other domains' payloads, so a d17 record
        // relabelled to another domain id — or dropped — must be caught by
        // counting, not silently measured as fewer hours.
        let mut s = good_stream();
        // Find the LAST d17 DATA record and relabel it to an unclaimed domain
        // id. The stream stays well formed — sequence, hour boundaries and
        // FRAME_END are untouched — so the completeness rule is the only rule
        // that can catch it.
        let mut pos = 0usize;
        let mut target = None;
        while pos < s.len() {
            let header = parse_compact_record_header(&s[pos..]).expect("well-formed");
            if header.domain_id == D17 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
            {
                target = Some(pos);
            }
            pos += COMPACT_RECORD_HEADER_LEN + header.payload_len;
        }
        let target = target.expect("good_stream carries a d17 DATA record");
        // domain_id sits at +20: magic(4) + version(2) + header_len(2) +
        // sequence(8) + game_hour(4).
        s[target + 20..target + 22].copy_from_slice(&99u16.to_le_bytes());
        let err = match scan_stream(&s, &d17::schema(), StreamIdentityMode::Legacy) {
            Err(e) => e,
            Ok(_) => panic!("an hour with no d17 record must refuse the stream"),
        };
        assert!(
            err.contains("game hour(s) but carries"),
            "the completeness check must be what rejects it: {err}"
        );
    }
}
