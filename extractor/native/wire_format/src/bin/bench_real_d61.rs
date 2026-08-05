//! Measure the wire format against the shipping compact stream on REAL d61
//! `air_loss_stats_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d61-scheduled capture
//! (`runs/batch3/deepwar_a`). The stream's own d61 DATA records are the shipping
//! format's cost for the domain: a 32-byte header plus a fixed 208-byte row per
//! country, full-replace every hour, exactly what the recorder stores today.
//! This tool replays those records into per-hour states — the same fold
//! `world_fold::fold_air_loss_stats` performs — and re-encodes every hour with
//! this crate's grammar: a BASELINE at each 48-hour block start, field-granular
//! stored-minimal DELTAs between.
//!
//! **Nothing here re-derives a contract it can call.** Row and payload decoding
//! go through `wire_format::d61`'s public boundary; H4CR framing, record
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
//! The DATA payloads of every domain other than 61 pass UNVALIDATED beyond
//! header sanity, the stream validator and the hour-boundary rules —
//! production validates every domain's payload (`stream_fold.rs:593`); this
//! harness owns only d61's grammar. That unvalidated remainder is explicitly
//! BOUNDED: every closed game hour must carry exactly one d61 DATA record of
//! any status (the `d61_records == closed_hours` completeness check in
//! [`scan_stream`]), so a d61 record relabelled to another domain id — or
//! dropped — refuses the run instead of silently shrinking it.
//!
//! # The three outcomes, named per fold path
//!
//! `fold_air_loss_stats` (`world_fold.rs:15896`-`:15921`) has four paths that do
//! not fold, and they are not the same kind of thing:
//!
//! | fold path | line | outcome here |
//! |---|---|---|
//! | `decode_payload_rows` returns `Err` (header or row malformed) | `:15897` | **stream rejection** |
//! | `handle_compact_failure` returns true (failure status) | `:15899`-`:15904` | **excluded hour** |
//! | `owner_tag_db_id != country_idx` | `:15908`-`:15912` | **stream rejection** |
//! | duplicate `owner_tag_db_id` | `:15913`-`:15918` | **stream rejection** |
//!
//! The last three are `Err` returns that say the payload is not this grammar, so
//! they invalidate the run rather than skipping an hour. Note that unlike
//! `fold_hub_motorization`, d61's fold calls no `mark_fold_domain_unknown` on any
//! of them — it propagates the `Err` directly.
//!
//! **The exclusion POSITION is derived from this fold's own call order, not
//! copied.** `decode_payload_rows` runs FIRST (`:15897`-`:15898`) and
//! `handle_compact_failure` is only reached if it returned `Ok`. So the codec's
//! header rules ([`d61::split_payload`]) and per-row rules
//! ([`d61::check_codec_row`] — the exact flag set and the 24 channel bands) apply
//! to **every** d61 record including the ones status excludes, while the fold's
//! own rules — the `owner == country_idx` identity that
//! [`d61::record_from_row`] enforces, and the duplicate-key rejection — sit AFTER
//! `handle_compact_failure` and apply only to records that are actually folded.
//! Hoisting either group into the other's position would make this harness
//! disagree with the recorder on exactly the frames that fail.
//!
//! # Other writers of the accumulator
//!
//! There are none, and that is by enumeration rather than by assumption:
//! `self.air_loss_stats` is declared at `world_fold.rs:1898`, initialised empty
//! at `:2476`, read by the JSON emitter at `:9157`, and assigned in exactly one
//! place — `:15918`, this domain's own fold. No other domain's fold, no
//! end-of-hour prune and no deletion row touches it, so there is no writer to
//! model or to assert inert per capture.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d61.bin   — the d61 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface, reported twice: cadence-independent
//! (over measured-hour transitions, taken before the block decision) and along
//! the delta path (over the hours that actually emitted a delta).

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
use wire_format::d61;

const D61: u16 = 61;
const BLOCK_HOURS: usize = 48;

