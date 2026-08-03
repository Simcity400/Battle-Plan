//! Measure the wire format against the shipping compact stream on REAL d68
//! `session_header_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d68-scheduled capture
//! (`runs/batch2/deepwar_t1`, 146 hours; `runs/batch1/deepwar_t1`, 903 hours).
//! The stream's own d68 DATA records are the shipping format's cost for the
//! domain: a 16-byte header, a fixed 20-byte scalar block and three
//! length-prefixed UTF-8 strings, one record per game hour.
//!
//! **d68 is not a table, and that is what this harness measures.** Each record is
//! an *update*, merged last-value-wins per field into ONE carried header
//! (`world_fold.rs:10143`-`:10167`); an unset presence bit means "unchanged this
//! frame", never "cleared". The per-hour state is therefore the accumulator after
//! the merge, not the record — so the fold runs through `wire_format::d68`'s
//! public `Accumulator`/`fold_payload` boundary rather than being re-derived here, and
//! the reader replay asserts the reader's reconstruction is stored-equal to that
//! accumulator every hour.
//!
//! **SPLIT deliverable.** Only the five scalars are encoded by the wire arm. The
//! three strings (`ideology`, `cosmetic_tag`, `version`) are stop-reported: they
//! are parsed (the fold's trailing-byte rule makes the payload invalid if they are
//! malformed) and their bytes are accounted for in the shipping arm two ways —
//! a byte split of every shipping record, and a second, cut shipping arm
//! `ship_d68_no_strings.bin` with the string block removed. That block is a
//! payload SUFFIX (`d68::STRING_BLOCK_OFFSET`..end), so the cut is exact; the cut
//! arm keeps each record's `payload_len` field as written, which makes it a
//! size-accounting artifact and NOT a valid compact record — d11's `PDQ1`
//! precedent (`bin/bench_real_d11.rs:26`-`:34`).
//!
//! **Nothing here re-derives a contract it can call.** Payload decoding, the
//! merge and the mapping go through `wire_format::d68`; H4CR framing, record
//! semantics and sequence continuity go through `compact_contract`'s own parser
//! and stateful validator.
//!
//! **The whole stream is validated before any artifact is written**, exactly as
//! `bench_real_d63.rs` does and for the same reason — a truncated or spliced
//! stream would otherwise yield a smaller but entirely plausible number.
//! `scan_stream` enforces, fail-closed:
//!
//! - every record header through `compact_contract::parse_compact_record_header`;
//! - every record through `compact_contract::stream_validator::StatefulStreamValidator`;
//! - the hour-boundary rules of `recorder_host::stream_fold::BoundaryTracker`
//!   (`recorder_host/src/stream_fold.rs:277`), including its DOMAIN_PERF hour
//!   binding (`:328`-`:365`) and the DATA arm's pending-PERF rule
//!   (`:378`-`:386`);
//! - the FRAME_END seal fields (`:46`-`:89`) and the seal's cross-check against
//!   distinct non-probe PERF domains (`:411`-`:419`);
//! - the EOF rules of its `eof_check` (`:459`).
//!
//! Production's status-independent payload authority,
//! `decode_compact_session_header_data_payload`
//! (`domain_data_authority.rs:12718`-`:12884`), applies the complete d68 grammar
//! to every record whatever its status: known presence flags, the scalar block,
//! zero-packed absent scalars, bounded present player id, three bounded
//! length-prefixed UTF-8 strings with flag agreement, and exact consumption.
//! Before exclusion this harness previously ran only `d68::split_payload`'s
//! 16-byte minimum. [`check_authority_payload`] now calls the domain module's own
//! complete `parse_update` boundary at that point. There is no extra fold-only
//! payload predicate after `parse_update`; a successful fold only merges it.
//!
//! The seal fields, PERF hour binding, pending-PERF DATA rule,
//! status-independent payload authority and stated identity mode are the five
//! checks carried back by the five-check sweep. The mode defaults to legacy, the
//! protocol of both measured captures.
//!
//! The DATA payloads of every domain other than 68 pass UNVALIDATED beyond
//! header sanity, the stream validator and the hour-boundary rules —
//! production validates every domain's payload (`stream_fold.rs:593`); this
//! harness owns only d68's grammar. That unvalidated remainder is explicitly
//! BOUNDED: every closed game hour must carry exactly one d68 DATA record of
//! any status (the `d68_records == closed_hours` completeness check in
//! [`scan_stream`]), so a d68 record relabelled to another domain id — or
//! dropped — refuses the run instead of silently shrinking it.
//!
//! **d68's three non-folding outcomes, named.** `fold_session_header` has exactly
//! one path that returns without folding, and one that returns `Err`:
//!
//! 1. **Excluded hour** — a failure status (`world_fold.rs:10044`, via
//!    `handle_compact_failure`): the recorder retains the whole prior header and
//!    folds nothing, so the hour carries no d68 data and enters neither arm. The
//!    accumulator is NOT advanced, which is what the recorder does.
//! 2. **Stream rejection** — any `Err` from the payload grammar (unknown presence
//!    flags, a short scalar block, a scalar without its flag, an out-of-range
//!    player id, a malformed string block, trailing bytes). The recorder folds the
//!    frame closed and retains the prior header; this harness refuses the run,
//!    per the recipe's third-outcome rule, because such a payload says the stream
//!    is not this grammar. No record in either capture takes this path.
//! 3. **Stream rejection, before classification** — an unassigned v1 status bit.
//!    The failure predicate classifies the *assigned* bits only
//!    (`compact_contract/src/status_flags_base.rs:21`-`:24`), so an unassigned bit
//!    has no defined classification and must be refused rather than read as
//!    "not a failure".
//!
//! There is no delta-before-baseline path: d68's fold has no baseline/delta mode
//! word at all — every payload is a merge update.
//!
//! Outputs, for the compression step (`scripts/compress_arms.py`):
//!   ship_d68.bin             — the d68 DATA records verbatim, in stream order
//!   ship_d68_no_strings.bin  — the same records with the string suffix cut
//!   wire_new.bin             — the wire-format bodies, in hour order
//!   *.offsets                — hour-boundary byte offsets, one per line

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
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedState,
};
use wire_format::d68;

