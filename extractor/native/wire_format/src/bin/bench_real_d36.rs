//! Measure the wire format against the shipping compact stream on REAL d36
//! `air_variant_reserves` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d36-scheduled capture
//! (`runs/batch2/deepwar_t1`). The stream's own d36 DATA records are the shipping
//! format's cost for the domain: a 48-byte header plus a fixed 24-byte row per
//! **changed** country/variant pair, exactly what the recorder stores today.
//!
//! **d36 is not a full-replace domain, and that shapes the whole measurement.**
//! `replace_or_sparse_after_first` (`world_fold.rs:2927`, called at `:15268`)
//! clears the accumulator for the FIRST non-delta record only; every record after
//! that is a sparse patch of upserts and low-bit deletes. So an hour's state is
//! the accumulator after that hour's record, not the hour's rows, and this tool
//! folds through `wire_format::d36::State::fold_payload` — the same public
//! boundary the module's own tests use — rather than treating a record as a
//! snapshot. The shipping arm consequently re-baselines **once per campaign**
//! while the wire arm re-baselines every 48 hours; the deliverable reports the
//! baseline/delta split so the grammar comparison stays recoverable underneath
//! that policy difference.
//!
//! **Nothing here re-derives a contract it can call.** Header, row and frame
//! decoding go through `wire_format::d36`'s public boundary; H4CR framing, record
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
//!   (`recorder_host/src/stream_fold.rs:277`): a DOMAIN_DATA record opens its
//!   game hour, a FRAME_END closes the open hour and only that hour, and closed
//!   hours strictly increase;
//! - that tracker's **DOMAIN_PERF hour binding** (`:328`-`:365`): no duplicate
//!   non-probe PERF domain inside one hour, no PERF for an hour other than the
//!   open one, no PERF for an already closed hour, and no conflict with the
//!   pending PERF hour before a DATA record opens one;
//! - the **FRAME_END seal fields**, `validate_frame_end_seal_fields`
//!   (`:46`-`:89`), re-implemented here as [`check_frame_end_seal`]: the 32-byte
//!   payload length, `reserved == 0`, `emitted_domains == scheduled_domains`, no
//!   status bit above 13, and no fatal status bit; plus the seal's domain-count
//!   cross-check against the distinct non-probe PERF domains actually observed
//!   in the hour (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority. Production additionally calls
//! `validate_domain_data_payload_at` on every DOMAIN_DATA record (`:594`), so a
//! malformed payload for another domain remains outside this single-domain
//! harness. For d36 itself, [`check_authority_payload`] replays the independent
//! `decode_compact_air_variant_reserves_data_payload` authority
//! (`domain_data_authority.rs:15075`-`:15245`) on every record whatever its
//! status. `d36::parse_header` already runs its minimum-length, four header-count
//! predicates and scale check before exclusion. What remains is the exact payload
//! length and the authority's row walk: bounded nonzero ids, unique
//! `(tag_db_id, variant_id)` keys, known flags, zero reserved words, bounded
//! amounts, and zero-packed deleted amounts. The authority only observes
//! `observed_air_variant_rows`; it places no predicate on that metric.
//!
//! The seal fields, PERF hour binding, DATA arm's pending-PERF rule,
//! status-independent payload authority and stated identity mode are the five
//! checks carried back by the five-check sweep. They cannot change a byte for a
//! recorder-valid capture: they only refuse streams production refuses, and the
//! identity mode defaults to the captures' existing legacy protocol.
//!
//! Two exclusion paths, both taken from the fold's own call order — the status
//! gates run before the row loop (`world_fold.rs:15246`, `:15253`), so a record
//! on either path folds nothing and its hour carries no d36 state:
//!
//! - failure status, via `compact_contract`'s own predicate, never a copied mask;
//! - `DELTA_DATA` before any baseline.
//!
//! Hour uniqueness is enforced across ALL d36 DATA records, of any status, and
//! BEFORE either exclusion, so a success+failure pair inside one hour cannot slip
//! through.
//!
//! Outputs, for the compression step (`scripts/compress_arms.py`):
//!   ship_d36.bin   — the d36 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! **Only d36 DATA payloads are validated.** Production runs
//! `validate_domain_data_payload_at` on the DATA payload of EVERY domain
//! (`stream_fold.rs:593`); this harness cannot, because validating another
//! domain's payload means owning that domain's authority, and `wire_format`
//! must not depend on `recorder_host`. Every DATA record of any other domain
//! therefore passes with its H4CR header, sequence continuity, hour-boundary,
//! PERF-binding and FRAME_END rules checked but its **payload bytes unread**.
//! The concrete hazard — a d36 record relabelled to another domain id, or
//! dropped outright, silently shrinking the measurement — is closed by an
//! explicit completeness check in `scan_stream`: every closed game hour must
//! carry exactly one d36 DATA record, of any status. That is a per-capture
//! assertion, not a grammar rule; a capture where d36 is not scheduled every
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
use wire_format::d36;

