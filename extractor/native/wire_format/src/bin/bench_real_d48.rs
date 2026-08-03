//! Measure the wire format against the shipping compact stream on REAL d48
//! `construction_buildings_state_damage_live` dynamics.
//!
//! Input: a real `collector_stream.bin` from a d48-scheduled capture
//! (`runs/batch3/deepwar_a`, 514 hours). The stream's own d48 DATA records are
//! the shipping format's cost for the domain: a 64-byte `CBSD` header, three
//! fixed-length row tables and an `RLC1` railway trailer, one full baseline
//! followed by deltas, exactly what the recorder stores today. This tool replays
//! those records into the two accumulators
//! `world_fold::fold_construction_buildings_state_damage` maintains and
//! re-encodes every hour with this crate's grammar: a BASELINE at each 48-hour
//! block start, field-granular stored-minimal DELTAs between.
//!
//! **Five collections, and the grammar lives in `wire_format::d48`.** Nothing
//! here re-derives a contract it can call: the payload header, the row grammar,
//! the lane gate, the three-way accumulator seed and the RLC1 full replacement
//! all go through `d48`'s public boundary; H4CR framing, record semantics and
//! sequence continuity go through `compact_contract`'s own parser and stateful
//! validator; the failure classification and the expected-partial tolerance go
//! through `compact_contract`'s status predicates rather than a copied bit mask.
//! A harness that reimplements any of them drifts from the recorder without
//! anything failing.
//!
//! **The whole stream is validated before any artifact is written**, fail-closed:
//!
//! - every record header through `compact_contract::parse_compact_record_header`;
//! - every record through `compact_contract::stream_validator::StatefulStreamValidator`;
//! - the hour boundary rules of `recorder_host::stream_fold::BoundaryTracker`
//!   (`recorder_host/src/stream_fold.rs:277`), including DOMAIN_PERF hour
//!   binding (`:328`-`:365`) and the DATA arm's pending-PERF rule (`:378`-`:386`);
//! - FRAME_END seal fields (`:46`-`:89`, `:401`-`:409`) and the seal's
//!   domain-count cross-check against distinct non-probe PERF domains
//!   (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`);
//! - and the status-independent d48 authority
//!   (`domain_data_authority.rs:7958`-`:8050`), on EVERY d48 DATA record of
//!   every status.
//!
//! `wire_format` must not depend on `recorder_host`, so this implements that
//! documented subset itself.
//!
//! The DATA payloads of every domain other than 48 pass UNVALIDATED beyond
//! header sanity, the stream validator and the hour-boundary rules —
//! production validates every domain's payload (`stream_fold.rs:593`); this
//! harness owns only d48's grammar. That unvalidated remainder is explicitly
//! BOUNDED: every closed game hour must carry exactly one d48 DATA record of
//! any status (the `d48_records == closed_hours` completeness check in
//! [`scan_stream`]), so a d48 record relabelled to another domain id — or
//! dropped — refuses the run instead of silently shrinking it.
//!
//! # Which hours are measured, and which of the three outcomes each path is
//!
//! `fold_construction_buildings_state_damage` has four non-folding paths, in
//! this order (`world_fold.rs:20226`-`:20294`):
//!
//! 1. **decode failure** (`:20226`-`:20237`) — the fold returns `Err`. This is a
//!    **stream rejection**, not an hour to skip, and it comes BEFORE the status
//!    check, so a malformed failure-status payload refuses the run rather than
//!    being quietly excluded. That is the d83 order, not d63's, and copying
//!    d63's would have hidden exactly those records. The production authority
//!    agrees: it decodes every d48 DATA payload regardless of status.
//! 2. **failure status** (`:20240`-`:20261`) — an **excluded hour**, classified
//!    by `compact_contract`'s own tolerance-aware predicate.
//! 3. **failed lane gate** (`:20262`-`:20275`) — the fold alone would make this
//!    an excluded hour: it returns `Ok(())`, folds nothing, and the accumulators
//!    stand. But the **payload authority is stricter**, and the harness has to
//!    satisfy both. A failed gate under a NON-failure status is a parse error
//!    (`domain_data_authority.rs:7983`), and a non-empty `parse_errors` makes
//!    the stream not decodable in production (`main.rs:16721`, `:17790`), so it
//!    is a **stream rejection** here. Under a failure status the authority
//!    records nothing and the hour is simply excluded by (2). The fold's own
//!    gate-exclusion branch is therefore unreachable on any stream this harness
//!    accepts; it is kept, and counted, so the fold's order stays visible.
//! 4. **delta before any baseline** (`:20278`-`:20294`) — an **excluded hour**.
//!
//! Excluded hours are excluded from BOTH arms — they are not d48 data. Hour
//! uniqueness is enforced over EVERY d48 DATA record, of every status, BEFORE
//! any exclusion, or a success+failure pair in one hour slips through. An
//! unassigned v1 status bit refuses the stream BEFORE classification: the
//! failure predicate classifies the *assigned* vocabulary only
//! (`compact_contract/src/status_flags_base.rs:21`-`:24`), so an unassigned bit
//! has no defined classification and is not evidence of a future informational
//! status. A stream cut exactly at a FRAME_END boundary is a well-formed shorter
//! stream and is accepted as such.
//!
//! # No other writer touches the d48 accumulator
//!
//! Recipe step 9 requires every other writer of the domain's accumulator to be
//! modelled, proved inert, or asserted inert per capture. For d48 the
//! enumeration is empty and that is a checkable fact, not an assumption:
//! `domain48_state` and `domain48_delta_state` are assigned at exactly two
//! places in the whole recorder — `world_fold.rs:20596` and `:20598`, both
//! inside `fold_construction_buildings_state_damage` — and initialised at
//! `:2336`/`:2339`. The only other d48 state the recorder touches is the
//! per-hour `domain48_known_this_hour` / `domain48_unknown_this_hour` pair
//! (`:3032`, `:5084`), which gates the JSON projection and never the
//! accumulator. There is no cross-domain prune of the d26/d20 kind.
//!
//! Outputs, for the compression step (`scripts/compress_arms.py`):
//!   ship_d48.bin   — the d48 DATA records verbatim (32-byte H4CR header +
//!                    payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line

use std::collections::BTreeSet;
use std::io::Write;
use std::ops::Range;

use compact_contract::stream_validator::{StatefulStreamValidator, StreamIdentityMode};
use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, compact_status_unknown_flags_v1,
    expected_partial_tolerance_applies, parse_compact_record_header, COMPACT_FRAME_END_PAYLOAD_LEN,
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_STATUS_DELTA_DATA, COMPACT_STATUS_FAILURE_MASK,
};
use wire_format::body::{
    apply, decode_baseline, decode_delta, diff, encode_baseline, encode_delta, KeyedDelta,
    KeyedState,
};
use wire_format::d48::{self, Domain48Fold, FrameMode};

const D48: u16 = 48;
const BLOCK_HOURS: usize = 48;
// Re-exported, not re-declared: a local `(1 << 14) - 1` became wrong the moment
// the contract assigned bit 14 to the measured-empty badge.
const FRAME_END_KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;
const PROBE_DOMAIN_ID: u16 = 250;

