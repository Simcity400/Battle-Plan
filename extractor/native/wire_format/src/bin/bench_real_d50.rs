//! Measure the wire format against the shipping compact stream on REAL d50
//! `naval_combat_members_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d49/d50-scheduled capture
//! (`runs/batch3/deepwar_a`). The stream's own d50 DATA records are the shipping
//! format's cost for the domain: a 28-byte header, a fixed 88-byte row per changed
//! member and a two-byte empty string table, exactly what the recorder stores
//! today. This tool replays those records into per-hour states — the same fold
//! `world_fold::fold_naval_combat_members` performs — and re-encodes every hour
//! with this crate's grammar: a BASELINE at each 48-hour block start and
//! field-granular stored-minimal DELTAs between.
//!
//! **Nothing here re-derives a contract it can call.** Header, row and frame
//! decoding go through `wire_format::d50`'s public boundary — including
//! `d50::State::fold_payload`, because one d50 record is not one d50 state: the
//! grammar is pure cumulative delta, so an hour's state is the whole history of
//! the stream up to that hour. H4CR framing, record semantics and sequence
//! continuity go through `compact_contract`'s own parser and stateful validator.
//! A harness that reimplements either drifts from the recorder without anything
//! failing.
//!
//! **The whole stream is validated before any artifact is written.** A stream that
//! is truncated, spliced, or corrupt would otherwise yield a smaller but entirely
//! plausible measurement — a silently wrong number, which is worse than a crash.
//! `scan_stream` therefore enforces, fail-closed:
//!
//! - every record header through `compact_contract::parse_compact_record_header`
//!   (magic, version, header length, payload bound);
//! - every record through `compact_contract::stream_validator::StatefulStreamValidator`
//!   (sequence continuity from the stream's first record, record-type and
//!   record-flag semantics, FRAME_END shape);
//! - the hour boundary rules of `recorder_host::stream_fold::BoundaryTracker`
//!   (`recorder_host/src/stream_fold.rs:277`), its DOMAIN_PERF hour binding
//!   (`:328`-`:365`), and the DATA arm's pending-PERF rule (`:378`-`:386`);
//! - FRAME_END seal validation (`:46`-`:89`) and its distinct non-probe PERF
//!   domain-count cross-check (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority. This crate cannot validate other domains' payloads, but it
//! replays d50's full status-independent payload authority.
//!
//! `decode_compact_naval_combat_members_live_data_payload`
//! (`domain_data_authority.rs:8709`-`:8722`) decodes and validates every member row
//! on every record regardless of status, via
//! `naval4950::decode_members_data_payload`. `d50::split_payload` **is** that
//! authority — the same header checks, the same string-table walk and the same
//! per-row validation — so this harness calls it on every d50 record before it
//! classifies the record's status, and a violation refuses the stream.
//!
//! **One exclusion path from the status word, three from the fold itself.**
//! `fold_naval_combat_members` decodes first, then returns without folding on a
//! failure status (`world_fold.rs:20140`), then on each of its three whole-hour
//! refusals — orphan member, duplicate identity, retired resurrection
//! (`:20146`, `:20153`, `:20167`). All four leave the accumulator retained and the
//! domain-hour unknown, so all four are *excluded hours*, never stream rejections.
//! The status predicate is
//! `compact_status_failure_flags_after_expected_partial_tolerance` — the
//! recorder's own, never a hand-copied mask — and the three refusals are decided
//! inside the shared fold, at the fold's own position in its call order. Hour
//! uniqueness is checked over EVERY d50 DATA record and BEFORE any exclusion, so a
//! success + failure pair inside one hour cannot slip through.
//!
//! **D49 is MODELLED, not asserted inert.** Two of d50's semantics read D49's
//! accumulator (the orphan refusal) and one D49 path writes d50's own accumulator
//! (a lifecycle tombstone drops the combat's whole `members` map,
//! `world_fold.rs:20098`). Neither "inert" treatment is available for a writer the
//! domain's own refusal reads, so `d50::State::fold_lifecycle_payload` reproduces
//! D49's structural validation, its three non-folding paths in the fold's own
//! order, and its two accumulator writes — over the KEY COLUMNS ONLY. **No d49
//! port exists here**: no d49 schema, no d49 field id, no d49 record.
//!
//! **The headline result on `runs/batch3/deepwar_a` is that the recorder publishes
//! no d50 state at all.** Measured here: not one of the capture's 109 live D49
//! lifecycle rows carries the promoted location/region pair, and the collector
//! badges every hour that emits one with lane-local status bit 11 (72 hours). Both
//! halves of `fold_naval_combat_lifecycle`'s first gate therefore fire on every
//! D49 record that carries a live row, D49's `naval_combats` accumulator stays
//! empty for all 514 hours, and 512 of the 514 d50 records are refused as orphans.
//! That is the fold-faithful number and it is the primary result.
//!
//! Because a grammar cannot be sized against an empty state, the harness also
//! accepts `d50::ParentPolicy::IdentityOnly`, which lets D49's value-completeness
//! gates mask D49's own values without masking the combat identity d50's orphan
//! refusal reads. Runs under that policy are a **size-accounting arm** and are
//! labelled as one wherever they are reported — the standing d11's
//! `ship_d11_no_pdq1.bin` has. The two runs write to different output
//! directories; nothing merges them.
//!
//! One deliberate divergence from production, and it is the D49 one. Production's
//! D49 DATA authority additionally re-promotes the lane-local status bit 11 and
//! records a parse error whenever a lifecycle payload carries it or omits a
//! promoted location/region pair (`domain_data_authority.rs:8687`-`:8706`) — which
//! is why the 3a capture's certification log names 72 D49 records. The WORLD FOLD
//! treats exactly those records as an excluded D49 hour with the accumulator
//! retained (`world_fold.rs:20047`-`:20066`), and it is the fold's accumulator that
//! d50's semantics read, so this harness follows the fold. The 72 hours are
//! counted and reported, not hidden.
//!
//! The DATA payloads of every domain other than 50 and 49 pass UNVALIDATED
//! beyond header sanity, the stream validator and the hour-boundary rules —
//! production validates every domain's payload (`stream_fold.rs:593`); this
//! harness owns only d50's grammar and its modelled D49 parent lane. That
//! unvalidated remainder is explicitly BOUNDED: every closed game hour must
//! carry exactly one d50 DATA record of any status, and the d49 lane must be
//! all-or-nothing — exactly one record per closed hour when scheduled, none at
//! all when not (the completeness checks in [`scan_stream`]) — so a d50 or d49
//! record relabelled to another domain id, or dropped, refuses the run instead
//! of silently shrinking it or silently hiding the parent-lifecycle writer.
//!
//! Outputs, for the compression step (`compress_arms.py` beside the run):
//!   ship_d50.bin   — the d50 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing
//!
//! It also prints the real churn surface twice — cadence-independent and along the
//! delta path — because one number cannot answer both questions.

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
use wire_format::d50;

const D49: u16 = 49;
const D50: u16 = 50;
const BLOCK_HOURS: usize = 48;

/// One usable d50 hour: its game hour, the folded member state at hour close, and
/// the byte span of the record that produced it (the shipping arm is those bytes
/// verbatim, header included).
#[derive(Debug)]
struct D50Hour {
    game_hour: i32,
    members: KeyedState,
    ship_span: Range<usize>,
}