/// One usable d61 hour: its game hour, the folded state of that hour, and its
/// byte span in the source stream (the shipping arm is those bytes verbatim,
/// header included).
///
/// The state is built during the scan, not later: every way a row can be
/// rejected is a property of the stream, so it belongs to the pass that either
/// accepts the stream or refuses it. Deferring it would leave the harness able
/// to accept a stream and then unwind while measuring it.
struct D61Hour {
    game_hour: i32,
    state: KeyedState,
    ship_span: Range<usize>,
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
/// and d61's cadence is hourly, so a measured hour that does not follow the
/// previous measured one by exactly one is preceded by one. Neither a delta
/// nor a retained reader state can be justified across it.
fn opens_baseline(hours_seen: usize, follows_previous: bool) -> bool {
    hours_seen % BLOCK_HOURS == 0 || !follows_previous
}

struct StreamScan {
    hours: Vec<D61Hour>,
    /// d61 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// Measured d61 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
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

/// The per-row pass `decode_payload_rows` performs on every d61 record
/// (`lanes/air_loss_stats_live/src/lib.rs:327`-`:344`), which the fold runs
/// BEFORE it looks at the status word. Header predicates are already enforced by
/// [`d61::split_payload`].
///
/// The recorder's status-independent payload authority for this domain,
/// `decode_compact_air_loss_stats_live_data_payload`
/// (`recorder_host/src/domain_data_authority.rs:8200`-`:8245`), calls the lane's
/// lenient summary decoder `decode_payload` (`lib.rs:218`), which checks strictly
/// less than this: it validates `field_count`, `row_len`, the body multiple, the
/// declared row count and the channel bands, but not `schema_version` and not the
/// row flag set. The fold is the higher authority (recipe step 1) and the fold
/// calls the strict `decode_payload_rows`, so this mirrors the strict one.
fn check_codec_rows(body: &[u8], at_byte: usize) -> Result<(), String> {
    for row in body.chunks_exact(d61::ROW_LEN) {
        d61::check_codec_row(row).map_err(|e| format!("d61 row at byte {at_byte}: {e}"))?;
    }
    Ok(())
}

/// Validate the whole stream against the canonical contract and fold the usable
/// d61 DATA records into per-hour states. Every rejection path is an `Err`,
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

    let mut hours: Vec<D61Hour> = Vec::new();
    let mut failure_records = 0usize;
    let mut non_contiguous = 0usize;
    // COMPLETENESS accounting: every d61 DATA record of ANY status, against
    // every closed game hour — see the check after the EOF rules.
    let mut d61_records = 0usize;
    let mut closed_hours = 0usize;

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d61 DATA record of ANY status — the uniqueness rule.
    let mut last_d61_hour: Option<i32> = None;
    // Hour of the last d61 record that entered the measurement — the
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

        if header.domain_id == D61 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d61_records += 1;
            let (d61_header, body) = d61::split_payload(payload)
                .map_err(|e| format!("d61 payload at byte {pos}: {e}"))?;

