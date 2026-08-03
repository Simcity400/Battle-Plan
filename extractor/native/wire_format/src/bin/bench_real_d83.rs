//! Measure the wire format against the shipping compact stream on REAL d83
//! `nuke_stockpile_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d83-scheduled capture
//! (`runs/batch1/deepwar_t1`). The stream's own d83 DATA records are the
//! shipping format's cost for the domain: a 164-byte NUK1 header, a 12-byte
//! snapshot prologue, then a fixed 60-byte block per country (a 12-byte country
//! row and its two 24-byte typed entries), full-replace every hour, exactly what
//! the recorder stores today. This tool replays those records into per-hour
//! states — the same fold `world_fold::fold_nuke_stockpile` performs — and
//! re-encodes every hour with this crate's grammar: a BASELINE at each 48-hour
//! block start, field-granular stored-minimal DELTAs between.
//!
//! **Two passes, because the state is large.** 439 countries and 878 typed rows
//! per hour over 903 hours is ~1.2 M records; holding every hour's state at once
//! is hundreds of megabytes for no reason. Pass 1 validates the whole stream and
//! proves every d83 record folds, retaining only record spans; pass 2 re-folds
//! through the identical code path while encoding. The fold itself lives in
//! `wire_format::d83::fold_payload`, shared with the tests, so the two passes and
//! the unit tests cannot drift apart.
//!
//! **Nothing here re-derives a contract it can call.** Payload, block, row and
//! entry decoding go through `wire_format::d83`'s public boundary; H4CR framing,
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
//!   (`recorder_host/src/stream_fold.rs:277`) that do not require DOMAIN_PERF
//!   accounting: a DOMAIN_DATA record opens its game hour, a FRAME_END closes
//!   the open hour and only that hour, and closed hours strictly increase;
//! - that tracker's DOMAIN_PERF hour binding (`:328`-`:365`), including the DATA
//!   arm's pending-PERF rule (`:378`-`:386`);
//! - FRAME_END seal validation (`stream_fold.rs:46`-`:89`) and the seal's count
//!   cross-check against distinct non-probe PERF domains (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END;
//! - every d83 payload through `d83::split_payload` and, when the record is
//!   usable, `d83::fold_payload`, so a block the recorder would reject stops the
//!   scan rather than unwinding later.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority. This crate cannot depend on `recorder_host` (it pulls in every
//! capture lane), so the relevant predicates are mirrored here.
//!
//! Check 4 adds no new d83 predicate. Production's
//! `decode_compact_nuke_stockpile_data_payload`
//! (`domain_data_authority.rs:13675`-`:13720`) delegates to the NUK1 decoder. For
//! the only records that can be excluded, its contract is already fully enforced
//! by `d83::split_payload` before status: a valid failure-status d83 payload is
//! necessarily the source-unavailable, rowless form. There is therefore no row
//! predicate an excluded record can bypass; success records reach
//! `d83::fold_payload`, which performs the block walk.
//! The regression test proves malformed excluded payloads still reach
//! `split_payload` for every failure bit. Identity mode is stated by the caller
//! (`stream_fold.rs:220`), never sniffed from the first record.
//!
//! The DATA payloads of every domain other than 83 pass UNVALIDATED beyond
//! header sanity, the stream validator and the hour-boundary rules —
//! production validates every domain's payload (`stream_fold.rs:593`); this
//! harness owns only d83's grammar. That unvalidated remainder is explicitly
//! BOUNDED: every closed game hour must carry exactly one d83 DATA record of
//! any status (the `d83_records == closed_hours` completeness check in
//! [`scan_stream`]), so a d83 record relabelled to another domain id — or
//! dropped — refuses the run instead of silently shrinking it.
//!
//! **Exclusions, in the fold's own order.** `fold_nuke_stockpile` decodes the
//! NUK1 payload FIRST (`world_fold.rs:11439`) and consults the status word only
//! afterwards (`:11441`), so a malformed payload is a refused stream whatever its
//! status is, and only a well-formed failure-status record is excluded. d83 is a
//! pure full-replace domain — there is no baseline/delta grammar in the payload
//! — so failure status is the *only* path on which the fold returns without
//! folding. The two remaining early returns (`:11449` missing receipt, `:11452`
//! missing snapshot) are `Err`, not silent skips, and are treated as such here.
//! The failure predicate is `compact_contract`'s own
//! (`compact_status_failure_flags_after_expected_partial_tolerance`), never a
//! hand-copied mask literal; d83 registers no expected-partial tolerance.
//! Hour uniqueness is enforced over EVERY d83 DATA record and BEFORE the status
//! exclusion, so a success + failure pair in one hour cannot slip through.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d83.bin   — the d83 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface (changed rows per hour at stored
//! precision) and two frame-level corroborations described at [`FrameNote`].

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
use wire_format::d83;
use wire_format::Schema;

const D83: u16 = 83;
const BLOCK_HOURS: usize = 48;