const D36: u16 = 36;
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

/// One usable d36 hour: its game hour, the folded state **after** that hour's
/// record, and the record's byte span in the source stream (the shipping arm is
/// those bytes verbatim, header included).
struct D36Hour {
    game_hour: i32,
    state: KeyedState,
    ship_span: Range<usize>,
    /// Rows the record carried, split by kind — the shipping arm's own churn,
    /// which for this domain is not the same quantity as the wire arm's.
    upserts: usize,
    deletes: usize,
}

struct StreamScan {
    hours: Vec<D36Hour>,
    /// d36 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// d36 records flagged DELTA_DATA before any baseline was seen.
    delta_before_baseline: usize,
    /// Measured d36 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// Records that folded a full reset (the domain's one baseline).
    resets: usize,
}

/// Every globally-defined compact status occupies one of bits 0..=13
/// (`recorder_host/src/stream_fold.rs:43`).
// Re-exported, not re-declared: a local `(1 << 14) - 1` became wrong the moment
// the contract assigned bit 14 to the measured-empty badge.
const FRAME_END_KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;

/// The reader-context probe domain, excluded from seal domain accounting
/// (`recorder_host/src/stream_fold.rs:330`).
const PROBE_DOMAIN_ID: u16 = 250;

/// Limits enforced by d36's independent payload authority
/// (`domain_data_authority.rs:715`-`:717`).
const AUTHORITY_MAX_TAG_DB_ID: u32 = 1_024;
const AUTHORITY_MAX_VARIANT_ID: u32 = i32::MAX as u32;
const AUTHORITY_MAX_AMOUNT_RAW: i64 = 100_000_000_000_000_000;

/// Mirror `validate_frame_end_seal_fields`
/// (`recorder_host/src/stream_fold.rs:46`-`:89`).
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

/// Replay the d36 predicates made by the status-independent second authority
/// (`domain_data_authority.rs:15075`-`:15245`) that `d36::parse_header` has not
/// already run before exclusion.
fn check_authority_payload(payload: &[u8], at_byte: usize) -> Result<(), String> {
    // The exact-length predicate (`:15123`-`:15130`) is already implemented by
    // the domain module; call it rather than re-deriving it.
    let (_, body) =
        d36::split_payload(payload).map_err(|e| format!("d36 payload at byte {at_byte}: {e}"))?;
    let mut seen = BTreeSet::new();
    for row in body.chunks_exact(d36::ROW_LEN) {
        // The module owns the flags and reserved-word predicates
        // (`domain_data_authority.rs:15212`-`:15220`).
        let flags = d36::row_flags(row).map_err(|e| format!("d36 row at byte {at_byte}: {e}"))?;
        let tag_db_id = u32::from_le_bytes(row[0..4].try_into().expect("4 bytes"));
        let variant_id = u32::from_le_bytes(row[4..8].try_into().expect("4 bytes"));
        let amount_raw = i64::from_le_bytes(row[16..24].try_into().expect("8 bytes"));
        if tag_db_id == 0 || tag_db_id > AUTHORITY_MAX_TAG_DB_ID {
            return Err(format!(
                "d36 row at byte {at_byte}: invalid tag_db_id {tag_db_id}"
            ));
        }
        if variant_id == 0 || variant_id > AUTHORITY_MAX_VARIANT_ID {
            return Err(format!(
                "d36 row at byte {at_byte}: invalid variant_id {variant_id}"
            ));
        }
        if !seen.insert((tag_db_id, variant_id)) {
            return Err(format!(
                "d36 row at byte {at_byte}: duplicate tag_db_id {tag_db_id} \
                 variant_id {variant_id}"
            ));
        }
        if !(-AUTHORITY_MAX_AMOUNT_RAW..=AUTHORITY_MAX_AMOUNT_RAW).contains(&amount_raw) {
            return Err(format!(
                "d36 row at byte {at_byte}: invalid amount_raw {amount_raw}"
            ));
        }
        if flags & d36::ROW_FLAG_DELETED != 0 && amount_raw != 0 {
            return Err(format!(
                "d36 row at byte {at_byte}: deleted row carries nonzero amount"
            ));
        }
    }
    Ok(())
}

