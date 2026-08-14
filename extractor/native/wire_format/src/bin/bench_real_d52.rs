//! Measure the wire format against the shipping compact stream on REAL d52
//! `occupation_resistance_compliance_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d52-scheduled capture
//! (`runs/batch3/deepwar_a`). The stream's own d52 DATA records are the shipping
//! format's cost for the domain: a 52-byte header, a fixed 64-byte row per
//! changed state, and up to two trailers — exactly what the recorder stores
//! today. This tool replays those records through `wire_format::d52`'s own
//! `State::fold_payload`, the same fold
//! `world_fold::fold_occupation_resistance_compliance` performs, and re-encodes
//! every hour with this crate's grammar: a BASELINE at each 48-hour block start,
//! field-granular stored-minimal DELTAs between.
//!
//! **Nothing here re-derives a contract it can call.** Header, row, trailer and
//! accumulator semantics go through `wire_format::d52`'s public boundary
//! (`split_payload`, `parse_trailers`, `record_from_*`, `key_from_*`,
//! `State::fold_payload`, `check_authority_payload`); H4CR framing, record
//! semantics and sequence continuity go through `compact_contract`'s own parser
//! and stateful validator.
//!
//! # Two passes, because the base table is an accumulator
//!
//! d52 is baseline-plus-delta, so an hour's state is the whole history of the
//! stream up to that hour rather than that hour's rows. Pass 1 validates the
//! WHOLE stream and folds it, retaining only each usable record's byte span;
//! pass 2 re-folds through the same `State::fold_payload` while encoding. No
//! artifact is written until pass 1 succeeds.
//!
//! # What pass 1 enforces, fail-closed
//!
//! - every record header through `compact_contract::parse_compact_record_header`;
//! - every record through `compact_contract::stream_validator::StatefulStreamValidator`;
//! - the hour boundary rules of `recorder_host::stream_fold::BoundaryTracker`
//!   (`recorder_host/src/stream_fold.rs:277`), its DOMAIN_PERF hour binding
//!   (`:328`-`:365`), the FRAME_END seal fields (`:46`-`:89`) with the seal's
//!   domain-count cross-check (`:411`-`:419`), and `eof_check` (`:459`).
//!   `wire_format` must not depend on `recorder_host`, so that documented subset
//!   is re-implemented here exactly as `bench_real_d63.rs` does. The DATA
//!   payloads of every domain other than 52 pass UNVALIDATED beyond header
//!   sanity, the stream validator and the hour-boundary rules — production
//!   validates every domain's payload (`recorder_host/src/stream_fold.rs:593`). That unvalidated
//!   remainder is explicitly BOUNDED: every closed game hour must carry exactly
//!   one d52 DATA record of any status (the `d52_records == closed_hours`
//!   completeness check in [`scan_stream`]), so a d52 record relabelled to
//!   another domain id — or dropped — refuses the run instead of silently
//!   shrinking it;
//! - **one d52 DATA record per game hour, over EVERY d52 record and BEFORE any
//!   status exclusion**, so a success+failure pair in one hour cannot slip
//!   through;
//! - **unassigned v1 status bits refused BEFORE classification**
//!   (`compact_contract/src/status_flags_base.rs:40`-`:44`): the failure helper
//!   classifies the assigned vocabulary and is not a validity predicate, so an
//!   unassigned bit would otherwise read as "not a failure" and be measured as a
//!   healthy hour;
//! - production's SECOND authority on every DOMAIN_DATA record whatever its
//!   status — `d52::check_authority_payload`, which is
//!   `lanes/occupation_resistance_compliance_live/domain44_compact.rs`'s own
//!   decoder. Any complaint there makes the record `malformed_payload_records`,
//!   which `strict_stream_certification_verdict`
//!   (`recorder_host/src/stream_fold.rs:507`-`:510`) refuses.
//!
//! # The three outcomes, named
//!
//! Every non-folding path of `fold_occupation_resistance_compliance` is
//! classified explicitly:
//!
//! 1. **Stream rejection** — every `Err` the fold can return: a payload shorter
//!    than the header or than its own rows (`world_fold.rs:16801`-`:16811`), a
//!    malformed `GAR1`/`COL1` trailer (`:16813`-`:16821`), and every row-loop
//!    refusal (`:16876`-`:16983`). These invalidate the run.
//! 2. **Excluded hour, failure status** (`:16825`-`:16835`): the fold marks the
//!    domain-hour unknown, retains the accumulator, and — this is what makes the
//!    hour safe to skip — **overwrites** the decoded `GAR1` alert with `Null` and
//!    clears the `COL1` rows (`:16827`-`:16829`). Nothing of that hour survives
//!    the fold, so excluding it from both arms loses nothing.
//! 3. **Run rejection, delta before baseline** (`:16836`-`:16845`). This path is
//!    NOT interchangeable with the one above and is not an excluded hour. The
//!    fold returns without touching the trailers, so the `GAR1`/`COL1` data
//!    decoded at `:16813`-`:16820` stays live and is published at `:5218`-`:5268`
//!    and `:6014`-`:6019` — real folded data on an hour whose base table is
//!    unknown. A keyed set cannot express "this collection's membership was not
//!    measured this hour" (`WIRE_FORMAT.md` §13, the gap that stopped d17), so
//!    the hour can be neither encoded faithfully nor silently skipped: skipping
//!    it would charge the shipping arm for trailer bytes the wire arm was never
//!    allowed to encode. The harness therefore **refuses the whole run** and says
//!    why. The measured capture contains no such record (its first frame is the
//!    baseline), so no measured number depends on this choice.
//!
//! **The exclusion's POSITION is the fold's own.** The fold decodes the trailers
//! BEFORE it reads the status word, so this harness parses and validates the
//! trailers on every record — including the ones it excludes — and only then
//! applies the status exclusion. Copying d63's "status first" order here would
//! accept a stream carrying a corrupt trailer on a failure-status hour, which the
//! recorder refuses.
//!
//! # Other writers of this domain's accumulator
//!
//! One, and it is **proved inert structurally rather than per capture**: d66
//! `occupation_state_extras_live` writes into the same JSON `occupation` object
//! (`world_fold.rs:12452`-`:12480`). It removes and inserts exactly five
//! TOP-LEVEL keys — `occupation_law_token_id`, `occupation_law_token_encoding`,
//! `occupation_controller_tag_db_id`, `occupied_tag_db_id`,
//! `garrison_manpower_source_tag_db_id` — and d52's slice is
//! `resistance`, `compliance`, `resistance_speed`, `compliance_speed`,
//! `occupier_tag`, `occupied_country_tag` plus everything nested inside
//! `_r2_occupation_resistance_compliance_live`. The two key sets are disjoint,
//! and d66 never removes the `occupation` object nor any state, so no d66 frame
//! can change what this port models. (d52's own non-delta clear removes d66's
//! keys along with its own, which is d52 clobbering d66 and not the reverse.)
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d52.bin   — the d52 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line

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
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedDelta,
    KeyedState,
};
use wire_format::d52::{self, FoldOutcome, State};