            // One d61 DATA record per game hour, checked over EVERY d61 record
            // and BEFORE any status-based exclusion. d61 is full-replace, so a
            // second record for the same hour would become a second measurement
            // hour built from the same clock time — and gating this on the
            // record being usable would let a success/failure pair through the
            // rule entirely.
            if let Some(previous) = last_d61_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d61 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d61_hour = Some(header.game_hour_raw);

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
            // measured as healthy. No capture carries an unassigned bit, so no
            // measured number moves.
            let unknown_status = compact_status_unknown_flags_v1(d61_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d61 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            // **The codec's per-row pass runs BEFORE the status check, on every
            // d61 record, because that is the order the fold runs it in.**
            // `fold_air_loss_stats` calls `decode_payload_rows` first
            // (`world_fold.rs:15897`) and only reaches `handle_compact_failure`
            // (`:15899`) if that decode returned `Ok`. So a row carrying the
            // wrong flag set or an out-of-band channel is an error even inside a
            // FAILURE-status record — the failure status never gets looked at.
            // Deferring this until after the status check would let this harness
            // accept a stream the fold refuses.
            check_codec_rows(body, pos)?;

            // `world_fold::handle_compact_failure` (called at `world_fold.rs:15899`)
            // returns true on a failure status, folding nothing from this record
            // and retaining the prior accumulator. The predicate is
            // `compact_contract`'s own, not a copied bit mask: it already applies
            // d61's lane-local mask (none) and expected partial tolerance (none,
            // d61 is absent from `COMPACT_EXPECTED_PARTIAL_TOLERANCES`), so it
            // cannot drift from the recorder.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D61,
                d61_header.status_flags,
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
                // `fold_air_loss_stats` (`:15906`-`:15918`) and so applies only
                // to a record the fold actually folds.
                //
                // `d61::record_from_row` carries the `owner_tag_db_id ==
                // country_idx` identity (`:15908`-`:15912`) — the invariant that
                // lets `country_idx` stay off the wire — and `key_from_row`
                // carries it too. A duplicate `owner_tag_db_id` is a rejected
                // payload (`:15913`-`:15918`), never a last-writer-wins
                // overwrite. Neither is hoisted above the status check, because
                // in a failure-status record the fold has already returned and
                // never looks.
                let mut rows = 0usize;
                let mut state: KeyedState = BTreeMap::new();
                for row in body.chunks_exact(d61::ROW_LEN) {
                    rows += 1;
                    let key =
                        d61::key_from_row(row).map_err(|e| format!("d61 row at byte {pos}: {e}"))?;
                    let record = d61::record_from_row(collection, row, 0)
                        .map_err(|e| format!("d61 row at byte {pos}: {e}"))?;
                    if state.insert(key, record).is_some() {
                        return Err(format!(
                            "d61 payload at byte {pos}: duplicate country row"
                        ));
                    }
                }
                // `split_payload` checked the declared row count against the
                // payload length; this closes the other side of the triangle
                // against the rows actually walked.
                if rows != d61_header.row_count as usize {
                    return Err(format!("d61 payload at byte {pos}: row count mismatch"));
                }
                hours.push(D61Hour {
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
        return Err("stream carries no usable d61 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d61 DATA
    // record, of any status. This bounds what this harness canNOT validate:
    // production runs `validate_domain_data_payload_at` on EVERY domain's DATA
    // payload (`stream_fold.rs:593`), while this harness owns only d61's
    // grammar, so a d61 record whose header was relabelled to another domain
    // id — or lost outright — would otherwise be silently ignored and the run
    // would report a plausible measurement over fewer hours. A per-capture
    // completeness assertion, NOT a grammar rule: a capture where d61 is not
    // scheduled every hour fails here loudly and a human adjusts, instead of a
    // number quietly shrinking (`bench_real_d65.rs` precedent).
    if d61_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d61_records} d61 DATA \
             record(s); every closed hour of a d61-scheduled capture must carry exactly one, \
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
    hour: &D61Hour,
    prior: Option<&KeyedState>,
    reader: &mut Option<KeyedState>,
    wire: &mut Vec<u8>,
) -> WireHourOutcome {
    let state: &KeyedState = &hour.state;
    let hour_start = wire.len();
    // BASELINE at each block start — and at every §1.5 gap, which forces a
    // fresh baseline and restarts the delta chain (`opens_baseline`).
    let block_start = opens_baseline(hours_seen, hour.follows_previous);
    let gap_forced = block_start && hours_seen % BLOCK_HOURS != 0;
    let mut emitted = true;
    let mut changed_rows = 0usize;
    let mut tombstones = 0usize;
    let opened_baseline = block_start || prior.is_none();
    if opened_baseline {
        encode_baseline(schema, std::slice::from_ref(state), wire).expect("baseline encodes");
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
        }
    }
    let bytes = wire.len() - hour_start;

    // Reader replay: decode the bytes just written against the reader's OWN
    // reconstructed prior, exactly as a conforming reader would, and require
    // the result to be stored-equal to this hour's state. An hour that emitted
    // nothing is still checked: the reader's retained state must already equal
    // the fold's, which is precisely what "nothing changed" asserts.
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
    WireHourOutcome {
        opened_baseline,
        gap_forced,
        emitted,
        bytes,
        changed_rows,
        tombstones,
    }
}

fn main() {
    const USAGE: &str = "usage: bench_real_d61 <collector_stream.bin> <out dir> \
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
    let schema = d61::schema();
    let collection = &schema.collections[0];

    // ---- Pass 1: validate the WHOLE stream and fold its d61 DATA records. ---
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
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut prior: Option<&KeyedState> = None;
    let mut reader: Option<KeyedState> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut gap_forced_baselines = 0usize;
    let mut empty_hours = 0usize;
    let mut rows_total = 0usize;
    // The wire arm's raw split. Reported because the block ratio measures
    // re-baseline POLICY as much as grammar: this arm re-baselines every 48 h
    // while a shipping lane baselines once per campaign, so the grammar
    // comparison is only recoverable if the two paths are reported apart.
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;

    // Churn, reported twice. CADENCE-INDEPENDENT: changed rows over measured-hour
    // transitions, taken before the block decision, so a change landing on a
    // block boundary still counts. DELTA-PATH: changed rows and tombstones over
    // the hours that actually emitted a delta. One number cannot answer both
    // questions, and dividing by the wrong denominator misreports the domain by
    // orders of magnitude.
    let mut transitions = 0usize;
    let mut transition_changed_rows = 0usize;
    let mut transition_tombstones = 0usize;
    let mut transition_inserts = 0usize;
    let mut delta_changed_rows = 0usize;
    let mut delta_tombstones = 0usize;

    // Monotonicity is a §4 HINT for this domain and the crack reads a yearly
    // cumulative window, so a channel may legitimately fall. Nothing refuses a
    // decrease; it is measured and reported.
    let mut decreasing_cells = 0usize;
    let mut decreasing_rows = 0usize;
    // Per channel, so the hint can be judged field by field rather than in
    // aggregate: `Monotone` is only right for a counter that USUALLY rises.
    let mut decreasing_by_channel = vec![0usize; d61::CHANNEL_COUNT];
    let mut changed_by_channel = vec![0usize; d61::CHANNEL_COUNT];
    let mut max_by_channel = vec![0u64; d61::CHANNEL_COUNT];

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());

        // The hour's state, folded during the scan: d61 is full-replace, so the
        // hour's rows ARE the hour's whole state.
        let state: &KeyedState = &hour.state;
        rows_total += state.len();

        // Per-channel value coverage: a channel that is zero in every row of
        // every hour had no non-zero value encoded anywhere in this capture, so
        // its value path is untested by the measurement whatever the totals say
        // (the d83 caveat). Tracked as the largest raw word seen.
        for record in state.values() {
            for (i, value) in record.values.iter().enumerate() {
                if let wire_format::value::Value::Unsigned(raw) = value {
                    if *raw > max_by_channel[i] {
                        max_by_channel[i] = *raw;
                    }
                }
            }
        }

        // Cadence-independent churn, and the monotone-hint measurement, both
        // taken here — before the block decision below can turn this hour into
        // a baseline and hide the transition.
        if let Some(prev) = prior {
            let d = diff(collection, prev, state);
            transitions += 1;
            transition_changed_rows += d.changed.len();
            transition_tombstones += d.tombstones.len();
            transition_inserts += d
                .changed
                .iter()
                .filter(|row| !prev.contains_key(&row.key))
                .count();
            for (key, record) in state {
                let Some(before) = prev.get(key) else { continue };
                let mut fell = false;
                for (i, value) in record.values.iter().enumerate() {
                    if let (
                        Some(wire_format::value::Value::Unsigned(now)),
                        wire_format::value::Value::Unsigned(then),
                    ) = (Some(value), &before.values[i])
                    {
                        if now != then {
                            changed_by_channel[i] += 1;
                        }
                        if now < then {
                            decreasing_cells += 1;
                            decreasing_by_channel[i] += 1;
                            fell = true;
                        }
                    }
                }
                if fell {
                    decreasing_rows += 1;
                }
            }
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
            delta_changed_rows += outcome.changed_rows;
            delta_tombstones += outcome.tombstones;
            delta_bytes += outcome.bytes;
        } else {
            empty_hours += 1;
        }
        wire_offsets.push(wire.len());

        prior = Some(state);
        hours_seen += 1;
    }

