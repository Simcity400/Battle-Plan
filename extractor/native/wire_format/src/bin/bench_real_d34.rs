//! Measure the wire format against the shipping compact stream on REAL d34
//! `air_base_status` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d34-scheduled capture
//! (`runs/batch2/deepwar_t1`). The stream's own d34 DATA records are the shipping
//! format's cost for the domain: a 36-byte header plus a fixed 32-byte row per
//! CHANGED air base, exactly what the recorder stores today. This tool replays
//! those records through `wire_format::d34::Accumulator` — the same fold
//! `world_fold::fold_air_base_status` performs — and re-encodes every hour's
//! accumulated state with this crate's grammar: a BASELINE at each 48-hour block
//! start, field-granular stored-minimal DELTAs between.
//!
//! **d34 is not full-replace, and that shapes the comparison.** The fold calls
//! `replace_or_sparse_after_first` (`world_fold.rs:15095`): the first record
//! clears and carries every air base, and every later record is a sparse patch
//! with tombstones. The shipping arm is therefore already an incremental
//! encoding, while this arm re-baselines every 48 hours. Report the wire arm's
//! baseline/delta split — without it the block-shape ratio measures re-baseline
//! policy rather than grammar (recipe §9).
//!
//! **Nothing here re-derives a contract it can call.** Header, row and
//! accumulator semantics go through `wire_format::d34`'s public boundary; H4CR
//! framing, record semantics and sequence continuity go through
//! `compact_contract`'s own parser and stateful validator. A harness that
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
//!   (`recorder_host/src/stream_fold.rs:277`): a DOMAIN_DATA record opens its
//!   game hour, a FRAME_END closes the open hour and only that hour, and closed
//!   hours strictly increase;
//! - that tracker's DOMAIN_PERF hour binding (`:328`-`:365`), including the DATA
//!   arm's pending-PERF rule (`:378`-`:386`);
//! - FRAME_END seal-field validation (`:46`-`:89`) and the seal's cross-check
//!   against distinct non-probe PERF domains (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority. The remaining crate-boundary limitation is malformed payloads
//! for domains other than d34: this crate cannot depend on `recorder_host`, so it
//! can replay only d34's payload authority.
//!
//! For d34, production's status-independent authority is
//! `decode_compact_air_base_status_data_payload`
//! (`domain_data_authority.rs:15481`-`:15658`). The pre-exclusion
//! `d34::peek_header` already checks the minimum length and reserved header word.
//! [`check_authority_payload`] adds every remaining predicate before exclusion:
//! header count bounds and exact length; unique, bounded nonzero air-base and
//! state ids; the domain module's own flag/province checks; zero-packed deleted
//! rows; live counter bounds; and the building-level/capacity relationship.
//!
//! These five checks were added together by the five-check sweep. The measured
//! capture is legacy-framed and recorder-valid, so stating legacy mode and
//! replaying these checks cannot change any emitted measurement byte.
//!
//! **The two paths on which d34's fold returns without folding** are excluded
//! from the measurement, in the fold's own order (`world_fold.rs:15066`-`:15094`):
//!
//! 1. `handle_compact_failure` (`:15077`) — a failure status. The predicate is
//!    `compact_contract`'s own, never a copied mask literal, so it cannot drift.
//! 2. `reject_delta_before_baseline` (`:15080`) — a DELTA record before any
//!    replacing record. Every domain whose fold implements a baseline-plus-delta
//!    grammar has this path, and d34's does.
//!
//! Both come AFTER the header's `need`/`reserved` checks and BEFORE the
//! exact-length check, which is why `d34::peek_header` exists as a separate
//! entry point. Hour uniqueness is enforced over ALL d34 DATA records — including
//! failure-status ones — BEFORE either exclusion, so a success + failure pair in
//! one hour cannot slip through.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d34.bin   — the d34 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface (changed rows per hour at stored
//! precision) and the wire arm's baseline/delta raw split.
//!
//! **Only d34 DATA payloads are validated.** Production runs
//! `validate_domain_data_payload_at` on the DATA payload of EVERY domain
//! (`stream_fold.rs:593`); this harness cannot, because validating another
//! domain's payload means owning that domain's authority, and `wire_format`
//! must not depend on `recorder_host`. Every DATA record of any other domain
//! therefore passes with its H4CR header, sequence continuity, hour-boundary,
//! PERF-binding and FRAME_END rules checked but its **payload bytes unread**.
//! The concrete hazard — a d34 record relabelled to another domain id, or
//! dropped outright, silently shrinking the measurement — is closed by an
//! explicit completeness check in `scan_stream`: every closed game hour must
//! carry exactly one d34 DATA record, of any status. That is a per-capture
//! assertion, not a grammar rule; a capture where d34 is not scheduled every
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
use wire_format::d34::{self, Accumulator, FoldOutcome};