const D52: u16 = 52;
/// The default re-baseline cadence, as every earlier bench bin uses. It is an
/// argument rather than a constant because the block ratio measures re-baseline
/// POLICY as much as grammar: the shipping lane baselines once per campaign and
/// this arm baselines every `BLOCK_HOURS`, so a run at the shipping cadence is
/// what makes the two grammars comparable. See the measurement write-up.
const DEFAULT_BLOCK_HOURS: usize = 48;

/// One usable d52 hour: its game hour, whether its payload was a baseline, and
/// the byte span of its record in the source stream (the shipping arm is those
/// bytes verbatim, header included).
#[derive(Debug)]
struct D52Hour {
    game_hour: i32,
    is_baseline: bool,
    /// True when this measured hour is exactly one game hour after the previous
    /// MEASURED hour. False on the first measured hour and after any gap — an
    /// excluded failure-status hour or a non-contiguous hour step — which is
    /// what forces the encoder to open a fresh baseline (`opens_baseline`).
    follows_previous: bool,
    /// The `row_count` the shipping payload carried — the collector's own notion
    /// of how many rows changed this hour, reported beside the wire arm's.
    shipping_rows: u32,
    record_span: Range<usize>,
    payload_span: Range<usize>,
}

/// Whether the wire arm opens a BASELINE at this measured hour.
///
/// Two reasons, and both are `WIRE_FORMAT.md` rules, not conveniences:
/// the block cadence (every block opens with its own baseline, §1.5's closing
/// paragraph), and a gap — `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the
/// state part forces the next state observation to be a `BASELINE`." Every
/// hour this domain did not successfully sample is such a gap — the excluded
/// failure-status hour, and any non-contiguous hour step — and d52's cadence
/// is hourly, so a measured hour that does not follow the previous measured
/// one by exactly one is preceded by one. Neither a delta nor a retained
/// reader state can be justified across it. `block_hours` is this bin's
/// runtime block cadence (`DEFAULT_BLOCK_HOURS` unless overridden).
fn opens_baseline(hours_seen: usize, block_hours: usize, follows_previous: bool) -> bool {
    hours_seen % block_hours == 0 || !follows_previous
}

/// One wire-arm hour: the §1.5/block baseline decision, the encode, and the
/// per-hour reader replay over every collection. Factored out of `main` so the
/// tests execute the PRODUCTION encode path over a scanned stream: a mutant
/// that reverts the baseline decision here to block-position-only fails the
/// gap test (a Codex round-1 finding — the earlier tests exercised the
/// decision function but not this callsite).
struct WireHourOutcome {
    opened_baseline: bool,
    /// The baseline sits outside a scheduled block start — §1.5 gap-forced.
    gap_forced: bool,
    emitted: bool,
    changed_by_collection: Vec<usize>,
    tombstones_by_collection: Vec<usize>,
}

#[allow(clippy::too_many_arguments)]
fn encode_wire_hour(
    schema: &wire_format::Schema,
    hours_seen: usize,
    block_hours: usize,
    follows_previous: bool,
    game_hour: i32,
    prior: Option<&[KeyedState]>,
    state: &[KeyedState],
    reader: &mut Option<Vec<KeyedState>>,
    wire: &mut Vec<u8>,
) -> WireHourOutcome {
    let hour_start = wire.len();
    // BASELINE at each block start — and at every §1.5 gap, which forces a
    // fresh baseline and restarts the delta chain (`opens_baseline`).
    let block_start = opens_baseline(hours_seen, block_hours, follows_previous);
    let gap_forced = block_start && hours_seen % block_hours != 0;
    let mut emitted = true;
    let mut changed_by_collection = vec![0usize; schema.collections.len()];
    let mut tombstones_by_collection = vec![0usize; schema.collections.len()];
    let opened_baseline = block_start || prior.is_none();
    if opened_baseline {
        encode_baseline(schema, state, wire).expect("baseline encodes");
    } else {
        let prev = prior.expect("a prior exists past the first hour");
        let deltas: Vec<Option<KeyedDelta>> = schema
            .collections
            .iter()
            .enumerate()
            .map(|(i, collection)| {
                let d = diff(collection, &prev[i], &state[i]);
                if d.is_empty() {
                    None
                } else {
                    Some(d)
                }
            })
            .collect();
        if deltas.iter().all(Option::is_none) {
            emitted = false;
        } else {
            for (i, delta) in deltas.iter().enumerate() {
                if let Some(delta) = delta {
                    changed_by_collection[i] = delta.changed.len();
                    tombstones_by_collection[i] = delta.tombstones.len();
                }
            }
            encode_delta(schema, prev, &deltas, wire).expect("delta encodes");
        }
    }

    // Reader replay: decode the bytes just written against the reader's OWN
    // reconstructed prior and require the result to be stored-equal to this
    // hour's folded state. An hour that emitted nothing is still checked.
    if emitted {
        let hour_bytes = &wire[hour_start..];
        if block_start || reader.is_none() {
            let (states, consumed) =
                decode_baseline(schema, hour_bytes, 0).expect("baseline decodes");
            assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
            *reader = Some(states);
        } else {
            let reader_prior = reader.take().expect("a reader prior exists");
            let (deltas, consumed) =
                decode_delta(schema, &reader_prior, hour_bytes, 0).expect("delta decodes");
            assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
            let mut next = reader_prior;
            for (state, delta) in next.iter_mut().zip(deltas.iter()) {
                if let Some(delta) = delta {
                    apply(state, delta);
                }
            }
            *reader = Some(next);
        }
    }
    let replayed = reader.as_ref().expect("a reader state exists");
    for (i, collection) in schema.collections.iter().enumerate() {
        assert_eq!(
            replayed[i].len(),
            state[i].len(),
            "game hour {game_hour}: collection {} row count",
            collection.collection_id
        );
        assert!(
            diff(collection, &replayed[i], &state[i]).is_empty(),
            "game hour {game_hour}: collection {} is not stored-equal to the fold",
            collection.collection_id
        );
    }
    WireHourOutcome {
        opened_baseline,
        gap_forced,
        emitted,
        changed_by_collection,
        tombstones_by_collection,
    }
}

