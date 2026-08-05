//! Measure the wire format against the shipping compact stream on REAL d62
//! `hub_motorization_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d62-scheduled capture
//! (`runs/batch1/deepwar_t1`). The stream's own d62 DATA records are the
//! shipping format's cost for the domain: a 32-byte header plus a fixed 32-byte
//! row per motorization map entry, full-replace every hour, exactly what the
//! recorder stores today. This tool replays those records into per-hour states —
//! the same fold `world_fold::fold_hub_motorization` performs — and re-encodes
//! every hour with this crate's grammar: a BASELINE at each 48-hour block start,
//! field-granular stored-minimal DELTAs between.
//!
//! **Nothing here re-derives a contract it can call.** Row and payload decoding
//! go through `wire_format::d62`'s public boundary; H4CR framing, record
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
//!   (`recorder_host/src/stream_fold.rs:277`) that do not require DOMAIN_PERF
//!   accounting: a DOMAIN_DATA record opens its game hour, a FRAME_END closes
//!   the open hour and only that hour, and closed hours strictly increase;
//! - that tracker's DOMAIN_PERF hour binding (`:328`-`:365`), including the DATA
//!   arm's pending-PERF rule (`:378`-`:386`);
//! - FRAME_END seal validation (`stream_fold.rs:46`-`:89`) and the seal's count
//!   cross-check against distinct non-probe PERF domains (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority. This crate cannot depend on `recorder_host` (it pulls in every
//! capture lane), so the relevant predicates are mirrored here.
//!
//! d62's status-independent payload authority is
//! `decode_compact_hub_motorization_live_data_payload`
//! (`domain_data_authority.rs:8248`-`:8315`), which delegates to the lane decoder.
//! `d62::split_payload` already enforces all header predicates before exclusion,
//! and the harness already walked every row there too. The sweep names that
//! existing row pass [`check_authority_payload`] and keeps it before status:
//! bounds, motorization level, and the exact verification-flag set all apply to
//! excluded records. Duplicate keys remain a fold-only, post-exclusion rule.
//! Identity mode is stated by the caller (`stream_fold.rs:220`), never sniffed.
//!
//! The DATA payloads of every domain other than 62 pass UNVALIDATED beyond
//! header sanity, the stream validator and the hour-boundary rules —
//! production validates every domain's payload (`stream_fold.rs:593`); this
//! harness owns only d62's grammar. That unvalidated remainder is explicitly
//! BOUNDED: every closed game hour must carry exactly one d62 DATA record of
//! any status (the `d62_records == closed_hours` completeness check in
//! [`scan_stream`]), so a d62 record relabelled to another domain id — or
//! dropped — refuses the run instead of silently shrinking it.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d62.bin   — the d62 DATA records verbatim (32-byte H4CR header +
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
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedState,
};
use wire_format::d62;

const D62: u16 = 62;
const BLOCK_HOURS: usize = 48;

/// One usable d62 hour: its game hour, the folded state of that hour, and its
/// byte span in the source stream (the shipping arm is those bytes verbatim,
/// header included).
///
/// The state is built during the scan, not later: every way a row can be
/// rejected — an out-of-range id, an illegal motorization level, a row missing a
/// verification witness, a duplicated key triple — is a property of the stream,
/// so it belongs to the pass that either accepts the stream or refuses it.
/// Deferring it would leave the harness able to accept a stream and then unwind
/// while measuring it.
struct D62Hour {
    game_hour: i32,
    state: KeyedState,
    ship_span: Range<usize>,
    /// True when this measured hour is exactly one game hour after the previous
    /// MEASURED hour. False on the first measured hour and after any gap — an
    /// excluded failure-status hour or a non-contiguous hour step — which is
    /// what forces the encoder to open a fresh baseline (`opens_baseline`).
    follows_previous: bool,
}

struct StreamScan {
    hours: Vec<D62Hour>,
    /// d62 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// Measured d62 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
}