const D68: u16 = 68;
const BLOCK_HOURS: usize = 48;

/// One usable d68 hour: its game hour, the MERGED accumulator after that hour's
/// update, and the record's byte span in the source stream.
struct D68Hour {
    game_hour: i32,
    /// The keyed set the fold's carried header is equivalent to: empty while the
    /// fold would still hold `None`, one row once anything has folded.
    state: KeyedState,
    ship_span: Range<usize>,
    /// Byte split of this record (`ship_span` totals to the sum of these four).
    framing_bytes: usize,
    payload_header_bytes: usize,
    scalar_bytes: usize,
    /// The stop-reported string block — a payload suffix.
    string_bytes: usize,
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
/// and d68's cadence is hourly, so a measured hour that does not follow the
/// previous measured one by exactly one is preceded by one. Neither a delta
/// nor a retained reader state can be justified across it.
fn opens_baseline(hours_seen: usize, follows_previous: bool) -> bool {
    hours_seen % BLOCK_HOURS == 0 || !follows_previous
}

struct StreamScan {
    hours: Vec<D68Hour>,
    failure_records: usize,
    non_contiguous: usize,
    /// Presence words observed over the MEASURED records, for the coverage
    /// caveat: a partial-presence merge is reachable in the grammar and may be
    /// unseen in this capture.
    presence_histogram: std::collections::BTreeMap<u32, usize>,
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

/// The complete d68 status-independent payload authority
/// (`domain_data_authority.rs:12718`-`:12884`) is the domain module's own
/// `parse_update` boundary.
fn check_authority_payload(payload: &[u8], at_byte: usize) -> Result<(), String> {
    d68::parse_update(payload)
        .map(|_| ())
        .map_err(|e| format!("d68 payload at byte {at_byte}: {e}"))
}

fn scan_stream(
    stream: &[u8],
    collection: &wire_format::Collection,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    // The protocol is stated by the caller, never inferred from the first record
    // (`stream_fold.rs:220`; `stream_validator.rs:328`).
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours: Vec<D68Hour> = Vec::new();
    // COMPLETENESS accounting: every d68 DATA record of ANY status, against
    // every closed game hour — see the check after the EOF rules.
    let mut d68_records = 0usize;
    let mut closed_hours = 0usize;
    let mut failure_records = 0usize;
    let mut non_contiguous = 0usize;
    let mut presence_histogram = std::collections::BTreeMap::new();

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    let mut last_d68_hour: Option<i32> = None;
    let mut last_measured_hour: Option<i32> = None;

    // THE accumulator: one carried header for the whole stream, exactly as
    // `WorldFold::session_header` is one `Option<SessionHeaderFoldState>` for the
    // whole capture (`world_fold.rs:1744`). It survives excluded hours untouched,
    // which is what `handle_compact_failure` does.
    let mut accumulator = d68::Accumulator::default();

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

        if header.domain_id == D68 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d68_records += 1;
            // The header split is the ONLY thing the fold does before its status
            // exit (`world_fold.rs:10041`), so it is the only thing that may run
            // before classification here.
            let (d68_header, _) = d68::split_payload(payload)
                .map_err(|e| format!("d68 payload at byte {pos}: {e}"))?;

            // One d68 DATA record per game hour, checked over EVERY d68 record and
            // BEFORE any status-based exclusion — a success+failure pair in one
            // hour would otherwise slip through. It matters more here than for a
            // full-replace domain: two merges in one hour would advance the
            // accumulator twice against one clock time.
            if let Some(previous) = last_d68_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d68 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d68_hour = Some(header.game_hour_raw);

            // VALIDITY BEFORE CLASSIFICATION (`compact_contract/src/lib.rs:121`-`:123`).
            let unknown_status = compact_status_unknown_flags_v1(d68_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d68 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            check_authority_payload(payload, pos)?;

            // Outcome 1: a failure status retains the prior header and folds
            // nothing (`world_fold.rs:10044`). The accumulator is left alone.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D68,
                d68_header.status_flags,
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
                *presence_histogram.entry(d68_header.presence).or_insert(0) += 1;

                // Outcome 2: any grammar error refuses the run. The merge is
                // all-or-nothing, so a refused payload leaves the accumulator
                // exactly as the recorder would.
                d68::fold_payload(&mut accumulator, payload)
                    .map_err(|e| format!("d68 payload at byte {pos}: {e}"))?;

                let state = d68::keyed_state(collection, &accumulator, 0)
                    .map_err(|e| format!("d68 payload at byte {pos}: {e}"))?;
                let string_bytes = payload
                    .len()
                    .checked_sub(d68::STRING_BLOCK_OFFSET)
                    .ok_or_else(|| format!("d68 payload at byte {pos}: no string block"))?;
                hours.push(D68Hour {
                    game_hour: header.game_hour_raw,
                    state,
                    ship_span: pos..record_end,
                    framing_bytes: COMPACT_RECORD_HEADER_LEN,
                    payload_header_bytes: d68::HEADER_LEN,
                    scalar_bytes: d68::SCALAR_BLOCK_LEN,
                    string_bytes,
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
        return Err("stream carries no usable d68 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d68 DATA
    // record, of any status. This bounds what this harness canNOT validate:
    // production runs `validate_domain_data_payload_at` on EVERY domain's DATA
    // payload (`stream_fold.rs:593`), while this harness owns only d68's
    // grammar, so a d68 record whose header was relabelled to another domain
    // id — or lost outright — would otherwise be silently ignored and the run
    // would report a plausible measurement over fewer hours. A per-capture
    // completeness assertion, NOT a grammar rule: a capture where d68 is not
    // scheduled every hour fails here loudly and a human adjusts, instead of a
    // number quietly shrinking (`bench_real_d65.rs` precedent).
    if d68_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d68_records} d68 DATA \
             record(s); every closed hour of a d68-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    Ok(StreamScan {
        hours,
        failure_records,
        non_contiguous,
        presence_histogram,
    })
}

/// One wire-arm hour: the §1.5/block baseline decision, the encode, and the
/// per-hour reader replay. Factored out of `main` so the tests execute the
/// PRODUCTION encode path over a scanned stream: a mutant that reverts the
/// baseline decision here to block-position-only fails the gap test (a Codex
/// round-1 finding — the earlier tests exercised the decision function but not
/// this callsite).
struct WireHourOutcome {
    opened_baseline: bool,
    /// The baseline sits outside a scheduled block start — §1.5 gap-forced.
    gap_forced: bool,
    emitted: bool,
    bytes: usize,
    changed_rows: usize,
    tombstones: usize,
}

fn encode_wire_hour(
    schema: &wire_format::Schema,
    collection: &wire_format::Collection,
    hours_seen: usize,
    hour: &D68Hour,
    prior: Option<&KeyedState>,
    reader: &mut Option<KeyedState>,
    wire: &mut Vec<u8>,
) -> WireHourOutcome {
    let state: &KeyedState = &hour.state;
    let mut changed_rows = 0usize;
    let mut tombstones = 0usize;
    let hour_start = wire.len();
    // BASELINE at each block start — and at every §1.5 gap, which forces a
    // fresh baseline and restarts the delta chain (`opens_baseline`).
    let block_start = opens_baseline(hours_seen, hour.follows_previous);
    let gap_forced = block_start && hours_seen % BLOCK_HOURS != 0;
    let mut emitted = true;
    if block_start || prior.is_none() {
        encode_baseline(schema, std::slice::from_ref(state), wire)
            .expect("baseline encodes");
        // baseline counters roll up via the returned outcome
    } else {
        let prev = prior.unwrap();
        let d = diff(collection, prev, state);
        if d.is_empty() {
            emitted = false;
        } else {
            changed_rows = d.changed.len();
            tombstones = d.tombstones.len();
            encode_delta(schema, std::slice::from_ref(prev), &[Some(d)], wire)
                .expect("delta encodes");
            // delta bytes roll up via the returned outcome
        }
    }

    // Reader replay: decode what was just written against the reader's OWN
    // reconstructed prior and require stored-equality with the fold's
    // accumulator. An hour that emitted nothing is still checked — the
    // reader's retained state must already equal the fold's, which is exactly
    // what "no field changed this hour" asserts, and for a merge accumulator
    // that assertion IS the deliverable.
    if emitted {
        let hour_bytes = &wire[hour_start..];
        if block_start || reader.is_none() {
            let (states, consumed) =
                decode_baseline(schema, hour_bytes, 0).expect("baseline decodes");
            assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
            *reader = Some(states.into_iter().next().unwrap());
        } else {
            let reader_prior = reader.take().unwrap();
            let (deltas, consumed) =
                decode_delta(schema, std::slice::from_ref(&reader_prior), hour_bytes, 0)
                    .expect("delta decodes");
            assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
            let mut next = reader_prior;
            apply(&mut next, deltas[0].as_ref().unwrap());
            *reader = Some(next);
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


    let opened_baseline = block_start || prior.is_none();
    WireHourOutcome {
        opened_baseline,
        gap_forced,
        emitted,
        bytes: wire.len() - hour_start,
        changed_rows,
        tombstones,
    }
}

fn main() {
    const USAGE: &str =
        "usage: bench_real_d68 <collector_stream.bin> <out dir> [legacy|identity-v1]";
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d68::schema();
    let collection = &schema.collections[0];

    // ---- Pass 1: validate the WHOLE stream and fold its d68 DATA records. ----
    let scan = match scan_stream(&stream, collection, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: encode both arms. ------------------------------------------
    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut ship_cut = Vec::new();
    let mut ship_cut_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut prior: Option<&KeyedState> = None;
    let mut reader: Option<KeyedState> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut gap_forced_baselines = 0usize;
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    let mut empty_hours = 0usize;
    let mut changed_rows_total = 0usize;
    let mut tombstones_total = 0usize;
    // Cadence-independent churn: changed rows over MEASURED-hour transitions,
    // taken before the block decision, so a change landing on a block boundary
    // still counts.
    let mut cadence_free_changed = 0usize;
    let mut cadence_free_transitions = 0usize;
    let mut framing_bytes = 0usize;
    let mut payload_header_bytes = 0usize;
    let mut scalar_bytes = 0usize;
    let mut string_bytes = 0usize;

    for hour in &scan.hours {
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());
        // The cut arm: the same record with its string suffix removed. The
        // `payload_len` header word is left as written, so this is a
        // size-accounting artifact and not a valid compact record.
        let cut_end = hour.ship_span.end - hour.string_bytes;
        ship_cut.extend_from_slice(&stream[hour.ship_span.start..cut_end]);
        ship_cut_offsets.push(ship_cut.len());

        framing_bytes += hour.framing_bytes;
        payload_header_bytes += hour.payload_header_bytes;
        scalar_bytes += hour.scalar_bytes;
        string_bytes += hour.string_bytes;

        let state: &KeyedState = &hour.state;

        if let Some(prev) = prior {
            let d = diff(collection, prev, state);
            cadence_free_changed += d.changed.len() + d.tombstones.len();
            cadence_free_transitions += 1;
        }

        // The wire-arm hour — baseline decision, encode and reader replay —
        // through the SAME function the tests execute (`encode_wire_hour`).
        let outcome = encode_wire_hour(
            &schema,
            collection,
            hours_seen,
            hour,
            prior,
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

        prior = Some(state);
        hours_seen += 1;
    }

    for (name, bytes) in [
        ("ship_d68.bin", &ship),
        ("ship_d68_no_strings.bin", &ship_cut),
        ("wire_new.bin", &wire),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d68.offsets", &ship_offsets),
        ("ship_d68_no_strings.offsets", &ship_cut_offsets),
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
        "failure-status hours {} (excluded from both arms; the fold retains the prior header)",
        scan.failure_records
    );
    print!("presence words       ");
    for (presence, count) in &scan.presence_histogram {
        print!("{presence:#04x}x{count} ");
    }
    println!();
    println!(
        "baselines            {baselines} ({} at a {BLOCK_HOURS}-h block start, \
         {gap_forced_baselines} forced by a WIRE_FORMAT \u{a7}1.5 state gap), \
         {baseline_bytes} raw bytes",
        baselines - gap_forced_baselines
    );
    println!(
        "delta hours          {delta_hours} ({delta_bytes} raw bytes)   empty hours {empty_hours}"
    );
    println!(
        "churn cadence-free   {:.4} changed rows per measured-hour transition ({} over {})",
        cadence_free_changed as f64 / cadence_free_transitions.max(1) as f64,
        cadence_free_changed,
        cadence_free_transitions
    );
    println!(
        "churn delta path     {:.4} changed rows/hour, {:.4} tombstones/hour over {delta_hours} emitted-delta hours",
        changed_rows_total as f64 / delta_hours.max(1) as f64,
        tombstones_total as f64 / delta_hours.max(1) as f64
    );
    println!("ship_d68.bin         {} bytes", ship.len());
    println!(
        "  byte split         {framing_bytes} H4CR framing ({:.1}%), {payload_header_bytes} payload header ({:.1}%), \
         {scalar_bytes} scalars CLAIMED ({:.1}%), {string_bytes} strings STOPPED ({:.1}%)",
        100.0 * framing_bytes as f64 / ship.len() as f64,
        100.0 * payload_header_bytes as f64 / ship.len() as f64,
        100.0 * scalar_bytes as f64 / ship.len() as f64,
        100.0 * string_bytes as f64 / ship.len() as f64,
    );
    println!(
        "ship_d68_no_strings  {} bytes (size-accounting artifact: the string suffix cut, payload_len left as written)",
        ship_cut.len()
    );
    println!(
        "wire_new.bin         {} bytes ({:.2}% of ship, {:.2}% of the cut arm)",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64,
        100.0 * wire.len() as f64 / ship_cut.len() as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{COMPACT_DOMAIN_FRAME, COMPACT_FRAME_END_PAYLOAD_LEN};

    fn test_collection() -> wire_format::Collection {
        d68::schema().collections[0].clone()
    }

    fn scan_stream(
        stream: &[u8],
        collection: &wire_format::Collection,
    ) -> Result<StreamScan, String> {
        super::scan_stream(stream, collection, StreamIdentityMode::Legacy)
    }

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

    /// A d68 DATA payload, packed as `attach_log.rs:61905`-`:61958` does.
    fn d68_payload(
        status_flags: u32,
        presence: u32,
        scalars: [u32; 5],
        strings: [&str; 3],
    ) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes());
        p.extend_from_slice(&presence.to_le_bytes());
        p.extend_from_slice(&status_flags.to_le_bytes());
        for v in scalars {
            p.extend_from_slice(&v.to_le_bytes());
        }
        for s in strings {
            p.extend_from_slice(&(s.len() as u16).to_le_bytes());
            p.extend_from_slice(s.as_bytes());
        }
        p
    }

    /// A full payload with every presence bit set.
    fn full_payload(status_flags: u32, session: u32) -> Vec<u8> {
        d68_payload(
            status_flags,
            d68::KNOWN_FLAGS,
            [368, 257_535, 60_759_371, 33, session],
            ["fascism", "FRA_VICHY", "Operation Postern v1.19.2.0.a729"],
        )
    }

    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D68,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &full_payload(0, 1),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D68,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &full_payload(0, 2),
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
    fn a_well_formed_stream_yields_its_d68_hours() {
        let scan = scan_stream(&good_stream(), &test_collection()).expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].game_hour, 100);
        assert_eq!(scan.hours[0].state.len(), 1, "one merged row");
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.non_contiguous, 0);
        assert_eq!(scan.hours[0].string_bytes, 9 + 11 + 34);
        assert_eq!(
            scan.hours[0].ship_span.len(),
            COMPACT_RECORD_HEADER_LEN
                + d68::HEADER_LEN
                + d68::SCALAR_BLOCK_LEN
                + scan.hours[0].string_bytes
        );
    }

    #[test]
    fn the_accumulator_carries_across_hours_and_a_clear_bit_changes_nothing() {
        // The merge property, end to end through the harness: hour 2 carries no
        // presence bit at all, so its measured state must equal hour 1's.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D68,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &full_payload(0, 7),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D68,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d68_payload(0, 0, [0; 5], ["", "", ""]),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &test_collection()).expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(
            scan.hours[0].state, scan.hours[1].state,
            "an empty update leaves the merged state untouched"
        );
        assert_eq!(
            diff(
                &test_collection(),
                &scan.hours[0].state,
                &scan.hours[1].state
            )
            .is_empty(),
            true,
            "and therefore emits no delta"
        );
    }

    #[test]
    fn a_first_frame_that_carries_nothing_still_measures_one_row() {
        // `get_or_insert_with` at `world_fold.rs:10143` materialises the carried
        // header BEFORE any presence bit is consulted, so an hour whose payload
        // carries no field at all is still an hour in which the recorder holds —
        // and `build_session_header_value` (`:4084`) attaches — a header. The
        // measured collection therefore has ONE row from the first folded record
        // onwards, whatever that record contained. Deriving membership from the
        // fields instead would have this harness measure an empty collection
        // against a recorder that publishes a row.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D68,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d68_payload(0, 0, [0; 5], ["", "", ""]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D68,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &full_payload(0, 4),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &test_collection()).expect("accepted");
        assert_eq!(
            scan.hours[0].state.len(),
            1,
            "an empty update still materialises the header"
        );
        assert_eq!(scan.hours[1].state.len(), 1);
        // And the two hours differ, so the second hour emits a real delta: the
        // all-Unknown row gains its values.
        assert!(
            !diff(
                &test_collection(),
                &scan.hours[0].state,
                &scan.hours[1].state
            )
            .is_empty(),
            "the first real read is a changed row, not a new member"
        );
    }

    #[test]
    fn an_excluded_hour_forces_the_next_observation_to_be_a_baseline() {
        // `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the state part forces
        // the next state observation to be a `BASELINE`." Both gap shapes d68
        // can meet — an excluded failure-status hour, and a whole hour the
        // capture never scheduled — must clear `follows_previous`, and
        // `opens_baseline` must open a baseline on it wherever it lands in a
        // block. A delta would relate two states across unobserved time.
        //
        // Shape 1: hour 101 carries a failure-status d68 record (excluded).
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D68,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &full_payload(0, 1),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D68,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &full_payload(1 << 0, 999), // failure status -> excluded
        ));
        s.extend(frame_end(4, 101));
        s.extend(record(
            5,
            102,
            D68,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &full_payload(0, 2),
        ));
        s.extend(frame_end(6, 102));
        let scan = scan_stream(&s, &test_collection()).expect("well-formed");
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
        let schema = d68::schema();
        let collection = &schema.collections[0];
        let mut wire = Vec::new();
        let mut reader: Option<KeyedState> = None;
        let o0 = encode_wire_hour(&schema, collection, 0, &scan.hours[0], None, &mut reader, &mut wire);
        assert!(o0.opened_baseline && !o0.gap_forced);
        let o1 = encode_wire_hour(
            &schema,
            collection,
            1,
            &scan.hours[1],
            Some(&scan.hours[0].state),
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
        s.extend(record(
            1,
            100,
            D68,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &full_payload(0, 1),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            102,
            D68,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &full_payload(0, 2),
        ));
        s.extend(frame_end(4, 102));
        let scan = scan_stream(&s, &test_collection()).expect("well-formed");
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
        let scan = scan_stream(&good_stream(), &test_collection()).expect("well-formed");
        assert_eq!(scan.non_contiguous, 0);
        assert!(scan.hours[1].follows_previous);
        assert!(!opens_baseline(1, scan.hours[1].follows_previous));
        let mut wire = Vec::new();
        let mut reader: Option<KeyedState> = None;
        encode_wire_hour(&schema, collection, 0, &scan.hours[0], None, &mut reader, &mut wire);
        let o1 = encode_wire_hour(
            &schema,
            collection,
            1,
            &scan.hours[1],
            Some(&scan.hours[0].state),
            &mut reader,
            &mut wire,
        );
        assert!(
            !o1.opened_baseline,
            "a contiguous non-block hour is not baselined"
        );
    }

    #[test]
    fn a_closed_hour_without_a_d68_record_refuses_the_stream() {
        // The cross-domain bound: this harness validates only d68 payloads, so
        // a d68 record relabelled to another domain — or dropped — would
        // otherwise be silently ignored and the run would report a plausible
        // measurement over fewer hours. Hour 100 carries a d68 record; hour
        // 101 carries only another domain's DATA record. Every hour is
        // structurally well formed — a DOMAIN_DATA record opens it and a
        // FRAME_END closes it — so only the completeness check can see it.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D68,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &full_payload(0, 1),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &[0u8; 8],
        ));
        s.extend(frame_end(4, 101));
        let err = match scan_stream(&s, &test_collection()) {
            Err(err) => err,
            Ok(_) => panic!("a closed hour with no d68 record must refuse the stream"),
        };
        assert!(
            err.contains("every closed hour"),
            "the refusal names the completeness rule: {err}"
        );
    }

