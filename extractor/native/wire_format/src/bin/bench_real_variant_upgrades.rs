//! Measure the wire format against the shipping compact stream on REAL d39
//! `air_variant_upgrades` and d40 `stockpile_variant_upgrades` dynamics.
//!
//! Usage: `bench_real_variant_upgrades <collector_stream.bin> <out dir> <39|40>
//! [legacy|identity-v1]`.
//!
//! **One binary, two domains**, for the same reason `wire_format::variant_upgrades`
//! is one module: `world_fold.rs:4684`-`:4685` dispatch one `fold_variant_upgrades`
//! for both, so a second harness would be a second copy of one authority. The
//! domain id on the command line selects `d39` or `d40`; nothing else differs.
//!
//! Input: a real `collector_stream.bin` from a capture that scheduled the domain
//! (`runs/batch2/deepwar_t1`). The stream's own DATA records are the shipping
//! format's cost for the domain: a 44-byte header plus a fixed 16-byte row per
//! carried variant plus a fixed 8-byte entry per carried upgrade. The shipping
//! form is **incremental and upsert-only** — the collector emits a variant row
//! only when that variant's whole upgrade list changed
//! (`attach_log.rs:53859`-`:53864`), and `fold_variant_upgrades` never clears its
//! accumulator (`world_fold.rs:15506`-`:15509`). There is no baseline record, no
//! delta bit, and no delete row. This tool replays those records into per-hour
//! variant/upgrade tables — the same fold the recorder performs, through
//! `wire_format::variant_upgrades::fold_payload` so there is one implementation
//! of it — and re-encodes every hour with this crate's grammar: a BASELINE at
//! each 48-hour block start, field-granular stored-minimal DELTAs between.
//!
//! **The two incremental layers are distinct.** The shipping layer is a stream of
//! upserts with no way to say "this variant lost an upgrade slot" other than
//! re-stating the variant with a shorter list; the wire layer decides BASELINE vs
//! DELTA by block position and by `body::diff`, and expresses that same loss as a
//! tombstone. A shipping record carrying no rows — 145 of d39's 146 hours and 141
//! of d40's — is a wire UNCHANGED hour, not a wire DELTA.
//!
//! **Nothing here re-derives a contract it can call.** Payload and row decoding
//! go through `wire_format::d39` / `d40`'s public boundary; H4CR framing, record
//! semantics, sequence continuity and the domain-hour failure predicate go
//! through `compact_contract`'s own parser, validator and constants. A harness
//! that reimplements either drifts from the recorder without anything failing.
//!
//! **Both production passes keep their own order.** `fold_variant_upgrades`
//! checks the two reserved header words (`:15447`) before its status exit
//! (`:15450`) and its own exact-length check only after it (`:15457`). The
//! independent authority checks the complete payload whatever its status
//! (`domain_data_authority.rs:9164`-`:9359`). This harness therefore calls
//! `parse_header`, rejects unassigned status bits, runs the complete second
//! authority, and only then classifies failure status. A failure status is the
//! only non-folding outcome; there is no delta-before-baseline path.
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
//!   (`recorder_host/src/stream_fold.rs:277`), including DOMAIN_PERF hour binding
//!   (`:328`-`:365`) and the DATA arm's pending-PERF rule (`:378`-`:386`);
//! - the FRAME_END seal fields (`:46`-`:89`) and their cross-check against
//!   distinct non-probe PERF domains (`:411`-`:419`);
//! - the EOF rules of that tracker's `eof_check` (`:459`): no residual bytes, no
//!   hour left open, at least one FRAME_END, and the final record is a FRAME_END;
//! - one DATA record per game hour for this domain, over EVERY such record and
//!   BEFORE any status-based exclusion — the domain is incremental, so two
//!   records in one hour would apply two shipping frames to one measurement hour,
//!   and gating the rule on the record being usable would let a success/failure
//!   pair through it entirely;
//! - every payload through the domain's `parse_header`/`split_payload` and every
//!   row through its `fold_payload`, so a row the fold could not read refuses the
//!   stream rather than unwinding later.
//!
//! That is a documented **subset** of the production boundary contract:
//! `recorder_host::stream_fold::verify_stream_logical_boundary` (`:537`) is the
//! full authority. Its shared d39/d40 payload decoder
//! `decode_compact_variant_upgrades_data_payload`
//! (`domain_data_authority.rs:9164`-`:9359`) runs on every record whatever its
//! status. `parse_header` already runs the 44-byte minimum and reserved-word
//! predicate before exclusion. [`check_authority_payload`] adds everything the
//! second authority still requires: four header count predicates, exact length,
//! bounded unique variant ids, known row flags, bounded/covered upgrade ranges,
//! exact declared coverage, bounded upgrade indexes and levels, and unique
//! upgrade indexes within each variant. The fold itself deliberately accepts
//! duplicate variants and duplicate upgrade indexes; certification does not, so
//! the second authority is decisive.
//!
//! The seal fields, PERF hour binding, pending-PERF DATA rule,
//! status-independent payload authority and stated identity mode are the five
//! checks carried back by the five-check sweep.
//!
//! The DATA payloads of every domain other than the measured one pass
//! UNVALIDATED beyond header sanity, the stream validator and the hour-boundary
//! rules — production validates every domain's payload (`stream_fold.rs:593`);
//! this harness owns only the d39/d40 grammar, and each invocation measures ONE
//! of the pair. That unvalidated remainder is explicitly BOUNDED: every closed
//! game hour must carry exactly one DATA record of the measured domain, of any
//! status (the `domain_records == closed_hours` completeness check in
//! [`scan_stream`]), so a record relabelled to another domain id — or dropped —
//! refuses the run instead of silently shrinking it.
//!
//! **Two passes, deliberately**, following `bench_real_d22`: pass 1 validates the
//! whole stream and folds every record through a single running accumulator it
//! then discards, keeping only each usable hour's byte span; pass 2 re-folds those
//! spans through the same `fold_payload` and encodes each hour as it goes.
//! Everything that can fail on stream *content* fails in pass 1, and pass 2
//! asserts only about the codec.
//!
//! Outputs, for the compression step (`scripts/compress_arms.py`):
//!   ship_d39.bin / ship_d40.bin — the domain's DATA records verbatim (32-byte
//!                    H4CR header + payload), concatenated in stream order
//!   wire_new.bin   — the wire-format bodies, concatenated in hour order
//!   *.offsets      — hour-boundary byte offsets, one per line, for exact
//!                    block-span slicing

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
use wire_format::variant_upgrades::{
    self as shared, Domain, PayloadHeader, State, COLLECTION_COUNT, UPGRADES_INDEX, VARIANTS_INDEX,
};
use wire_format::Schema;

const BLOCK_HOURS: usize = 48;