/// Whether the wire arm opens a BASELINE at this measured hour.
///
/// Two reasons, and both are `WIRE_FORMAT.md` rules, not conveniences:
/// the 48-hour block cadence (every block opens with its own baseline, §1.5's
/// closing paragraph), and a gap — `WIRE_FORMAT.md:187`-`:190`: "A gap
/// affecting the state part forces the next state observation to be a
/// `BASELINE`." Every hour this domain did not successfully sample is such a
/// gap — the excluded failure-status hour, and any non-contiguous hour step —
/// and d62's cadence is hourly, so a measured hour that does not follow the
/// previous measured one by exactly one is preceded by one. Neither a delta
/// nor a retained reader state can be justified across it.
fn opens_baseline(hours_seen: usize, follows_previous: bool) -> bool {
    hours_seen % BLOCK_HOURS == 0 || !follows_previous
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

/// The row walk performed on every d62 record by
/// `decode_compact_hub_motorization_live_data_payload`
/// (`domain_data_authority.rs:8248`-`:8315`). Header predicates are already
/// enforced by `d62::split_payload`.
fn check_authority_payload(
    collection: &wire_format::Collection,
    body: &[u8],
    at_byte: usize,
) -> Result<Vec<(wire_format::body::Key, wire_format::Record)>, String> {
    let mut decoded = Vec::with_capacity(body.len() / d62::ROW_LEN);
    for row in body.chunks_exact(d62::ROW_LEN) {
        let key = d62::key_from_row(row).map_err(|e| format!("d62 row at byte {at_byte}: {e}"))?;
        let record = d62::record_from_row(collection, row, 0)
            .map_err(|e| format!("d62 row at byte {at_byte}: {e}"))?;
        decoded.push((key, record));
    }
    Ok(decoded)
}

/// Validate the whole stream against the canonical contract and fold the usable
/// d62 DATA records into per-hour states. Every rejection path is an `Err`,
/// never a silently shorter measurement and never a panic — see the module docs
/// for what is enforced and what the recorder's own validator additionally
/// checks.
fn scan_stream(
    stream: &[u8],
    collection: &wire_format::Collection,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    // Production receives the identity mode from its caller
    // (`stream_fold.rs:220`); the first record cannot state it safely.
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours: Vec<D62Hour> = Vec::new();
    let mut failure_records = 0usize;
    let mut non_contiguous = 0usize;
    // COMPLETENESS accounting: every d62 DATA record of ANY status, against
    // every closed game hour — see the check after the EOF rules.
    let mut d62_records = 0usize;
    let mut closed_hours = 0usize;

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d62 DATA record of ANY status — the uniqueness rule.
    let mut last_d62_hour: Option<i32> = None;
    // Hour of the last d62 record that entered the measurement — the
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

        if header.domain_id == D62 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d62_records += 1;
            let (d62_header, body) = d62::split_payload(payload)
                .map_err(|e| format!("d62 payload at byte {pos}: {e}"))?;

            // One d62 DATA record per game hour, checked over EVERY d62 record
            // and BEFORE any status-based exclusion. d62 is full-replace, so a
            // second record for the same hour would become a second measurement
            // hour built from the same clock time — and gating this on the
            // record being usable would let a success/failure pair through the
            // rule entirely.
            if let Some(previous) = last_d62_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d62 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d62_hour = Some(header.game_hour_raw);

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
            let unknown_status = compact_status_unknown_flags_v1(d62_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d62 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            let decoded = check_authority_payload(collection, body, pos)?;

            // **Per-row decoding runs BEFORE the status check, on every d62
            // record, because that is the order the fold runs it in.**
            // `fold_hub_motorization` calls `decode_payload_rows` first
            // (`world_fold.rs:15851`) and only reaches `handle_compact_failure`
            // (`:15862`) if that decode returned `Ok`. `decode_payload_rows`
            // applies `validate_row` (`lanes/hub_motorization_live/src/lib.rs:295`,
            // `:327`) to every row, so a malformed row in a FAILURE-status record
            // is still an error that marks the domain unknown and returns `Err`
            // (`:15854`-`:15860`) — the failure status never gets looked at.
            // Deferring this until after the status check would let this harness
            // accept a stream the fold refuses.
            // `world_fold::handle_compact_failure` (called at `world_fold.rs:15862`)
            // marks the domain-hour unknown and RETAINS the prior accumulator on
            // a failure status, folding nothing from this record. The predicate
            // is `compact_contract`'s own, not a copied bit mask: it already
            // applies d62's lane-local mask (none) and expected partial
            // tolerance (none), so it cannot drift from the recorder.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D62,
                d62_header.status_flags,
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

                // The rest of the fold, in the fold's own order: everything from
                // here down sits AFTER `handle_compact_failure` in
                // `fold_hub_motorization` (`:15869`-`:15909`) and so applies only
                // to a record the fold actually folds. A duplicated
                // `(owner, province, actor)` triple marks the domain unknown and
                // folds nothing (`:15881`-`:15897`) — never a last-writer-wins
                // overwrite — but in a failure-status record the fold has already
                // returned and never looks, which is why this is not hoisted.
                let mut rows = 0usize;
                let mut state: KeyedState = BTreeMap::new();
                for (key, record) in decoded {
                    rows += 1;
                    if state.insert(key, record).is_some() {
                        return Err(format!(
                            "d62 payload at byte {pos}: duplicate owner/province/actor row"
                        ));
                    }
                }
                // `world_fold.rs:15899`: the fold counts the rows it walked and
                // requires the header's total to match. `split_payload` checks
                // the same total against the declared row count; this closes the
                // third side of the triangle against the rows actually walked.
                if rows != d62_header.row_count as usize
                    || rows != d62_header.total_map_entries as usize
                {
                    return Err(format!("d62 payload at byte {pos}: row count mismatch"));
                }
                hours.push(D62Hour {
                    game_hour: header.game_hour_raw,
                    state,
                    ship_span: pos..record_end,
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
        return Err("stream carries no usable d62 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d62 DATA
    // record, of any status. This bounds what this harness canNOT validate:
    // production runs `validate_domain_data_payload_at` on EVERY domain's DATA
    // payload (`stream_fold.rs:593`), while this harness owns only d62's
    // grammar, so a d62 record whose header was relabelled to another domain
    // id — or lost outright — would otherwise be silently ignored and the run
    // would report a plausible measurement over fewer hours. A per-capture
    // completeness assertion, NOT a grammar rule: a capture where d62 is not
    // scheduled every hour fails here loudly and a human adjusts, instead of a
    // number quietly shrinking (`bench_real_d65.rs` precedent).
    if d62_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d62_records} d62 DATA \
             record(s); every closed hour of a d62-scheduled capture must carry exactly one, \
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

/// The wire arm — BASELINE at each block start and at every §1.5 gap,
/// stored-minimal DELTA between — with the per-hour reader replay. Factored
/// out of `main` so the tests execute the PRODUCTION encode path over a
/// scanned stream: a mutant that reverts the baseline decision here to
/// block-position-only fails the gap test (a Codex round-1 finding).
struct WireMeasurement {
    wire: Vec<u8>,
    wire_offsets: Vec<usize>,
    hours_seen: usize,
    baselines: usize,
    gap_forced_baselines: usize,
    empty_hours: usize,
    changed_rows_total: usize,
    tombstones_total: usize,
    rows_total: usize,
}

fn measure_wire_arm(
    schema: &wire_format::Schema,
    collection: &wire_format::Collection,
    hours: &[D62Hour],
) -> WireMeasurement {
    let mut m = WireMeasurement {
        wire: Vec::new(),
        wire_offsets: vec![0usize],
        hours_seen: 0,
        baselines: 0,
        gap_forced_baselines: 0,
        empty_hours: 0,
        changed_rows_total: 0,
        tombstones_total: 0,
        rows_total: 0,
    };
    let mut prior: Option<&KeyedState> = None;
    let mut reader: Option<KeyedState> = None;

    for hour in hours {
        // The hour's state, folded during the scan: d62 is full-replace, so the
        // hour's rows ARE the hour's whole state.
        let state: &KeyedState = &hour.state;
        m.rows_total += state.len();

        // BASELINE at each block start — and at every §1.5 gap, which forces a
        // fresh baseline and restarts the delta chain (`opens_baseline`).
        let hour_start = m.wire.len();
        let block_start = opens_baseline(m.hours_seen, hour.follows_previous);
        if block_start && m.hours_seen % BLOCK_HOURS != 0 {
            m.gap_forced_baselines += 1;
        }
        let mut emitted = true;
        if block_start || prior.is_none() {
            encode_baseline(schema, std::slice::from_ref(state), &mut m.wire)
                .expect("baseline encodes");
            m.baselines += 1;
        } else {
            let prev = prior.unwrap();
            let d = diff(collection, prev, state);
            if d.is_empty() {
                m.empty_hours += 1;
                emitted = false;
            } else {
                m.changed_rows_total += d.changed.len();
                m.tombstones_total += d.tombstones.len();
                encode_delta(schema, std::slice::from_ref(prev), &[Some(d)], &mut m.wire)
                    .expect("delta encodes");
            }
        }
        m.wire_offsets.push(m.wire.len());

        // Reader replay: decode the bytes just written against the reader's OWN
        // reconstructed prior, exactly as a conforming reader would, and require
        // the result to be stored-equal to this hour's state. An hour that
        // emitted nothing is still checked: the reader's retained state must
        // already equal the fold's, which is precisely what "nothing changed"
        // asserts.
        if emitted {
            let hour_bytes = &m.wire[hour_start..];
            if block_start || reader.is_none() {
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
        m.hours_seen += 1;
    }
    m
}

fn main() {
    const USAGE: &str = "usage: bench_real_d62 <collector_stream.bin> <out dir> \
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
    let schema = d62::schema();
    let collection = &schema.collections[0];

    // ---- Pass 1: validate the WHOLE stream and fold its d62 DATA records. ---
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result. Everything that
    // can fail on stream content fails here, with a diagnosis and no unwind.
    let scan = match scan_stream(&stream, collection, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: encode both arms. Nothing here reads stream content again,
    // so every assertion below is about the codec, not about the input. -------

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());
    }

    let m = measure_wire_arm(&schema, collection, &scan.hours);
    let wire = m.wire;
    let wire_offsets = m.wire_offsets;
    let hours_seen = m.hours_seen;
    let baselines = m.baselines;
    let gap_forced_baselines = m.gap_forced_baselines;
    let empty_hours = m.empty_hours;
    let changed_rows_total = m.changed_rows_total;
    let tombstones_total = m.tombstones_total;
    let entries_total = m.rows_total;

    for (name, bytes) in [("ship_d62.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d62.offsets", &ship_offsets),
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
        "map entries avg      {:.1}",
        entries_total as f64 / hours_seen as f64
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
    println!("ship_d62.bin         {} bytes", ship.len());
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
        d62::schema().collections[0].clone()
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

    /// One 32-byte d62 row, in the lane's row order.
    fn row(owner: u32, actor: u32, province: u32, level: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity(d62::ROW_LEN);
        for word in [owner, actor, province, 5, 1, 3, level, d62::REQUIRED_FLAGS] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out
    }

    /// A d62 DATA payload: the 32-byte header then the rows.
    fn d62_payload(status_flags: u32, rows: &[Vec<u8>]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        payload.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&d62::SCHEMA_VERSION.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes()); // total_settings_nodes
        payload.extend_from_slice(&(d62::ROW_LEN as u32).to_le_bytes());
        payload.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // total_map_entries
        assert_eq!(payload.len(), d62::HEADER_LEN);
        for r in rows {
            payload.extend_from_slice(r);
        }
        payload
    }

    fn one_row_payload(status_flags: u32, level: u32) -> Vec<u8> {
        d62_payload(status_flags, &[row(16, 17, 13071, level)])
    }

    /// Two closed hours, each carrying one d62 DATA record.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D62,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 1),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D62,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 2),
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
            D62,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 1),
        )
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        let mut ok = domain_data(1, 100);
        ok.extend(frame_end(2, 100));
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
        ok.extend(perf(2, 100, D62));
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
            s.extend(perf(2, 100, D62));
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
        let collection = test_collection();
        for payload in row_rejecting_payloads(1) {
            let (_, body) = d62::split_payload(&payload).expect("header remains well-formed");
            assert!(check_authority_payload(&collection, body, 0).is_err());
        }

        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let bad = d62_payload(1 << bit, &[row(0, 17, 13071, 1)]);
            let mut s = record(1, 100, D62, COMPACT_RECORD_TYPE_DOMAIN_DATA, &bad);
            s.extend(frame_end(2, 100));
            s.extend(domain_data(3, 101));
            s.extend(frame_end(4, 101));
            assert!(scan_stream(&s, &collection, StreamIdentityMode::Legacy).is_err());
        }

        let excluded = one_row_payload(1, 1);
        let mut s = record(1, 100, D62, COMPACT_RECORD_TYPE_DOMAIN_DATA, &excluded);
        s.extend(frame_end(2, 100));
        s.extend(domain_data(3, 101));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &collection, StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 1);

        // Duplicate keys are a fold-only rule after status exclusion, not part
        // of the authority decoder. The excluded record therefore stays valid.
        let duplicate = d62_payload(1, &[row(16, 17, 13071, 1), row(16, 17, 13071, 2)]);
        let mut s = record(1, 100, D62, COMPACT_RECORD_TYPE_DOMAIN_DATA, &duplicate);
        s.extend(frame_end(2, 100));
        s.extend(domain_data(3, 101));
        s.extend(frame_end(4, 101));
        assert!(scan_stream(&s, &collection, StreamIdentityMode::Legacy).is_ok());
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
    fn a_well_formed_stream_yields_its_d62_hours() {
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
        assert_eq!(scan.hours[0].state.len(), 1);
        // The key is the fold's triple in the fold's order, not row order.
        assert_eq!(
            scan.hours[0].state.keys().next().unwrap(),
            &wire_format::body::key(&[16, 13071, 17])
        );
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
            // informational bit; with rows and without. The rejection must not
            // depend on any of it.
            for extra in [0u32, 1, 1 << 10] {
                for rows in [Vec::new(), vec![row(16, 17, 13071, 1)]] {
                    let mut s = Vec::new();
                    s.extend(record(
                        1,
                        100,
                        D62,
                        COMPACT_RECORD_TYPE_DOMAIN_DATA,
                        &d62_payload(status | extra, &rows),
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
        // `fold_hub_motorization` never reads an informational bit at all.
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D62,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(informational, 1),
            ));
            s.extend(frame_end(2, 100));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 1, "bit {informational:#x}");
            assert_eq!(scan.failure_records, 0, "bit {informational:#x}");
        }
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        // A cut immediately after a d62 DATA record leaves that hour open and
        // ends the stream on a record that is not a FRAME_END. That would
        // otherwise produce a plausible shorter number; it must be refused.
        // Records 1 and 3 of `good_stream` are the DATA records.
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
        // Version, magic and header length, at every record in the stream —
        // the canonical parser owns these and the harness must not read past it.
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
            D62,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 1),
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
        // A closed hour that repeats or goes backwards would fold two different
        // clock times into one measurement step.
        let mut s = Vec::new();
        s.extend(record(
            1,
            101,
            D62,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 1),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D62,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 2),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_second_d62_record_in_one_hour_is_rejected() {
        // d62 is full-replace; two records in one hour would become two
        // measurement hours built from the same clock time. The rule is checked
        // over EVERY d62 DATA record and before status-based exclusion, so all
        // four success/failure orderings are refused — gating it on the record
        // being usable would let a success + failure pair through.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D62,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(first, 1),
            ));
            s.extend(record(
                2,
                100,
                D62,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(second, 2),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "two d62 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn an_excluded_hour_forces_the_next_observation_to_be_a_baseline() {
        // `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the state part forces
        // the next state observation to be a `BASELINE`." Both gap shapes d62
        // can meet — an excluded failure-status hour, and a whole hour the
        // capture never scheduled — must clear `follows_previous`, and
        // `opens_baseline` must open a baseline on it wherever it lands in a
        // block. A delta would relate two states across unobserved time.
        //
        // Shape 1: hour 101 carries a failure-status d62 record (excluded).
        let mut s = Vec::new();
        s.extend(domain_data(1, 100));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D62,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(1 << 0, 2), // failure status -> excluded
        ));
        s.extend(frame_end(4, 101));
        s.extend(record(
            5,
            102,
            D62,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 2),
        ));
        s.extend(frame_end(6, 102));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.non_contiguous, 1);
        assert!(!scan.hours[0].follows_previous, "the first hour follows none");
        assert!(
            !scan.hours[1].follows_previous,
            "hour 102 does not follow hour 100"
        );
        // ...and that is what makes the encoder open a baseline for it, even
        // though it is only the second measured hour of the block.
        assert!(opens_baseline(1, scan.hours[1].follows_previous));
        // Through the PRODUCTION encode path, not just the decision function:
        // the second measured hour must be a gap-forced baseline, not a delta.
        let m = measure_wire_arm(&d62::schema(), &test_collection(), &scan.hours);
        assert_eq!(m.hours_seen, 2);
        assert_eq!(m.baselines, 2, "hour 100 opens the block; hour 102 is gap-forced");
        assert_eq!(m.gap_forced_baselines, 1);
        // The gap hour's bytes decode as a BASELINE on their own.
        let (states, consumed) = decode_baseline(
            &d62::schema(),
            &m.wire[m.wire_offsets[1]..m.wire_offsets[2]],
            0,
        )
        .expect("the post-gap hour decodes as a baseline");
        assert_eq!(consumed, m.wire_offsets[2] - m.wire_offsets[1]);
        assert_eq!(states[0].len(), 1);

        // Shape 2: hour 101 is never scheduled at all — the capture closes
        // hours 100 and 102 only.
        let mut s = Vec::new();
        s.extend(domain_data(1, 100));
        s.extend(frame_end(2, 100));
        s.extend(domain_data(3, 102));
        s.extend(frame_end(4, 102));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
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
        let scan = scan_stream(
            &good_stream(),
            &test_collection(),
            StreamIdentityMode::Legacy,
        )
        .expect("well-formed");
        assert_eq!(scan.non_contiguous, 0);
        assert!(scan.hours[1].follows_previous);
        assert!(!opens_baseline(1, scan.hours[1].follows_previous));
        let m = measure_wire_arm(&d62::schema(), &test_collection(), &scan.hours);
        assert_eq!(m.baselines, 1, "only the block boundary baselines");
        assert_eq!(m.gap_forced_baselines, 0);
    }

    #[test]
    fn a_closed_hour_without_a_d62_record_refuses_the_stream() {
        // The cross-domain bound: this harness validates only d62 payloads, so
        // a d62 record relabelled to another domain — or dropped — would
        // otherwise be silently ignored and the run would report a plausible
        // measurement over fewer hours. Hour 100 carries a d62 record; hour
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
        let err = match scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy) {
            Err(err) => err,
            Ok(_) => panic!("a closed hour with no d62 record must refuse the stream"),
        };
        assert!(
            err.contains("every closed hour"),
            "the refusal names the completeness rule: {err}"
        );
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time:
        // the fold retains the accumulator and folds nothing, so the hour is not
        // d62 data and must not enter either arm.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D62,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(1 << bit, 1),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D62,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(0, 2),
            ));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .expect("the stream itself is well-formed");
            assert_eq!(scan.failure_records, 1, "bit {bit}");
            assert_eq!(scan.hours.len(), 1, "bit {bit}");
            assert_eq!(scan.hours[0].game_hour, 101, "bit {bit}");
        }
        // And a non-failure bit (DELTA_DATA, 1<<10) is NOT treated as failure.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D62,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(1 << 10, 1),
        ));
        s.extend(frame_end(2, 100));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn a_stream_with_no_d62_records_is_refused_not_reported_as_zero() {
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
    fn a_malformed_d62_payload_is_refused_not_skipped() {
        // A short or inconsistent d62 payload is a corrupt stream, not an hour
        // to drop quietly.
        let mut payload = one_row_payload(0, 1);
        payload.truncate(payload.len() - 1);
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D62,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());

        // A row the CODEC rejects must make the SCAN fail, not be accepted and
        // then unwind while measuring. The whole row-rejection space, not one
        // sampled case. (The status crossing lives in
        // `a_malformed_row_is_refused_whatever_the_records_status`.)
        let mut invalid: Vec<Vec<u8>> = row_rejecting_payloads(0);
        // A header declaring the wrong schema version or row stride.
        for (offset, value) in [(16usize, 3u32), (24, 16)] {
            let mut bad = d62_payload(0, &[row(16, 17, 13071, 1)]);
            bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            invalid.push(bad);
        }
        // A header whose total_map_entries disagrees with its row count
        // (`world_fold.rs:15899`).
        let mut mismatch = d62_payload(0, &[row(16, 17, 13071, 1)]);
        mismatch[28..32].copy_from_slice(&7u32.to_le_bytes());
        invalid.push(mismatch);

        for (i, payload) in invalid.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D62,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "invalid payload {i} must make the scan fail"
            );
        }

        // A key triple repeated within ONE frame marks the domain unknown and
        // folds nothing (`world_fold.rs:15881`-`:15897`) — it must be caught in
        // the scan, not asserted in the measuring pass. Two rows that differ in
        // a NON-key word still collide, which is the case a key-blind duplicate
        // check would miss.
        let mut dup_row = row(16, 17, 13071, 1);
        dup_row[16..20].copy_from_slice(&99u32.to_le_bytes()); // other record_index
        let payload = d62_payload(0, &[row(16, 17, 13071, 1), dup_row]);
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D62,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        assert!(
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
            "duplicate (owner, province, actor) in one frame"
        );

        // Two rows that differ ONLY in a key word are NOT duplicates.
        let payload = d62_payload(0, &[row(16, 17, 13071, 1), row(16, 18, 13071, 2)]);
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D62,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
            .expect("two distinct actors");
        assert_eq!(scan.hours[0].state.len(), 2);
    }

    /// Every payload whose ROWS the codec rejects, paired with every status.
    fn row_rejecting_payloads(status_flags: u32) -> Vec<Vec<u8>> {
        let mut invalid: Vec<Vec<u8>> = vec![
            d62_payload(status_flags, &[row(0, 17, 13071, 1)]), // owner 0
            d62_payload(status_flags, &[row(16, 0, 13071, 1)]), // actor 0
            d62_payload(status_flags, &[row(16, 17, 0, 1)]),    // province 0
            d62_payload(
                status_flags,
                &[row(d62::MAX_COUNTRY_COUNT + 1, 17, 13071, 1)],
            ),
            d62_payload(
                status_flags,
                &[row(16, d62::MAX_COUNTRY_COUNT + 1, 13071, 1)],
            ),
            d62_payload(status_flags, &[row(16, 17, d62::MAX_PROVINCE_ID + 1, 1)]),
            d62_payload(status_flags, &[row(16, 17, 13071, 0)]), // level 0
            d62_payload(status_flags, &[row(16, 17, 13071, 3)]), // level 3
        ];
        // A zero state_id (`lib.rs:346`).
        let mut zero_state = d62_payload(status_flags, &[row(16, 17, 13071, 1)]);
        zero_state[d62::HEADER_LEN + 12..d62::HEADER_LEN + 16].copy_from_slice(&0u32.to_le_bytes());
        invalid.push(zero_state);
        // A missing verification witness, an unknown flag bit, and no flags at
        // all (`lib.rs:358`-`:368`).
        for flags in [
            d62::REQUIRED_FLAGS & !d62::ROW_FLAG_ACTOR_VERIFIED,
            d62::REQUIRED_FLAGS | (1 << 20),
            0,
        ] {
            let mut bad = d62_payload(status_flags, &[row(16, 17, 13071, 1)]);
            bad[d62::HEADER_LEN + 28..d62::HEADER_LEN + 32].copy_from_slice(&flags.to_le_bytes());
            invalid.push(bad);
        }
        invalid
    }

    #[test]
    fn a_malformed_row_is_refused_whatever_the_records_status() {
        // The reviewer's input, generalized. `fold_hub_motorization` calls
        // `decode_payload_rows` FIRST (`world_fold.rs:15851`) and only reaches
        // `handle_compact_failure` (`:15862`) if that decode succeeded, so a row
        // `validate_row` rejects is an error even in a failure-status record —
        // the status is never examined. A harness that classified the status
        // first would accept a stream the fold refuses. Swept over the whole
        // row-rejection space, crossed with a clean status, the reviewer's
        // `status_flags = 1`, and another failure bit.
        for status in [0u32, 1, 1 << 3] {
            for (i, payload) in row_rejecting_payloads(status).into_iter().enumerate() {
                let mut s = Vec::new();
                s.extend(record(
                    1,
                    100,
                    D62,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &payload,
                ));
                s.extend(frame_end(2, 100));
                // A second, wholly valid hour: without it a bug could be masked
                // by the "no usable d62 records" refusal rather than caught.
                s.extend(record(
                    3,
                    101,
                    D62,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &one_row_payload(0, 2),
                ));
                s.extend(frame_end(4, 101));
                assert!(
                    scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                    "payload {i} with status {status:#x} must make the scan fail"
                );
            }
        }
    }

    #[test]
    fn a_duplicate_key_in_a_failure_status_record_is_excluded_not_refused() {
        // The other direction of the same ordering rule, and the reason the
        // duplicate check is NOT hoisted alongside the row decode: the fold's
        // duplicate detection lives at `world_fold.rs:15881`, AFTER
        // `handle_compact_failure` has already returned `Ok` for a failed
        // record. Imposing a check the fold does not perform would make this
        // harness refuse a stream the recorder accepts.
        let dup = d62_payload(1, &[row(16, 17, 13071, 1), row(16, 17, 13071, 2)]);
        let mut s = Vec::new();
        s.extend(record(1, 100, D62, COMPACT_RECORD_TYPE_DOMAIN_DATA, &dup));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D62,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 2),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
            .expect("the fold accepts this stream");
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 1, "only the clean hour is measured");
        assert_eq!(scan.hours[0].game_hour, 101);

        // ... and with a clean status the very same payload IS refused.
        let dup = d62_payload(0, &[row(16, 17, 13071, 1), row(16, 17, 13071, 2)]);
        let mut s = Vec::new();
        s.extend(record(1, 100, D62, COMPACT_RECORD_TYPE_DOMAIN_DATA, &dup));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
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
}