/// Every constant `d48.rs` restates from `compact_contract::build_profile` and
/// from the lane, checked against its source so the two copies cannot drift.
/// `wire_format`'s library half deliberately carries neither dependency (d24's
/// and d32's convention); the measurement bin does, so this is where the drift
/// guard belongs.
fn assert_restated_constants_match_their_sources() {
    use compact_contract::build_profile as bp;
    assert_eq!(d48::SCOPE_STATE, bp::SCOPE_STATE);
    assert_eq!(d48::SCOPE_PROVINCE, bp::SCOPE_PROVINCE);
    assert_eq!(d48::MAX_STATE_ID, bp::MAX_STATE_ID);
    assert_eq!(d48::MAX_PROVINCE_ID, bp::MAX_PROVINCE_ID);
    assert_eq!(
        d48::MAX_COMPACT_OWNER_TAG_DB_ID,
        bp::MAX_COMPACT_OWNER_TAG_DB_ID
    );
    assert_eq!(d48::BUILDING_LEVEL_SANE_MAX, bp::BUILDING_LEVEL_SANE_MAX);
    assert_eq!(
        d48::MAX_CONSTRUCTION_TEMPLATE_ID,
        bp::MAX_CONSTRUCTION_TEMPLATE_ID
    );
    assert_eq!(d48::MAX_ACTIVE_FACTORIES, bp::MAX_ACTIVE_FACTORIES);
    assert_eq!(
        d48::STATE_RESOURCE_MAX_ABS_RAW,
        bp::STATE_RESOURCE_MAX_ABS_RAW
    );
    assert_eq!(d48::STATE_RESOURCE_NO_ID, bp::STATE_RESOURCE_NO_ID);
    assert_eq!(
        d48::CBUILDING_PARTIAL_HEALTH_RAW_PER_LEVEL,
        bp::CBUILDING_PARTIAL_HEALTH_RAW_PER_LEVEL
    );
    // The two payload status bits `d48::gate_frame` reads directly.
    assert_eq!(
        1u32 << 9,
        compact_contract::COMPACT_STATUS_PARTIAL_DATA,
        "gate_frame's PARTIAL_DATA bit"
    );
    assert_eq!(
        1u32 << 10,
        COMPACT_STATUS_DELTA_DATA,
        "gate_frame's DELTA_DATA bit"
    );
    // The lane gate's two blocking bits (`lib.rs:21`-`:22`, `:1415`).
    assert_eq!(1u32 << 6, compact_contract::COMPACT_STATUS_WORKER_PANIC);
    assert_eq!(1u32 << 8, compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE);
    // Domain 48 has no expected-partial tolerance registered
    // (`compact_contract/src/lib.rs:136`-`:158`), which is what makes the
    // fold's partial seed unreachable on any stream this harness can be given.
    // Asserted rather than assumed: if a tolerance is ever added, this fires
    // and the coverage caveat below has to be rewritten.
    assert!(
        compact_contract::expected_partial_tolerance(D48).is_none(),
        "domain 48 gained an expected-partial tolerance; the partial seed is now reachable and \
         this harness's coverage caveat is stale"
    );
}

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

/// One usable d48 hour: its game hour, its byte span in the source stream (the
/// shipping arm is those bytes verbatim, header included) and the payload span
/// plus mode the encode pass re-folds from.
///
/// The state is deliberately NOT retained here: d48's accumulator runs to
/// ~10.1k rows across the five collections and holding 514 hours of it — twice,
/// since the fold keeps a projected state and a delta baseline — would cost
/// gigabytes. The scan folds
/// every hour in full, so every way a row can be rejected is exercised in the
/// pass that either accepts the stream or refuses it; the encode pass then
/// re-folds through the identical code path, which cannot fail where the scan
/// did not.
#[derive(Debug)]
struct D48Hour {
    game_hour: i32,
    ship_span: Range<usize>,
    payload_span: Range<usize>,
    mode: FrameMode,
    /// True when this measured hour is exactly one game hour after the previous
    /// MEASURED hour. False on the first measured hour and after any gap — an
    /// excluded hour (failure-status, gate-failed, delta-before-baseline) or a
    /// non-contiguous hour step — which is what forces the encoder to open a
    /// fresh baseline (`opens_baseline`).
    follows_previous: bool,
}

/// Whether the wire arm opens a BASELINE at this measured hour.
///
/// Two reasons, and both are `WIRE_FORMAT.md` rules, not conveniences:
/// the 48-hour block cadence (every block opens with its own baseline, §1.5's
/// closing paragraph), and a gap — `WIRE_FORMAT.md:187`-`:190`: "A gap
/// affecting the state part forces the next state observation to be a
/// `BASELINE`." Every hour this domain did not successfully sample is such a
/// gap — each of the fold's excluded-hour paths, and any non-contiguous hour
/// step — and d48's cadence is hourly, so a measured hour that does not follow
/// the previous measured one by exactly one is preceded by one. Neither a
/// delta nor a retained reader state can be justified across it.
fn opens_baseline(hours_seen: usize, follows_previous: bool) -> bool {
    hours_seen % BLOCK_HOURS == 0 || !follows_previous
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
    bytes: usize,
    changed_by_collection: Vec<usize>,
    tombstones_by_collection: Vec<usize>,
}

#[allow(clippy::too_many_arguments)]
fn encode_wire_hour(
    schema: &wire_format::Schema,
    hours_seen: usize,
    follows_previous: bool,
    game_hour: i32,
    prior: Option<&[KeyedState]>,
    states: &[KeyedState],
    reader: &mut Option<Vec<KeyedState>>,
    wire: &mut Vec<u8>,
) -> WireHourOutcome {
    let hour_start = wire.len();
    let block_start = opens_baseline(hours_seen, follows_previous);
    let gap_forced = block_start && hours_seen % BLOCK_HOURS != 0;
    let mut emitted = true;
    let mut changed_by_collection = vec![0usize; schema.collections.len()];
    let mut tombstones_by_collection = vec![0usize; schema.collections.len()];
    let opened_baseline = block_start || prior.is_none();
    if opened_baseline {
        encode_baseline(schema, states, wire).expect("baseline encodes");
    } else {
        let previous = prior.expect("prior exists");
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
            emitted = false;
        } else {
            for (i, delta) in deltas.iter().enumerate() {
                if let Some(d) = delta {
                    changed_by_collection[i] = d.changed.len();
                    tombstones_by_collection[i] = d.tombstones.len();
                }
            }
            encode_delta(schema, previous, &deltas, wire).expect("delta encodes");
        }
    }
    let bytes = wire.len() - hour_start;

    // Reader replay: decode the bytes just written against the reader's OWN
    // reconstructed prior, exactly as a conforming reader would, and require
    // the result to be stored-equal to this hour's state. An hour that
    // emitted nothing is still checked: the reader's retained state must
    // already equal the fold's, which is precisely what "nothing changed"
    // asserts.
    if emitted {
        let hour_bytes = &wire[hour_start..];
        if block_start || reader.is_none() {
            let (decoded, consumed) =
                decode_baseline(schema, hour_bytes, 0).expect("baseline decodes");
            assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
            *reader = Some(decoded);
        } else {
            let mut reader_state = reader.take().expect("reader exists");
            let (deltas, consumed) =
                decode_delta(schema, &reader_state, hour_bytes, 0).expect("delta decodes");
            assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
            for (i, delta) in deltas.iter().enumerate() {
                if let Some(d) = delta {
                    apply(&mut reader_state[i], d);
                }
            }
            *reader = Some(reader_state);
        }
    }
    let replayed = reader.as_ref().expect("reader exists");
    for (i, collection) in schema.collections.iter().enumerate() {
        assert_eq!(
            replayed[i].len(),
            states[i].len(),
            "game hour {game_hour}: collection {} row count",
            collection.collection_id
        );
        assert!(
            diff(collection, &replayed[i], &states[i]).is_empty(),
            "game hour {game_hour}: collection {} reader state is not stored-equal to the fold",
            collection.collection_id
        );
    }
    WireHourOutcome {
        opened_baseline,
        gap_forced,
        emitted,
        bytes,
        changed_by_collection,
        tombstones_by_collection,
    }
}