const D34: u16 = 34;
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

/// One usable d34 hour: its game hour, the ACCUMULATED state after folding that
/// hour's record, and the record's byte span in the source stream (the shipping
/// arm is those bytes verbatim, header included).
///
/// The state is built during the scan, not later: every way a row can be
/// rejected — an illegal flag bit, an out-of-range province — is a property of
/// the stream, so it belongs to the pass that either accepts the stream or
/// refuses it. Deferring it would leave the harness able to accept a stream and
/// then unwind while measuring it.
///
/// d34's accumulator is small (374 rows on the reference capture, constant), so
/// a per-hour snapshot is affordable and the two-pass harness the large-state
/// domains need is not required here.
struct D34Hour {
    game_hour: i32,
    state: KeyedState,
    ship_span: Range<usize>,
    /// Rows this record carried, and how many of them were tombstones — the
    /// shipping layer's own churn, distinct from the wire arm's stored churn.
    shipped_rows: usize,
    shipped_tombstones: usize,
    /// `Replace` for the first folded record, `Sparse` for the rest.
    outcome: FoldOutcome,
}

struct StreamScan {
    hours: Vec<D34Hour>,
    /// d34 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// d34 records the fold would reject as a delta before any baseline.
    delta_before_baseline_records: usize,
    /// Measured d34 hours that do not follow the previous measured one by one.
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

/// Replay d34's status-independent production payload authority
/// (`domain_data_authority.rs:15481`-`:15658`).
fn check_authority_payload(
    payload: &[u8],
    status_flags: u32,
    at_byte: usize,
) -> Result<(), String> {
    const MAX_AIR_BASE_ID: u32 = i32::MAX as u32;
    const MAX_STATE_ID: u32 = 65_535;
    const MAX_CAPACITY: u32 = 1_000_000;
    const MAX_BUILDING_LEVEL: u32 = 1_000;
    const MAX_WING_COUNT: u32 = 20_000;
    const MAX_PLANES: u32 = 10_000_000;

    let (header, rows) =
        d34::split_payload(payload).map_err(|e| format!("d34 payload at byte {at_byte}: {e}"))?;
    if header.changed_rows > header.row_count {
        return Err(format!(
            "d34 payload at byte {at_byte}: changed_rows > row_count"
        ));
    }
    if status_flags & COMPACT_STATUS_DELTA_DATA == 0
        && header.row_count > header.observed_air_base_rows
    {
        return Err(format!(
            "d34 payload at byte {at_byte}: row_count > observed_air_base_rows"
        ));
    }
    if header.deleted_rows > header.row_count {
        return Err(format!(
            "d34 payload at byte {at_byte}: deleted_rows > row_count"
        ));
    }

    let mut seen = BTreeSet::new();
    for (index, row) in rows.chunks_exact(d34::ROW_LEN).enumerate() {
        let parsed =
            d34::parse_row(row).map_err(|e| format!("d34 row {index} at byte {at_byte}: {e}"))?;
        if parsed.air_base_id == 0 || parsed.air_base_id > MAX_AIR_BASE_ID {
            return Err(format!(
                "d34 row {index} at byte {at_byte}: invalid air_base_id {}",
                parsed.air_base_id
            ));
        }
        if !seen.insert(parsed.air_base_id) {
            return Err(format!(
                "d34 row {index} at byte {at_byte}: duplicate air_base_id {}",
                parsed.air_base_id
            ));
        }
        if parsed.state_id == 0 || parsed.state_id > MAX_STATE_ID {
            return Err(format!(
                "d34 row {index} at byte {at_byte}: invalid state_id {}",
                parsed.state_id
            ));
        }
        if parsed.deleted {
            if parsed.capacity != 0
                || parsed.building_level != 0
                || parsed.based_wing_count != 0
                || parsed.based_plane_count != 0
            {
                return Err(format!(
                    "d34 deleted row {index} at byte {at_byte} carries nonzero live fields"
                ));
            }
            continue;
        }
        if parsed.capacity > MAX_CAPACITY
            || parsed.building_level > MAX_BUILDING_LEVEL
            || parsed.based_wing_count > MAX_WING_COUNT
            || parsed.based_plane_count > MAX_PLANES
        {
            return Err(format!(
                "d34 row {index} at byte {at_byte} carries an out-of-range live counter"
            ));
        }
        if parsed.building_level == 0 && parsed.capacity != 0 {
            return Err(format!(
                "d34 row {index} at byte {at_byte}: capacity with building_level 0"
            ));
        }
    }
    Ok(())
}

/// Validate the whole stream against the canonical contract and fold the usable
/// d34 DATA records into per-hour accumulated states. Every rejection path is an
/// `Err`, never a silently shorter measurement and never a panic — see the module
/// docs for what is enforced and what the recorder's own validator additionally
/// checks.
fn scan_stream(
    stream: &[u8],
    collection: &wire_format::Collection,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours: Vec<D34Hour> = Vec::new();
    let mut failure_records = 0usize;
    let mut delta_before_baseline_records = 0usize;
    let mut non_contiguous = 0usize;
    let mut accumulator = Accumulator::new();

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d34 DATA record of ANY status — the uniqueness rule.
    let mut last_d34_hour: Option<i32> = None;
    // Hour of the last d34 record that entered the measurement — the contiguity
    // report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;

    // Closed game hours and d34 DATA records of ANY status — the
    // completeness rule at the end of the scan.
    let mut closed_hours = 0usize;
    let mut d34_records = 0usize;
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

        if header.domain_id == D34 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d34_records += 1;
            // The fold's FIRST checks (`world_fold.rs:15066`-`:15076`): header
            // length and the reserved word, before any status interpretation.
            let d34_header =
                d34::peek_header(payload).map_err(|e| format!("d34 payload at byte {pos}: {e}"))?;

            // One d34 DATA record per game hour, checked over EVERY d34 record
            // and BEFORE any exclusion. d34 accumulates, so a second record for
            // the same hour would fold two patches into one measurement hour —
            // and gating this on the record being usable would let a
            // success/failure pair through the rule entirely.
            if let Some(previous) = last_d34_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d34 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d34_hour = Some(header.game_hour_raw);

            // VALIDITY BEFORE CLASSIFICATION. Both exclusions below read the
            // status word to classify the v1 vocabulary; neither is a validity
            // predicate. `compact_contract/src/lib.rs:121`-`:123` says so in as
            // many words: "Callers handling untrusted status words must first
            // reject a non-zero `compact_status_unknown_flags_v1` result;
            // otherwise an unassigned bit has no defined classification." Bits
            // 15..=31 are unassigned (`status_flags_base.rs:21`-`:24`), and an
            // unassigned bit "is not evidence of a future informational status".
            //
            // Without this, a record with `status_flags = 1 << 14` reads as "not
            // a failure" — the failure helper masks with the *known* failure set
            // and returns zero for a bit it does not know — and the hour is
            // measured as healthy. The production lane rejects first
            // (`lanes/division_state_movement_live/domain20_compact.rs:419`);
            // this harness does the same rather than re-deriving a lenient rule.
            // No capture carries an unassigned bit, so no measured number moves.
            let unknown_status = compact_status_unknown_flags_v1(d34_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d34 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            check_authority_payload(payload, d34_header.status_flags, pos)?;

            // Exclusion 1 (`world_fold.rs:15077`, helper at `:2843`): a failure
            // status marks the domain-hour unknown and RETAINS the accumulator,
            // folding nothing from this record. `compact_contract`'s predicate
            // already applies d34's lane-local mask (none) and expected partial
            // tolerance (none — the tolerance list is domains 26, 27, 28, 54).
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D34,
                d34_header.status_flags,
            ) != 0
            {
                failure_records += 1;
                pos = record_end;
                continue;
            }

            // Exclusions 1→2 in the fold's order: `fold_payload` performs the
            // delta-before-baseline decision at the fold's own position
            // (`:15080`, ahead of its `:15087` length check). The independent
            // certification authority above has already validated exact length.
            let is_delta = d34_header.status_flags & COMPACT_STATUS_DELTA_DATA != 0;
            let outcome = accumulator
                .fold_payload(collection, payload, is_delta)
                .map_err(|e| format!("d34 payload at byte {pos}: {e}"))?;
            if outcome == FoldOutcome::DeltaBeforeBaseline {
                delta_before_baseline_records += 1;
                pos = record_end;
                continue;
            }

            if let Some(previous) = last_measured_hour {
                if header.game_hour_raw != previous + 1 {
                    non_contiguous += 1;
                }
            }
            last_measured_hour = Some(header.game_hour_raw);

            // Shipping-layer churn, read back through the same public boundary.
            // `split_payload` cannot fail here: `fold_payload` just ran it.
            let (_, rows) = d34::split_payload(payload)
                .map_err(|e| format!("d34 payload at byte {pos}: {e}"))?;
            let mut shipped_rows = 0usize;
            let mut shipped_tombstones = 0usize;
            for row in rows.chunks_exact(d34::ROW_LEN) {
                shipped_rows += 1;
                if d34::parse_row(row)
                    .map_err(|e| format!("d34 row at byte {pos}: {e}"))?
                    .deleted
                {
                    shipped_tombstones += 1;
                }
            }

            hours.push(D34Hour {
                game_hour: header.game_hour_raw,
                state: accumulator.bases.clone(),
                ship_span: pos..record_end,
                shipped_rows,
                shipped_tombstones,
                outcome,
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
        return Err("stream carries no usable d34 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d34 DATA
    // record, of any status. Production validates the payload of EVERY domain's
    // DATA record (`stream_fold.rs:593`); this harness cannot (module docs), so
    // a d34 record relabelled to another domain id — or lost outright — would
    // otherwise be silently ignored and the run would report a plausible
    // measurement over fewer hours. Per-hour uniqueness is already enforced
    // above, so equality of the two counts is exactly-one per closed hour. A
    // per-capture assertion, not a grammar rule: a capture that does not
    // schedule d34 every hour fails here loudly and a human adjusts, instead
    // of a number quietly shrinking.
    if d34_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d34_records} d34 DATA \
             record(s); every closed hour of a d34-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    Ok(StreamScan {
        hours,
        failure_records,
        delta_before_baseline_records,
        non_contiguous,
    })
}

fn main() {
    const USAGE: &str =
        "usage: bench_real_d34 <collector_stream.bin> <out dir> [legacy|identity-v1]";
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d34::schema();
    let collection = &schema.collections[0];

    // ---- Pass 1: validate the WHOLE stream and fold its d34 DATA records. ---
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
    // Baselines the §1.5 gap rule forced outside a scheduled block start.
    let mut gap_forced_baselines = 0usize;
    let mut previous_measured_hour: Option<i32> = None;
    let mut baseline_bytes = 0usize;
    let mut delta_bytes = 0usize;
    let mut empty_hours = 0usize;
    let mut changed_rows_total = 0usize;
    let mut tombstones_total = 0usize;
    let mut bases_total = 0usize;
    let mut shipped_rows_total = 0usize;
    let mut shipped_tombstones_total = 0usize;
    let mut shipping_replaces = 0usize;

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());
        shipped_rows_total += hour.shipped_rows;
        shipped_tombstones_total += hour.shipped_tombstones;
        if hour.outcome == FoldOutcome::Replace {
            shipping_replaces += 1;
        }

        // The hour's state is the ACCUMULATOR after folding this hour's record —
        // d34 is sparse-after-first, so the record's rows are only the patch.
        let state: &KeyedState = &hour.state;
        bases_total += state.len();

        // New arm: BASELINE at each block start, stored-minimal DELTA between.
        let hour_start = wire.len();
        let baseline_here = opens_baseline(hours_seen, previous_measured_hour, hour.game_hour);
        if baseline_here && hours_seen % BLOCK_HOURS != 0 {
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
            if d.is_empty() {
                empty_hours += 1;
                emitted = false;
            } else {
                changed_rows_total += d.changed.len();
                tombstones_total += d.tombstones.len();
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
        // equal the fold's, which is precisely what "nothing changed" asserts.
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

    for (name, bytes) in [("ship_d34.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d34.offsets", &ship_offsets),
        ("wire_new.offsets", &wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    let delta_hours = hours_seen - baselines - empty_hours;
    println!(
        "hours                {hours_seen} ({} non-contiguous steps), reader-replay verified {hours_seen}",
        scan.non_contiguous
    );
    println!(
        "failure-status hours {} (excluded from both arms)",
        scan.failure_records
    );
    println!(
        "delta-before-baseline {} (excluded from both arms)",
        scan.delta_before_baseline_records
    );
    println!(
        "shipping layer       {shipping_replaces} replacing record(s), {} sparse; \
         {shipped_rows_total} rows carried ({shipped_tombstones_total} tombstones)",
        hours_seen - shipping_replaces
    );
    println!(
        "air bases avg        {:.1} (accumulated state per hour)",
        bases_total as f64 / hours_seen as f64
    );
    println!(
        "baselines            {baselines} ({} every {BLOCK_HOURS} h, {gap_forced_baselines} \
         forced by a gap in measured hours — WIRE_FORMAT §1.5)",
        baselines - gap_forced_baselines
    );
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "real churn           {:.2} changed rows/hour, {:.3} tombstones/hour (stored precision, \
         over {delta_hours} delta hours)",
        changed_rows_total as f64 / delta_hours.max(1) as f64,
        tombstones_total as f64 / delta_hours.max(1) as f64
    );
    println!("ship_d34.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.1}%)",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64
    );
    println!(
        "wire raw split       {baseline_bytes} B baseline ({:.1}%) + {delta_bytes} B delta ({:.1}%)",
        100.0 * baseline_bytes as f64 / wire.len().max(1) as f64,
        100.0 * delta_bytes as f64 / wire.len().max(1) as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{COMPACT_DOMAIN_FRAME, COMPACT_FRAME_END_PAYLOAD_LEN};

    fn test_collection() -> wire_format::Collection {
        d34::schema().collections[0].clone()
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

    fn raw_row(air_base_id: u32, flags: u32, capacity: u32, province_id: u32) -> Vec<u8> {
        let mut row = Vec::with_capacity(d34::ROW_LEN);
        let building_level = u32::from(flags & d34::ROW_FLAG_DELETED == 0);
        for word in [
            air_base_id,
            1000 + air_base_id,
            flags,
            capacity,
            building_level,
            0,
            0,
            province_id,
        ] {
            row.extend_from_slice(&word.to_le_bytes());
        }
        row
    }

    /// A d34 DATA payload: 36-byte header then the given 32-byte rows.
    fn d34_payload(status_flags: u32, rows: &[Vec<u8>]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // changed_rows
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // observed_air_base_rows
        p.extend_from_slice(&0u32.to_le_bytes()); // observed_pool_rows
        let deleted = rows
            .iter()
            .filter(|row| u32::from_le_bytes(row[8..12].try_into().unwrap()) & 1 != 0)
            .count() as u32;
        p.extend_from_slice(&deleted.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes()); // reserved
        assert_eq!(p.len(), d34::HEADER_LEN);
        for row in rows {
            p.extend_from_slice(row);
        }
        p
    }

    fn one_base(status_flags: u32, capacity: u32) -> Vec<u8> {
        d34_payload(status_flags, &[raw_row(7, 0, capacity, 501)])
    }

    /// Two closed hours, each carrying one d34 DATA record.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D34,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_base(0, 10),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D34,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_base(0, 11),
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
    fn a_well_formed_stream_yields_its_d34_hours() {
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
        assert_eq!(scan.delta_before_baseline_records, 0);
        assert_eq!(scan.non_contiguous, 0);
        assert_eq!(scan.hours[0].state.len(), 1);
        // The first folded record replaces; every later one is sparse.
        assert_eq!(scan.hours[0].outcome, FoldOutcome::Replace);
        assert_eq!(scan.hours[1].outcome, FoldOutcome::Sparse);
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helper classifies
        // the v1 vocabulary and is not a validity predicate, so an unassigned bit
        // must be rejected BEFORE classification — otherwise it silently reads as
        // "not a failure", gets a baseline/delta meaning, and the hour is measured
        // as healthy.
        for bit in 15..32u32 {
            let status = 1u32 << bit;
            // Alone, combined with a known failure bit, and combined with the
            // informational DELTA_DATA bit; as the first record and after a
            // baseline. The rejection must not depend on any of it.
            for extra in [0u32, 1, COMPACT_STATUS_DELTA_DATA] {
                let mut first = Vec::new();
                first.extend(record(
                    1,
                    100,
                    D34,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &one_base(status | extra, 10),
                ));
                first.extend(frame_end(2, 100));
                assert!(
                    scan_stream(&first, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                    "status bit {bit} (+{extra:#x}) as the first record must be refused"
                );
                let mut after = Vec::new();
                after.extend(record(
                    1,
                    100,
                    D34,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &one_base(0, 10),
                ));
                after.extend(frame_end(2, 100));
                after.extend(record(
                    3,
                    101,
                    D34,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &one_base(status | extra, 11),
                ));
                after.extend(frame_end(4, 101));
                assert!(
                    scan_stream(&after, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                    "status bit {bit} (+{extra:#x}) after a baseline must be refused"
                );
            }
        }
        // And the whole known v1 mask, minus the failure bits, is still accepted:
        // the check must reject unassigned bits and nothing else. DELTA_DATA is
        // the one d34 itself reads, and after a baseline it is a sparse patch.
        for informational in [
            1u32 << 4,
            1 << 5,
            COMPACT_STATUS_DELTA_DATA,
            1 << 11,
            1 << 13,
        ] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D34,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_base(0, 10),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D34,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_base(informational, 11),
            ));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 2, "bit {informational:#x}");
            assert_eq!(scan.failure_records, 0, "bit {informational:#x}");
            assert_eq!(
                scan.delta_before_baseline_records, 0,
                "bit {informational:#x}"
            );
        }
    }

    #[test]
    fn an_hour_state_is_the_accumulator_not_the_record() {
        // The property that makes this harness different from a full-replace
        // one: hour 2 carries ONE row and its state must still hold both bases.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D34,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d34_payload(0, &[raw_row(1, 0, 10, 501), raw_row(2, 0, 20, 502)]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D34,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d34_payload(0, &[raw_row(2, 0, 22, 502)]),
        ));
        s.extend(frame_end(4, 101));
        // An hour that emits no rows at all leaves the state unchanged.
        s.extend(record(
            5,
            102,
            D34,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d34_payload(0, &[]),
        ));
        s.extend(frame_end(6, 102));
        // And a tombstone removes a key.
        s.extend(record(
            7,
            103,
            D34,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d34_payload(0, &[raw_row(1, d34::ROW_FLAG_DELETED, 0, 501)]),
        ));
        s.extend(frame_end(8, 103));

        let scan =
            scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).expect("accepted");
        assert_eq!(scan.hours.len(), 4);
        assert_eq!(scan.hours[0].state.len(), 2);
        assert_eq!(scan.hours[1].state.len(), 2, "the omitted base is retained");
        assert_eq!(scan.hours[1].shipped_rows, 1, "the record carried one row");
        assert_eq!(
            scan.hours[2].state.len(),
            2,
            "an empty record changes nothing"
        );
        assert_eq!(scan.hours[2].shipped_rows, 0);
        assert_eq!(scan.hours[3].state.len(), 1, "the tombstone removed a key");
        assert_eq!(scan.hours[3].shipped_tombstones, 1);
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        // A cut immediately after a d34 DATA record leaves that hour open and
        // ends the stream on a record that is not a FRAME_END. That would produce
        // a plausible shorter number; it must be refused.
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan_stream(&full[..cut], &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary is a different thing: that IS a
        // well-formed shorter stream, and `eof_check` accepts it too. What
        // matters is that it reports the hours it actually closed.
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
            D34,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_base(0, 10),
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
            D34,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_base(0, 10),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D34,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_base(0, 11),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
    }

    #[test]
    fn a_second_d34_record_in_one_hour_is_rejected() {
        // d34 accumulates; two records in one hour would fold two patches into
        // one measurement step. The rule is checked over EVERY d34 DATA record
        // and before status-based exclusion, so all four success/failure
        // orderings are refused — gating it on the record being usable would let
        // a success + failure pair through.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D34,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_base(first, 10),
            ));
            s.extend(record(
                2,
                100,
                D34,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_base(second, 11),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err(),
                "two d34 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time: the
        // fold retains the accumulator and folds nothing, so the hour is not d34
        // data and must not enter either arm — and, because it folds nothing, the
        // NEXT record is still the accumulator's first and therefore replaces.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D34,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_base(1 << bit, 10),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D34,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_base(0, 11),
            ));
            s.extend(frame_end(4, 101));
            let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
                .expect("the stream itself is well-formed");
            assert_eq!(scan.failure_records, 1, "bit {bit}");
            assert_eq!(scan.hours.len(), 1, "bit {bit}");
            assert_eq!(scan.hours[0].game_hour, 101, "bit {bit}");
            assert_eq!(
                scan.hours[0].outcome,
                FoldOutcome::Replace,
                "bit {bit}: a skipped record leaves the accumulator un-baselined"
            );
        }
        // And a non-failure bit (DELTA_DATA, 1<<10) is NOT treated as failure —
        // though on a fresh accumulator it IS a delta before any baseline.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D34,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_base(1 << 10, 10),
        ));
        s.extend(frame_end(2, 100));
        let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy);
        assert!(scan.is_err(), "no usable d34 record remains");
    }

    #[test]
    fn a_delta_before_baseline_record_is_excluded_rather_than_measured() {
        // The second exclusion path (`world_fold.rs:15080`): the fold marks the
        // hour unknown and RETAINS the accumulator. It must not enter either arm,
        // and it must not count as the baseline for what follows.
        const DELTA: u32 = 1 << 10;
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D34,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_base(DELTA, 10),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D34,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_base(0, 11),
        ));
        s.extend(frame_end(4, 101));
        s.extend(record(
            5,
            102,
            D34,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_base(DELTA, 12),
        ));
        s.extend(frame_end(6, 102));
        let scan = scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy)
            .expect("the stream is well-formed");
        assert_eq!(scan.delta_before_baseline_records, 1);
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].game_hour, 101);
        assert_eq!(scan.hours[0].outcome, FoldOutcome::Replace);
        // The delta AFTER the baseline is ordinary data, folded sparsely.
        assert_eq!(scan.hours[1].game_hour, 102);
        assert_eq!(scan.hours[1].outcome, FoldOutcome::Sparse);
    }

    #[test]
    fn a_stream_with_no_d34_records_is_refused_not_reported_as_zero() {
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
    fn a_malformed_d34_payload_is_refused_not_skipped() {
        // A short or inconsistent d34 payload is a corrupt stream, not an hour to
        // drop quietly.
        let mut payload = one_base(0, 10);
        payload.truncate(payload.len() - 1);
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D34,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        ));
        s.extend(frame_end(2, 100));
        assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());

        // A row or header the FOLD rejects must make the SCAN fail, not be
        // accepted and then unwind while measuring.
        let mut invalid: Vec<Vec<u8>> = Vec::new();
        // A non-zero reserved header word (`world_fold.rs:15074`).
        let mut bad_reserved = one_base(0, 10);
        bad_reserved[32..36].copy_from_slice(&1u32.to_le_bytes());
        invalid.push(bad_reserved);
        // An illegal row flag bit (`:15104`).
        invalid.push(d34_payload(0, &[raw_row(7, 1 << 3, 10, 501)]));
        // A province id past `SUPPLY_MAX_PROVINCE_ID` (`:15111`).
        invalid.push(d34_payload(
            0,
            &[raw_row(7, 0, 10, d34::MAX_PROVINCE_ID + 1)],
        ));
        // The same, on a tombstone: the range check precedes the delete branch.
        invalid.push(d34_payload(
            0,
            &[raw_row(
                7,
                d34::ROW_FLAG_DELETED,
                0,
                d34::MAX_PROVINCE_ID + 1,
            )],
        ));
        // A header whose row_count does not match the payload length (`:15089`).
        let mut short_rows = one_base(0, 10);
        short_rows[8..12].copy_from_slice(&2u32.to_le_bytes());
        invalid.push(short_rows);
        for (i, payload) in invalid.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D34,
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
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D34,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_base(0, 10),
            )
        };
        for (emitted, scheduled, status, reserved) in [(0, 1, 0, 0), (1, 0, 0, 0), (0, 0, 0, 1)] {
            let mut s = data(1, 100);
            s.extend(frame_end_sealed(
                2, 100, emitted, scheduled, status, reserved,
            ));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }
        for bit in 15..32u32 {
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
                D34,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_base(0, 10),
            )
        };
        let mut ok = perf(1, 100, 20);
        ok.extend(perf(2, 100, D34));
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
        for mut counts in [(1, 1), (3, 3), (2, 3), (3, 2)].map(|(e, scheduled)| {
            let mut s = perf(1, 100, 20);
            s.extend(perf(2, 100, D34));
            s.extend(data(3, 100));
            s.extend(frame_end_sealed(4, 100, e, scheduled, 0, 0));
            s
        }) {
            cases.push(std::mem::take(&mut counts));
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
                D34,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_base(0, 10),
            ));
            s.extend(frame_end_sealed(3, data_hour, 1, 1, 0, 0));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }
        let mut agreeing = perf(1, 100, 20);
        agreeing.extend(record(
            2,
            100,
            D34,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_base(0, 10),
        ));
        agreeing.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan_stream(&agreeing, &test_collection(), StreamIdentityMode::Legacy).is_ok());
    }

    #[test]
    fn an_excluded_record_is_still_checked_by_the_payload_authority() {
        let failed = 1u32;
        let mut bad = Vec::new();
        let mut changed = one_base(failed, 10);
        changed[12..16].copy_from_slice(&2u32.to_le_bytes());
        bad.push(changed);
        let mut observed = one_base(failed, 10);
        observed[20..24].copy_from_slice(&0u32.to_le_bytes());
        bad.push(observed);
        let mut deleted_count = one_base(failed, 10);
        deleted_count[28..32].copy_from_slice(&2u32.to_le_bytes());
        bad.push(deleted_count);
        let mut short = one_base(failed, 10);
        short.pop();
        bad.push(short);
        bad.push(d34_payload(failed, &[raw_row(0, 0, 10, 501)]));
        bad.push(d34_payload(
            failed,
            &[raw_row(7, 0, 10, 501), raw_row(7, 0, 11, 501)],
        ));
        let mut state_zero = one_base(failed, 10);
        state_zero[d34::HEADER_LEN + 4..d34::HEADER_LEN + 8].copy_from_slice(&0u32.to_le_bytes());
        bad.push(state_zero);
        bad.push(d34_payload(failed, &[raw_row(7, 1 << 3, 10, 501)]));
        bad.push(d34_payload(
            failed,
            &[raw_row(7, 0, 10, d34::MAX_PROVINCE_ID + 1)],
        ));
        let mut deleted_live = d34_payload(failed, &[raw_row(7, d34::ROW_FLAG_DELETED, 0, 501)]);
        deleted_live[d34::HEADER_LEN + 12..d34::HEADER_LEN + 16]
            .copy_from_slice(&1u32.to_le_bytes());
        bad.push(deleted_live);
        bad.push(d34_payload(failed, &[raw_row(7, 0, 1_000_001, 501)]));
        let mut zero_level = one_base(failed, 10);
        zero_level[d34::HEADER_LEN + 16..d34::HEADER_LEN + 20].copy_from_slice(&0u32.to_le_bytes());
        bad.push(zero_level);
        // Every bad payload rides in a stream that WOULD otherwise measure: a
        // second, well-formed hour follows it. Without that trailing hour the
        // scan refuses for an unrelated reason — "no usable d34 DATA records" —
        // and the assertion would hold with `check_authority_payload` deleted,
        // which is a test that cannot fail for its own cause.
        for payload in bad {
            let mut s = record(1, 100, D34, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload);
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D34,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_base(0, 11),
            ));
            s.extend(frame_end(4, 101));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let payload = d34_payload(1 << bit, &[raw_row(0, 0, 10, 501)]);
            let mut s = record(1, 100, D34, COMPACT_RECORD_TYPE_DOMAIN_DATA, &payload);
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D34,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &one_base(0, 11),
            ));
            s.extend(frame_end(4, 101));
            assert!(scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy).is_err());
        }
        let mut s = record(
            1,
            100,
            D34,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_base(failed | COMPACT_STATUS_DELTA_DATA, 10),
        );
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D34,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &one_base(0, 11),
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
        // And corruption of a genuinely well-formed stream, byte by byte.
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
    fn an_hour_closed_without_a_d34_record_refuses_the_stream() {
        // The completeness bound on what this harness does NOT validate (module
        // docs): it cannot check other domains' payloads, so a d34 record
        // relabelled to another domain id — or dropped — must be caught by
        // counting, not silently measured as fewer hours.
        let mut s = good_stream();
        // Find the LAST d34 DATA record and relabel it to an unclaimed domain
        // id. The stream stays well formed — sequence, hour boundaries and
        // FRAME_END are untouched — so the completeness rule is the only rule
        // that can catch it.
        let mut pos = 0usize;
        let mut target = None;
        while pos < s.len() {
            let header = parse_compact_record_header(&s[pos..]).expect("well-formed");
            if header.domain_id == D34 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
            {
                target = Some(pos);
            }
            pos += COMPACT_RECORD_HEADER_LEN + header.payload_len;
        }
        let target = target.expect("good_stream carries a d34 DATA record");
        // domain_id sits at +20: magic(4) + version(2) + header_len(2) +
        // sequence(8) + game_hour(4).
        s[target + 20..target + 22].copy_from_slice(&99u16.to_le_bytes());
        let err = match scan_stream(&s, &test_collection(), StreamIdentityMode::Legacy) {
            Err(e) => e,
            Ok(_) => panic!("an hour with no d34 record must refuse the stream"),
        };
        assert!(
            err.contains("game hour(s) but carries"),
            "the completeness check must be what rejects it: {err}"
        );
    }
}