/// One usable hour: its game hour, its byte span in the source stream (the
/// shipping arm is those bytes verbatim, header included), and how much of that
/// span was rows rather than header.
///
/// No folded state is retained — see the module docs on the two passes.
struct DomainHour {
    game_hour: i32,
    ship_span: Range<usize>,
    carried_rows: bool,
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
/// and the pair's cadence is hourly, so a measured hour that does not follow
/// the previous measured one by exactly one is preceded by one. Neither a
/// delta nor a retained reader state can be justified across it.
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
    changed_variants: usize,
    changed_upgrades: usize,
    tombstones: usize,
}

#[allow(clippy::too_many_arguments)]
fn encode_wire_hour(
    schema: &Schema,
    hours_seen: usize,
    follows_previous: bool,
    game_hour: i32,
    prior: Option<&[KeyedState]>,
    state: &[KeyedState],
    reader: &mut Option<Vec<KeyedState>>,
    wire: &mut Vec<u8>,
) -> WireHourOutcome {
    let variants_c = &schema.collections[VARIANTS_INDEX];
    let upgrades_c = &schema.collections[UPGRADES_INDEX];
    let hour_start = wire.len();
    let block_start = opens_baseline(hours_seen, follows_previous);
    let gap_forced = block_start && hours_seen % BLOCK_HOURS != 0;
    let mut emitted = true;
    let mut changed_variants = 0usize;
    let mut changed_upgrades = 0usize;
    let mut tombstones = 0usize;
    let opened_baseline = block_start || prior.is_none();
    if opened_baseline {
        encode_baseline(schema, state, wire).expect("baseline encodes");
    } else {
        let prev = prior.unwrap();
        let dv = diff(variants_c, &prev[VARIANTS_INDEX], &state[VARIANTS_INDEX]);
        let du = diff(upgrades_c, &prev[UPGRADES_INDEX], &state[UPGRADES_INDEX]);
        if dv.is_empty() && du.is_empty() {
            emitted = false;
        } else {
            changed_variants = dv.changed.len();
            changed_upgrades = du.changed.len();
            tombstones = dv.tombstones.len() + du.tombstones.len();
            // §7's collection change bitmap: a collection that did not move
            // contributes nothing at all, which is the common case here.
            let contributions = [
                if dv.is_empty() { None } else { Some(dv) },
                if du.is_empty() { None } else { Some(du) },
            ];
            encode_delta(schema, prev, &contributions, wire).expect("delta encodes");
        }
    }
    let bytes = wire.len() - hour_start;

    // Reader replay: decode the bytes just written against the reader's OWN
    // reconstructed prior, exactly as a conforming reader would, and require
    // the result to be stored-equal to this hour's state. This is what makes
    // the size claim a claim about decodable bytes. An hour that emitted
    // nothing is still checked: the reader's retained state must already equal
    // the fold's, which is precisely what "nothing changed" asserts.
    if emitted {
        let hour_bytes = &wire[hour_start..];
        if block_start || reader.is_none() {
            let (decoded, consumed) =
                decode_baseline(schema, hour_bytes, 0).expect("baseline decodes");
            assert_eq!(consumed, hour_bytes.len(), "baseline consumed exactly");
            assert_eq!(
                decoded.len(),
                COLLECTION_COUNT,
                "two collections in the body"
            );
            *reader = Some(decoded);
        } else {
            let reader_prior = reader.take().unwrap();
            let (deltas, consumed) =
                decode_delta(schema, &reader_prior, hour_bytes, 0).expect("delta decodes");
            assert_eq!(consumed, hour_bytes.len(), "delta consumed exactly");
            let mut next = reader_prior;
            for (i, delta) in deltas.iter().enumerate() {
                if let Some(delta) = delta {
                    apply(&mut next[i], delta);
                }
            }
            *reader = Some(next);
        }
    }
    let replayed = reader.as_ref().unwrap();
    for (i, collection) in [variants_c, upgrades_c].into_iter().enumerate() {
        assert_eq!(
            replayed[i].len(),
            state[i].len(),
            "game hour {game_hour}: collection {} row count",
            collection.collection_id
        );
        assert!(
            diff(collection, &replayed[i], &state[i]).is_empty(),
            "game hour {game_hour}: reader collection {} is not stored-equal to the fold",
            collection.collection_id
        );
    }
    WireHourOutcome {
        opened_baseline,
        gap_forced,
        emitted,
        bytes,
        changed_variants,
        changed_upgrades,
        tombstones,
    }
}

struct StreamScan {
    hours: Vec<DomainHour>,
    /// Records whose payload status flags mark the domain-hour failed. The fold
    /// retains the accumulator and folds nothing (`world_fold.rs:15450`).
    failure_records: usize,
    /// Measured hours that do not follow the previous measured one by one.
    non_contiguous: usize,
    /// Variant rows and upgrade entries the shipping records carried, summed over
    /// measured hours.
    shipped_variant_rows: usize,
    shipped_upgrade_rows: usize,
}

// Re-exported, not re-declared: a local `(1 << 14) - 1` became wrong the moment
// the contract assigned bit 14 to the measured-empty badge.
const FRAME_END_KNOWN_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;
const PROBE_DOMAIN_ID: u16 = 250;
const AUTHORITY_MAX_VARIANT_ID: u32 = i32::MAX as u32;
const AUTHORITY_MAX_UPGRADE_INDEX: u32 = 256;
const AUTHORITY_MAX_LEVEL: u32 = 255;

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