/// One usable d83 hour: its game hour and the two byte spans pass 2 needs. The
/// folded state is deliberately NOT retained — see the module docs.
struct D83Hour {
    game_hour: i32,
    /// The whole record, header included: the shipping arm is these bytes.
    ship_span: Range<usize>,
    /// The payload alone, for the re-fold.
    payload_span: Range<usize>,
    /// True when this measured hour is exactly one game hour after the previous
    /// MEASURED hour. False on the first measured hour and after any gap — an
    /// excluded failure-status hour or a non-contiguous hour step — which is
    /// what forces the encoder to open a fresh baseline (`opens_baseline`).
    follows_previous: bool,
}

/// Whether the wire arm opens a BASELINE at this measured hour.
///
/// Two reasons, and both are `WIRE_FORMAT.md` rules, not conveniences:
/// the 48-hour block cadence (every block opens with its own baseline, §1.5's
/// closing paragraph), and a gap — `WIRE_FORMAT.md:187`-`:190`: "A gap
/// affecting the state part forces the next state observation to be a
/// `BASELINE`." Every hour this domain did not successfully sample is such a
/// gap — the excluded failure-status hour, and any non-contiguous hour step —
/// and d83's cadence is hourly, so a measured hour that does not follow the
/// previous measured one by exactly one is preceded by one. Neither a delta
/// nor a retained reader state can be justified across it.
fn opens_baseline(hours_seen: usize, follows_previous: bool) -> bool {
    hours_seen % BLOCK_HOURS == 0 || !follows_previous
}

/// One wire-arm hour: the §1.5/block baseline decision, the encode, and the
/// per-hour reader replay over both collections. Factored out of `main` so the
/// tests execute the PRODUCTION encode path over a scanned stream: a mutant
/// that reverts the baseline decision here to block-position-only fails the
/// gap test (a Codex round-1 finding — the earlier tests exercised the
/// decision function but not this callsite).
struct WireHourOutcome {
    opened_baseline: bool,
    /// The baseline sits outside a scheduled block start — §1.5 gap-forced.
    gap_forced: bool,
    emitted: bool,
    bytes: usize,
    changed_rows: usize,
    tombstones: usize,
}

#[allow(clippy::too_many_arguments)]
fn encode_wire_hour(
    schema: &Schema,
    hours_seen: usize,
    follows_previous: bool,
    game_hour: i32,
    prior: &d83::NukeStockpileState,
    state: &d83::NukeStockpileState,
    reader: &mut Vec<KeyedState>,
    wire: &mut Vec<u8>,
) -> WireHourOutcome {
    let hour_start = wire.len();
    let block_start = opens_baseline(hours_seen, follows_previous);
    let gap_forced = block_start && hours_seen % BLOCK_HOURS != 0;
    let mut emitted = true;
    let mut changed_rows = 0usize;
    let mut tombstones = 0usize;
    if block_start {
        encode_baseline(schema, &state.as_slice(), wire).expect("baseline encodes");
    } else {
        let deltas: Vec<Option<KeyedDelta>> = (0..d83::COLLECTION_COUNT)
            .map(|i| {
                let d = diff(&schema.collections[i], prior.get(i), state.get(i));
                (!d.is_empty()).then_some(d)
            })
            .collect();
        if deltas.iter().all(Option::is_none) {
            emitted = false;
        } else {
            for (i, delta) in deltas.iter().enumerate() {
                let Some(delta) = delta else { continue };
                changed_rows += delta.changed.len();
                tombstones += delta.tombstones.len();
                // §7.1 minimality, checked rather than assumed: `diff` is
                // supposed to produce the one minimal delta, and this is the
                // rule that says so.
                check_minimal(&schema.collections[i], prior.get(i), state.get(i), delta)
                    .unwrap_or_else(|e| panic!("hour {game_hour}: {e:?}"));
            }
            encode_delta(schema, &prior.as_slice(), &deltas, wire).expect("delta encodes");
        }
    }
    let bytes = wire.len() - hour_start;

    // Reader replay: decode the bytes just written against the reader's OWN
    // reconstructed prior, exactly as a conforming reader would, and require
    // the result to be stored-equal to this hour's state. This is what makes
    // the size claim a claim about decodable bytes. An hour that emitted
    // nothing is still checked: the reader's retained state must already
    // equal the fold's, which is precisely what "nothing changed" asserts.
    if emitted {
        let hour_bytes = &wire[hour_start..];
        if block_start {
            let (states, consumed) =
                decode_baseline(schema, hour_bytes, 0).expect("baseline decodes");
            assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
            *reader = states;
        } else {
            let (deltas, consumed) =
                decode_delta(schema, reader, hour_bytes, 0).expect("delta decodes");
            assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
            for (i, delta) in deltas.iter().enumerate() {
                if let Some(delta) = delta {
                    apply(&mut reader[i], delta);
                }
            }
        }
    }
    for i in 0..d83::COLLECTION_COUNT {
        assert_eq!(
            reader[i].len(),
            state.get(i).len(),
            "game hour {game_hour}: collection {} row count",
            schema.collections[i].collection_id
        );
        assert!(
            diff(&schema.collections[i], &reader[i], state.get(i)).is_empty(),
            "game hour {game_hour}: collection {} is not stored-equal to the fold",
            schema.collections[i].collection_id
        );
    }
    WireHourOutcome {
        opened_baseline: block_start,
        gap_forced,
        emitted,
        bytes,
        changed_rows,
        tombstones,
    }
}