/// The shape of the rows the capture actually carried, so the coverage caveats in
/// the deliverable are measured rather than assumed.
#[derive(Debug, Default)]
struct RowStats {
    rows: usize,
    deleted: usize,
    convoy: usize,
    /// Non-convoy rows with a clear `CSHIP_JOIN_VALIDATED` bit — the only shape on
    /// which the six join-derived fields map to `unknown`.
    unjoined_ship: usize,
    /// Of those, the ones whose state is `FEX_STATE_SUNK` (the lane admits no
    /// other).
    unjoined_ship_sunk: usize,
    /// Rows on which a member-owned gate was clear, i.e. the `unknown` path for
    /// that field was exercised.
    owner_unknown: usize,
    designed_max_strength_unknown: usize,
    state_unknown: usize,
    engagement_weight_unknown: usize,
    /// Rows on which a join-derived gate was clear while the join validated — the
    /// grammar admits it, the producer does not.
    joined_but_gate_clear: usize,
    /// `state_bitmask` code histogram, over rows that carry it.
    state_codes: BTreeMap<u32, usize>,
}

#[derive(Debug)]
struct StreamScan {
    hours: Vec<D50Hour>,
    /// d50 records whose payload status flags mark the domain-hour failed.
    failure_records: usize,
    /// d50 records refused by each of the fold's three whole-hour gates.
    orphan_records: usize,
    duplicate_identity_records: usize,
    retired_resurrection_records: usize,
    /// Measured d50 hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// Total member rows the shipping records carried, and their shape.
    stats: RowStats,
    /// D49 records, and each of its non-folding paths.
    d49_records: usize,
    d49_location_region_records: usize,
    d49_failure_records: usize,
    d49_side_scalar_records: usize,
    /// D49 records folded although a value-completeness gate would have masked the
    /// hour — non-zero only under `ParentPolicy::IdentityOnly`, and the exact
    /// count by which this run diverges from the recorder.
    d49_value_gate_overridden: usize,
    /// D49 tombstones, i.e. how often the cross-domain write into this domain's
    /// accumulator actually fired, and how many member keys it removed.
    ///
    /// Both are read from `d50::State`'s own PER-ROW counters at EOF, never
    /// derived from the parent set's size: one payload may insert and delete, and
    /// the net size change then hides both (a Codex round-2 finding).
    parent_deletes: u64,
    parent_cascade_members_dropped: u64,
    /// The persistent retired-key set at EOF, and the parent key set at EOF.
    retired_keys: usize,
    parent_keys: usize,
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

/// Accumulate the row-shape counters from one already-validated d50 payload.
///
/// This reads only through `d50`'s public row boundary, so it cannot disagree with
/// the mapper about what a row says.
fn tally_rows(payload: &[u8], stats: &mut RowStats) -> Result<(), String> {
    let (_, rows) = d50::split_payload(payload)?;
    for row in rows.chunks_exact(d50::ROW_LEN) {
        let parsed = d50::parse_row(row)?;
        stats.rows += 1;
        if parsed.deleted {
            stats.deleted += 1;
        }
        if parsed.is_convoy {
            stats.convoy += 1;
        } else if !parsed.cship_join_validated {
            stats.unjoined_ship += 1;
            if parsed.state_bitmask == Some(d50::FEX_STATE_SUNK) {
                stats.unjoined_ship_sunk += 1;
            }
        }
        if parsed.owner_tag_db_id.is_none() {
            stats.owner_unknown += 1;
        }
        if parsed.designed_max_strength_raw.is_none() {
            stats.designed_max_strength_unknown += 1;
        }
        match parsed.state_bitmask {
            None => stats.state_unknown += 1,
            Some(raw) => {
                let code = d50::state_code(raw).ok_or("validated row carries an unknown state")?;
                *stats.state_codes.entry(code).or_default() += 1;
            }
        }
        if parsed.engagement_weight_raw.is_none() {
            stats.engagement_weight_unknown += 1;
        }
        if parsed.cship_join_validated
            && (parsed.task_force_id.is_none()
                || parsed.fleet_id.is_none()
                || parsed.strength_raw.is_none()
                || parsed.organisation_raw.is_none()
                || parsed.experience_raw.is_none()
                || parsed.sunk_convoys.is_none())
        {
            stats.joined_but_gate_clear += 1;
        }
    }
    Ok(())
}

/// Validate the whole stream against the canonical contract and fold its usable
/// d50 DATA records into per-hour states. Every rejection path is an `Err`, never
/// a silently shorter measurement and never a panic — see the module docs for what
/// is enforced and what the recorder's own validator additionally checks.
fn scan_stream(
    schema: &wire_format::Schema,
    stream: &[u8],
    identity_mode: StreamIdentityMode,
    parent_policy: d50::ParentPolicy,
) -> Result<StreamScan, String> {
    let collection = &schema.collections[0];

    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    // COMPLETENESS accounting: every d50 DATA record of ANY status, against
    // every closed game hour — see the checks after the EOF rules.
    let mut d50_records = 0usize;
    let mut closed_hours = 0usize;
    let mut scan = StreamScan {
        hours: Vec::new(),
        failure_records: 0,
        orphan_records: 0,
        duplicate_identity_records: 0,
        retired_resurrection_records: 0,
        non_contiguous: 0,
        stats: RowStats::default(),
        d49_records: 0,
        d49_location_region_records: 0,
        d49_failure_records: 0,
        d49_side_scalar_records: 0,
        d49_value_gate_overridden: 0,
        parent_deletes: 0,
        parent_cascade_members_dropped: 0,
        retired_keys: 0,
        parent_keys: 0,
    };

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last DATA record of ANY status, per domain — the uniqueness rule.
    let mut last_d50_hour: Option<i32> = None;
    let mut last_d49_hour: Option<i32> = None;
    // Hour of the last d50 record that entered the measurement — the contiguity
    // report, which a skipped excluded hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;

    // The retained accumulator, its persistent retired-key set and the D49 parent
    // key set, all of which live across the whole scan.
    let mut state = d50::State::new();
    // Whether the hour currently open produced a measurable d50 state.
    let mut pending_hour: Option<D50Hour> = None;

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
                            "FRAME_END at byte {pos} closes hour {} without an open \
                             DOMAIN_DATA hour",
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

                // The hour's state is taken at hour close, after EVERY fold of the
                // hour — d49's included, because a lifecycle tombstone landing
                // after the member record still removes members from the state the
                // recorder would have held at snapshot time.
                if let Some(mut hour) = pending_hour.take() {
                    hour.members = state.members.clone();
                    scan.hours.push(hour);
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

        if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA && header.domain_id == D49 {
            // One D49 DATA record per game hour, over EVERY record and before any
            // exclusion: two in one hour would fold the parent set twice.
            if let Some(previous) = last_d49_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d49 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d49_hour = Some(header.game_hour_raw);
            scan.d49_records += 1;

            // VALIDITY BEFORE CLASSIFICATION, on the parent mirror as much as on
            // d50's own pass. The failure predicate classifies the v1 status
            // vocabulary and is not a validity check
            // (`compact_contract/src/lib.rs:121`-`:123`), so a status word carrying
            // a bit version 1 does not assign is refused before it runs — bits
            // 14..=31 are unassigned (`status_flags_base.rs:21`-`:24`) and an
            // unassigned bit "is not evidence of a future informational status".
            // Note bit 11 is assigned in v1 and is additionally D49's lane-local
            // location-candidate badge, which the failure predicate masks for
            // domain 49 (`compact_contract/src/lib.rs:93`-`:101`) and the fold
            // reads directly.
            let d49_header = d50::peek_header(payload, d50::LIFECYCLE_ROW_LEN)
                .map_err(|e| format!("d49 payload at byte {pos}: {e}"))?;
            let unknown_status = compact_status_unknown_flags_v1(d49_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d49 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }
            let failure = compact_status_failure_flags_after_expected_partial_tolerance(
                D49,
                d49_header.status_flags,
            ) != 0;
            let outcome = state
                .fold_lifecycle_payload(payload, failure, parent_policy)
                .map_err(|e| format!("d49 payload at byte {pos}: {e}"))?;
            match outcome {
                d50::LifecycleOutcome::Applied | d50::LifecycleOutcome::AppliedOverValueGate => {
                    if outcome == d50::LifecycleOutcome::AppliedOverValueGate {
                        scan.d49_value_gate_overridden += 1;
                    }
                }
                d50::LifecycleOutcome::LocationRegionIncomplete => {
                    scan.d49_location_region_records += 1
                }
                d50::LifecycleOutcome::FailureStatus => scan.d49_failure_records += 1,
                d50::LifecycleOutcome::IncompleteSideScalars => scan.d49_side_scalar_records += 1,
            }
        }

        if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA && header.domain_id == D50 {
            d50_records += 1;
            // One d50 DATA record per game hour, checked over EVERY d50 record and
            // BEFORE any exclusion: gating this on the record being usable would
            // let a success + failure pair through the rule entirely, and two
            // records in one hour would fold twice into a state measured once.
            if let Some(previous) = last_d50_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d50 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d50_hour = Some(header.game_hour_raw);

            let d50_header = d50::peek_header(payload, d50::ROW_LEN)
                .map_err(|e| format!("d50 payload at byte {pos}: {e}"))?;
            // VALIDITY BEFORE CLASSIFICATION — see the d49 arm above.
            let unknown_status = compact_status_unknown_flags_v1(d50_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d50 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            // Production's status-independent payload authority, replayed in full
            // and on every record — a malformed payload is a stream rejection even
            // under a failure status, because `decode_member_rows` runs before
            // `handle_compact_failure` (`world_fold.rs:20136`, `:20140`).
            tally_rows(payload, &mut scan.stats)
                .map_err(|e| format!("d50 payload at byte {pos}: {e}"))?;

            // `handle_compact_failure` (`world_fold.rs:20140`) marks the
            // domain-hour unknown and RETAINS the prior accumulator, folding
            // nothing from this record. The predicate is `compact_contract`'s own,
            // not a copied bit mask.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D50,
                d50_header.status_flags,
            ) != 0
            {
                scan.failure_records += 1;
                pos = record_end;
                continue;
            }

            match state
                .fold_payload(collection, payload)
                .map_err(|e| format!("d50 payload at byte {pos}: {e}"))?
            {
                d50::FoldOutcome::Applied => {}
                d50::FoldOutcome::OrphanMemberReference => {
                    scan.orphan_records += 1;
                    pos = record_end;
                    continue;
                }
                d50::FoldOutcome::DuplicateMemberIdentity => {
                    scan.duplicate_identity_records += 1;
                    pos = record_end;
                    continue;
                }
                d50::FoldOutcome::RetiredMemberResurrection => {
                    scan.retired_resurrection_records += 1;
                    pos = record_end;
                    continue;
                }
            }

            if let Some(previous) = last_measured_hour {
                if header.game_hour_raw != previous + 1 {
                    scan.non_contiguous += 1;
                }
            }
            last_measured_hour = Some(header.game_hour_raw);
            // The state itself is filled in at FRAME_END, at hour close.
            pending_hour = Some(D50Hour {
                game_hour: header.game_hour_raw,
                members: KeyedState::new(),
                ship_span: pos..record_end,
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
        return Err("stream carries no usable d50 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d50 DATA
    // record of any status, and the d49 lane must be all-or-nothing — exactly
    // one record per closed hour when scheduled, none at all when not. This
    // bounds what this harness canNOT validate: production runs
    // `validate_domain_data_payload_at` on EVERY domain's DATA payload
    // (`stream_fold.rs:593`), while this harness owns only d50's and d49's
    // grammars, so a record of either domain whose header was relabelled — or
    // lost outright — would otherwise be silently ignored: a lost d50 record
    // shrinks the run, and a lost d49 record silently hides the
    // parent-lifecycle writer of this domain's accumulator. A per-capture
    // completeness assertion, NOT a grammar rule (`bench_real_d65.rs`
    // precedent).
    if d50_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d50_records} d50 DATA \
             record(s); every closed hour of a d50-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of unmodelled domains to see where \
             the difference went"
        ));
    }
    if scan.d49_records != 0 && scan.d49_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {} d49 DATA record(s); a \
             capture scheduling d49 carries exactly one per closed hour, and a \
             partially-missing d49 lane would silently hide the parent-lifecycle writer of \
             this domain's accumulator",
            scan.d49_records
        ));
    }
    scan.retired_keys = state.retired().len();
    scan.parent_keys = state.parents().len();
    scan.parent_deletes = state.parent_deletes_applied();
    scan.parent_cascade_members_dropped = state.members_dropped_by_parent_delete();

    Ok(scan)
}