/// Replay the shared d39/d40 second authority
/// (`domain_data_authority.rs:9164`-`:9359`) after `parse_header`'s already-run
/// minimum-length and reserved-word predicates.
fn check_authority_payload(
    domain: Domain,
    payload: &[u8],
    header: &PayloadHeader,
    at_byte: usize,
) -> Result<(), String> {
    if header.changed_rows > header.row_count {
        return Err(format!(
            "{} payload at byte {at_byte}: changed_rows {} > row_count {}",
            domain.name, header.changed_rows, header.row_count
        ));
    }
    if header.status_flags & COMPACT_STATUS_DELTA_DATA == 0
        && header.row_count > header.observed_variant_rows
    {
        return Err(format!(
            "{} payload at byte {at_byte}: row_count {} > observed_variant_rows {}",
            domain.name, header.row_count, header.observed_variant_rows
        ));
    }
    if header.status_flags & COMPACT_STATUS_DELTA_DATA == 0
        && header.emitted_upgrade_rows > header.observed_upgrade_rows
    {
        return Err(format!(
            "{} payload at byte {at_byte}: emitted_upgrade_rows {} > \
             observed_upgrade_rows {}",
            domain.name, header.emitted_upgrade_rows, header.observed_upgrade_rows
        ));
    }
    if header.invalid_variant_rows > header.observed_variant_rows {
        return Err(format!(
            "{} payload at byte {at_byte}: invalid_variant_rows {} > \
             observed_variant_rows {}",
            domain.name, header.invalid_variant_rows, header.observed_variant_rows
        ));
    }

    // The domain module owns exact length, row flags, range bounds and declared
    // coverage. Calling it also gives bounded slices for the additional checks.
    let (_, tables) = shared::split_payload(domain, payload)
        .map_err(|e| format!("{} payload at byte {at_byte}: {e}", domain.name))?;
    let mut seen_variants = BTreeSet::new();
    let mut ranges = Vec::new();
    for row in tables.variants.chunks_exact(shared::ROW_LEN) {
        let (variant_id, range) = shared::variant_row(domain, header, row)
            .map_err(|e| format!("{} row at byte {at_byte}: {e}", domain.name))?;
        if variant_id > AUTHORITY_MAX_VARIANT_ID {
            return Err(format!(
                "{} row at byte {at_byte}: invalid variant_id {variant_id}",
                domain.name
            ));
        }
        if !seen_variants.insert(variant_id) {
            return Err(format!(
                "{} row at byte {at_byte}: duplicate variant_id {variant_id}",
                domain.name
            ));
        }
        ranges.push(range);
    }

    let mut entries = Vec::new();
    for entry in tables.upgrades.chunks_exact(shared::ENTRY_ROW_LEN) {
        let upgrade_index = u32::from_le_bytes(entry[0..4].try_into().expect("4 bytes"));
        let level = u32::from_le_bytes(entry[4..8].try_into().expect("4 bytes"));
        if upgrade_index >= AUTHORITY_MAX_UPGRADE_INDEX {
            return Err(format!(
                "{} upgrade row at byte {at_byte}: invalid upgrade_index {upgrade_index}",
                domain.name
            ));
        }
        if level > AUTHORITY_MAX_LEVEL {
            return Err(format!(
                "{} upgrade row at byte {at_byte}: invalid level {level}",
                domain.name
            ));
        }
        entries.push(upgrade_index);
    }
    for range in ranges {
        let mut seen_indexes = BTreeSet::new();
        for upgrade_index in &entries[range] {
            if !seen_indexes.insert(*upgrade_index) {
                return Err(format!(
                    "{} payload at byte {at_byte}: duplicate upgrade_index {upgrade_index}",
                    domain.name
                ));
            }
        }
    }
    Ok(())
}