/// Two frame-level facts this harness reports rather than enforces.
///
/// `frame_hour` is the NUK1 header's own hour and `source_sequence` is the
/// collector's snapshot ordinal, set to `frame_hour + 1`
/// (`collector/src/attach_log.rs:31908`-`:31911`). Neither is a row field (see
/// `d83`'s module docs), and neither is checked by `fold_nuke_stockpile`, so
/// asserting them would reject streams the recorder accepts. Counting them says
/// how far the "frame-level and derivable" claim holds on real data.
struct FrameNote {
    /// Records whose `frame_hour` equals `hour_index_from_raw(game_hour_raw)`.
    frame_hour_matches_record: usize,
    /// Records whose `source_sequence` equals `frame_hour + 1`.
    sequence_is_frame_hour_plus_one: usize,
}

struct StreamScan {
    hours: Vec<D83Hour>,
    /// d83 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// Measured d83 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// Peak per-hour row counts, for the report.
    countries_max: usize,
    entries_max: usize,
    notes: FrameNote,
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

/// Validate the whole stream against the canonical contract and prove every
/// usable d83 DATA record folds. Every rejection path is an `Err`, never a
/// silently shorter measurement and never a panic — see the module docs for what
/// is enforced and what the recorder's own validator additionally checks.
fn scan_stream(
    stream: &[u8],
    schema: &Schema,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    // Production receives the identity mode from its caller
    // (`stream_fold.rs:220`); the first record cannot state it safely.
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours: Vec<D83Hour> = Vec::new();
    let mut failure_records = 0usize;
    let mut non_contiguous = 0usize;
    // COMPLETENESS accounting: every d83 DATA record of ANY status, against
    // every closed game hour — see the check after the EOF rules.
    let mut d83_records = 0usize;
    let mut closed_hours = 0usize;
    let mut countries_max = 0usize;
    let mut entries_max = 0usize;
    let mut notes = FrameNote {
        frame_hour_matches_record: 0,
        sequence_is_frame_hour_plus_one: 0,
    };

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d83 DATA record of ANY status — the uniqueness rule.
    let mut last_d83_hour: Option<i32> = None;
    // Hour of the last d83 record that entered the measurement — the
    // contiguity report, which a skipped failure hour legitimately breaks.
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
                pending_perf_hour = None;
                non_probe_perf_domains.clear();
                closed_hours += 1;
            }
            _ => {}
        }
        last_record_type = Some(header.record_type);

        if header.domain_id == D83 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d83_records += 1;
            // The fold decodes BEFORE it reads the status word
            // (`world_fold.rs:11439`-`:11441`), so this runs first and a
            // malformed payload is refused whatever its status is.
            let (nuk_header, body) = d83::split_payload(payload)
                .map_err(|e| format!("d83 payload at byte {pos}: {e}"))?;

            // One d83 DATA record per game hour, checked over EVERY d83 record
            // and BEFORE any status-based exclusion. d83 is full-replace, so a
            // second record for the same hour would become a second measurement
            // hour built from the same clock time — and gating this on the
            // record being usable would let a success/failure pair through the
            // rule entirely.
            if let Some(previous) = last_d83_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d83 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d83_hour = Some(header.game_hour_raw);

            // VALIDITY BEFORE CLASSIFICATION. The failure predicate below reads
            // the status word to classify the v1 vocabulary; it is not a validity
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
            let unknown_status = compact_status_unknown_flags_v1(nuk_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d83 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            // Reported, never enforced — see `FrameNote`.
            if compact_contract::calendar::hour_index_from_raw(header.game_hour_raw)
                .and_then(|hour| u64::try_from(hour).ok())
                == Some(nuk_header.frame_hour)
            {
                notes.frame_hour_matches_record += 1;
            }
            if nuk_header.has_snapshot
                && nuk_header.source_sequence == nuk_header.frame_hour.saturating_add(1)
            {
                notes.sequence_is_frame_hour_plus_one += 1;
            }

            // `world_fold::handle_compact_failure` (called at `:11441`) clears
            // the domain-hour and folds nothing on a failure status. The
            // predicate is `compact_contract`'s own, not a copied bit mask: it
            // already applies d83's lane-local mask (none) and expected partial
            // tolerance (none), so it cannot drift from the recorder.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D83,
                nuk_header.status_flags,
            ) != 0
            {
                failure_records += 1;
            } else {
                let follows_previous =
                    last_measured_hour.is_some_and(|previous| header.game_hour_raw == previous + 1);
                if last_measured_hour.is_some() && !follows_previous {
                    non_contiguous += 1;
                }
                last_measured_hour = Some(header.game_hour_raw);

                // Prove the record folds here, in the pass that either accepts
                // the stream or refuses it. The state is dropped immediately;
                // pass 2 rebuilds it through this same call.
                let state = d83::fold_payload(schema, &nuk_header, body, 0)
                    .map_err(|e| format!("d83 payload at byte {pos}: {e}"))?;
                countries_max = countries_max.max(state.countries.len());
                entries_max = entries_max.max(state.entries.len());
                hours.push(D83Hour {
                    game_hour: header.game_hour_raw,
                    ship_span: pos..record_end,
                    payload_span: payload_start..record_end,
                    follows_previous,
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
        return Err("stream carries no usable d83 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d83 DATA
    // record, of any status. This bounds what this harness canNOT validate:
    // production runs `validate_domain_data_payload_at` on EVERY domain's DATA
    // payload (`stream_fold.rs:593`), while this harness owns only d83's
    // grammar, so a d83 record whose header was relabelled to another domain
    // id — or lost outright — would otherwise be silently ignored and the run
    // would report a plausible measurement over fewer hours. A per-capture
    // completeness assertion, NOT a grammar rule: a capture where d83 is not
    // scheduled every hour fails here loudly and a human adjusts, instead of a
    // number quietly shrinking (`bench_real_d65.rs` precedent).
    if d83_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d83_records} d83 DATA \
             record(s); every closed hour of a d83-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    Ok(StreamScan {
        hours,
        failure_records,
        non_contiguous,
        countries_max,
        entries_max,
        notes,
    })
}

/// Re-fold one payload pass 1 already accepted.
fn fold_span(schema: &Schema, payload: &[u8]) -> d83::NukeStockpileState {
    let (header, body) = d83::split_payload(payload).expect("pass 1 accepted this payload");
    d83::fold_payload(schema, &header, body, 0).expect("pass 1 accepted this payload")
}

fn main() {
    const USAGE: &str = "usage: bench_real_d83 <collector_stream.bin> <out dir> \
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
    let schema = d83::schema();

    // ---- Pass 1: validate the WHOLE stream and prove every d83 record folds.
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

    let mut prior = d83::NukeStockpileState::new();
    let mut reader: Vec<KeyedState> = vec![KeyedState::new(); d83::COLLECTION_COUNT];
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut gap_forced_baselines = 0usize;
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    let mut empty_hours = 0usize;
    let mut changed_rows_total = 0usize;
    let mut tombstones_total = 0usize;
    let mut rows_total = 0usize;

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());

        // d83 is full-replace, so this record's blocks ARE the hour's whole
        // state — the same call pass 1 made, on the same bytes.
        let state = fold_span(&schema, &stream[hour.payload_span.clone()]);
        rows_total += state.rows();

        // New arm: BASELINE at each block start — and at every §1.5 gap, which
        // forces a fresh baseline and restarts the delta chain (`opens_baseline`).
        // The wire-arm hour — baseline decision, encode and reader replay —
        // through the SAME function the tests execute (`encode_wire_hour`).
        let outcome = encode_wire_hour(
            &schema,
            hours_seen,
            hour.follows_previous,
            hour.game_hour,
            &prior,
            &state,
            &mut reader,
            &mut wire,
        );
        if outcome.gap_forced {
            gap_forced_baselines += 1;
        }
        if outcome.opened_baseline {
            baselines += 1;
            baseline_bytes += outcome.bytes;
        } else if outcome.emitted {
            changed_rows_total += outcome.changed_rows;
            tombstones_total += outcome.tombstones;
            delta_bytes += outcome.bytes;
        } else {
            empty_hours += 1;
        }
        wire_offsets.push(wire.len());

        prior = state;
        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d83.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d83.offsets", &ship_offsets),
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
        "rows avg             {:.1} over both collections (max {} countries, {} entries)",
        rows_total as f64 / hours_seen as f64,
        scan.countries_max,
        scan.entries_max
    );
    println!(
        "frame-level notes    frame_hour == hour_index(game_hour) on {}/{} records; \
         source_sequence == frame_hour + 1 on {}/{}",
        scan.notes.frame_hour_matches_record,
        hours_seen + scan.failure_records,
        scan.notes.sequence_is_frame_hour_plus_one,
        hours_seen + scan.failure_records
    );
    println!(
        "baselines            {baselines} ({} at a {BLOCK_HOURS}-h block start, \
         {gap_forced_baselines} forced by a WIRE_FORMAT \u{a7}1.5 state gap)",
        baselines - gap_forced_baselines
    );
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "real churn           {:.2} changed rows/hour, {:.3} tombstones/hour (stored precision)",
        changed_rows_total as f64 / delta_hours.max(1) as f64,
        tombstones_total as f64 / delta_hours.max(1) as f64
    );
    println!("ship_d83.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.2}%)  baseline {} B, delta {} B",
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
        d83::schema()
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

    fn country_block(tag_db_id: u32, capacity: i32, amounts: [i64; 2], ready: [u32; 2]) -> Vec<u8> {
        let mut block = Vec::new();
        block.extend_from_slice(&tag_db_id.to_le_bytes());
        block.extend_from_slice(&capacity.to_le_bytes());
        block.extend_from_slice(&2u32.to_le_bytes());
        for index in 0..2usize {
            block.extend_from_slice(&(index as u32).to_le_bytes());
            block.extend_from_slice(&0u32.to_le_bytes());
            block.extend_from_slice(&amounts[index].to_le_bytes());
            block.extend_from_slice(&ready[index].to_le_bytes());
            block.extend_from_slice(&0u32.to_le_bytes());
        }
        assert_eq!(block.len(), d83::COUNTRY_BLOCK_LEN);
        block
    }

    /// A receipted one-country d83 DATA payload.
    fn d83_payload(status_flags: u32, tag_db_id: u32, amount: i64) -> Vec<u8> {
        // A non-zero status word forces the unavailable, rowless form: the codec
        // couples authority, status, receipt and snapshot presence
        // (`lanes/nuke_stockpile_live/src/lib.rs:270`-`:291`), so a "failing"
        // receipted frame is not a payload the recorder can produce.
        let receipted = status_flags == 0;
        let mut p = Vec::new();
        p.extend_from_slice(&d83::MAGIC.to_le_bytes());
        p.extend_from_slice(&d83::SCHEMA_VERSION.to_le_bytes());
        p.extend_from_slice(
            &if receipted {
                d83::AUTHORITY_EXACT_A729_RECEIPT
            } else {
                d83::AUTHORITY_SOURCE_UNAVAILABLE
            }
            .to_le_bytes(),
        );
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&900u64.to_le_bytes()); // frame_hour
        p.extend_from_slice(&d83::EXECUTABLE_SHA256);
        p.extend_from_slice(&if receipted {
            d83::RECEIPT_SHA256
        } else {
            [0u8; 32]
        });
        p.extend_from_slice(&d83::CONTAINER_CONSTRUCTOR_SHA256);
        p.extend_from_slice(&d83::VTABLE_ARTIFACT_SHA256);
        p.push(u8::from(receipted));
        p.extend_from_slice(&[0; 3]);
        assert_eq!(p.len(), d83::HEADER_LEN);
        if receipted {
            p.extend_from_slice(&901u64.to_le_bytes()); // source_sequence
            p.extend_from_slice(&1u32.to_le_bytes()); // country_count
            p.extend_from_slice(&country_block(tag_db_id, 2, [amount, 0], [0, 0]));
        }
        p
    }

    /// Two closed hours, each carrying one d83 DATA record.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D83,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d83_payload(0, 7, 0),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D83,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d83_payload(0, 7, 100_000_000),
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

    fn domain_data(sequence: u64, game_hour: i32) -> Vec<u8> {
        record(
            sequence,
            game_hour,
            D83,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d83_payload(0, 7, 0),
        )
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        let mut ok = domain_data(1, 100);
        ok.extend(frame_end(2, 100));
        assert!(scan_stream(&ok, &test_schema(), StreamIdentityMode::Legacy).is_ok());
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
            let mut s = domain_data(1, 100);
            s.extend(seal);
            assert!(scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err());
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
            assert!(scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err());
        }
        for status in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = domain_data(1, 100);
            s.extend(frame_end_sealed(2, 100, 0, 0, status, 0));
            assert!(scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_ok());
        }
        let mut later = domain_data(1, 100);
        later.extend(frame_end(2, 100));
        later.extend(domain_data(3, 101));
        later.extend(frame_end_sealed(4, 101, 0, 0, 0, 1));
        assert!(scan_stream(&later, &test_schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        let mut ok = perf(1, 100, 20);
        ok.extend(perf(2, 100, D83));
        ok.extend(perf(3, 100, PROBE_DOMAIN_ID));
        ok.extend(domain_data(4, 100));
        ok.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
        assert!(scan_stream(&ok, &test_schema(), StreamIdentityMode::Legacy).is_ok());
        let mut probes = perf(1, 100, PROBE_DOMAIN_ID);
        probes.extend(perf(2, 100, PROBE_DOMAIN_ID));
        probes.extend(domain_data(3, 100));
        probes.extend(frame_end(4, 100));
        assert!(scan_stream(&probes, &test_schema(), StreamIdentityMode::Legacy).is_ok());
        let mut duplicate = perf(1, 100, 20);
        duplicate.extend(perf(2, 100, 20));
        duplicate.extend(domain_data(3, 100));
        duplicate.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        assert!(scan_stream(&duplicate, &test_schema(), StreamIdentityMode::Legacy).is_err());
        let mut interleaved = domain_data(1, 100);
        interleaved.extend(perf(2, 999, 20));
        interleaved.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&interleaved, &test_schema(), StreamIdentityMode::Legacy).is_err());
        let mut stale = domain_data(1, 100);
        stale.extend(frame_end(2, 100));
        stale.extend(perf(3, 100, 20));
        stale.extend(domain_data(4, 101));
        stale.extend(frame_end_sealed(5, 101, 1, 1, 0, 0));
        assert!(scan_stream(&stale, &test_schema(), StreamIdentityMode::Legacy).is_err());
        let mut conflict = perf(1, 100, 20);
        conflict.extend(perf(2, 101, 21));
        conflict.extend(domain_data(3, 100));
        conflict.extend(frame_end_sealed(4, 100, 2, 2, 0, 0));
        assert!(scan_stream(&conflict, &test_schema(), StreamIdentityMode::Legacy).is_err());
        for (emitted, scheduled) in [(1, 1), (3, 3), (2, 3), (3, 2)] {
            let mut s = perf(1, 100, 20);
            s.extend(perf(2, 100, D83));
            s.extend(domain_data(3, 100));
            s.extend(frame_end_sealed(4, 100, emitted, scheduled, 0, 0));
            assert!(scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err());
        }
        let mut two = perf(1, 100, 20);
        two.extend(domain_data(2, 100));
        two.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        two.extend(perf(4, 101, 20));
        two.extend(domain_data(5, 101));
        two.extend(frame_end_sealed(6, 101, 1, 1, 0, 0));
        assert_eq!(
            scan_stream(&two, &test_schema(), StreamIdentityMode::Legacy)
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
            assert!(scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err());
        }
        let mut agreeing = perf(1, 100, 20);
        agreeing.extend(domain_data(2, 100));
        agreeing.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&agreeing, &test_schema(), StreamIdentityMode::Legacy).is_ok());
    }

    #[test]
    fn an_excluded_record_is_still_checked_by_the_payload_authority() {
        // d83 has no missing status-independent predicate: the authority's
        // `decode_payload` contract is already `d83::split_payload`, which runs
        // before exclusion. Moreover every valid failure-status payload is the
        // source-unavailable, rowless form, so no excluded record can reach a
        // country block. Prove that structural validation still runs for every
        // failure bit.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut malformed = d83_payload(1 << bit, 7, 0);
            malformed[161] = 1; // unavailable-frame reserved byte
            let mut s = record(1, 100, D83, COMPACT_RECORD_TYPE_DOMAIN_DATA, &malformed);
            s.extend(frame_end(2, 100));
            s.extend(domain_data(3, 101));
            s.extend(frame_end(4, 101));
            assert!(scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err());
        }

        let mut excluded = d83_payload(1, 7, 0);
        excluded[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
        excluded[24..32].copy_from_slice(&u64::MAX.to_le_bytes());
        let mut s = record(1, 100, D83, COMPACT_RECORD_TYPE_DOMAIN_DATA, &excluded);
        s.extend(frame_end(2, 100));
        s.extend(domain_data(3, 101));
        s.extend(frame_end(4, 101));
        let scan =
            scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn the_identity_protocol_is_stated_not_sniffed() {
        use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;

        let decapitated = good_stream();
        assert!(scan_stream(&decapitated, &test_schema(), StreamIdentityMode::IdentityV1).is_err());
        assert!(scan_stream(&decapitated, &test_schema(), StreamIdentityMode::Legacy).is_ok());
        let mut identity = record(
            0,
            0,
            0,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            b"CID1-not-a-real-identity-payload",
        );
        identity.extend(good_stream());
        assert!(scan_stream(&identity, &test_schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_well_formed_stream_yields_its_d83_hours() {
        let scan = scan_stream(&good_stream(), &test_schema(), StreamIdentityMode::Legacy)
            .expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].game_hour, 100);
        assert_eq!(scan.hours[1].game_hour, 101);
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.non_contiguous, 0);
        assert_eq!(scan.countries_max, 1);
        assert_eq!(scan.entries_max, 2, "two typed rows per country");
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helper classifies
        // the v1 vocabulary and is not a validity predicate, so an unassigned bit
        // must be rejected BEFORE classification — otherwise it silently reads as
        // "not a failure" and the hour is measured as healthy.
        //
        // d83's payload contract couples status to authority, on the READ side
        // too: `split_payload` refuses a receipted frame whose status word is
        // non-zero (`d83.rs:427`), so every non-zero-status d83 record that
        // reaches the status decision is the unavailable, rowless A17 form. That
        // form is also refused further down, by `fold_payload`'s authority check
        // (`d83.rs:756`) — which is why "is_err()" alone would prove nothing here.
        // The assertion is therefore on the REASON: the stream must be refused as
        // an unassigned status bit, before the word is classified at all. Without
        // the gate the scan classifies first ("not a failure"), measures the hour,
        // and only then fails on the missing receipt.
        for bit in 15..32u32 {
            let status = 1u32 << bit;
            // Alone, combined with a known failure bit, and combined with a known
            // informational bit. The rejection must not depend on any of it.
            for extra in [0u32, 1, 1 << 10] {
                let mut s = Vec::new();
                s.extend(record(
                    1,
                    100,
                    D83,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d83_payload(status | extra, 7, 0),
                ));
                s.extend(frame_end(2, 100));
                let err = match scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy) {
                    Ok(_) => panic!("status bit {bit} (+{extra:#x}) must be refused"),
                    Err(e) => e,
                };
                assert!(
                    err.contains("unassigned v1 status bits"),
                    "status bit {bit} (+{extra:#x}) must be refused AS an unassigned status \
                     bit, not incidentally: {err}"
                );
            }
        }
        // And the check rejects unassigned bits and nothing else. d83's own
        // vocabulary is narrow: a clean status word (0, the receipted form — the
        // one every hour of the reference capture carries) is measured, and a
        // known informational bit riding on a failure status is still EXCLUDED by
        // the failure predicate rather than refused by this gate.
        let mut clean = Vec::new();
        clean.extend(record(
            1,
            100,
            D83,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d83_payload(0, 7, 0),
        ));
        clean.extend(frame_end(2, 100));
        let scan = scan_stream(&clean, &test_schema(), StreamIdentityMode::Legacy)
            .expect("a clean receipted frame is measured");
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.failure_records, 0);
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D83,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d83_payload(
                    informational | compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
                    7,
                    0,
                ),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D83,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d83_payload(0, 7, 0),
            ));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(
                scan.failure_records, 1,
                "bit {informational:#x} excluded, not refused"
            );
            assert_eq!(scan.hours.len(), 1, "bit {informational:#x}");
        }
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        // A cut immediately after a d83 DATA record leaves that hour open and
        // ends the stream on a record that is not a FRAME_END; that would produce
        // a plausible shorter number and must be refused.
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&full[..cut], &test_schema(), StreamIdentityMode::Legacy).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary is a different thing: that IS a
        // well-formed shorter stream, and `eof_check` accepts it too. What matters
        // is that it reports the hours it actually closed.
        for (records, expected_hours) in [(2usize, 1usize), (4, 2)] {
            let cut = boundary(&full, records);
            let scan = scan_stream(&full[..cut], &test_schema(), StreamIdentityMode::Legacy)
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
            D83,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d83_payload(0, 7, 0),
        ));
        s.extend(frame_end(2, 999));
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
            D83,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d83_payload(0, 7, 0),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D83,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d83_payload(0, 7, 1),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_second_d83_record_in_one_hour_is_rejected() {
        // d83 is full-replace; two records in one hour would become two
        // measurement hours built from the same clock time. The rule is checked
        // over EVERY d83 DATA record and before status-based exclusion, so all
        // four success/failure orderings are refused.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D83,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d83_payload(first, 7, 0),
            ));
            s.extend(record(
                2,
                100,
                D83,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d83_payload(second, 7, 1),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err(),
                "two d83 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn an_excluded_hour_forces_the_next_observation_to_be_a_baseline() {
        // `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the state part forces
        // the next state observation to be a `BASELINE`." Both gap shapes d83
        // can meet — an excluded failure-status hour, and a whole hour the
        // capture never scheduled — must clear `follows_previous`, and
        // `opens_baseline` must open a baseline on it wherever it lands in a
        // block. A delta would relate two states across unobserved time.
        //
        // Shape 1: hour 101 carries a failure-status d83 record (excluded).
        let mut s = Vec::new();
        s.extend(domain_data(1, 100));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D83,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d83_payload(1 << 0, 7, 0), // failure status -> excluded
        ));
        s.extend(frame_end(4, 101));
        s.extend(record(
            5,
            102,
            D83,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d83_payload(0, 7, 100),
        ));
        s.extend(frame_end(6, 102));
        let scan =
            scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.non_contiguous, 1);
        assert!(!scan.hours[0].follows_previous, "the first hour follows none");
        assert!(
            !scan.hours[1].follows_previous,
            "hour 102 does not follow hour 100"
        );
        // ...and that is what makes the encoder open a baseline for it, even
        // though it is only the second measured hour of the block — through the
        // PRODUCTION encode path, not just the decision function.
        assert!(opens_baseline(1, scan.hours[1].follows_previous));
        let schema = test_schema();
        let state0 = fold_span(&schema, &s[scan.hours[0].payload_span.clone()]);
        let state1 = fold_span(&schema, &s[scan.hours[1].payload_span.clone()]);
        let mut wire = Vec::new();
        let mut reader: Vec<KeyedState> = vec![KeyedState::new(); d83::COLLECTION_COUNT];
        let o0 = encode_wire_hour(
            &schema,
            0,
            scan.hours[0].follows_previous,
            scan.hours[0].game_hour,
            &d83::NukeStockpileState::default(),
            &state0,
            &mut reader,
            &mut wire,
        );
        assert!(o0.opened_baseline && !o0.gap_forced);
        let o1 = encode_wire_hour(
            &schema,
            1,
            scan.hours[1].follows_previous,
            scan.hours[1].game_hour,
            &state0,
            &state1,
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
        s.extend(domain_data(1, 100));
        s.extend(frame_end(2, 100));
        s.extend(domain_data(3, 102));
        s.extend(frame_end(4, 102));
        let scan =
            scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.non_contiguous, 1);
        assert!(!scan.hours[1].follows_previous);
        assert!(opens_baseline(1, scan.hours[1].follows_previous));

        // The decision function over its whole input space: a block boundary
        // always opens one, and so does any gap, at every position in a block.
        for hours_seen in 0..(2 * BLOCK_HOURS) {
            assert!(
                opens_baseline(hours_seen, false),
                "a gap at position {hours_seen} must open a baseline"
            );
            assert_eq!(
                opens_baseline(hours_seen, true),
                hours_seen % BLOCK_HOURS == 0,
                "contiguous position {hours_seen}"
            );
        }

        // A contiguous run is unaffected: only the block boundary baselines.
        let scan = scan_stream(&good_stream(), &test_schema(), StreamIdentityMode::Legacy)
            .expect("well-formed");
        assert_eq!(scan.non_contiguous, 0);
        assert!(scan.hours[1].follows_previous);
        assert!(!opens_baseline(1, scan.hours[1].follows_previous));
    }

    #[test]
    fn a_closed_hour_without_a_d83_record_refuses_the_stream() {
        // The cross-domain bound: this harness validates only d83 payloads, so
        // a d83 record relabelled to another domain — or dropped — would
        // otherwise be silently ignored and the run would report a plausible
        // measurement over fewer hours. Hour 100 carries a d83 record; hour
        // 101 carries only another domain's DATA record. Every hour is
        // structurally well formed — a DOMAIN_DATA record opens it and a
        // FRAME_END closes it — so only the completeness check can see it.
        let mut s = Vec::new();
        s.extend(domain_data(1, 100));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &[0u8; 8],
        ));
        s.extend(frame_end(4, 101));
        let err = match scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy) {
            Err(err) => err,
            Ok(_) => panic!("a closed hour with no d83 record must refuse the stream"),
        };
        assert!(
            err.contains("every closed hour"),
            "the refusal names the completeness rule: {err}"
        );
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time: the
        // fold clears the hour and folds nothing, so the hour is not d83 data and
        // must not enter either arm.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D83,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d83_payload(1 << bit, 7, 0),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D83,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d83_payload(0, 7, 1),
            ));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy)
                .expect("the stream itself is well-formed");
            assert_eq!(scan.failure_records, 1, "bit {bit}");
            assert_eq!(scan.hours.len(), 1, "bit {bit}");
            assert_eq!(scan.hours[0].game_hour, 101, "bit {bit}");
        }
        // A non-failure status bit (DELTA_DATA, 1<<10) is NOT a failure — and for
        // d83 it is also not a payload the codec can carry cleanly: the frame must
        // then be the rowless unavailable form, which the fold turns into a hard
        // error (`world_fold.rs:11449`), so the scan refuses the stream rather
        // than measuring an hour with no state.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D83,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d83_payload(1 << 10, 7, 0),
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_stream_with_no_d83_records_is_refused_not_reported_as_zero() {
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
    fn a_malformed_d83_payload_is_refused_not_skipped() {
        // A short or inconsistent payload is a corrupt stream, not an hour to
        // drop quietly.
        let mut payload = d83_payload(0, 7, 0);
        payload.truncate(payload.len() - 1);
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D83,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err());

        // A block the FOLD rejects must make the SCAN fail, not be accepted and
        // then unwind while measuring. The whole rejection space, not one case.
        let mut invalid: Vec<Vec<u8>> = Vec::new();
        // zero tag_db_id (`lib.rs:249`)
        invalid.push(d83_payload(0, 0, 0));
        let body_at = d83::HEADER_LEN + 12;
        // capacity below NUKE_TYPE_COUNT (`lib.rs:252`)
        let mut capacity = d83_payload(0, 7, 0);
        capacity[body_at + 4..body_at + 8].copy_from_slice(&1i32.to_le_bytes());
        invalid.push(capacity);
        // entry_count that is not NUKE_TYPE_COUNT (`lib.rs:255`)
        let mut entry_count = d83_payload(0, 7, 0);
        entry_count[body_at + 8..body_at + 12].copy_from_slice(&3u32.to_le_bytes());
        invalid.push(entry_count);
        // a type ordinal that is not its own index (`lib.rs:258`)
        let mut ordinal = d83_payload(0, 7, 0);
        ordinal[body_at + 12..body_at + 16].copy_from_slice(&1u32.to_le_bytes());
        invalid.push(ordinal);
        // a non-zero entry reserved word (`lib.rs:369`)
        let mut reserved = d83_payload(0, 7, 0);
        reserved[body_at + 16..body_at + 20].copy_from_slice(&1u32.to_le_bytes());
        invalid.push(reserved);
        // a changed authority blob (`lib.rs:337`)
        let mut blob = d83_payload(0, 7, 0);
        blob[32] ^= 0x01;
        invalid.push(blob);
        // a receipted frame carrying a status word (`lib.rs:281`)
        let mut dirty = d83_payload(0, 7, 0);
        dirty[12..16].copy_from_slice(&1u32.to_le_bytes());
        invalid.push(dirty);
        for (i, payload) in invalid.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D83,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err(),
                "invalid payload {i} must make the scan fail"
            );
        }

        // A country repeated within ONE snapshot is a rejected snapshot
        // (`lib.rs:249`), and it must be caught in the scan.
        let mut payload = d83_payload(0, 7, 0);
        let block = payload[d83::HEADER_LEN + 12..].to_vec();
        payload.extend_from_slice(&block);
        payload[d83::HEADER_LEN + 8..d83::HEADER_LEN + 12].copy_from_slice(&2u32.to_le_bytes());
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D83,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        assert!(
            scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy).is_err(),
            "duplicate country in one frame"
        );
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
            let _ = scan_stream(&bytes, &test_schema(), StreamIdentityMode::Legacy);
        }
        // And corruption of a genuinely well-formed stream, byte by byte.
        let good = good_stream();
        for i in 0..good.len() {
            let mut s = good.clone();
            s[i] ^= 0xFF;
            let _ = scan_stream(&s, &test_schema(), StreamIdentityMode::Legacy);
        }
    }
}