#[derive(Debug)]
struct StreamScan {
    hours: Vec<D52Hour>,
    /// d52 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// d52 records that arrived as a delta before any baseline. Always zero on
    /// an accepted stream — the scan refuses the run on the first one — and kept
    /// as a field so the refusal has something to report.
    delta_before_baseline_records: usize,
    /// Measured d52 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// Hours whose payload carried a `GAR1` trailer, and of those, how many were
    /// the `complete` form rather than `source unavailable`.
    gar1_hours: usize,
    gar1_complete_hours: usize,
    gar1_equipment_rows: usize,
    /// Same for `COL1`.
    col1_hours: usize,
    col1_complete_hours: usize,
    col1_relation_rows: usize,
    /// Union of every row flag bit seen, and of every status bit seen.
    row_flags_or: u32,
    status_flags_or: u32,
}

/// Every globally-defined compact status occupies one of bits 0..=14
/// (`recorder_host/src/stream_fold.rs:43`).
// Re-exported, not re-declared: a local `(1 << 14) - 1` became wrong the moment
// the contract assigned bit 14 to the measured-empty badge.
const FRAME_END_KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;

/// The reader-context probe domain, excluded from the seal's domain accounting
/// (`recorder_host/src/stream_fold.rs:330`).
const PROBE_DOMAIN_ID: u16 = 250;