    for (name, bytes) in [("ship_d61.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d61.offsets", &ship_offsets),
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
        "country rows avg     {:.1}",
        rows_total as f64 / hours_seen as f64
    );
    println!(
        "baselines            {baselines} ({} at a {BLOCK_HOURS}-h block start, \
         {gap_forced_baselines} forced by a WIRE_FORMAT \u{a7}1.5 state gap)",
        baselines - gap_forced_baselines
    );
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "churn cadence-indep  {:.2} changed rows/hour, {:.3} tombstones/hour, \
         {:.3} inserts/hour over {transitions} measured-hour transitions",
        transition_changed_rows as f64 / transitions.max(1) as f64,
        transition_tombstones as f64 / transitions.max(1) as f64,
        transition_inserts as f64 / transitions.max(1) as f64
    );
    println!(
        "churn delta-path     {:.2} changed rows/hour, {:.3} tombstones/hour over \
         {delta_hours} emitted-delta hours",
        delta_changed_rows as f64 / delta_hours.max(1) as f64,
        delta_tombstones as f64 / delta_hours.max(1) as f64
    );
    println!(
        "monotone-hint falls  {decreasing_cells} channel cells in {decreasing_rows} row \
         transitions (a yearly window may legitimately reset; never refused)"
    );
    // Per channel: `Monotone` is a §4 hint that the value USUALLY rises, so a
    // channel whose changes are mostly falls is carrying the wrong hint. The
    // record's value fields are in `CHANNELS` order (the schema pushes the key
    // first, then the channels in that order), so index i names channel i.
    for (k, spec) in d61::CHANNELS.iter().enumerate() {
        let falls = if changed_by_channel[k] == 0 {
            "never changed".to_string()
        } else {
            format!(
                "{:>4} changes, {:>4} falls ({:.0}%)",
                changed_by_channel[k],
                decreasing_by_channel[k],
                100.0 * decreasing_by_channel[k] as f64 / changed_by_channel[k] as f64
            )
        };
        let coverage = if max_by_channel[k] == 0 {
            "  ZERO IN EVERY ROW OF EVERY HOUR (no non-zero value encoded)"
        } else {
            ""
        };
        println!(
            "  channel {k:>2} {:<24} max raw {:>12}  {falls}{coverage}",
            spec.label, max_by_channel[k]
        );
    }
    println!(
        "wire raw split       {baseline_bytes} B in {baselines} baselines ({:.1} B each), \
         {delta_bytes} B in {delta_hours} deltas ({:.1} B each)",
        baseline_bytes as f64 / baselines.max(1) as f64,
        delta_bytes as f64 / delta_hours.max(1) as f64
    );
    println!("ship_d61.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.1}%)",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::COMPACT_DOMAIN_FRAME;

    fn test_collection() -> wire_format::Collection {
        d61::schema().collections[0].clone()
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

    /// One 208-byte d61 row, in the lane's row order.
    fn row(owner: u32, country_idx: u32, flags: u32, first_channel: i64) -> Vec<u8> {
        let mut out = Vec::with_capacity(d61::ROW_LEN);
        out.extend_from_slice(&owner.to_le_bytes());
        out.extend_from_slice(&country_idx.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // pad
        for k in 0..d61::CHANNEL_COUNT {
            let raw = first_channel + k as i64;
            out.extend_from_slice(&raw.to_le_bytes());
        }
        assert_eq!(out.len(), d61::ROW_LEN);
        out
    }

    fn good_row(owner: u32, first_channel: i64) -> Vec<u8> {
        row(owner, owner, d61::REQUIRED_FLAGS, first_channel)
    }

    /// A d61 DATA payload: the 32-byte header then the rows.
    fn d61_payload(status_flags: u32, rows: &[Vec<u8>]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        payload.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&d61::SCHEMA_VERSION.to_le_bytes());
        payload.extend_from_slice(&d61::EXPECTED_FIELD_COUNT.to_le_bytes());
        payload.extend_from_slice(&(d61::ROW_LEN as u32).to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes()); // reserved
        assert_eq!(payload.len(), d61::HEADER_LEN);
        for r in rows {
            payload.extend_from_slice(r);
        }
        payload
    }

    fn one_row_payload(status_flags: u32, first_channel: i64) -> Vec<u8> {
        d61_payload(status_flags, &[good_row(16, first_channel)])
    }

    /// Two closed hours, each carrying one d61 DATA record.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D61,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 1_000),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D61,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 2_000),
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
            D61,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 1_000),
        )
    }

    #[test]
    fn a_well_formed_stream_yields_its_d61_hours() {
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
        // The key is the fold's `owner_tag_db_id`.
        assert_eq!(
            scan.hours[0].state.keys().next().unwrap(),
            &wire_format::body::key(&[16])
        );
    }

    #[test]
    fn the_codec_row_pass_runs_on_excluded_records_but_the_fold_rules_do_not() {
        // THE ordering test, and it is specific to this fold. `decode_payload_rows`
        // runs BEFORE `handle_compact_failure` (`world_fold.rs:15897` vs `:15899`),
        // so a bad flag set or an out-of-band channel refuses the stream even in a
        // failure-status record. The `owner == country_idx` identity and the
        // duplicate rule run AFTER it (`:15908`, `:15913`), so a failure-status
        // record may violate BOTH and must still be a quiet exclusion — refusing
        // it would reject a stream the recorder accepts and folds nothing from.
        let collection = test_collection();
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE

        // Codec-level rejections, inside a FAILURE-status record: refused.
        let codec_bad: Vec<Vec<u8>> = vec![
            d61_payload(FAILED, &[row(16, 16, 0, 1_000)]), // no witness bits
            d61_payload(FAILED, &[row(16, 16, d61::ROW_FLAG_BINDING_VERIFIED, 1_000)]),
            d61_payload(FAILED, &[row(16, 16, d61::REQUIRED_FLAGS | 4, 1_000)]), // unknown bit
            d61_payload(FAILED, &[row(16, 16, d61::REQUIRED_FLAGS, -1)]),        // negative
            d61_payload(
                FAILED,
                &[row(16, 16, d61::REQUIRED_FLAGS, d61::MAX_CHANNEL_RAW)],
            ), // last channel overshoots the band
        ];
        for payload in codec_bad {
            let mut s = record(1, 100, D61, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload);
            s.extend(frame_end(2, 100));
            s.extend(domain_data(3, 101));
            s.extend(frame_end(4, 101));
            assert!(
                scan_stream(&s, &collection, StreamIdentityMode::Legacy).is_err(),
                "a codec-level row defect must refuse the stream even under a failure status"
            );
        }

        // Fold-level rules, inside a FAILURE-status record: excluded, not refused.
        let fold_only: Vec<Vec<u8>> = vec![
            // owner != country_idx
            d61_payload(FAILED, &[row(16, 17, d61::REQUIRED_FLAGS, 1_000)]),
            // duplicate country
            d61_payload(
                FAILED,
                &[good_row(16, 1_000), good_row(16, 2_000)],
            ),
        ];
        for payload in fold_only {
            let mut s = record(1, 100, D61, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload);
            s.extend(frame_end(2, 100));
            s.extend(domain_data(3, 101));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &collection, StreamIdentityMode::Legacy).expect(
                "a fold-level rule sits after the status gate, so the record is merely excluded",
            );
            assert_eq!(scan.failure_records, 1);
            assert_eq!(scan.hours.len(), 1);
            assert_eq!(scan.hours[0].game_hour, 101);
        }

        // And the same fold-level defects in a SUCCESS record refuse the stream.
        for payload in [
            d61_payload(0, &[row(16, 17, d61::REQUIRED_FLAGS, 1_000)]),
            d61_payload(0, &[good_row(16, 1_000), good_row(16, 2_000)]),
        ] {
            let mut s = record(1, 100, D61, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload);
            s.extend(frame_end(2, 100));
            assert!(scan_stream(&s, &collection, StreamIdentityMode::Legacy).is_err());
        }
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
            for extra in [0u32, 1, 1 << 10] {
                for rows in [Vec::new(), vec![good_row(16, 1_000)]] {
                    let mut s = Vec::new();
                    s.extend(record(
                        1,
                        100,
                        D61,
                        COMPACT_RECORD_TYPE_DOMAIN_DATA,
                        &d61_payload(status | extra, &rows),
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
        // `fold_air_loss_stats` never reads an informational bit at all.
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D61,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(informational, 1_000),
            ));
            s.extend(frame_end(2, 100));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 1, "bit {informational:#x}");
            assert_eq!(scan.failure_records, 0, "bit {informational:#x}");
        }
    }

    #[test]
    fn an_excluded_hour_forces_the_next_observation_to_be_a_baseline() {
        // `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the state part forces
        // the next state observation to be a `BASELINE`." Both gap shapes d61
        // can meet — an excluded failure-status hour, and a whole hour the
        // capture never scheduled — must clear `follows_previous`, and
        // `opens_baseline` must open a baseline on it wherever it lands in a
        // block. A delta would relate two states across unobserved time.
        //
        // Shape 1: hour 101 carries a failure-status d61 record (excluded).
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D61,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 10),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D61,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(1 << 0, 11), // failure status -> excluded
        ));
        s.extend(frame_end(4, 101));
        s.extend(record(
            5,
            102,
            D61,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12),
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
        // though it is only the second measured hour of the block — through the
        // PRODUCTION encode path, not just the decision function.
        assert!(opens_baseline(1, scan.hours[1].follows_previous));
        let schema = d61::schema();
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
            D61,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 10),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            102,
            D61,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 12),
        ));
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
    fn a_closed_hour_without_a_d61_record_refuses_the_stream() {
        // The cross-domain bound: this harness validates only d61 payloads, so
        // a d61 record relabelled to another domain — or dropped — would
        // otherwise be silently ignored and the run would report a plausible
        // measurement over fewer hours. Hour 100 carries a d61 record; hour
        // 101 carries only another domain's DATA record. Every hour is
        // structurally well formed — a DOMAIN_DATA record opens it and a
        // FRAME_END closes it — so only the completeness check can see it.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D61,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 10),
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
        let err = match scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy) {
            Err(err) => err,
            Ok(_) => panic!("a closed hour with no d61 record must refuse the stream"),
        };
        assert!(
            err.contains("every closed hour"),
            "the refusal names the completeness rule: {err}"
        );
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time: the
        // fold retains the accumulator and folds nothing, so the hour is not d61
        // data and must not enter either arm.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D61,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(1 << bit, 1_000),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D61,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(0, 2_000),
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
            D61,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(1 << 10, 1_000),
        ));
        s.extend(frame_end(2, 100));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn a_second_d61_record_in_one_hour_is_rejected() {
        // d61 is full-replace; two records in one hour would become two
        // measurement hours built from the same clock time. The rule is checked
        // over EVERY d61 DATA record and before status-based exclusion, so all
        // four success/failure orderings are refused.
        const FAILED: u32 = 1;
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D61,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(first, 1_000),
            ));
            s.extend(record(
                2,
                100,
                D61,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_row_payload(second, 2_000),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "two d61 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
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
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        let mut ok = perf(1, 100, 20);
        ok.extend(perf(2, 100, D61));
        ok.extend(perf(3, 100, PROBE_DOMAIN_ID));
        ok.extend(domain_data(4, 100));
        ok.extend(frame_end_sealed(5, 100, 2, 2, 0, 0));
        assert!(scan_stream(&ok, &test_collection(), StreamIdentityMode::Legacy).is_ok());
        let mut duplicate = perf(1, 100, 20);
        duplicate.extend(perf(2, 100, 20));
        duplicate.extend(domain_data(3, 100));
        duplicate.extend(frame_end_sealed(4, 100, 1, 1, 0, 0));
        assert!(scan_stream(&duplicate, &test_collection(), StreamIdentityMode::Legacy).is_err());
        let mut interleaved = domain_data(1, 100);
        interleaved.extend(perf(2, 999, 20));
        interleaved.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&interleaved, &test_collection(), StreamIdentityMode::Legacy).is_err());
        let mut conflict = perf(1, 100, 20);
        conflict.extend(perf(2, 101, 21));
        conflict.extend(domain_data(3, 100));
        conflict.extend(frame_end_sealed(4, 100, 2, 2, 0, 0));
        assert!(scan_stream(&conflict, &test_collection(), StreamIdentityMode::Legacy).is_err());
        for (emitted, scheduled) in [(1, 1), (3, 3), (2, 3), (3, 2)] {
            let mut s = perf(1, 100, 20);
            s.extend(perf(2, 100, D61));
            s.extend(domain_data(3, 100));
            s.extend(frame_end_sealed(4, 100, emitted, scheduled, 0, 0));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }
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
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        // A cut immediately after a d61 DATA record leaves that hour open and
        // ends the stream on a record that is not a FRAME_END.
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&full[..cut], &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary IS a well-formed shorter stream,
        // and `eof_check` accepts it. What matters is that it reports the hours
        // it actually closed, not the hours the untruncated file had.
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
            D61,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 1_000),
        ));
        s.extend(frame_end(2, 999)); // closes an hour that is not open
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
            D61,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 1_000),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D61,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_row_payload(0, 2_000),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_stream_with_no_d61_records_is_refused_not_reported_as_zero() {
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
    fn a_malformed_d61_header_refuses_the_stream_whatever_its_status() {
        // `decode_payload_rows` validates the header before the fold reads the
        // status word, so these refuse the run rather than skipping an hour.
        const FAILED: u32 = 1;
        for status in [0u32, FAILED] {
            let good = one_row_payload(status, 1_000);
            for (offset, patch) in [
                (16usize, 3u32),                       // schema_version
                (20, 88),                              // field_count
                (24, 207),                             // row_len
                (8, 2),                                // row_count vs payload length
            ] {
                let mut payload = good.clone();
                payload[offset..offset + 4].copy_from_slice(&patch.to_le_bytes());
                let mut s = record(1, 100, D61, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload);
                s.extend(frame_end(2, 100));
                assert!(
                    scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                    "status {status:#x}, header word at +{offset} = {patch}"
                );
            }
            // A short payload, and one that is not a whole number of rows.
            for len in [0usize, 8, 31, d61::HEADER_LEN + 1, d61::HEADER_LEN + 207] {
                let mut s = record(
                    1,
                    100,
                    D61,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &good[..len.min(good.len())],
                );
                s.extend(frame_end(2, 100));
                assert!(
                    scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                    "status {status:#x}, truncated payload of {len}"
                );
            }
        }
        // The reserved header word is reported and never enforced — the fold
        // never reads it, so a non-zero value must not refuse the stream.
        let mut payload = one_row_payload(0, 1_000);
        payload[28..32].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        let mut s = record(1, 100, D61, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload);
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_ok());
    }

    #[test]
    fn an_empty_d61_table_is_a_measured_hour_not_a_refusal() {
        // The fold accepts a zero-row payload and replaces the accumulator with
        // an empty map; that is a measured hour in which no country carries an
        // air stat, not a malformed record.
        let mut s = record(
            1,
            100,
            D61,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d61_payload(0, &[]),
        );
        s.extend(frame_end(2, 100));
        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("well-formed");
        assert_eq!(scan.hours.len(), 1);
        assert!(scan.hours[0].state.is_empty());
    }
}