/// Validate the whole stream against the canonical contract and fold every usable
/// DATA record of `domain`. Every rejection path is an `Err`, never a silently
/// shorter measurement and never a panic — see the module docs for what is
/// enforced and what the recorder's own validator additionally checks.
fn scan_stream(
    stream: &[u8],
    domain: Domain,
    schema: &Schema,
    identity_mode: StreamIdentityMode,
) -> Result<StreamScan, String> {
    // The protocol is stated by the caller, never inferred from the first record
    // (`stream_fold.rs:220`; `stream_validator.rs:328`).
    let mut validator = match identity_mode {
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
    };

    let domain_id = u16::try_from(domain.id).map_err(|_| format!("domain id {}", domain.id))?;
    let mut scan = StreamScan {
        hours: Vec::new(),
        failure_records: 0,
        non_contiguous: 0,
        shipped_variant_rows: 0,
        shipped_upgrade_rows: 0,
    };

    let mut open_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut last_record_type: Option<u16> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut non_probe_perf_domains: BTreeSet<u16> = BTreeSet::new();
    // COMPLETENESS accounting: every DATA record of the measured domain of ANY
    // status, against every closed game hour — see the check after the EOF rules.
    let mut domain_records = 0usize;
    let mut closed_hours = 0usize;
    // Hour of the last DATA record of ANY status — the uniqueness rule.
    let mut last_domain_hour: Option<i32> = None;
    // Hour of the last record that entered the measurement — the contiguity
    // report, which a skipped failure hour legitimately breaks.
    let mut last_measured_hour: Option<i32> = None;
    // The pass-1 accumulator, kept only so that every row's decodability and the
    // fold's own upsert order are checked here.
    let mut accumulator = State::new();

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

        if header.domain_id == domain_id && header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            domain_records += 1;
            // Pre-status half of the fold's header handling: the 44-byte minimum
            // (`world_fold.rs:15437`) and the two reserved words (`:15447`), both
            // of which the recorder enforces on a failure-status record too.
            let payload_header: PayloadHeader = shared::parse_header(domain, payload)
                .map_err(|e| format!("{} payload at byte {pos}: {e}", domain.name))?;

            // One DATA record per game hour, checked over EVERY record of this
            // domain and BEFORE any status-based exclusion.
            if let Some(previous) = last_domain_hour {
                if header.game_hour_raw <= previous {
                    return Err(format!(
                        "{} DATA at byte {pos} carries hour {} after hour {previous}",
                        domain.name, header.game_hour_raw
                    ));
                }
            }
            last_domain_hour = Some(header.game_hour_raw);

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
            // Neither capture carries an unassigned bit, so no measured number
            // moves for either domain.
            let unknown_status = compact_status_unknown_flags_v1(payload_header.status_flags);
            if unknown_status != 0 {
                return Err(format!(
                    "{} DATA at byte {pos}: hour {} carries unassigned v1 status bits \
                     {unknown_status:#x}; an unassigned bit has no defined classification",
                    domain.name, header.game_hour_raw
                ));
            }

            check_authority_payload(domain, payload, &payload_header, pos)?;

            // `world_fold::handle_compact_failure` marks the domain-hour unknown
            // and RETAINS the prior accumulator on a failure status (`:15450`),
            // folding nothing from this record. The independent authority above
            // has still checked its complete payload. The predicate is
            // `compact_contract`'s own, not a copied bit mask: it already applies
            // any lane-local mask and expected partial tolerance, so it cannot
            // drift from the recorder. Neither 39
            // nor 40 has a tolerance registered, so `COMPACT_STATUS_PARTIAL_DATA`
            // — which the read sets whenever it dropped a variant it could not
            // validate (`attach_log.rs:53856`) — excludes the hour.
            if compact_status_failure_flags_after_expected_partial_tolerance(
                domain_id,
                payload_header.status_flags,
            ) != 0
            {
                scan.failure_records += 1;
                pos = record_end;
                continue;
            }

            // Post-status half: exact length, per-row checks, and the coverage
            // rule, then the fold itself.
            let (payload_header, _) = shared::split_payload(domain, payload)
                .map_err(|e| format!("{} payload at byte {pos}: {e}", domain.name))?;
            shared::fold_payload(domain, schema, &mut accumulator, payload, 0)
                .map_err(|e| format!("{} payload at byte {pos}: {e}", domain.name))?;

            let follows_previous =
                last_measured_hour.is_some_and(|previous| header.game_hour_raw == previous + 1);
            if last_measured_hour.is_some() && !follows_previous {
                scan.non_contiguous += 1;
            }
            last_measured_hour = Some(header.game_hour_raw);
            scan.shipped_variant_rows += payload_header.row_count as usize;
            scan.shipped_upgrade_rows += payload_header.emitted_upgrade_rows as usize;
            scan.hours.push(DomainHour {
                game_hour: header.game_hour_raw,
                ship_span: pos..record_end,
                carried_rows: payload_header.row_count > 0,
                follows_previous,
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
        return Err(format!(
            "stream carries no usable {} DATA records",
            domain.name
        ));
    }

    // COMPLETENESS: every closed game hour must carry exactly one DATA record
    // of the measured domain, of any status. This bounds what this harness
    // canNOT validate: production runs `validate_domain_data_payload_at` on
    // EVERY domain's DATA payload (`stream_fold.rs:593`), while this harness
    // owns only the d39/d40 grammar, so a record whose header was relabelled to
    // another domain id — or lost outright — would otherwise be silently
    // ignored and the run would report a plausible measurement over fewer
    // hours. A per-capture completeness assertion, NOT a grammar rule: a
    // capture where the domain is not scheduled every hour fails here loudly
    // and a human adjusts, instead of a number quietly shrinking
    // (`bench_real_d65.rs` precedent).
    if domain_records != closed_hours {
        return Err(format!(
            "stream closes {closed_hours} game hour(s) but carries {domain_records} {} DATA \
             record(s); every closed hour of a capture scheduling this domain must carry \
             exactly one, and this harness cannot validate the payloads of other domains to \
             see where the difference went",
            domain.name
        ));
    }

    Ok(scan)
}

fn main() {
    let mut args = std::env::args().skip(1);
    const USAGE: &str = "usage: bench_real_variant_upgrades <collector_stream.bin> <out dir> \
                         <39|40> [legacy|identity-v1]";
    let input = args.next().expect(USAGE);
    let out_dir = args.next().expect(USAGE);
    let domain_arg: u32 = args.next().expect(USAGE).parse().expect("domain id");
    let identity_mode = match args.next().as_deref() {
        None | Some("legacy") => StreamIdentityMode::Legacy,
        Some("identity-v1") => StreamIdentityMode::IdentityV1,
        Some(other) => panic!("unknown identity mode {other:?}; {USAGE}"),
    };
    let (domain, schema, ship_name) = match domain_arg {
        39 => (
            wire_format::d39::AIR_VARIANT_UPGRADES,
            wire_format::d39::schema(),
            "ship_d39.bin",
        ),
        40 => (
            wire_format::d40::STOCKPILE_VARIANT_UPGRADES,
            wire_format::d40::schema(),
            "ship_d40.bin",
        ),
        other => panic!("{USAGE} (got {other})"),
    };
    let stream = std::fs::read(&input).expect("read stream");

    // ---- Pass 1: validate the WHOLE stream and fold its DATA records. --------
    // No artifact is created before this succeeds, so a refused stream leaves no
    // partial measurement behind to be mistaken for a result. Everything that can
    // fail on stream content fails here, with a diagnosis and no unwind.
    let scan = match scan_stream(&stream, domain, &schema, identity_mode) {
        Ok(scan) => scan,
        Err(reason) => {
            eprintln!("{input}: {reason}");
            std::process::exit(1);
        }
    };
    std::fs::create_dir_all(&out_dir).expect("out dir");

    // ---- Pass 2: re-fold the accepted spans and encode both arms. Nothing here
    // can fail on stream content — pass 1 already accepted every byte it reads —
    // so every assertion below is about the codec. ----------------------------

    let mut ship = Vec::new();
    let mut ship_offsets = vec![0usize];
    let mut wire = Vec::new();
    let mut wire_offsets = vec![0usize];

    let mut accumulator = State::new();
    let mut prior: Option<Vec<KeyedState>> = None;
    let mut reader: Option<Vec<KeyedState>> = None;
    let mut hours_seen = 0usize;
    let mut baselines = 0usize;
    let mut gap_forced_baselines = 0usize;
    let mut empty_hours = 0usize;
    let mut changed_variants_total = 0usize;
    let mut changed_upgrades_total = 0usize;
    let mut tombstones_total = 0usize;
    let mut variant_rows_total = 0usize;
    let mut upgrade_rows_total = 0usize;
    // The two arms do not re-state the world equally often, and that dominates
    // the totals. The wire split is grammar-defined (BASELINE vs DELTA); the ship
    // split is not — this shipping grammar has no baseline record at all, so its
    // split is "records that carried rows" vs "records that carried none", a
    // description of the capture rather than of the format.
    let mut ship_row_bearing_bytes = 0usize;
    let mut ship_rowless_bytes = 0usize;
    let mut wire_baseline_bytes = 0usize;
    let mut wire_delta_bytes = 0usize;

    for hour in &scan.hours {
        // Shipping arm: the record's exact bytes, header included.
        ship.extend_from_slice(&stream[hour.ship_span.clone()]);
        ship_offsets.push(ship.len());
        if hour.carried_rows {
            ship_row_bearing_bytes += hour.ship_span.len();
        } else {
            ship_rowless_bytes += hour.ship_span.len();
        }

        // Re-fold this hour's shipping record into the running accumulator,
        // through the same boundary pass 1 used.
        let payload = &stream[hour.ship_span.start + COMPACT_RECORD_HEADER_LEN..hour.ship_span.end];
        shared::fold_payload(domain, &schema, &mut accumulator, payload, 0)
            .expect("pass 1 accepted this payload");
        let state: Vec<KeyedState> = accumulator.as_slice().to_vec();
        variant_rows_total += state[VARIANTS_INDEX].len();
        upgrade_rows_total += state[UPGRADES_INDEX].len();

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
            &state,
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
            changed_variants_total += outcome.changed_variants;
            changed_upgrades_total += outcome.changed_upgrades;
            tombstones_total += outcome.tombstones;
            wire_delta_bytes += outcome.bytes;
        } else {
            empty_hours += 1;
        }
        wire_offsets.push(wire.len());

        prior = Some(state);
        hours_seen += 1;
    }

    for (name, bytes) in [(ship_name, &ship), ("wire_new.bin", &wire)] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        f.write_all(bytes).unwrap();
    }
    for (name, offsets) in [
        (
            format!("{}.offsets", ship_name.trim_end_matches(".bin")),
            &ship_offsets,
        ),
        ("wire_new.offsets".to_string(), &wire_offsets),
    ] {
        let mut f = std::fs::File::create(std::path::Path::new(&out_dir).join(name)).unwrap();
        for o in offsets {
            writeln!(f, "{o}").unwrap();
        }
    }

    let delta_hours = hours_seen - baselines - empty_hours;
    println!("domain               {} (d{})", domain.name, domain.id);
    println!(
        "hours                {hours_seen} ({} non-contiguous steps)",
        scan.non_contiguous
    );
    println!(
        "failure-status hours {} (folded nothing; excluded from both arms)",
        scan.failure_records
    );
    println!(
        "shipping layer       {} variant rows + {} upgrade entries carried, over {} \
         row-bearing and {} rowless records",
        scan.shipped_variant_rows,
        scan.shipped_upgrade_rows,
        scan.hours.iter().filter(|h| h.carried_rows).count(),
        scan.hours.iter().filter(|h| !h.carried_rows).count()
    );
    println!(
        "reconstructed rows   {} variants, {} upgrades at the last measured hour",
        prior.as_ref().map_or(0, |p| p[VARIANTS_INDEX].len()),
        prior.as_ref().map_or(0, |p| p[UPGRADES_INDEX].len())
    );
    println!(
        "table size avg       {:.1} variants, {:.1} upgrades",
        variant_rows_total as f64 / hours_seen as f64,
        upgrade_rows_total as f64 / hours_seen as f64
    );
    println!(
        "baselines            {baselines} ({} at a {BLOCK_HOURS}-h block start, \
         {gap_forced_baselines} forced by a WIRE_FORMAT \u{a7}1.5 state gap)",
        baselines - gap_forced_baselines
    );
    println!("delta hours          {delta_hours}   empty hours {empty_hours}");
    println!(
        "real churn           {:.3} changed variants/hour, {:.3} changed upgrades/hour, \
         {:.3} tombstones/hour (stored precision, over {} delta hours)",
        changed_variants_total as f64 / delta_hours.max(1) as f64,
        changed_upgrades_total as f64 / delta_hours.max(1) as f64,
        tombstones_total as f64 / delta_hours.max(1) as f64,
        delta_hours
    );
    println!("{ship_name}         {} bytes", ship.len());
    println!(
        "wire_new.bin         {} bytes ({:.1}%)",
        wire.len(),
        100.0 * wire.len() as f64 / ship.len() as f64
    );
    println!(
        "  wire raw split     {wire_baseline_bytes} baseline in {baselines} block start(s), \
         {wire_delta_bytes} delta over {delta_hours} hour(s)"
    );
    println!(
        "  ship raw split     {ship_row_bearing_bytes} in records carrying rows, \
         {ship_rowless_bytes} in rowless records (this shipping grammar has no \
         baseline record)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::{COMPACT_DOMAIN_FRAME, COMPACT_FRAME_END_PAYLOAD_LEN};

    const D39: u16 = 39;

    fn domain() -> Domain {
        wire_format::d39::AIR_VARIANT_UPGRADES
    }

    fn schema() -> Schema {
        wire_format::d39::schema()
    }

    fn scan(stream: &[u8]) -> Result<StreamScan, String> {
        scan_stream(stream, domain(), &schema(), StreamIdentityMode::Legacy)
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

    fn variant_row(variant_id: u32, flags: u32, first: u32, count: u32) -> Vec<u8> {
        let mut row = Vec::new();
        for v in [variant_id, flags, first, count] {
            row.extend_from_slice(&v.to_le_bytes());
        }
        row
    }

    fn entry(upgrade_index: u32, level: u32) -> Vec<u8> {
        let mut e = Vec::new();
        for v in [upgrade_index, level] {
            e.extend_from_slice(&v.to_le_bytes());
        }
        e
    }

    /// A DATA payload: the 44-byte header then the two arrays.
    fn payload(status_flags: u32, rows: &[Vec<u8>], entries: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // row_count
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // changed_rows
        out.extend_from_slice(&status_flags.to_le_bytes());
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes()); // observed_variant_rows
        out.extend_from_slice(&(entries.len() as u32).to_le_bytes()); // observed_upgrade_rows
        out.extend_from_slice(&(entries.len() as u32).to_le_bytes()); // emitted_upgrade_rows
        out.extend_from_slice(&0u32.to_le_bytes()); // invalid_variant_rows
        out.extend_from_slice(&0u32.to_le_bytes()); // reserved0
        out.extend_from_slice(&0u32.to_le_bytes()); // reserved1
        for row in rows.iter().chain(entries) {
            out.extend_from_slice(row);
        }
        out
    }

    /// Two closed hours, the second carrying no rows — the shape 145 of d39's 146
    /// hours actually have.
    fn good_stream() -> Vec<u8> {
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D39,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload(0, &[variant_row(7, 0, 0, 1)], &[entry(0, 3)]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D39,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload(0, &[], &[]),
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
    fn a_well_formed_stream_yields_its_hours() {
        let s = scan(&good_stream()).expect("accepted");
        assert_eq!(s.hours.len(), 2);
        assert_eq!(s.hours[0].game_hour, 100);
        assert!(s.hours[0].carried_rows);
        assert_eq!(s.hours[1].game_hour, 101);
        assert!(
            !s.hours[1].carried_rows,
            "a rowless record is still an hour"
        );
        assert_eq!(s.failure_records, 0);
        assert_eq!(s.non_contiguous, 0);
        assert_eq!(s.shipped_variant_rows, 1);
        assert_eq!(s.shipped_upgrade_rows, 1);
    }

    #[test]
    fn an_unassigned_status_bit_is_refused_not_classified() {
        // `compact_contract/src/lib.rs:121`-`:123`: the failure helper classifies
        // the v1 vocabulary and is not a validity predicate, so an unassigned bit
        // must be rejected BEFORE classification — otherwise it silently reads as
        // "not a failure" and the hour is measured as healthy.
        for bit in 15..32u32 {
            let status = 1u32 << bit;
            // Alone, combined with a known failure bit, and combined with a known
            // informational bit; with rows and without (145 of d39's 146 hours are
            // rowless). The rejection must not depend on any of it.
            for extra in [0u32, 1, COMPACT_STATUS_DELTA_DATA] {
                for (rows, entries) in [
                    (Vec::new(), Vec::new()),
                    (vec![variant_row(7, 0, 0, 1)], vec![entry(0, 3)]),
                ] {
                    let mut s = Vec::new();
                    s.extend(record(
                        1,
                        100,
                        D39,
                        COMPACT_RECORD_TYPE_DOMAIN_DATA,
                        &payload(status | extra, &rows, &entries),
                    ));
                    s.extend(frame_end(2, 100));
                    assert!(
                        scan(&s).is_err(),
                        "status bit {bit} (+{extra:#x}) with {} row(s) must be refused",
                        rows.len()
                    );
                }
            }
        }
        // And the whole known v1 mask, minus the failure bits, is still accepted:
        // the check must reject unassigned bits and nothing else. Neither d39 nor
        // d40 reads an informational bit — the domains have no baseline/delta
        // grammar — so each is measured exactly as a clean status word is.
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
                D39,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload(informational, &[variant_row(7, 0, 0, 1)], &[entry(0, 3)]),
            ));
            s.extend(frame_end(2, 100));
            let scanned =
                scan(&s).unwrap_or_else(|e| panic!("informational bit {informational:#x}: {e}"));
            assert_eq!(scanned.hours.len(), 1, "bit {informational:#x}");
            assert_eq!(scanned.failure_records, 0, "bit {informational:#x}");
        }
    }

    #[test]
    fn the_other_domain_of_the_pair_is_not_measured_by_this_one() {
        // The two domains share a grammar and a stream. A harness that keyed on
        // the payload shape rather than the record's domain id would measure both
        // at once and report a number for neither.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D39,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload(0, &[variant_row(7, 0, 0, 0)], &[]),
        ));
        s.extend(record(
            2,
            100,
            40,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload(0, &[variant_row(9, 0, 0, 0)], &[]),
        ));
        s.extend(frame_end(3, 100));
        let scanned = scan(&s).expect("well-formed");
        assert_eq!(scanned.hours.len(), 1, "only the d39 record is measured");
        assert_eq!(scanned.shipped_variant_rows, 1);
        // And scanning the same stream for d40 measures the other one.
        let other = scan_stream(
            &s,
            wire_format::d40::STOCKPILE_VARIANT_UPGRADES,
            &wire_format::d40::schema(),
            StreamIdentityMode::Legacy,
        )
        .expect("well-formed");
        assert_eq!(other.hours.len(), 1);
        assert_eq!(other.shipped_variant_rows, 1);
    }

    #[test]
    fn an_excluded_hour_forces_the_next_observation_to_be_a_baseline() {
        // `WIRE_FORMAT.md:187`-`:190`: "A gap affecting the state part forces
        // the next state observation to be a `BASELINE`." Both gap shapes this
        // pair can meet — an excluded failure-status hour, and a whole hour the
        // capture never scheduled — must clear `follows_previous`, and
        // `opens_baseline` must open a baseline on it wherever it lands in a
        // block. A delta would relate two states across unobserved time.
        //
        // Shape 1: hour 101 carries a failure-status d39 record (excluded).
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D39,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload(0, &[variant_row(7, 0, 0, 1)], &[entry(0, 3)]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D39,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload(1 << 0, &[], &[]), // failure status -> excluded
        ));
        s.extend(frame_end(4, 101));
        s.extend(record(
            5,
            102,
            D39,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload(0, &[], &[]),
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
        assert!(opens_baseline(1, scanned.hours[1].follows_previous));
        let full_schema = schema();
        let mut accumulator = State::new();
        let mut states: Vec<Vec<KeyedState>> = Vec::new();
        for hour in &scanned.hours {
            let payload =
                &s[hour.ship_span.start + COMPACT_RECORD_HEADER_LEN..hour.ship_span.end];
            shared::fold_payload(domain(), &full_schema, &mut accumulator, payload, 0)
                .expect("pass 1 accepted this payload");
            states.push(accumulator.as_slice().to_vec());
        }
        let mut wire = Vec::new();
        let mut reader: Option<Vec<KeyedState>> = None;
        let o0 = encode_wire_hour(
            &full_schema,
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
            &full_schema,
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

        // Shape 2: hour 101 is never scheduled at all — the capture closes
        // hours 100 and 102 only.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D39,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload(0, &[variant_row(7, 0, 0, 1)], &[entry(0, 3)]),
        ));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            102,
            D39,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload(0, &[], &[]),
        ));
        s.extend(frame_end(4, 102));
        let scanned = scan(&s).expect("well-formed");
        assert_eq!(scanned.hours.len(), 2);
        assert_eq!(scanned.non_contiguous, 1);
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
        assert_eq!(scanned.non_contiguous, 0);
        assert!(scanned.hours[1].follows_previous);
        assert!(!opens_baseline(1, scanned.hours[1].follows_previous));
    }

    #[test]
    fn a_closed_hour_without_a_record_of_the_measured_domain_refuses_the_stream() {
        // The cross-domain bound: this harness validates only the measured
        // domain's payloads, so a record relabelled to another domain — or
        // dropped — would otherwise be silently ignored and the run would
        // report a plausible measurement over fewer hours. Hour 100 carries a
        // d39 record; hour 101 carries only another domain's DATA record.
        // Every hour is structurally well formed, so only the completeness
        // check can see it.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D39,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload(0, &[variant_row(7, 0, 0, 1)], &[entry(0, 3)]),
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
        let err = match scan(&s) {
            Err(err) => err,
            Ok(_) => panic!("a closed hour with no d39 record must refuse the stream"),
        };
        assert!(
            err.contains("every closed hour"),
            "the refusal names the completeness rule: {err}"
        );
    }

    #[test]
    fn a_failure_status_record_is_excluded_and_leaves_the_accumulator_alone() {
        // Every failure bit in `COMPACT_STATUS_FAILURE_MASK`, one at a time: the
        // fold retains the accumulator and folds nothing (`world_fold.rs:15450`),
        // so the hour is not this domain's data and must not enter either arm.
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D39,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload(1 << bit, &[variant_row(7, 0, 0, 0)], &[]),
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D39,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload(0, &[variant_row(9, 0, 0, 0)], &[]),
            ));
            s.extend(frame_end(4, 101));
            let result = scan(&s).expect("the stream itself is well-formed");
            assert_eq!(result.failure_records, 1, "bit {bit}");
            assert_eq!(result.hours.len(), 1, "bit {bit}");
            assert_eq!(result.hours[0].game_hour, 101, "bit {bit}");
            assert_eq!(result.shipped_variant_rows, 1, "bit {bit}");
        }
        // And the DELTA bit is not a failure bit. This fold never reads it
        // (`world_fold.rs:15431`-`:15510` has no `COMPACT_STATUS_DELTA_DATA`), so
        // a record carrying it is an ordinary upsert here, not a delta.
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D39,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload(COMPACT_STATUS_DELTA_DATA, &[variant_row(7, 0, 0, 0)], &[]),
        ));
        s.extend(frame_end(2, 100));
        let result = scan(&s).expect("well-formed");
        assert_eq!(result.failure_records, 0);
        assert_eq!(result.hours.len(), 1);
    }

    #[test]
    fn a_failure_status_record_is_still_length_checked_by_certification() {
        // The fold exits before its own length check, but strict certification's
        // independent decoder checks exact length on every record
        // (`domain_data_authority.rs:9216`-`:9226`).
        let mut bad = payload(1, &[variant_row(7, 0, 0, 0)], &[]);
        bad.pop();
        let mut s = Vec::new();
        s.extend(record(1, 100, D39, COMPACT_RECORD_TYPE_DOMAIN_DATA, &bad));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D39,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload(0, &[variant_row(9, 0, 0, 0)], &[]),
        ));
        s.extend(frame_end(4, 101));
        assert!(scan(&s).is_err());

        // The reserved words are on the OTHER side of that seam (`:15447` before
        // `:15450`), so a failure-status record with a nonzero reserved word is
        // still refused.
        let mut reserved = payload(1, &[variant_row(7, 0, 0, 0)], &[]);
        reserved[36..40].copy_from_slice(&1u32.to_le_bytes());
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D39,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &reserved,
        ));
        s.extend(frame_end(2, 100));
        assert!(scan(&s).is_err());
    }

    #[test]
    fn a_stream_truncated_on_a_record_boundary_is_rejected_or_shorter() {
        // A cut immediately after a DATA record leaves that hour open and ends the
        // stream on a record that is not a FRAME_END. That would otherwise produce
        // a plausible shorter number.
        let full = good_stream();
        for records in [1usize, 3] {
            let cut = boundary(&full, records);
            assert!(
                scan(&full[..cut]).is_err(),
                "a stream cut after {records} record(s) leaves an hour open"
            );
        }
        // A cut exactly at a FRAME_END boundary IS a well-formed shorter stream,
        // and `eof_check` accepts it. Refusing it would be wrong; what matters is
        // that it reports the hours it actually closed.
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
            D39,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload(0, &[variant_row(7, 0, 0, 0)], &[]),
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
            D39,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload(0, &[variant_row(7, 0, 0, 0)], &[]),
        ));
        s.extend(frame_end(2, 101));
        s.extend(record(
            3,
            100,
            D39,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload(0, &[variant_row(8, 0, 0, 0)], &[]),
        ));
        s.extend(frame_end(4, 100));
        assert!(scan(&s).is_err());
    }

    #[test]
    fn a_second_record_of_this_domain_in_one_hour_is_rejected() {
        // The domain is incremental, so two records in one hour would apply two
        // shipping frames to one measurement hour. The rule is checked over EVERY
        // DATA record of the domain and before status-based exclusion, so all four
        // success/failure orderings are refused.
        const FAILED: u32 = 1; // COMPACT_STATUS_STATE_UNAVAILABLE
        for (first, second) in [(0u32, 0u32), (0, FAILED), (FAILED, 0), (FAILED, FAILED)] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D39,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload(first, &[variant_row(7, 0, 0, 0)], &[]),
            ));
            s.extend(record(
                2,
                100,
                D39,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload(second, &[variant_row(8, 0, 0, 0)], &[]),
            ));
            s.extend(frame_end(3, 100));
            assert!(
                scan(&s).is_err(),
                "two d39 records in hour 100 with statuses {first:#x}/{second:#x}"
            );
        }
    }

    #[test]
    fn a_stream_with_no_records_of_this_domain_is_refused_not_reported_as_zero() {
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
    fn a_malformed_payload_is_refused_not_skipped() {
        // Short of the header, every length that does not match the header's two
        // counts, a nonzero reserved word, a row the fold rejects, and a coverage
        // mismatch. A structurally sized payload whose rows do not add up is a
        // corrupt stream, not an hour to drop quietly.
        let base = payload(0, &[variant_row(7, 0, 0, 1)], &[entry(0, 3)]);
        let mut bad: Vec<Vec<u8>> = Vec::new();
        for len in [0usize, 8, 43] {
            bad.push(base[..len].to_vec());
        }
        let mut short = base.clone();
        short.pop();
        bad.push(short);
        let mut long = base.clone();
        long.push(0);
        bad.push(long);
        for offset in [8usize, 28] {
            let mut huge = base.clone();
            huge[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            bad.push(huge);
            let mut big = base.clone();
            big[offset..offset + 4].copy_from_slice(&1000u32.to_le_bytes());
            bad.push(big);
        }
        for offset in [36usize, 40] {
            let mut reserved = base.clone();
            reserved[offset..offset + 4].copy_from_slice(&1u32.to_le_bytes());
            bad.push(reserved);
        }
        bad.push(payload(0, &[variant_row(0, 0, 0, 0)], &[])); // zero variant_id
        bad.push(payload(0, &[variant_row(1, 1, 0, 0)], &[])); // nonzero flags
        bad.push(payload(0, &[variant_row(1, 0, 0, 0)], &[entry(0, 1)])); // coverage
        for (i, p) in bad.into_iter().enumerate() {
            let mut s = Vec::new();
            s.extend(record(1, 100, D39, COMPACT_RECORD_TYPE_DOMAIN_DATA, &p));
            s.extend(frame_end(2, 100));
            assert!(
                scan(&s).is_err(),
                "malformed payload {i} must refuse the stream"
            );
        }
    }

    #[test]
    fn a_frame_end_that_does_not_seal_its_hour_refuses_the_stream() {
        let data = || payload(0, &[variant_row(7, 0, 0, 1)], &[entry(0, 3)]);
        let mut ok = Vec::new();
        ok.extend(record(
            1,
            100,
            D39,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &data(),
        ));
        ok.extend(frame_end_sealed(2, 100, 0, 0, 0, 0));
        assert!(scan(&ok).is_ok());

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
                D39,
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
                D39,
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
            assert!(scan(&s).is_err(), "{len}");
        }
        for informational in [1u32 << 4, 1 << 5, 1 << 10, 1 << 11, 1 << 13] {
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D39,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &data(),
            ));
            s.extend(frame_end_sealed(2, 100, 0, 0, informational, 0));
            assert!(scan(&s).is_ok());
        }
        let mut later = good_stream();
        let at = boundary(&later, 3) + COMPACT_RECORD_HEADER_LEN + 28;
        later[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
        assert!(scan(&later).is_err());
    }

    #[test]
    fn perf_hour_binding_is_enforced() {
        let data = |seq, hour| {
            record(
                seq,
                hour,
                D39,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload(0, &[variant_row(7, 0, 0, 1)], &[entry(0, 3)]),
            )
        };
        let mut ok = Vec::new();
        ok.extend(perf(1, 100, 20));
        ok.extend(perf(2, 100, D39));
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
            s.extend(perf(2, 100, D39));
            s.extend(data(3, 100));
            s.extend(frame_end_sealed(4, 100, emitted, scheduled, 0, 0));
            assert!(scan(&s).is_err());
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
                D39,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload(0, &[variant_row(7, 0, 0, 1)], &[entry(0, 3)]),
            )
        };
        for (perf_hour, data_hour) in [(100, 101), (101, 100), (100, 200), (200, 100)] {
            let mut s = Vec::new();
            s.extend(perf(1, perf_hour, 20));
            s.extend(data(2, data_hour));
            s.extend(frame_end_sealed(3, data_hour, 1, 1, 0, 0));
            assert!(scan(&s).is_err());
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
        let base = payload(FAILED, &[variant_row(7, 0, 0, 1)], &[entry(0, 3)]);
        let mut bad = Vec::new();
        let mut short = base.clone();
        short.pop();
        bad.push(short);
        let mut long = base.clone();
        long.push(0);
        bad.push(long);
        let mut changed = base.clone();
        changed[12..16].copy_from_slice(&2u32.to_le_bytes());
        bad.push(changed);
        let mut too_many_variants = base.clone();
        too_many_variants[20..24].copy_from_slice(&0u32.to_le_bytes());
        bad.push(too_many_variants);
        let mut too_many_upgrades = base.clone();
        too_many_upgrades[24..28].copy_from_slice(&0u32.to_le_bytes());
        bad.push(too_many_upgrades);
        let mut invalid_variants = base.clone();
        invalid_variants[32..36].copy_from_slice(&2u32.to_le_bytes());
        bad.push(invalid_variants);
        bad.push(payload(
            FAILED,
            &[variant_row(AUTHORITY_MAX_VARIANT_ID + 1, 0, 0, 0)],
            &[],
        ));
        bad.push(payload(
            FAILED,
            &[variant_row(7, 0, 0, 0), variant_row(7, 0, 0, 0)],
            &[],
        ));
        bad.push(payload(FAILED, &[variant_row(7, 1, 0, 0)], &[]));
        bad.push(payload(FAILED, &[variant_row(7, 0, 0, 2)], &[entry(0, 3)]));
        bad.push(payload(FAILED, &[variant_row(7, 0, 0, 0)], &[entry(0, 3)]));
        bad.push(payload(
            FAILED,
            &[variant_row(7, 0, 0, 1)],
            &[entry(AUTHORITY_MAX_UPGRADE_INDEX, 3)],
        ));
        bad.push(payload(
            FAILED,
            &[variant_row(7, 0, 0, 1)],
            &[entry(0, AUTHORITY_MAX_LEVEL + 1)],
        ));
        bad.push(payload(
            FAILED,
            &[variant_row(7, 0, 0, 2)],
            &[entry(4, 1), entry(4, 2)],
        ));

        // Every bad payload rides in a stream that WOULD otherwise measure: a
        // second, well-formed hour follows it. Without that trailing hour the
        // scan refuses for an unrelated reason — "no usable DATA records" —
        // and the assertion would hold with `check_authority_payload` deleted,
        // which is a test that cannot fail for its own cause.
        for (case, payload_bytes) in bad.into_iter().enumerate() {
            for (domain, schema, domain_id) in [
                (
                    wire_format::d39::AIR_VARIANT_UPGRADES,
                    wire_format::d39::schema(),
                    39u16,
                ),
                (
                    wire_format::d40::STOCKPILE_VARIANT_UPGRADES,
                    wire_format::d40::schema(),
                    40u16,
                ),
            ] {
                let mut s = Vec::new();
                s.extend(record(
                    1,
                    100,
                    domain_id,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &payload_bytes,
                ));
                s.extend(frame_end(2, 100));
                s.extend(record(
                    3,
                    101,
                    domain_id,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    &payload(0, &[variant_row(9, 0, 0, 0)], &[]),
                ));
                s.extend(frame_end(4, 101));
                assert!(
                    scan_stream(&s, domain, &schema, StreamIdentityMode::Legacy).is_err(),
                    "d{domain_id} excluded malformed payload {case}"
                );
            }
        }
        for bit in [0u32, 1, 2, 3, 6, 7, 8, 9, 12] {
            let mut malformed = payload(1 << bit, &[variant_row(7, 0, 0, 1)], &[entry(0, 3)]);
            malformed[12..16].copy_from_slice(&2u32.to_le_bytes());
            let mut s = Vec::new();
            s.extend(record(
                1,
                100,
                D39,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &malformed,
            ));
            s.extend(frame_end(2, 100));
            s.extend(record(
                3,
                101,
                D39,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &payload(0, &[variant_row(9, 0, 0, 0)], &[]),
            ));
            s.extend(frame_end(4, 101));
            assert!(scan(&s).is_err(), "failure bit {bit}");
        }

        let mut s = Vec::new();
        s.extend(record(1, 100, D39, COMPACT_RECORD_TYPE_DOMAIN_DATA, &base));
        s.extend(frame_end(2, 100));
        s.extend(record(
            3,
            101,
            D39,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload(0, &[variant_row(9, 0, 0, 0)], &[]),
        ));
        s.extend(frame_end(4, 101));
        let scanned = scan(&s).expect("well-formed excluded record");
        assert_eq!(scanned.failure_records, 1);
        assert_eq!(scanned.hours.len(), 1);

        // The two count predicates are conditional on DELTA_DATA in the second
        // authority (`domain_data_authority.rs:9194`-`:9205`).
        let mut delta = payload(
            COMPACT_STATUS_DELTA_DATA,
            &[variant_row(7, 0, 0, 1)],
            &[entry(0, 3)],
        );
        delta[20..28].fill(0);
        let mut s = Vec::new();
        s.extend(record(1, 100, D39, COMPACT_RECORD_TYPE_DOMAIN_DATA, &delta));
        s.extend(frame_end(2, 100));
        assert!(
            scan(&s).is_ok(),
            "DELTA_DATA disables the two observed-count gates"
        );
    }

    #[test]
    fn the_identity_protocol_is_stated_not_sniffed() {
        use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;

        let decapitated = good_stream();
        assert!(scan_stream(
            &decapitated,
            domain(),
            &schema(),
            StreamIdentityMode::IdentityV1
        )
        .is_err());
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
    fn a_payload_both_authorities_accept_is_measured_however_unusual() {
        // Descending but unique indexes are accepted by both passes. Duplicate
        // indexes are accepted by the fold but refused by the second authority
        // (`domain_data_authority.rs:9329`-`:9339`).
        let entries = vec![entry(5, 50), entry(2, 20)];
        let mut s = Vec::new();
        s.extend(record(
            1,
            100,
            D39,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload(0, &[variant_row(1, 0, 0, 2)], &entries),
        ));
        s.extend(frame_end(2, 100));
        let scanned = scan(&s).expect("both authorities accept descending indexes");
        assert_eq!(scanned.hours.len(), 1);
        assert_eq!(scanned.shipped_upgrade_rows, 2);
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
            let _ = scan(&bytes);
        }
        // And corruption of a genuinely well-formed stream, byte by byte.
        let good = good_stream();
        for i in 0..good.len() {
            let mut s = good.clone();
            s[i] ^= 0xFF;
            let _ = scan(&s);
        }
    }
}