#[derive(Debug, Default)]
struct ScanCounts {
    /// d48 records whose payload status marks the domain-hour failed.
    failure_records: usize,
    /// d48 records the lane gate refused (`world_fold.rs:20262`).
    gate_failed: usize,
    /// Delta records that arrived before a complete baseline.
    delta_before_baseline: usize,
    /// Records the tolerance table classified as an expected partial. Always
    /// zero for d48 today — see `assert_restated_constants_match_their_sources`.
    expected_partial_records: usize,
    /// Measured hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// Measured frames carrying no `RLC1` trailer at all (a legacy payload).
    frames_without_trailer: usize,
    /// Measured frames badging the construction target-state join unavailable.
    target_state_unavailable_frames: usize,
    /// Measured frames carrying at least one construction tombstone.
    frames_with_construction_tombstones: usize,
    frames_with_building_tombstones: usize,
    frames_with_resource_tombstones: usize,
    /// Live construction rows with no `template_id` — the recorded Null/Unknown
    /// tie-break, counted so the report can say how often it is exercised.
    construction_rows_without_template: usize,
    /// Live building rows with no `healthy_level` / `partial_health_raw`.
    building_rows_without_healthy_level: usize,
    building_rows_without_partial_health: usize,
    /// Railway rows carrying the `-1` no-current-edge sentinel.
    railway_rows_with_null_current_segment: usize,
    /// Total RLC1 rows and route provinces seen across measured frames.
    railway_rows: usize,
    railway_route_provinces: usize,
}

#[derive(Debug)]
struct StreamScan {
    hours: Vec<D48Hour>,
    counts: ScanCounts,
}

/// Production's status-independent d48 authority
/// (`domain_data_authority.rs:7958`-`:8050`), rule for rule: decode every
/// payload whatever its status, and treat a failed lane gate as a **parse
/// error** when — and only when — the status is not itself a failure
/// (`:7983`-`:7989`).
///
/// **A parse error is a stream rejection, not an excluded hour.** In production
/// a non-empty `parse_errors` makes the stream not decodable and not
/// framing-intact (`main.rs:16721`, `:17790`-`:17792`), so measuring such a
/// stream would report a plausible number for a capture the recorder itself
/// would flag. That is why this refuses rather than skipping the hour, even
/// though `fold_construction_buildings_state_damage` alone would only mark the
/// domain-hour unknown (`world_fold.rs:20262`) — the fold decides what folds,
/// the authority decides whether the stream is measurable at all, and the
/// harness has to satisfy both. This is the same rule d26's harness applies to
/// its own authority, where even an EXCLUDED record is still checked.
///
/// The consequence, stated because it makes a branch below unreachable: a
/// record that survives this check either carries a failure status — in which
/// case the fold excludes it before ever reaching its gate check (`:20248`
/// precedes `:20262`) — or has a passing gate. So the fold's gate-exclusion
/// path is reachable only on a stream this function has already refused.
fn check_authority_payload(payload: &[u8], at_byte: usize) -> Result<d48::Frame, String> {
    let frame =
        d48::decode_frame(payload).map_err(|e| format!("d48 payload at byte {at_byte}: {e}"))?;
    // `compact_status_is_failure` is the raw-mask predicate the authority uses,
    // not the tolerance-aware one. For domain 48 the two agree (no registered
    // tolerance), and using the authority's own predicate is what keeps this a
    // mirror rather than a re-derivation.
    if !compact_contract::compact_status_is_failure(frame.header.status_flags) {
        let gate = d48::gate_frame(&frame);
        if !gate.ok {
            return Err(format!(
                "d48 payload at byte {at_byte}: lane gate failed under a non-failure status \
                 ({:#x}), which the production payload authority records as a parse error \
                 (domain_data_authority.rs:7983): {}",
                frame.header.status_flags,
                gate.errors.join("; ")
            ));
        }
    }
    Ok(frame)
}

fn frame_mode(status_flags: u32) -> FrameMode {
    FrameMode {
        delta: status_flags & COMPACT_STATUS_DELTA_DATA != 0,
        expected_partial: expected_partial_tolerance_applies(D48, status_flags).is_some(),
    }
}

/// Validate the whole stream against the canonical contract and fold every
/// usable d48 DATA record. Every rejection path is an `Err`, never a silently
/// shorter measurement and never a panic.
fn scan_stream(stream: &[u8], identity_mode: StreamIdentityMode) -> Result<StreamScan, String> {
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let mut hours: Vec<D48Hour> = Vec::new();
    let mut counts = ScanCounts::default();
    // COMPLETENESS accounting: every d48 DATA record of ANY status, against
    // every closed game hour — see the check after the EOF rules.
    let mut d48_records = 0usize;
    let mut closed_hours = 0usize;

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // Hour of the last d48 DATA record of ANY status — the uniqueness rule.
    let mut last_d48_hour: Option<i32> = None;
    // Hour of the last d48 record that entered the measurement — the contiguity
    // report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;
    // `delta_baselines_seen` for d48 (`world_fold.rs:20601`).
    let mut baseline_seen = false;
    let mut fold = Domain48Fold::new();

    let mut pos = 0usize;
    while pos < stream.len() {
        let header = parse_compact_record_header(&stream[pos..])
            .map_err(|e| format!("record at byte {pos}: {e:?}"))?;
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

        if header.domain_id == D48 && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            d48_records += 1;
            // One d48 DATA record per game hour, checked over EVERY d48 record
            // and BEFORE any status-based exclusion.
            if let Some(previous) = last_d48_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "d48 DATA at byte {pos} carries hour {} after hour {previous}",
                        header.game_hour_raw
                    ));
                }
            }
            last_d48_hour = Some(header.game_hour_raw);

            // VALIDITY BEFORE CLASSIFICATION. `compact_contract/src/lib.rs:121`:
            // "Callers handling untrusted status words must first reject a
            // non-zero `compact_status_unknown_flags_v1` result; otherwise an
            // unassigned bit has no defined classification." d48 registers no
            // lane-local status mask (`lib.rs:95`-`:101` covers 49 and 51 only),
            // so bits 15..=31 are unassigned here with no exceptions.
            //
            // The status word lives in the PAYLOAD, so it needs the header read
            // first — but not the body, which is what `peek_header` is for.
            let d48_header = d48::peek_header(payload)
                .map_err(|e| format!("d48 payload at byte {pos}: {e}"))?;
            let unknown_status = compact_status_unknown_flags_v1(d48_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "d48 DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    header.game_hour_raw
                ));
            }

            // The fold decodes FIRST and returns `Err` on failure, so this is a
            // stream rejection and it happens before the status is classified.
            let frame = check_authority_payload(payload, pos)?;
            if frame.header != d48_header {
                return Err(format!(
                    "d48 payload at byte {pos}: peek_header and decode_frame disagree"
                ));
            }

            let mode = frame_mode(d48_header.status_flags);
            if mode.expected_partial {
                counts.expected_partial_records += 1;
            }

            // The fold's four non-folding paths, in the fold's own order.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                D48,
                d48_header.status_flags,
            ) != 0
            {
                counts.failure_records += 1;
            } else if !d48::gate_frame(&frame).ok {
                // Kept because the fold has this path in this position, and
                // dropping it would hide the fold's own order. It is
                // UNREACHABLE on any stream that got here: a non-failure status
                // with a failed gate was already refused by
                // `check_authority_payload`, and a failure status is caught by
                // the arm above — which is exactly the fold's order too
                // (`world_fold.rs:20248` precedes `:20262`). The count is
                // reported so a future relaxation of the authority check shows
                // up as a number rather than as silence.
                counts.gate_failed += 1;
            } else if mode.delta && !baseline_seen {
                counts.delta_before_baseline += 1;
            } else {
                let follows_previous =
                    last_measured_hour.is_some_and(|previous| header.game_hour_raw == previous + 1);
                if last_measured_hour.is_some() && !follows_previous {
                    counts.non_contiguous += 1;
                }
                last_measured_hour = Some(header.game_hour_raw);

                // Coverage bookkeeping, over the frames that actually fold.
                if frame.railway.is_none() {
                    counts.frames_without_trailer += 1;
                }
                if frame.header.target_state_unavailable() {
                    counts.target_state_unavailable_frames += 1;
                }
                if frame.construction.iter().any(|r| r.deleted) {
                    counts.frames_with_construction_tombstones += 1;
                }
                if frame.buildings.iter().any(|r| r.deleted) {
                    counts.frames_with_building_tombstones += 1;
                }
                if frame.resources.iter().any(|r| r.deleted) {
                    counts.frames_with_resource_tombstones += 1;
                }
                counts.construction_rows_without_template += frame
                    .construction
                    .iter()
                    .filter(|r| !r.deleted && r.template_id.is_none())
                    .count();
                counts.building_rows_without_healthy_level += frame
                    .buildings
                    .iter()
                    .filter(|r| !r.deleted && r.healthy_level.is_none())
                    .count();
                counts.building_rows_without_partial_health += frame
                    .buildings
                    .iter()
                    .filter(|r| !r.deleted && r.partial_health_raw.is_none())
                    .count();
                if let Some(rows) = frame.railway.as_ref() {
                    counts.railway_rows += rows.len();
                    counts.railway_route_provinces +=
                        rows.iter().map(|r| r.route_province_ids.len()).sum::<usize>();
                    counts.railway_rows_with_null_current_segment +=
                        rows.iter().filter(|r| r.current_segment_index < 0).count();
                }

                // PROJECT -> CHECK -> COMMIT, never apply-then-check. The only
                // way `to_states` can fail on stream content is a state that has
                // never observed an RLC1 trailer
                // (`d48::Domain48State::to_states`), so pass 1 proves that here
                // rather than discovering it in the encode pass; every other
                // value it builds comes from bytes `decode_frame` already
                // accepted. Checking it AFTER committing would advance both
                // accumulators and only then refuse the stream — the same
                // ordering defect `d48::fold_payload` was fixed for, and the
                // reason `project`/`commit` are separate public halves at all.
                // `scan_stream` returning `Err` aborts the run, so no caller
                // observes the difference today; the ordering is still wrong to
                // write, because it makes the accumulator's state depend on a
                // failure path.
                let state = fold
                    .project(&frame, mode)
                    .map_err(|e| format!("d48 payload at byte {pos}: {e}"))?;
                if state.railway.is_none() {
                    return Err(format!(
                        "d48 payload at byte {pos}: hour {} folds to a state that has never \
                         observed an RLC1 trailer; a keyed set cannot express an unmeasured \
                         collection",
                        header.game_hour_raw
                    ));
                }
                fold.commit(state, mode);
                if !mode.delta && !mode.expected_partial {
                    baseline_seen = true;
                }

                hours.push(D48Hour {
                    game_hour: header.game_hour_raw,
                    ship_span: pos..record_end,
                    payload_span: payload_start..record_end,
                    mode,
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
        return Err("stream carries no usable d48 DATA records".to_string());
    }

    // COMPLETENESS: every closed game hour must carry exactly one d48 DATA
    // record, of any status. This bounds what this harness canNOT validate:
    // production runs `validate_domain_data_payload_at` on EVERY domain's DATA
    // payload (`stream_fold.rs:593`), while this harness owns only d48's
    // grammar, so a d48 record whose header was relabelled to another domain
    // id — or lost outright — would otherwise be silently ignored and the run
    // would report a plausible measurement over fewer hours. A per-capture
    // completeness assertion, NOT a grammar rule: a capture where d48 is not
    // scheduled every hour fails here loudly and a human adjusts, instead of a
    // number quietly shrinking (`bench_real_d65.rs` precedent).
    if d48_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {d48_records} d48 DATA \
             record(s); every closed hour of a d48-scheduled capture must carry exactly one, \
             and this harness cannot validate the payloads of other domains to see where the \
             difference went"
        ));
    }

    Ok(StreamScan { hours, counts })
}