    #[test]
    fn a_failure_status_record_is_excluded_and_does_not_advance_the_accumulator() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time. The
        // hour is not d68 data, and — the merge-specific half — the failed
        // record's payload must NOT be merged: hour 102 must still read session 1.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D68,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &full_payload(0, 1),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D68,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &full_payload(1 << bit, 999),
            ));
            s.extend(frame_end(4, 101));
            s.extend(record(
                5,
                102,
                D68,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d68_payload(0, 0, [0; 5], ["", "", ""]),
            ));
            s.extend(frame_end(6, 102));
            let scan = scan_stream(&s, &test_collection()).expect("well-formed");
            assert_eq!(scan.failure_records, 1, "bit {bit}");
            assert_eq!(scan.hours.len(), 2, "bit {bit}");
            assert_eq!(scan.hours[1].game_hour, 102, "bit {bit}");
            assert_eq!(
                scan.hours[0].state, scan.hours[1].state,
                "bit {bit}: the failed record must not have merged"
            );
        }
        // A non-failure informational bit is NOT treated as a failure.
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D68,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &full_payload(informational, 1),
            ));
            s.extend(frame_end(2, 100));
            let scan = scan_stream(&s, &test_collection())
                .unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 1);
            assert_eq!(scan.failure_records, 0);
        }
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        for bit in 15..32u32 {
            let status = 1u32 << bit;
            for extra in [0u32, 1, 1 << 10] {
                let mut s = Vec::new();
                s.extend(record(
                    1,
                    100,
                    D68,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &full_payload(status | extra, 1),
                ));
                s.extend(frame_end(2, 100));
                assert!(
                    scan_stream(&s, &test_collection()).is_err(),
                    "status bit {bit} (+{extra:#x}) must be refused"
                );
            }
        }
    }

    #[test]
    fn a_garbage_presence_word_under_a_failure_status_is_rejected_by_certification() {
        // The fold exits before its presence check, but production's independent
        // decoder runs on every record and rejects this payload
        // (`domain_data_authority.rs:12745`-`:12749`).
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D68,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d68_payload(1, 1 << 31, [0; 5], ["", "", ""]),
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection()).is_err());

        // The same presence word under a SUCCESS status is a stream rejection.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D68,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d68_payload(0, 1 << 31, [0; 5], ["", "", ""]),
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection()).is_err());
    }

    #[test]
    fn a_malformed_d68_payload_is_refused_not_skipped() {
        let mut cases: Vec<Vec<u8>> = Vec::new();
        // an unknown presence bit under a success status
        cases.push(d68_payload(0, 1 << 8, [0; 5], ["", "", ""]));
        // a scalar carrying a value without its flag
        cases.push(d68_payload(0, 0, [0, 1, 0, 0, 0], ["", "", ""]));
        // an out-of-range player id
        cases.push(d68_payload(
            0,
            d68::FLAG_PLAYER,
            [d68::MAX_PLAYER_TAG_DB_ID + 1, 0, 0, 0, 0],
            ["", "", ""],
        ));
        // a flag set over an empty string
        cases.push(d68_payload(0, d68::FLAG_IDEOLOGY, [0; 5], ["", "", ""]));
        // string bytes without the flag
        cases.push(d68_payload(0, 0, [0; 5], ["fascism", "", ""]));
        // trailing bytes
        let mut trailing = full_payload(0, 1);
        trailing.push(0);
        cases.push(trailing);
        // a payload short of the scalar block
        cases.push(full_payload(0, 1)[..d68::HEADER_LEN + 4].to_vec());
        // a payload short of the header
        cases.push(vec![0u8; d68::HEADER_LEN - 1]);
        for (i, payload) in cases.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D68,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            assert!(
                scan_stream(&s, &test_collection()).is_err(),
                "malformed payload {i} must refuse the stream"
            );
        }
    }

    #[test]
    fn a_second_d68_record_in_one_hour_is_rejected() {
        // Two merges against one clock time would advance the accumulator twice.
        // Checked over every success/failure ordering, before status exclusion.
        const FAILED: u32 = 1;
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D68,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &full_payload(first, 1),
            ));
            s.extend(record(
                2,
                100,
                D68,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &full_payload(second, 2),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&s, &test_collection()).is_err(),
                "two d68 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&full[..cut], &test_collection()).is_err(),
                "cut after {records}"
            );
        }
        for (records, expected_hours) in [(2usize, 1usize), (4, 2)] {
            let cut = boundary(&full, records);
            let scan = scan_stream(&full[..cut], &test_collection())
                .expect("a cut at a FRAME_END is a well-formed shorter stream");
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
                scan_stream(&full[..cut], &test_collection()).is_err(),
                "mid-record cut at {cut}"
            );
        }
        assert!(scan_stream(&[], &test_collection()).is_err());
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
                    scan_stream(&s, &test_collection()).is_err(),
                    "record {records} patched at +{offset}"
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
                    scan_stream(&s, &test_collection()).is_err(),
                    "record {records}"
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
            D68,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &full_payload(0, 1),
        ));
        s.extend(frame_end(2, 999));
        assert!(scan_stream(&s, &test_collection()).is_err());

        let mut s = Vec::new();
        s.extend(frame_end(1, 100));
        assert!(scan_stream(&s, &test_collection()).is_err());
    }

    #[test]
    fn non_monotonic_hours_are_rejected() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            101,
            D68,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &full_payload(0, 1),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D68,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &full_payload(0, 2),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&s, &test_collection()).is_err());
    }

    #[test]
    fn a_stream_with_no_d68_records_is_refused_not_reported_as_zero() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            20,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &[0u8; 8],
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection()).is_err());
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        let data = || full_payload(0, 1);
        let mut ok = Vec::new();
        ok.extend(record(
            1,
            100,
            D68,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &data(),
        ));
        ok.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        assert!(scan_stream(&ok, &test_collection()).is_ok());

        let mut bad = vec![
            frame_end_sealed(2, 100, 0, 0, 0, 1),
            frame_end_sealed(2, 100, 0, 1, 0, 0),
        ];
        for bit in 15..32u32 {
            bad.push(frame_end_sealed(2, 100, 0, 0, 1 << bit, 0));
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            bad.push(frame_end_sealed(2, 100, 0, 0, 1 << bit, 0));
        }
        for sealed in bad {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D68,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &data(),
            ));
            s.extend(sealed);
            assert!(scan_stream(&s, &test_collection()).is_err());
        }
        for len in [0usize, 1, 16, 31, 33, 64] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D68,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &data(),
            ));
            s.extend(record(
                2,
                100,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                &vec![0; len],
            ));
            assert!(scan_stream(&s, &test_collection()).is_err(), "{len}");
        }
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D68,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &data(),
            ));
            s.extend(frame_end_sealed(2, 100, 0, 0, informational, 0));
            assert!(scan_stream(&s, &test_collection()).is_ok());
        }
        let mut later = good_stream();
        let at = boundary(&later, 3) + COMPACT_RECORD_HEADER_LEN + 28;
        later[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
        assert!(scan_stream(&later, &test_collection()).is_err());
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D68,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &full_payload(0, 1),
            )
        };
        let mut ok = Vec::new();
        ok.extend(perf(1, 100, 20));
        ok.extend(perf(2, 100, D68));
        ok.extend(perf(3, 100, PROBE_DOMAIN_ID));
        ok.extend(data(4, 100));
        ok.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
        assert!(scan_stream(&ok, &test_collection()).is_ok());

        let mut probe_twice = Vec::new();
        probe_twice.extend(perf(1, 100, PROBE_DOMAIN_ID));
        probe_twice.extend(perf(2, 100, PROBE_DOMAIN_ID));
        probe_twice.extend(data(3, 100));
        probe_twice.extend(frame_end_sealed(4, 100, 0, 0, 0, 0));
        assert!(scan_stream(&probe_twice, &test_collection()).is_ok());

        let mut duplicate = Vec::new();
        duplicate.extend(perf(1, 100, 20));
        duplicate.extend(perf(2, 100, 20));
        duplicate.extend(data(3, 100));
        duplicate.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        assert!(scan_stream(&duplicate, &test_collection()).is_err());

        let mut interleaved = Vec::new();
        interleaved.extend(data(1, 100));
        interleaved.extend(perf(2, 999, 20));
        interleaved.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&interleaved, &test_collection()).is_err());

        let mut stale = Vec::new();
        stale.extend(data(1, 100));
        stale.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        stale.extend(perf(3, 100, 20));
        stale.extend(data(4, 101));
        stale.extend(frame_end_sealed(5, 101, 1, 1, 0, 0));
        assert!(scan_stream(&stale, &test_collection()).is_err());

        let mut conflict = Vec::new();
        conflict.extend(perf(1, 100, 20));
        conflict.extend(perf(2, 101, 21));
        conflict.extend(data(3, 100));
        conflict.extend(frame_end_sealed(4, 100, 2, 2, 0, 0));
        assert!(scan_stream(&conflict, &test_collection()).is_err());

        for (emitted, scheduled) in [(1, 1), (3, 3), (2, 3), (3, 2)] {
            let mut s = Vec::new();
            s.extend(perf(1, 100, 20));
            s.extend(perf(2, 100, D68));
            s.extend(data(3, 100));
            s.extend(frame_end_sealed(4, 100, emitted, scheduled, 0, 0));
            assert!(scan_stream(&s, &test_collection()).is_err());
        }

        let mut two_hours = Vec::new();
        two_hours.extend(perf(1, 100, 20));
        two_hours.extend(data(2, 100));
        two_hours.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        two_hours.extend(perf(4, 101, 20));
        two_hours.extend(data(5, 101));
        two_hours.extend(frame_end_sealed(6, 101, 1, 1, 0, 0));
        assert_eq!(
            scan_stream(&two_hours, &test_collection())
                .expect("set resets")
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
                D68,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &full_payload(0, 1),
            )
        };
        for (perf_hour, data_hour) in [(100, 101), (101, 100), (100, 200), (200, 100)] {
            let mut s = Vec::new();
            s.extend(perf(1, perf_hour, 20));
            s.extend(data(2, data_hour));
            s.extend(frame_end_sealed(3, data_hour, 1, 1, 0, 0));
            assert!(scan_stream(&s, &test_collection()).is_err());
        }
        let mut agreeing = Vec::new();
        agreeing.extend(perf(1, 100, 20));
        agreeing.extend(data(2, 100));
        agreeing.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&agreeing, &test_collection()).is_ok());
    }

    #[test]
    fn an_excluded_record_is_still_checked_by_the_payload_authority() {
        const FAILED: u32 = 1;
        let mut bad = Vec::new();
        bad.push(d68_payload(FAILED, 1 << 31, [0; 5], ["", "", ""]));
        bad.push(full_payload(FAILED, 1)[..d68::STRING_BLOCK_OFFSET - 1].to_vec());
        for i in 0..5 {
            let mut scalars = [0; 5];
            scalars[i] = 1;
            bad.push(d68_payload(FAILED, 0, scalars, ["", "", ""]));
        }
        bad.push(d68_payload(FAILED, d68::FLAG_PLAYER, [0; 5], ["", "", ""]));
        bad.push(d68_payload(
            FAILED,
            d68::FLAG_PLAYER,
            [d68::MAX_PLAYER_TAG_DB_ID + 1, 0, 0, 0, 0],
            ["", "", ""],
        ));
        let empty = d68_payload(FAILED, 0, [0; 5], ["", "", ""]);
        bad.push(empty[..d68::STRING_BLOCK_OFFSET].to_vec());
        let mut overlong = empty.clone();
        overlong[d68::STRING_BLOCK_OFFSET..d68::STRING_BLOCK_OFFSET + 2]
            .copy_from_slice(&((d68::MAX_STRING_LEN + 1) as u16).to_le_bytes());
        bad.push(overlong);
        let mut truncated = empty[..d68::STRING_BLOCK_OFFSET + 2].to_vec();
        truncated[d68::STRING_BLOCK_OFFSET..d68::STRING_BLOCK_OFFSET + 2]
            .copy_from_slice(&1u16.to_le_bytes());
        bad.push(truncated);
        let mut bad_utf8 = d68_payload(FAILED, d68::FLAG_IDEOLOGY, [0; 5], ["x", "", ""]);
        bad_utf8[d68::STRING_BLOCK_OFFSET + 2] = 0xff;
        bad.push(bad_utf8);
        bad.push(d68_payload(FAILED, 0, [0; 5], ["x", "", ""]));
        bad.push(d68_payload(
            FAILED,
            d68::FLAG_IDEOLOGY,
            [0; 5],
            ["", "", ""],
        ));
        let mut trailing = empty;
        trailing.push(0);
        bad.push(trailing);

        // Every bad payload rides in a stream that WOULD otherwise measure: a
        // second, well-formed hour follows it. Without that trailing hour the
        // scan refuses for an unrelated reason — "no usable d68 DATA records" —
        // and the assertion would hold with `check_authority_payload` deleted,
        // which is a test that cannot fail for its own cause.
        for (i, payload) in bad.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D68,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D68,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &full_payload(0, 1),
            ));
            s.extend(frame_end(4, 101));
            assert!(
                scan_stream(&s, &test_collection()).is_err(),
                "excluded malformed payload {i}"
            );
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D68,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d68_payload(1 << bit, 1 << 31, [0; 5], ["", "", ""]),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D68,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &full_payload(0, 1),
            ));
            s.extend(frame_end(4, 101));
            assert!(scan_stream(&s, &test_collection()).is_err(), "bit {bit}");
        }

        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D68,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &full_payload(FAILED, 999),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D68,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &full_payload(0, 1),
        ));
        s.extend(frame_end(4, 101));
        let scan = scan_stream(&s, &test_collection()).expect("well-formed excluded record");
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 1);
        assert_eq!(scan.hours[0].game_hour, 101);
    }

    #[test]
    fn the_identity_protocol_is_stated_not_sniffed() {
        use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;

        let decapitated = good_stream();
        assert!(super::scan_stream(
            &decapitated,
            &test_collection(),
            StreamIdentityMode::IdentityV1
        )
        .is_err());
        assert!(scan_stream(&decapitated, &test_collection()).is_ok());

        let mut identity_stream = record(
            0,
            0,
            0,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            b"CID1-not-a-real-identity-payload",
        );
        identity_stream.extend(good_stream());
        assert!(scan_stream(&identity_stream, &test_collection()).is_err());
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
            let _ = scan_stream(&bytes, &test_collection());
        }
        let good = good_stream();
        for i in 0..good.len() {
            let mut s = good.clone();
            s[i] ^= 0xFF;
            let _ = scan_stream(&s, &test_collection());
        }
    }
}