/// The FRAME_END seal check, re-implemented from
/// `recorder_host::stream_fold::validate_frame_end_seal_fields`
/// (`recorder_host/src/stream_fold.rs:46`-`:89`) because `wire_format` must not
/// depend on `recorder_host`. Total: no payload of any length panics.
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
            "FRAME_END seal violation at byte {at_byte}: reserved word is nonzero ({reserved:#010x})"
        ));
    }
    if emitted_domains != scheduled_domains {
        return Err(format!(
            "FRAME_END seal violation at byte {at_byte}: emitted domain count {emitted_domains} \
             does not match scheduled domain count {scheduled_domains}"
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

/// Validate the whole stream against the canonical contract and fold every
/// usable d52 DATA record. Every rejection path is an `Err`, never a silently
/// shorter measurement and never a panic.
fn scan_stream(
    stream: &[u8],
    schema: &wire_format::Schema,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    // The identity protocol is an INPUT, never sniffed: an `IdentityV1` stream
    // whose mandatory CID1 record has been spliced off is byte-indistinguishable
    // from a legacy stream, so a harness that sniffs accepts a decapitated
    // identity stream and measures it.
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    // COMPLETENESS accounting: every d52 DATA record of ANY status, against
    // every closed game hour — see the check after the EOF rules.
    let mut d52_records = 0usize;
    let mut closed_hours = 0usize;
    let mut scan = StreamScan {
        hours: Vec::new(),
        failure_records: 0,
        delta_before_baseline_records: 0,
        non_contiguous: 0,
        gar1_hours: 0,
        gar1_complete_hours: 0,
        gar1_equipment_rows: 0,
        col1_hours: 0,
        col1_complete_hours: 0,
        col1_relation_rows: 0,
        row_flags_or: 0,
        status_flags_or: 0,
    };

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d52 DATA record of ANY status — the uniqueness rule.
    let mut last_d52_hour: Option<i32> = None;
    // Hour of the last d52 record that entered the measurement.
    let mut last_measured_hour: Option<i32> = None;
    // Pass 1 folds into a throwaway accumulator, because every way a row can be
    // rejected is a property of the stream and so belongs to the pass that either
    // accepts the stream or refuses it — and because `delta_before_baseline` is
    // decided from the accumulator's own baseline bit.
    let mut fold = State::new();

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
                            "hour interleaving at byte {pos}: DOMAIN_PERF for hour {} while hour \
                             {open} is still open",
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
                            "FRAME_END at byte {pos} closes hour {} without an open DOMAIN_DATA \
                             hour",
                            header.game_hour_raw
                        ))
                    }
                }
                if let Some(last_closed) = last_closed_hour {
                    if header.game_hour_raw <= last_closed {
                        return Err(format!(
                            "non-monotonic FRAME_END at byte {pos}: closed hour {last_closed}, \
                             then {}",
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

        if header.domain_id == D52 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d52_records += 1;
            // One d52 DATA record per game hour, over EVERY d52 record and
            // BEFORE any status-based exclusion: gating this on the record being
            // usable would let a success/failure pair through the rule entirely.
            if let Some(previous) = last_d52_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d52 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d52_hour = Some(header.game_hour_raw);

            // VALIDITY BEFORE CLASSIFICATION. Bits 15..=31 are unassigned and an
            // unassigned bit has no defined classification, so it refuses the
            // stream rather than falling through to "success".
            let unknown_status = compact_status_unknown_flags_v1(
                d52::peek_header(payload).map_err(|e| format!("d52 payload at byte {pos}: {e}"))?
                    .status_flags,
            );
            if unknown_status != 0 {
                return Err(format!(
                    "d52 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            // Production's SECOND authority, which runs on every DOMAIN_DATA
            // record whatever its status and whose complaint refuses the stream
            // at certification. Before the exclusion, or a record the recorder
            // would refuse gets quietly skipped instead.
            d52::check_authority_payload(payload)
                .map_err(|e| format!("d52 payload at byte {pos}: {e}"))?;

            let (d52_header, rows, trailer_bytes) = d52::split_payload(payload)
                .map_err(|e| format!("d52 payload at byte {pos}: {e}"))?;
            scan.status_flags_or |= d52_header.status_flags;

            // The fold decodes the trailers BEFORE the status check, so a
            // malformed trailer refuses the stream even on an excluded hour.
            let trailers = d52::parse_trailers(trailer_bytes)
                .map_err(|e| format!("d52 trailer at byte {pos}: {e}"))?;
            if let Some(alert) = &trailers.garrison_alert {
                scan.gar1_hours += 1;
                if !alert.source_unavailable() {
                    scan.gar1_complete_hours += 1;
                }
                scan.gar1_equipment_rows += alert.equipment.len();
            }
            if let Some(collaboration) = &trailers.collaboration {
                scan.col1_hours += 1;
                if !collaboration.source_unavailable() {
                    scan.col1_complete_hours += 1;
                }
                scan.col1_relation_rows += collaboration.rows.len();
            }
            for row in rows.chunks_exact(d52::ROW_LEN) {
                scan.row_flags_or |= d52::row_flags(row)
                    .map_err(|e| format!("d52 row at byte {pos}: {e}"))?;
            }

            // `handle_compact_failure`-equivalent (`world_fold.rs:16825`): the
            // predicate is `compact_contract`'s own, not a copied bit mask, so it
            // cannot drift from the recorder.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D52,
                d52_header.status_flags,
            ) != 0
            {
                scan.failure_records += 1;
            } else {
                match fold
                    .fold_payload(schema, payload)
                    .map_err(|e| format!("d52 payload at byte {pos}: {e}"))?
                {
                    FoldOutcome::DeltaBeforeBaseline => {
                        scan.delta_before_baseline_records += 1;
                        // NOT an excluded hour. The fold leaves this hour's
                        // trailers live and the recorder publishes them, while
                        // the base table is unknown — a mixed availability no
                        // keyed set can express. Skipping the hour would drop
                        // recorder-folded data and still charge the shipping arm
                        // for its bytes, so the run is invalid instead.
                        return Err(format!(
                            "d52 DATA at byte {pos}: hour {} is a delta before any baseline.                              The fold folds nothing into the base table but RETAINS this                              hour's GAR1/COL1 trailers (world_fold.rs:16838-16844) and the                              recorder publishes them, so the hour carries measured trailer                              data alongside an unknown base table. A keyed set cannot express                              an unmeasured collection (WIRE_FORMAT.md section 13), so this run                              is refused rather than measured with the hour skipped",
                            header.game_hour_raw
                        ));
                    }
                    outcome => {
                        let follows_previous = last_measured_hour
                            .is_some_and(|previous| header.game_hour_raw == previous + 1);
                        if last_measured_hour.is_some() && !follows_previous {
                            scan.non_contiguous += 1;
                        }
                        last_measured_hour = Some(header.game_hour_raw);
                        scan.hours.push(D52Hour {
                            game_hour: header.game_hour_raw,
                            is_baseline: outcome == FoldOutcome::Baseline,
                            follows_previous,
                            shipping_rows: d52_header.row_count,
                            record_span: pos..record_end,
                            payload_span: payload_start..record_end,
                        });
                    }
                }
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
    if scan.hours.is_empty() {
        return Err("stream carries no usable d52 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d52 DATA
    // record, of any status. This bounds what this harness canNOT validate:
    // production runs `validate_domain_data_payload_at` on EVERY domain's DATA
    // payload (`recorder_host/src/stream_fold.rs:593`), while this harness owns only d52's
    // grammar, so a d52 record whose header was relabelled to another domain
    // id — or lost outright — would otherwise be silently ignored and the run
    // would report a plausible measurement over fewer hours. A per-capture
    // completeness assertion, NOT a grammar rule: a capture where d52 is not
    // scheduled every hour fails here loudly and a human adjusts, instead of a
    // number quietly shrinking (`bench_real_d65.rs` precedent).
    if d52_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d52_records} d52 DATA \
             record(s); every closed hour of a d52-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    Ok(scan)
}

fn snapshot(state: &State) -> Vec<KeyedState> {
    state.collections().iter().map(|c| (*c).clone()).collect()
}

fn main() {
    const USAGE: &str = "usage: bench_real_d52 <collector_stream.bin> <out dir> \
                         [legacy|identity-v1]";
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };
    let block_hours: usize = match args.next() {
        None => DEFAULT_BLOCK_HOURS,
        Some(raw) => raw
            .parse()
            .ok()
            .filter(|n| *n > 0)
            .unwrap_or_else(|| panic!("block hours must be a positive integer; {USAGE}")),
    };
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d52::schema();

    // ---- Pass 1: validate the WHOLE stream and fold its d52 DATA records. ---
    let scan = match scan_stream(&stream, &schema, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: re-fold through the SAME boundary while encoding. ----------

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut fold = State::new();
    let mut prior: Option<Vec<KeyedState>> = None;
    let mut reader: Option<Vec<KeyedState>> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut gap_forced_baselines = 0usize;
    // Whether each measured hour opened a baseline in the wire arm, for the
    // baseline/delta byte split below — with §1.5 gap-forced baselines the
    // block-position rule alone no longer decides it.
    let mut baseline_flags: Vec<bool> = Vec::new();
    let mut empty_hours = 0usize;
    // Cadence-independent churn: changed rows over MEASURED-HOUR transitions,
    // taken before the block decision, so a change landing on a block boundary
    // still counts.
    let mut transition_changed = [0usize; 5];
    let mut transition_tombstones = [0usize; 5];
    let mut transitions = 0usize;
    // Delta-path churn: over the hours that actually emitted a delta.
    let mut delta_changed = [0usize; 5];
    let mut delta_tombstones = [0usize; 5];
    let mut delta_hours = 0usize;
    let mut rows_total = 0usize;
    let mut shipping_rows_total = 0usize;
    let mut shipping_payload_bytes = 0usize;

    for hour in &scan.hours {
        ship.extend_from_slice(&stream[hour.record_span.clone()]);
        ship_offsets.push(ship.len());
        shipping_payload_bytes += hour.payload_span.len();
        shipping_rows_total += hour.shipping_rows as usize;

        // Re-fold through the same public boundary pass 1 used. Any error here
        // would be a disagreement between the two passes, not stream content.
        fold.fold_payload(&schema, &stream[hour.payload_span.clone()])
            .unwrap_or_else(|e| panic!("game hour {}: pass 2 disagrees with pass 1: {e}", hour.game_hour));
        let state = snapshot(&fold);
        rows_total += state[0].len();

        // Cadence-independent churn, before the block decision.
        if let Some(prev) = &prior {
            transitions += 1;
            for (i, collection) in schema.collections.iter().enumerate() {
                let d = diff(collection, &prev[i], &state[i]);
                transition_changed[i] += d.changed.len();
                transition_tombstones[i] += d.tombstones.len();
            }
        }

        // The wire-arm hour — baseline decision, encode and reader replay —
        // through the SAME function the tests execute (`encode_wire_hour`).
        let outcome = encode_wire_hour(
            &schema,
            hours_seen,
            block_hours,
            hour.follows_previous,
            hour.game_hour,
            prior.as_deref(),
            &state,
            &mut reader,
            &mut wire,
        );
        if outcome.gap_forced {
            gap_forced_baselines += 1;
        }
        baseline_flags.push(outcome.opened_baseline);
        if outcome.opened_baseline {
            baselines += 1;
        } else if outcome.emitted {
            delta_hours += 1;
            for (i, _) in schema.collections.iter().enumerate() {
                delta_changed[i] += outcome.changed_by_collection[i];
                delta_tombstones[i] += outcome.tombstones_by_collection[i];
            }
        } else {
            empty_hours += 1;
        }
        wire_offsets.push(wire.len());

        prior = Some(state);
        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d52.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d52.offsets", &ship_offsets),
        ("wire_new.offsets", &wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    // The wire arm's baseline/delta raw split, from the per-hour baseline
    // decisions the encode loop actually made (a §1.5 gap-forced baseline is a
    // baseline wherever it sits in the block, so block position alone cannot
    // classify these bytes).
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    for (index, window) in wire_offsets.windows(2).enumerate() {
        let len = window[1] - window[0];
        if baseline_flags[index] {
            baseline_bytes += len;
        } else {
            delta_bytes += len;
        }
    }

    println!(
        "hours                {hours_seen} ({} non-contiguous steps)",
        scan.non_contiguous
    );
    println!(
        "excluded hours       {} failure-status (a delta-before-baseline record          refuses the run; {} seen)",
        scan.failure_records, scan.delta_before_baseline_records
    );
    println!(
        "shipping frames      {} baseline, {} delta (status OR {:#x})",
        scan.hours.iter().filter(|h| h.is_baseline).count(),
        scan.hours.iter().filter(|h| !h.is_baseline).count(),
        scan.status_flags_or
    );
    println!(
        "shipping rows        {shipping_rows_total} total, {:.2}/hour",
        shipping_rows_total as f64 / hours_seen as f64
    );
    println!("row flags observed   {:#06x}", scan.row_flags_or);
    println!(
        "states accumulated   {:.1} avg (max {})",
        rows_total as f64 / hours_seen as f64,
        prior.as_ref().map_or(0, |p| p[0].len())
    );
    println!(
        "GAR1 trailers        {} hours ({} complete, {} equipment rows)",
        scan.gar1_hours, scan.gar1_complete_hours, scan.gar1_equipment_rows
    );
    println!(
        "COL1 trailers        {} hours ({} complete, {} relation rows)",
        scan.col1_hours, scan.col1_complete_hours, scan.col1_relation_rows
    );
    println!(
        "baselines            {baselines} ({} at a {block_hours}-h block start, \
         {gap_forced_baselines} forced by a WIRE_FORMAT \u{a7}1.5 state gap)",
        baselines - gap_forced_baselines
    );
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!("churn, cadence-independent (over {transitions} measured-hour transitions):");
    for (i, collection) in schema.collections.iter().enumerate() {
        println!(
            "  collection {}       {:.3} changed rows/hour, {:.3} tombstones/hour",
            collection.collection_id,
            transition_changed[i] as f64 / transitions.max(1) as f64,
            transition_tombstones[i] as f64 / transitions.max(1) as f64
        );
    }
    println!("churn, delta path (over {delta_hours} hours that emitted a delta):");
    for (i, collection) in schema.collections.iter().enumerate() {
        println!(
            "  collection {}       {:.3} changed rows/hour, {:.3} tombstones/hour",
            collection.collection_id,
            delta_changed[i] as f64 / delta_hours.max(1) as f64,
            delta_tombstones[i] as f64 / delta_hours.max(1) as f64
        );
    }
    println!(
        "ship_d52.bin         {} bytes ({} payload + {} framing)",
        ship.len(),
        shipping_payload_bytes,
        ship.len() - shipping_payload_bytes
    );
    println!(
        "wire_new.bin         {} bytes ({:.2}%) — {baseline_bytes} baseline + {delta_bytes} delta",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{
        COMPACT_DOMAIN_FRAME, COMPACT_DOMAIN_PERF_BASE_LEN, COMPACT_STATUS_DELTA_DATA,
    };

    /// Build one H4CR record the way the recorder frames it
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

    /// A one-state d52 DATA payload with a `GAR1` trailer in the shape every hour
    /// of the measured capture carries.
    fn d52_payload(status_flags: u32, state_id: u32, resistance: i32) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&1u32.to_le_bytes()); // row_count
        p.extend_from_slice(&1u32.to_le_bytes()); // changed_rows
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&1u32.to_le_bytes()); // observed_state_rows
        p.extend_from_slice(&0u32.to_le_bytes()); // occupied_state_rows
        p.extend_from_slice(&0u32.to_le_bytes()); // wall_badge_rows
        p.extend_from_slice(&100_000u32.to_le_bytes()); // scale_raw
        p.extend_from_slice(&0u32.to_le_bytes()); // law_token_rows
        p.extend_from_slice(&0u32.to_le_bytes()); // garrison_template_rows
        p.extend_from_slice(&0u32.to_le_bytes()); // garrison_scalar_rows
        p.extend_from_slice(&0u32.to_le_bytes()); // garrison_anomaly_rows
        // invalid_state_rows: one root walked, one row emitted, so zero drops.
        // The full-frame identity `row_count + invalid == observed` is checked by
        // the lane decoder, and this fixture is the shape a real hour carries.
        p.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(p.len(), d52::HEADER_LEN);
        let mut row = vec![0u8; d52::ROW_LEN];
        row[0..4].copy_from_slice(&state_id.to_le_bytes());
        row[16..20].copy_from_slice(&resistance.to_le_bytes());
        row[52..56].copy_from_slice(
            &(d52::ROW_FLAG_RESISTANCE
                | d52::ROW_FLAG_COMPLIANCE
                | d52::ROW_FLAG_RESISTANCE_SPEED
                | d52::ROW_FLAG_COMPLIANCE_SPEED)
                .to_le_bytes(),
        );
        p.extend_from_slice(&row);
        p.extend_from_slice(&gar1_unavailable(0));
        p
    }

    fn gar1_unavailable(game_hour: i32) -> Vec<u8> {
        let mut t = Vec::new();
        t.extend_from_slice(d52::GARRISON_ALERT_MAGIC);
        t.extend_from_slice(&1u16.to_le_bytes());
        t.extend_from_slice(&(d52::GARRISON_ALERT_HEADER_LEN as u16).to_le_bytes());
        t.extend_from_slice(&game_hour.to_le_bytes());
        t.extend_from_slice(&0u32.to_le_bytes()); // controller
        t.extend_from_slice(&d52::GARRISON_ALERT_FLAG_SOURCE_UNAVAILABLE.to_le_bytes());
        t.extend_from_slice(&0i64.to_le_bytes()); // missing_manpower
        t.extend_from_slice(&0u32.to_le_bytes()); // equipment_count
        t.extend_from_slice(&100_000u32.to_le_bytes()); // divisor
        t.extend_from_slice(&0u32.to_le_bytes()); // reserved
        assert_eq!(t.len(), d52::GARRISON_ALERT_HEADER_LEN);
        t
    }

    /// Two closed hours: a baseline then a delta, each carrying one d52 record.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(1, 100, D52, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d52_payload(0, 7, 10)));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D52,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d52_payload(COMPACT_STATUS_DELTA_DATA, 7, 11),
        ));
        s.extend(frame_end(4, 101));
        s
    }

    fn scan(stream: &[u8]) -> Result<StreamScan, String> {
        scan_stream(stream, &d52::schema(), StreamIdentityMode::Legacy)
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
    fn a_well_formed_stream_yields_its_d52_hours() {
        let s = scan(&good_stream()).expect("accepted");
        assert_eq!(s.hours.len(), 2);
        assert_eq!(s.hours[0].game_hour, 100);
        assert!(s.hours[0].is_baseline);
        assert!(!s.hours[1].is_baseline);
        assert_eq!(s.failure_records, 0);
        assert_eq!(s.delta_before_baseline_records, 0);
        assert_eq!(s.non_contiguous, 0);
        assert_eq!(s.gar1_hours, 2);
        assert_eq!(s.gar1_complete_hours, 0);
        assert_eq!(s.col1_hours, 0);
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // The failure helper classifies the assigned v1 vocabulary and is not a
        // validity predicate, so an unassigned bit must be rejected BEFORE
        // classification — otherwise it silently reads as "not a failure".
        // Bit 15 is ASSIGNED (`COMPACT_STATUS_BASELINE_CLEAR`, the stream's deliberate
        // resync primitive), so the UNASSIGNED run starts at 16.
        for bit in 16..32u32 {
            for extra in [0u32, 1, COMPACT_STATUS_DELTA_DATA] {
                let mut s = Vec::new();
                s.extend(record(
                    1,
                    100,
                    D52,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d52_payload((1 << bit) | extra, 7, 10),
                ));
                s.extend(frame_end(2, 100));
                assert!(
                    scan(&s).is_err(),
                    "status bit {bit} (+{extra:#x}) must be refused"
                );
            }
        }
        // And every informational bit is still accepted.
        for informational in [1u32 << 4, 1 << 5, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D52,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d52_payload(informational, 7, 10),
            ));
            s.extend(frame_end(2, 100));
            let scanned = scan(&s).unwrap_or_else(|e| panic!("bit {informational:#x}: {e}"));
            assert_eq!(scanned.hours.len(), 1, "bit {informational:#x}");
        }
    }

    #[test]
    fn an_excluded_hour_forces_the_next_observation_to_be_a_baseline() {
        // `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the state part forces
        // the next state observation to be a `BASELINE`." Both gap shapes d52
        // can meet — an excluded failure-status hour, and a whole hour the
        // capture never scheduled — must clear `follows_previous`, and
        // `opens_baseline` must open a baseline on it wherever it lands in a
        // block. A delta would relate two states across unobserved time.
        //
        // Shape 1: hour 101 carries a failure-status d52 record (excluded).
        let mut s = Vec::new();
        s.extend(record(1, 100, D52, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d52_payload(0, 7, 10)));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D52,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d52_payload(1 << 0, 7, 11), // failure status -> excluded
        ));
        s.extend(frame_end(4, 101));
        s.extend(record(
            5,
            102,
            D52,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d52_payload(COMPACT_STATUS_DELTA_DATA, 7, 12),
        ));
        s.extend(frame_end(6, 102));
        let scanned = scan(&s).expect("well-formed");
        assert_eq!(scanned.hours.len(), 2);
        assert_eq!(scanned.failure_records, 1);
        assert_eq!(scanned.non_contiguous, 1);
        assert!(
            !scanned.hours[0].follows_previous,
            "the first hour follows none"
        );
        assert!(
            !scanned.hours[1].follows_previous,
            "hour 102 does not follow hour 100"
        );
        // ...and that is what makes the encoder open a baseline for it, even
        // though it is only the second measured hour of the block — through the
        // PRODUCTION encode path, not just the decision function.
        assert!(opens_baseline(
            1,
            DEFAULT_BLOCK_HOURS,
            scanned.hours[1].follows_previous
        ));
        let schema = d52::schema();
        let mut fold = State::new();
        let mut states = Vec::new();
        for hour in &scanned.hours {
            fold.fold_payload(&schema, &s[hour.payload_span.clone()])
                .expect("pass 1 accepted this payload");
            states.push(snapshot(&fold));
        }
        let mut wire = Vec::new();
        let mut reader: Option<Vec<KeyedState>> = None;
        let o0 = encode_wire_hour(
            &schema,
            0,
            DEFAULT_BLOCK_HOURS,
            scanned.hours[0].follows_previous,
            scanned.hours[0].game_hour,
            None,
            &states[0],
            &mut reader,
            &mut wire,
        );
        assert!(o0.opened_baseline && !o0.gap_forced);
        let o1 = encode_wire_hour(
            &schema,
            1,
            DEFAULT_BLOCK_HOURS,
            scanned.hours[1].follows_previous,
            scanned.hours[1].game_hour,
            Some(&states[0]),
            &states[1],
            &mut reader,
            &mut wire,
        );
        assert!(
            o1.opened_baseline && o1.gap_forced,
            "the post-gap hour must be a gap-forced baseline, not a delta"
        );

        // Shape 2: hour 101 is never scheduled at all — the capture closes
        // hours 100 and 102 only.
        let mut s = Vec::new();
        s.extend(record(1, 100, D52, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d52_payload(0, 7, 10)));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            102,
            D52,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d52_payload(COMPACT_STATUS_DELTA_DATA, 7, 12),
        ));
        s.extend(frame_end(4, 102));
        let scanned = scan(&s).expect("well-formed");
        assert_eq!(scanned.hours.len(), 2);
        assert_eq!(scanned.non_contiguous, 1);
        assert!(!scanned.hours[1].follows_previous);
        assert!(opens_baseline(
            1,
            DEFAULT_BLOCK_HOURS,
            scanned.hours[1].follows_previous
        ));

        // The decision function over its whole input space: a block boundary
        // always opens one, and so does any gap, at every position in a block.
        for hours_seen in 0..(2 * DEFAULT_BLOCK_HOURS) {
            assert!(
                opens_baseline(hours_seen, DEFAULT_BLOCK_HOURS, false),
                "a gap at position {hours_seen} must open a baseline"
            );
            assert_eq!(
                opens_baseline(hours_seen, DEFAULT_BLOCK_HOURS, true),
                hours_seen % DEFAULT_BLOCK_HOURS == 0,
                "contiguous position {hours_seen}"
            );
        }

        // A contiguous run is unaffected: only the block boundary baselines.
        let scanned = scan(&good_stream()).expect("well-formed");
        assert_eq!(scanned.non_contiguous, 0);
        assert!(scanned.hours[1].follows_previous);
        assert!(!opens_baseline(
            1,
            DEFAULT_BLOCK_HOURS,
            scanned.hours[1].follows_previous
        ));
    }

    #[test]
    fn a_closed_hour_without_a_d52_record_refuses_the_stream() {
        // The cross-domain bound: this harness validates only d52 payloads, so
        // a d52 record relabelled to another domain — or dropped — would
        // otherwise be silently ignored and the run would report a plausible
        // measurement over fewer hours. Hour 100 carries a d52 record; hour
        // 101 carries only another domain's DATA record. Every hour is
        // structurally well formed — a DOMAIN_DATA record opens it and a
        // FRAME_END closes it — so only the completeness check can see it.
        let mut s = Vec::new();
        s.extend(record(1, 100, D52, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d52_payload(0, 7, 10)));
        s.extend(frame_end(2, 100));
        s.extend(record(3, 101, 20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &[0u8; 8]));
        s.extend(frame_end(4, 101));
        let err = match scan(&s) {
            Err(err) => err,
            Ok(_) => panic!("a closed hour with no d52 record must refuse the stream"),
        };
        assert!(
            err.contains("every closed hour"),
            "the refusal names the completeness rule: {err}"
        );
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time: the
        // fold retains the accumulator and folds nothing, so the hour is not d52
        // data and must not enter either arm.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D52,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d52_payload(1 << bit, 7, 10),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(3, 101, D52, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d52_payload(0, 7, 11)));
            s.extend(frame_end(4, 101));
            let scanned = scan(&s).expect("the stream itself is well-formed");
            assert_eq!(scanned.failure_records, 1, "bit {bit}");
            assert_eq!(scanned.hours.len(), 1, "bit {bit}");
            assert_eq!(scanned.hours[0].game_hour, 101, "bit {bit}");
        }
    }

    #[test]
    fn a_delta_before_any_baseline_refuses_the_run_rather_than_skipping_an_hour() {
        // The fold's second non-folding path (`world_fold.rs:16836`-`:16845`) is
        // NOT the failure path: it leaves this hour's trailers live and the
        // recorder publishes them. Skipping the hour would drop recorder-folded
        // data and still charge the shipping arm for its bytes, so the run is
        // invalid. Asserted with a trailer present and with one absent, and with
        // a healthy baseline behind it, so the refusal does not depend on any of
        // that.
        for later_hours in [false, true] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D52,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d52_payload(COMPACT_STATUS_DELTA_DATA, 7, 10),
            ));
            s.extend(frame_end(2, 100));
            if later_hours {
                s.extend(record(
                    3,
                    101,
                    D52,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d52_payload(0, 7, 11),
                ));
                s.extend(frame_end(4, 101));
            }
            let error = scan(&s).expect_err("a delta before any baseline refuses the run");
            assert!(
                error.contains("delta before any baseline"),
                "unexpected refusal reason: {error}"
            );
        }
        // And a delta AFTER a baseline is ordinary measured data, so the refusal
        // is about the ordering and nothing else.
        let mut s = Vec::new();
        s.extend(record(1, 100, D52, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d52_payload(0, 7, 10)));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D52,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d52_payload(COMPACT_STATUS_DELTA_DATA, 7, 11),
        ));
        s.extend(frame_end(4, 101));
        let scanned = scan(&s).expect("a delta behind a baseline is measured");
        assert_eq!(scanned.hours.len(), 2);
        assert_eq!(scanned.delta_before_baseline_records, 0);
    }

    #[test]
    fn a_malformed_trailer_refuses_the_stream_even_on_an_excluded_hour() {
        // The fold decodes the trailers BEFORE it reads the status word, so this
        // is a stream rejection and not an hour to skip. Copying a status-first
        // harness would accept it.
        for status in [0u32, 1 /* STATE_UNAVAILABLE */, COMPACT_STATUS_DELTA_DATA] {
            let mut payload = d52_payload(status, 7, 10);
            // Zero the GAR1 flag word: neither unavailable nor complete.
            let at = payload.len() - d52::GARRISON_ALERT_HEADER_LEN + 16;
            payload[at..at + 4].copy_from_slice(&0u32.to_le_bytes());
            let mut s = Vec::new();
            s.extend(record(1, 100, D52, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
            s.extend(frame_end(2, 100));
            assert!(scan(&s).is_err(), "status {status:#x}");
        }
    }

    #[test]
    fn a_second_d52_record_in_one_hour_is_rejected() {
        const FAILED: u32 = 1;
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(1, 100, D52, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d52_payload(first, 7, 10)));
            s.extend(record(2, 100, D52, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d52_payload(second, 7, 11)));
            s.extend(frame_end(3, 100));
            assert!(
                scan(&s).is_err(),
                "two d52 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_payload_the_lane_contract_refuses_refuses_the_stream() {
        // Production's second authority runs on EVERY DOMAIN_DATA record, so a
        // record it complains about is a refused stream and not a skipped hour —
        // including on a failure-status record the fold would never reach.
        for status in [0u32, 1] {
            let mut payload = d52_payload(status, 7, 10);
            // A header counter that disagrees with the rows.
            payload[24..28].copy_from_slice(&1u32.to_le_bytes());
            let mut s = Vec::new();
            s.extend(record(1, 100, D52, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload));
            s.extend(frame_end(2, 100));
            assert!(scan(&s).is_err(), "status {status:#x}");
        }
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected_unless_it_closed_its_hour() {
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(scan(&full[..cut]).is_err(), "cut after {records} record(s)");
        }
        for (records, expected_hours) in [(2usize, 1usize), (4, 2)] {
            let cut = boundary(&full, records);
            let scanned = scan(&full[..cut]).expect("closed stream is well-formed");
            assert_eq!(scanned.hours.len(), expected_hours, "cut after {records}");
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
    fn a_header_or_sequence_the_canonical_parser_rejects_is_rejected_here() {
        for records in 0..4 {
            let at = boundary(&good_stream(), records);
            for (offset, patch) in [
                (0usize, vec![b'X']),
                (4, 2u16.to_le_bytes().to_vec()),
                (6, 33u16.to_le_bytes().to_vec()),
                (8, 9u64.to_le_bytes().to_vec()),
            ] {
                let mut s = good_stream();
                s[at + offset..at + offset + patch.len()].copy_from_slice(&patch);
                assert!(scan(&s).is_err(), "record {records} patched at +{offset}");
            }
        }
    }

    #[test]
    fn a_frame_end_that_does_not_close_its_open_hour_is_rejected() {
        let mut s = Vec::new();
        s.extend(record(1, 100, D52, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d52_payload(0, 7, 10)));
        s.extend(frame_end(2, 999));
        assert!(scan(&s).is_err());
        let mut s = Vec::new();
        s.extend(frame_end(1, 100));
        assert!(scan(&s).is_err());
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        // The seal predicate, swept: a nonzero reserved word, mismatched domain
        // counts, an unknown status bit and a fatal status bit.
        let seal = |emitted: u32, scheduled: u32, status: u32, reserved: u32| {
            let mut payload = Vec::with_capacity(COMPACT_FRAME_END_PAYLOAD_LEN);
            payload.extend_from_slice(&0u64.to_le_bytes());
            payload.extend_from_slice(&0u64.to_le_bytes());
            payload.extend_from_slice(&emitted.to_le_bytes());
            payload.extend_from_slice(&scheduled.to_le_bytes());
            payload.extend_from_slice(&status.to_le_bytes());
            payload.extend_from_slice(&reserved.to_le_bytes());
            record(2, 100, COMPACT_DOMAIN_FRAME, COMPACT_RECORD_TYPE_FRAME_END, &payload)
        };
        let data = record(1, 100, D52, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d52_payload(0, 7, 10));
        for (label, tail) in [
            ("nonzero reserved", seal(0, 0, 0, 1)),
            ("mismatched counts", seal(1, 0, 0, 0)),
            // Bit 16: bits 14 and 15 are both ASSIGNED now (measured-empty and
            // BASELINE_CLEAR), so a seal carrying either names a known status
            // and is not an unknown-bit refusal.
            ("unknown status bit", seal(0, 0, 1 << 16, 0)),
            ("fatal status bit", seal(0, 0, 1, 0)),
            ("counts without PERF records", seal(2, 2, 0, 0)),
        ] {
            let mut s = data.clone();
            s.extend(tail);
            assert!(scan(&s).is_err(), "FRAME_END with {label}");
        }
        // The well-formed seal, for contrast.
        let mut s = data.clone();
        s.extend(seal(0, 0, 0, 0));
        assert!(scan(&s).is_ok());
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        let perf = |sequence: u64, hour: i32, domain: u16| {
            record(
                sequence,
                hour,
                domain,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                &vec![0u8; COMPACT_DOMAIN_PERF_BASE_LEN],
            )
        };
        // A duplicate non-probe PERF domain in one hour.
        let mut s = Vec::new();
        s.extend(perf(1, 100, D52));
        s.extend(perf(2, 100, D52));
        assert!(scan(&s).is_err());
        // A PERF for an hour other than the open one.
        let mut s = Vec::new();
        s.extend(record(1, 100, D52, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d52_payload(0, 7, 10)));
        s.extend(perf(2, 101, D52));
        assert!(scan(&s).is_err());
        // A well-formed hour whose seal counts its one PERF domain.
        let mut payload = Vec::with_capacity(COMPACT_FRAME_END_PAYLOAD_LEN);
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        let mut s = Vec::new();
        s.extend(perf(1, 100, D52));
        s.extend(record(2, 100, D52, COMPACT_RECORD_TYPE_DOMAIN_DATA, &d52_payload(0, 7, 10)));
        s.extend(record(
            3,
            100,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &payload,
        ));
        assert!(scan(&s).is_ok());
    }

    #[test]
    fn a_stream_with_no_d52_records_is_refused_not_reported_as_zero() {
        let mut s = Vec::new();
        s.extend(record(1, 100, 20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &[0u8; 8]));
        s.extend(frame_end(2, 100));
        assert!(scan(&s).is_err());
    }

    #[test]
    fn a_row_the_fold_rejects_makes_the_scan_fail() {
        // Not "is accepted and then unwinds while measuring".
        let mut zero_state = d52_payload(0, 0, 10);
        assert_eq!(u32::from_le_bytes(zero_state[d52::HEADER_LEN..d52::HEADER_LEN + 4].try_into().unwrap()), 0);
        let mut s = Vec::new();
        s.extend(record(1, 100, D52, COMPACT_RECORD_TYPE_DOMAIN_DATA, &zero_state));
        s.extend(frame_end(2, 100));
        assert!(scan(&s).is_err());
        // An unknown row flag bit.
        zero_state = d52_payload(0, 7, 10);
        let at = d52::HEADER_LEN + 52;
        zero_state[at..at + 4].copy_from_slice(&(1u32 << 20).to_le_bytes());
        let mut s = Vec::new();
        s.extend(record(1, 100, D52, COMPACT_RECORD_TYPE_DOMAIN_DATA, &zero_state));
        s.extend(frame_end(2, 100));
        assert!(scan(&s).is_err());
    }
}