/// Validate the whole stream against the canonical contract and fold its d36
/// DATA records into per-hour states. Every rejection path is an `Err`, never a
/// silently shorter measurement and never a panic.
fn scan_stream(
    stream: &[u8],
    schema: &wire_format::Schema,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    // The two constants `d36` names locally, against the contract crate that owns
    // them. A drift here would silently change which hours are measured.
    if d36::STATUS_DELTA_DATA != COMPACT_STATUS_DELTA_DATA {
        return Err("d36::STATUS_DELTA_DATA disagrees with compact_contract".to_string());
    }
    if d36::FAILURE_STATUS_MASK != COMPACT_STATUS_FAILURE_MASK {
        return Err("d36::FAILURE_STATUS_MASK disagrees with compact_contract".to_string());
    }

    // The protocol is stated by the caller, never inferred from the first record
    // (`stream_fold.rs:220`; `stream_validator.rs:328`). A decapitated IdentityV1
    // stream otherwise looks exactly like legacy.
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours: Vec<D36Hour> = Vec::new();
    let mut failure_records = 0usize;
    let mut delta_before_baseline = 0usize;
    let mut non_contiguous = 0usize;
    let mut resets = 0usize;

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d36 DATA record of ANY status — the uniqueness rule.
    let mut last_d36_hour: Option<i32> = None;
    // Hour of the last d36 record that entered the measurement — the contiguity
    // report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;

    // The accumulator, carried across records exactly as the recorder carries it.
    let mut state = d36::empty_state();

    // Closed game hours and d36 DATA records of ANY status — the
    // completeness rule at the end of the scan.
    let mut closed_hours = 0usize;
    let mut d36_records = 0usize;
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
                    // `stream_fold.rs:378`-`:386`: DATA must open the hour any
                    // pending PERF records already bound.
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
                // Seal validation precedes the open-hour match in production
                // (`stream_fold.rs:401`-`:419`).
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

        if header.domain_id == D36 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d36_records += 1;
            // One d36 DATA record per game hour, checked over EVERY d36 record and
            // BEFORE any status-based exclusion. d36 accumulates, so a second
            // record in one hour would fold twice into one measurement step — and
            // gating this on the record being usable would let a success/failure
            // pair through the rule entirely.
            if let Some(previous) = last_d36_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d36 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d36_hour = Some(header.game_hour_raw);

            let d36_header = d36::parse_header(payload)
                .map_err(|e| format!("d36 payload at byte {pos}: {e}"))?;

            // VALIDITY BEFORE CLASSIFICATION. Everything below that reads the
            // status word — the failure predicate here, the failure and
            // baseline/delta decisions inside `fold_payload` — classifies the v1
            // vocabulary; none of it is a validity predicate.
            // `compact_contract/src/lib.rs:121`-`:123` says so in as many words:
            // "Callers handling untrusted status words must first reject a
            // non-zero `compact_status_unknown_flags_v1` result; otherwise an
            // unassigned bit has no defined classification." Bits 14..=31 are
            // unassigned (`status_flags_base.rs:21`-`:24`), and an unassigned bit
            // "is not evidence of a future informational status".
            //
            // Without this, a record with `status_flags = 1 << 14` reads as "not
            // a failure" — the failure helper masks with the *known* failure set
            // and returns zero for a bit it does not know — and the hour is
            // measured as healthy. The production lane rejects first
            // (`lanes/division_state_movement_live/domain20_compact.rs:419`);
            // this harness does the same rather than re-deriving a lenient rule.
            // No capture carries an unassigned bit, so no measured number moves.
            let unknown_status = compact_status_unknown_flags_v1(d36_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d36 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            // The independent payload authority runs on every DOMAIN_DATA record
            // whatever its status (`stream_fold.rs:594`).
            check_authority_payload(payload, pos)?;

            // The fold's own predicate, not a copied bit mask: it applies d36's
            // lane-local mask (none) and expected-partial tolerance (none), so it
            // cannot drift from the recorder. `d36::State::fold_payload` reaches
            // the same verdict from its own constant; both are asserted to agree.
            let contract_failure = compact_status_failure_flags_after_expected_partial_tolerance(
                D36,
                d36_header.status_flags,
            ) != 0;

            let outcome = state
                .fold_payload(schema, payload, 0)
                .map_err(|e| format!("d36 payload at byte {pos}: {e}"))?;

            match outcome {
                d36::FoldOutcome::FailureRetained => {
                    if !contract_failure {
                        return Err(format!(
                            "d36 payload at byte {pos}: failure verdicts disagree"
                        ));
                    }
                    failure_records += 1;
                }
                d36::FoldOutcome::DeltaBeforeBaseline => {
                    delta_before_baseline += 1;
                }
                d36::FoldOutcome::Folded {
                    reset,
                    upserts,
                    deletes,
                } => {
                    if contract_failure {
                        return Err(format!(
                            "d36 payload at byte {pos}: failure verdicts disagree"
                        ));
                    }
                    if reset {
                        resets += 1;
                    }
                    if let Some(previous) = last_measured_hour {
                        if header.game_hour_raw != previous + 1 {
                            non_contiguous += 1;
                        }
                    }
                    last_measured_hour = Some(header.game_hour_raw);
                    hours.push(D36Hour {
                        game_hour: header.game_hour_raw,
                        state: state.reserves.clone(),
                        ship_span: pos..record_end,
                        upserts,
                        deletes,
                    });
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
    if hours.is_empty() {
        return Err("stream carries no usable d36 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d36 DATA
    // record, of any status. Production validates the payload of EVERY domain's
    // DATA record (`stream_fold.rs:593`); this harness cannot (module docs), so
    // a d36 record relabelled to another domain id — or lost outright — would
    // otherwise be silently ignored and the run would report a plausible
    // measurement over fewer hours. Per-hour uniqueness is already enforced
    // above, so equality of the two counts is exactly-one per closed hour. A
    // per-capture assertion, not a grammar rule: a capture that does not
    // schedule d36 every hour fails here loudly and a human adjusts, instead
    // of a number quietly shrinking.
    if d36_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d36_records} d36 DATA \
             record(s); every closed hour of a d36-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    Ok(StreamScan {
        hours,
        failure_records,
        delta_before_baseline,
        non_contiguous,
        resets,
    })
}

fn main() {
    const USAGE: &str =
        "usage: bench_real_d36 <collector_stream.bin> <out dir> [legacy|identity-v1]";
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d36::schema();
    let collection = &schema.collections[0];

    // ---- Pass 1: validate the WHOLE stream and fold its d36 DATA records. ----
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result.
    let scan = match scan_stream(&stream, &schema, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: encode both arms. Nothing here reads stream content again. --

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
    let mut ship_upserts_total = 0usize;
    let mut ship_deletes_total = 0usize;
    let mut rows_total = 0usize;

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());
        ship_upserts_total += hour.upserts;
        ship_deletes_total += hour.deletes;

        // The hour's state: the accumulator AFTER this hour's record.
        let state: &KeyedState = &hour.state;
        rows_total += state.len();

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
        // nothing is still checked: the reader's retained state must already equal
        // the fold's, which is precisely what "nothing changed" asserts.
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

    for (name, bytes) in [("ship_d36.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d36.offsets", &ship_offsets),
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
        "failure-status hours {} (excluded)   delta-before-baseline {} (excluded)",
        scan.failure_records, scan.delta_before_baseline
    );
    println!(
        "shipping resets      {} (the domain's own baseline count over the capture)",
        scan.resets
    );
    println!(
        "shipping churn       {:.2} upserts/hour, {:.3} deletes/hour ({} + {} rows total)",
        ship_upserts_total as f64 / hours_seen as f64,
        ship_deletes_total as f64 / hours_seen as f64,
        ship_upserts_total,
        ship_deletes_total
    );
    println!(
        "table rows avg       {:.1} (max {})",
        rows_total as f64 / hours_seen as f64,
        scan.hours.iter().map(|h| h.state.len()).max().unwrap_or(0)
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
    println!("ship_d36.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.1}%)   baseline {} + delta {}",
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
        record(
            sequence,
            game_hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &payload,
        )
    }

    fn row(tag_db_id: u32, variant_id: u32, flags: u32, amount_raw: i64) -> Vec<u8> {
        let mut r = Vec::new();
        r.extend_from_slice(&tag_db_id.to_le_bytes());
        r.extend_from_slice(&variant_id.to_le_bytes());
        r.extend_from_slice(&flags.to_le_bytes());
        r.extend_from_slice(&0u32.to_le_bytes()); // reserved
        r.extend_from_slice(&amount_raw.to_le_bytes());
        r
    }

    /// A d36 DATA payload: the 48-byte header then the rows.
    fn d36_payload(status_flags: u32, rows: &[Vec<u8>]) -> Vec<u8> {
        let deleted = rows
            .iter()
            .filter(|r| u32::from_le_bytes([r[8], r[9], r[10], r[11]]) & 1 != 0)
            .count() as u32;
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        for word in [
            rows.len() as u32, // row_count
            rows.len() as u32, // changed_rows
            status_flags,
            8,                 // observed_country_rows
            64,                // observed_stockpile_rows
            16,                // observed_air_variant_rows
            rows.len() as u32, // matched_stockpile_rows
            0,                 // invalid_country_rows
            deleted,
            d36::FIXED_SCALE_RAW,
        ] {
            p.extend_from_slice(&word.to_le_bytes());
        }
        assert_eq!(p.len(), d36::HEADER_LEN);
        for r in rows {
            p.extend_from_slice(r);
        }
        p
    }

    /// Two closed hours: a baseline, then a sparse patch.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D36,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d36_payload(0, &[row(7, 11, 0, 100_000), row(7, 12, 0, 250_000)]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D36,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d36_payload(0, &[row(7, 11, 0, 300_000)]),
        ));
        s.extend(frame_end(4, 101));
        s
    }

    fn scan(stream: &[u8]) -> Result<StreamScan, String> {
        scan_stream(stream, &d36::schema(), StreamIdentityMode::Legacy)
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
    fn a_well_formed_stream_yields_its_d36_hours_as_accumulated_states() {
        let s = scan(&good_stream()).expect("accepted");
        assert_eq!(s.hours.len(), 2);
        assert_eq!(s.hours[0].game_hour, 100);
        assert_eq!(s.hours[1].game_hour, 101);
        assert_eq!(s.failure_records, 0);
        assert_eq!(s.delta_before_baseline, 0);
        assert_eq!(s.non_contiguous, 0);
        assert_eq!(s.resets, 1, "only the first record clears");
        // The second record carries ONE row and the second hour still holds TWO:
        // the accumulated state, not the record's rows. This is the whole
        // difference between d36 and a full-replace domain, and a harness that
        // treated a record as a snapshot would report 1 here.
        assert_eq!(s.hours[0].state.len(), 2);
        assert_eq!(s.hours[1].state.len(), 2);
        assert_eq!(s.hours[1].upserts, 1);
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helper classifies
        // the v1 vocabulary and is not a validity predicate, so an unassigned bit
        // must be rejected BEFORE classification — otherwise it silently reads as
        // "not a failure", `fold_payload` gives it a baseline/delta meaning, and
        // the hour is measured as healthy.
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
                    D36,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d36_payload(status | extra, &[row(7, 11, 0, 100_000)]),
                ));
                first.extend(frame_end(2, 100));
                assert!(
                    scan(&first).is_err(),
                    "status bit {bit} (+{extra:#x}) as the first record must be refused"
                );
                let mut after = Vec::new();
                after.extend(record(
                    1,
                    100,
                    D36,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d36_payload(0, &[row(7, 11, 0, 100_000)]),
                ));
                after.extend(frame_end(2, 100));
                after.extend(record(
                    3,
                    101,
                    D36,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &d36_payload(status | extra, &[row(7, 11, 0, 300_000)]),
                ));
                after.extend(frame_end(4, 101));
                assert!(
                    scan(&after).is_err(),
                    "status bit {bit} (+{extra:#x}) after a baseline must be refused"
                );
            }
        }
        // And the whole known v1 mask, minus the failure bits, is still accepted:
        // the check must reject unassigned bits and nothing else. DELTA_DATA is
        // the one d36 itself reads, and after a baseline it is a sparse patch.
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
                D36,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d36_payload(0, &[row(7, 11, 0, 100_000)]),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D36,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d36_payload(informational, &[row(7, 11, 0, 300_000)]),
            ));
            s.extend(frame_end(4, 101));
            let scanned =
                scan(&s).unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scanned.hours.len(), 2, "bit {informational:#x}");
            assert_eq!(scanned.failure_records, 0, "bit {informational:#x}");
            assert_eq!(scanned.delta_before_baseline, 0, "bit {informational:#x}");
        }
    }

    #[test]
    fn a_delete_row_shrinks_the_measured_state() {
        let mut st = Vec::new();
        st.extend(record(
            1,
            100,
            D36,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d36_payload(0, &[row(7, 11, 0, 100_000), row(7, 12, 0, 250_000)]),
        ));
        st.extend(frame_end(2, 100));
        st.extend(record(
            3,
            101,
            D36,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d36_payload(0, &[row(7, 12, 1, 0)]),
        ));
        st.extend(frame_end(4, 101));
        let s = scan(&st).expect("accepted");
        assert_eq!(s.hours[1].state.len(), 1);
        assert_eq!(s.hours[1].deletes, 1);
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected() {
        // A cut immediately after a d36 DATA record leaves that hour open and ends
        // the stream on a record that is not a FRAME_END.
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan(&full[..cut]).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary IS a well-formed shorter stream,
        // and `eof_check` accepts it too. What matters is that it reports the
        // hours it actually closed.
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
                (0usize, vec![b'X']),              // magic
                (4, 2u16.to_le_bytes().to_vec()),  // unsupported version
                (6, 33u16.to_le_bytes().to_vec()), // unsupported header length
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
            D36,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d36_payload(0, &[row(7, 11, 0, 100_000)]),
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
            D36,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d36_payload(0, &[row(7, 11, 0, 100_000)]),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D36,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d36_payload(0, &[row(7, 11, 0, 200_000)]),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan(&s).is_err());
    }

    #[test]
    fn a_second_d36_record_in_one_hour_is_rejected() {
        // d36 accumulates, so two records in one hour would fold twice into one
        // measurement step. The rule is checked over EVERY d36 DATA record and
        // before status-based exclusion, so all four success/failure orderings are
        // refused.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D36,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d36_payload(first, &[row(7, 11, 0, 100_000)]),
            ));
            s.extend(record(
                2,
                100,
                D36,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d36_payload(second, &[row(7, 11, 0, 200_000)]),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan(&s).is_err(),
                "two d36 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time: the
        // fold retains the accumulator and folds nothing, so the hour is not d36
        // data and must not enter either arm — and it must not consume the
        // domain's one baseline either.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D36,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d36_payload(1 << bit, &[row(7, 11, 0, 100_000)]),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D36,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d36_payload(0, &[row(7, 12, 0, 200_000)]),
            ));
            s.extend(frame_end(4, 101));
            let scanned = scan(&s).expect("the stream itself is well-formed");
            assert_eq!(scanned.failure_records, 1, "bit {bit}");
            assert_eq!(scanned.hours.len(), 1, "bit {bit}");
            assert_eq!(scanned.hours[0].game_hour, 101, "bit {bit}");
            assert_eq!(scanned.resets, 1, "bit {bit}: the baseline survives");
            assert_eq!(
                scanned.hours[0].state.len(),
                1,
                "bit {bit}: the failed record folded nothing"
            );
        }
    }

    #[test]
    fn a_delta_before_any_baseline_is_excluded_rather_than_measured() {
        // The second exclusion path (`world_fold.rs:15253`).
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D36,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d36_payload(COMPACT_STATUS_DELTA_DATA, &[row(7, 11, 0, 100_000)]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D36,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d36_payload(0, &[row(7, 12, 0, 200_000)]),
        ));
        s.extend(frame_end(4, 101));
        let scanned = scan(&s).expect("the stream itself is well-formed");
        assert_eq!(scanned.delta_before_baseline, 1);
        assert_eq!(scanned.hours.len(), 1);
        assert_eq!(scanned.hours[0].game_hour, 101);
        assert_eq!(
            scanned.hours[0].state.len(),
            1,
            "the rejected delta's row never entered the table"
        );
    }

    #[test]
    fn a_stream_with_no_d36_records_is_refused_not_reported_as_zero() {
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
    fn a_malformed_d36_payload_is_refused_not_skipped() {
        // A short or inconsistent d36 payload is a corrupt stream, not an hour to
        // drop quietly. The whole rejection space the fold defines, not one case.
        let mut short = d36_payload(0, &[row(7, 11, 0, 100_000)]);
        short.truncate(short.len() - 1);
        let mut bad_payloads: Vec<Vec<u8>> = vec![short, vec![0u8; 47]];
        // an unknown row flag bit (`world_fold.rs:15278`)
        bad_payloads.push(d36_payload(0, &[row(7, 11, 2, 100_000)]));
        // a nonzero reserved word (`:15282`)
        let mut reserved = d36_payload(0, &[row(7, 11, 0, 100_000)]);
        let at = d36::HEADER_LEN + 12;
        reserved[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
        bad_payloads.push(reserved);
        // a wrong scale word (`:15245`)
        let mut scale = d36_payload(0, &[row(7, 11, 0, 100_000)]);
        scale[44..48].copy_from_slice(&32_768u32.to_le_bytes());
        bad_payloads.push(scale);
        // changed_rows > row_count (`:15217`)
        let mut counts = d36_payload(0, &[row(7, 11, 0, 100_000)]);
        counts[12..16].copy_from_slice(&99u32.to_le_bytes());
        bad_payloads.push(counts);
        for (i, payload) in bad_payloads.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D36,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            assert!(
                scan(&s).is_err(),
                "malformed payload {i} must make the scan fail"
            );
        }
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        let data = || d36_payload(0, &[row(7, 11, 0, 100_000)]);
        let mut ok = Vec::new();
        ok.extend(record(
            1,
            100,
            D36,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &data(),
        ));
        ok.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        assert!(scan(&ok).is_ok());

        let mut bad_seals = vec![
            frame_end_sealed(2, 100, 0, 0, 0, 1),
            frame_end_sealed(2, 100, 0, 1, 0, 0),
        ];
        for bit in 15..32u32 {
            bad_seals.push(frame_end_sealed(2, 100, 0, 0, 1 << bit, 0));
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            bad_seals.push(frame_end_sealed(2, 100, 0, 0, 1 << bit, 0));
        }
        for sealed in bad_seals {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D36,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &data(),
            ));
            s.extend(sealed);
            assert!(scan(&s).is_err());
        }
        for len in [0usize, 1, 16, 31, 33, 64] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D36,
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
            assert!(scan(&s).is_err(), "{len}-byte FRAME_END");
        }
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D36,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &data(),
            ));
            s.extend(frame_end_sealed(2, 100, 0, 0, informational, 0));
            assert!(scan(&s).is_ok(), "informational status {informational:#x}");
        }
        let mut later = good_stream();
        let last = boundary(&later, 3);
        later[last + COMPACT_RECORD_HEADER_LEN + 28..last + COMPACT_RECORD_HEADER_LEN + 32]
            .copy_from_slice(&1u32.to_le_bytes());
        assert!(
            scan(&later).is_err(),
            "a later bad seal refuses the whole stream"
        );
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D36,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d36_payload(0, &[row(7, 11, 0, 100_000)]),
            )
        };
        let mut ok = Vec::new();
        ok.extend(perf(1, 100, 20));
        ok.extend(perf(2, 100, D36));
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

        for (emitted, scheduled) in [(1, 1), (3, 3), (2, 3), (3, 2)] {
            let mut s = Vec::new();
            s.extend(perf(1, 100, 20));
            s.extend(perf(2, 100, D36));
            s.extend(data(3, 100));
            s.extend(frame_end_sealed(4, 100, emitted, scheduled, 0, 0));
            assert!(scan(&s).is_err(), "{emitted}/{scheduled}");
        }

        let mut two_hours = Vec::new();
        two_hours.extend(perf(1, 100, 20));
        two_hours.extend(data(2, 100));
        two_hours.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        two_hours.extend(perf(4, 101, 20));
        two_hours.extend(data(5, 101));
        two_hours.extend(frame_end_sealed(6, 101, 1, 1, 0, 0));
        assert_eq!(scan(&two_hours).expect("set resets").hours.len(), 2);
    }

    #[test]
    fn a_data_record_must_open_the_hour_the_pending_perf_records_bound() {
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D36,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d36_payload(0, &[row(7, 11, 0, 100_000)]),
            )
        };
        for (perf_hour, data_hour) in [(100, 101), (101, 100), (100, 200), (200, 100)] {
            let mut s = Vec::new();
            s.extend(perf(1, perf_hour, 20));
            s.extend(data(2, data_hour));
            s.extend(frame_end_sealed(3, data_hour, 1, 1, 0, 0));
            assert!(scan(&s).is_err(), "{perf_hour} then {data_hour}");
        }
        let mut agreeing = Vec::new();
        agreeing.extend(perf(1, 100, 20));
        agreeing.extend(data(2, 100));
        agreeing.extend(frame_end_sealed(3, 100, 1, 1, 0, 0));
        assert!(scan(&agreeing).is_ok());
    }

    #[test]
    fn an_excluded_record_is_still_checked_by_the_payload_authority() {
        const FAILED: u32 = 1;
        let mut bad_payloads = Vec::new();

        let mut short = d36_payload(FAILED, &[row(7, 11, 0, 100_000)]);
        short.pop();
        bad_payloads.push(short);
        let mut long = d36_payload(FAILED, &[row(7, 11, 0, 100_000)]);
        long.push(0);
        bad_payloads.push(long);
        bad_payloads.push(d36_payload(FAILED, &[row(0, 11, 0, 100_000)]));
        bad_payloads.push(d36_payload(
            FAILED,
            &[row(AUTHORITY_MAX_TAG_DB_ID + 1, 11, 0, 100_000)],
        ));
        bad_payloads.push(d36_payload(FAILED, &[row(7, 0, 0, 100_000)]));
        bad_payloads.push(d36_payload(
            FAILED,
            &[row(7, AUTHORITY_MAX_VARIANT_ID + 1, 0, 100_000)],
        ));
        bad_payloads.push(d36_payload(
            FAILED,
            &[row(7, 11, 0, 100_000), row(7, 11, 0, 200_000)],
        ));
        bad_payloads.push(d36_payload(FAILED, &[row(7, 11, 2, 100_000)]));
        let mut reserved = d36_payload(FAILED, &[row(7, 11, 0, 100_000)]);
        reserved[d36::HEADER_LEN + 12..d36::HEADER_LEN + 16].copy_from_slice(&1u32.to_le_bytes());
        bad_payloads.push(reserved);
        bad_payloads.push(d36_payload(
            FAILED,
            &[row(7, 11, 0, AUTHORITY_MAX_AMOUNT_RAW + 1)],
        ));
        bad_payloads.push(d36_payload(
            FAILED,
            &[row(7, 11, 0, -AUTHORITY_MAX_AMOUNT_RAW - 1)],
        ));
        bad_payloads.push(d36_payload(FAILED, &[row(7, 11, 1, 1)]));

        // Every bad payload rides in a stream that WOULD otherwise measure: a
        // second, well-formed hour follows it. Without that trailing hour the
        // scan refuses for an unrelated reason — "no usable d36 DATA records" —
        // and the assertion would hold with `check_authority_payload` deleted,
        // which is a test that cannot fail for its own cause.
        for (i, payload) in bad_payloads.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D36,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload,
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D36,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d36_payload(0, &[row(7, 12, 0, 200_000)]),
            ));
            s.extend(frame_end(4, 101));
            assert!(scan(&s).is_err(), "excluded malformed payload {i}");
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D36,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d36_payload(1 << bit, &[row(0, 11, 0, 0)]),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D36,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d36_payload(0, &[row(7, 12, 0, 200_000)]),
            ));
            s.extend(frame_end(4, 101));
            assert!(scan(&s).is_err(), "failure bit {bit}");
        }

        let mut well_formed = d36_payload(FAILED, &[row(7, 11, 0, 100_000)]);
        // This authority records `observed_air_variant_rows` as a metric but
        // places no predicate on it (`domain_data_authority.rs:15092`, `:15152`).
        well_formed[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D36,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &well_formed,
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D36,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &d36_payload(0, &[row(7, 12, 0, 200_000)]),
        ));
        s.extend(frame_end(4, 101));
        let scanned = scan(&s).expect("well-formed excluded record");
        assert_eq!(scanned.failure_records, 1);
        assert_eq!(scanned.hours.len(), 1);
        assert_eq!(scanned.hours[0].game_hour, 101);
    }

    #[test]
    fn the_identity_protocol_is_stated_not_sniffed() {
        use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;

        let decapitated = good_stream();
        assert!(scan_stream(&decapitated, &d36::schema(), StreamIdentityMode::IdentityV1).is_err());
        assert!(scan(&decapitated).is_ok());

        let mut identity_stream = record(
            0,
            0,
            0,
            COMPACT_RECORD_TYPE_STREAM_HEADER,
            b"CID1-not-a-real-identity-payload",
        );
        identity_stream.extend(good_stream());
        assert!(scan(&identity_stream).is_err());
    }

    #[test]
    fn an_authority_bounded_extreme_amount_is_measured_not_refused() {
        // The mapper must not impose its old 2^51 exactness band as a validity
        // limit. Production's independent authority does impose the wider
        // +/-1e17 bound (`domain_data_authority.rs:717`, `:15222`-`:15229`), so
        // both endpoints remain valid and anything beyond belongs in check 4.
        for amount in [
            (1i64 << 51) + 1,
            AUTHORITY_MAX_AMOUNT_RAW,
            -AUTHORITY_MAX_AMOUNT_RAW,
        ] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D36,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &d36_payload(0, &[row(7, 11, 0, amount)]),
            ));
            s.extend(frame_end(2, 100));
            let scanned = scan(&s).unwrap_or_else(|e| panic!("amount {amount}: {e}"));
            assert_eq!(scanned.hours.len(), 1, "amount {amount}");
            assert_eq!(scanned.hours[0].state.len(), 1, "amount {amount}");
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
    fn an_hour_closed_without_a_d36_record_refuses_the_stream() {
        // The completeness bound on what this harness does NOT validate (module
        // docs): it cannot check other domains' payloads, so a d36 record
        // relabelled to another domain id — or dropped — must be caught by
        // counting, not silently measured as fewer hours.
        let mut s = good_stream();
        // Find the LAST d36 DATA record and relabel it to an unclaimed domain
        // id. The stream stays well formed — sequence, hour boundaries and
        // FRAME_END are untouched — so the completeness rule is the only rule
        // that can catch it.
        let mut pos = 0usize;
        let mut target = None;
        while pos < s.len() {
            let header = parse_compact_record_header(&s[pos..]).expect("well-formed");
            if header.domain_id == D36 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
            {
                target = Some(pos);
            }
            pos += COMPACT_RECORD_HEADER_LEN + header.payload_len;
        }
        let target = target.expect("good_stream carries a d36 DATA record");
        // domain_id sits at +20: magic(4) + version(2) + header_len(2) +
        // sequence(8) + game_hour(4).
        s[target + 20..target + 22].copy_from_slice(&99u16.to_le_bytes());
        let err = match scan_stream(&s, &d36::schema(), StreamIdentityMode::Legacy) {
            Err(e) => e,
            Ok(_) => panic!("an hour with no d36 record must refuse the stream"),
        };
        assert!(
            err.contains("game hour(s) but carries"),
            "the completeness check must be what rejects it: {err}"
        );
    }
}