#[derive(Debug, Default)]
struct Measurement {
    ship: Vec<u8>,
    ship_offsets: Vec<usize>,
    wire: Vec<u8>,
    wire_offsets: Vec<usize>,
    hours_seen: usize,
    baselines: usize,
    /// Baselines the §1.5 gap rule forced outside a scheduled block start.
    gap_forced_baselines: usize,
    baseline_bytes: usize,
    delta_bytes: usize,
    delta_hours: usize,
    empty_hours: usize,
    state_rows_total: usize,
    max_state_rows: usize,
    /// Delta-path churn: changed rows and tombstones over the hours that emitted a
    /// delta.
    delta_changed_rows: usize,
    delta_tombstones: usize,
    /// Cadence-independent churn: changed rows over MEASURED-HOUR transitions,
    /// taken before the block decision, so a change landing on a block boundary
    /// still counts.
    real_changed_rows: usize,
    real_tombstones: usize,
    real_changed_hours: usize,
}

/// Build both arms, verifying the reader replay every hour.
///
/// Factored out of `main` so the block-cadence behaviour is testable: a change
/// placed on a block boundary is invisible from `main`.
fn measure(
    schema: &wire_format::Schema,
    collection: &wire_format::Collection,
    stream: &[u8],
    hours: &[D50Hour],
    block_hours: usize,
) -> Measurement {
    let mut m = Measurement {
        ship_offsets: vec![0usize],
        wire_offsets: vec![0usize],
        ..Measurement::default()
    };
    let mut prior: Option<&KeyedState> = None;
    let mut reader: Option<KeyedState> = None;
    let mut previous_game_hour: Option<i32> = None;

    for hour in hours {
        // Shipping arm: the record's exact bytes, header included.
        m.ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        m.ship_offsets.push(m.ship.len());

        // The hour's state is the ACCUMULATOR at hour close, folded during the
        // scan: d50 is pure cumulative delta, so the record's rows are not the
        // hour's state.
        let state: &KeyedState = &hour.members;
        m.state_rows_total += state.len();
        m.max_state_rows = m.max_state_rows.max(state.len());

        // Cadence-independent churn, taken BEFORE the block decision so a change
        // on a block boundary is counted like any other.
        if let Some(prev) = prior {
            let real = diff(collection, prev, state);
            if !real.is_empty() {
                m.real_changed_hours += 1;
                m.real_changed_rows += real.changed.len();
                m.real_tombstones += real.tombstones.len();
            }
        }

        // §1.5: "A gap affecting the state part forces the next state observation
        // to be a `BASELINE`." Every hour this domain did not successfully sample
        // is such a gap — the failure-status hour and each of the fold's three
        // whole-hour refusals — and d50's cadence is hourly, so a measured hour
        // that does not follow the previous one by exactly one is preceded by one.
        // State may have moved and returned while nobody was looking, so neither a
        // delta nor a retained reader state can be justified across it.
        let gap_before = previous_game_hour
            .is_some_and(|previous| hour.game_hour != previous.saturating_add(1));
        let hour_start = m.wire.len();
        let block_start = m.hours_seen % block_hours == 0;
        let baseline_here = block_start || gap_before || prior.is_none();
        let mut emitted = true;
        if baseline_here {
            if gap_before && !block_start {
                m.gap_forced_baselines += 1;
            }
            encode_baseline(schema, std::slice::from_ref(state), &mut m.wire)
                .expect("baseline encodes");
            m.baselines += 1;
            m.baseline_bytes += m.wire.len() - hour_start;
        } else {
            let prev = prior.unwrap();
            let d = diff(collection, prev, state);
            if d.is_empty() {
                m.empty_hours += 1;
                emitted = false;
            } else {
                m.delta_hours += 1;
                m.delta_changed_rows += d.changed.len();
                m.delta_tombstones += d.tombstones.len();
                encode_delta(schema, std::slice::from_ref(prev), &[Some(d)], &mut m.wire)
                    .expect("delta encodes");
                m.delta_bytes += m.wire.len() - hour_start;
            }
        }
        m.wire_offsets.push(m.wire.len());

        // Reader replay: decode the bytes just written against the reader's OWN
        // reconstructed prior, exactly as a conforming reader would, and require
        // the result to be stored-equal to this hour's state. This is what makes
        // the size claim a claim about decodable bytes. An hour that emitted
        // nothing is still checked: the reader's retained state must already equal
        // the fold's, which is precisely what "nothing changed" asserts.
        if emitted {
            let hour_bytes = &m.wire[hour_start..];
            if baseline_here || reader.is_none() {
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
                apply(&mut next, deltas[0].as_ref().expect("the one collection"));
                reader = Some(next);
            }
        }
        let replayed = reader.as_ref().expect("a reader state exists");
        assert_eq!(
            replayed.len(),
            state.len(),
            "game hour {}: row count",
            hour.game_hour
        );
        assert!(
            diff(collection, replayed, state).is_empty(),
            "game hour {}: reader state is not stored-equal to the fold",
            hour.game_hour
        );

        prior = Some(state);
        previous_game_hour = Some(hour.game_hour);
        m.hours_seen += 1;
    }
    m
}