fn main() {
    const USAGE: &str = "usage: bench_real_d48 <collector_stream.bin> <out dir> \
                         [legacy|identity-v1]";
    assert_restated_constants_match_their_sources();
    let mut args = std::env::args().skip(1);
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };
    let stream = std::fs::read(&input).expect("read stream");
    let schema = d48::schema();
    schema
        .validate(wire_format::TickGrid::HOURLY.log2())
        .expect("d48 schema is well-formed");

    // ---- Pass 1: validate the WHOLE stream and fold its d48 DATA records. ---
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result.
    let scan = match scan_stream(&stream, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };

    // ---- Pass 2: re-fold the accepted hours and encode both arms. -----------
    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut fold = Domain48Fold::new();
    let mut prior: Option<Vec<KeyedState>> = None;
    let mut reader: Option<Vec<KeyedState>> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut gap_forced_baselines = 0usize;
    let mut empty_hours = 0usize;
    let mut delta_hours_with_change = 0usize;
    let mut wire_baseline_bytes = 0usize;
    let mut wire_delta_bytes = 0usize;
    // Delta-path churn: over the hours that actually emitted a delta.
    let mut delta_changed = [0usize; d48::COLLECTION_COUNT];
    let mut delta_tombstones = [0usize; d48::COLLECTION_COUNT];
    // Cadence-independent churn: over every measured-hour transition, taken
    // BEFORE the block decision so a change landing on a block boundary still
    // counts. One number cannot answer both questions.
    let mut transition_changed = [0usize; d48::COLLECTION_COUNT];
    let mut transition_tombstones = [0usize; d48::COLLECTION_COUNT];
    let mut transitions = 0usize;
    let mut rows_total = [0usize; d48::COLLECTION_COUNT];

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());

        let payload = &stream[hour.payload_span.clone()];
        let frame = match d48::decode_frame(payload) {
            Ok(frame) => frame,
            Err(reason) => {
                eprintln!("{input}: game hour {} re-decode: {reason}", hour.game_hour);
                std::process::exit(1);
            }
        };
        // The recipe's fourth boundary, used as one call rather than as
        // `apply` + `to_states`: it is all-or-nothing over both of its fallible
        // steps, and routing the real capture through it is what exercises that
        // boundary on real data instead of only in unit tests.
        let states = match d48::fold_payload(&schema, &mut fold, &frame, hour.mode, 0) {
            Ok(states) => states,
            Err(reason) => {
                eprintln!("{input}: game hour {} re-fold: {reason}", hour.game_hour);
                std::process::exit(1);
            }
        };
        for i in 0..d48::COLLECTION_COUNT {
            rows_total[i] += states[i].len();
        }

        // Cadence-independent churn, before the block decision.
        if let Some(previous) = prior.as_ref() {
            transitions += 1;
            for (i, collection) in schema.collections.iter().enumerate() {
                let d = diff(collection, &previous[i], &states[i]);
                transition_changed[i] += d.changed.len();
                transition_tombstones[i] += d.tombstones.len();
            }
        }

        // New arm: BASELINE at each block start — and at every §1.5 gap, which
        // forces a fresh baseline and restarts the delta chain (`opens_baseline`).
        // The wire-arm hour — baseline decision, encode and reader replay —
        // through the SAME function the tests execute (`encode_wire_hour`).
        let outcome = encode_wire_hour(
            &schema,
            hours_seen,
            hour.follows_previous,
            hour.game_hour,
            prior.as_deref(),
            &states,
            &mut reader,
            &mut wire,
        );
        if outcome.gap_forced {
            gap_forced_baselines += 1;
        }
        if outcome.opened_baseline {
            baselines += 1;
            wire_baseline_bytes += outcome.bytes;
        } else if outcome.emitted {
            delta_hours_with_change += 1;
            for i in 0..d48::COLLECTION_COUNT {
                delta_changed[i] += outcome.changed_by_collection[i];
                delta_tombstones[i] += outcome.tombstones_by_collection[i];
            }
            wire_delta_bytes += outcome.bytes;
        } else {
            empty_hours += 1;
        }
        wire_offsets.push(wire.len());

        prior = Some(states);
        hours_seen += 1;
    }

    std::fs::create_dir_all(&out_dir).expect("out dir");
    for (name, bytes) in [("ship_d48.bin", &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        ("ship_d48.offsets", &ship_offsets),
        ("wire_new.offsets", &wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    let c = &scan.counts;
    let names = [
        "construction",
        "buildings",
        "state_resources",
        "railway_construction",
        "railway_route",
    ];
    println!(
        "hours                {hours_seen} ({} non-contiguous steps)",
        c.non_contiguous
    );
    println!(
        "excluded hours       {} failure-status, {} gate-failed, {} delta-before-baseline",
        c.failure_records, c.gate_failed, c.delta_before_baseline
    );
    println!(
        "expected-partial     {} records (domain 48 has no registered tolerance, so the fold's \
         partial seed is unreachable on this stream)",
        c.expected_partial_records
    );
    println!(
        "coverage             {} frames without an RLC1 trailer, {} frames badging \
         target-state-unavailable",
        c.frames_without_trailer, c.target_state_unavailable_frames
    );
    println!(
        "tombstone frames     {} construction, {} buildings, {} resources",
        c.frames_with_construction_tombstones,
        c.frames_with_building_tombstones,
        c.frames_with_resource_tombstones
    );
    println!(
        "clear-gate rows      {} construction rows without template_id, {} building rows without \
         healthy_level, {} without partial_health_raw",
        c.construction_rows_without_template,
        c.building_rows_without_healthy_level,
        c.building_rows_without_partial_health
    );
    println!(
        "railway rows seen    {} rows, {} route provinces, {} with a null current_segment_index",
        c.railway_rows, c.railway_route_provinces, c.railway_rows_with_null_current_segment
    );
    for (i, name) in names.iter().enumerate() {
        println!(
            "rows avg {name:<20} {:.1}",
            rows_total[i] as f64 / hours_seen as f64
        );
    }
    println!(
        "baselines            {baselines} ({} at a {BLOCK_HOURS}-h block start, \
         {gap_forced_baselines} forced by a WIRE_FORMAT \u{a7}1.5 state gap)",
        baselines - gap_forced_baselines
    );
    println!("delta hours          {delta_hours_with_change}   empty hours {empty_hours}");
    println!(
        "wire raw split       {wire_baseline_bytes} baseline bytes + {wire_delta_bytes} delta \
         bytes = {} total",
        wire.len()
    );
    let transition_denominator = transitions.max(1) as f64;
    for (i, name) in names.iter().enumerate() {
        println!(
            "churn (cadence-independent) {name:<20} {:.2} changed rows/hour, {:.3} \
             tombstones/hour over {transitions} measured transitions (totals {} / {})",
            transition_changed[i] as f64 / transition_denominator,
            transition_tombstones[i] as f64 / transition_denominator,
            transition_changed[i],
            transition_tombstones[i]
        );
    }
    let delta_denominator = delta_hours_with_change.max(1) as f64;
    for (i, name) in names.iter().enumerate() {
        println!(
            "churn (delta path)          {name:<20} {:.2} changed rows/hour, {:.3} \
             tombstones/hour over {delta_hours_with_change} emitted deltas (totals {} / {})",
            delta_changed[i] as f64 / delta_denominator,
            delta_tombstones[i] as f64 / delta_denominator,
            delta_changed[i],
            delta_tombstones[i]
        );
    }
    println!("ship_d48.bin         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.1}%)",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{
        COMPACT_DOMAIN_FRAME, COMPACT_RECORD_HEADER_LEN, COMPACT_STATUS_PARTIAL_DATA,
    };

    // --- Stream builders -----------------------------------------------------

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
        assert_eq!(out.len(), COMPACT_RECORD_HEADER_LEN);
        out.extend_from_slice(payload);
        out
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

    fn frame_end(sequence: u64, game_hour: i32, domains: u32) -> Vec<u8> {
        let mut payload = vec![0u8; COMPACT_FRAME_END_PAYLOAD_LEN];
        payload[16..20].copy_from_slice(&domains.to_le_bytes());
        payload[20..24].copy_from_slice(&domains.to_le_bytes());
        record(
            sequence,
            game_hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &payload,
        )
    }

    // --- d48 payload builders (the same shapes `d48`'s own tests use) --------

    #[allow(clippy::too_many_arguments)]
    fn construction_row(
        tag_db_id: u32,
        line_word: u32,
        active_factories: i32,
        cost_raw: i32,
        priority: u32,
    ) -> Vec<u8> {
        let mut row = vec![0u8; d48::CONSTRUCTION_ROW_LEN];
        row[0..4].copy_from_slice(&tag_db_id.to_le_bytes());
        row[4..8].copy_from_slice(&line_word.to_le_bytes());
        row[8..12].copy_from_slice(&u32::MAX.to_le_bytes()); // template absent
        row[12..16].copy_from_slice(&active_factories.to_le_bytes());
        row[16..20].copy_from_slice(&cost_raw.to_le_bytes());
        row[24..28].copy_from_slice(&priority.to_le_bytes());
        row
    }

    fn construction_tombstone(tag_db_id: u32, line_id: u32) -> Vec<u8> {
        construction_row(tag_db_id, line_id | d48::ROW_ID_DELETED, 0, 0, 0)
    }

    fn building_row(state_id: u32, type_id: u32, level: u32) -> Vec<u8> {
        let mut row = vec![0u8; d48::BUILDING_ROW_LEN];
        row[0..4].copy_from_slice(&d48::SCOPE_STATE.to_le_bytes());
        row[4..8].copy_from_slice(&state_id.to_le_bytes());
        row[8..12].copy_from_slice(&state_id.to_le_bytes());
        row[12..16].copy_from_slice(&type_id.to_le_bytes());
        row[16..20].copy_from_slice(&level.to_le_bytes());
        row[20..24].copy_from_slice(&u32::MAX.to_le_bytes()); // healthy absent
        row
    }

    fn resource_row(state_id: u32, resource_id: u32, amount_raw: i32) -> Vec<u8> {
        let mut row = vec![0u8; d48::RESOURCE_ROW_LEN];
        row[0..4].copy_from_slice(&state_id.to_le_bytes());
        row[4..8].copy_from_slice(&resource_id.to_le_bytes());
        row[8..12].copy_from_slice(&amount_raw.to_le_bytes());
        row
    }

    fn railway_trailer(lines: &[(u32, u32, u32)]) -> Vec<u8> {
        let route: Vec<u32> = vec![11, 22, 33];
        let mut out = Vec::new();
        out.extend_from_slice(&d48::RAILWAY_TRAILER_MAGIC);
        out.extend_from_slice(&d48::RAILWAY_TRAILER_VERSION.to_le_bytes());
        out.extend_from_slice(&(d48::RAILWAY_TRAILER_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&(lines.len() as u32).to_le_bytes());
        out.extend_from_slice(&((lines.len() * route.len()) as u32).to_le_bytes());
        out.extend_from_slice(&(d48::RAILWAY_TRAILER_ROW_LEN as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for (index, (tag, line, queue)) in lines.iter().enumerate() {
            let mut row = vec![0u8; d48::RAILWAY_TRAILER_ROW_LEN];
            row[0..4].copy_from_slice(&tag.to_le_bytes());
            row[4..8].copy_from_slice(&line.to_le_bytes());
            row[8..12].copy_from_slice(&queue.to_le_bytes());
            row[12..16].copy_from_slice(&2i32.to_le_bytes());
            row[16..20].copy_from_slice(&10i32.to_le_bytes());
            row[20..24].copy_from_slice(&5i32.to_le_bytes());
            row[24..28].copy_from_slice(&60_000_000u32.to_le_bytes());
            row[28..32].copy_from_slice(&1u32.to_le_bytes());
            row[32..36].copy_from_slice(&3u32.to_le_bytes());
            row[36..40].copy_from_slice(&(-1i32).to_le_bytes());
            row[40..48].copy_from_slice(&1_500_000i64.to_le_bytes());
            row[48..52].copy_from_slice(&((index * route.len()) as u32).to_le_bytes());
            row[52..56].copy_from_slice(&(route.len() as u32).to_le_bytes());
            out.extend_from_slice(&row);
        }
        for _ in lines {
            for province in &route {
                out.extend_from_slice(&province.to_le_bytes());
            }
        }
        out
    }

    struct D48Payload {
        status_flags: u32,
        source_flags: u32,
        construction: Vec<Vec<u8>>,
        buildings: Vec<Vec<u8>>,
        resources: Vec<Vec<u8>>,
        railway: Option<Vec<(u32, u32, u32)>>,
    }

    impl D48Payload {
        /// A clean baseline: the target-state join is badged unavailable, which
        /// is what lets the construction rows omit it and still pass the gate.
        fn baseline() -> Self {
            Self {
                status_flags: 0,
                source_flags: d48::SOURCE_FLAG_DLL_NATIVE
                    | d48::SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE,
                construction: vec![construction_row(7, 1, 1, 100, 0)],
                buildings: vec![building_row(42, 0x1234, 5)],
                resources: vec![resource_row(42, 9, 1_234)],
                railway: Some(vec![(7, 99, 0)]),
            }
        }

        fn delta() -> Self {
            let mut payload = Self::baseline();
            payload.status_flags = COMPACT_STATUS_DELTA_DATA;
            payload
        }

        fn bytes(&self) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend_from_slice(b"CBSD");
            out.extend_from_slice(&d48::VERSION.to_le_bytes());
            out.extend_from_slice(&(d48::HEADER_LEN as u16).to_le_bytes());
            out.extend_from_slice(&48u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&self.status_flags.to_le_bytes());
            out.extend_from_slice(&0i32.to_le_bytes());
            out.extend_from_slice(&(self.construction.len() as u32).to_le_bytes());
            out.extend_from_slice(&(self.buildings.len() as u32).to_le_bytes());
            out.extend_from_slice(&(self.resources.len() as u32).to_le_bytes());
            out.extend_from_slice(&(d48::CONSTRUCTION_ROW_LEN as u16).to_le_bytes());
            out.extend_from_slice(&(d48::BUILDING_ROW_LEN as u16).to_le_bytes());
            out.extend_from_slice(&(d48::RESOURCE_ROW_LEN as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&1u32.to_le_bytes());
            out.extend_from_slice(&1u32.to_le_bytes());
            out.extend_from_slice(&1u32.to_le_bytes());
            out.extend_from_slice(&self.source_flags.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            for row in &self.construction {
                out.extend_from_slice(row);
            }
            for row in &self.buildings {
                out.extend_from_slice(row);
            }
            for row in &self.resources {
                out.extend_from_slice(row);
            }
            if let Some(lines) = &self.railway {
                out.extend_from_slice(&railway_trailer(lines));
            }
            out
        }
    }

    /// Two hours: a clean baseline then a delta, each hour PERF + DATA +
    /// FRAME_END, sealed with one non-probe PERF domain.
    fn stream_of(payloads: &[D48Payload]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut sequence = 1u64;
        for (index, payload) in payloads.iter().enumerate() {
            let hour = index as i32;
            out.extend_from_slice(&perf(sequence, hour, D48));
            sequence += 1;
            out.extend_from_slice(&record(
                sequence,
                hour,
                D48,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload.bytes(),
            ));
            sequence += 1;
            out.extend_from_slice(&frame_end(sequence, hour, 1));
            sequence += 1;
        }
        out
    }

    fn good_stream() -> Vec<u8> {
        stream_of(&[D48Payload::baseline(), D48Payload::delta()])
    }

    fn scan(stream: &[u8]) -> Result<StreamScan, String> {
        scan_stream(stream, StreamIdentityMode::Legacy)
    }

    fn fold_hours(stream: &[u8], scan: &StreamScan) -> Vec<Vec<KeyedState>> {
        let schema = d48::schema();
        let mut fold = Domain48Fold::new();
        let mut out = Vec::new();
        for hour in &scan.hours {
            let frame = d48::decode_frame(&stream[hour.payload_span.clone()]).expect("decodes");
            fold.apply(&frame, hour.mode).expect("folds");
            out.push(fold.projected.to_states(&schema, 0).expect("states"));
        }
        out
    }

    #[test]
    fn the_restated_constants_match_their_sources() {
        assert_restated_constants_match_their_sources();
    }

    #[test]
    fn a_well_formed_stream_yields_its_d48_hours() {
        let stream = good_stream();
        let scan = scan(&stream).expect("accepted");
        assert_eq!(scan.hours.len(), 2);
        assert_eq!(scan.counts.failure_records, 0);
        assert_eq!(scan.counts.gate_failed, 0);
        assert_eq!(scan.counts.delta_before_baseline, 0);
        assert_eq!(scan.counts.expected_partial_records, 0);
        assert_eq!(scan.counts.target_state_unavailable_frames, 2);
        assert_eq!(scan.counts.railway_rows, 2);
        assert_eq!(scan.counts.railway_route_provinces, 6);
        assert_eq!(scan.counts.railway_rows_with_null_current_segment, 2);

        let states = fold_hours(&stream, &scan);
        assert_eq!(states[0][d48::CONSTRUCTION_INDEX].len(), 1);
        assert_eq!(states[0][d48::RAILWAY_ROUTE_INDEX].len(), 3);
    }

    #[test]
    fn a_deleted_row_becomes_a_tombstone_in_its_collection() {
        let mut delta = D48Payload::delta();
        delta.construction = vec![construction_tombstone(7, 1)];
        delta.buildings = vec![];
        delta.resources = vec![];
        let stream = stream_of(&[D48Payload::baseline(), delta]);
        let scan = scan(&stream).expect("accepted");
        assert_eq!(scan.counts.frames_with_construction_tombstones, 1);
        let states = fold_hours(&stream, &scan);
        assert_eq!(states[0][d48::CONSTRUCTION_INDEX].len(), 1);
        assert_eq!(states[1][d48::CONSTRUCTION_INDEX].len(), 0);
        let schema = d48::schema();
        let d = diff(
            &schema.collections[d48::CONSTRUCTION_INDEX],
            &states[0][d48::CONSTRUCTION_INDEX],
            &states[1][d48::CONSTRUCTION_INDEX],
        );
        assert_eq!(d.tombstones.len(), 1);
        assert!(d.changed.is_empty());
        // ...and the other tables are untouched by a delta that omits them.
        assert_eq!(states[1][d48::BUILDINGS_INDEX].len(), 1);
        assert_eq!(states[1][d48::STATE_RESOURCES_INDEX].len(), 1);
    }

    #[test]
    fn a_failure_status_record_is_excluded_rather_than_measured() {
        let mut failed = D48Payload::baseline();
        failed.status_flags = COMPACT_STATUS_PARTIAL_DATA;
        let stream = stream_of(&[D48Payload::baseline(), failed, D48Payload::delta()]);
        let scan = scan(&stream).expect("accepted");
        assert_eq!(scan.counts.failure_records, 1);
        assert_eq!(scan.hours.len(), 2, "the failed hour is not measured");
        assert_eq!(scan.counts.non_contiguous, 1);
    }

    #[test]
    fn an_excluded_hour_forces_the_next_observation_to_be_a_baseline() {
        // `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the state part forces
        // the next state observation to be a `BASELINE`." Both gap shapes d48
        // can meet — an excluded hour (here failure-status), and a whole hour
        // the capture never scheduled — must clear `follows_previous`, and
        // `opens_baseline` must open a baseline on it wherever it lands in a
        // block. A delta would relate two states across unobserved time.
        //
        // Shape 1: hour 1 carries a failure-status d48 record (excluded).
        let mut failed = D48Payload::baseline();
        failed.status_flags = COMPACT_STATUS_PARTIAL_DATA;
        let stream = stream_of(&[D48Payload::baseline(), failed, D48Payload::delta()]);
        let scanned = scan(&stream).expect("accepted");
        assert_eq!(scanned.hours.len(), 2);
        assert_eq!(scanned.counts.non_contiguous, 1);
        assert!(!scanned.hours[0].follows_previous, "the first hour follows none");
        assert!(
            !scanned.hours[1].follows_previous,
            "hour 2 does not follow hour 0"
        );
        // ...and that is what makes the encoder open a baseline for it, even
        // though it is only the second measured hour of the block — through the
        // PRODUCTION encode path, not just the decision function.
        assert!(opens_baseline(1, scanned.hours[1].follows_previous));
        let schema = d48::schema();
        let states = fold_hours(&stream, &scanned);
        let mut wire = Vec::new();
        let mut reader: Option<Vec<KeyedState>> = None;
        let o0 = encode_wire_hour(
            &schema,
            0,
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

        // Shape 2: hour 1 is never scheduled at all — the capture closes hours
        // 0 and 2 only.
        let mut s = Vec::new();
        s.extend(perf(1, 0, D48));
        s.extend(record(
            2,
            0,
            D48,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &D48Payload::baseline().bytes(),
        ));
        s.extend(frame_end(3, 0, 1));
        s.extend(perf(4, 2, D48));
        s.extend(record(
            5,
            2,
            D48,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &D48Payload::delta().bytes(),
        ));
        s.extend(frame_end(6, 2, 1));
        let scanned = scan(&s).expect("well-formed");
        assert_eq!(scanned.hours.len(), 2);
        assert_eq!(scanned.counts.non_contiguous, 1);
        assert!(!scanned.hours[1].follows_previous);
        assert!(opens_baseline(1, scanned.hours[1].follows_previous));

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
        let scanned = scan(&good_stream()).expect("well-formed");
        assert_eq!(scanned.counts.non_contiguous, 0);
        assert!(scanned.hours[1].follows_previous);
        assert!(!opens_baseline(1, scanned.hours[1].follows_previous));
    }

    #[test]
    fn a_closed_hour_without_a_d48_record_refuses_the_stream() {
        // The cross-domain bound: this harness validates only d48 payloads, so
        // a d48 record relabelled to another domain — or dropped — would
        // otherwise be silently ignored and the run would report a plausible
        // measurement over fewer hours. Hour 0 carries a d48 record; hour 1
        // carries only another domain's DATA record. Every hour is
        // structurally well formed — a DOMAIN_DATA record opens it and a
        // FRAME_END closes it — so only the completeness check can see it.
        let mut s = Vec::new();
        s.extend(perf(1, 0, D48));
        s.extend(record(
            2,
            0,
            D48,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &D48Payload::baseline().bytes(),
        ));
        s.extend(frame_end(3, 0, 1));
        s.extend(record(4, 1, 20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &[0u8; 8]));
        s.extend(frame_end(5, 1, 0));
        let err = match scan(&s) {
            Err(err) => err,
            Ok(_) => panic!("a closed hour with no d48 record must refuse the stream"),
        };
        assert!(
            err.contains("every closed hour"),
            "the refusal names the completeness rule: {err}"
        );
    }

    /// Codex round-1 finding, class 7, carried in as its own regression test:
    /// the harness accepted a non-failure frame whose lane gate failed and
    /// classified it as an excluded hour, while the production payload
    /// authority records it as a parse error and production treats a
    /// parse error as an undecodable stream.
    ///
    /// The trigger is the reviewer's own: a structurally valid DATA record with
    /// a non-failure status carrying a construction row that lacks
    /// `target_state_id` while
    /// `SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE` is clear.
    #[test]
    fn a_gate_failure_under_a_non_failure_status_refuses_the_stream() {
        let mut ungated = D48Payload::baseline();
        ungated.source_flags = d48::SOURCE_FLAG_DLL_NATIVE; // no unavailable badge
        let stream = stream_of(&[ungated, D48Payload::baseline(), D48Payload::delta()]);
        let err = scan(&stream).expect_err("refused");
        assert!(err.contains("lane gate failed under a non-failure status"), "{err}");
        assert!(err.contains("target_state_id absent"), "{err}");
    }

    /// The whole gate vocabulary, swept — the finding is a property of "the
    /// gate failed under a non-failure status", not of the one input that
    /// exposed it, so every way `gate_frame` can fail must refuse the stream.
    #[test]
    fn every_gate_failure_under_a_non_failure_status_refuses_the_stream() {
        let cases: Vec<(&str, D48Payload)> = vec![
            ("missing DLL-native source flag", {
                let mut p = D48Payload::baseline();
                p.source_flags = d48::SOURCE_FLAG_CONSTRUCTION_TARGET_STATE_UNAVAILABLE;
                p
            }),
            ("clean frame with no building rows", {
                let mut p = D48Payload::baseline();
                p.buildings = vec![];
                p
            }),
            ("clean frame with no resource rows", {
                let mut p = D48Payload::baseline();
                p.resources = vec![];
                p
            }),
            ("target_state_id absent without the badge", {
                let mut p = D48Payload::baseline();
                p.source_flags = d48::SOURCE_FLAG_DLL_NATIVE;
                p
            }),
            ("duplicate construction identity", {
                let mut p = D48Payload::baseline();
                p.construction = vec![
                    construction_row(7, 1, 1, 100, 0),
                    construction_row(7, 1, 1, 100, 1),
                ];
                p
            }),
            ("duplicate building identity", {
                let mut p = D48Payload::baseline();
                p.buildings = vec![building_row(42, 0x1234, 5), building_row(42, 0x1234, 6)];
                p
            }),
            ("duplicate resource identity", {
                let mut p = D48Payload::baseline();
                p.resources = vec![resource_row(42, 9, 1), resource_row(42, 9, 2)];
                p
            }),
        ];
        for (name, payload) in cases {
            let stream = stream_of(&[payload]);
            let err = scan(&stream).unwrap_err();
            assert!(
                err.contains("lane gate failed under a non-failure status"),
                "gate case {name:?} must refuse the stream, got: {err}"
            );
        }
    }

    /// The other half of the authority's rule: under a FAILURE status it records
    /// nothing, so a gate-failing record is an ordinary excluded hour rather
    /// than a refused stream. Without this the fix above would be a blanket
    /// "any gate failure refuses", which the authority does not say.
    #[test]
    fn a_gate_failure_under_a_failure_status_is_merely_an_excluded_hour() {
        let mut ungated = D48Payload::baseline();
        ungated.source_flags = d48::SOURCE_FLAG_DLL_NATIVE; // gate would fail
        ungated.status_flags = COMPACT_STATUS_PARTIAL_DATA; // ...but the status is a failure
        let stream = stream_of(&[ungated, D48Payload::baseline(), D48Payload::delta()]);
        let scan = scan(&stream).expect("accepted");
        assert_eq!(scan.counts.failure_records, 1);
        assert_eq!(
            scan.counts.gate_failed, 0,
            "the failure arm comes first, exactly as in the fold"
        );
        assert_eq!(scan.hours.len(), 2);
    }

    #[test]
    fn a_delta_before_any_baseline_is_excluded() {
        let stream = stream_of(&[D48Payload::delta(), D48Payload::baseline()]);
        let scan = scan(&stream).expect("accepted");
        assert_eq!(scan.counts.delta_before_baseline, 1);
        assert_eq!(scan.hours.len(), 1);
    }

    #[test]
    fn a_malformed_payload_refuses_the_stream_whatever_its_status() {
        // The fold decodes BEFORE it looks at the status (`world_fold.rs:20226`),
        // so a malformed failure-status payload is a stream rejection, not a
        // quietly excluded hour. Copying d63's status-first order would have
        // measured this stream as healthy.
        for status in [0u32, COMPACT_STATUS_PARTIAL_DATA] {
            let mut broken = D48Payload::baseline();
            broken.status_flags = status;
            let mut bytes = broken.bytes();
            bytes[4] = 9; // version
            let stream = stream_of(&[D48Payload::baseline()]);
            let mut spliced = stream.clone();
            // Replace the DATA record's payload wholesale by rebuilding.
            spliced.clear();
            spliced.extend_from_slice(&perf(1, 0, D48));
            spliced.extend_from_slice(&record(
                2,
                0,
                D48,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &bytes,
            ));
            spliced.extend_from_slice(&frame_end(3, 0, 1));
            let err = scan(&spliced).expect_err("refused");
            assert!(err.contains("version mismatch"), "{err}");
        }
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        let mut payload = D48Payload::baseline();
        // Bit 15, not 14: the contract assigned bit 14 to the measured-empty
        // badge, so it is now a KNOWN informational status and refusing it
        // would reject a valid record. The rule under test is unchanged --
        // unassigned bits have no defined classification and must be rejected
        // before anything classifies the word.
        payload.status_flags = 1 << 15;
        let stream = stream_of(&[payload]);
        let err = scan(&stream).expect_err("refused");
        assert!(err.contains("unassigned v1 status bits"), "{err}");
    }

    #[test]
    fn a_second_d48_record_in_one_hour_is_refused() {
        let mut stream = Vec::new();
        stream.extend_from_slice(&perf(1, 0, D48));
        stream.extend_from_slice(&record(
            2,
            0,
            D48,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &D48Payload::baseline().bytes(),
        ));
        stream.extend_from_slice(&record(
            3,
            0,
            D48,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &D48Payload::baseline().bytes(),
        ));
        stream.extend_from_slice(&frame_end(4, 0, 1));
        let err = scan(&stream).expect_err("refused");
        assert!(err.contains("carries hour"), "{err}");
    }

    #[test]
    fn hour_uniqueness_is_enforced_before_status_exclusion() {
        // A success + failure pair in one hour must not slip through.
        let mut failed = D48Payload::baseline();
        failed.status_flags = COMPACT_STATUS_PARTIAL_DATA;
        let mut stream = Vec::new();
        stream.extend_from_slice(&perf(1, 0, D48));
        stream.extend_from_slice(&record(
            1,
            0,
            D48,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &D48Payload::baseline().bytes(),
        ));
        stream.extend_from_slice(&record(
            2,
            0,
            D48,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &failed.bytes(),
        ));
        stream.extend_from_slice(&frame_end(3, 0, 1));
        assert!(scan(&stream).is_err());
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_accepted_and_mid_record_is_not() {
        let stream = good_stream();
        // The first hour's three records.
        let boundary = perf(1, 0, D48).len()
            + record(
                1,
                0,
                D48,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &D48Payload::baseline().bytes(),
            )
            .len()
            + frame_end(3, 0, 1).len();
        let cut = scan(&stream[..boundary]).expect("a FRAME_END cut is a shorter well-formed stream");
        assert_eq!(cut.hours.len(), 1);
        assert!(scan(&stream[..boundary - 1]).is_err(), "mid-record is corrupt");
        assert!(
            scan(&stream[..boundary + 4]).is_err(),
            "a partial trailing record is corrupt"
        );
    }

    #[test]
    fn a_stream_that_never_closes_an_hour_is_refused() {
        let mut stream = Vec::new();
        stream.extend_from_slice(&perf(1, 0, D48));
        stream.extend_from_slice(&record(
            1,
            0,
            D48,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &D48Payload::baseline().bytes(),
        ));
        assert!(scan(&stream).is_err());
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        let mut stream = Vec::new();
        stream.extend_from_slice(&perf(1, 0, D48));
        stream.extend_from_slice(&record(
            2,
            0,
            D48,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &D48Payload::baseline().bytes(),
        ));
        // The seal claims two domains; only one non-probe PERF was observed.
        stream.extend_from_slice(&frame_end(3, 0, 2));
        let err = scan(&stream).expect_err("refused");
        assert!(err.contains("FRAME_END domain counts"), "{err}");
    }

    #[test]
    fn a_stream_with_no_d48_records_is_refused_not_reported_as_zero() {
        let mut stream = Vec::new();
        stream.extend_from_slice(&perf(1, 0, 20));
        stream.extend_from_slice(&record(2, 0, 20, COMPACT_RECORD_TYPE_DOMAIN_DATA, &[0u8; 8]));
        stream.extend_from_slice(&frame_end(3, 0, 1));
        let err = scan(&stream).expect_err("refused");
        assert!(err.contains("no usable d48 DATA records"), "{err}");
    }

    #[test]
    fn a_state_that_never_saw_a_trailer_refuses_the_stream() {
        // The recorded §8 divergence, enforced in pass 1 so no artifact is
        // written from a state the keyed set cannot honestly express.
        let mut legacy = D48Payload::baseline();
        legacy.railway = None;
        let stream = stream_of(&[legacy]);
        let err = scan(&stream).expect_err("refused");
        assert!(err.contains("never observed an RLC1 trailer"), "{err}");
    }

    /// Codex round-3 finding, class 6, carried in as its own regression test.
    ///
    /// Pass 1 used to call `fold.apply` and only THEN run the RLC1
    /// representability check, so the trigger frame — a valid legacy clean
    /// `CBSD` frame with no `RLC1` trailer — advanced both accumulators before
    /// `scan_stream` refused the stream. Same ordering defect as round 2's, one
    /// layer out. Pass 1 now projects, checks, and commits in that order.
    ///
    /// `scan_stream` owns its accumulator, so the assertion is made against the
    /// same boundary the harness uses: after the refusal, a fold driven through
    /// `project`/`commit` over the identical frame sequence must hold the
    /// pre-frame state, not the post-frame one.
    #[test]
    fn a_refused_trailerless_frame_does_not_advance_the_accumulator() {
        let schema = d48::schema();
        let mut legacy = D48Payload::baseline();
        legacy.railway = None;
        // The rejecting frame carries rows that WOULD be visible in the
        // accumulator had it been committed, so a commit-then-reject is
        // detectable rather than vacuous.
        legacy.construction = vec![construction_row(7, 4_242, 1, 100, 0)];

        let legacy_bytes = legacy.bytes();

        // A good baseline first, so there is a prior state to corrupt.
        let stream = stream_of(&[D48Payload::baseline(), legacy]);
        let err = scan(&stream).expect_err("refused");
        assert!(err.contains("never observed an RLC1 trailer"), "{err}");

        // Drive the same two frames through the public boundary in the order
        // pass 1 uses, and assert the refusal left the accumulator alone.
        let mut fold = Domain48Fold::new();
        let good = d48::decode_frame(&D48Payload::baseline().bytes()).expect("decodes");
        let good_mode = frame_mode(good.header.status_flags);
        let state = fold.project(&good, good_mode).expect("projects");
        fold.commit(state, good_mode);
        let before = fold.clone();

        let bad = d48::decode_frame(&legacy_bytes).expect("decodes");
        let bad_mode = frame_mode(bad.header.status_flags);
        let state = fold.project(&bad, bad_mode).expect("projects");
        assert!(
            state.railway.is_none(),
            "the trigger frame is the one that fails the representability check"
        );
        assert!(
            state.construction.contains_key(&(7, 4_242)),
            "and it really would have changed the accumulator had it committed"
        );
        // The check fires here; `commit` is never reached.
        assert_eq!(
            fold, before,
            "projecting a frame that fails the check must not advance either accumulator"
        );
        // Belt and braces: the library boundary agrees.
        assert!(fold_payload_is_atomic(&schema, &mut fold, &bad, bad_mode));
        assert_eq!(fold, before);
    }

    /// `d48::fold_payload` returns `Err` without mutating — asserted here too so
    /// the harness's ordering and the library's ordering are pinned together.
    fn fold_payload_is_atomic(
        schema: &wire_format::Schema,
        fold: &mut Domain48Fold,
        frame: &d48::Frame,
        mode: FrameMode,
    ) -> bool {
        let before = fold.clone();
        let result = d48::fold_payload(schema, fold, frame, mode, 0);
        result.is_err() && *fold == before
    }

    #[test]
    fn scan_stream_never_panics_on_arbitrary_bytes() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for len in [0usize, 1, 31, 32, 33, 96, 200, 512] {
            for _ in 0..40 {
                let bytes: Vec<u8> = (0..len).map(|_| (next() & 0xff) as u8).collect();
                let _ = scan(&bytes);
            }
        }
        // A well-formed frame with a corrupted d48 payload body.
        let good = good_stream();
        for offset in 0..good.len().min(400) {
            let mut bad = good.clone();
            bad[offset] ^= 0xff;
            let _ = scan(&bad);
        }
    }
}