fn main() {
    const USAGE: &str = "usage: bench_real_d50 <collector_stream.bin> <out dir> \
                         [legacy|identity-v1] [fold-faithful|parent-identity-only]";
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };
    // The default is the recorder's own policy: a run that forgets to choose
    // reports the truth, not the convenient number.
    let parent_policy = match args.next().as_deref() {
        None | Some("fold-faithful") => d50::ParentPolicy::FoldFaithful,
        Some("parent-identity-only") => d50::ParentPolicy::IdentityOnly,
        Some(other) => panic!("unknown parent policy {other:?}; {USAGE}"),
    };
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d50::schema();
    let collection = &schema.collections[0];

    // The one constant this crate's library half mirrors rather than imports
    // (`d50.rs`'s `MAX_OWNER_TAG_DB_ID`). This bin does depend on
    // `compact_contract`, so it is the natural place to prove the mirror right.
    assert_eq!(
        d50::MAX_OWNER_TAG_DB_ID,
        compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID,
        "d50's mirrored owner-tag bound must equal the shipping profile's"
    );

    // ---- Pass 1: validate the WHOLE stream and fold its d50 DATA records. ----
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result.
    let scan = match scan_stream(&schema, &stream, identity_mode, parent_policy) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: encode both arms. Nothing here reads stream content again. --
    let m = measure(&schema, collection, &stream, &scan.hours, BLOCK_HOURS);

    for (name, bytes) in [("ship_d50.bin", &m.ship), ("wire_new.bin", &m.wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d50.offsets", &m.ship_offsets),
        ("wire_new.offsets", &m.wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    let s = &scan.stats;
    if parent_policy == d50::ParentPolicy::IdentityOnly {
        println!(
            "*** SIZE-ACCOUNTING ARM: parent policy = identity-only. {} d49 record(s) were \
             folded although a value-completeness gate would have masked the hour. This is \
             NOT what the recorder does; see the module docs. ***",
            scan.d49_value_gate_overridden
        );
    }
    println!(
        "hours                {} ({} non-contiguous steps)",
        m.hours_seen, scan.non_contiguous
    );
    println!(
        "excluded hours       {} failure-status, {} orphan, {} duplicate-identity, \
         {} retired-resurrection",
        scan.failure_records,
        scan.orphan_records,
        scan.duplicate_identity_records,
        scan.retired_resurrection_records
    );
    println!(
        "d49 parent records   {} total: {} location/region-incomplete, {} failure-status, \
         {} side-scalars-incomplete, {} value-gate-overridden",
        scan.d49_records,
        scan.d49_location_region_records,
        scan.d49_failure_records,
        scan.d49_side_scalar_records,
        scan.d49_value_gate_overridden
    );
    println!(
        "d49 cross-domain     {} parent tombstones dropped {} member rows from this domain's \
         accumulator",
        scan.parent_deletes, scan.parent_cascade_members_dropped
    );
    println!(
        "shipping rows        {} member rows ({} deletes, {} convoy, {} unjoined ship of which \
         {} terminal-sunk)",
        s.rows, s.deleted, s.convoy, s.unjoined_ship, s.unjoined_ship_sunk
    );
    println!(
        "unknown coverage     owner {} rows, designed_max_strength {}, state {}, \
         engagement_weight {}, joined-but-gate-clear {}",
        s.owner_unknown,
        s.designed_max_strength_unknown,
        s.state_unknown,
        s.engagement_weight_unknown,
        s.joined_but_gate_clear
    );
    println!("state code histogram {:?}", s.state_codes);
    println!(
        "state size           {:.1} members/hour average, {} peak; {} retired keys and {} live \
         parents at EOF",
        m.state_rows_total as f64 / m.hours_seen.max(1) as f64,
        m.max_state_rows,
        scan.retired_keys,
        scan.parent_keys
    );
    println!(
        "baselines            {} ({} at a {BLOCK_HOURS}-h block start, {} forced by a \
         WIRE_FORMAT §1.5 state gap)",
        m.baselines,
        m.baselines - m.gap_forced_baselines,
        m.gap_forced_baselines
    );
    println!(
        "delta hours          {}   empty hours {}",
        m.delta_hours, m.empty_hours
    );
    // The denominator is every measured-hour TRANSITION, not only the ones that
    // changed: the recipe's cadence-independent figure asks how much this domain
    // moves per sampled hour, and conditioning it on "something moved" makes an
    // idle domain look busy (a Codex round-2 finding, and the same denominator
    // trap the d17 round-2 review named on the delta path).
    let transitions = m.hours_seen.saturating_sub(1);
    println!(
        "real churn           {} changed rows and {} tombstone(s) over {} measured-hour \
         transitions ({} of which changed anything); {:.4} rows/h, {:.4} tombstones/h \
         (cadence-independent, stored precision)",
        m.real_changed_rows,
        m.real_tombstones,
        transitions,
        m.real_changed_hours,
        m.real_changed_rows as f64 / transitions.max(1) as f64,
        m.real_tombstones as f64 / transitions.max(1) as f64
    );
    println!(
        "delta-path churn     {} changed rows and {} tombstone(s) over {} emitted-delta hours; \
         {:.4} rows/h, {:.4} tombstones/h ({} changed row(s) fell on a baseline instead)",
        m.delta_changed_rows,
        m.delta_tombstones,
        m.delta_hours,
        m.delta_changed_rows as f64 / m.delta_hours.max(1) as f64,
        m.delta_tombstones as f64 / m.delta_hours.max(1) as f64,
        m.real_changed_rows.saturating_sub(m.delta_changed_rows)
    );
    println!("ship_d50.bin         {} bytes", m.ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.1}%)",
        m.wire.len(),
        100.0 * m.wire.len() as f64 / m.ship.len() as f64
    );
    println!(
        "wire raw split       baseline {} bytes, delta {} bytes",
        m.baseline_bytes, m.delta_bytes
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{
        COMPACT_DOMAIN_FRAME, COMPACT_STATUS_DELTA_DATA, COMPACT_STATUS_PARTIAL_DATA,
    };

    fn schema() -> wire_format::Schema {
        d50::schema()
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
        let payload = vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN];
        record(
            sequence,
            game_hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &payload,
        )
    }

    fn naval_payload(row_len: u16, status_flags: u32, rows: &[Vec<u8>]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&2u16.to_le_bytes()); // schema version
        p.extend_from_slice(&row_len.to_le_bytes());
        p.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        p.extend_from_slice(&status_flags.to_le_bytes());
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        p.extend_from_slice(&1u32.to_le_bytes()); // string_count
        p.extend_from_slice(&0u32.to_le_bytes()); // reserved
        for r in rows {
            p.extend_from_slice(r);
        }
        p.extend_from_slice(&0u16.to_le_bytes());
        p
    }

    fn lifecycle_row(combat_id: u32, generation: u64, deleted: bool) -> Vec<u8> {
        let mut row = vec![0u8; d50::LIFECYCLE_ROW_LEN];
        row[0..4].copy_from_slice(&combat_id.to_le_bytes());
        row[4..12].copy_from_slice(&generation.to_le_bytes());
        if deleted {
            row[20..24].copy_from_slice(&1u32.to_le_bytes());
            return row;
        }
        // Every presence flag except DELETED: location, region, both counts, both
        // positioning cells and both anti-air cells.
        let flags: u32 = (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5) | (1 << 6)
            | (1 << 7)
            | (1 << 8);
        row[20..24].copy_from_slice(&flags.to_le_bytes());
        row[24..28].copy_from_slice(&4_242u32.to_le_bytes()); // location
        row[28..32].copy_from_slice(&77u32.to_le_bytes()); // region
        row[32..36].copy_from_slice(&1u32.to_le_bytes());
        row[36..40].copy_from_slice(&1u32.to_le_bytes());
        row[40..44].copy_from_slice(&50_000i32.to_le_bytes());
        row[44..48].copy_from_slice(&50_000i32.to_le_bytes());
        row[48..56].copy_from_slice(&1i64.to_le_bytes());
        row[56..64].copy_from_slice(&1i64.to_le_bytes());
        row
    }

    /// A live ship member row with a validated CShip join and every gate set.
    fn member_row(combat_id: u32, generation: u64, slot: u32, strength: u32) -> Vec<u8> {
        member_row_flagged(combat_id, generation, slot, strength, false)
    }

    fn member_row_flagged(
        combat_id: u32,
        generation: u64,
        slot: u32,
        strength: u32,
        deleted: bool,
    ) -> Vec<u8> {
        let mut flags = d50::SHIP_INSTANCE_PRESENT
            | d50::CSHIP_JOIN_VALIDATED
            | d50::OWNER_PRESENT
            | d50::DESIGNED_MAX_STRENGTH_PRESENT
            | d50::STATE_BITMASK_PRESENT
            | d50::ENGAGEMENT_WEIGHT_PRESENT
            | d50::TASK_FORCE_PRESENT
            | d50::FLEET_PRESENT
            | d50::STRENGTH_PRESENT
            | d50::ORGANISATION_PRESENT
            | d50::EXPERIENCE_PRESENT
            | d50::SUNK_CONVOYS_PRESENT;
        if deleted {
            flags |= d50::ROW_DELETED;
        }
        let mut row = vec![0u8; d50::ROW_LEN];
        row[0..4].copy_from_slice(&combat_id.to_le_bytes());
        row[4..12].copy_from_slice(&generation.to_le_bytes());
        row[12..16].copy_from_slice(&flags.to_le_bytes());
        row[16..20].copy_from_slice(&d50::SIDE_ATTACKER.to_le_bytes());
        row[20..24].copy_from_slice(&0u32.to_le_bytes()); // group_index
        row[24..28].copy_from_slice(&1u32.to_le_bytes()); // member_index
        row[28..32].copy_from_slice(&d50::CSHIP_TYPE_TAG.to_le_bytes());
        row[32..36].copy_from_slice(&4_544u32.to_le_bytes()); // ship_instance_id
        row[36..40].copy_from_slice(&368u32.to_le_bytes()); // owner
        row[40..44].copy_from_slice(&32_000_000u32.to_le_bytes()); // designed max
        row[44..48].copy_from_slice(&slot.to_le_bytes());
        row[48..52].copy_from_slice(&2u32.to_le_bytes()); // FEX_STATE_FIGHTING
        row[52..56].copy_from_slice(&300_000i32.to_le_bytes()); // engagement weight
        row[60..64].copy_from_slice(&17_617u32.to_le_bytes()); // task force
        row[64..68].copy_from_slice(&17_732u32.to_le_bytes()); // fleet
        row[68..72].copy_from_slice(&strength.to_le_bytes());
        row[72..76].copy_from_slice(&6_750_000u32.to_le_bytes()); // organisation
        row[76..80].copy_from_slice(&7u32.to_le_bytes()); // sunk convoys
        row[80..84].copy_from_slice(&10_000_000u32.to_le_bytes()); // experience
        row
    }

    fn hour(
        sequence: u64,
        game_hour: i32,
        lifecycle_rows: &[Vec<u8>],
        member_rows: &[Vec<u8>],
        d49_status: u32,
        d50_status: u32,
    ) -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            sequence,
            game_hour,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &naval_payload(d50::LIFECYCLE_ROW_LEN as u16, d49_status, lifecycle_rows),
        ));
        s.extend(record(
            sequence + 1,
            game_hour,
            D50,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &naval_payload(d50::ROW_LEN as u16, d50_status, member_rows),
        ));
        s.extend(frame_end(sequence + 2, game_hour));
        s
    }

    /// Two closed hours: hour 100 opens a combat with one member, hour 101 moves
    /// that member's strength.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(hour(
            1,
            100,
            &[lifecycle_row(7, 1, false)],
            &[member_row(7, 1, 11, 1_500_000)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        s.extend(hour(
            4,
            101,
            &[],
            &[member_row(7, 1, 11, 1_400_000)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        s
    }

    #[test]
    fn a_closed_hour_without_a_d50_record_refuses_the_stream() {
        // The cross-domain bound: this harness validates only d50 and d49
        // payloads, so a d50 record relabelled to another domain — or dropped —
        // would otherwise be silently ignored and the run would report a
        // plausible measurement over fewer hours. Hour 101 carries the d49
        // record and another domain's DATA record but no d50 record — every
        // hour is structurally well formed, so only the completeness check can
        // see it.
        let mut s = Vec::new();
        s.extend(hour(
            1,
            100,
            &[lifecycle_row(7, 1, false)],
            &[member_row(7, 1, 11, 1_500_000)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        s.extend(record(
            4,
            101,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &naval_payload(d50::LIFECYCLE_ROW_LEN as u16, COMPACT_STATUS_DELTA_DATA, &[]),
        ));
        s.extend(record(5, 101, 20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &[0u8; 8]));
        s.extend(frame_end(6, 101));
        let err = match scan_stream(
            &schema(),
            &s,
            StreamIdentityMode::Legacy,
            d50::ParentPolicy::FoldFaithful,
        ) {
            Err(err) => err,
            Ok(_) => panic!("a closed hour with no d50 record must refuse the stream"),
        };
        assert!(
            err.contains("every closed hour"),
            "the refusal names the completeness rule: {err}"
        );

        // And a partially-missing d49 lane is refused too: hour 101 carries a
        // d50 record but no d49 record at all.
        let mut s = Vec::new();
        s.extend(hour(
            1,
            100,
            &[lifecycle_row(7, 1, false)],
            &[member_row(7, 1, 11, 1_500_000)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        s.extend(record(
            4,
            101,
            D50,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &naval_payload(
                d50::ROW_LEN as u16,
                COMPACT_STATUS_DELTA_DATA,
                &[member_row(7, 1, 11, 1_400_000)],
            ),
        ));
        s.extend(frame_end(5, 101));
        let err = match scan_stream(
            &schema(),
            &s,
            StreamIdentityMode::Legacy,
            d50::ParentPolicy::FoldFaithful,
        ) {
            Err(err) => err,
            Ok(_) => panic!("a partially-missing d49 lane must refuse the stream"),
        };
        assert!(err.contains("d49"), "the refusal names the d49 bound: {err}");
    }

    #[test]
    fn a_well_formed_stream_yields_its_d50_hours() {
        let scan =
            scan_stream(&schema(), &good_stream(), StreamIdentityMode::Legacy, d50::ParentPolicy::FoldFaithful).expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].game_hour, 100);
        assert_eq!(scan.hours[1].game_hour, 101);
        assert_eq!(scan.hours[0].members.len(), 1);
        // Pure cumulative delta: hour 101's record carries one row and the state
        // is still the one member, updated — not replaced.
        assert_eq!(scan.hours[1].members.len(), 1);
        assert_eq!(scan.failure_records, 0);
        assert_eq!(scan.orphan_records, 0);
        assert_eq!(scan.d49_records, 2);
        assert_eq!(scan.stats.rows, 2);
        assert_eq!(scan.stats.convoy, 0);
        assert_eq!(scan.non_contiguous, 0);
        // And the two states differ, so the delta path is exercised.
        let m = measure(
            &schema(),
            &schema().collections[0],
            &good_stream(),
            &scan.hours,
            BLOCK_HOURS,
        );
        assert_eq!(m.baselines, 1);
        assert_eq!(m.delta_hours, 1);
        assert_eq!(m.real_changed_rows, 1);
        assert_eq!(m.delta_changed_rows, 1);
    }

    #[test]
    fn a_failure_status_hour_is_excluded_and_retains_the_accumulator() {
        let mut s = Vec::new();
        s.extend(hour(
            1,
            100,
            &[lifecycle_row(7, 1, false)],
            &[member_row(7, 1, 11, 1_500_000)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        // The capture's own `0x600` shape: PARTIAL_DATA over DELTA_DATA, with an
        // empty payload.
        s.extend(hour(
            4,
            101,
            &[],
            &[],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA | COMPACT_STATUS_PARTIAL_DATA,
        ));
        s.extend(hour(
            7,
            102,
            &[],
            &[member_row(7, 1, 11, 1_400_000)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        let scan = scan_stream(&schema(), &s, StreamIdentityMode::Legacy, d50::ParentPolicy::FoldFaithful).expect("accepted");
        assert_eq!(scan.failure_records, 1);
        assert_eq!(scan.hours.len(), 2, "the failed hour is not measured");
        assert_eq!(scan.non_contiguous, 1, "and the gap is reported");
        // The accumulator survived the excluded hour: hour 102 still holds the
        // member first seen in hour 100.
        assert_eq!(scan.hours[1].members.len(), 1);
    }

    #[test]
    fn each_fold_refusal_excludes_its_hour_rather_than_rejecting_the_stream() {
        // Orphan: a member whose combat D49 never announced.
        let mut orphan = Vec::new();
        orphan.extend(hour(
            1,
            100,
            &[lifecycle_row(7, 1, false)],
            &[member_row(7, 1, 11, 1_500_000)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        orphan.extend(hour(
            4,
            101,
            &[],
            &[member_row(999, 1, 11, 1_500_000)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        let scan = scan_stream(&schema(), &orphan, StreamIdentityMode::Legacy, d50::ParentPolicy::FoldFaithful).expect("accepted");
        assert_eq!(scan.orphan_records, 1);
        assert_eq!(scan.hours.len(), 1);

        // Duplicate identity: the same (combat, generation, side, slot) twice.
        let mut duplicate = Vec::new();
        duplicate.extend(hour(
            1,
            100,
            &[lifecycle_row(7, 1, false)],
            &[
                member_row(7, 1, 11, 1_500_000),
                member_row(7, 1, 11, 1_400_000),
            ],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        duplicate.extend(hour(
            4,
            101,
            &[],
            &[member_row(7, 1, 11, 1_500_000)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        let scan =
            scan_stream(&schema(), &duplicate, StreamIdentityMode::Legacy, d50::ParentPolicy::FoldFaithful).expect("accepted");
        assert_eq!(scan.duplicate_identity_records, 1);
        assert_eq!(scan.hours.len(), 1);

        // Resurrection: a key that a previous delete retired, live again.
        let mut resurrect = Vec::new();
        resurrect.extend(hour(
            1,
            100,
            &[lifecycle_row(7, 1, false)],
            &[member_row(7, 1, 11, 1_500_000)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        resurrect.extend(hour(
            4,
            101,
            &[],
            &[member_row_flagged(7, 1, 11, 1_500_000, true)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        resurrect.extend(hour(
            7,
            102,
            &[],
            &[member_row(7, 1, 11, 1_500_000)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        let scan =
            scan_stream(&schema(), &resurrect, StreamIdentityMode::Legacy, d50::ParentPolicy::FoldFaithful).expect("accepted");
        assert_eq!(scan.retired_resurrection_records, 1);
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.retired_keys, 1);
    }

    #[test]
    fn a_parent_tombstone_removes_this_domains_members() {
        // The cross-domain write this harness models rather than asserts inert.
        let mut s = Vec::new();
        s.extend(hour(
            1,
            100,
            &[lifecycle_row(7, 1, false)],
            &[member_row(7, 1, 11, 1_500_000)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        s.extend(hour(
            4,
            101,
            &[lifecycle_row(7, 1, true)],
            &[],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        let scan = scan_stream(&schema(), &s, StreamIdentityMode::Legacy, d50::ParentPolicy::FoldFaithful).expect("accepted");
        assert_eq!(scan.parent_deletes, 1);
        assert_eq!(scan.parent_cascade_members_dropped, 1);
        // And the count survives a payload that also inserts, where the parent
        // set's net size does not move at all.
        let mut mixed = Vec::new();
        mixed.extend(hour(
            1,
            100,
            &[lifecycle_row(7, 1, false)],
            &[member_row(7, 1, 11, 1_500_000)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        mixed.extend(hour(
            4,
            101,
            &[lifecycle_row(7, 1, true), lifecycle_row(8, 1, false)],
            &[],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        let mixed_scan = scan_stream(
            &schema(),
            &mixed,
            StreamIdentityMode::Legacy,
            d50::ParentPolicy::FoldFaithful,
        )
        .expect("accepted");
        assert_eq!(mixed_scan.parent_keys, 1, "the net parent count never moved");
        assert_eq!(mixed_scan.parent_deletes, 1, "but one tombstone did fire");
        assert_eq!(mixed_scan.parent_cascade_members_dropped, 1);
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.hours[0].members.len(), 1);
        assert_eq!(
            scan.hours[1].members.len(),
            0,
            "the member is gone even though the d50 record carried no delete row"
        );
        // And the wire arm reproduces that: the reader replay inside `measure`
        // asserts stored equality every hour, so a tombstone that never reached
        // the wire would fail there.
        let m = measure(
            &schema(),
            &schema().collections[0],
            &s,
            &scan.hours,
            BLOCK_HOURS,
        );
        assert_eq!(m.delta_tombstones, 1);
        assert_eq!(m.real_tombstones, 1);
    }

    #[test]
    fn the_d49_location_region_badge_excludes_that_hour_but_not_the_stream() {
        // The 3a capture's 72-hour case: the fold retains D49's accumulator, so
        // d50 keeps folding against the parent set it already holds.
        let mut s = Vec::new();
        s.extend(hour(
            1,
            100,
            &[lifecycle_row(7, 1, false)],
            &[member_row(7, 1, 11, 1_500_000)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        s.extend(hour(
            4,
            101,
            &[lifecycle_row(8, 1, false)],
            &[member_row(7, 1, 12, 1_500_000)],
            COMPACT_STATUS_DELTA_DATA | d50::D49_STATUS_LOCATION_CANDIDATE_UNVALIDATED,
            COMPACT_STATUS_DELTA_DATA,
        ));
        let scan = scan_stream(&schema(), &s, StreamIdentityMode::Legacy, d50::ParentPolicy::FoldFaithful).expect("accepted");
        assert_eq!(scan.d49_location_region_records, 1);
        assert_eq!(scan.parent_keys, 1, "combat 8 never entered the parent set");
        assert_eq!(scan.hours.len(), 2, "d50's own hour is still measured");
        assert_eq!(scan.hours[1].members.len(), 2);
    }

    /// A lifecycle row shaped like every live one in `runs/batch3/deepwar_a`: no
    /// promoted location, no promoted region.
    fn unpromoted_lifecycle_row(combat_id: u32, generation: u64) -> Vec<u8> {
        let mut row = lifecycle_row(combat_id, generation, false);
        let flags: u32 = (1 << 2) | (1 << 3) | (1 << 4) | (1 << 5) | (1 << 7) | (1 << 8);
        row[20..24].copy_from_slice(&flags.to_le_bytes());
        row[24..28].copy_from_slice(&0u32.to_le_bytes());
        row[28..32].copy_from_slice(&0u32.to_le_bytes());
        row
    }

    #[test]
    fn an_unpromoted_parent_row_orphans_the_whole_capture_under_the_recorders_policy() {
        // The measured shape of `runs/batch3/deepwar_a`, in miniature: D49 emits a
        // live row with no promoted location/region and badges the hour, so the
        // fold folds nothing, the parent set stays empty, and d50's own record is
        // refused as an orphan. This is the primary result on that capture, and it
        // has to be reproducible from a synthetic stream or the number is a
        // coincidence.
        let mut s = Vec::new();
        s.extend(hour(
            1,
            100,
            &[unpromoted_lifecycle_row(7, 1)],
            &[member_row(7, 1, 11, 1_500_000)],
            COMPACT_STATUS_DELTA_DATA | d50::D49_STATUS_LOCATION_CANDIDATE_UNVALIDATED,
            COMPACT_STATUS_DELTA_DATA,
        ));
        s.extend(hour(
            4,
            101,
            &[],
            &[member_row(7, 1, 11, 1_400_000)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        // An hour whose d50 record carries no row at all is not an orphan, so the
        // stream still has one measurable (empty) hour — exactly the shape the
        // real capture produced.
        s.extend(hour(
            7,
            102,
            &[],
            &[],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        let faithful = scan_stream(
            &schema(),
            &s,
            StreamIdentityMode::Legacy,
            d50::ParentPolicy::FoldFaithful,
        )
        .expect("accepted");
        assert_eq!(faithful.d49_location_region_records, 1);
        assert_eq!(faithful.orphan_records, 2);
        assert_eq!(faithful.d49_value_gate_overridden, 0);
        assert_eq!(faithful.hours.len(), 1);
        assert!(faithful.hours[0].members.is_empty());
        assert_eq!(faithful.parent_keys, 0);

        // The size-accounting arm admits the identity and the same stream becomes
        // measurable — with the divergence counted, not hidden.
        let relaxed = scan_stream(
            &schema(),
            &s,
            StreamIdentityMode::Legacy,
            d50::ParentPolicy::IdentityOnly,
        )
        .expect("accepted");
        assert_eq!(relaxed.d49_value_gate_overridden, 1);
        assert_eq!(relaxed.d49_location_region_records, 0);
        assert_eq!(relaxed.orphan_records, 0);
        assert_eq!(relaxed.hours.len(), 3);
        assert_eq!(relaxed.hours[0].members.len(), 1);
        assert_eq!(relaxed.parent_keys, 1);
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helpers classify
        // the v1 vocabulary and are not validity predicates, so an unassigned bit
        // must be rejected BEFORE classification — otherwise it silently reads as
        // "not a failure" and the hour is measured as healthy. Both status readers
        // in this bin are covered: d50's own pass and the D49 parent mirror.
        for bit in 15..32u32 {
            let status = 1u32 << bit;
            for extra in [0u32, 1, COMPACT_STATUS_DELTA_DATA] {
                let mut on_d50 = Vec::new();
                on_d50.extend(hour(
                    1,
                    100,
                    &[lifecycle_row(7, 1, false)],
                    &[member_row(7, 1, 11, 1_500_000)],
                    COMPACT_STATUS_DELTA_DATA,
                    status | extra,
                ));
                assert!(
                    scan_stream(&schema(), &on_d50, StreamIdentityMode::Legacy, d50::ParentPolicy::FoldFaithful).is_err(),
                    "d50 status bit {bit} (+{extra:#x}) must be refused"
                );
                let mut on_d49 = Vec::new();
                on_d49.extend(hour(
                    1,
                    100,
                    &[lifecycle_row(7, 1, false)],
                    &[member_row(7, 1, 11, 1_500_000)],
                    status | extra,
                    COMPACT_STATUS_DELTA_DATA,
                ));
                assert!(
                    scan_stream(&schema(), &on_d49, StreamIdentityMode::Legacy, d50::ParentPolicy::FoldFaithful).is_err(),
                    "d49 status bit {bit} (+{extra:#x}) must be refused"
                );
            }
        }
        // And every assigned informational bit is still accepted on both readers:
        // the check must reject unassigned bits and nothing else. Bit 11 is
        // D49's lane-local badge, so it is exercised through its own excluded-hour
        // path rather than as a plain informational bit.
        for informational in [1u32 << 4, 1 << 5, COMPACT_STATUS_DELTA_DATA, 1 << 13] {
            let mut s = Vec::new();
            s.extend(hour(
                1,
                100,
                &[lifecycle_row(7, 1, false)],
                &[member_row(7, 1, 11, 1_500_000)],
                informational,
                informational,
            ));
            let scan = scan_stream(&schema(), &s, StreamIdentityMode::Legacy, d50::ParentPolicy::FoldFaithful)
                .unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scan.hours.len(), 1, "bit {informational:#x}");
            assert_eq!(scan.failure_records, 0, "bit {informational:#x}");
        }
    }

    #[test]
    fn a_malformed_payload_refuses_the_stream_even_under_a_failure_status() {
        // `decode_member_rows` runs BEFORE `handle_compact_failure`
        // (`world_fold.rs:20136`, `:20140`), and production's payload authority is
        // status-independent, so a bad row is a stream rejection whatever the
        // status word says.
        let mut bad = member_row(7, 1, 11, 1_500_000);
        bad[56..60].copy_from_slice(&1u32.to_le_bytes()); // reserved != 0
        for status in [
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA | COMPACT_STATUS_PARTIAL_DATA,
        ] {
            let s = hour(
                1,
                100,
                &[lifecycle_row(7, 1, false)],
                &[bad.clone()],
                COMPACT_STATUS_DELTA_DATA,
                status,
            );
            assert!(
                scan_stream(&schema(), &s, StreamIdentityMode::Legacy, d50::ParentPolicy::FoldFaithful).is_err(),
                "status {status:#x}"
            );
        }
        // Same for the D49 parent payload.
        let mut bad_lifecycle = lifecycle_row(7, 1, false);
        bad_lifecycle[64..72].copy_from_slice(&1u64.to_le_bytes()); // reserved tail
        let s = hour(
            1,
            100,
            &[bad_lifecycle],
            &[member_row(7, 1, 11, 1_500_000)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        );
        assert!(scan_stream(&schema(), &s, StreamIdentityMode::Legacy, d50::ParentPolicy::FoldFaithful).is_err());
    }

    #[test]
    fn two_d50_records_in_one_hour_refuse_the_stream() {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D49,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &naval_payload(
                d50::LIFECYCLE_ROW_LEN as u16,
                COMPACT_STATUS_DELTA_DATA,
                &[lifecycle_row(7, 1, false)],
            ),
        ));
        s.extend(record(
            2,
            100,
            D50,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &naval_payload(
                d50::ROW_LEN as u16,
                COMPACT_STATUS_DELTA_DATA,
                &[member_row(7, 1, 11, 1_500_000)],
            ),
        ));
        s.extend(record(
            3,
            100,
            D50,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &naval_payload(
                d50::ROW_LEN as u16,
                COMPACT_STATUS_DELTA_DATA | COMPACT_STATUS_PARTIAL_DATA,
                &[],
            ),
        ));
        s.extend(frame_end(4, 100));
        assert!(
            scan_stream(&schema(), &s, StreamIdentityMode::Legacy, d50::ParentPolicy::FoldFaithful).is_err(),
            "hour uniqueness is checked over every record, failure status included"
        );
    }

    #[test]
    fn a_truncated_or_unclosed_stream_is_refused() {
        let good = good_stream();
        // Cut inside the last record.
        assert!(scan_stream(
            &schema(),
            &good[..good.len() - 4],
            StreamIdentityMode::Legacy,
            d50::ParentPolicy::FoldFaithful,
        )
            .is_err());
        // Cut exactly at a FRAME_END boundary is a well-formed shorter stream.
        let first_hour_len = {
            let mut pos = 0usize;
            for _ in 0..3 {
                let h = parse_compact_record_header(&good[pos..]).expect("framed");
                pos += COMPACT_RECORD_HEADER_LEN + h.payload_len;
            }
            pos
        };
        let scan = scan_stream(
            &schema(),
            &good[..first_hour_len],
            StreamIdentityMode::Legacy,
            d50::ParentPolicy::FoldFaithful,
        )
            .expect("a stream cut at a FRAME_END boundary is accepted");
        assert_eq!(scan.hours.len(), 1);
        // A stream whose last hour was never closed is refused.
        let mut unclosed = good[..first_hour_len].to_vec();
        unclosed.extend(record(
            4,
            101,
            D50,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &naval_payload(d50::ROW_LEN as u16, COMPACT_STATUS_DELTA_DATA, &[]),
        ));
        assert!(scan_stream(&schema(), &unclosed, StreamIdentityMode::Legacy, d50::ParentPolicy::FoldFaithful).is_err());
    }

    #[test]
    fn an_excluded_hour_forces_the_next_observation_to_be_a_baseline() {
        // `WIRE_FORMAT.md` §1.5: "A gap affecting the state part forces the next
        // state observation to be a `BASELINE`." Every excluded hour is such a
        // gap — state may have moved and returned while nobody was looking — so a
        // delta across it would be unjustifiable and a reader retaining its state
        // across it would be reconstructing from a prior it must not trust.
        let mut s = Vec::new();
        s.extend(hour(
            1,
            100,
            &[lifecycle_row(7, 1, false)],
            &[member_row(7, 1, 11, 1_500_000)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        // Excluded: a failure status.
        s.extend(hour(
            4,
            101,
            &[],
            &[],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA | COMPACT_STATUS_PARTIAL_DATA,
        ));
        s.extend(hour(
            7,
            102,
            &[],
            &[member_row(7, 1, 11, 1_400_000)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        // And a fourth, contiguous hour, which must NOT be forced.
        s.extend(hour(
            10,
            103,
            &[],
            &[member_row(7, 1, 11, 1_300_000)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        let scan = scan_stream(
            &schema(),
            &s,
            StreamIdentityMode::Legacy,
            d50::ParentPolicy::FoldFaithful,
        )
        .expect("accepted");
        assert_eq!(scan.hours.len(), 3);
        assert_eq!(scan.non_contiguous, 1);
        let m = measure(&schema(), &schema().collections[0], &s, &scan.hours, BLOCK_HOURS);
        assert_eq!(
            m.baselines, 2,
            "hour 100 opens the block; hour 102 is forced by the gap"
        );
        assert_eq!(m.gap_forced_baselines, 1);
        assert_eq!(m.delta_hours, 1, "only hour 103 takes the delta path");
        // The churn counters are unaffected: the gap rule decides the ENCODING,
        // not what changed.
        assert_eq!(m.real_changed_hours, 2);
        assert_eq!(m.real_changed_rows, 2);
        // A block start that is also a gap is counted once, as a block start.
        let m1 = measure(&schema(), &schema().collections[0], &s, &scan.hours, 1);
        assert_eq!(m1.baselines, 3);
        assert_eq!(m1.gap_forced_baselines, 0);
    }

    #[test]
    fn a_change_on_a_block_boundary_is_counted_by_the_cadence_independent_churn() {
        // The d16 round-3 finding, applied here: with a block size of 2, the
        // change into hour 101 lands on a baseline and emits no delta, so the
        // delta-path churn cannot see it. The cadence-independent number must.
        let s = good_stream();
        let scan = scan_stream(&schema(), &s, StreamIdentityMode::Legacy, d50::ParentPolicy::FoldFaithful).expect("accepted");
        let m = measure(&schema(), &schema().collections[0], &s, &scan.hours, 1);
        assert_eq!(m.baselines, 2, "every hour is a block start");
        assert_eq!(m.delta_hours, 0, "so nothing takes the delta path");
        assert_eq!(m.delta_changed_rows, 0);
        assert_eq!(
            m.real_changed_rows, 1,
            "but the change is still real and still counted"
        );
        assert_eq!(m.real_changed_hours, 1);
        // The cadence-independent denominator is every measured-hour TRANSITION,
        // not only the ones that changed: two measured hours are one transition,
        // and a rate conditioned on "something changed" would report the same
        // number for a domain that idles for a year and moves once.
        assert_eq!(m.hours_seen, 2);
        assert_eq!(m.hours_seen - 1, 1, "one transition");
    }

    #[test]
    fn the_cadence_independent_denominator_is_transitions_not_changed_hours() {
        // Four measured hours, one change. Transitions = 3, changed hours = 1.
        // Dividing by the latter reports 1.0 rows/h for a domain that moved once
        // in three hours; the recipe's figure is 0.333.
        let mut s = Vec::new();
        s.extend(hour(
            1,
            100,
            &[lifecycle_row(7, 1, false)],
            &[member_row(7, 1, 11, 1_500_000)],
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_DELTA_DATA,
        ));
        for (i, seq) in [(1u32, 4u64), (2, 7), (3, 10)] {
            let strength = if i >= 2 { 1_400_000 } else { 1_500_000 };
            s.extend(hour(
                seq,
                100 + i as i32,
                &[],
                &[member_row(7, 1, 11, strength)],
                COMPACT_STATUS_DELTA_DATA,
                COMPACT_STATUS_DELTA_DATA,
            ));
        }
        let scan = scan_stream(
            &schema(),
            &s,
            StreamIdentityMode::Legacy,
            d50::ParentPolicy::FoldFaithful,
        )
        .expect("accepted");
        assert_eq!(scan.hours.len(), 4);
        let m = measure(&schema(), &schema().collections[0], &s, &scan.hours, BLOCK_HOURS);
        assert_eq!(m.hours_seen, 4);
        assert_eq!(m.real_changed_hours, 1, "only hour 102 moved");
        assert_eq!(m.real_changed_rows, 1);
        let transitions = m.hours_seen - 1;
        assert_eq!(transitions, 3);
        assert!(
            (m.real_changed_rows as f64 / transitions as f64 - 1.0 / 3.0).abs() < 1e-12,
            "the reported rate divides by transitions"
        );
    }
}
